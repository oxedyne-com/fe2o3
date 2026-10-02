// U10 owns this file. A package is supplied by the host and never fetched: in Daimond its origin serves
// the package corpus, mirrored ahead of use, and the engine reads what the host hands it. An archive's
// SHA-256 is the host's to check, since the engine receives files and never the archive; what the engine
// holds to is that a version, once supplied, does not change. The host hands packages over in one of two
// ways: as files in memory, through `supply` (the wasm surface does this), or as directories on disc laid
// out `<dir>/<namespace>/<name>/<version>/`, through `add_dir` (a native host's package path and cache).
// Either way each package appears in the vfs under its own root, `@<namespace>/<name>:<version>`, a
// relative path that no project path can reach, so a package's files and a project's never mix and a
// diagnostic inside a package names its file as Typst does, `@preview/cetz:0.3.4/src/lib.typ:1:8`.
//
// Semantics and wording are Typst 0.15.1's: `typst-syntax/src/package.rs` (specification, manifest,
// version bound) and `typst-eval/src/import.rs` (manifest validation), Apache-2.0. A manifest error is the
// text Typst's serde derive would give, since that text is what a document's author sees.

use crate::eval::lib::data::toml_parse;
use crate::eval::value::Value;
use crate::syntax::lexer::is_ident;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::{
	Component,
	Path,
	PathBuf,
};
use std::sync::{
	Arc,
	RwLock,
};

pub const DEFAULT_NAMESPACE:	&str				= "preview";		// the registry's namespace
pub const MANIFEST:				&str				= "typst.toml";
pub const TYPST_VERSION:		(u32, u32, u32)		= (0, 15, 1);		// the Typst whose language is evaluated

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageSpec {
	pub namespace:	String,		// `preview`, `local`
	pub name:		String,
	pub version:	(u32, u32, u32),
}

impl PackageSpec {
	/// The package's root in the vfs, `@preview/cetz:0.3.4`: relative, so no project path can name it.
	pub fn root(&self) -> PathBuf {
		PathBuf::from(fmt!("@{}", self.namespace)).join(fmt!("{}:{}", self.name, version_text(self.version)))
	}

	/// The package whose vfs root `path` lies under, and the path within it (`src/lib.typ`, empty at the
	/// root itself).
	pub fn of_path(path: &Path) -> Option<(PackageSpec, String)> {
		let mut comps = path.components();
		let ns = match comps.next().and_then(normal) {
			Some(c) if c.starts_with('@')	=> c,
			_								=> return None,
		};
		let named = match comps.next().and_then(normal) {
			Some(c)	=> c,
			None	=> return None,
		};
		let spec = match parse_spec(&fmt!("{}/{}", ns, named)) {
			Ok(s)	=> s,
			Err(_)	=> return None,
		};
		let mut rel = String::new();
		for c in comps {
			match normal(c) {
				Some(s) => {
					if !rel.is_empty() {
						rel.push('/');
					}
					rel.push_str(s);
				}
				None => return None,
			}
		}
		Some((spec, rel))
	}
}

impl fmt::Display for PackageSpec {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "@{}/{}:{}", self.namespace, self.name, version_text(self.version))
	}
}

pub fn version_text(v: (u32, u32, u32)) -> String {
	fmt!("{}.{}.{}", v.0, v.1, v.2)
}

// A path component that is an ordinary UTF-8 name.
fn normal(c: Component<'_>) -> Option<&str> {
	match c {
		Component::Normal(s)	=> s.to_str(),
		_						=> None,
	}
}

/// Parses `@namespace/name:1.2.3` as Typst does, with Typst's messages for each malformed part.
pub fn parse_spec(spec: &str) -> Outcome<PackageSpec> {
	let rest = match spec.strip_prefix('@') {
		Some(r)	=> r,
		None	=> return Err(bad("package specification must start with '@'".to_string())),
	};
	let (namespace, rest) = rest.split_once('/').unwrap_or((rest, ""));
	if namespace.is_empty() {
		return Err(bad("package specification is missing namespace".to_string()));
	}
	if !is_ident(namespace) {
		return Err(bad(fmt!("`{}` is not a valid package namespace", namespace)));
	}
	let (name, version) = rest.split_once(':').unwrap_or((rest, ""));
	if name.is_empty() {
		return Err(bad("package specification is missing name".to_string()));
	}
	if !is_ident(name) {
		return Err(bad(fmt!("`{}` is not a valid package name", name)));
	}
	if version.is_empty() {
		return Err(bad("package specification is missing version".to_string()));
	}
	let version = match parse_version(version) {
		Ok(v)	=> v,
		Err(m)	=> return Err(bad(m)),
	};
	Ok(PackageSpec {
		namespace:	namespace.to_string(),
		name:		name.to_string(),
		version,
	})
}

/// A `major.minor.patch` version, as a specification and a manifest both write it.
pub fn parse_version(s: &str) -> Result<(u32, u32, u32), String> {
	let mut parts = s.split('.');
	let mut next = |which: &str| -> Result<u32, String> {
		match parts.next().filter(|p| !p.is_empty()) {
			Some(p)	=> p.parse::<u32>().map_err(|_| fmt!("`{}` is not a valid {} version", p, which)),
			None	=> Err(fmt!("version number is missing {} version", which)),
		}
	};
	let major = ok!(next("major"));
	let minor = ok!(next("minor"));
	let patch = ok!(next("patch"));
	if let Some(extra) = parts.next() {
		return Err(fmt!("version number has unexpected fourth component: `{}`", extra));
	}
	Ok((major, minor, patch))
}

fn bad(msg: String) -> Error<ErrTag> {
	err!("{}", msg; Input, Invalid)
}

/// A lower bound on the Typst version a package needs: `0.12`, `0.12.1`, or just `1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VersionBound {
	pub major:	u32,
	pub minor:	Option<u32>,
	pub patch:	Option<u32>,
}

impl VersionBound {
	pub fn parse(s: &str) -> Result<Self, String> {
		let mut parts = s.split('.');
		let mut next = |which: &str| -> Result<Option<u32>, String> {
			match parts.next() {
				Some(p)	=> p.parse::<u32>().map(Some).map_err(|_| fmt!("`{}` is not a valid {} version bound", p, which)),
				None	=> Ok(None),
			}
		};
		let major = match ok!(next("major")) {
			Some(m)	=> m,
			None	=> return Err("version bound is missing major version".to_string()),
		};
		let minor = ok!(next("minor"));
		let patch = ok!(next("patch"));
		if let Some(extra) = parts.next() {
			return Err(fmt!("version bound has unexpected fourth component: `{}`", extra));
		}
		Ok(Self { major, minor, patch })
	}

	/// Does `v` reach the bound, comparing only the parts the bound names?
	pub fn admits(&self, v: (u32, u32, u32)) -> bool {
		if v.0 != self.major {
			return v.0 > self.major;
		}
		let minor = match self.minor {
			Some(m)	=> m,
			None	=> return true,
		};
		if v.1 != minor {
			return v.1 > minor;
		}
		match self.patch {
			Some(p)	=> v.2 >= p,
			None	=> true,
		}
	}
}

impl fmt::Display for VersionBound {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		let mut s = fmt!("{}", self.major);
		if let Some(m) = self.minor {
			s.push_str(&fmt!(".{}", m));
		}
		if let Some(p) = self.patch {
			s.push_str(&fmt!(".{}", p));
		}
		write!(f, "{}", s)
	}
}

// Manifest

/// What a package's `typst.toml` says about it, as far as importing it needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
	pub name:		String,
	pub version:	(u32, u32, u32),
	pub entrypoint:	String,
	pub compiler:	Option<VersionBound>,
}

impl Manifest {
	/// Reads a manifest's text. An error is the reason Typst puts inside "package manifest is malformed
	/// (..)".
	pub fn parse(text: &str) -> Result<Self, String> {
		match toml_parse(text) {
			Ok(t)		=> Self::from_table(&t),
			Err((_, m))	=> Err(m),
		}
	}

	/// Reads a manifest from its parsed TOML table. Typst deserialises the manifest with serde, so an
	/// entry of the wrong type is reported in document order, and a missing field after every entry
	/// present has been read.
	pub fn from_table(table: &Value) -> Result<Self, String> {
		let top = match table {
			Value::Dict(d)	=> d,
			other			=> return Err(invalid_type(other, "struct PackageManifest")),
		};
		let mut package = None;
		for (key, v) in top.iter() {
			match key {
				"package"	=> package = Some(ok!(Self::package(v))),
				"template"	=> ok!(template(v)),
				"tool"		=> ok!(tool(v)),
				_			=> (),
			}
		}
		match package {
			Some(m)	=> Ok(m),
			None	=> Err(missing("package")),
		}
	}

	fn package(v: &Value) -> Result<Self, String> {
		let d = match v {
			Value::Dict(d)	=> d,
			other			=> return Err(invalid_type(other, "struct PackageInfo")),
		};
		let mut name		= None;
		let mut version		= None;
		let mut entrypoint	= None;
		let mut compiler	= None;
		for (key, v) in d.iter() {
			match key {
				"name"			=> name = Some(ok!(want_str(v))),
				"version"		=> version = Some(ok!(parse_version(&ok!(want_str(v))))),
				"entrypoint"	=> entrypoint = Some(ok!(want_str(v))),
				"compiler"		=> compiler = Some(ok!(VersionBound::parse(&ok!(want_str(v))))),
				"license" | "description" | "homepage" | "repository" => {
					let _ = ok!(want_str(v));
				}
				"authors" | "keywords" | "categories" | "disciplines" | "exclude" => ok!(want_strs(v)),
				_ => (),
			}
		}
		Ok(Self {
			name:		ok!(name.ok_or_else(|| missing("name"))),
			version:	ok!(version.ok_or_else(|| missing("version"))),
			entrypoint:	ok!(entrypoint.ok_or_else(|| missing("entrypoint"))),
			compiler,
		})
	}

	/// Is this the manifest of `spec`, and can this Typst run it? The error is Typst's own message.
	pub fn validate(&self, spec: &PackageSpec) -> Result<(), String> {
		if self.name != spec.name {
			return Err(fmt!("package manifest contains mismatched name `{}`", self.name));
		}
		if self.version != spec.version {
			return Err(fmt!("package manifest contains mismatched version {}", version_text(self.version)));
		}
		if let Some(needed) = &self.compiler {
			if !needed.admits(TYPST_VERSION) {
				return Err(fmt!("package requires Typst {} or newer (current version is {})",
					needed, version_text(TYPST_VERSION)));
			}
		}
		Ok(())
	}
}

// A `[template]` table: read only so that a malformed one is refused as Typst refuses it.
fn template(v: &Value) -> Result<(), String> {
	let d = match v {
		Value::Dict(d)	=> d,
		other			=> return Err(invalid_type(other, "struct TemplateInfo")),
	};
	let mut path		= false;
	let mut entrypoint	= false;
	for (key, v) in d.iter() {
		match key {
			"path"			=> { let _ = ok!(want_str(v)); path = true; }
			"entrypoint"	=> { let _ = ok!(want_str(v)); entrypoint = true; }
			"thumbnail"		=> { let _ = ok!(want_str(v)); }
			_				=> (),
		}
	}
	if !path {
		return Err(missing("path"));
	}
	if !entrypoint {
		return Err(missing("entrypoint"));
	}
	Ok(())
}

// A `[tool]` table: each entry is a tool's own table.
fn tool(v: &Value) -> Result<(), String> {
	let d = match v {
		Value::Dict(d)	=> d,
		other			=> return Err(invalid_type(other, "struct ToolInfo")),
	};
	for (_, v) in d.iter() {
		if !matches!(v, Value::Dict(_)) {
			return Err(invalid_type(v, "a map"));
		}
	}
	Ok(())
}

fn want_str(v: &Value) -> Result<String, String> {
	match v {
		Value::Str(s)	=> Ok(s.to_string()),
		other			=> Err(invalid_type(other, "a string")),
	}
}

fn want_strs(v: &Value) -> Result<(), String> {
	match v {
		Value::Array(a) => {
			for x in a.iter() {
				let _ = ok!(want_str(x));
			}
			Ok(())
		}
		other => Err(invalid_type(other, "a sequence")),
	}
}

fn missing(field: &str) -> String {
	fmt!("missing field `{}`", field)
}

// serde's "invalid type" message: the value found as serde names it, then what was expected. A TOML
// datetime reaches serde as a map.
fn invalid_type(v: &Value, expected: &str) -> String {
	let found = match v {
		Value::Int(i)		=> fmt!("integer `{}`", i),
		Value::Float(f)		=> fmt!("floating point `{}`", serde_float(*f)),
		Value::Bool(b)		=> fmt!("boolean `{}`", b),
		Value::Str(s)		=> fmt!("string {:?}", s.as_str()),
		Value::Array(_)		=> "sequence".to_string(),
		_					=> "map".to_string(),
	};
	fmt!("invalid type: {}, expected {}", found, expected)
}

// A float as serde's `Unexpected::Float` shows it: Rust's display, given a decimal point when it has none.
fn serde_float(f: f64) -> String {
	let s = fmt!("{}", f);
	if f.is_finite() && !s.contains('.') {
		fmt!("{}.0", s)
	} else {
		s
	}
}

// The store

/// The files a host handed over for one package, by package-relative path (`src/lib.typ`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackageFiles {
	files:	BTreeMap<String, Arc<Vec<u8>>>,
}

impl PackageFiles {
	pub fn len(&self) -> usize { self.files.len() }

	pub fn is_empty(&self) -> bool { self.files.is_empty() }

	pub fn get(&self, rel: &str) -> Option<&Arc<Vec<u8>>> { self.files.get(rel) }

	/// Is `rel` a directory of the package: its root, or a prefix of some file's path?
	pub fn is_dir(&self, rel: &str) -> bool {
		if rel.is_empty() {
			return true;
		}
		let prefix = fmt!("{}/", rel);
		self.files.range(prefix.clone()..).next().map(|(k, _)| k.starts_with(&prefix)).unwrap_or(false)
	}

	pub fn paths(&self) -> impl Iterator<Item = &str> { self.files.keys().map(|k| k.as_str()) }
}

// Every package a host supplied in memory, and the directories on disc searched for any other, in order.
#[derive(Debug)]
struct Store {
	supplied:	BTreeMap<PackageSpec, Arc<PackageFiles>>,
	dirs:		Vec<PathBuf>,
}

static STORE: RwLock<Store> = RwLock::new(Store { supplied: BTreeMap::new(), dirs: Vec::new() });

/// Where one package was found.
#[derive(Clone, Debug)]
pub enum Found {
	Supplied(Arc<PackageFiles>),	// handed over in memory
	Dir(PathBuf),					// its own directory on disc, `<dir>/<namespace>/<name>/<version>`
}

/// Hands a package's files to the engine and returns how many files it holds. Paths are package-relative;
/// a leading `/` means the package root, as it does inside the package. The files serve every later
/// compile, in memory, until withdrawn. A version is immutable: supplying the same files again changes
/// nothing, and supplying different ones is an error until the package is withdrawn.
pub fn supply(spec: &PackageSpec, files: Vec<(String, Vec<u8>)>) -> Outcome<usize> {
	let mut held = PackageFiles::default();
	for (path, bytes) in files {
		let rel = match normalise_rel(&path) {
			Some(r)	=> r,
			None	=> return Err(err!(
				"The file path {:?} supplied for {} does not name a file within the package.", path, spec;
				Input, Invalid)),
		};
		held.files.insert(rel, Arc::new(bytes));
	}
	let n = held.len();
	let mut store = lock_write!(STORE, "While supplying a package");
	if let Some(have) = store.supplied.get(spec) {
		if **have == held {
			return Ok(n);
		}
		return Err(err!(
			"The package {} is already supplied with different files. A version is immutable, so withdraw it \
			before supplying another.", spec;
			Input, Invalid, Exists));
	}
	store.supplied.insert(spec.clone(), Arc::new(held));
	Ok(n)
}

/// Withdraws a package supplied in memory; false when none was held for it.
pub fn withdraw(spec: &PackageSpec) -> Outcome<bool> {
	let mut store = lock_write!(STORE, "While withdrawing a package");
	Ok(store.supplied.remove(spec).is_some())
}

/// Every package supplied in memory, in order.
pub fn supplied() -> Outcome<Vec<PackageSpec>> {
	let store = lock_read!(STORE, "While listing the supplied packages");
	Ok(store.supplied.keys().cloned().collect())
}

/// Adds a directory on disc to search for packages not supplied in memory, laid out as Typst lays out
/// its package path and cache: `<dir>/<namespace>/<name>/<version>/`. Directories are searched in the
/// order added; one already present is not added twice.
pub fn add_dir(dir: PathBuf) -> Outcome<()> {
	let mut store = lock_write!(STORE, "While adding a package directory");
	if !store.dirs.contains(&dir) {
		store.dirs.push(dir);
	}
	Ok(())
}

pub fn dirs() -> Outcome<Vec<PathBuf>> {
	let store = lock_read!(STORE, "While listing the package directories");
	Ok(store.dirs.clone())
}

/// Where the package is, if the host supplied it.
pub fn find(spec: &PackageSpec) -> Option<Found> {
	let store = match STORE.read() {
		Ok(s)	=> s,
		Err(_)	=> return None,
	};
	if let Some(files) = store.supplied.get(spec) {
		return Some(Found::Supplied(files.clone()));
	}
	for dir in &store.dirs {
		let own = dir.join(&spec.namespace).join(&spec.name).join(version_text(spec.version));
		if native_is_dir(&own) {
			return Some(Found::Dir(own));
		}
	}
	None
}

/// Every version of `namespace/name` the host supplied, oldest first.
pub fn versions(namespace: &str, name: &str) -> Vec<(u32, u32, u32)> {
	let mut out = Vec::new();
	if let Ok(store) = STORE.read() {
		for spec in store.supplied.keys() {
			if spec.namespace == namespace && spec.name == name {
				out.push(spec.version);
			}
		}
		for dir in &store.dirs {
			let dir = dir.join(namespace).join(name);
			for v in native_subdirs(&dir) {
				if let Ok(parsed) = parse_version(&v) {
					out.push(parsed);
				}
			}
		}
	}
	out.sort();
	out.dedup();
	out
}

// A supplied path within a package, normalised as a package-relative path. `None` when it would leave the
// package, names the root itself, or holds a backslash.
fn normalise_rel(path: &str) -> Option<String> {
	let mut segs: Vec<&str> = Vec::new();
	for s in path.split('/') {
		match s {
			"" | "."				=> (),
			".."					=> if segs.pop().is_none() { return None; },
			s if s.contains('\\')	=> return None,
			s						=> segs.push(s),
		}
	}
	if segs.is_empty() {
		return None;
	}
	Some(segs.join("/"))
}

// The vfs's view of package paths. A path under a package root is served from the package, wherever the
// host put it, and never from the project's files or the real filesystem.

/// The bytes at a package path, or `None` when `path` is not under a package root.
pub fn read(path: &Path) -> Option<io::Result<Vec<u8>>> {
	let (spec, rel) = PackageSpec::of_path(path)?;
	Some(match find(&spec) {
		Some(Found::Supplied(files)) => match files.get(&rel) {
			Some(b)						=> Ok((**b).clone()),
			None if files.is_dir(&rel)	=> Err(io::Error::new(io::ErrorKind::IsADirectory, "is a directory")),
			None						=> Err(not_found(path)),
		},
		Some(Found::Dir(dir)) => {
			let real = dir.join(&rel);
			if native_is_dir(&real) {
				Err(io::Error::new(io::ErrorKind::IsADirectory, "is a directory"))
			} else {
				native_read(&real)
			}
		}
		None => Err(not_found(path)),
	})
}

/// Is the package path a file? `None` when `path` is not under a package root.
pub fn is_file(path: &Path) -> Option<bool> {
	let (spec, rel) = PackageSpec::of_path(path)?;
	Some(match find(&spec) {
		Some(Found::Supplied(files))	=> files.get(&rel).is_some(),
		Some(Found::Dir(dir))			=> native_is_file(&dir.join(&rel)),
		None							=> false,
	})
}

/// Is the package path a directory, the package root included? `None` when `path` is not under one.
pub fn is_dir(path: &Path) -> Option<bool> {
	let (spec, rel) = PackageSpec::of_path(path)?;
	Some(match find(&spec) {
		Some(Found::Supplied(files))	=> files.is_dir(&rel),
		Some(Found::Dir(dir))			=> native_is_dir(&dir.join(&rel)),
		None							=> false,
	})
}

/// Every file beneath a package path, as vfs paths, or `None` when `path` is not under a package root.
pub fn list(path: &Path) -> Option<Vec<PathBuf>> {
	let (spec, rel) = PackageSpec::of_path(path)?;
	let root = spec.root();
	let mut out = Vec::new();
	match find(&spec) {
		Some(Found::Supplied(files)) => {
			let prefix = if rel.is_empty() { String::new() } else { fmt!("{}/", rel) };
			for p in files.paths() {
				if p.starts_with(&prefix) {
					out.push(root.join(p));
				}
			}
		}
		Some(Found::Dir(dir)) => {
			let mut found = Vec::new();
			native_list(&dir.join(&rel), &mut found, LIST_DEPTH);
			for f in found {
				if let Ok(within) = f.strip_prefix(&dir) {
					out.push(root.join(within));
				}
			}
		}
		None => (),
	}
	out.sort();
	Some(out)
}

/// How a diagnostic names the file at a vfs path: a file in a package kept on disc by its real path, as
/// Typst names it; any other file by its vfs path.
pub fn display(path: &Path) -> String {
	if let Some((spec, rel)) = PackageSpec::of_path(path) {
		if let Some(Found::Dir(dir)) = find(&spec) {
			return dir.join(rel).display().to_string();
		}
	}
	path.display().to_string()
}

fn not_found(path: &Path) -> io::Error {
	io::Error::new(io::ErrorKind::NotFound, fmt!("{} was not supplied", path.display()))
}

// How deep a package directory on disc is listed.
const LIST_DEPTH: u32 = 16;

// The real filesystem, for packages kept on disc. The wasm target has none, so there a package can only
// be supplied in memory.

#[cfg(not(target_arch = "wasm32"))]
fn native_read(path: &Path) -> io::Result<Vec<u8>> {
	std::fs::read(path)
}

#[cfg(target_arch = "wasm32")]
fn native_read(path: &Path) -> io::Result<Vec<u8>> {
	Err(not_found(path))
}

#[cfg(not(target_arch = "wasm32"))]
fn native_is_file(path: &Path) -> bool {
	path.is_file()
}

#[cfg(target_arch = "wasm32")]
fn native_is_file(_path: &Path) -> bool {
	false
}

#[cfg(not(target_arch = "wasm32"))]
fn native_is_dir(path: &Path) -> bool {
	path.is_dir()
}

#[cfg(target_arch = "wasm32")]
fn native_is_dir(_path: &Path) -> bool {
	false
}

#[cfg(not(target_arch = "wasm32"))]
fn native_subdirs(dir: &Path) -> Vec<String> {
	let mut out = Vec::new();
	if let Ok(entries) = std::fs::read_dir(dir) {
		for e in entries.flatten() {
			if e.path().is_dir() {
				if let Some(n) = e.file_name().to_str() {
					out.push(n.to_string());
				}
			}
		}
	}
	out
}

#[cfg(target_arch = "wasm32")]
fn native_subdirs(_dir: &Path) -> Vec<String> {
	Vec::new()
}

#[cfg(not(target_arch = "wasm32"))]
fn native_list(dir: &Path, out: &mut Vec<PathBuf>, depth: u32) {
	if depth == 0 {
		return;
	}
	let entries = match std::fs::read_dir(dir) {
		Ok(e)	=> e,
		Err(_)	=> return,
	};
	for entry in entries.flatten() {
		let path = entry.path();
		if path.is_dir() {
			native_list(&path, out, depth - 1);
		} else if path.is_file() {
			out.push(path);
		}
	}
}

#[cfg(target_arch = "wasm32")]
fn native_list(_dir: &Path, _out: &mut Vec<PathBuf>, _depth: u32) {}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::eval::value::Dict;

	fn table(entries: &[(&str, Value)]) -> Value {
		let mut d = Dict::new();
		for (k, v) in entries {
			d.insert(k, v.clone());
		}
		Value::dict(d)
	}

	fn pkg(entries: &[(&str, Value)]) -> Value {
		table(&[("package", table(entries))])
	}

	#[test]
	fn roots_are_relative_and_round_trip() -> Outcome<()> {
		let spec = res!(parse_spec("@preview/cetz:0.3.4"));
		assert_eq!(spec.root(), PathBuf::from("@preview/cetz:0.3.4"));
		assert!(spec.root().is_relative());
		assert_eq!(fmt!("{}", spec), "@preview/cetz:0.3.4");
		let (back, rel) = res!(PackageSpec::of_path(&spec.root().join("src/lib.typ"))
			.ok_or_else(|| err!("No package in the path."; Missing)));
		assert_eq!(back, spec);
		assert_eq!(rel, "src/lib.typ");
		assert!(PackageSpec::of_path(Path::new("/@preview/cetz:0.3.4/lib.typ")).is_none());
		assert!(PackageSpec::of_path(Path::new("@preview/cetz/lib.typ")).is_none());
		Ok(())
	}

	#[test]
	fn version_bounds_admit_as_typst_does() -> Outcome<()> {
		let b = |s: &str| VersionBound::parse(s);
		let v = (1, 1, 1);
		assert!(res!(b("1").map_err(|m| err!("{}", m; Invalid))).admits(v));
		assert!(res!(b("1.1").map_err(|m| err!("{}", m; Invalid))).admits(v));
		assert!(!res!(b("1.2").map_err(|m| err!("{}", m; Invalid))).admits(v));
		assert!(res!(b("1.0").map_err(|m| err!("{}", m; Invalid))).admits(v));
		assert!(res!(b("0.15.1").map_err(|m| err!("{}", m; Invalid))).admits(TYPST_VERSION));
		assert!(!res!(b("0.15.2").map_err(|m| err!("{}", m; Invalid))).admits(TYPST_VERSION));
		assert!(!res!(b("0.16").map_err(|m| err!("{}", m; Invalid))).admits(TYPST_VERSION));
		assert_eq!(b("abc"), Err("`abc` is not a valid major version bound".to_string()));
		assert_eq!(b(""), Err("`` is not a valid major version bound".to_string()));
		assert_eq!(b("0.15.1.2"), Err("version bound has unexpected fourth component: `2`".to_string()));
		assert_eq!(fmt!("{}", res!(b("0.16").map_err(|m| err!("{}", m; Invalid)))), "0.16");
		Ok(())
	}

	// Each message is the one typst 0.15.1 gives for the same manifest (checked by hand against the
	// oracle; `tests/eval_package.rs` checks them end to end).
	#[test]
	fn manifests_are_read_with_serde_wording() {
		let s = |x: &str| Value::str(x);
		let good = Manifest::from_table(&pkg(&[
			("name", s("demo")), ("version", s("0.1.0")), ("entrypoint", s("lib.typ")),
			("authors", Value::array(vec![s("me")])), ("compiler", s("0.12")),
		]));
		assert_eq!(good.map(|m| (m.name, m.version, m.entrypoint)),
			Ok(("demo".to_string(), (0, 1, 0), "lib.typ".to_string())));
		let cases: Vec<(Value, &str)> = vec![
			(table(&[("name", s("x"))]), "missing field `package`"),
			(table(&[("package", s("x"))]), "invalid type: string \"x\", expected struct PackageInfo"),
			(pkg(&[("name", s("x")), ("version", s("0.1.0"))]), "missing field `entrypoint`"),
			(pkg(&[("name", Value::Int(5)), ("version", s("0.1.0")), ("entrypoint", s("l"))]),
				"invalid type: integer `5`, expected a string"),
			(pkg(&[("version", Value::Float(1.0))]), "invalid type: floating point `1.0`, expected a string"),
			(pkg(&[("version", Value::Bool(true))]), "invalid type: boolean `true`, expected a string"),
			(pkg(&[("version", Value::array(vec![]))]), "invalid type: sequence, expected a string"),
			(pkg(&[("version", s("0.1"))]), "version number is missing patch version"),
			(pkg(&[("authors", s("me"))]), "invalid type: string \"me\", expected a sequence"),
			(pkg(&[("entrypoint", Value::Int(5)), ("name", Value::Int(7))]),
				"invalid type: integer `5`, expected a string"),
			(pkg(&[("compiler", s("abc"))]), "`abc` is not a valid major version bound"),
			(table(&[("package", table(&[("name", s("x")), ("version", s("0.1.0")), ("entrypoint", s("l"))])),
				("tool", table(&[("x", Value::Int(1))]))]), "invalid type: integer `1`, expected a map"),
			(table(&[("package", table(&[("name", s("x")), ("version", s("0.1.0")), ("entrypoint", s("l"))])),
				("template", table(&[("path", s("t"))]))]), "missing field `entrypoint`"),
		];
		for (t, want) in cases {
			assert_eq!(Manifest::from_table(&t).map(|m| m.name), Err(want.to_string()));
		}
	}

	#[test]
	fn validation_uses_typst_messages() -> Outcome<()> {
		let spec = res!(parse_spec("@local/demo:0.1.0"));
		let m = |name: &str, version: (u32, u32, u32), compiler: Option<&str>| Manifest {
			name:		name.to_string(),
			version,
			entrypoint:	"lib.typ".to_string(),
			compiler:	compiler.and_then(|c| VersionBound::parse(c).ok()),
		};
		assert_eq!(m("demo", (0, 1, 0), None).validate(&spec), Ok(()));
		assert_eq!(m("other", (0, 1, 0), None).validate(&spec),
			Err("package manifest contains mismatched name `other`".to_string()));
		assert_eq!(m("demo", (0, 1, 1), None).validate(&spec),
			Err("package manifest contains mismatched version 0.1.1".to_string()));
		assert_eq!(m("demo", (0, 1, 0), Some("0.99.0")).validate(&spec),
			Err("package requires Typst 0.99.0 or newer (current version is 0.15.1)".to_string()));
		assert_eq!(m("demo", (0, 1, 0), Some("0.15")).validate(&spec), Ok(()));
		Ok(())
	}

	#[test]
	fn supplied_files_are_package_relative() {
		assert_eq!(normalise_rel("typst.toml"), Some("typst.toml".to_string()));
		assert_eq!(normalise_rel("/src/./lib.typ"), Some("src/lib.typ".to_string()));
		assert_eq!(normalise_rel("src//a/../lib.typ"), Some("src/lib.typ".to_string()));
		assert_eq!(normalise_rel("../x"), None);
		assert_eq!(normalise_rel("/"), None);
		assert_eq!(normalise_rel("a\\b"), None);
	}
}
