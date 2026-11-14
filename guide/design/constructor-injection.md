# Rust: inject through constructors

`OrderShipper::new(warehouse, clock)` storing both in private fields. Not a global (`static`, `OnceLock`,
`lazy_static`) that the type reaches for: that's the welded-in dependency of
[untestable-means-fix](https://github.com/Brendan-Black/black/blob/master/testing/untestable-means-fix.md).
