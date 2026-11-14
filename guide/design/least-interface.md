# Rust: the least interface

Take `&str` not `&String`, `&[T]` not `&Vec<T>`, `impl AsRef<Path>` or `&Path` not `&PathBuf`, `impl IntoIterator`
where any sequence will do. Keep the identity: an operation on user keys takes `&UserKey`, not `&impl HasId`.
