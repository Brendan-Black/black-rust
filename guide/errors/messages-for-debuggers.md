# Rust: panic messages

`expect` says why the value can't be absent: `.expect("the header was validated when the file was opened")`. A
bare `unwrap()` panics with "called `Option::unwrap()` on a `None` value", which says nothing; Clippy's `unwrap_used`
warns on it outside tests.
