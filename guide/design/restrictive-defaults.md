# Rust: restrictive defaults

Rust starts private and immutable; the rule is to not undo that by reflex.

- **Visibility.** `pub(crate)` for what other modules of the crate need; `pub` only for the crate's API. A `pub`
  field is a promise too: it lets callers build the value without its constructor's checks.
- **Closed to extension.** There's no inheritance; the extension point is a public trait. A public trait callers
  shouldn't implement is sealed (a supertrait in a private module).
- **`#[non_exhaustive]`** keeps a public enum open to new variants, but makes downstream `match`es need a `_` arm, which
  defeats [exhaustive outcomes](../errors/expected-failures.md). Use it on errors you expect to grow, not on outcomes
  callers must handle.
