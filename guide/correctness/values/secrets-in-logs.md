# Rust: secrets

A derived `Debug` prints every field, and a value in an error, a panic message, or a `{:?}` log goes with it. Write
`Debug` by hand for types that hold secrets or personal data (`f.write_str("ApiKey(redacted)")`), and don't implement
`Display` on them at all.
