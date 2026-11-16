//! Which parameters of the package's own functions are messages: used only where text for people goes (a message
//! macro's message, `Err(..)`, an error type's function, `expect` and the other message methods, or another function's
//! message parameter). A literal passed to one is a label for an error, like `check("JSON", ..)`, not a value the code
//! handles. A parameter used anywhere else (compared, looked up, stored, returned) isn't one, and neither is any
//! parameter of a function defined outside the package, whose body can't be read.

use std::collections::{HashMap, HashSet};

use syn::visit::{self, Visit};

use super::{MESSAGE_METHODS, is_lone_string, is_message_call, is_test, item_attrs, macro_arguments};

/// A function as calls name it. `owner` is the type of an associated function without `self` (`Type::f(..)`), and
/// `None` for free functions and for methods, which a call names by `.f(..)` alone.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Callee {
	owner: Option<String>,
	name: String,
	/// Arguments besides `self`.
	arity: usize,
	method: bool,
}

/// For each function the package defines, whether each of its arguments is a message. Functions that calls can't tell
/// apart (same name, same number of arguments) share one entry, which says message only where every one of them does.
#[derive(Default, PartialEq)]
pub struct MessageParams {
	params: HashMap<Callee, Vec<bool>>,
	/// The package's module names (every `mod x`), so `x::f(..)` is known to be the package's own and `fs::read(..)` isn't.
	modules: HashSet<String>,
}

impl MessageParams {
	/// Works it out for these files: no parameter is a message at first, then each pass marks the ones used only in what
	/// is already known to be a message, until nothing changes. Every pass only adds, so it ends. Starting from nothing
	/// keeps a function from vouching for itself: `fn join(&self, name) { self.path.join(name) }` calls a `join`, but
	/// not one known to take a message.
	pub fn of(files: &[&syn::File]) -> Self {
		let mut collector = Collector { functions: Vec::new(), owner: None, modules: HashSet::new() };
		files.iter().for_each(|file| collector.visit_file(file));
		let (functions, modules) = (collector.functions, collector.modules);
		let mut params = combine(&functions, &modules, |f| vec![false; f.params.len()]);
		loop {
			let next = combine(&functions, &modules, |f| {
				let mut uses = Uses {
					names: f.params.iter().flatten().map(String::as_str).collect(),
					params: &params,
					owner: f.owner(),
					in_message: false,
					outside: HashSet::new(),
					seen: HashSet::new(),
				};
				uses.visit_block(f.body);
				f.params.iter().map(|p| p.as_ref().is_some_and(|name| uses.seen.contains(name.as_str()) && !uses.outside.contains(name.as_str()))).collect()
			});
			if next == params {
				return params;
			}
			params = next;
		}
	}

	/// Whether the `index`th argument of a call to `f(..)` or `path::f(..)` is a message. `owner` is the type `Self`
	/// stands for where the call is. Only a call that can reach the package's own code counts: `f(..)`, `Type::f(..)`,
	/// `Self::f(..)`, or a path through `crate`, `self`, `super` or one of the package's modules. `fs::read(..)` names
	/// no function of the package's, however many it has called `read`.
	pub fn call(&self, path: &syn::Path, arity: usize, index: usize, owner: Option<&str>) -> bool {
		let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
		let Some((name, before)) = segments.split_last() else { return false };
		let callee = match before {
			[] => Callee { owner: None, name: name.clone(), arity, method: false },
			[.., qualifier] if qualifier == SELF_TYPE || qualifier.starts_with(char::is_uppercase) => {
				let owner = if qualifier == SELF_TYPE { owner.map(str::to_owned) } else { Some(qualifier.clone()) };
				Callee { owner, name: name.clone(), arity, method: false }
			}
			[first, ..] if PATH_ROOTS.contains(&first.as_str()) || self.modules.contains(first) => Callee { owner: None, name: name.clone(), arity, method: false },
			_ => return false,
		};
		self.params.get(&callee).is_some_and(|flags| flags[index])
	}

	/// Whether the `index`th argument of a call to `receiver.name(..)` is a message. Only on `self`: without types, any
	/// other receiver may be a standard type's, whose `push` or `insert` is not the package's.
	pub fn method(&self, receiver: &syn::Expr, name: &syn::Ident, arity: usize, index: usize) -> bool {
		let on_self = matches!(receiver, syn::Expr::Path(p) if p.path.is_ident(SELF_VALUE));
		on_self && self.params.get(&Callee { owner: None, name: name.to_string(), arity, method: true }).is_some_and(|flags| flags[index])
	}
}

/// The macros that build a string rather than show one.
const STRING_MACROS: [&str; 4] = ["format", "format_args", "write", "writeln"];

/// The names a format string captures: `"{name}"` and `"{name:>8}"`, not `"{0}"` or `"{}"`. An escaped `"{{x"` reads as
/// one too, which only ever makes a parameter count as used.
fn captures(format: &str) -> Vec<String> {
	format
		.split('{')
		.skip(1)
		.filter_map(|after| {
			let name: String = after.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
			name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_').then_some(name)
		})
		.collect()
}

/// The path roots that lead into the package itself.
const PATH_ROOTS: [&str; 3] = ["crate", "self", "super"];
/// What the receiver is called in a method.
const SELF_VALUE: &str = "self";

/// What `Self` is called in a path.
const SELF_TYPE: &str = "Self";

fn combine(functions: &[Function], modules: &HashSet<String>, each: impl Fn(&Function) -> Vec<bool>) -> MessageParams {
	let mut out: HashMap<Callee, Vec<bool>> = HashMap::new();
	for f in functions {
		let flags = each(f);
		match out.get_mut(&f.callee) {
			Some(existing) => existing.iter_mut().zip(&flags).for_each(|(a, b)| *a &= *b),
			None => {
				out.insert(f.callee.clone(), flags);
			}
		}
	}
	MessageParams { params: out, modules: modules.clone() }
}

struct Function<'ast> {
	callee: Callee,
	/// Each parameter's name, or `None` for a pattern (`(a, b): (T, U)`), which is never treated as a message.
	params: Vec<Option<String>>,
	/// The type of the `impl` it is in, which `Self` stands for in its body.
	impl_type: Option<String>,
	body: &'ast syn::Block,
}

impl Function<'_> {
	fn owner(&self) -> Option<&str> {
		self.impl_type.as_deref()
	}
}

/// Every function the package defines that has a body, outside test code.
struct Collector<'ast> {
	functions: Vec<Function<'ast>>,
	/// The type of the `impl` being visited.
	owner: Option<String>,
	/// Every `mod x` the package declares.
	modules: HashSet<String>,
}

impl<'ast> Collector<'ast> {
	fn add(&mut self, sig: &syn::Signature, body: &'ast syn::Block, in_impl: bool) {
		let method = sig.receiver().is_some();
		let params: Vec<Option<String>> = sig
			.inputs
			.iter()
			.filter_map(|input| match input {
				syn::FnArg::Receiver(_) => None,
				syn::FnArg::Typed(typed) => Some(match &*typed.pat {
					syn::Pat::Ident(p) => Some(p.ident.to_string()),
					_ => None,
				}),
			})
			.collect();
		let owner = if in_impl && !method { self.owner.clone() } else { None };
		let impl_type = if in_impl { self.owner.clone() } else { None };
		self.functions.push(Function { callee: Callee { owner, name: sig.ident.to_string(), arity: params.len(), method }, params, impl_type, body });
	}
}

impl<'ast> Visit<'ast> for Collector<'ast> {
	fn visit_item(&mut self, item: &'ast syn::Item) {
		if !is_test(item_attrs(item)) {
			if let syn::Item::Mod(module) = item {
				self.modules.insert(module.ident.to_string());
			}
			visit::visit_item(self, item);
		}
	}

	fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
		self.add(&f.sig, &f.block, false);
		visit::visit_item_fn(self, f);
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

	fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
		if !is_test(&f.attrs) {
			self.add(&f.sig, &f.block, true);
			visit::visit_impl_item_fn(self, f);
		}
	}

	fn visit_trait_item_fn(&mut self, f: &'ast syn::TraitItemFn) {
		if let Some(body) = &f.default {
			self.add(&f.sig, body, false);
		}
		visit::visit_trait_item_fn(self, f);
	}
}

/// The parameters a function's body uses outside a message.
struct Uses<'a> {
	names: HashSet<&'a str>,
	params: &'a MessageParams,
	owner: Option<&'a str>,
	in_message: bool,
	outside: HashSet<String>,
	/// Every parameter used at all: one that never is isn't a message either, since nothing shows what it is for.
	seen: HashSet<String>,
}

impl Uses<'_> {
	fn used(&mut self, name: String) {
		if !self.names.contains(name.as_str()) {
			return;
		}
		if !self.in_message {
			self.outside.insert(name.clone());
		}
		self.seen.insert(name);
	}

	/// Visits an argument, as a message or not. A call inside a message decides afresh for its own arguments: passed to
	/// any function that doesn't take a message (`to_json(item)`), a value is used, whatever is done with the result,
	/// because a log line can carry data.
	fn argument(&mut self, expr: &syn::Expr, message: bool) {
		let outer = self.in_message;
		self.in_message = message;
		self.visit_expr(expr);
		self.in_message = outer;
	}

	/// A macro's tokens outside its message: an identifier there is a use of whatever it names.
	fn tokens(&mut self, tokens: proc_macro2::TokenStream) {
		for token in tokens {
			match token {
				proc_macro2::TokenTree::Ident(ident) => self.used(ident.to_string()),
				proc_macro2::TokenTree::Group(group) => self.tokens(group.stream()),
				proc_macro2::TokenTree::Literal(literal) => {
					if let syn::Lit::Str(s) = syn::Lit::new(literal) {
						captures(&s.value()).into_iter().for_each(|name| self.used(name));
					}
				}
				proc_macro2::TokenTree::Punct(_) => {}
			}
		}
	}
}

impl<'ast> Visit<'ast> for Uses<'_> {
	fn visit_expr_path(&mut self, p: &'ast syn::ExprPath) {
		if p.qself.is_none()
			&& let Some(ident) = p.path.get_ident()
		{
			self.used(ident.to_string());
		}
	}

	fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
		self.visit_expr(&call.func);
		let all = is_message_call(&call.func);
		let path = match &*call.func {
			syn::Expr::Path(p) => Some(&p.path),
			_ => None,
		};
		for (i, arg) in call.args.iter().enumerate() {
			let message = all || path.is_some_and(|path| self.params.call(path, call.args.len(), i, self.owner));
			self.argument(arg, message);
		}
	}

	fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
		self.visit_expr(&call.receiver);
		let all = MESSAGE_METHODS.contains(&call.method.to_string().as_str());
		for (i, arg) in call.args.iter().enumerate() {
			let message = all || self.params.method(&call.receiver, &call.method, call.args.len(), i);
			self.argument(arg, message);
		}
	}

	fn visit_macro(&mut self, mac: &'ast syn::Macro) {
		let (name, arguments, counted) = macro_arguments(mac);
		let builds_string = STRING_MACROS.contains(&name.as_str());
		// Formatted into a message (`Err(format!(..))`), all of it is message. Otherwise `format!` and `write!` build
		// strings that are often values (a path, a key, a tag), so what goes into them is used; and so are any other
		// macro's arguments, but for a message macro's message and what is formatted into it.
		let counted = if builds_string { if self.in_message { 0 } else { arguments.len() } } else { counted };
		let outer = self.in_message;
		for (i, argument) in arguments.into_iter().enumerate() {
			if i < counted || is_lone_string(&argument) {
				// Before the message, a use; the format string itself, a message (its captures, `"{name}"`, included).
				self.in_message = i >= counted;
				self.tokens(argument);
				continue;
			}
			// What is formatted into the message is visited as an expression, so a call on the way decides for itself:
			// `println!("{}", to_json(item))` uses `item`. What isn't one (`key = value` fields) counts as a use.
			match syn::parse2::<syn::Expr>(argument.clone()) {
				Ok(expr) => {
					self.in_message = true;
					<Self as Visit<'_>>::visit_expr(self, &expr);
				}
				Err(_) => {
					self.in_message = false;
					self.tokens(argument);
				}
			}
		}
		self.in_message = outer;
	}

	/// A nested function's body is its own: its parameters are other names.
	fn visit_item(&mut self, _: &'ast syn::Item) {}
}
