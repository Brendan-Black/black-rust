# black-rust

My Rust standards: the configuration that enforces the [black style guide](https://github.com/Brendan-Black/black) in
a Rust project, `cargo black` to install and check it, and the guide's Rust specifics in [`guide/`](guide/INDEX.md).

## The rules

[`standards/`](standards) holds the canonical files:

- **[`rustfmt.toml`](standards/rustfmt.toml).** Tabs, shown 2 wide (`tab_spaces` is the width rustfmt counts a tab
  as). Lines up to 160. Short things stay on one line up to the full width (`use_small_heuristics = "Max"`), the
  nearest rustfmt gets to the Java profile's one-line blocks.
- **[`lints.toml`](standards/lints.toml).** rustc's and Clippy's defaults, which `check` fails on, plus the guide's
  rules a lint can hold. `unused_must_use` is `forbid`: [don't swallow errors](https://github.com/Brendan-Black/black/blob/master/errors/dont-swallow.md)
  is a correctness rule, and `forbid` is the level no `#[allow]` can lift.
- **[`clippy.toml`](standards/clippy.toml).** Tests may `unwrap()`.
- **[`editorconfig`](standards/editorconfig)** and **[`vscode-settings.json`](standards/vscode-settings.json).** The
  editor's side: tabs 2 wide, rust-analyzer formatting, and rust-analyzer checking with Clippy, so the editor warns
  exactly where the build fails.

## What rustfmt can't do

**Keep your line breaks.** The Java profile never takes away a line break you put in; rustfmt always lays a statement
out again from scratch, by width, and has no setting to keep a break. How a long call is wrapped is rustfmt's choice,
not yours. `#[rustfmt::skip]` on an item is the only way to keep a layout by hand. Options that would help
(`wrap_comments`, `group_imports`) are nightly-only, so they're left out.

## `cargo black`

```sh
cargo install --git https://github.com/Brendan-Black/black-rust --tag v0.2.4   # or: --path ~/github/rust/black-rust
```

| Command | What it does |
|---|---|
| `cargo black sync` | Writes `rustfmt.toml`, `clippy.toml` and `.editorconfig`, and puts the lints in `Cargo.toml` (`[workspace.lints]` in a workspace, with `lints.workspace = true` in each package). Creates `.vscode/settings.json` if there is none, or lists what to change in an existing one. |
| `cargo black check` | Fails if any of those files differ from the standards, if `cargo fmt` would change anything, if `cargo clippy --all-targets` warns, or if a package has a [magic string](#magic-strings). For CI and before a commit. |

The standards are compiled into `cargo black`, and `sync` records its version in `Cargo.toml`
(`[package.metadata.black]` or `[workspace.metadata.black]`). `check` refuses to run with a different version, so a
project keeps a fixed set of rules until it syncs with a newer one.

**The files it writes are its own.** `sync` overwrites `rustfmt.toml`, `clippy.toml`, `.editorconfig` and the lints
table, so a project's own lint settings there are replaced. To relax a lint in one place, use `#[expect(lint, reason
= "...")]` on the item (any lint but the `forbid` ones). To change a rule everywhere, change it here and release.

`sync` puts the lints and the version at the end of `Cargo.toml`, wherever they were before, so its output is the
same however often it runs; that's what `check` compares against.

## Magic strings

`check` reports every string value that appears in two literals of a package's non-test code, and every place it
appears, by the definition in the guide's
[name magic strings](https://github.com/Brendan-Black/black/blob/master/values/magic-strings.md#what-the-build-checks).
Name it, with a `const` or an enum, and the literal appears once. Each package is checked on its own.

Only identical values match: `"orderId"` matches `"orderId"` and nothing else, not `"orderid"` or
`"orderIdList"`. Escapes are resolved first, so `"\x41"` matches `"A"`, and `r"a"` matches `"a"`.

| A repeated value… | Is |
|---|---|
| passed as an argument at least once: `get("id")`, `.args(["--all"])`, `vec!["a"]`, `info!(target: "x", ...)` | an error |
| passed as an argument once, every other use in other files or in that same function, and no constant among them | a warning: most likely another concept that shares the spelling, such as one flag in two commands' parsers |
| in a `const` or `static` and also written as a literal passed as an argument where that constant can be named (it is `pub`, or the literal is in its module or one inside it) | an error: the name exists, so use it |
| only ever in one function, with at most one of its literals an argument (`strip_prefix("--x=")` beside a `"--x=" =>` arm) | not reported: read together, changed together. Two arguments in one function (declaring a flag and looking it up) are still an error |
| all within `magic-strings-proximity` of each other, with no constant among them: in one statement, by default (`_ if arg.starts_with("--author") => value("--author")`) | not reported: read together, however many are arguments |
| never an argument, in three or more functions: the same `==` or match arm written again (`"true" \| "yes" \| "on"` in four parsers) | a warning |
| never an argument, in fewer: `let`, `return`, `==`, match arms | not reported, unless `magic-strings-elsewhere` asks |
| only ever in `const`s and `static`s | not reported |

Rust specifics: a literal is an argument when it is passed straight to a call, or inside `&`, `( )`, `[ ]`, a tuple
or a constructor there (`Some(..)`, `Ok(..)`, a tuple struct or variant). A constructor's literal is otherwise
elsewhere, so `Some("hard")` returned from a function is not an argument. Every literal in a macro call is an argument,
except the message. A function is any `fn`, and a literal outside one belongs to the item it is in. Not counted:
messages (the format string of `format!`, `print!`/`println!`/`eprintln!`, `write!`, `panic!`, `assert!` and its kin,
`anyhow!`/`bail!`/`ensure!` and the `log`/`tracing` macros, and what is formatted into it; the arguments of `expect`,
`expect_err`, `context`, `wrap_err`, `ok_or`, `Err`, and any function of a type whose name ends in `Error`, such as
`Error::invalid` or `io::Error::other`); the literals of the compile-time macros `env!`, `option_env!`, `include!`,
`include_str!`, `include_bytes!`, `concat!` and `stringify!`, which name variables and files or build other literals;
values with no letter (a backslash and the letter after it, `"\\t"`, is an escape spelled out); values with whitespace
in them; the type names, field names and entries of `Debug` output (`f.debug_struct("Diff").field("myers", ..)`, only
on a chain that starts at a `debug_*` builder); attributes and `extern "C"`; files whose opening comment has a
line starting `@generated`; and test code: `tests/`, `benches/`, `examples/`, and anything under `#[test]` or
`#[cfg(test)]`.

Labels aren't counted either: a literal passed straight to a parameter of the package's own functions that only reaches
a message, like `check("JSON", token)` where `check` only puts it in an `Err`. A function matches a call by name and
number of arguments, and only a call that can reach the package's own code: `f(..)`, `Type::f(..)`, `Self::f(..)`, a
path through `crate`, `self`, `super` or one of the package's modules, or `self.f(..)`. `fs::read(..)` and
`v.push(..)` never match, whatever the package calls its own functions. A parameter that is used any other way, or
passed to a function that isn't known to take a message, is a value.

A package changes these in its `Cargo.toml`, a workspace's root sets them in `[workspace.metadata.black]` for every
package that doesn't set its own, and a module's table sets them for part of a package. `sync` keeps them all.

```toml
[package.metadata.black]
magic-strings = "warn"              # values passed as arguments: "deny" (default), "warn" or "allow"
magic-strings-elsewhere = "warn"    # the most a value that is never an argument can be: "allow" (default), "warn" or "deny"
magic-strings-whitespace = true     # also check values with whitespace in them (default false)
magic-strings-proximity = "file"    # how close a value's literals must be to be read together: "statement" (default), "function", "file", "module", "crate"

[package.metadata.black.modules."cli"]  # the module cli and the modules inside it; "cli::log" for one of those
magic-strings = "warn"
magic-strings-proximity = "module"
```

`magic-strings = "allow"` turns the check off for the package; `magic-strings-elsewhere = "warn"` or `"deny"` also
reports values that are never an argument.
Values with whitespace in them (a space, a tab, a newline) are skipped unless `magic-strings-whitespace = true`: keys,
headers and flags have none, so such a value is almost always text for people.

`magic-strings-proximity` says how close a value's literals must all be to be read together, and so not reported, however
many are arguments. A constant among them is never close: a constant plus a literal should use the constant.

| Proximity | Literals are close when they are all in |
|---|---|
| `"statement"` (default) | one statement, or one branch's head and what it leads to: a match arm's pattern and guard with its expression or its block's first statement, an `if`'s condition with its block's first statement. Line breaks don't matter |
| `"function"` | one function, or one item outside a function |
| `"file"` | one file |
| `"module"` | one module below the crate root and the modules inside it (`cli`, `cli::log`), or the root module's own files |
| `"crate"` | the package: nothing is reported |

A module's table, `[package.metadata.black.modules."cli::log"]` (with or without `crate::`), takes the same four settings
for that module and the modules inside it. A file's module comes from its path: `src/cli/log.rs` and
`src/cli/log/mod.rs` are `cli::log`; `src/lib.rs`, `src/main.rs`, `src/bin/*.rs` and `build.rs` are the crate root. The
longest matching table decides each setting it sets; the rest come from the next module up, then the package, then the
workspace. A value whose literals are in modules with different settings takes the most severe level and the
narrowest proximity among them, and a module set to `"allow"` isn't counted at all.

Each finding lists first the places that make it its level: for a value passed as an argument, its arguments and the
other uses in their files. Uses in files with no argument follow on one line, by file. Errors are always listed. Values that only warn
are counted on one line, so the build output stays short; `cargo black check --warnings` lists them too.
