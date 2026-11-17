# Rust: atomic writes

Write to a temporary file in the same directory, `sync_all`, then `fs::rename` it over the original; `tempfile`'s
`NamedTempFile::persist` does the same. `fs::write` on the real path leaves a truncated file if it fails partway.
