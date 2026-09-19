//! The one seam through which the assembler reads a document's files.
//!
//! A book compile follows a Typst root's `#include` chain and reads its `config.typ`, `terms.typ`,
//! bibliography and figure assets. On a native build those come off the real filesystem; a
//! `wasm32-unknown-unknown` build has no filesystem, so its files are injected here as a `path -> bytes`
//! map before the same assembler runs. Every library file read routes through this module, so the
//! assembler is written once and reads either source without a `#cfg` at each call.
//!
//! The contract that keeps a native build byte-for-byte what it was: with no map installed -- the native
//! default -- a read is exactly the `std::fs` call it replaced, and every predicate (`exists`, `is_file`,
//! `canonicalize`) falls through to the real filesystem unchanged. A map is installed only by the wasm
//! surface ([`crate::wasm`]), and only there does a read resolve from injected bytes. The global mirrors
//! the crate's other assembly-time singletons -- the image base directory and the term dictionary --
//! since the reader sets one file at a time and threads no source map of its own.

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;
use std::io;
use std::path::{
	Component,
	Path,
	PathBuf,
};
use std::sync::RwLock;

// The injected source map. `None` on the native default path, where every read and predicate falls
// through to `std::fs`; `Some` once the wasm surface installs a document's files, under which reads
// resolve from the map and the real filesystem is never touched.
static SOURCES: RwLock<Option<HashMap<PathBuf, Vec<u8>>>> = RwLock::new(None);

/// Installs an injected source map, so every subsequent read resolves from `files` rather than the real
/// filesystem. Keys are normalised on install and again on lookup, so a `dir.join("../x")` and a direct
/// `x` resolve alike. The wasm compile surface calls this before it runs the assembler and clears it
/// after, so one process can compile many documents in turn.
pub fn install(files: HashMap<PathBuf, Vec<u8>>) -> Outcome<()> {
	let mut norm: HashMap<PathBuf, Vec<u8>> = HashMap::with_capacity(files.len());
	for (k, v) in files {
		norm.insert(normalise(&k), v);
	}
	let mut guard = lock_write!(SOURCES, "While installing the source map");
	*guard = Some(norm);
	Ok(())
}

/// Removes any installed source map, restoring the native filesystem behaviour.
pub fn clear() -> Outcome<()> {
	let mut guard = lock_write!(SOURCES, "While clearing the source map");
	*guard = None;
	Ok(())
}

/// Is a source map installed? True only under a wasm compile; false on the native default path, where a
/// poisoned lock also reads as absent so a read still falls through to `std::fs`.
pub fn is_installed() -> bool {
	match SOURCES.read() {
		Ok(guard)	=> guard.is_some(),
		Err(_)		=> false,
	}
}

/// Reads a file's bytes: from the installed source map where one holds the path, else from the real
/// filesystem. With no map installed this is exactly [`std::fs::read`], so a native compile is unchanged.
pub fn read(path: &Path) -> io::Result<Vec<u8>> {
	match SOURCES.read() {
		Ok(guard) => match guard.as_ref() {
			Some(map) => match map.get(&normalise(path)) {
				Some(bytes)	=> Ok(bytes.clone()),
				None		=> native_read(path),
			},
			None => native_read(path),
		},
		Err(_) => native_read(path),
	}
}

/// Reads a file as UTF-8 text, the drop-in for [`std::fs::read_to_string`]: invalid UTF-8 is the same
/// `InvalidData` error the standard call raises, so a caller's error handling is unchanged.
pub fn read_to_string(path: &Path) -> io::Result<String> {
	let bytes = ok!(read(path));
	match String::from_utf8(bytes) {
		Ok(s)	=> Ok(s),
		Err(e)	=> Err(io::Error::new(io::ErrorKind::InvalidData, e)),
	}
}

/// Writes a file's bytes: into the installed source map where one is present (so a later read in the same
/// compile sees it), else to the real filesystem. With no map installed this is exactly [`std::fs::write`].
pub fn write(path: &Path, contents: &[u8]) -> io::Result<()> {
	if is_installed() {
		let mut guard = match SOURCES.write() {
			Ok(g)	=> g,
			Err(_)	=> return Err(io::Error::new(io::ErrorKind::Other, "the source map lock is poisoned")),
		};
		if let Some(map) = guard.as_mut() {
			map.insert(normalise(path), contents.to_vec());
		}
		Ok(())
	} else {
		native_write(path, contents)
	}
}

/// Does a path resolve to a file? A map lookup where one is installed, falling through to the real
/// filesystem; exactly [`Path::exists`] on the native default path.
pub fn exists(path: &Path) -> bool {
	match SOURCES.read() {
		Ok(guard) => match guard.as_ref() {
			Some(map)	=> map.contains_key(&normalise(path)) || native_exists(path),
			None		=> native_exists(path),
		},
		Err(_) => native_exists(path),
	}
}

/// Is a path a readable file? For the injected map a key is a file, so this matches [`exists`] there;
/// exactly [`Path::is_file`] on the native default path.
pub fn is_file(path: &Path) -> bool {
	match SOURCES.read() {
		Ok(guard) => match guard.as_ref() {
			Some(map)	=> map.contains_key(&normalise(path)) || native_is_file(path),
			None		=> native_is_file(path),
		},
		Err(_) => native_is_file(path),
	}
}

/// Resolves a path to an absolute, canonical form. Under an installed map there is no filesystem to walk,
/// so the path is normalised lexically (`.` and `..` folded); exactly [`std::fs::canonicalize`] on the
/// native default path.
pub fn canonicalize(path: &Path) -> io::Result<PathBuf> {
	match SOURCES.read() {
		Ok(guard) => match guard.as_ref() {
			Some(_)	=> Ok(normalise(path)),
			None	=> native_canonicalize(path),
		},
		Err(_) => native_canonicalize(path),
	}
}

/// Folds `.` and `..` out of a path lexically, without touching the filesystem, so an injected map keys
/// consistently whether a file is named directly or reached through a `join("..")`.
fn normalise(path: &Path) -> PathBuf {
	let mut out = PathBuf::new();
	for comp in path.components() {
		match comp {
			Component::CurDir		=> {},
			Component::ParentDir	=> { out.pop(); },
			other					=> out.push(other.as_os_str()),
		}
	}
	out
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ NATIVE FALL-THROUGH                                                        │
// └───────────────────────────────────────────────────────────────────────────┘
// On every target but wasm the fall-through is the real filesystem, so a native build with no map
// installed behaves exactly as the direct `std::fs` calls these replaced. On wasm there is no
// filesystem: a path absent from the injected map cannot be read, and a missing entry names itself and
// the map so a diagnostic carries the file rather than a bare "not found".

#[cfg(not(target_arch = "wasm32"))]
fn native_read(path: &Path) -> io::Result<Vec<u8>> {
	std::fs::read(path)
}

#[cfg(target_arch = "wasm32")]
fn native_read(path: &Path) -> io::Result<Vec<u8>> {
	Err(io::Error::new(
		io::ErrorKind::NotFound,
		fmt!("{:?} is not in the injected source map, and the wasm target has no filesystem.", path)))
}

#[cfg(not(target_arch = "wasm32"))]
fn native_write(path: &Path, contents: &[u8]) -> io::Result<()> {
	std::fs::write(path, contents)
}

#[cfg(target_arch = "wasm32")]
fn native_write(path: &Path, _contents: &[u8]) -> io::Result<()> {
	Err(io::Error::new(
		io::ErrorKind::Unsupported,
		fmt!("cannot write {:?}: the wasm target has no filesystem and no source map is installed.", path)))
}

#[cfg(not(target_arch = "wasm32"))]
fn native_exists(path: &Path) -> bool {
	path.exists()
}

#[cfg(target_arch = "wasm32")]
fn native_exists(_path: &Path) -> bool {
	false
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
fn native_canonicalize(path: &Path) -> io::Result<PathBuf> {
	std::fs::canonicalize(path)
}

#[cfg(target_arch = "wasm32")]
fn native_canonicalize(path: &Path) -> io::Result<PathBuf> {
	Ok(normalise(path))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A path with `.` and `..` components folds to the same key whether it was named directly or reached
	/// through a join, so an injected map keyed on one form is found by the other.
	#[test]
	fn normalise_folds_dot_and_parent() {
		assert_eq!(normalise(Path::new("a/b/../c/./d")), PathBuf::from("a/c/d"));
		assert_eq!(normalise(Path::new("./x")), PathBuf::from("x"));
	}

	/// With no map installed the module reports itself absent, so every read falls through to `std::fs`
	/// and a native compile is unchanged.
	#[test]
	fn not_installed_by_default() {
		assert!(!is_installed());
	}
}
