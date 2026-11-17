# Rust: pass dependencies in

A type that keeps its collaborators takes them in `new` and stores them in private fields:
`OrderShipper::new(warehouse, clock)`. Code that uses a collaborator for one call takes it as a parameter instead:
`fn ship(order: &Order, warehouse: &impl Warehouse)`. Whether a stored collaborator is a generic or a
`Box<dyn Trait>` is [the seams choice](seams.md).

Not a global (`static`, `OnceLock`, `lazy_static`) that the code reaches for: that's the welded-in dependency of
[untestable-means-fix](https://github.com/Brendan-Black/black/blob/master/philosophy/untestable-means-fix.md).
