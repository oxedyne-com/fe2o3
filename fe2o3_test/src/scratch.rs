//! Scratch directories for fixtures that need a real filesystem.
//!
//! A test that wants a directory of its own must not take one from
//! `std::env::temp_dir()`.  On the machines this library is developed on `/tmp` is a
//! tmpfs, so every byte a fixture writes is resident memory charged to the process
//! that wrote it, and nothing reclaims it when the test binary exits.  A suite that
//! leaves a directory behind per test leaves memory behind per test.
//!
//! [`scratch_dir`] answers with a directory under the user's cache instead: on disk,
//! never under the system temporary directory, unique to the call, and swept clean of
//! the fixtures left by processes that have since exited.
//!
//! ```no_run
//! use oxedyne_fe2o3_core::prelude::*;
//! use oxedyne_fe2o3_test::scratch::scratch_dir;
//!
//! # fn main() -> Outcome<()> {
//! let dir = res!(scratch_dir("my_crate_widget"));
//! res!(std::fs::write(dir.join("in.txt"), b"hello"));
//! # Ok(())
//! # }
//! ```

use oxedyne_fe2o3_core::prelude::*;

use std::{
	fs,
	path::{
		Path,
		PathBuf,
	},
	sync::{
		atomic::{
			AtomicU64,
			Ordering,
		},
		Once,
	},
	time::{
		Duration,
		SystemTime,
		UNIX_EPOCH,
	},
};

/// Where scratch directories are gathered, relative to the user's cache directory.
pub const SCRATCH_REL: &str = "oxedyne/scratch";

/// Names the scratch root outright, overriding the cache directory.
pub const SCRATCH_ENV: &str = "OXEDYNE_SCRATCH_DIR";

/// The last-resort root, relative to the current directory, when no cache
/// directory can be written to.
pub const SCRATCH_FALLBACK: &str = ".oxedyne-scratch";

/// How old a fixture must be before the sweep removes it when it cannot tell
/// whether the process that made it is still running.
const UNKNOWN_LIFE_AGE: Duration = Duration::from_secs(60 * 60);

/// How old a fixture must be before the sweep removes it regardless of who
/// appears to own it.  Process identifiers are reused, so a live-looking owner
/// is not proof for ever.
const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// How many names one call tries before giving up.
const NAME_ATTEMPTS: u32 = 64;

/// Distinguishes two calls that land in the same nanosecond.
static SEQ: AtomicU64 = AtomicU64::new(0);

/// Guards the sweep, which is worth doing once per process and no more.
static SWEPT: Once = Once::new();

/// The directory scratch fixtures are gathered under, created if it is absent.
///
/// Resolved from the first of these that can be created:
///
/// 1. `$OXEDYNE_SCRATCH_DIR`, used as given.
/// 2. `$XDG_CACHE_HOME/oxedyne/scratch`.
/// 3. `$HOME/.cache/oxedyne/scratch`.
/// 4. `./.oxedyne-scratch`.
///
/// # Errors
/// Fails if the resolved root lies under the system temporary directory, which is
/// the thing this module exists to avoid, or if no candidate can be created.  There
/// is deliberately no fall back to `/tmp`: a fixture that quietly lands in a tmpfs
/// is the failure being prevented, so it is reported instead.
pub fn scratch_root() -> Outcome<PathBuf> {
	// An explicit request is honoured or refused, never quietly replaced.
	if let Some(dir) = env_dir(SCRATCH_ENV) {
		if under_temp(&dir) {
			return Err(err!(
				"{} names {:?}, which is under the system temporary directory {:?}. \
				Scratch fixtures must not be written to a tmpfs.",
				SCRATCH_ENV, dir, std::env::temp_dir();
			Invalid, Input));
		}
		res!(fs::create_dir_all(&dir), IO, File);
		return Ok(dir);
	}

	let mut tried: Vec<String> = Vec::new();
	let mut cands: Vec<PathBuf> = Vec::new();
	if let Some(cache) = env_dir("XDG_CACHE_HOME") {
		cands.push(cache.join(SCRATCH_REL));
	}
	if let Some(home) = env_dir("HOME") {
		cands.push(home.join(".cache").join(SCRATCH_REL));
	}
	cands.push(PathBuf::from(SCRATCH_FALLBACK));

	for cand in cands {
		if under_temp(&cand) {
			tried.push(fmt!("{:?} (under the system temporary directory)", cand));
			continue;
		}
		match fs::create_dir_all(&cand) {
			Ok(()) => return Ok(cand),
			Err(e) => tried.push(fmt!("{:?} ({})", cand, e)),
		}
	}
	Err(err!(
		"No scratch root could be created; tried {}. Set {} to a writable path \
		outside {:?}.",
		tried.join(", "), SCRATCH_ENV, std::env::temp_dir();
	IO, File, Create))
}

/// Creates a fresh, empty scratch directory for `label` and returns its path.
///
/// The name carries the process identifier, so a second process never shares a
/// directory with this one, and the directory is created rather than merely named:
/// uniqueness is settled by the filesystem, which is the only party that can settle
/// it.  A clash is retried under a new name.
///
/// The first call in a process also sweeps the scratch root of fixtures whose owning
/// process has exited, which is what keeps the root from growing run over run.  There
/// is no reliable teardown hook in a Rust test binary, so the cleaning is done on the
/// way in rather than on the way out.
///
/// # Errors
/// Fails if the scratch root cannot be resolved (see [`scratch_root`]) or if
/// [`NAME_ATTEMPTS`] names in a row are all taken.
pub fn scratch_dir(label: &str) -> Outcome<PathBuf> {
	let root = res!(scratch_root());
	SWEPT.call_once(|| sweep(&root));

	let label = clean_label(label);
	let pid = std::process::id();
	for _ in 0..NAME_ATTEMPTS {
		let nanos = match SystemTime::now().duration_since(UNIX_EPOCH) {
			Ok(d)  => d.as_nanos(),
			Err(_) => 0,
		};
		let seq = SEQ.fetch_add(1, Ordering::Relaxed);
		let dir = root.join(fmt!("{}.{}.{}.{}", label, pid, nanos, seq));
		match fs::create_dir(&dir) {
			Ok(()) => return Ok(dir),
			// Taken: some other call got this name first, so take another.
			Err(ref e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
			Err(e) => return Err(err!(e,
				"Could not create the scratch directory {:?}.", dir;
			IO, File, Create)),
		}
	}
	Err(err!(
		"Could not find an unused scratch name for {:?} under {:?} in {} attempts.",
		label, root, NAME_ATTEMPTS;
	IO, File, Conflict))
}

/// Reads an environment variable as a directory, ignoring it when unset or empty.
fn env_dir(name: &str) -> Option<PathBuf> {
	match std::env::var(name) {
		Ok(s) if !s.trim().is_empty() => Some(PathBuf::from(s)),
		_ => None,
	}
}

/// Whether `p` lies under the system temporary directory.
///
/// `/tmp` is checked as well as `std::env::temp_dir()`, because `TMPDIR` may point
/// somewhere else while `/tmp` is still the tmpfs that must be kept clear.
fn under_temp(p: &Path) -> bool {
	let tmp = std::env::temp_dir();
	p.starts_with(&tmp) || p.starts_with("/tmp")
}

/// Reduces `label` to characters that are safe in a directory name.
///
/// A dot is not one of them: the sweep reads the trailing fields of a name back,
/// and a dotted label would make that ambiguous.
fn clean_label(label: &str) -> String {
	let mut out = String::with_capacity(label.len());
	for c in label.chars() {
		if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
			out.push(c);
		} else {
			out.push('_');
		}
	}
	if out.is_empty() {
		out.push_str("scratch");
	}
	out
}

/// Whether a process with this identifier is still running.
///
/// Answered from `/proc`, which is Linux's; where there is no `/proc` there is no
/// answer without a system call, so `None` comes back and the caller falls back to
/// the age of the directory.
fn pid_alive(pid: u32) -> Option<bool> {
	let proc = Path::new("/proc");
	if !proc.is_dir() {
		return None;
	}
	Some(proc.join(pid.to_string()).is_dir())
}

/// The owning process identifier encoded in a scratch directory name.
///
/// `None` for anything not written by [`scratch_dir`], which is then left alone --
/// the sweep only removes what it made.
fn owner_pid(name: &str) -> Option<u32> {
	// Read back to front: sequence, nanoseconds, process identifier, label.  A
	// fourth field must be there, or the label is missing and the name is not ours.
	let parts: Vec<&str> = name.rsplitn(4, '.').collect();
	if parts.len() < 4 {
		return None;
	}
	if parts[0].parse::<u64>().is_err() || parts[1].parse::<u128>().is_err() {
		return None;
	}
	match parts[2].parse::<u32>() {
		Ok(pid) => Some(pid),
		Err(_)  => None,
	}
}

/// Removes the scratch directories left behind by processes that have exited.
///
/// Directories owned by a running process are left where they are, including this
/// process's own, so two suites running side by side never tread on each other.  A
/// directory older than [`MAX_AGE`] goes regardless, because process identifiers are
/// reused and a stale one can look alive for ever.
///
/// Errors are swallowed: a sweep that cannot remove something has not stopped the
/// caller from getting the directory it asked for.
pub fn sweep(root: &Path) {
	let self_pid = std::process::id();
	let entries = match fs::read_dir(root) {
		Ok(e)  => e,
		Err(_) => return,
	};
	for entry in entries.flatten() {
		let name = entry.file_name().to_string_lossy().to_string();
		let pid = match owner_pid(&name) {
			Some(p) => p,
			None    => continue, // Not ours to remove.
		};
		// Our own, and never stale. On Linux the liveness test below reaches the
		// same answer, so this only saves a `/proc` lookup; where there is no
		// `/proc` it is the whole protection, since a suite that runs longer than
		// [`UNKNOWN_LIFE_AGE`] would otherwise sweep its own working directories
		// away underneath itself.
		if pid == self_pid {
			continue;
		}
		let age = entry
			.metadata()
			.ok()
			.and_then(|m| m.modified().ok())
			.and_then(|t| SystemTime::now().duration_since(t).ok());
		let stale = match pid_alive(pid) {
			Some(true)  => age.map(|a| a > MAX_AGE).unwrap_or(false),
			Some(false) => true,
			None        => age.map(|a| a > UNKNOWN_LIFE_AGE).unwrap_or(false),
		};
		if stale {
			let _ = fs::remove_dir_all(entry.path());
		}
	}
}


// ┌───────────────────────────────────────────────────────────────┐
// │ Tests                                                          │
// └───────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
	use super::*;

	/// A directory to plant fixtures in and sweep.
	///
	/// Taken from [`scratch_dir`] like everything else, so that a run cut short
	/// leaves behind something the next run knows how to remove.
	fn test_root(name: &str) -> PathBuf {
		match scratch_dir(&fmt!("selftest_{}", name)) {
			Ok(d)  => d,
			Err(e) => panic!("a self-test root: {}", e),
		}
	}

	#[test]
	fn test_scratch_dir_is_never_under_tmp() {
		// The whole point: a fixture in a tmpfs is resident memory nothing frees.
		let dir = match scratch_dir("fe2o3_test_selfcheck") {
			Ok(d)  => d,
			Err(e) => panic!("a scratch directory: {}", e),
		};
		assert!(!dir.starts_with("/tmp"),
			"scratch directory landed in the tmpfs: {:?}", dir);
		assert!(!dir.starts_with(std::env::temp_dir()),
			"scratch directory landed in the system temporary directory: {:?}", dir);
		assert!(dir.is_dir(), "scratch directory was not created: {:?}", dir);
		let _ = fs::remove_dir_all(&dir);
	}

	#[test]
	fn test_scratch_dir_is_unique_per_call() {
		// Two calls in the same tick used to be able to agree on a name, which is
		// how two tests come to share one directory.
		let mut seen = std::collections::HashSet::new();
		let mut made = Vec::new();
		for _ in 0..50 {
			let dir = match scratch_dir("fe2o3_test_unique") {
				Ok(d)  => d,
				Err(e) => panic!("a scratch directory: {}", e),
			};
			assert!(seen.insert(dir.clone()), "two calls shared {:?}", dir);
			made.push(dir);
		}
		for dir in made {
			let _ = fs::remove_dir_all(&dir);
		}
	}

	#[test]
	fn test_sweep_removes_dead_owners_and_spares_live_ones() {
		// Bounded accumulation rests entirely on this: a test binary has no
		// teardown hook, so what the last run left is removed by the next one.
		let root = test_root("sweep");
		// Process 0 is not a process, so nothing owns this one.
		let dead = root.join("fixture.0.123456789.0");
		// Process 1 always is one, and it is not this process -- so sparing it can
		// only be the liveness test doing the work, not the shortcut for our own.
		let other = root.join("fixture.1.123456789.1");
		let mine = root.join(fmt!("fixture.{}.123456789.2", std::process::id()));
		let alien = root.join("someone-elses-directory");
		for d in [&dead, &other, &mine, &alien] {
			match fs::create_dir_all(d) {
				Ok(())  => {}
				Err(e)  => panic!("a fixture: {}", e),
			}
		}
		sweep(&root);
		assert!(!dead.exists(), "a dead owner's fixture survived the sweep");
		assert!(other.exists(), "a live owner's fixture was swept away");
		assert!(mine.exists(), "this process's own fixture was swept away");
		assert!(alien.exists(), "the sweep removed a directory it did not create");
		let _ = fs::remove_dir_all(&root);
	}

	#[test]
	fn test_a_tmp_scratch_root_is_refused() {
		// Refused rather than obeyed: an override is the one way a caller could
		// still put fixtures in the tmpfs by hand.
		let dir = std::env::temp_dir().join("fe2o3_test_should_never_exist");
		assert!(under_temp(&dir), "the temporary directory should read as temporary");
		assert!(under_temp(Path::new("/tmp/anything")),
			"/tmp should read as temporary whatever TMPDIR says");
		assert!(!under_temp(Path::new("/home/someone/.cache/oxedyne/scratch")),
			"a cache path should not read as temporary");
		// And nothing above created it.
		assert!(!dir.exists(), "the refusal should not have made the directory");
	}

	#[test]
	fn test_only_our_own_names_are_understood() {
		assert_eq!(owner_pid("label.42.123.7"), Some(42));
		assert_eq!(owner_pid("has-dashes_and_underscores.42.123.7"), Some(42));
		assert_eq!(owner_pid("nodots"), None);
		assert_eq!(owner_pid("42.123.7"), None); // No label field.
		assert_eq!(owner_pid("label.notapid.123.7"), None);
	}

	#[test]
	fn test_a_label_cannot_smuggle_a_path_or_a_dot() {
		assert_eq!(clean_label("../../etc"), "______etc");
		assert_eq!(clean_label("a.b"), "a_b");
		assert_eq!(clean_label(""), "scratch");
		assert_eq!(clean_label("daimond-gw_test"), "daimond-gw_test");
	}
}
