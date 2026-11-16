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
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Level {
	Allow,
	Warn,
	Deny,
}

/// The level for values passed as arguments: `[package.metadata.black]`, `[workspace.metadata.black]` for every
/// package that doesn't say, or a module's table (see [`MODULES_TABLE`]). Deny by default.
pub const SETTING: &str = "magic-strings";
/// The most severe a value that is never an argument can be, set the same way. Allow by default: only a value passed as
/// an argument is reported unless a package opts in.
pub const ELSEWHERE_SETTING: &str = "magic-strings-elsewhere";
/// Whether values with whitespace in them are checked, set the same way: `true` or `false`. False by default: a key, a
/// header or a flag has none, so a value with a space or a newline is almost always text for people.
pub const WHITESPACE_SETTING: &str = "magic-strings-whitespace";
/// How close a value's literals must all be to be read together, and so not reported, set the same way: one of the
/// [`Proximity`] words. `"statement"` by default.
pub const PROXIMITY_SETTING: &str = "magic-strings-proximity";
/// The table, under `[package.metadata.black]`, of settings for part of the package, keyed by module path:
/// `[package.metadata.black.modules."cli::log"]`. A module's table covers it and the modules inside it.
pub const MODULES_TABLE: &str = "modules";

/// How close a value's literals must all be to be read together, narrowest first. A constant among them is never close:
/// a constant plus a literal should use the constant.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Proximity {
	/// One statement, or one branch: a match arm with its guard, or an `if`'s condition with its block. Where the
	/// formatter breaks the lines doesn't matter.
	Statement,
	/// One function (or other item).
	Function,
	File,
	/// One module below the crate root and the modules inside it (`cli` and `cli::log`), or the root module's own code.
	Module,
	/// The whole crate: no value is reported.
	Crate,
}

/// Each proximity by the word a manifest uses for it.
const PROXIMITIES: [(&str, Proximity); 5] = [
	("statement", Proximity::Statement),
	("function", Proximity::Function),
	("file", Proximity::File),
	("module", Proximity::Module),
	("crate", Proximity::Crate),
];

impl Proximity {
	pub fn parse(value: &str) -> Result<Self, String> {
		PROXIMITIES.iter().find(|(word, _)| *word == value).map(|(_, p)| *p).ok_or_else(|| {
			let words: Vec<String> = PROXIMITIES.iter().map(|(word, _)| format!("\"{word}\"")).collect();
			format!("{PROXIMITY_SETTING} is \"{value}\"; expected one of {}", words.join(", "))
		})
	}

	/// Whether these literals are all this close.
	fn holds(self, places: &[Place]) -> bool {
		let Some(first) = places.first() else { return false };
		match self {
			Self::Statement => places.iter().all(|p| p.unit == first.unit),
			Self::Function => places.iter().all(|p| p.scope == first.scope),
			Self::File => places.iter().all(|p| p.file == first.file),
			Self::Module => match first.module.first() {
				Some(top) => places.iter().all(|p| p.module.first() == Some(top)),
				None => places.iter().all(|p| p.module.is_empty()),
			},
			Self::Crate => true,
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

/// Settings as one table gives them: each may be left to the table above (a module's to its parent module's, then the
/// package's, then the workspace's).
#[derive(Clone, Copy, Default, Debug)]
pub struct Settings {
	pub level: Option<Level>,
	pub elsewhere: Option<Level>,
	pub whitespace: Option<bool>,
	pub proximity: Option<Proximity>,
}

impl Settings {
	/// These, with what they leave unset taken from `outer`.
	pub fn or(self, outer: Self) -> Self {
		Self {
			level: self.level.or(outer.level),
			elsewhere: self.elsewhere.or(outer.elsewhere),
			whitespace: self.whitespace.or(outer.whitespace),
			proximity: self.proximity.or(outer.proximity),
		}
	}
}

/// The settings a literal is checked by, everything decided.
#[derive(Clone, Copy, Debug)]
struct Resolved {
	level: Level,
	/// Already capped at `level`.
	elsewhere: Level,
	whitespace: bool,
	proximity: Proximity,
}

/// The package's settings, and its modules'.
pub struct Policy {
	package: Settings,
	/// Module path and its table, shortest path first, so a longer one is applied after (over) a shorter.
	modules: Vec<(Vec<String>, Settings)>,
}

impl Policy {
	/// `package` is the package's settings over the workspace's; `modules` maps a module path (`"cli::log"`, with or
	/// without `crate::`) to its table.
	pub fn new(package: Settings, modules: Vec<(String, Settings)>) -> Self {
		let mut modules: Vec<(Vec<String>, Settings)> = modules.into_iter().map(|(path, settings)| (module_path_of(&path), settings)).collect();
		modules.sort_by_key(|(path, _)| path.len());
		Self { package, modules }
	}

	/// The settings for a module: its longest matching table's, then shorter ones', then the package's, then the defaults.
	fn resolve(&self, module: &[String]) -> Resolved {
		let mut settings = self.package;
		for (path, table) in &self.modules {
			if module.starts_with(path) {
				settings = table.or(settings);
			}
		}
		let level = settings.level.unwrap_or(Level::Deny);
		Resolved {
			level,
			elsewhere: settings.elsewhere.unwrap_or(Level::Allow).min(level),
			whitespace: settings.whitespace.unwrap_or(false),
			proximity: settings.proximity.unwrap_or(Proximity::Statement),
		}
	}
}

/// `"crate::cli::log"` or `"cli::log"` as the module path `["cli", "log"]`.
fn module_path_of(path: &str) -> Vec<String> {
	path.split("::").filter(|part| !part.is_empty() && *part != "crate").map(str::to_owned).collect()
}

/// A file's module path, from where it is in the package: `src/cli/log.rs` and `src/cli/log/mod.rs` are `cli::log`;
/// `src/lib.rs`, `src/main.rs`, a binary in `src/bin/` and `build.rs` are the crate root.
fn module_of(package_dir: &Path, file: &Path) -> Vec<String> {
	let relative = file.strip_prefix(package_dir).unwrap_or(file);
	let mut parts: Vec<String> = relative.with_extension("").components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
	if parts.first().is_some_and(|p| p == "src") {
		parts.remove(0);
	}
	if parts.first().is_some_and(|p| p == BIN_DIR) || parts.as_slice() == ["lib"] || parts.as_slice() == ["main"] || parts.as_slice() == ["build"] {
		return Vec::new();
	}
	if parts.last().is_some_and(|p| p == "mod") {
		parts.pop();
	}
	parts
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
	Argument,
	Definition,
	Elsewhere,
}

/// Where a literal is: its file, its 1-based line and column, its kind, and what it is in.
pub struct Place {
	pub file: PathBuf,
	pub line: usize,
	pub column: usize,
	pub kind: Kind,
	/// The function, or other item outside one, the literal is in: unique across the package.
	scope: usize,
	/// The statement or branch the literal is in: unique across the package.
	unit: usize,
	/// The module the file is.
	module: Vec<String>,
	/// The settings of that module.
	settings: Resolved,
	/// For a definition, whether code outside its module can name it.
	public: bool,
}

/// A value to report: where it appears, at which level, and whether any of those is an argument.
pub struct Finding {
	pub places: Vec<Place>,
	pub level: Level,
	pub as_argument: bool,
}

impl Finding {
	/// The places that make the value this level, and the rest. For a value passed as an argument: the arguments, and
	/// the other uses in their files. Otherwise all of them.
	pub fn triggering(&self) -> (Vec<&Place>, Vec<&Place>) {
		if !self.as_argument {
			return (self.places.iter().collect(), Vec::new());
		}
		let files: Vec<&Path> = self.places.iter().filter(|p| p.kind == Kind::Argument).map(|p| p.file.as_path()).collect();
		self.places.iter().partition(|p| files.contains(&p.file.as_path()))
	}
}

/// Every value to report in the package's non-test code, sorted by value.
pub fn findings(package_dir: &Path, policy: &Policy) -> Result<BTreeMap<String, Finding>, String> {
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
	Ok(findings_in(package_dir, &parsed, policy))
}

/// Every value to report in these parsed files, sorted by value.
fn findings_in(package_dir: &Path, parsed: &[(PathBuf, syn::File)], policy: &Policy) -> BTreeMap<String, Finding> {
	let messages = MessageParams::of(&parsed.iter().map(|(_, syntax)| syntax).collect::<Vec<_>>());
	let mut seen: BTreeMap<String, Vec<Place>> = BTreeMap::new();
	let mut counters = Counters::default();
	for (file, syntax) in parsed {
		let module = module_of(package_dir, file);
		let settings = policy.resolve(&module);
		if settings.level == Level::Allow {
			// Not checked, so not counted: like a Java package set to "ignore".
			continue;
		}
		let mut finder = Finder {
			file,
			module,
			settings,
			seen: &mut seen,
			in_definition: false,
			public_definition: false,
			messages: &messages,
			owner: None,
			scope: 0,
			unit: 0,
			join_next: false,
			counters: &mut counters,
		};
		finder.in_scope(|f| f.visit_file(syntax));
	}
	seen.into_iter().filter_map(|(value, places)| finding(places).map(|f| (value, f))).collect()
}

/// What to report of a value's literals, if anything: not a value seen once, nor one whose literals are all definitions,
/// nor one whose literals are all in one function (or other item) with at most one of them an argument, nor one whose
/// literals are all close enough to be read together. A value across modules takes the most severe of their levels and
/// the narrowest of their proximities.
fn finding(places: Vec<Place>) -> Option<Finding> {
	// One function's literals are exempt only with at most one argument among them: two (declaring a flag and looking it
	// up, or writing a key and reading it) are the typo the rule is for.
	let arguments_in_all = places.iter().filter(|p| p.kind == Kind::Argument).count();
	let one_scope = places.iter().all(|p| p.scope == places[0].scope) && arguments_in_all <= 1;
	let proximity = places.iter().map(|p| p.settings.proximity).min()?;
	// Literals this close, however many are arguments, are read together: `starts_with("--x")` and `value("--x")` in one
	// match arm. Not with a constant among them, though: a constant plus a literal should use the constant.
	// A constant among them, though, is never close: a constant plus a literal should use the constant.
	let defined = places.iter().any(|p| p.kind == Kind::Definition);
	let close = !defined && proximity.holds(&places);
	if places.len() < 2 || places.iter().all(|p| p.kind == Kind::Definition) || (one_scope && !defined) || close {
		return None;
	}
	let mut arguments = places.iter().filter(|p| p.kind == Kind::Argument);
	let first = arguments.next();
	let lone_argument = match (first, arguments.next()) {
		// Never with a constant the argument could name: the name exists, so the literal is wrong.
		(Some(argument), None) if !usable_constant(&places) => places.iter().all(|p| p.scope == argument.scope || p.file != argument.file),
		_ => false,
	};
	let as_argument = first.is_some();
	let as_argument_level = places.iter().map(|p| p.settings.level).max()?;
	let level = match (as_argument, lone_argument) {
		// A lone argument, likely another concept sharing the spelling, is at most a warning.
		(true, true) => as_argument_level.min(Level::Warn),
		(true, false) => as_argument_level,
		(false, _) => {
			let elsewhere = places.iter().map(|p| p.settings.elsewhere).max()?;
			// Compared or matched in several functions, a value is the same check written again each time (four parsers
			// of "true" | "yes" | "on"): at least a warning, though it is never passed on.
			let mut functions: Vec<usize> = places.iter().filter(|p| p.kind != Kind::Definition).map(|p| p.scope).collect();
			functions.sort_unstable();
			functions.dedup();
			if functions.len() >= SPREAD { elsewhere.max(as_argument_level.min(Level::Warn)) } else { elsewhere }
		}
	};
	(level != Level::Allow).then_some(Finding { places, level, as_argument })
}

/// Whether a constant among these literals can be named where one of them is passed as an argument: it is `pub`, or the
/// argument is in its module or a module inside it. A private constant of another module is no name the literal could
/// use, so it says nothing about whether the two are one concept.
fn usable_constant(places: &[Place]) -> bool {
	let arguments: Vec<&Place> = places.iter().filter(|p| p.kind == Kind::Argument).collect();
	places.iter().filter(|d| d.kind == Kind::Definition).any(|d| d.public || arguments.iter().any(|a| a.module.starts_with(&d.module)))
}

/// Whether an item can be named outside its module.
fn is_public(vis: &syn::Visibility) -> bool {
	!matches!(vis, syn::Visibility::Inherited)
}

/// How many functions a value never passed as an argument must be in to be the same check written again.
const SPREAD: usize = 3;

/// Number sources, so each scope and unit gets its own number across the package.
#[derive(Default)]
struct Counters {
	scopes: usize,
	units: usize,
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
/// `Formatter`'s builders of `Debug` output, which black's debug-strings rule treats as text for people: the name they
/// take, and the field names and entries added to them (`f.debug_struct("Myers").field("minimal", &self.minimal)`).
const DEBUG_BUILDERS: [&str; 5] = ["debug_struct", "debug_tuple", "debug_map", "debug_set", "debug_list"];
/// What the `Debug` builders add: a field's name, a map's key or value, an entry.
const DEBUG_PARTS: [&str; 5] = ["field", "entry", "entries", "key", "value"];

/// Whether a call is a `Debug` builder, or adds to one: its string arguments are labels in debug output.
fn is_debug_output(call: &syn::ExprMethodCall) -> bool {
	let method = call.method.to_string();
	if DEBUG_BUILDERS.contains(&method.as_str()) {
		return true;
	}
	if !DEBUG_PARTS.contains(&method.as_str()) {
		return false;
	}
	// Only on a chain that starts at a builder: `MapBuilder::new().field("type", ..)` builds a value.
	let mut receiver = &*call.receiver;
	while let syn::Expr::MethodCall(inner) = receiver {
		if DEBUG_BUILDERS.contains(&inner.method.to_string().as_str()) {
			return true;
		}
		receiver = &inner.receiver;
	}
	false
}

/// Whether a value has a letter in it, not counting the one after a backslash: `"\\t"` is an escape, spelled out.
fn has_letter(value: &str) -> bool {
	let mut chars = value.chars();
	while let Some(c) = chars.next() {
		if c == '\\' {
			chars.next();
		} else if c.is_alphabetic() {
			return true;
		}
	}
	false
}

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
	/// The module the file is, and its settings.
	module: Vec<String>,
	settings: Resolved,
	seen: &'a mut BTreeMap<String, Vec<Place>>,
	/// Inside a `const` or `static`, where every literal is a definition.
	in_definition: bool,
	/// Whether that constant can be named outside its module (`pub`, `pub(crate)` and the like).
	public_definition: bool,
	/// Which parameters of the package's functions are messages.
	messages: &'a MessageParams,
	/// The type of the `impl` being visited, which `Self` stands for.
	owner: Option<String>,
	/// The function (or other item) being visited.
	scope: usize,
	/// The statement or branch being visited.
	unit: usize,
	/// Just past a branch's head (a match arm's pattern and guard, an `if`'s condition): the next statement, the first of
	/// its body, is in the head's unit; every later one is a unit of its own.
	join_next: bool,
	counters: &'a mut Counters,
}

impl Finder<'_> {
	/// Visit an item in a scope, and a unit, of its own.
	fn in_scope(&mut self, visit: impl FnOnce(&mut Self)) {
		self.counters.scopes += 1;
		let outer = std::mem::replace(&mut self.scope, self.counters.scopes);
		self.in_unit(false, visit);
		self.scope = outer;
	}

	/// Visit a statement, or a branch, in a unit of its own. A branch (`join` true) shares it with its body's first statement.
	fn in_unit(&mut self, join: bool, visit: impl FnOnce(&mut Self)) {
		self.counters.units += 1;
		let outer = (std::mem::replace(&mut self.unit, self.counters.units), std::mem::replace(&mut self.join_next, join));
		visit(self);
		(self.unit, self.join_next) = outer;
	}

	fn record(&mut self, value: String, span: proc_macro2::Span, kind: Kind) {
		if !has_letter(&value) || (!self.settings.whitespace && value.chars().any(char::is_whitespace)) {
			return;
		}
		let kind = if self.in_definition { Kind::Definition } else { kind };
		let start = span.start();
		let place = Place {
			file: self.file.to_owned(),
			line: start.line,
			column: start.column + 1,
			kind,
			scope: self.scope,
			unit: self.unit,
			module: self.module.clone(),
			settings: self.settings,
			public: self.in_definition && self.public_definition,
		};
		self.seen.entry(value).or_default().push(place);
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

	/// Visit a constant or static, `public` if code outside its module can name it.
	fn in_definition(&mut self, public: bool, visit: impl FnOnce(&mut Self)) {
		let outer = (self.in_definition, self.public_definition);
		(self.in_definition, self.public_definition) = (true, public);
		visit(self);
		(self.in_definition, self.public_definition) = outer;
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
		if MESSAGE_METHODS.contains(&call.method.to_string().as_str()) || is_debug_output(call) {
			return;
		}
		for (i, arg) in call.args.iter().enumerate() {
			if !(is_str_literal(arg) && self.messages.method(&call.receiver, &call.method, call.args.len(), i)) {
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

	// Statements and branches, for the "statement" proximity. A branch's head (a match arm's pattern and guard, an `if`'s
	// condition) is one unit with what it leads to: an arm's expression, or the first statement of its block. So
	// `_ if a.starts_with("--x") => value("--x")` is one however the formatter breaks its lines, but two statements in
	// one arm (declaring a flag, then looking it up) are two.
	fn visit_stmt(&mut self, stmt: &'ast syn::Stmt) {
		if std::mem::take(&mut self.join_next) {
			visit::visit_stmt(self, stmt);
		} else {
			self.in_unit(false, |f| visit::visit_stmt(f, stmt));
		}
	}

	fn visit_arm(&mut self, arm: &'ast syn::Arm) {
		self.in_unit(true, |f| visit::visit_arm(f, arm));
	}

	fn visit_expr_if(&mut self, e: &'ast syn::ExprIf) {
		self.in_unit(true, |f| {
			f.visit_expr(&e.cond);
			f.visit_block(&e.then_branch);
		});
		if let Some((_, otherwise)) = &e.else_branch {
			match &**otherwise {
				// `else if` is a branch of its own.
				syn::Expr::If(_) => self.visit_expr(otherwise),
				block => self.in_unit(true, |f| f.visit_expr(block)),
			}
		}
	}

	/// `extern "C"` names an ABI, not a value.
	fn visit_abi(&mut self, _: &'ast syn::Abi) {}

	fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
		self.in_definition(is_public(&item.vis), |f| visit::visit_item_const(f, item));
	}

	fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
		self.in_definition(is_public(&item.vis), |f| visit::visit_item_static(f, item));
	}

	fn visit_impl_item_const(&mut self, item: &'ast syn::ImplItemConst) {
		self.in_definition(is_public(&item.vis), |f| visit::visit_impl_item_const(f, item));
	}

	fn visit_trait_item_const(&mut self, item: &'ast syn::TraitItemConst) {
		self.in_definition(true, |f| visit::visit_trait_item_const(f, item));
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

	/// The findings in these files, each given as its path in the package and its source, by these settings.
	fn check_with(files: &[(&str, &str)], package: Settings, modules: Vec<(&str, Settings)>) -> BTreeMap<String, Finding> {
		let parsed: Vec<(PathBuf, syn::File)> = files.iter().map(|(path, text)| (PathBuf::from(path), syn::parse_file(text).unwrap())).collect();
		let policy = Policy::new(package, modules.into_iter().map(|(path, s)| (path.to_owned(), s)).collect());
		findings_in(Path::new(""), &parsed, &policy)
	}

	/// The findings in these files, by the default settings.
	fn check(files: &[(&str, &str)]) -> BTreeMap<String, Finding> {
		check_with(files, Settings::default(), Vec::new())
	}

	fn at(proximity: Proximity) -> Settings {
		Settings { proximity: Some(proximity), ..Settings::default() }
	}

	#[test]
	fn a_value_passed_from_two_functions_is_reported() {
		let found = check(&[("src/a.rs", "fn put() { set(\"allow-ff\"); }\nfn get() { read(\"allow-ff\"); }")]);
		assert!(found["allow-ff"].as_argument && found["allow-ff"].level == Level::Deny);
	}

	#[test]
	fn literals_in_one_function_are_not() {
		let found = check(&[("src/a.rs", "fn parse(a: &str) { if let Some(v) = a.strip_prefix(\"--abbrev=\") {} match a { \"--abbrev=\" => {}, _ => {} } }")]);
		assert!(found.is_empty());
	}

	#[test]
	fn two_arguments_in_one_function_are_reported() {
		let found = check(&[("src/a.rs", "fn run(rest: &[&str]) {\nlet a = parse(rest, &[(\"allow-http\", 0)]);\nif a.has(\"allow-http\") {}\n}")]);
		assert_eq!(found["allow-http"].level, Level::Deny);
	}

	#[test]
	fn one_statement_is_read_together_however_it_is_wrapped() {
		// A match arm and its guard, broken over lines the way rustfmt does.
		let arm = "fn parse(arg: &str) { match arg {\n_ if arg.starts_with(\"--date\") =>\n{ value(\"--date\"); }\n_ => {} } }";
		assert!(check(&[("src/a.rs", arm)]).is_empty());
		// An `if`'s condition and its block.
		let branch = "fn parse(arg: &str) {\nif arg.starts_with(\"--author\") {\nvalue(\"--author\");\n}\n}";
		assert!(check(&[("src/a.rs", branch)]).is_empty());
		// Two statements are two.
		let two = "fn run() {\nlet a = parse(&[(\"allow-http\", 0)]);\nlet b = a.has(\"allow-http\");\n}";
		assert!(check(&[("src/a.rs", two)]).contains_key("allow-http"));
		// Two statements in one arm are two as well: declaring a flag, then looking it up.
		let arm_of_two = "fn run(cmd: &str) { match cmd { \"log\" => {\nlet a = parse(&[(\"oldest\", 0)]);\nquery(!a.has(\"oldest\"));\n} _ => {} } }";
		assert!(check(&[("src/a.rs", arm_of_two)]).contains_key("oldest"));
	}

	#[test]
	fn proximity_widens_what_is_read_together() {
		let two_functions = [("src/cli/a.rs", "fn put() { set(\"allow-ff\"); }\nfn get() { read(\"allow-ff\"); }")];
		assert!(check_with(&two_functions, at(Proximity::Statement), Vec::new()).contains_key("allow-ff"));
		assert!(check_with(&two_functions, at(Proximity::Function), Vec::new()).contains_key("allow-ff"));
		assert!(check_with(&two_functions, at(Proximity::File), Vec::new()).is_empty());
		let two_files = [("src/cli/a.rs", "fn put() { set(\"allow-ff\"); }"), ("src/cli/b.rs", "fn get() { read(\"allow-ff\"); }")];
		assert!(check_with(&two_files, at(Proximity::File), Vec::new()).contains_key("allow-ff"));
		assert!(check_with(&two_files, at(Proximity::Module), Vec::new()).is_empty());
		let two_modules = [("src/cli.rs", "fn put() { set(\"allow-ff\"); }"), ("src/odb.rs", "fn get() { read(\"allow-ff\"); }")];
		assert!(check_with(&two_modules, at(Proximity::Module), Vec::new()).contains_key("allow-ff"));
		assert!(check_with(&two_modules, at(Proximity::Crate), Vec::new()).is_empty());
		assert!(Proximity::parse("line").is_err() && Proximity::parse("module").is_ok());
	}

	#[test]
	fn a_constant_is_never_close() {
		let found = check_with(&[("src/a.rs", "const KEY: &str = \"allow-ff\"; fn a() { set(\"allow-ff\"); }")], at(Proximity::Crate), Vec::new());
		assert!(found.contains_key("allow-ff"));
	}

	#[test]
	fn a_module_table_covers_its_module_and_those_inside_it() {
		let files =
			[("src/cli/log.rs", "fn a() { set(\"-n\"); }\nfn b() { read(\"-n\"); }"), ("src/odb.rs", "fn c() { set(\"HEAD\"); }\nfn d() { read(\"HEAD\"); }")];
		let warn = Settings { level: Some(Level::Warn), ..Settings::default() };
		let found = check_with(&files, Settings::default(), vec![("cli", warn)]);
		assert_eq!(found["-n"].level, Level::Warn);
		assert_eq!(found["HEAD"].level, Level::Deny);
		// The longer path wins, for what it sets; `crate::` is optional.
		let deny = Settings { level: Some(Level::Deny), ..Settings::default() };
		assert_eq!(check_with(&files, Settings::default(), vec![("cli", warn), ("crate::cli::log", deny)])["-n"].level, Level::Deny);
		// "allow" leaves the module's literals out altogether.
		let allow = Settings { level: Some(Level::Allow), ..Settings::default() };
		assert!(!check_with(&files, Settings::default(), vec![("cli", allow)]).contains_key("-n"));
		// A value across modules takes the most severe level.
		let across = [("src/cli/log.rs", "fn a() { set(\"pack\"); }"), ("src/odb.rs", "fn c() { read(\"pack\"); }")];
		assert_eq!(check_with(&across, Settings::default(), vec![("cli", warn)])["pack"].level, Level::Deny);
	}

	#[test]
	fn the_report_leads_with_what_makes_it_an_error() {
		let files =
			[("src/cli/log.rs", "fn a() { set(\"-n\"); }\nfn b() { read(\"-n\"); }"), ("src/cli/clone.rs", "fn c(a: &str) { match a { \"-n\" => {}, _ => {} } }")];
		let found = check(&files);
		let (shown, rest) = found["-n"].triggering();
		assert_eq!((shown.len(), rest.len()), (2, 1));
	}

	#[test]
	fn arrays_passed_on_stay_arguments() {
		let found = check(&[("src/a.rs", "fn a() { c.args([\"--all\"]); }\nfn b() { d.args(vec![\"--all\"]); }")]);
		assert!(found["--all"].as_argument);
	}

	#[test]
	fn a_wrapped_value_returned_is_not_an_argument() {
		let found = check_with(
			&[("src/a.rs", "fn name() -> Option<&'static str> { Some(\"hard\") }\nfn is(a: &str) -> bool { a == \"hard\" }")],
			Settings { elsewhere: Some(Level::Warn), ..Settings::default() },
			Vec::new(),
		);
		assert!(!found["hard"].as_argument);
		// Passed on, it still is.
		let found = check(&[("src/a.rs", "fn one() { f(Some(\"hard\")); }\nfn two() { g(\"hard\"); }")]);
		assert!(found["hard"].as_argument);
	}

	#[test]
	fn compile_time_macros_are_not_counted() {
		let found = check(&[("src/a.rs", "fn a() { f(env!(\"CARGO_PKG_VERSION\")); }\nfn b() { g(env!(\"CARGO_PKG_VERSION\")); }")]);
		assert!(found.is_empty());
	}

	#[test]
	fn debug_output_is_not_counted() {
		let debug = "fn fmt(&self, f: &mut Formatter) -> Result { f.debug_struct(\"Diff\").field(\"myers\", &self.myers).finish() }";
		let found = check(&[("src/a.rs", debug), ("src/b.rs", "fn x() { set(\"myers\"); }")]);
		assert!(found.is_empty());
		// A builder of a value is not one: these are keys.
		let found = check(&[("src/a.rs", "fn a() { MapBuilder::new().field(\"type\", 1); }\nfn b() { get(\"type\"); }")]);
		assert!(found.contains_key("type"));
	}

	#[test]
	fn an_escape_spelled_out_has_no_letter() {
		let found = check(&[("src/a.rs", "fn a() { f(\"\\\\t\"); }\nfn b() { g(\"\\\\t\"); }")]);
		assert!(found.is_empty());
	}

	#[test]
	fn a_lone_argument_among_other_files_uses_is_a_warning() {
		let found = check(&[
			("src/a.rs", "fn quiet(a: &str) { f(\"-q\"); match a { \"-q\" => {}, _ => {} } }"),
			("src/b.rs", "fn parse(a: &str) { match a { \"-q\" => {}, _ => {} } }"),
		]);
		assert_eq!(found["-q"].level, Level::Warn);
		// Another use in the argument's own file, outside its function, is the same concept as far as anyone can tell.
		let found = check(&[("src/a.rs", "fn one() { f(\"-q\"); }\nfn two(a: &str) { match a { \"-q\" => {}, _ => {} } }")]);
		assert_eq!(found["-q"].level, Level::Deny);
	}

	#[test]
	fn a_label_is_not_counted() {
		let found = check(&[(
			"src/a.rs",
			"fn check(fmt: &str) -> Result<(), Error> { Err(Error::invalid(format!(\"{fmt} is bad\"))) }\nfn a() { check(\"JSON\"); }\nfn b() { check(\"JSON\"); }",
		)]);
		assert!(found.is_empty());
	}

	#[test]
	fn an_unused_parameter_is_not_a_message() {
		let found = check(&[("src/a.rs", "fn set(_key: &str) {}\nfn a() { set(\"allow-ff\"); }\nfn b() { set(\"allow-ff\"); }")]);
		assert!(found["allow-ff"].as_argument);
	}

	#[test]
	fn a_parameter_passed_through_a_call_into_a_log_is_a_value() {
		let found = check(&[(
			"src/a.rs",
			"fn store(item: &str) { println!(\"{}\", to_json(item)); }\nfn a() { store(\"originalEventId\"); }\nfn b() { store(\"originalEventId\"); }",
		)]);
		assert!(found["originalEventId"].as_argument);
	}

	#[test]
	fn module_paths_come_from_files() {
		let module = |file: &str| module_of(Path::new("pkg"), Path::new(file)).join("::");
		assert_eq!(module("pkg/src/lib.rs"), "");
		assert_eq!(module("pkg/src/cli.rs"), "cli");
		assert_eq!(module("pkg/src/cli/mod.rs"), "cli");
		assert_eq!(module("pkg/src/cli/log.rs"), "cli::log");
		assert_eq!(module("pkg/src/bin/tool.rs"), "");
	}
	#[test]
	fn the_same_comparison_in_three_functions_is_a_warning() {
		let parsers = "fn a(v: &str) -> bool { matches!(v, \"yes\" | \"on\") }\nfn b(v: &str) -> bool { v == \"yes\" }\nfn c(v: &str) -> bool { match v { \"yes\" => true, _ => false } }";
		assert_eq!(check(&[("src/a.rs", parsers)])["yes"].level, Level::Warn);
		// In two functions it is still nobody's business but theirs.
		assert!(!check(&[("src/a.rs", "fn a(v: &str) -> bool { v == \"yes\" }\nfn b(v: &str) -> bool { v == \"yes\" }")]).contains_key("yes"));
	}

	#[test]
	fn a_literal_beside_a_constant_keeps_the_full_level() {
		// One argument, its only other use in another file: a warning, but not when that use is the constant.
		let files = [("src/refs.rs", "pub const HEAD: &str = \"HEAD\";"), ("src/log.rs", "fn a() { resolve(\"HEAD\"); }")];
		assert_eq!(check(&files)["HEAD"].level, Level::Deny);
		// A private constant of another module is no name the literal could use: just a lone argument.
		let private = [("src/speech.rs", "const ESPEAK_WAV: &str = \"-w\";"), ("src/awake.rs", "fn a() { run(\"-w\"); }")];
		assert_eq!(check(&private)["-w"].level, Level::Warn);
		// In its own module, or one inside it, it is.
		let inside = [("src/speech.rs", "const WAV: &str = \"-w\";"), ("src/speech/espeak.rs", "fn a() { run(\"-w\"); }")];
		assert_eq!(check(&inside)["-w"].level, Level::Deny);
	}

	#[test]
	fn a_label_needs_a_call_that_reaches_the_package() {
		let note = "struct S;\nimpl S { fn note(&self, what: &str) { println!(\"{what}\"); } }\nfn read(what: &str) { println!(\"{what}\"); }\n";
		// On self, or by its own path: labels.
		let ours = format!(
			"{note}impl S {{ fn a(&self) {{ self.note(\"JSON\"); }}\nfn b(&self) {{ self.note(\"JSON\"); }} }}\nfn c() {{ read(\"x.toml\"); }}\nfn d() {{ crate::read(\"x.toml\"); }}"
		);
		assert!(check(&[("src/a.rs", &ours)]).is_empty());
		// On anything else, or through another crate's path, no telling: values.
		let theirs = format!(
			"{note}fn a(v: &mut Vec<&str>) {{ v.note(\"JSON\"); }}\nfn b(v: &mut Vec<&str>) {{ v.note(\"JSON\"); }}\nfn c() {{ fs::read(\"x.toml\"); }}\nfn d() {{ fs::read(\"x.toml\"); }}"
		);
		let found = check(&[("src/a.rs", &theirs)]);
		assert!(found.contains_key("JSON") && found.contains_key("x.toml"));
	}
}
