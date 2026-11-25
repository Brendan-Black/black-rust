# Rust: elapsed time

`Instant` is the monotonic source; put it behind the clock trait. `SystemTime` is the wall clock, and says so: its
`duration_since` returns an `Err` when the clock has stepped back.
