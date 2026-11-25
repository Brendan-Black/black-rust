# Rust: expected outcomes

Outcomes callers branch on are an enum, or the `Err` of a `Result` whose error is an enum:

```rust
pub enum ChargeResult {
	Charged(PaymentId),
	Declined { reason: String },
}

match payments.charge(&order)? {
	ChargeResult::Charged(id) => confirm(&order, id),
	ChargeResult::Declined { reason } => notify_customer(&order, &reason),
}
```

The `?` passes infrastructure failures up; the `match` handles the outcomes. Bugs panic (`expect`, `unreachable!`), and
aren't the way to report an outcome. Libraries define their error enums (by hand or with `thiserror`); `anyhow` is
for binaries, at the edge, where nobody matches on the error.

The guide's ["when you don't know,
throw"](https://github.com/Brendan-Black/black/blob/master/philosophy/judgement.md) is an `Err` passed up with `?`:
loud, and nothing guessed. The guess is `unwrap_or_default()`.
