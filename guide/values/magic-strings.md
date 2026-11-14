# Rust: magic strings

Closed set: an enum (`enum OrderStatus { Pending, Paid, Shipped, Cancelled }`), with serde's `rename_all` if it has a
wire form. Open set: `const`s in a module named for the concept:

```rust
pub mod headers {
	pub const CORRELATION_ID: &str = "X-Correlation-Id";
	pub const TENANT: &str = "X-Tenant";
}
```
