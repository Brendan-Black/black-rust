# Black for Rust

What Rust adds to the [black style guide](https://github.com/Brendan-Black/black) (`~/github/black/INDEX.md`). Each
file here sits at the same path as the rule it adds to; read it alongside that rule, only when the rule is in play.
A rule with no file here applies as written.

| File | Adds |
|---|---|
| [design/seams](design/seams.md) | traits; generics vs `dyn` |
| [design/least-interface](design/least-interface.md) | `&str`, `&[T]`, `impl AsRef<Path>`; the identity example |
| [design/no-chained-interfaces](design/no-chained-interfaces.md) | supertraits |
| [design/restrictive-defaults](design/restrictive-defaults.md) | private and immutable already; `pub(crate)`, sealed traits, `#[non_exhaustive]` |
| [design/constructor-injection](design/constructor-injection.md) | `new(dep)`, no global statics for dependencies |
| [values/nulls](values/nulls.md) | no null: `Option` everywhere, so the optional-in-constructor rule doesn't apply; `Option<Option<T>>` |
| [values/magic-strings](values/magic-strings.md) | enums, `const` in a module |
| [values/typed-strings](values/typed-strings.md) | a newtype with a private field, `FromStr`, serde `try_from` |
| [values/trusted-path](values/trusted-path.md) | `pub(crate) fn trusted`, not `unsafe` |
| [values/debug-strings](values/debug-strings.md) | `Debug` is debug output; `Display` only for small wrappers and errors |
| [values/secrets-in-logs](values/secrets-in-logs.md) | derived `Debug` prints every field |
| [values/no-markup-from-strings](values/no-markup-from-strings.md) | `format!` is concatenation; `Command::arg` |
| [errors/dont-swallow](errors/dont-swallow.md) | `let _ =`, `.ok()`, `if let Ok`; `?` with context |
| [errors/expected-failures](errors/expected-failures.md) | enums and `Result`; panics are for bugs |
| [errors/messages-for-debuggers](errors/messages-for-debuggers.md) | `expect("why")`, not `unwrap()` |
| [errors/messages-for-callers](errors/messages-for-callers.md) | `ErrorCode` enum |
| [errors/nothing-half-done](errors/nothing-half-done.md) | write a temporary file, then `fs::rename` |
| [testing/self-contained](testing/self-contained.md) | `Instant`, no `thread::sleep`, tests run in parallel, `set_var` |
| [testing/pseudos](testing/pseudos.md) | a pseudo behind a trait |

## Tools that hold the line

`cargo black check` (see [the README](../README.md)) runs all of these; the editor shows the same through rust-analyzer
with `check.command = "clippy"`.

| Rule | Tool |
|---|---|
| Layout | rustfmt, with [`rustfmt.toml`](../standards/rustfmt.toml): tabs (2 wide), 160 columns |
| Compiler and Clippy warnings | rustc and Clippy defaults, plus [`lints.toml`](../standards/lints.toml), with `-D warnings` |
| Don't swallow errors (correctness) | `unused_must_use = "forbid"`: an ignored `Result` is an error no `#[allow]` lifts; Clippy's `let_underscore_must_use` |
| Messages for the person debugging | Clippy's `unwrap_used` (tests may unwrap) |
| No null across boundaries; no nullable optional | The language: there is no null |
| Exhaustive handling of outcomes | The compiler: `match` on an enum |
| Name magic strings | `cargo black check`: a string value repeated in a package's non-test code is an error if it is ever passed as an argument, and not reported otherwise; `magic-strings`, `magic-strings-elsewhere`, `magic-strings-whitespace` and `magic-strings-proximity` change that ([README](../README.md#magic-strings)) |
