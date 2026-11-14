# Rust: error codes

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
	ShipWrongStatus,
}

impl fmt::Display for ErrorCode {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		f.write_str(match self {
			ErrorCode::ShipWrongStatus => "E-9",
		})
	}
}

log::error!("[{}] cannot ship order {}: status is {:?}, expected Paid", ErrorCode::ShipWrongStatus, order.id, order.status);
```
