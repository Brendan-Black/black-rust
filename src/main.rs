//! `cargo black`: keeps a Rust project on the black standards.
//!
//! `sync` writes the standards into a project; `check` fails if the project has drifted from them, isn't formatted, or
//! has a warning. The standards are compiled in, so a project gets a fixed version of them, recorded in its
//! Cargo.toml, which only changes when it syncs with a newer cargo-black.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use toml_edit::DocumentMut;

mod magic_strings;

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
  check  fail if any of those differ from the standards, if cargo fmt would change anything, if cargo clippy warns,
         or if a string literal's value appears twice in a package's non-test code (magic-strings)";

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
		Ok(read_if_there(&self.path)?.is_some_and(|existing| existing == self.contents))
	}
}

/// The file's contents, or None if there is no such file.
fn read_if_there(path: &Path) -> Result<Option<String>, String> {
	match fs::read_to_string(path) {
		Ok(text) => Ok(Some(text)),
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
		Err(e) => Err(format!("reading {}: {e}", path.display())),
	}
}

/// The file's contents; a missing file is an error.
fn read(path: &Path) -> Result<String, String> {
	read_if_there(path)?.ok_or_else(|| format!("{} doesn't exist", path.display()))
}

/// Writes the file and says so.
fn write(path: &Path, contents: &str) -> Result<(), String> {
	fs::write(path, contents).map_err(|e| format!("writing {}: {e}", path.display()))?;
	println!("wrote {}", path.display());
	Ok(())
}

fn sync() -> Result<bool, String> {
	let workspace = Workspace::find()?;
	for wanted in workspace.wanted_files()? {
		if !wanted.is_current()? {
			write(&wanted.path, &wanted.contents)?;
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
	let manifest = workspace.root.join(MANIFEST);
	let manifest = manifest.to_str().ok_or("the workspace path isn't UTF-8")?;
	ok &= cargo(manifest, &["fmt", "--all", "--check"])?;
	ok &= cargo(manifest, &["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])?;
	ok &= check_magic_strings(&workspace)?;
	Ok(ok)
}

const WARNING_LABEL: &str = "warning";
const ERROR_LABEL: &str = "error";

/// Reports every value repeated in a package's string literals, as an error or a warning by the package's settings, and
/// says whether there were no errors.
fn check_magic_strings(workspace: &Workspace) -> Result<bool, String> {
	use magic_strings::{ELSEWHERE_SETTING, Kind, Level, SETTING, WHITESPACE_SETTING};
	let root = workspace.root_manifest()?;
	let mut ok = true;
	for member in &workspace.members {
		let manifest = read_manifest(member)?;
		// The package's own setting, else the workspace's, else the default.
		let level_of = |key: &str, default: Level| -> Result<Level, String> {
			let value = match black_setting(&manifest, PACKAGE_BLACK, key)? {
				Some(value) => Some(value),
				None => black_setting(&root, WORKSPACE_BLACK, key)?,
			};
			value.map_or(Ok(default), |v| Level::parse(key, &v).map_err(|e| format!("{}: {e}", member.display())))
		};
		let as_argument = level_of(SETTING, Level::Deny)?;
		let elsewhere = as_argument.min(level_of(ELSEWHERE_SETTING, Level::Allow)?);
		let whitespace = match black_item(&manifest, PACKAGE_BLACK, WHITESPACE_SETTING).or_else(|| black_item(&root, WORKSPACE_BLACK, WHITESPACE_SETTING)) {
			None => false,
			Some(item) => item.as_bool().ok_or_else(|| format!("{}: {WHITESPACE_SETTING} isn't true or false", member.display()))?,
		};
		if as_argument == Level::Allow {
			continue;
		}
		let dir = member.parent().ok_or("a package manifest has no directory")?;
		for (value, finding) in magic_strings::findings(dir, whitespace)? {
			let level = if finding.as_argument { as_argument } else { elsewhere };
			let label = match level {
				Level::Allow => continue,
				Level::Warn => WARNING_LABEL,
				Level::Deny => ERROR_LABEL,
			};
			let arguments = finding.places.iter().filter(|p| p.kind == Kind::Argument).count();
			eprintln!("{label}: magic string {value:?} appears {} times, {arguments} as an argument: name it (black values/magic-strings)", finding.places.len());
			for place in &finding.places {
				let file = place.file.strip_prefix(&workspace.root).unwrap_or(&place.file);
				let kind = match place.kind {
					Kind::Argument => "argument",
					Kind::Definition => "definition",
					Kind::Elsewhere => "elsewhere",
				};
				eprintln!("  {}:{}:{} ({kind})", file.display(), place.line, place.column);
			}
			ok &= level != Level::Deny;
		}
	}
	Ok(ok)
}

/// The cargo that ran cargo black, or the one on the PATH.
fn cargo_program() -> String {
	std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned())
}

/// Runs a cargo subcommand (`args[0]`) on the manifest, letting it print to the terminal, and says whether it succeeded.
fn cargo(manifest: &str, args: &[&str]) -> Result<bool, String> {
	let cargo = cargo_program();
	let (subcommand, rest) = args.split_first().ok_or("no cargo subcommand")?;
	let status = Command::new(&cargo)
		.arg(subcommand)
		.args(["--manifest-path", manifest])
		.args(rest)
		.status()
		.map_err(|e| format!("running {cargo} {}: {e}", args.join(" ")))?;
	Ok(status.success())
}

/// .vscode/settings.json is the project's, not ours: create it if there is none, otherwise only say what to change.
fn sync_vscode(root: &Path) -> Result<(), String> {
	let dir = root.join(".vscode");
	let path = dir.join("settings.json");
	let wanted: serde_json::Value = serde_json::from_str(VSCODE_SETTINGS).map_err(|e| format!("the bundled VS Code settings: {e}"))?;
	let Some(existing) = read_if_there(&path)? else {
		fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
		return write(&path, VSCODE_SETTINGS);
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
		let cargo = cargo_program();
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
		read_manifest(&self.root.join(MANIFEST))
	}

	/// The black-rust version the project last synced with, from `[package.metadata.black]` or `[workspace.metadata.black]`.
	fn recorded_version(&self) -> Result<Option<String>, String> {
		let doc = self.root_manifest()?;
		black_setting(&doc, owner(&doc), "version")
	}

	/// Every file the standards decide, with the contents they want: the owned files, and each manifest's lints.
	fn wanted_files(&self) -> Result<Vec<Wanted>, String> {
		let mut wanted: Vec<Wanted> = OWNED.iter().map(|(path, contents)| Wanted { path: self.root.join(path), contents: (*contents).to_owned() }).collect();
		let root_path = self.root.join(MANIFEST);
		let doc = self.root_manifest()?;
		let owner = owner(&doc);
		// The project's own magic-strings settings live in the table sync rewrites, so they are carried over, as written.
		let mut level = String::new();
		for key in [magic_strings::SETTING, magic_strings::ELSEWHERE_SETTING, magic_strings::WHITESPACE_SETTING] {
			if let Some(value) = black_item(&doc, owner, key).and_then(toml_edit::Item::as_value) {
				level.push_str(&format!("{key} = {}\n", value.to_string().trim()));
			}
		}
		if owner == WORKSPACE_BLACK {
			// The lints are declared once, for the workspace; every package, the root's included, inherits them.
			let block = format!("[{WORKSPACE_BLACK}]\nversion = \"{VERSION}\"\n{level}\n{}", LINTS.replace("[lints.", "[workspace.lints."));
			let mut contents = replace_sections(doc, &[WORKSPACE_BLACK, "workspace.lints"], &block);
			if contents.contains("\n[package]") || contents.starts_with("[package]") {
				contents = replace_sections(parse(&contents, &root_path)?, &[LINTS_TABLE], INHERIT_LINTS);
			}
			wanted.push(Wanted { path: root_path.clone(), contents });
			for member in self.members.iter().filter(|m| **m != root_path) {
				let contents = replace_sections(read_manifest(member)?, &[LINTS_TABLE], INHERIT_LINTS);
				wanted.push(Wanted { path: member.clone(), contents });
			}
		} else {
			let block = format!("[{PACKAGE_BLACK}]\nversion = \"{VERSION}\"\n{level}\n{LINTS}");
			wanted.push(Wanted { path: root_path, contents: replace_sections(doc, &[PACKAGE_BLACK, LINTS_TABLE], &block) });
		}
		Ok(wanted)
	}
}

/// A package's manifest, by its file name.
pub(crate) const MANIFEST: &str = "Cargo.toml";
const WORKSPACE_BLACK: &str = "workspace.metadata.black";
const PACKAGE_BLACK: &str = "package.metadata.black";
const LINTS_TABLE: &str = "lints";
/// A package's lints, in a workspace: the workspace's.
const INHERIT_LINTS: &str = "[lints]\nworkspace = true\n";

/// Which table holds the project's black settings: the workspace's in a workspace's root manifest, the package's otherwise.
fn owner(doc: &DocumentMut) -> &'static str {
	if doc.contains_key("workspace") { WORKSPACE_BLACK } else { PACKAGE_BLACK }
}

/// `key` in the dotted `table`, if it is there. Anything but a string is an error, not a silent default.
fn black_setting(doc: &DocumentMut, table: &str, key: &str) -> Result<Option<String>, String> {
	let Some(item) = black_item(doc, table, key) else { return Ok(None) };
	item.as_str().map(|v| Some(v.to_owned())).ok_or_else(|| format!("[{table}] {key} isn't a string"))
}

/// `key` in the dotted `table`, if it is there, whatever its type.
fn black_item<'a>(doc: &'a DocumentMut, table: &str, key: &str) -> Option<&'a toml_edit::Item> {
	table.split('.').chain([key]).try_fold(doc.as_item(), |item, part| item.get(part))
}

fn read_manifest(path: &Path) -> Result<DocumentMut, String> {
	parse(&read(path)?, path)
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
