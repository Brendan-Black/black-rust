# Rust: pseudos

```rust
pub trait HttpGateway {
	fn fetch(&self, request: &Request) -> Result<Response, Error>;
}

pub struct PseudoHttpGateway {
	canned: HashMap<Uri, Response>,
	received: RefCell<Vec<Request>>,
}

impl HttpGateway for PseudoHttpGateway {
	fn fetch(&self, request: &Request) -> Result<Response, Error> {
		self.received.borrow_mut().push(request.clone());
		match self.canned.get(&request.uri) {
			Some(response) => Ok(response.clone()),
			None => panic!("no canned response for {}", request.uri),
		}
	}
}

impl PseudoHttpGateway {
	/// What the code under test asked for, in order.
	pub fn received(&self) -> Vec<Request> {
		self.received.borrow().clone()
	}
}
```

The panic is the strictness: an unexpected call is a bug in the test or the code, not an outcome. Use `Mutex` instead
of `RefCell` if the trait needs `Send + Sync`. Canned *failures* are `Err` values in the map, not panics.
