# Rust: a crate per unit

A unit that a second project wants gets its own repository and is referenced by tag:

```toml
[dependencies]
black-zlib = { git = "https://github.com/Brendan-Black/black-zlib", tag = "v0.1.0" }
```

- **A repository per crate, a tag per release.** Name the tag, not a branch: a branch moves under you.
- **A `path` dependency is for working on the two together.** What you release names the tag.
- **Units stack.** `black-hevc` references `black-png`, which references `black-zlib`: deflate was written once.
- **Depend on as little as you can.** Whatever the crate depends on, every project that references it now does.
  `black-zlib` depends on nothing.
