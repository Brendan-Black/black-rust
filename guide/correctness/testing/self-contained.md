# Rust: self-contained tests

- **Tests run in parallel by default**, in one process. Anything process-wide is shared: the current directory,
  environment variables (`std::env::set_var` is `unsafe` in edition 2024 for this reason), globals. Pass values in
  instead.
- **No `thread::sleep`** to wait: a channel, a `Barrier`, or a polled condition with a timeout.
- **Folders:** `tempfile::tempdir()`, which is the real thing, brought along.
