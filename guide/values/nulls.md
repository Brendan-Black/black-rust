# Rust: absence

Rust has no null; `Option<T>` is the only absence, and it can't itself be null. So the nulls rule is the type system,
and [no optional in a constructor](https://github.com/Brendan-Black/black/blob/master/values/no-optional-in-constructor.md)
doesn't apply: `fn new(name: String, nickname: Option<String>)` is fine.

The three-state problem does exist as `Option<Option<T>>`. Use it only where all three states mean something (a
patch's "leave as is" / "clear" / "set"), and then prefer an enum that names them.
