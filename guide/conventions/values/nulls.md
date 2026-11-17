# Rust: absence

Rust has no null; `Option<T>` is the only absence, and it can't itself be null. So the nulls rule is the type system,
and an `Option` parameter is fine: `fn new(name: String, nickname: Option<String>)`.

The three-state problem does exist as `Option<Option<T>>`. Use it only where all three states mean something (a
patch's "leave as is" / "clear" / "set"), and then prefer an enum that names them.
