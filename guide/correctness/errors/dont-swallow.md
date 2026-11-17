# Rust: don't swallow errors

The swallows Rust allows: `let _ = result;`, `result.ok();`, `if let Ok(x) = result { ... }` with no else,
`.unwrap_or_default()` on an I/O or parse result. Propagate with `?` instead, adding context where the caller can't
see it: `.map_err(|e| Error::Sync { order, warehouse, source: e })`, or `anyhow`'s `.context(...)` in a binary.

A designed fallback is fine; say so, so the lint and the reader both know: `#[expect(clippy::let_underscore_must_use,
reason = "a failed send only means the receiver is gone, which is how shutdown works")]`.

`unused_must_use` is `forbid` (correctness), so an ignored `Result` can't be `#[allow]`ed. Handle it or bind it.
