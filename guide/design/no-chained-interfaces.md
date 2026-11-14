# Rust: supertraits

`trait User: Cacheable` is the chained interface. Keep `trait User` alone and let the cache layer hold
`Cache<UserId, User>`, or implement the cache's trait for your type in the cache module. Supertraits are right where
the trait makes no sense without the other (`trait Eukaryote: Cell`).
