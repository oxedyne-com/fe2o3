// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-eval `import.rs`, typst-syntax `path.rs` and typst-library `foundations/path.rs` and `diag.rs`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
// U10 owns this file: paths, `import` and `include`, and reading the files they name. Every path is
// resolved within a root, the project's or a package's, as Typst 0.15.1 resolves it: relative to the
// directory of the file it is written in, or with a leading `/` to that file's root, and never out of the
// root. A module is evaluated once and cached by its vfs path; re-entering one still on `World::route` is
// a cycle. Semantics and wording are ported from typst-syntax `path.rs`, typst-library
// `foundations/path.rs` and `diag.rs`, and typst-eval `import.rs`, all 0.15.1, Apache-2.0.

use crate::diag::{
	message_of,
	Diagnostic,
	DiagnosticKind,
};
use crate::eval::content::Content;
use crate::eval::eval::eval_module;
use crate::eval::lib::foundations::repr;
use crate::eval::package::{
	self,
	Manifest,
	PackageSpec,
	DEFAULT_NAMESPACE,
	MANIFEST,
};
use crate::eval::value::{
	Module,
	Value,
};
use crate::eval::Engine;
use crate::syntax::{
	FileId,
	Span,
};
use crate::vfs;

use oxedyne_fe2o3_core::prelude::*;

use std::io;
use std::path::{
	Path,
	PathBuf,
};
use std::sync::Arc;

/// The root a path lives under.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Root {
	Project,
	Package(PackageSpec),
}

impl Root {
	fn kind(&self) -> &'static str {
		match self {
			Root::Project		=> "project",
			Root::Package(_)	=> "package",
		}
	}
}

/// A path within its root, as Typst's `path` value holds one: absolute within the root and normalised,
/// `/src/lib.typ`, with no empty, `.` or `..` segment.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RootedPath {
	pub root:	Root,
	pub vpath:	String,
}

/// Why a path cannot be formed within its root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathError {
	Escapes,	// a `..` above the root
	Backslash,	// not a separator in Typst, and not portable
}

impl RootedPath {
	/// The rooted path of a file loaded from vfs path `path`: in a package when under a package's root,
	/// else in the project. `None` for a project file outside `project_root`.
	pub fn of(path: &Path, project_root: &Path) -> Option<Self> {
		if let Some((spec, rel)) = PackageSpec::of_path(path) {
			return Some(Self { root: Root::Package(spec), vpath: fmt!("/{}", rel) });
		}
		let rel = match path.strip_prefix(project_root) {
			Ok(r)	=> r,
			Err(_)	=> return None,
		};
		let mut segs = Vec::new();
		for c in rel.components() {
			match c.as_os_str().to_str() {
				Some(s)	=> segs.push(s.to_string()),
				None	=> return None,
			}
		}
		Some(Self { root: Root::Project, vpath: fmt!("/{}", segs.join("/")) })
	}

	/// Resolves `path` as written in this file: against the file's directory, or against the root when it
	/// starts with `/`.
	pub fn resolve(&self, path: &str) -> Result<Self, PathError> {
		let dir = match self.vpath.rfind('/') {
			Some(i)	=> &self.vpath[..i],
			None	=> "",
		};
		let vpath = ok!(join_vpath(dir, path));
		Ok(Self { root: self.root.clone(), vpath })
	}

	/// Where the file is in the vfs: under the project root, or under the package's own root.
	pub fn vfs(&self, project_root: &Path) -> PathBuf {
		let rel = self.vpath.trim_start_matches('/');
		let base = match &self.root {
			Root::Project		=> project_root.to_path_buf(),
			Root::Package(spec)	=> spec.root(),
		};
		if rel.is_empty() { base } else { base.join(rel) }
	}

	/// Typst's `repr` of a `path` value: always the absolute form, and never the package.
	pub fn repr(&self) -> String {
		fmt!("path({})", repr(&Value::str(self.vpath.clone())))
	}
}

// Typst's `VirtualPath::join`: the components of `path` applied in turn to the segments of `base`. A
// leading `/` restarts at the root, empty and `.` segments do nothing, and a `..` with nothing left to
// remove would leave the root.
fn join_vpath(base: &str, path: &str) -> Result<String, PathError> {
	let mut segs: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
	for (i, s) in path.split('/').enumerate() {
		match s {
			"" if i == 0 && !path.is_empty()	=> segs.clear(),
			"" | "."							=> (),
			".."								=> if segs.pop().is_none() { return Err(PathError::Escapes); },
			s if s.contains('\\')				=> return Err(PathError::Backslash),
			s									=> segs.push(s),
		}
	}
	Ok(fmt!("/{}", segs.join("/")))
}

/// The rooted path of the file `from`, which a path written in it resolves against.
pub fn within(engine: &mut Engine, from: FileId, span: Span) -> Outcome<RootedPath> {
	if from == FileId::DETACHED {
		return Err(engine.error(DiagnosticKind::Type, span, "cannot access file system from here"));
	}
	let path = match engine.world.source(from) {
		Some(s)	=> s.path.clone(),
		None	=> return Err(err!("No source with id {} is loaded.", from.0; Missing, Input)),
	};
	match RootedPath::of(&path, &engine.world.root) {
		Some(r)	=> Ok(r),
		None	=> Err(engine.error(DiagnosticKind::Type, span, "source file must be contained in project root")),
	}
}

/// Resolves `path` as written in the file `from`, within that file's root.
pub fn resolve(engine: &mut Engine, path: &str, from: FileId, span: Span) -> Outcome<RootedPath> {
	let base = res!(within(engine, from, span));
	match base.resolve(path) {
		Ok(p)	=> Ok(p),
		Err(e)	=> Err(path_error(engine, span, e, &base.root, path)),
	}
}

/// The vfs path a path written in `from` names.
pub fn resolve_path(engine: &mut Engine, spec: &str, from: FileId, span: Span) -> Outcome<PathBuf> {
	let p = res!(resolve(engine, spec, from, span));
	Ok(p.vfs(&engine.world.root))
}

// Typst's message for a path that cannot be formed, with its hints.
fn path_error(engine: &mut Engine, span: Span, e: PathError, root: &Root, written: &str) -> Error<ErrTag> {
	match e {
		PathError::Escapes => fail(engine, DiagnosticKind::Type, span,
			fmt!("path `{}` would escape the {} root", repr(&Value::str(written)), root.kind()),
			vec![fmt!("cannot access files outside of the {} sandbox", root.kind())]),
		PathError::Backslash => fail(engine, DiagnosticKind::Type, span, "path must not contain a backslash".to_string(), vec![
			fmt!("use forward slashes instead: `{}`", repr(&Value::str(written.replace('\\', "/")))),
			"in earlier Typst versions, backslashes indicated path separators on Windows".to_string(),
			"this behavior is no longer supported as it is not portable".to_string(),
		]),
	}
}

/// Reads the file at a vfs path, failing as Typst does: not found (naming where it looked), a directory,
/// or access denied. The error is a `MissingFile`.
pub fn read_file(engine: &mut Engine, path: &Path, span: Span) -> Outcome<Vec<u8>> {
	read_file_as(engine, DiagnosticKind::MissingFile, path, span)
}

/// Checks that the file at a vfs path can be read, without reading it: what Typst's `image(path)` does at
/// the call, where it loads the file. The error is the one `read_file` gives.
pub fn require_file(engine: &mut Engine, path: &Path, span: Span) -> Outcome<()> {
	if vfs::is_file(path) {
		return Ok(());
	}
	let _ = res!(read_file(engine, path, span));
	Ok(())
}

// As `read_file`, with the kind a failure carries: a manifest that cannot be read is the package's fault,
// not a document's missing file.
fn read_file_as(engine: &mut Engine, kind: DiagnosticKind, path: &Path, span: Span) -> Outcome<Vec<u8>> {
	if vfs::is_dir(path) {
		return Err(fail(engine, kind, span, "failed to load file (is a directory)".to_string(), Vec::new()));
	}
	match vfs::read(path) {
		Ok(b)	=> Ok(b),
		Err(e)	=> Err(fail(engine, kind, span, file_error(path, &e), Vec::new())),
	}
}

/// Typst's message for a file that could not be read.
pub fn file_error(path: &Path, e: &io::Error) -> String {
	match e.kind() {
		io::ErrorKind::NotFound			=> fmt!("file not found (searched at {})", package::display(path)),
		io::ErrorKind::PermissionDenied	=> "failed to load file (access denied)".to_string(),
		io::ErrorKind::IsADirectory		=> "failed to load file (is a directory)".to_string(),
		_								=> fmt!("failed to load file ({})", e),
	}
}

// The source at a vfs path, parsed and added to the world on first use. A leading byte-order mark is
// dropped, as Typst drops it.
fn load_source(engine: &mut Engine, path: &Path, span: Span) -> Outcome<FileId> {
	if let Some(s) = engine.world.sources.iter().find(|s| s.path == path) {
		return Ok(s.id);
	}
	let bytes = res!(read_file(engine, path, span));
	let text = match String::from_utf8(bytes) {
		Ok(t)	=> t,
		Err(_)	=> return Err(fail(engine, DiagnosticKind::Encoding, span, "file is not valid UTF-8".to_string(), Vec::new())),
	};
	let text = match text.strip_prefix('\u{feff}') {
		Some(t)	=> t.to_string(),
		None	=> text,
	};
	engine.world.add_source(path.to_path_buf(), text)
}

/// `import spec`: the evaluated module, from the cache when already evaluated.
pub fn import_module(engine: &mut Engine, spec: &str, from: FileId, span: Span) -> Outcome<Arc<Module>> {
	let mark = engine.diags.len();
	let out = import_spec(engine, spec, from, span);
	if out.is_err() {
		trace(engine, mark, span, fmt!("while importing `{}`", spec));
	}
	out
}

/// `include spec`: the module's content.
pub fn include(engine: &mut Engine, spec: &str, from: FileId, span: Span) -> Outcome<Content> {
	let mark = engine.diags.len();
	match import_spec(engine, spec, from, span) {
		Ok(m)	=> Ok(m.content.clone()),
		Err(e)	=> {
			trace(engine, mark, span, fmt!("while including `{}`", spec));
			Err(e)
		}
	}
}

/// `import path(..)`: the module at a rooted path.
pub fn import_path(engine: &mut Engine, path: &RootedPath, span: Span) -> Outcome<Arc<Module>> {
	let at = path.vfs(&engine.world.root);
	import_file(engine, &at, span)
}

fn import_spec(engine: &mut Engine, spec: &str, from: FileId, span: Span) -> Outcome<Arc<Module>> {
	if spec.starts_with('@') {
		return match package::parse_spec(spec) {
			Ok(p)	=> import_package(engine, &p, span),
			Err(e)	=> Err(engine.error(DiagnosticKind::Type, span, message_of(&e))),
		};
	}
	let path = res!(resolve_path(engine, spec, from, span));
	import_file(engine, &path, span)
}

// A module by its vfs path: from the cache, or loaded and evaluated on the route. Loading comes first, so
// a missing file is reported as missing even when the import would also close a cycle, as in Typst.
fn import_file(engine: &mut Engine, path: &Path, span: Span) -> Outcome<Arc<Module>> {
	if let Some(m) = engine.world.modules.get(path) {
		return Ok(m.clone());
	}
	let id = res!(load_source(engine, path, span));
	if let Some(at) = engine.world.route.iter().position(|p| p == path) {
		// Typst's message is the bare "cyclic import"; the hint names the files that close the cycle.
		let root = engine.world.root.clone();
		let chain: Vec<String> = engine.world.route[at..].iter().map(|p| shown(&root, p))
			.chain(std::iter::once(shown(&root, path))).collect();
		return Err(fail(engine, DiagnosticKind::Type, span, "cyclic import".to_string(),
			vec![fmt!("the import chain is {}", chain.join(" -> "))]));
	}
	engine.world.route.push(path.to_path_buf());
	// A module nests as a call does, so a chain of imports is bounded as Typst bounds its route: by the
	// call depth, never by the stack.
	let out = match engine.enter_call(span) {
		Ok(())	=> {
			let out = eval_module(engine, id);
			engine.exit_call();
			out
		}
		Err(e)	=> Err(e),
	};
	engine.world.route.pop();
	let module = Arc::new(res!(out));
	engine.world.modules.insert(path.to_path_buf(), module.clone());
	Ok(module)
}

// A package's module, named by its manifest, as its entrypoint evaluates. It is cached under the
// package's root, a directory, which no file's module can share.
fn import_package(engine: &mut Engine, spec: &PackageSpec, span: Span) -> Outcome<Arc<Module>> {
	let key = spec.root();
	if let Some(m) = engine.world.modules.get(&key) {
		return Ok(m.clone());
	}
	let (name, entry) = res!(resolve_package(engine, spec, span));
	let module = res!(import_file(engine, &entry, span));
	let named = Arc::new(Module {
		name:		Arc::new(name),
		scope:		module.scope.clone(),
		content:	module.content.clone(),
	});
	engine.world.modules.insert(key, named.clone());
	Ok(named)
}

/// A package's name and the vfs path of its entrypoint, read from its manifest and checked as Typst
/// checks them. Every failure is reported at the import.
pub fn resolve_package(engine: &mut Engine, spec: &PackageSpec, span: Span) -> Outcome<(String, PathBuf)> {
	if package::find(spec).is_none() {
		return Err(not_found(engine, spec, span));
	}
	let manifest = RootedPath { root: Root::Package(spec.clone()), vpath: fmt!("/{}", MANIFEST) };
	let at = manifest.vfs(&engine.world.root);
	let bytes = res!(read_file_as(engine, DiagnosticKind::Package, &at, span));
	let text = match std::str::from_utf8(&bytes) {
		Ok(t)	=> t,
		Err(_)	=> return Err(fail(engine, DiagnosticKind::Package, span, "file is not valid UTF-8".to_string(), Vec::new())),
	};
	let read = match Manifest::parse(text) {
		Ok(m)	=> m,
		Err(m)	=> return Err(fail(engine, DiagnosticKind::Package, span,
			fmt!("package manifest is malformed ({})", m), Vec::new())),
	};
	if let Err(m) = read.validate(spec) {
		return Err(fail(engine, DiagnosticKind::Package, span, m, Vec::new()));
	}
	// The entrypoint is resolved like a path written in the manifest, so it stays within the package.
	let entry = match manifest.resolve(&read.entrypoint) {
		Ok(p)	=> p,
		Err(e)	=> return Err(path_error(engine, span, e, &manifest.root, &read.entrypoint)),
	};
	Ok((read.name, entry.vfs(&engine.world.root)))
}

// A package the host did not supply. Typst says "package found, but version .. does not exist" when its
// registry holds other versions; here the host's store is the registry, and only for its namespace.
fn not_found(engine: &mut Engine, spec: &PackageSpec, span: Span) -> Error<ErrTag> {
	let others: Vec<(u32, u32, u32)> = package::versions(&spec.namespace, &spec.name)
		.into_iter().filter(|v| *v != spec.version).collect();
	let message = match others.last() {
		Some(latest) if spec.namespace == DEFAULT_NAMESPACE => fmt!(
			"package found, but version {} does not exist (latest is {})",
			package::version_text(spec.version), package::version_text(*latest)),
		_ => fmt!("package not found (searched for {})", spec),
	};
	let mut hints = vec!["the host did not supply it, and Austenite never downloads a package".to_string()];
	if !others.is_empty() {
		let listed: Vec<String> = others.iter().map(|v| package::version_text(*v)).collect();
		hints.push(fmt!("the host supplied {} {}", if listed.len() == 1 { "version" } else { "versions" },
			listed.join(", ")));
	}
	fail(engine, DiagnosticKind::Package, span, message, hints)
}

// A file as a diagnostic names it: relative to the project root, or by its package path.
fn shown(root: &Path, path: &Path) -> String {
	match path.strip_prefix(root) {
		Ok(rel)	=> rel.display().to_string(),
		Err(_)	=> package::display(path),
	}
}

// Records an error with its hints at `span` and returns it, as `Engine::error` does for a bare message.
fn fail(engine: &mut Engine, kind: DiagnosticKind, span: Span, message: String, hints: Vec<String>) -> Error<ErrTag> {
	let mut d = Diagnostic::error(kind, span, message.clone());
	for h in hints {
		d = d.with_hint(h);
	}
	engine.diags.push(d);
	err!("{}", message; Input, Invalid)
}

// Typst's import tracepoint: every error the import raised outside its own span, such as one inside the
// imported file, notes the import it came through.
fn trace(engine: &mut Engine, mark: usize, span: Span, text: String) {
	if let Some(recorded) = engine.diags.get_mut(mark..) {
		for d in recorded.iter_mut() {
			let inside = d.span.file == span.file && span.start <= d.span.start && d.span.end <= span.end;
			if d.is_error() && !inside {
				d.trace.push((span, text.clone()));
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn paths_join_as_typst_virtual_paths_do() {
		let j = |b: &str, p: &str| join_vpath(b, p);
		assert_eq!(j("/src", "works.bib"), Ok("/src/works.bib".to_string()));
		assert_eq!(j("/src", ""), Ok("/src".to_string()));
		assert_eq!(j("/src", "."), Ok("/src".to_string()));
		assert_eq!(j("/src", ".."), Ok("/".to_string()));
		assert_eq!(j("/src", "../.."), Err(PathError::Escapes));
		assert_eq!(j("/src", "a\\b"), Err(PathError::Backslash));
		assert_eq!(j("/src", "/a///b/"), Ok("/a/b".to_string()));
		assert_eq!(j("/src", "./x/../y.typ"), Ok("/src/y.typ".to_string()));
		assert_eq!(j("/src", "/../x"), Err(PathError::Escapes));
		assert_eq!(j("", "hello/.../world"), Ok("/hello/.../world".to_string()));
	}

	#[test]
	fn rooted_paths_map_to_the_vfs_and_back() -> Outcome<()> {
		let spec = res!(package::parse_spec("@local/demo:0.1.0"));
		let file = RootedPath { root: Root::Package(spec.clone()), vpath: "/src/lib.typ".to_string() };
		let at = file.vfs(Path::new("/proj"));
		assert_eq!(at, PathBuf::from("@local/demo:0.1.0/src/lib.typ"));
		assert_eq!(RootedPath::of(&at, Path::new("/proj")), Some(file.clone()));
		assert_eq!(file.resolve("/data/d.txt").map(|p| p.vpath), Ok("/data/d.txt".to_string()));
		assert_eq!(file.resolve("../../x.txt"), Err(PathError::Escapes));
		let main = RootedPath { root: Root::Project, vpath: "/ch/a.typ".to_string() };
		assert_eq!(main.vfs(Path::new("/proj")), PathBuf::from("/proj/ch/a.typ"));
		assert_eq!(RootedPath::of(Path::new("/proj/ch/a.typ"), Path::new("/proj")), Some(main));
		assert_eq!(RootedPath::of(Path::new("/elsewhere/a.typ"), Path::new("/proj")), None);
		Ok(())
	}
}
