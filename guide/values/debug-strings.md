# Rust: `Debug` and `Display`

Rust calls neither for you: `{}` asks for `Display`, `{:?}` for `Debug`. That makes the rule simple:

- **`Debug` is the debug output.** Derive it on nearly everything. Never parse it or assert on its text.
- **`Display` is a commitment.** Implement it only for small, context-free wrappers (`Sku`, `CorrelationId`,
  `ErrorCode`), whose output is their canonical form, and for error types, whose output is the message.
