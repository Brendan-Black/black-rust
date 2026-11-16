//! The magic-string check (black values/magic-strings): a string value that appears in more than one literal of a
//! package's non-test code should be named, so `check` reports every such value.
//!
//! Values match only when they are identical: same characters, same case, nothing trimmed. "orderId" matches
//! "orderId" and nothing else: not "orderid", not "orderIdList". Escapes are resolved first, so
//! `"\x41"` and `"A"` are the same value, as are a raw and a plain literal with the same contents.
//!
//! Each literal is one of three kinds. An *argument* is passed straight to a function, method or macro (`get("id")`,
//! `.args(["--all"])`): the biggest offenders, so a value with an argument among its literals gets the package's full
//! level. A *definition* is inside a `const` or `static`: a value whose literals are all definitions is two names that
//! happen to share a spelling, and isn't reported. Everything else (assignments, returns, comparisons, match arms) is
//! *elsewhere*, and a value seen only there and in definitions is capped at the `magic-strings-elsewhere` level. A
//! literal in a constructor (`Some("x")`, `Ok("x")`) is an argument only if the constructor is passed on.
//!
//! Two shapes are eased. A value whose literals are all in one function, with at most one of them an argument, isn't
//! reported: a parser's `strip_prefix("--x=")` beside its `"--x=" =>` arm is read and changed together. And a value
//! passed as an argument just once, with every other use in other files (or in that same function), is at most a
//! warning: it is most likely another concept that shares the spelling, such as one flag in two commands' parsers.
//!
//! Not counted at all: values with no letter (`", "`, `"\n"`, `"404"`); values with whitespace in them, unless
//! `magic-strings-whitespace` is true; messages (the format string of `format!`, `println!`, `panic!`, `write!`, the
//! `assert` macros and the logging macros, and what is formatted into it; the arguments of `expect`, `context`,
//! `ok_or`, `Err`, and any function of a type whose name ends in `Error`, such as `io::Error::other`); the literals of
//! compile-time macros such as `env!` and `include_str!`; attributes and `extern "C"`; labels (a literal passed straight
//! to a parameter of the package's own functions that only reaches a message: see [`message_params`]); generated files
//! (a line of the opening comment that starts with `@generated`); and test code (`tests/`, `benches/`, `examples/`, and
//! items marked `#[test]` or `#[cfg(test)]`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use syn::visit::{self, Visit};

mod message_params;

use message_params::MessageParams;

/// How severe a finding is, least severe first.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
	Allow,
	Warn,
	Deny,
}

/// The level for values passed as arguments: `[package.metadata.black]`, or `[workspace.metadata.black]` for every
/// package that doesn't say. Deny by default.
pub const SETTING: &str = "magic-strings";
/// The most severe a value that is never an argument can be, set the same way. Allow by default: only a value passed as
/// an argument is reported unless a package opts in.
pub const ELSEWHERE_SETTING: &str = "magic-strings-elsewhere";
/// Whether values with whitespace in them are checked, set the same way: `true` or `false`. False by default: a key, a
/// header or a flag has none, so a value with a space or a newline is almost always text for people.
pub const WHITESPACE_SETTING: &str = "magic-strings-whitespace";
/// How close a value's literals must all be to be read together, and so not reported, set the same way: a number of
/// lines (0, the default, is one line), or -1 for one file, -2 for one directory (a module and its submodules), -3 for
/// the whole package.
pub const PROXIMITY_SETTING: &str = "magic-strings-proximity";

/// How close a value's literals must all be to be read together. A constant among them is never close: a constant plus
/// a literal should use the constant.
#[derive(Clone, Copy)]
pub enum Proximity {
	/// Within this many lines of each other, in one file: 0 is one line.
	Lines(usize),
	File,
	/// One directory: a module and its submodules.
	Directory,
	/// The whole package, so no value is reported.
	Package,
}

impl Proximity {
	pub fn parse(n: i64) -> Result<Self, String> {
		match n {
			-1 => Ok(Self::File),
			-2 => Ok(Self::Directory),
			-3 => Ok(Self::Package),
			_ => usize::try_from(n).map(Self::Lines).map_err(|_| format!("{PROXIMITY_SETTING} is {n}; expected a number of lines (0 or more), -1, -2 or -3")),
		}
	}

	/// Whether these literals are all this close.
	fn holds(self, places: &[Place]) -> bool {
		let Some(first) = places.first() else { return false };
		let same_file = places.iter().all(|p| p.file == first.file);
		match self {
			Self::Lines(n) => {
				let low = places.iter().map(|p| p.line).min().unwrap_or(first.line);
				let high = places.iter().map(|p| p.line).max().unwrap_or(first.line);
				same_file && high - low <= n
			}
			Self::File => same_file,
			Self::Directory => places.iter().all(|p| p.file.parent() == first.file.parent()),
			Self::Package => true,
		}
	}
}

/// Each level by the word a manifest uses for it.
const LEVELS: [(&str, Level); 3] = [("deny", Level::Deny), ("warn", Level::Warn), ("allow", Level::Allow)];

impl Level {
	pub fn parse(key: &str, value: &str) -> Result<Self, String> {
		LEVELS
			.iter()
			.find(|(word, _)| *word == value)
			.map(|(_, level)| *level)
			.ok_or_else(|| format!("{key} is \"{value}\"; expected \"deny\", \"warn\" or \"allow\""))
	}
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
	Argument,
	Definition,
	Elsewhere,
}

/// Where a literal is: its file, its 1-based line and column, its kind, and the function (or other item) it is in.
pub struct Place {
	pub file: PathBuf,
	pub line: usize,
	pub column: usize,
	pub kind: Kind,
	/// The function, or other item outside one, the literal is in: unique across the package.
	pub scope: usize,
}

/// A value to report: where it appears, whether any of those is an argument, and whether that argument is alone.
pub struct Finding {
	pub places: Vec<Place>,
	pub as_argument: bool,
	/// One argument, every other use being outside its file (or in its own function): most likely another concept
	/// that shares the spelling, such as one flag in two commands' parsers, so it is reported at most as a warning.
	pub lone_argument: bool,
}

/// Every value to report in the package's non-test code, sorted by value.
pub fn findings(package_dir: &Path, whitespace: bool, proximity: Proximity) -> Result<BTreeMap<String, Finding>, String> {
	let mut files = Vec::new();
	collect_files(package_dir, true, &mut files)?;
	files.sort();
	let mut parsed = Vec::new();
	// `#[cfg(test)] mod tests;` keeps its code in another file, which the walk finds like any other.
	let mut test_modules = Vec::new();
	for file in files {
		let text = crate::read(&file)?;
		if is_generated(&text) {
			continue;
		}
		let syntax = syn::parse_file(&text).map_err(|e| format!("parsing {} for the magic-string check: {e}", file.display()))?;
		out_of_line_tests(&syntax.items, &module_dir(&file), file.parent().unwrap_or(Path::new("")), &mut test_modules);
		parsed.push((file, syntax));
	}
	parsed.retain(|(file, _)| !test_modules.iter().any(|m| file.starts_with(m)));
	Ok(findings_in(&parsed, whitespace, proximity))
}

/// Every value to report in these parsed files, sorted by value.
fn findings_in(parsed: &[(PathBuf, syn::File)], whitespace: bool, proximity: Proximity) -> BTreeMap<String, Finding> {
	let messages = MessageParams::of(&parsed.iter().map(|(_, syntax)| syntax).collect::<Vec<_>>());
	let mut seen: BTreeMap<String, Vec<Place>> = BTreeMap::new();
	let mut scopes = 0;
	for (file, syntax) in parsed {
		let mut finder = Finder { file, seen: &mut seen, in_definition: false, whitespace, messages: &messages, owner: None, scope: 0, scopes: &mut scopes };
		finder.new_scope();
		finder.visit_file(syntax);
	}
	seen.into_iter().filter_map(|(value, places)| finding(places, proximity).map(|f| (value, f))).collect()
}

/// What to report of a value's literals, if anything: not a value seen once, nor one whose literals are all definitions,
/// nor one whose literals are all in one function (or other item) with at most one of them an argument, where they are
/// read together and change together.
fn finding(places: Vec<Place>, proximity: Proximity) -> Option<Finding> {
	// One function's literals are exempt only with at most one argument among them: two (declaring a flag and looking it
	// up, or writing a key and reading it) are the typo the rule is for.
	let arguments_in_all = places.iter().filter(|p| p.kind == Kind::Argument).count();
	let one_scope = places.iter().all(|p| p.scope == places[0].scope) && arguments_in_all <= 1;
	// Literals this close, however many are arguments, are read together: `starts_with("--x")` and `value("--x")` on one
	// parser line. Not with a constant among them, though: a constant plus a literal should use the constant.
	let close = places.iter().all(|p| p.kind != Kind::Definition) && proximity.holds(&places);
	if places.len() < 2 || places.iter().all(|p| p.kind == Kind::Definition) || one_scope || close {
		return None;
	}
	let mut arguments = places.iter().filter(|p| p.kind == Kind::Argument);
	let first = arguments.next();
	let lone_argument = match (first, arguments.next()) {
		(Some(argument), None) => places.iter().all(|p| p.scope == argument.scope || p.file != argument.file),
		_ => false,
	};
	let as_argument = first.is_some();
	Some(Finding { places, as_argument, lone_argument })
}

/// Files that are modules' roots, so their `mod x;` is a sibling `x.rs` rather than one in a directory named after them.
const ROOT_FILES: [&str; 4] = ["lib.rs", "main.rs", "mod.rs", "build.rs"];
/// The directory of binaries, each a crate root of its own.
const BIN_DIR: &str = "bin";

/// The directory where `mod x;` in the file looks for `x.rs` and `x/`.
fn module_dir(file: &Path) -> PathBuf {
	let parent = file.parent().unwrap_or(Path::new(""));
	let name = file.file_name().and_then(|n| n.to_str()).unwrap_or_default();
	if ROOT_FILES.contains(&name) || parent.file_name().is_some_and(|d| d == BIN_DIR) {
		parent.to_path_buf()
	} else {
		parent.join(file.file_stem().unwrap_or_default())
	}
}

/// The files and directories of the out-of-line modules marked as test code among `items`, a module whose files are
/// looked for in `dir`: `x.rs` and `x/` for `mod x;`, or the `#[path]` it names, which is relative to `file_dir`, the
/// directory of the file that declares it.
fn out_of_line_tests(items: &[syn::Item], dir: &Path, file_dir: &Path, out: &mut Vec<PathBuf>) {
	for item in items {
		let syn::Item::Mod(module) = item else { continue };
		let name = module.ident.to_string();
		match &module.content {
			Some((_, inner)) => out_of_line_tests(inner, &dir.join(&name), file_dir, out),
			None if is_test(&module.attrs) => match path_attribute(&module.attrs) {
				Some(path) => out.push(file_dir.join(path)),
				None => out.extend([dir.join(format!("{name}.rs")), dir.join(&name)]),
			},
			None => {}
		}
	}
}

/// `#[path = "..."]`'s value.
fn path_attribute(attrs: &[syn::Attribute]) -> Option<String> {
	attrs.iter().find(|a| a.path().is_ident("path")).and_then(|a| match &a.meta {
		syn::Meta::NameValue(syn::MetaNameValue { value: syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }), .. }) => Some(s.value()),
		_ => None,
	})
}

/// Whether a line of the comments that open the file starts with `@generated`, the marker code generators write
/// (`// @generated by prost`). Only at the start, so a comment that merely mentions it doesn't count.
fn is_generated(text: &str) -> bool {
	text
		.lines()
		.map(str::trim_start)
		.take_while(|line| line.is_empty() || line.starts_with("//") || line.starts_with("/*") || line.starts_with('*') || line.starts_with("#!"))
		.any(|line| line.trim_start_matches(['/', '*', '!', ' ', '\t']).starts_with("@generated"))
}

/// Directories that hold test code, or no code: never searched.
const SKIPPED_DIRS: [&str; 4] = ["target", "tests", "benches", "examples"];

/// The package's .rs files, leaving out test code and any package nested inside it, which is checked on its own.
fn collect_files(dir: &Path, is_root: bool, out: &mut Vec<PathBuf>) -> Result<(), String> {
	if !is_root && dir.join(crate::MANIFEST).exists() {
		return Ok(());
	}
	let listing = |e: std::io::Error| format!("listing {}: {e}", dir.display());
	for entry in fs::read_dir(dir).map_err(listing)? {
		let entry = entry.map_err(listing)?;
		let path = entry.path();
		let name = entry.file_name();
		let name = name.to_string_lossy();
		let kind = entry.file_type().map_err(listing)?;
		if kind.is_dir() {
			if !name.starts_with('.') && !SKIPPED_DIRS.contains(&name.as_ref()) {
				collect_files(&path, false, out)?;
			}
		} else if kind.is_file() && name.ends_with(".rs") {
			out.push(path);
		}
	}
	Ok(())
}

/// Methods whose string arguments are messages for people.
const MESSAGE_METHODS: [&str; 5] = ["expect", "expect_err", "context", "wrap_err", "ok_or"];
/// Functions, by the last part of their path, whose string arguments are messages for people.
const MESSAGE_FUNCTIONS: [&str; 1] = ["Err"];
/// The end of a type's name that says its functions build errors (`Error::invalid`, `io::Error::other`), whose string
/// arguments are messages: the counterpart of an exception's constructor.
const ERROR_TYPE_SUFFIX: &str = "Error";

/// Whether a call's string arguments are messages: `Err(..)`, or a function of an error type.
fn is_message_call(func: &syn::Expr) -> bool {
	let syn::Expr::Path(p) = func else { return false };
	let mut segments = p.path.segments.iter().rev();
	let last = segments.next().is_some_and(|s| MESSAGE_FUNCTIONS.contains(&s.ident.to_string().as_str()));
	last || segments.next().is_some_and(|s| s.ident.to_string().ends_with(ERROR_TYPE_SUFFIX))
}

/// Whether a call builds a value rather than doing something with one: a tuple struct or enum variant, such as
/// `Some(..)`, `Ok(..)` or `Mode::Named(..)`, by the capital its path ends with. A literal it wraps is passed on only
/// if the value it builds is.
fn is_constructor(func: &syn::Expr) -> bool {
	let syn::Expr::Path(p) = func else { return false };
	p.path.segments.last().is_some_and(|s| s.ident.to_string().starts_with(char::is_uppercase))
}

/// Macros that read their literals at compile time, as names of environment variables and files or as pieces of
/// another literal: `env!("CARGO_PKG_VERSION")` names no value of the program's.
const COMPILE_TIME_MACROS: [&str; 7] = ["env", "option_env", "include", "include_str", "include_bytes", "concat", "stringify"];

/// Which of a macro's comma-separated arguments is its message, if it has one.
enum Message {
	At(usize),
	/// The logging macros, which may start with `target: "...",` or fields: their first argument that is a lone string.
	FirstString,
}

/// The macro that is a `match` in an expression.
const MATCHES: &str = "matches";

/// The `log` and `tracing` macros.
const LOG_MACROS: [&str; 6] = ["trace", "debug", "info", "warn", "error", "event"];

fn message_of(macro_name: &str) -> Option<Message> {
	match macro_name {
		"format" | "format_args" | "print" | "println" | "eprint" | "eprintln" | "panic" | "unreachable" | "todo" | "unimplemented" | "compile_error"
		| "anyhow" | "bail" => Some(Message::At(0)),
		"write" | "writeln" | "assert" | "debug_assert" | "ensure" => Some(Message::At(1)),
		"assert_eq" | "assert_ne" | "debug_assert_eq" | "debug_assert_ne" => Some(Message::At(2)),
		name if LOG_MACROS.contains(&name) => Some(Message::FirstString),
		_ => None,
	}
}

struct Finder<'a> {
	file: &'a Path,
	seen: &'a mut BTreeMap<String, Vec<Place>>,
	/// Inside a `const` or `static`, where every literal is a definition.
	in_definition: bool,
	/// Whether values with whitespace in them are counted.
	whitespace: bool,
	/// Which parameters of the package's functions are messages.
	messages: &'a MessageParams,
	/// The type of the `impl` being visited, which `Self` stands for.
	owner: Option<String>,
	/// The function (or other item) being visited.
	scope: usize,
	/// How many scopes the package has had, so each gets its own number.
	scopes: &'a mut usize,
}

impl Finder<'_> {
	/// Enter a scope of its own, returning the one it was in.
	fn new_scope(&mut self) -> usize {
		*self.scopes += 1;
		std::mem::replace(&mut self.scope, *self.scopes)
	}

	/// Visit an item in a scope of its own.
	fn in_scope(&mut self, visit: impl FnOnce(&mut Self)) {
		let outer = self.new_scope();
		visit(self);
		self.scope = outer;
	}

	fn record(&mut self, value: String, span: proc_macro2::Span, kind: Kind) {
		if !value.chars().any(char::is_alphabetic) || (!self.whitespace && value.chars().any(char::is_whitespace)) {
			return;
		}
		let kind = if self.in_definition { Kind::Definition } else { kind };
		let start = span.start();
		self.seen.entry(value).or_default().push(Place { file: self.file.to_owned(), line: start.line, column: start.column + 1, kind, scope: self.scope });
	}

	/// An expression passed to a call: a string literal in it is an argument, also inside `&`, `( )`, `[ ]` and tuples.
	fn argument(&mut self, expr: &syn::Expr) {
		match expr {
			syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) => self.record(s.value(), s.span(), Kind::Argument),
			syn::Expr::Reference(r) => self.argument(&r.expr),
			syn::Expr::Paren(p) => self.argument(&p.expr),
			syn::Expr::Array(a) => a.elems.iter().for_each(|e| self.argument(e)),
			syn::Expr::Tuple(t) => t.elems.iter().for_each(|e| self.argument(e)),
			// `Some("x")` passed on is still passed on.
			syn::Expr::Call(c) if is_constructor(&c.func) => c.args.iter().for_each(|e| self.argument(e)),
			other => self.visit_expr(other),
		}
	}

	fn in_definition(&mut self, visit: impl FnOnce(&mut Self)) {
		let outer = self.in_definition;
		self.in_definition = true;
		visit(self);
		self.in_definition = outer;
	}

	/// Every string literal among a macro's tokens, as `kind`.
	fn macro_tokens(&mut self, tokens: proc_macro2::TokenStream, kind: Kind) {
		for token in tokens {
			match token {
				proc_macro2::TokenTree::Literal(literal) => {
					let span = literal.span();
					if let syn::Lit::Str(s) = syn::Lit::new(literal) {
						self.record(s.value(), span, kind);
					}
				}
				proc_macro2::TokenTree::Group(group) => self.macro_tokens(group.stream(), kind),
				proc_macro2::TokenTree::Ident(_) | proc_macro2::TokenTree::Punct(_) => {}
			}
		}
	}
}

/// A macro's arguments: its tokens, split at the commas outside any brackets.
fn split_arguments(tokens: proc_macro2::TokenStream) -> Vec<proc_macro2::TokenStream> {
	let mut arguments = vec![proc_macro2::TokenStream::new()];
	for token in tokens {
		match &token {
			proc_macro2::TokenTree::Punct(p) if p.as_char() == ',' => arguments.push(proc_macro2::TokenStream::new()),
			_ => arguments.last_mut().expect("arguments starts with one, and only grows").extend([token]),
		}
	}
	arguments
}

/// Whether an argument is a string literal, also behind `&` or `( )`. Only one of those, passed straight to a message
/// parameter, is a label: anything else there (a map, a call) still holds values of its own.
fn is_str_literal(expr: &syn::Expr) -> bool {
	match expr {
		syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(_), .. }) => true,
		syn::Expr::Reference(r) => is_str_literal(&r.expr),
		syn::Expr::Paren(p) => is_str_literal(&p.expr),
		_ => false,
	}
}

/// A macro's name, its arguments, and how many of those come before its message: all of them, if it has none. The
/// message, and what is formatted into it, are text for people.
fn macro_arguments(mac: &syn::Macro) -> (String, Vec<proc_macro2::TokenStream>, usize) {
	let name = mac.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
	let arguments = split_arguments(mac.tokens.clone());
	let message = match message_of(&name) {
		Some(Message::At(i)) => Some(i),
		Some(Message::FirstString) => arguments.iter().position(is_lone_string),
		None => None,
	};
	let counted = message.unwrap_or(arguments.len());
	(name, arguments, counted)
}

fn is_lone_string(argument: &proc_macro2::TokenStream) -> bool {
	let mut tokens = argument.clone().into_iter();
	matches!((tokens.next(), tokens.next()), (Some(proc_macro2::TokenTree::Literal(l)), None) if matches!(syn::Lit::new(l.clone()), syn::Lit::Str(_)))
}

impl<'ast> Visit<'ast> for Finder<'_> {
	fn visit_attribute(&mut self, _: &'ast syn::Attribute) {}

	fn visit_lit_str(&mut self, s: &'ast syn::LitStr) {
		self.record(s.value(), s.span(), Kind::Elsewhere);
	}

	fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
		self.visit_expr(&call.func);
		if is_message_call(&call.func) {
			return;
		}
		// A value wrapped (`Some("hard")`, `Ok("x")`, `Mode::Named("x")`) is only passed on if the wrapper is.
		if is_constructor(&call.func) {
			call.args.iter().for_each(|arg| self.visit_expr(arg));
			return;
		}
		let path = match &*call.func {
			syn::Expr::Path(p) => Some(&p.path),
			_ => None,
		};
		for (i, arg) in call.args.iter().enumerate() {
			let label = is_str_literal(arg) && path.is_some_and(|path| self.messages.call(path, call.args.len(), i, self.owner.as_deref()));
			if !label {
				self.argument(arg);
			}
		}
	}

	fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
		self.visit_expr(&call.receiver);
		if MESSAGE_METHODS.contains(&call.method.to_string().as_str()) {
			return;
		}
		for (i, arg) in call.args.iter().enumerate() {
			if !(is_str_literal(arg) && self.messages.method(&call.method, call.args.len(), i)) {
				self.argument(arg);
			}
		}
	}

	fn visit_macro(&mut self, mac: &'ast syn::Macro) {
		let (name, arguments, counted) = macro_arguments(mac);
		if COMPILE_TIME_MACROS.contains(&name.as_str()) {
			return;
		}
		for (i, argument) in arguments.into_iter().take(counted).enumerate() {
			// `matches!(x, "a" | "b")` is a match: its patterns and guard are match arms, not arguments.
			let kind = if name == MATCHES && i > 0 { Kind::Elsewhere } else { Kind::Argument };
			self.macro_tokens(argument, kind);
		}
	}

	fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
		let outer = self.owner.take();
		self.owner = match &*item.self_ty {
			syn::Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
			_ => None,
		};
		visit::visit_item_impl(self, item);
		self.owner = outer;
	}

	/// `extern "C"` names an ABI, not a value.
	fn visit_abi(&mut self, _: &'ast syn::Abi) {}

	fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
		self.in_definition(|f| visit::visit_item_const(f, item));
	}

	fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
		self.in_definition(|f| visit::visit_item_static(f, item));
	}

	fn visit_impl_item_const(&mut self, item: &'ast syn::ImplItemConst) {
		self.in_definition(|f| visit::visit_impl_item_const(f, item));
	}

	fn visit_trait_item_const(&mut self, item: &'ast syn::TraitItemConst) {
		self.in_definition(|f| visit::visit_trait_item_const(f, item));
	}

	fn visit_item(&mut self, item: &'ast syn::Item) {
		if is_test(item_attrs(item)) {
			return;
		}
		// An item holding items gives each its own scope; any other item is a scope of its own.
		match item {
			syn::Item::Mod(_) | syn::Item::Impl(_) | syn::Item::Trait(_) | syn::Item::ForeignMod(_) => visit::visit_item(self, item),
			_ => self.in_scope(|f| visit::visit_item(f, item)),
		}
	}

	fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
		self.in_scope(|f| visit::visit_trait_item(f, item));
	}

	fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
		let attrs: &[syn::Attribute] = match item {
			syn::ImplItem::Const(i) => &i.attrs,
			syn::ImplItem::Fn(i) => &i.attrs,
			syn::ImplItem::Type(i) => &i.attrs,
			syn::ImplItem::Macro(i) => &i.attrs,
			_ => &[],
		};
		if !is_test(attrs) {
			self.in_scope(|f| visit::visit_impl_item(f, item));
		}
	}
}

fn item_attrs(item: &syn::Item) -> &[syn::Attribute] {
	match item {
		syn::Item::Const(i) => &i.attrs,
		syn::Item::Enum(i) => &i.attrs,
		syn::Item::ExternCrate(i) => &i.attrs,
		syn::Item::Fn(i) => &i.attrs,
		syn::Item::ForeignMod(i) => &i.attrs,
		syn::Item::Impl(i) => &i.attrs,
		syn::Item::Macro(i) => &i.attrs,
		syn::Item::Mod(i) => &i.attrs,
		syn::Item::Static(i) => &i.attrs,
		syn::Item::Struct(i) => &i.attrs,
		syn::Item::Trait(i) => &i.attrs,
		syn::Item::TraitAlias(i) => &i.attrs,
		syn::Item::Type(i) => &i.attrs,
		syn::Item::Union(i) => &i.attrs,
		syn::Item::Use(i) => &i.attrs,
		_ => &[],
	}
}

const TEST: &str = "test";

/// `#[test]`, or a `#[cfg(...)]` that mentions `test`: `cfg(test)`, `cfg(all(test, unix))` and so on.
fn is_test(attrs: &[syn::Attribute]) -> bool {
	attrs.iter().any(|attr| {
		let path = attr.path();
		path.is_ident(TEST) || (path.is_ident("cfg") && mentions_test(attr.meta.require_list().map(|l| l.tokens.clone()).unwrap_or_default()))
	})
}

fn mentions_test(tokens: proc_macro2::TokenStream) -> bool {
	tokens.into_iter().any(|token| match token {
		proc_macro2::TokenTree::Ident(ident) => ident == TEST,
		proc_macro2::TokenTree::Group(group) => mentions_test(group.stream()),
		proc_macro2::TokenTree::Literal(_) | proc_macro2::TokenTree::Punct(_) => false,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The findings in these files, each given as its path and source.
	fn check(files: &[(&str, &str)]) -> BTreeMap<String, Finding> {
		let parsed: Vec<(PathBuf, syn::File)> = files.iter().map(|(path, text)| (PathBuf::from(path), syn::parse_file(text).unwrap())).collect();
		findings_in(&parsed, false, Proximity::Lines(0))
	}

	#[test]
	fn a_value_passed_from_two_functions_is_reported() {
		let found = check(&[("a.rs", "fn put() { set(\"allow-ff\"); }\nfn get() { read(\"allow-ff\"); }")]);
		assert!(found["allow-ff"].as_argument && !found["allow-ff"].lone_argument);
	}

	#[test]
	fn literals_in_one_function_are_not() {
		let found = check(&[("a.rs", "fn parse(a: &str) { if let Some(v) = a.strip_prefix(\"--abbrev=\") {} match a { \"--abbrev=\" => {}, _ => {} } }")]);
		assert!(found.is_empty());
	}

	#[test]
	fn two_arguments_in_one_function_are_reported() {
		let found = check(&[("a.rs", "fn run(rest: &[&str]) {\nlet a = parse(rest, &[(\"allow-http\", 0)]);\nif a.has(\"allow-http\") {}\n}")]);
		assert!(found["allow-http"].as_argument && !found["allow-http"].lone_argument);
	}

	/// The findings in one file at a proximity.
	fn check_at(text: &str, proximity: Proximity) -> BTreeMap<String, Finding> {
		findings_in(&[(PathBuf::from("a.rs"), syn::parse_file(text).unwrap())], false, proximity)
	}

	#[test]
	fn arguments_on_one_line_are_read_together() {
		let found = check(&[("a.rs", "fn parse(arg: &str) { if arg.starts_with(\"--author\") { value(\"--author\"); } }")]);
		assert!(found.is_empty());
	}

	#[test]
	fn proximity_widens_what_is_read_together() {
		let text = "fn put() {\nset(\"allow-ff\");\n}\nfn get() {\nread(\"allow-ff\");\n}";
		assert!(check_at(text, Proximity::Lines(0)).contains_key("allow-ff"));
		assert!(check_at(text, Proximity::Lines(2)).contains_key("allow-ff"));
		assert!(check_at(text, Proximity::Lines(3)).is_empty());
		assert!(check_at(text, Proximity::File).is_empty());
		let two_files = [("a/x.rs", "fn put() { set(\"allow-ff\"); }"), ("a/y.rs", "fn get() { read(\"allow-ff\"); }")];
		let parsed: Vec<(PathBuf, syn::File)> = two_files.iter().map(|(p, t)| (PathBuf::from(p), syn::parse_file(t).unwrap())).collect();
		assert!(findings_in(&parsed, false, Proximity::File).contains_key("allow-ff"));
		assert!(findings_in(&parsed, false, Proximity::Directory).is_empty());
	}

	#[test]
	fn a_constant_is_never_close() {
		let found = check(&[("a.rs", "const KEY: &str = \"allow-ff\"; fn a() { set(\"allow-ff\"); }")]);
		assert!(found.contains_key("allow-ff"));
		assert!(Proximity::parse(-4).is_err() && Proximity::parse(5).is_ok());
	}

	#[test]
	fn arrays_passed_on_stay_arguments() {
		let found = check(&[("a.rs", "fn a() { c.args([\"--all\"]); }\nfn b() { d.args(vec![\"--all\"]); }")]);
		assert!(found["--all"].as_argument);
	}

	#[test]
	fn a_wrapped_value_returned_is_not_an_argument() {
		let found = check(&[("a.rs", "fn name() -> Option<&'static str> { Some(\"hard\") }\nfn is(a: &str) -> bool { a == \"hard\" }")]);
		assert!(!found["hard"].as_argument);
		// Passed on, it still is.
		let found = check(&[("a.rs", "fn one() { f(Some(\"hard\")); }\nfn two() { g(\"hard\"); }")]);
		assert!(found["hard"].as_argument);
	}

	#[test]
	fn compile_time_macros_are_not_counted() {
		let found = check(&[("a.rs", "fn a() { f(env!(\"CARGO_PKG_VERSION\")); }\nfn b() { g(env!(\"CARGO_PKG_VERSION\")); }")]);
		assert!(found.is_empty());
	}

	#[test]
	fn a_lone_argument_among_other_files_uses_is_marked() {
		let found = check(&[
			("a.rs", "fn quiet(a: &str) { f(\"-q\"); match a { \"-q\" => {}, _ => {} } }"),
			("b.rs", "fn parse(a: &str) { match a { \"-q\" => {}, _ => {} } }"),
		]);
		assert!(found["-q"].lone_argument);
		// Another use in the argument's own file, outside its function, is the same concept as far as anyone can tell.
		let found = check(&[("a.rs", "fn one() { f(\"-q\"); }\nfn two(a: &str) { match a { \"-q\" => {}, _ => {} } }")]);
		assert!(!found["-q"].lone_argument);
	}

	#[test]
	fn a_label_is_not_counted() {
		let found = check(&[(
			"a.rs",
			"fn check(fmt: &str) -> Result<(), Error> { Err(Error::invalid(format!(\"{fmt} is bad\"))) }\nfn a() { check(\"JSON\"); }\nfn b() { check(\"JSON\"); }",
		)]);
		assert!(found.is_empty());
	}

	#[test]
	fn an_unused_parameter_is_not_a_message() {
		let found = check(&[("a.rs", "fn set(_key: &str) {}\nfn a() { set(\"allow-ff\"); }\nfn b() { set(\"allow-ff\"); }")]);
		assert!(found["allow-ff"].as_argument);
	}

	#[test]
	fn a_parameter_passed_through_a_call_into_a_log_is_a_value() {
		let found = check(&[(
			"a.rs",
			"fn store(item: &str) { println!(\"{}\", to_json(item)); }\nfn a() { store(\"originalEventId\"); }\nfn b() { store(\"originalEventId\"); }",
		)]);
		assert!(found["originalEventId"].as_argument);
	}
}
