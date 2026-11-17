# Black for Rust

What Rust adds to the [black style guide](https://github.com/Brendan-Black/black) (`~/github/black/INDEX.md`). Each
file here sits at the same path as the rule it adds to; read it alongside that rule, only when the rule is in play.
A rule with no file here applies as written.

| File | Adds |
|---|---|
| [conventions/design/seams](conventions/design/seams.md) | traits; generics vs `dyn` |
| [conventions/design/least-interface](conventions/design/least-interface.md) | `&str`, `&[T]`, `impl AsRef<Path>`; the identity example |
| [conventions/design/no-chained-interfaces](conventions/design/no-chained-interfaces.md) | supertraits |
| [conventions/design/restrictive-defaults](conventions/design/restrictive-defaults.md) | private and immutable already; `pub(crate)`, sealed traits, `#[non_exhaustive]` |
| [conventions/design/constructor-injection](conventions/design/constructor-injection.md) | `new(dep)` or a parameter; no global statics for dependencies |
| [conventions/values/nulls](conventions/values/nulls.md) | no null: `Option` everywhere; `Option<Option<T>>` |
| [conventions/values/magic-strings](conventions/values/magic-strings.md) | enums, `const` in a module |
| [conventions/values/typed-strings](conventions/values/typed-strings.md) | a newtype with a private field, `FromStr`, serde `try_from` |
| [conventions/values/trusted-path](conventions/values/trusted-path.md) | `pub(crate) fn trusted`, not `unsafe` |
| [conventions/values/debug-strings](conventions/values/debug-strings.md) | `Debug` is debug output; `Display` only for small wrappers and errors |
| [correctness/values/secrets-in-logs](correctness/values/secrets-in-logs.md) | derived `Debug` prints every field |
| [correctness/values/no-markup-from-strings](correctness/values/no-markup-from-strings.md) | `format!` is concatenation; `Command::arg` |
| [correctness/errors/dont-swallow](correctness/errors/dont-swallow.md) | `let _ =`, `.ok()`, `if let Ok`; `?` with context |
| [conventions/errors/expected-failures](conventions/errors/expected-failures.md) | enums and `Result`; panics are for bugs |
| [conventions/errors/messages-for-debuggers](conventions/errors/messages-for-debuggers.md) | `expect("why")`, not `unwrap()` |
| [conventions/errors/messages-for-callers](conventions/errors/messages-for-callers.md) | `ErrorCode` enum |
| [correctness/errors/nothing-half-done](correctness/errors/nothing-half-done.md) | write a temporary file, then `fs::rename` |
| [correctness/testing/self-contained](correctness/testing/self-contained.md) | `Instant`, no `thread::sleep`, tests run in parallel, `set_var` |
| [conventions/testing/pseudos](conventions/testing/pseudos.md) | a pseudo behind a trait |

## Tools that hold the line

`cargo black check` (see [the README](../README.md)) runs all of these; the editor shows the same through rust-analyzer
with `check.command = "clippy"`.

| Rule | Tool |
|---|---|
| Layout | rustfmt, with [`rustfmt.toml`](../standards/rustfmt.toml): tabs (2 wide), 160 columns |
| Compiler and Clippy warnings | rustc and Clippy defaults, plus [`lints.toml`](../standards/lints.toml), with `-D warnings` |
| Don't swallow errors (correctness) | `unused_must_use = "forbid"`: an ignored `Result` is an error no `#[allow]` lifts; Clippy's `let_underscore_must_use` |
| Messages for the person debugging | Clippy's `unwrap_used` (tests may unwrap) |
| No null across boundaries | The language: there is no null |
| Exhaustive handling of outcomes | The compiler: `match` on an enum |
| Name magic strings | `cargo black check`: a string value repeated in a package's non-test code is an error if it is ever passed as an argument, and not reported otherwise; `magic-strings`, `magic-strings-elsewhere`, `magic-strings-whitespace` and `magic-strings-proximity` change that ([README](../README.md#magic-strings)) |
