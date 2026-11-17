# Rust: typed strings

A newtype with a private field, so the only way in is the checked one:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Sku(String);

impl FromStr for Sku {
	type Err = Error;

	fn from_str(raw: &str) -> Result<Self, Error> {
		let b = raw.as_bytes();
		let ok = b.len() == 10 && b[..3].iter().all(u8::is_ascii_uppercase) && b[3] == b'-' && b[4..].iter().all(u8::is_ascii_digit);
		if ok { Ok(Sku(raw.to_owned())) } else { Err(Error::NotASku(raw.to_owned())) }
	}
}

impl AsRef<str> for Sku {
	fn as_ref(&self) -> &str {
		&self.0
	}
}

impl fmt::Display for Sku {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		f.write_str(&self.0)
	}
}
```

- **Deserialize through the check:** `#[serde(try_from = "String")]` with a `TryFrom<String>` that calls the parser.
  A derived `Deserialize` on the newtype would skip it.
- A newtype has the same layout as its field: the wrapper costs nothing at run time, only the check does.
- A `TypedString` marker trait is worth it only if something uses it, such as one generic serde or database impl for
  all of them.
