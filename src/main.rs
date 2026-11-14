//! `cargo black`: keeps a Rust project on the black standards.
//!
//! `sync` writes the standards into a project; `check` fails if the project has drifted from them, isn't formatted, or
//! has a warning. The standards are compiled in, so a project gets a fixed version of them, recorded in its
//! Cargo.toml, which only changes when it syncs with a newer cargo-black.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use toml_edit::DocumentMut;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const LINTS: &str = include_str!("../standards/lints.toml");
const VSCODE_SETTINGS: &str = include_str!("../standards/vscode-settings.json");

/// Files cargo black owns outright, by their path in a project: sync overwrites them, and check fails if they differ.
const OWNED: [(&str, &str); 3] = [
	("rustfmt.toml", include_str!("../standards/rustfmt.toml")),
	("clippy.toml", include_str!("../standards/clippy.toml")),
	(".editorconfig", include_str!("../standards/editorconfig")),
];

const USAGE: &str = "usage: cargo black <sync|check>
  sync   write the standards into this project: rustfmt.toml, clippy.toml, .editorconfig, the lints in Cargo.toml,
         and .vscode/settings.json if there is none
  check  fail if any of those differ from the standards, if cargo fmt would change anything, or if cargo clippy warns";

fn main() -> ExitCode {
	// Cargo runs `cargo black x` as `cargo-black black x`.
	let args: Vec<String> = std::env::args().skip(1).skip_while(|a| a == "black").collect();
	let result = match args.first().map(String::as_str) {
		Some("sync") => sync(),
		Some("check") => check(),
		Some("--version" | "-V") => {
			println!("cargo-black {VERSION}");
			Ok(true)
		}
		_ => {
			eprintln!("{USAGE}");
			return ExitCode::from(2);
		}
	};
	match result {
		Ok(true) => ExitCode::SUCCESS,
		Ok(false) => ExitCode::FAILURE,
		Err(message) => {
			eprintln!("cargo black: {message}");
			ExitCode::FAILURE
		}
	}
}

/// A file as the standards want it.
struct Wanted {
	path: PathBuf,
	contents: String,
}

impl Wanted {
	fn is_current(&self) -> Result<bool, String> {
		match fs::read_to_string(&self.path) {
			Ok(existing) => Ok(existing == self.contents),
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
			Err(e) => Err(format!("reading {}: {e}", self.path.display())),
		}
	}
}

fn sync() -> Result<bool, String> {
	let workspace = Workspace::find()?;
	for wanted in workspace.wanted_files()? {
		if !wanted.is_current()? {
			fs::write(&wanted.path, &wanted.contents).map_err(|e| format!("writing {}: {e}", wanted.path.display()))?;
			println!("wrote {}", wanted.path.display());
		}
	}
	sync_vscode(&workspace.root)?;
	Ok(true)
}

fn check() -> Result<bool, String> {
	let workspace = Workspace::find()?;
	let mut ok = true;
	if let Some(recorded) = workspace.recorded_version()?
		&& recorded != VERSION
	{
		eprintln!("this project is on black-rust {recorded}, and this cargo-black is {VERSION}:");
		eprintln!("  install {recorded} (cargo install --git https://github.com/Brendan-Black/black-rust --tag v{recorded}),");
		eprintln!("  or move the project to {VERSION} with cargo black sync");
		return Ok(false);
	}
	for wanted in workspace.wanted_files()? {
		if !wanted.is_current()? {
			eprintln!("{} isn't what the standards say: run cargo black sync", wanted.path.display());
			ok = false;
		}
	}
	let manifest = workspace.root.join("Cargo.toml");
	let manifest = manifest.to_str().ok_or("the workspace path isn't UTF-8")?;
	ok &= cargo(&["fmt", "--all", "--check", "--manifest-path", manifest])?;
	ok &= cargo(&["clippy", "--workspace", "--all-targets", "--manifest-path", manifest, "--", "-D", "warnings"])?;
	Ok(ok)
}

/// Runs cargo with the given arguments, letting it print to the terminal, and says whether it succeeded.
fn cargo(args: &[&str]) -> Result<bool, String> {
	let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
	let status = Command::new(&cargo).args(args).status().map_err(|e| format!("running {cargo} {}: {e}", args.join(" ")))?;
	Ok(status.success())
}

/// .vscode/settings.json is the project's, not ours: create it if there is none, otherwise only say what to change.
fn sync_vscode(root: &Path) -> Result<(), String> {
	let path = root.join(".vscode").join("settings.json");
	let wanted: serde_json::Value = serde_json::from_str(VSCODE_SETTINGS).map_err(|e| format!("the bundled VS Code settings: {e}"))?;
	let existing = match fs::read_to_string(&path) {
		Ok(existing) => existing,
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
			fs::create_dir_all(root.join(".vscode")).map_err(|e| format!("creating .vscode: {e}"))?;
			fs::write(&path, VSCODE_SETTINGS).map_err(|e| format!("writing {}: {e}", path.display()))?;
			println!("wrote {}", path.display());
			return Ok(());
		}
		Err(e) => return Err(format!("reading {}: {e}", path.display())),
	};
	let Ok(existing) = serde_json::from_str::<serde_json::Value>(&existing) else {
		// VS Code allows comments and trailing commas, which serde_json doesn't. Say everything instead.
		println!("couldn't read {} as plain JSON; make sure it has these settings:\n{VSCODE_SETTINGS}", path.display());
		return Ok(());
	};
	let mut missing = String::new();
	differences(&wanted, &existing, "", &mut missing);
	if !missing.is_empty() {
		println!("{} needs:{missing}", path.display());
	}
	Ok(())
}

/// Appends a line to `out` for each setting in `wanted` that `existing` lacks or sets differently.
fn differences(wanted: &serde_json::Value, existing: &serde_json::Value, prefix: &str, out: &mut String) {
	let serde_json::Value::Object(wanted) = wanted else { return };
	for (key, value) in wanted {
		let name = if prefix.is_empty() { format!("\"{key}\"") } else { format!("{prefix} > \"{key}\"") };
		match (value, existing.get(key)) {
			(serde_json::Value::Object(_), Some(inner @ serde_json::Value::Object(_))) => differences(value, inner, &name, out),
			(_, Some(found)) if found == value => {}
			_ => {
				out.push_str(&format!("\n  {name}: {value}"));
			}
		}
	}
}

/// The workspace cargo black was run in: its root, and the manifests of its packages.
struct Workspace {
	root: PathBuf,
	members: Vec<PathBuf>,
}

impl Workspace {
	fn find() -> Result<Self, String> {
		let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
		let output =
			Command::new(&cargo).args(["metadata", "--no-deps", "--format-version", "1"]).output().map_err(|e| format!("running {cargo} metadata: {e}"))?;
		if !output.status.success() {
			return Err(format!("{cargo} metadata failed:\n{}", String::from_utf8_lossy(&output.stderr)));
		}
		let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| format!("reading cargo metadata's output: {e}"))?;
		let root = metadata["workspace_root"].as_str().ok_or("cargo metadata gave no workspace_root")?;
		let members =
			metadata["packages"].as_array().ok_or("cargo metadata gave no packages")?.iter().filter_map(|p| p["manifest_path"].as_str().map(PathBuf::from)).collect();
		Ok(Self { root: PathBuf::from(root), members })
	}

	fn root_manifest(&self) -> Result<DocumentMut, String> {
		read_manifest(&self.root.join("Cargo.toml"))
	}

	/// The black-rust version the project last synced with, from `[package.metadata.black]` or `[workspace.metadata.black]`.
	fn recorded_version(&self) -> Result<Option<String>, String> {
		let doc = self.root_manifest()?;
		let section = if doc.contains_key("workspace") { "workspace" } else { "package" };
		Ok(doc.get(section).and_then(|s| s.get("metadata")).and_then(|m| m.get("black")).and_then(|b| b.get("version")).and_then(|v| v.as_str()).map(str::to_owned))
	}

	/// Every file the standards decide, with the contents they want: the owned files, and each manifest's lints.
	fn wanted_files(&self) -> Result<Vec<Wanted>, String> {
		let mut wanted: Vec<Wanted> = OWNED.iter().map(|(path, contents)| Wanted { path: self.root.join(path), contents: (*contents).to_owned() }).collect();
		let root_path = self.root.join("Cargo.toml");
		let doc = self.root_manifest()?;
		if doc.contains_key("workspace") {
			// The lints are declared once, for the workspace; every package, the root's included, inherits them.
			let block = format!("[workspace.metadata.black]\nversion = \"{VERSION}\"\n\n{}", LINTS.replace("[lints.", "[workspace.lints."));
			let mut contents = replace_sections(doc, &["workspace.metadata.black", "workspace.lints"], &block);
			if contents.contains("\n[package]") || contents.starts_with("[package]") {
				contents = replace_sections(parse(&contents, &root_path)?, &["lints"], "[lints]\nworkspace = true\n");
			}
			wanted.push(Wanted { path: root_path.clone(), contents });
			for member in self.members.iter().filter(|m| **m != root_path) {
				let contents = replace_sections(read_manifest(member)?, &["lints"], "[lints]\nworkspace = true\n");
				wanted.push(Wanted { path: member.clone(), contents });
			}
		} else {
			let block = format!("[package.metadata.black]\nversion = \"{VERSION}\"\n\n{LINTS}");
			wanted.push(Wanted { path: root_path, contents: replace_sections(doc, &["package.metadata.black", "lints"], &block) });
		}
		Ok(wanted)
	}
}

fn read_manifest(path: &Path) -> Result<DocumentMut, String> {
	let text = fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
	parse(&text, path)
}

fn parse(text: &str, path: &Path) -> Result<DocumentMut, String> {
	text.parse().map_err(|e| format!("parsing {}: {e}", path.display()))
}

/// Removes each dotted section from the manifest and appends `block` in their place, at the end. Always putting the
/// block last makes the result the same however often it is applied, so check can compare against it.
fn replace_sections(mut doc: DocumentMut, sections: &[&str], block: &str) -> String {
	for section in sections {
		remove(doc.as_table_mut(), &section.split('.').collect::<Vec<_>>());
	}
	format!("{}\n\n{block}", doc.to_string().trim_end())
}

/// Removes the item at `path` below `table`, if there is one.
fn remove(table: &mut dyn toml_edit::TableLike, path: &[&str]) {
	match path {
		[] => {}
		[last] => {
			table.remove(last);
		}
		[first, rest @ ..] => {
			if let Some(inner) = table.get_mut(first).and_then(|item| item.as_table_like_mut()) {
				remove(inner, rest);
			}
		}
	}
}
