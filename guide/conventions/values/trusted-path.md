# Rust: the trusted path

`pub(crate) fn trusted(raw: String) -> Self` beside `FromStr`. Don't mark it `unsafe`: `unsafe` means memory safety
depends on the caller, and an unchecked SKU breaks no memory. Don't call it `new_unchecked` either, unless the
codebase already does; `trusted` says what the caller is vouching for.
