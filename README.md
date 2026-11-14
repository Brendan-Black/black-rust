# black-rust

My Rust standards: the configuration that enforces the [black style guide](https://github.com/Brendan-Black/black) in
a Rust project, `cargo black` to install and check it, and the guide's Rust specifics in [`guide/`](guide/INDEX.md).

## The rules

[`standards/`](standards) holds the canonical files:

- **[`rustfmt.toml`](standards/rustfmt.toml).** Tabs, shown 2 wide (`tab_spaces` is the width rustfmt counts a tab
  as). Lines up to 160. Short things stay on one line up to the full width (`use_small_heuristics = "Max"`), the
  nearest rustfmt gets to the Java profile's one-line blocks.
- **[`lints.toml`](standards/lints.toml).** rustc's and Clippy's defaults, which `check` fails on, plus the guide's
  rules a lint can hold. `unused_must_use` is `forbid`: [don't swallow errors](https://github.com/Brendan-Black/black/blob/master/errors/dont-swallow.md)
  is a correctness rule, and `forbid` is the level no `#[allow]` can lift.
- **[`clippy.toml`](standards/clippy.toml).** Tests may `unwrap()`.
- **[`editorconfig`](standards/editorconfig)** and **[`vscode-settings.json`](standards/vscode-settings.json).** The
  editor's side: tabs 2 wide, rust-analyzer formatting, and rust-analyzer checking with Clippy, so the editor warns
  exactly where the build fails.

## What rustfmt can't do

**Keep your line breaks.** The Java profile never takes away a line break you put in; rustfmt always lays a statement
out again from scratch, by width, and has no setting to keep a break. How a long call is wrapped is rustfmt's choice,
not yours. `#[rustfmt::skip]` on an item is the only way to keep a layout by hand. Options that would help
(`wrap_comments`, `group_imports`) are nightly-only, so they're left out.

## `cargo black`

```sh
cargo install --git https://github.com/Brendan-Black/black-rust --tag v0.1.0   # or: --path ~/github/rust/black-rust
```

| Command | What it does |
|---|---|
| `cargo black sync` | Writes `rustfmt.toml`, `clippy.toml` and `.editorconfig`, and puts the lints in `Cargo.toml` (`[workspace.lints]` in a workspace, with `lints.workspace = true` in each package). Creates `.vscode/settings.json` if there is none, or lists what to change in an existing one. |
| `cargo black check` | Fails if any of those files differ from the standards, if `cargo fmt` would change anything, or if `cargo clippy --all-targets` warns. For CI and before a commit. |

The standards are compiled into `cargo black`, and `sync` records its version in `Cargo.toml`
(`[package.metadata.black]` or `[workspace.metadata.black]`). `check` refuses to run with a different version, so a
project keeps a fixed set of rules until it syncs with a newer one.

**The files it writes are its own.** `sync` overwrites `rustfmt.toml`, `clippy.toml`, `.editorconfig` and the lints
table, so a project's own lint settings there are replaced. To relax a lint in one place, use `#[expect(lint, reason
= "...")]` on the item (any lint but the `forbid` ones). To change a rule everywhere, change it here and release.

`sync` puts the lints and the version at the end of `Cargo.toml`, wherever they were before, so its output is the
same however often it runs; that's what `check` compares against.
