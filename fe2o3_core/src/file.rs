use crate::{
    prelude::*,
    path::NormalPath,
};

use std::{
    ffi::{
        OsStr,
        OsString,
    },
    fmt,
    fs::{
        self,
        File,
        OpenOptions,
    },
    io::Write,
    path::{
        Path,
        PathBuf,
    },
    sync::atomic::{
        AtomicU64,
        Ordering,
    },
    time::{
        Duration,
        SystemTime,
    },
};


#[derive(Clone, Debug)]
pub enum OsPath {
    Dir(PathBuf),
    File(PathBuf),
}

#[derive(Clone, Copy, Debug)]
pub enum PathState {
    DirMustExist,
    FileMustExist,
    Create,
}

impl PathState {

    pub fn validate(
        &self,
        root:       &PathBuf,
        rel_path:   &str,
    )
        -> Outcome<()>
    {
        let rel_path = Path::new(rel_path).normalise();
        if rel_path.escapes() {
            return Err(err!(
                "The relative path '{:?}' escapes the root directory.", rel_path;
            Invalid, Input, Path));
        }
        let abs_path = root.clone().join(rel_path).normalise().absolute();
        if abs_path.exists() {
            if let Self::DirMustExist = self {
                if !abs_path.is_dir() {
                    return Err(err!(
                        "Path '{:?}' exists but is not a directory.", root;
                    Input, Invalid, File, Path));
                }
            }
        } else {
            match self {
                Self::DirMustExist |
                Self::FileMustExist => return Err(err!(
                    "The path '{:?}' must exist but was not found.", abs_path;
                Path, File, Missing)),
                Self::Create => res!(fs::create_dir_all(&abs_path)),
            }
        }
        Ok(())
    }
}


pub trait Loadable {
    fn load<P: AsRef<Path>>(path: P) -> Outcome<Self> where Self: Sized;
}

pub fn touch(path: &Path) -> Outcome<File> {
    Ok(res!(
        OpenOptions::new().create(true).write(true).open(path),
        File, Write,
    ))
}

// Atomic replacement
pub const TMP_SWEEP_AGE:    Duration    = Duration::from_secs(10 * 60); // a live writer's tmp is far younger
const TMP_TRIES:            u64         = 16;                           // fresh names tried before giving up
static TMP_SEQ:             AtomicU64   = AtomicU64::new(0);            // per process, so per thread too

/// The permission bits a file saved by [`save_atomic`] ends with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SaveMode {
    Owner,  // 0600 from creation, whatever the umask and whatever the replaced file held
    Keep,   // the replaced file's bits, else those `fs::write` would have made, umask and all
}

/// Replaces `path` with `data` whole, where `fs::write` truncates the file and
/// then writes it. A reader, or a crash, finds the old bytes or the new ones and
/// never an empty or partial file: between `fs::write`'s truncate and its last
/// byte a SIGKILL leaves the file empty for good, and a small state file that
/// every later command parses is then refused for ever.
///
/// The write lands on a sibling `<name>.<pid>.<n>.tmp`, where `n` counts saves
/// across the whole process, so no other writer -- another process, or another
/// thread of this one -- ever uses the same name. It is made with `create_new`
/// (never `create` then `chmod`, which leaves a window at the process's default
/// mode), fsynced, then renamed over `path`; on unix the directory is fsynced
/// too, so the rename cannot survive a crash while the directory entry pointing
/// at it does not. The tmp is removed on every error path. Two writers of one
/// `path` each rename a whole file of their own, so the last rename wins whole.
///
/// `mode` says which permission bits the new file has: see [`SaveMode`]. What
/// the rename replaces is the name and not the inode, so a symlink at `path` is
/// replaced rather than followed, a hard link to the old file is left holding
/// the old bytes, and ownership is the caller's. The caller needs write access to
/// the directory and not to the file, so a read-only file is replaced too.
///
/// Once its own rename has landed, a save sweeps, best-effort, any
/// `<name>.<digits>.<digits>.tmp` older than [`TMP_SWEEP_AGE`] that crashed
/// writers left beside `path`. A failed sweep never fails the save. A writer that
/// stalls past that age between create and rename, or whose tmp a writer on
/// older code removes, finds its tmp gone at the rename: it returns an error
/// tagged `Missing` and leaves `path` as this call found it. `Ok` means this
/// call renamed over `path` a file it created and wrote itself.
pub fn save_atomic(
    path:   &Path,
    data:   &[u8],
    mode:   SaveMode,
)
    -> Outcome<()>
{
    replace_file(path, data, mode, TMP_SWEEP_AGE, false)
}

/// The one body of [`save_atomic`] and [`save_secret`]. `sweep_legacy` also
/// removes `<name>.tmp`, a name only the secret saves ever wrote and which an
/// ordinary neighbour could be using, and `sweep_age` lets a test put the sweep
/// onto a live writer's file.
fn replace_file(
    path:           &Path,
    data:           &[u8],
    mode:           SaveMode,
    sweep_age:      Duration,
    sweep_legacy:   bool,
)
    -> Outcome<()>
{
    let name = res!(file_name_of(path));
    #[cfg(unix)]
    let old = match mode {
        SaveMode::Keep  => mode_bits_of(path),
        SaveMode::Owner => None,
    };
    let (tmp, mut f) = res!(create_tmp(path, &name, mode));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Before a byte is written, so nothing is ever there at a wider mode
        // than the file it replaces.
        if let Some(bits) = old {
            if let Err(e) = f.set_permissions(fs::Permissions::from_mode(bits)) {
                let _ = fs::remove_file(&tmp);
                return Err(err!(e,
                    "Could not give the temporary file {:?} the mode {:04o} of {:?}.",
                    tmp, bits, path;
                    File, IO, Write));
            }
        }
    }
    if let Err(e) = f.write_all(data) {
        let _ = fs::remove_file(&tmp);
        return Err(err!(e,
            "Could not write the temporary file {:?}.", tmp;
            File, IO, Write));
    }
    if let Err(e) = f.sync_all() {
        let _ = fs::remove_file(&tmp);
        return Err(err!(e,
            "Could not fsync the temporary file {:?}.", tmp;
            File, IO, Write));
    }
    // Closed before the rename, which off unix cannot move a file still open.
    drop(f);
    if let Err(e) = fs::rename(&tmp, path) {
        if e.kind() == std::io::ErrorKind::NotFound {
            // No other writer makes this name, so it is gone because another
            // writer's sweep took it, and the rename moved nothing.
            return Err(err!(e,
                "The temporary file {:?} is gone, most likely removed by another \
                writer's cleanup, so {:?} was not changed by this call.", tmp, path;
                File, IO, Missing));
        }
        let _ = fs::remove_file(&tmp);
        return Err(err!(e,
            "Could not rename {:?} over {:?}.", tmp, path;
            File, IO, Write));
    }
    #[cfg(unix)]
    res!(sync_parent_dir(path));
    sweep_tmps(path, &name, sweep_age, sweep_legacy);
    Ok(())
}

/// The permission bits `path` holds now, if it is there to be asked.
#[cfg(unix)]
fn mode_bits_of(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;

    fs::metadata(path).ok().map(|m| m.permissions().mode() & 0o777)
}

// Secrets

/// Writes `data` to `path` as key material: atomically, as [`save_atomic`] does,
/// and on unix at mode 0600 whatever the caller's umask, so the bytes are never
/// briefly readable by anyone else and a crash never leaves a loose or partial
/// file where the secret should be. A pre-existing, more permissive file also
/// ends at 0600, since the rename replaces its mode along with its contents.
///
/// Once its own rename has landed, a save also sweeps the legacy `<name>.tmp`,
/// which no writer uses any more.
pub fn save_secret(path: &Path, data: &[u8]) -> Outcome<()> {
    save_secret_aged(path, data, TMP_SWEEP_AGE)
}

/// [`save_secret`], sweeping temporary files at least `sweep_age` old, so a
/// test can put the sweep onto a live writer's file.
pub(crate) fn save_secret_aged(
    path:       &Path,
    data:       &[u8],
    sweep_age:  Duration,
)
    -> Outcome<()>
{
    replace_file(path, data, SaveMode::Owner, sweep_age, true)
}

/// Creates `path` and any missing parents, as `create_dir_all` does, but at
/// mode 0700 rather than the process default: a directory meant to hold key
/// material must not be group- or world-searchable, since `create_dir_all`'s
/// default mode is only ever narrowed by the umask, and umasks such as 002
/// or 022 leave it group- or world-readable and -searchable, letting anyone
/// in the group list, and on some setups swap, the keys inside.
///
/// Only directories this call actually creates get 0700; one that already
/// exists is left at whatever mode it holds, since narrowing that is
/// `restrict_secret_dir`'s job. A plain `create_dir_all` off unix, where there
/// is no mode to set.
#[cfg(unix)]
pub fn create_secret_dir(path: &Path) -> Outcome<()> {
    use std::{
        fs::DirBuilder,
        os::unix::fs::DirBuilderExt,
    };

    if let Err(e) = DirBuilder::new().recursive(true).mode(0o700).create(path) {
        return Err(err!(e,
            "Could not create key directory {:?} at mode 0700.", path;
            File, IO, Create));
    }
    Ok(())
}

/// A plain recursive create off unix: there is no mode to set.
#[cfg(not(unix))]
pub fn create_secret_dir(path: &Path) -> Outcome<()> {
    if let Err(e) = fs::create_dir_all(path) {
        return Err(err!(e,
            "Could not create key directory {:?}.", path;
            File, IO, Create));
    }
    Ok(())
}

/// Narrows an existing file's mode to its owner read/write bits, dropping
/// group, other and execute bits, for a key file that predates this
/// codebase's atomic `save_secret` writes, or that arrived by some other
/// route -- a backup restore, an `scp`, a deploy step -- at whatever mode
/// its source held.
///
/// Unlike widening a mode, narrowing one has no window to close: the file
/// already exists at its current mode throughout, and `chmod` only ever
/// removes bits, so there is no intermediate state where the file is any
/// more exposed than it already was. A no-op when the mode is already 0600
/// or narrower, and a no-op entirely off unix, where there are no POSIX mode
/// bits to narrow. Only ever removes bits from the owner's read/write pair
/// too -- a 0440 key ends at 0400, never gaining the write bit it did not
/// have.
///
/// Returns the mode the file was narrowed from, so a caller that silences the
/// log can still tell its user, and `None` when nothing changed.
///
/// A failed narrowing warns and returns `Ok(None)` rather than erroring: the
/// file was already readable at whatever mode it held, so refusing to start
/// over a `chmod` this process cannot make -- EPERM on a key it can read but
/// does not own, EROFS on a read-only mount -- would trade a narrower mode
/// for no service at all.
#[cfg(unix)]
pub fn restrict_secret(path: &Path) -> Outcome<Option<u32>> {
    use std::os::unix::fs::PermissionsExt;

    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) => return Err(err!(e,
            "Could not stat {:?} to check whether its mode needs narrowing.", path;
            File, IO, Read)),
    };
    let mode = meta.permissions().mode() & 0o777;
    if mode & !0o600 == 0 {
        return Ok(None);
    }
    let narrowed = mode & 0o600; // keep only the owner rw bits already present, never add one
    warn!("Narrowing key file {:?} from mode {:04o} to {:04o}.", path, mode, narrowed);
    if let Err(e) = fs::set_permissions(path, fs::Permissions::from_mode(narrowed)) {
        warn!("Could not narrow {:?} from mode {:04o} to {:04o}: {}. Leaving the key at its \
            current, already-readable mode rather than refusing to start.", path, mode, narrowed, e);
        return Ok(None);
    }
    Ok(Some(mode))
}

/// A no-op off unix: there are no POSIX mode bits to narrow.
#[cfg(not(unix))]
pub fn restrict_secret(_path: &Path) -> Outcome<Option<u32>> {
    Ok(None)
}

/// Narrows an existing directory of key material to its owner's bits alone,
/// for one made before `create_secret_dir` was used, or by some other route. A
/// group-writable one lets a group member rename a file of their own over a
/// 0600 key inside it, so narrowing the keys alone is not enough.
///
/// The same contract as `restrict_secret`, with the owner's search bit kept:
/// only bits are removed, never added, so 0500 stays 0500. Returns the mode it
/// narrowed from, `None` when nothing changed, and warns and returns `Ok(None)`
/// for a `chmod` it cannot make. `restrict_secret` itself would not do here,
/// since its 0600 mask takes the search bit and with it the way in.
#[cfg(unix)]
pub fn restrict_secret_dir(path: &Path) -> Outcome<Option<u32>> {
    use std::os::unix::fs::PermissionsExt;

    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) => return Err(err!(e,
            "Could not stat the key directory {:?} to check whether its mode needs narrowing.", path;
            File, IO, Read)),
    };
    if !meta.is_dir() {
        return Err(err!(
            "{:?} is not a directory, so it cannot be narrowed as a key directory.", path;
            Invalid, Input, File));
    }
    let mode = meta.permissions().mode() & 0o777;
    if mode & !0o700 == 0 {
        return Ok(None);
    }
    let narrowed = mode & 0o700; // keep only the owner bits already present, never add one
    warn!("Narrowing key directory {:?} from mode {:04o} to {:04o}.", path, mode, narrowed);
    if let Err(e) = fs::set_permissions(path, fs::Permissions::from_mode(narrowed)) {
        warn!("Could not narrow the key directory {:?} from mode {:04o} to {:04o}: {}. Leaving \
            it at its current mode rather than refusing to start.", path, mode, narrowed, e);
        return Ok(None);
    }
    Ok(Some(mode))
}

/// A no-op off unix: there are no POSIX mode bits to narrow.
#[cfg(not(unix))]
pub fn restrict_secret_dir(_path: &Path) -> Outcome<Option<u32>> {
    Ok(None)
}

/// Fsyncs the directory holding `path`, after the rename that lands a file
/// there. Without this, the rename itself can survive a crash while the
/// directory entry pointing at it does not, which can bring back a file --
/// or the previous contents of one -- that was already reported saved.
#[cfg(unix)]
fn sync_parent_dir(path: &Path) -> Outcome<()> {
    let dir = parent_dir(path);
    let d = match File::open(dir) {
        Ok(d) => d,
        Err(e) => return Err(err!(e,
            "Could not open the directory {:?} to fsync it after saving {:?}.", dir, path;
            File, IO, Write)),
    };
    if let Err(e) = d.sync_all() {
        return Err(err!(e,
            "Could not fsync the directory {:?} after saving {:?}.", dir, path;
            File, IO, Write));
    }
    Ok(())
}

fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    }
}

fn file_name_of(path: &Path) -> Outcome<OsString> {
    match path.file_name() {
        Some(n) => Ok(n.to_os_string()),
        None => Err(err!(
            "Path {:?} has no file-name component; cannot save a file there.", path;
            Invalid, Input, Path)),
    }
}

/// Creates this writer's own temporary sibling of `path`. It is always a new
/// file, never an existing one, so on unix the 0600 asked for by
/// [`SaveMode::Owner`] is the mode it gets.
fn create_tmp(
    path:   &Path,
    name:   &OsStr,
    mode:   SaveMode,
)
    -> Outcome<(PathBuf, File)>
{
    let pid = std::process::id();
    for _ in 0..TMP_TRIES {
        let n = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = path.with_file_name(tmp_name(name, pid, n));
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        if mode == SaveMode::Owner {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        match opts.open(&tmp) {
            Ok(f) => return Ok((tmp, f)),
            // Only a crashed writer whose pid has come round again, or a
            // namesake on another host sharing this directory, holds the name.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(err!(e,
                "Could not create the temporary file {:?}.", tmp;
                File, IO, Create)),
        }
    }
    Err(err!(
        "All {} fresh temporary names tried for the file {:?} were already taken.",
        TMP_TRIES, path;
        File, IO, Create, Exists))
}

/// The temporary name writer `n` of process `pid` gives file `name`:
/// `<name>.<pid>.<n>.tmp`.
fn tmp_name(name: &OsStr, pid: u32, n: u64) -> OsString {
    // Built as an `OsString`, not via `to_string_lossy`, so a non-UTF-8 file
    // name is not mangled into one that could collide with another file's.
    let mut tmp = name.to_os_string();
    tmp.push(fmt!(".{}.{}.tmp", pid, n));
    tmp
}

/// The one temporary name every secret writer used before names were made unique.
fn legacy_tmp_name(name: &OsStr) -> OsString {
    let mut tmp = name.to_os_string();
    tmp.push(".tmp");
    tmp
}

/// Is `entry` strictly `<name>.<digits>.<digits>.tmp`, a writer's temporary
/// file for file `name`?
fn is_tmp_of(entry: &OsStr, name: &OsStr) -> bool {
    let digits = |p: &[u8]| !p.is_empty() && p.iter().all(u8::is_ascii_digit);
    let mid = entry.as_encoded_bytes()
        .strip_prefix(name.as_encoded_bytes())
        .and_then(|r| r.strip_prefix(b"."))
        .and_then(|r| r.strip_suffix(b".tmp"));
    match mid {
        Some(mid) => {
            let mut parts = mid.split(|b| *b == b'.');
            match (parts.next(), parts.next(), parts.next()) {
                (Some(pid), Some(n), None) => digits(pid) && digits(n),
                _ => false,
            }
        },
        None => false,
    }
}

/// Removes, best-effort, what crashed writers of file `name` left beside `path`:
/// each `<name>.<pid>.<n>.tmp` at least `sweep_age` old, and when `legacy` the
/// `<name>.tmp` of the old secret saves. A live writer's file is younger than
/// any sensible bound, so it is left alone. Nothing here can fail a save that has
/// already landed.
fn sweep_tmps(path: &Path, name: &OsStr, sweep_age: Duration, legacy: bool) {
    let dir = parent_dir(path);
    let entries = match fs::read_dir(dir) {
        Ok(it) => it,
        Err(e) => {
            warn!("Could not list {:?} to sweep leftover temporary copies of {:?}: {}.",
                dir, path, e);
            return;
        },
    };
    let legacy_name = legacy_tmp_name(name);
    for entry in entries.flatten() {
        let entry_name = entry.file_name();
        let stale = if legacy && entry_name == legacy_name {
            true
        } else if is_tmp_of(&entry_name, name) {
            match entry.metadata().and_then(|m| m.modified()) {
                // An mtime ahead of the clock reads as fresh, never as stale.
                Ok(t) => match SystemTime::now().duration_since(t) {
                    Ok(age) => age >= sweep_age,
                    Err(_)  => false,
                },
                Err(_) => false,
            }
        } else {
            false
        };
        if stale {
            match fs::remove_file(entry.path()) {
                Ok(()) => (),
                // Another writer's sweep got there first.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => {
                    warn!("Could not remove the leftover temporary file {:?}: {}.",
                        entry.path(), e);
                },
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct TextFileState {
    pub path:       String,
    pub line_num:   usize,
}

impl fmt::Display for TextFileState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.path, self.line_num)
    }
}


#[cfg(all(test, unix))]
mod tests {
    use super::*;

    use std::{
        io::Read,
        os::unix::fs::PermissionsExt,
        process::{
            Child,
            Command,
            ExitStatus,
            Stdio,
        },
        sync::{
            Arc,
            Barrier,
            atomic::{
                AtomicU64,
                Ordering,
            },
        },
        thread,
        time::{
            Duration,
            Instant,
            SystemTime,
        },
    };

    // Combined with the PID this gives each test a scratch path that cannot
    // collide, even when the suite runs across threads.
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn scratch_path(label: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(fmt!(
            "fe2o3_core_file_secret_test_{}_{}_{}", std::process::id(), n, label,
        ))
    }

    fn mode_of(path: &Path) -> Outcome<u32> {
        let meta = match fs::metadata(path) {
            Ok(m) => m,
            Err(e) => return Err(err!(e,
                "Could not stat {:?}.", path;
                Test, File, IO, Read)),
        };
        Ok(meta.permissions().mode() & 0o777)
    }

    // Set only inside the re-exec'd child below, so it knows to run the real
    // check instead of spawning a further child.
    const UMASK_CHILD_ENV: &str = "FE2O3_CORE_TEST_UMASK_CHILD";

    /// A permissive umask must not leak into the secret's mode: 0600 has no
    /// group or other bits, so no umask can widen it, but only if the mode
    /// is requested at creation rather than left to the default and chmodded
    /// after.
    ///
    /// `umask` is process-wide and this codebase avoids `unsafe`, so nothing
    /// here calls it directly. Instead the test re-executes its own test
    /// binary as a child process, letting `sh` set the umask before
    /// `exec`-ing into it filtered to just this one test; that avoids both
    /// an `unsafe` libc call and any race with other tests in this binary,
    /// which never sees its umask changed at all.
    #[test]
    fn test_save_secret_ignores_a_permissive_umask() -> Outcome<()> {
        if std::env::var(UMASK_CHILD_ENV).is_ok() {
            // Inside the re-exec'd child: the shell already set umask 002
            // before handing control to this binary, so just run the check.
            let path = scratch_path("permissive_umask");
            res!(save_secret(&path, b"top secret"));
            let mode = res!(mode_of(&path));
            let _ = fs::remove_file(&path);
            if mode != 0o600 {
                return Err(err!(
                    "{:?} ended at mode {:o} under umask 0o002, not 0600.", path, mode;
                    Test, Mismatch));
            }
            return Ok(());
        }

        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => return Err(err!(e,
                "Could not find this test binary's own path to re-exec it under a set umask.";
                Test, File, IO)),
        };
        let test_name = "file::tests::test_save_secret_ignores_a_permissive_umask";
        let script = fmt!("umask 002 && exec \"$0\" --exact {}", test_name);
        let output = match Command::new("sh")
            .arg("-c")
            .arg(&script)
            .arg(&exe)
            .env(UMASK_CHILD_ENV, "1")
            .output()
        {
            Ok(o) => o,
            Err(e) => return Err(err!(e,
                "Could not spawn the umask-002 child re-running {:?}.", exe;
                Test, IO)),
        };
        if !output.status.success() {
            return Err(err!(
                "The umask-002 child ({:?} --exact {}) failed: {:?}.", exe, test_name, output.status;
                Test, Mismatch));
        }
        // `--exact {test_name}` matching nothing -- for example after a rename of
        // this very test -- also exits 0, reporting "0 passed" for "running 0
        // tests". That would make this test vacuously pass forever, so require
        // the child to say it ran exactly the one test.
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !stdout.contains("1 passed") {
            return Err(err!(
                "The umask-002 child ({:?} --exact {}) reported no matching test, so \
                nothing was actually checked under umask 002. Child stdout: {}",
                exe, test_name, stdout;
                Test, Mismatch));
        }
        Ok(())
    }

    /// An existing, more permissive file must still end at 0600: the rename
    /// over it replaces its mode along with its contents.
    #[test]
    fn test_save_secret_restricts_an_existing_permissive_file() -> Outcome<()> {
        let path = scratch_path("existing_permissive");
        if let Err(e) = fs::write(&path, b"old, world-readable content") {
            return Err(err!(e, "Could not pre-seed {:?}.", path; Test, File, IO, Write));
        }
        if let Err(e) = fs::set_permissions(&path, fs::Permissions::from_mode(0o644)) {
            return Err(err!(e, "Could not set 0644 on {:?}.", path; Test, File, IO));
        }

        res!(save_secret(&path, b"new secret"));

        let mode = res!(mode_of(&path));
        let contents = fs::read(&path);
        let _ = fs::remove_file(&path);
        match contents {
            Ok(c) if c == b"new secret" => (),
            Ok(c) => return Err(err!(
                "{:?} held {:?} after save_secret, not the new bytes.", path, c;
                Test, Mismatch)),
            Err(e) => return Err(err!(e, "Could not read back {:?}.", path; Test, File, IO)),
        }
        if mode != 0o600 {
            return Err(err!(
                "{:?} was 0644 before saving and ended at {:o}, not 0600.", path, mode;
                Test, Mismatch));
        }
        Ok(())
    }

    /// A legacy `.tmp` sibling, left at a wide mode by an interrupted write on
    /// older code, must not leak that mode into the finished file, and is swept
    /// once the save has landed, since no writer uses that name any more.
    #[test]
    fn test_save_secret_ignores_a_stale_permissive_tmp_file() -> Outcome<()> {
        let path = scratch_path("stale_tmp");
        let tmp = path.with_file_name(legacy_tmp_name(&res!(file_name_of(&path))));
        if let Err(e) = fs::write(&tmp, b"leftover from a killed run") {
            return Err(err!(e, "Could not pre-seed {:?}.", tmp; Test, File, IO, Write));
        }
        if let Err(e) = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644)) {
            return Err(err!(e, "Could not set 0644 on {:?}.", tmp; Test, File, IO));
        }

        res!(save_secret(&path, b"new secret"));

        let mode = res!(mode_of(&path));
        let contents = fs::read(&path);
        let left = tmp.exists();
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&tmp);
        match contents {
            Ok(c) if c == b"new secret" => (),
            Ok(c) => return Err(err!(
                "{:?} held {:?} after save_secret, not the new bytes.", path, c;
                Test, Mismatch)),
            Err(e) => return Err(err!(e, "Could not read back {:?}.", path; Test, File, IO)),
        }
        if mode != 0o600 {
            return Err(err!(
                "{:?} ended at {:o} despite a 0644 stale .tmp, not 0600.", path, mode;
                Test, Mismatch));
        }
        if left {
            return Err(err!(
                "The legacy {:?} was still there after a save landed.", tmp;
                Test, Mismatch));
        }
        Ok(())
    }

    /// The sweep after a save takes the legacy `.tmp` and a writer's tmp older
    /// than the bound, and nothing else: not a fresh one, which may be a live
    /// writer's, and not a name that only resembles one.
    #[test]
    fn test_save_secret_sweeps_only_stale_leftovers() -> Outcome<()> {
        let dir = scratch_path("sweep");
        res!(fs::create_dir(&dir));
        let outcome = sweeps_only_stale_leftovers(&dir);
        let _ = fs::remove_dir_all(&dir);
        outcome
    }

    fn sweeps_only_stale_leftovers(dir: &Path) -> Outcome<()> {
        let key = dir.join("key");
        let old = res!(SystemTime::now()
            .checked_sub(TMP_SWEEP_AGE + Duration::from_secs(60))
            .ok_or_else(|| err!("The clock is too early to backdate a file."; Test, Invalid)));
        // Name, backdated past the bound, swept.
        let cases = [
            ("key.tmp",         false,  true),
            ("key.1.2.tmp",     true,   true),
            ("key.3.4.tmp",     false,  false),
            ("key.x.tmp",       true,   false),
            ("key.1.2.3.tmp",   true,   false),
            ("key.1.2.tmp.bak", true,   false),
            ("other.1.2.tmp",   true,   false),
        ];
        for (name, aged, _) in cases {
            let p = dir.join(name);
            res!(fs::write(&p, b"leftover"));
            if aged {
                let f = res!(OpenOptions::new().write(true).open(&p));
                res!(f.set_modified(old));
            }
        }

        res!(save_secret(&key, b"new secret"));

        for (name, aged, swept) in cases {
            let left = dir.join(name).exists();
            if left == swept {
                return Err(err!(
                    "After a save, {:?} (backdated past the bound: {}) was {}.",
                    name, aged, if left { "left, but should have been swept" }
                        else { "swept, but should have been left" };
                    Test, Mismatch));
            }
        }
        let contents = res!(fs::read(&key));
        let mode = res!(mode_of(&key));
        if contents != b"new secret" || mode != 0o600 {
            return Err(err!(
                "{:?} held {:?} at {:04o} after the save, not the new bytes at 0600.",
                key, contents, mode;
                Test, Mismatch));
        }
        Ok(())
    }

    /// A wider existing mode is narrowed to 0600, with the content untouched.
    #[test]
    fn test_restrict_secret_narrows_a_wide_mode() -> Outcome<()> {
        let path = scratch_path("restrict_wide");
        if let Err(e) = fs::write(&path, b"pre-existing key material") {
            return Err(err!(e, "Could not pre-seed {:?}.", path; Test, File, IO, Write));
        }
        if let Err(e) = fs::set_permissions(&path, fs::Permissions::from_mode(0o664)) {
            return Err(err!(e, "Could not set 0664 on {:?}.", path; Test, File, IO));
        }

        res!(restrict_secret(&path));

        let mode = res!(mode_of(&path));
        let contents = fs::read(&path);
        let _ = fs::remove_file(&path);
        match contents {
            Ok(c) if c == b"pre-existing key material" => (),
            Ok(c) => return Err(err!(
                "{:?} held {:?} after restrict_secret, which must not touch content.", path, c;
                Test, Mismatch)),
            Err(e) => return Err(err!(e, "Could not read back {:?}.", path; Test, File, IO)),
        }
        if mode != 0o600 {
            return Err(err!(
                "{:?} was 0664 and ended at {:o} after restrict_secret, not 0600.", path, mode;
                Test, Mismatch));
        }
        Ok(())
    }

    /// The caller is told the mode a file was narrowed from, and `None` when
    /// there was nothing to narrow, so an app that silences the log can still
    /// say so itself.
    #[test]
    fn test_restrict_secret_reports_the_mode_it_narrowed_from() -> Outcome<()> {
        let path = scratch_path("restrict_reports");
        if let Err(e) = fs::write(&path, b"key material") {
            return Err(err!(e, "Could not pre-seed {:?}.", path; Test, File, IO, Write));
        }
        // Mode before, what restrict_secret must report, mode after.
        let cases = [
            (0o644, Some(0o644),    0o600),
            (0o600, None,           0o600),
            (0o440, Some(0o440),    0o400),
            (0o400, None,           0o400),
        ];
        let mut outcome = Ok(());
        for (before, said, after) in cases {
            if let Err(e) = fs::set_permissions(&path, fs::Permissions::from_mode(before)) {
                outcome = Err(err!(e, "Could not set {:04o} on {:?}.", before, path; Test, File, IO));
                break;
            }
            let got = match restrict_secret(&path) {
                Ok(g) => g,
                Err(e) => { outcome = Err(e); break; },
            };
            let mode = match mode_of(&path) {
                Ok(m) => m,
                Err(e) => { outcome = Err(e); break; },
            };
            if got != said || mode != after {
                outcome = Err(err!(
                    "{:?} at {:04o}: restrict_secret reported {:?} and left {:04o}, \
                    expected {:?} and {:04o}.", path, before, got, mode, said, after;
                    Test, Mismatch));
                break;
            }
        }
        let _ = fs::remove_file(&path);
        outcome
    }

    /// A mode already at or narrower than 0600 is left exactly as it is.
    #[test]
    fn test_restrict_secret_is_a_noop_when_already_narrow() -> Outcome<()> {
        let path = scratch_path("restrict_already_narrow");
        if let Err(e) = fs::write(&path, b"already tight") {
            return Err(err!(e, "Could not pre-seed {:?}.", path; Test, File, IO, Write));
        }
        if let Err(e) = fs::set_permissions(&path, fs::Permissions::from_mode(0o400)) {
            return Err(err!(e, "Could not set 0400 on {:?}.", path; Test, File, IO));
        }

        res!(restrict_secret(&path));

        let mode = res!(mode_of(&path));
        let _ = fs::remove_file(&path);
        if mode != 0o400 {
            return Err(err!(
                "{:?} was 0400 and ended at {:o} after restrict_secret, which should not widen it.",
                path, mode;
                Test, Mismatch));
        }
        Ok(())
    }

    /// A key directory is narrowed to its owner's bits, search bit included,
    /// and never widened; a file is refused rather than given a directory's
    /// mask.
    #[test]
    fn test_restrict_secret_dir_narrows_to_the_owner() -> Outcome<()> {
        let dir = scratch_path("restrict_dir");
        res!(fs::create_dir(&dir));
        // Mode before, what restrict_secret_dir must report, mode after.
        let cases = [
            (0o775, Some(0o775),    0o700),
            (0o700, None,           0o700),
            (0o500, None,           0o500),
            (0o750, Some(0o750),    0o700),
        ];
        let mut outcome = Ok(());
        for (before, said, after) in cases {
            if let Err(e) = fs::set_permissions(&dir, fs::Permissions::from_mode(before)) {
                outcome = Err(err!(e, "Could not set {:04o} on {:?}.", before, dir; Test, File, IO));
                break;
            }
            let got = match restrict_secret_dir(&dir) {
                Ok(g) => g,
                Err(e) => { outcome = Err(e); break; },
            };
            let mode = match mode_of(&dir) {
                Ok(m) => m,
                Err(e) => { outcome = Err(e); break; },
            };
            if got != said || mode != after {
                outcome = Err(err!(
                    "{:?} at {:04o}: restrict_secret_dir reported {:?} and left {:04o}, \
                    expected {:?} and {:04o}.", dir, before, got, mode, said, after;
                    Test, Mismatch));
                break;
            }
        }
        if outcome.is_ok() {
            let file = dir.join("key");
            outcome = match fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
                .and_then(|()| fs::write(&file, b"key"))
            {
                Ok(()) => match restrict_secret_dir(&file) {
                    Ok(got) => Err(err!(
                        "restrict_secret_dir took the file {:?} for a directory: {:?}.", file, got;
                        Test, Mismatch)),
                    Err(_) => Ok(()),
                },
                Err(e) => Err(err!(e, "Could not seed {:?}.", file; Test, File, IO)),
            };
        }
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
        let _ = fs::remove_dir_all(&dir);
        outcome
    }

    // The race tests: two writers of one key path, and a reader that must only
    // ever find one of their payloads there, whole.
    const RACE_ROLE_ENV:    &str        = "FE2O3_CORE_TEST_RACE_ROLE";  // set only in a writer child
    const RACE_DIR_ENV:     &str        = "FE2O3_CORE_TEST_RACE_DIR";   // the directory both write in
    const RACE_LEN:         usize       = 1 << 20;                      // one payload, 1 MiB
    const RACE_SAVES:       u64         = 300;                          // saves per writer
    const RACE_CAP:         Duration    = Duration::from_secs(20);      // or until this has passed
    const RACE_HUNG:        Duration    = Duration::from_secs(60);      // a child this late is stuck
    const RACE_SWEPT_CAP:   Duration    = Duration::from_secs(30);      // rounds to find one swept writer

    /// What one writer's saves came to.
    #[derive(Debug, Default)]
    struct Tally {
        ok:     u64,
        swept:  u64,            // Errs tagged Missing: its tmp was swept away
        other:  u64,
        first:  Option<String>, // the first other Err, in words
    }

    impl Tally {
        fn count(&mut self, saved: Outcome<()>) {
            match saved {
                Ok(()) => self.ok += 1,
                Err(e) if e.tags().contains(&ErrTag::Missing) => self.swept += 1,
                Err(e) => {
                    self.other += 1;
                    if self.first.is_none() {
                        self.first = Some(e.plain());
                    }
                },
            }
        }
    }

    /// What the reader found each time it read the key.
    #[derive(Debug)]
    struct Reads {
        whole:  u64,
        torn:   u64,            // short, mixed, foreign, or gone once it existed
        first:  Option<String>, // the first torn read, described
        seen:   bool,           // the key has existed
        a:      Vec<u8>,        // writer A's payload, whole
        b:      Vec<u8>,        // writer B's
    }

    impl Reads {
        fn new() -> Self {
            Self {
                whole:  0,
                torn:   0,
                first:  None,
                seen:   false,
                a:      vec![0xA5; RACE_LEN],
                b:      vec![0x5A; RACE_LEN],
            }
        }

        fn take(&mut self, key: &Path) {
            let fault = match fs::read(key) {
                Ok(bytes) => {
                    self.seen = true;
                    // Compared whole first, a memcmp, so the reader keeps pace
                    // with the writers even in a debug build.
                    if bytes == self.a || bytes == self.b {
                        None
                    } else {
                        Some(torn(&bytes))
                    }
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && !self.seen => return,
                Err(e) => Some(fmt!("the read failed: {}", e)),
            };
            match fault {
                None => self.whole += 1,
                Some(what) => {
                    self.torn += 1;
                    if self.first.is_none() {
                        self.first = Some(what);
                    }
                },
            }
        }
    }

    /// What is wrong with a read of the key that is not one payload, whole.
    fn torn(bytes: &[u8]) -> String {
        let first = match bytes.first() {
            Some(b) => *b,
            None    => return fmt!("0 bytes"),
        };
        if first != 0xA5 && first != 0x5A {
            return fmt!("{} bytes, the first {:#04x}, neither payload", bytes.len(), first);
        }
        match bytes.iter().enumerate().find(|(_, b)| **b != first) {
            Some((i, b)) => fmt!("{} bytes, {:#04x} until offset {}, which holds {:#04x}",
                bytes.len(), first, i, b),
            None => fmt!("{} bytes of {:#04x}, not {}", bytes.len(), first, RACE_LEN),
        }
    }

    fn race_byte(role: &str) -> Outcome<u8> {
        match role {
            "A" => Ok(0xA5),
            "B" => Ok(0x5A),
            _   => Err(err!("Unknown race writer role {:?}.", role; Test, Invalid)),
        }
    }

    /// Saves one payload of `byte` to `key`, `RACE_SAVES` times or until
    /// `RACE_CAP` has passed, sweeping every tmp it finds when `aged`.
    fn race_writer(key: &Path, byte: u8, aged: bool) -> Tally {
        match aged {
            true    => race_writer_by(key, byte, |k, d| save_secret_aged(k, d, Duration::ZERO)),
            false   => race_writer_by(key, byte, save_secret),
        }
    }

    /// [`race_writer`] saving by `save`.
    fn race_writer_by(key: &Path, byte: u8, save: fn(&Path, &[u8]) -> Outcome<()>) -> Tally {
        let payload = vec![byte; RACE_LEN];
        let mut tally = Tally::default();
        let start = Instant::now();
        let mut n = 0;
        while n < RACE_SAVES && start.elapsed() < RACE_CAP {
            tally.count(save(key, &payload));
            n += 1;
        }
        tally
    }

    /// The body of a re-exec'd writer child: say it is ready, wait to be told
    /// to go, write, and print its tally for the parent to read.
    fn race_child(role: &str, aged: bool) -> Outcome<()> {
        let byte = res!(race_byte(role));
        let dir = PathBuf::from(res!(std::env::var(RACE_DIR_ENV)));
        res!(fs::write(dir.join(fmt!("ready.{}", role)), b""));
        let begun = Instant::now();
        while !dir.join("go").exists() {
            if begun.elapsed() > RACE_HUNG {
                return Err(err!("Writer {} was never told to go.", role; Test, Timeout));
            }
            thread::sleep(Duration::from_millis(1));
        }
        let tally = race_writer(&dir.join("key"), byte, aged);
        println!("race-writer {} ok={} swept={} other={}", role, tally.ok, tally.swept, tally.other);
        if let Some(first) = &tally.first {
            println!("race-writer {} first other Err: {}", role, first);
        }
        Ok(())
    }

    /// Writer children still running when a race test gives up are killed, so
    /// none outlives the test.
    struct Writers(Vec<Child>);

    impl Drop for Writers {
        fn drop(&mut self) {
            for kid in &mut self.0 {
                let _ = kid.kill();
                let _ = kid.wait();
            }
        }
    }

    /// Runs writer children A and B of `test` over one key in a fresh directory,
    /// reading the key until both have exited.
    fn race_processes(test: &str, label: &str) -> Outcome<(Reads, Vec<Tally>, Duration)> {
        let exe = res!(std::env::current_exe());
        let dir = scratch_path(label);
        res!(fs::create_dir(&dir));
        let outcome = race_processes_in(&exe, &dir, test);
        let _ = fs::remove_dir_all(&dir);
        outcome
    }

    fn race_processes_in(
        exe:    &Path,
        dir:    &Path,
        test:   &str,
    )
        -> Outcome<(Reads, Vec<Tally>, Duration)>
    {
        let mut writers = Writers(Vec::new());
        for role in ["A", "B"] {
            let kid = match Command::new(exe)
                .arg("--exact")
                .arg(test)
                .arg("--nocapture")
                .env(RACE_ROLE_ENV, role)
                .env(RACE_DIR_ENV, dir)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
            {
                Ok(k) => k,
                Err(e) => return Err(err!(e,
                    "Could not spawn writer child {} of {:?} --exact {}.", role, exe, test;
                    Test, IO)),
            };
            writers.0.push(kid);
        }
        // Both ready before either writes, so the two really overlap.
        let begun = Instant::now();
        while !(dir.join("ready.A").exists() && dir.join("ready.B").exists()) {
            for kid in &mut writers.0 {
                if let Some(status) = res!(kid.try_wait()) {
                    return Err(err!(
                        "A writer child of {} exited {:?} before it was ready: {}",
                        test, status, said(kid);
                        Test, Mismatch));
                }
            }
            if begun.elapsed() > RACE_HUNG {
                return Err(err!("The writer children of {} never became ready.", test; Test, Timeout));
            }
            thread::sleep(Duration::from_millis(1));
        }
        res!(fs::write(dir.join("go"), b""));

        let key = dir.join("key");
        let mut reads = Reads::new();
        let mut ended: Vec<Option<ExitStatus>> = vec![None, None];
        let start = Instant::now();
        while ended.iter().any(Option::is_none) {
            reads.take(&key);
            for (kid, end) in writers.0.iter_mut().zip(ended.iter_mut()) {
                if end.is_none() {
                    *end = res!(kid.try_wait());
                }
            }
            if start.elapsed() > RACE_HUNG {
                return Err(err!("The writer children of {} were still running after {:?}.",
                    test, RACE_HUNG; Test, Timeout));
            }
        }
        let took = start.elapsed();

        let mut tallies = Vec::new();
        for (kid, end) in writers.0.iter_mut().zip(ended.iter()) {
            let out = said(kid);
            if !end.map_or(false, |s| s.success()) || !out.contains("1 passed") {
                // Exiting 0 is not enough: `--exact` matching nothing also does.
                return Err(err!(
                    "A writer child of {} exited {:?} without running its one test: {}",
                    test, end, out;
                    Test, Mismatch));
            }
            tallies.push(res!(race_tally(&out)));
        }
        Ok((reads, tallies, took))
    }

    /// Everything a finished child wrote, stdout then stderr.
    fn said(kid: &mut Child) -> String {
        let mut out = String::new();
        if let Some(mut s) = kid.stdout.take() {
            let _ = s.read_to_string(&mut out);
        }
        if let Some(mut s) = kid.stderr.take() {
            let _ = s.read_to_string(&mut out);
        }
        out
    }

    /// Reads back the tally a writer child printed.
    fn race_tally(out: &str) -> Outcome<Tally> {
        let line = match out.lines().find(|l| l.starts_with("race-writer ") && l.contains(" ok=")) {
            Some(l) => l,
            None => return Err(err!("A writer child printed no tally: {}", out; Test, Missing)),
        };
        let mut tally = Tally::default();
        for word in line.split_whitespace() {
            if let Some((k, v)) = word.split_once('=') {
                let n: u64 = res!(v.parse());
                match k {
                    "ok"    => tally.ok = n,
                    "swept" => tally.swept = n,
                    "other" => tally.other = n,
                    _       => (),
                }
            }
        }
        tally.first = out.lines()
            .find_map(|l| l.split_once("first other Err: "))
            .map(|(_, e)| e.to_string());
        Ok(tally)
    }

    /// Every read whole and every save Ok, or the evidence that they were not.
    fn race_verdict(
        what:       &str,
        reads:      &Reads,
        tallies:    &[Tally],
        took:       Duration,
    )
        -> Outcome<()>
    {
        println!("{}: {} whole reads and {} torn in {:?}; writers {:?}",
            what, reads.whole, reads.torn, took, tallies);
        let errs: u64 = tallies.iter().map(|t| t.swept + t.other).sum();
        if reads.torn > 0 || errs > 0 {
            return Err(err!(
                "{}: {} of {} reads of the key were not one payload, whole, in {:?} (the \
                first: {:?}), and the writers' saves erred {} times: {:?}.",
                what, reads.torn, reads.torn + reads.whole, took, reads.first, errs, tallies;
                Test, Mismatch));
        }
        if reads.whole == 0 || tallies.iter().any(|t| t.ok == 0) {
            return Err(err!(
                "{}: nothing was checked, with {} whole reads and writers {:?}.",
                what, reads.whole, tallies;
                Test, Missing));
        }
        Ok(())
    }

    /// Two processes saving one key path, the kernel scheduling them, never
    /// leave it torn and are never refused: each renames a whole file of its
    /// own.
    #[test]
    fn test_save_secret_two_processes_never_tear_the_key() -> Outcome<()> {
        const TEST: &str = "file::tests::test_save_secret_two_processes_never_tear_the_key";
        if let Ok(role) = std::env::var(RACE_ROLE_ENV) {
            return race_child(&role, false);
        }
        let (reads, tallies, took) = res!(race_processes(TEST, "race_procs"));
        race_verdict("two processes", &reads, &tallies, took)
    }

    /// Two threads of one process share its pid, so only the per-process
    /// counter keeps their temporary names apart.
    #[test]
    fn test_save_secret_two_threads_never_tear_the_key() -> Outcome<()> {
        let dir = scratch_path("race_threads");
        res!(fs::create_dir(&dir));
        let outcome = race_threads(&dir, save_secret);
        let _ = fs::remove_dir_all(&dir);
        outcome
    }

    fn race_threads(dir: &Path, save: fn(&Path, &[u8]) -> Outcome<()>) -> Outcome<()> {
        let key = dir.join("key");
        let go = Arc::new(Barrier::new(3));
        let mut writers = Vec::new();
        for byte in [0xA5u8, 0x5A] {
            let (key, go) = (key.clone(), go.clone());
            writers.push(thread::spawn(move || {
                go.wait();
                race_writer_by(&key, byte, save)
            }));
        }
        go.wait();
        let mut reads = Reads::new();
        let start = Instant::now();
        while !writers.iter().all(|w| w.is_finished()) {
            reads.take(&key);
        }
        let took = start.elapsed();
        let mut tallies = Vec::new();
        for w in writers {
            match w.join() {
                Ok(t)   => tallies.push(t),
                Err(_)  => return Err(err!("A writer thread panicked."; Test, Thread)),
            }
        }
        race_verdict("two threads", &reads, &tallies, took)
    }

    /// With the sweep's bound at zero, each writer's sweep takes the other's
    /// live tmp. The writer that loses its tmp must say so with the `Missing`
    /// Err -- never a false Ok, never another failure -- and the key stays one
    /// payload, whole.
    #[test]
    fn test_save_secret_a_swept_writer_errs_honestly() -> Outcome<()> {
        const TEST: &str = "file::tests::test_save_secret_a_swept_writer_errs_honestly";
        if let Ok(role) = std::env::var(RACE_ROLE_ENV) {
            return race_child(&role, true);
        }
        let start = Instant::now();
        let mut rounds = 0;
        let mut swept = 0;
        while swept == 0 && start.elapsed() < RACE_SWEPT_CAP {
            let (reads, tallies, took) = res!(race_processes(TEST, "race_swept"));
            rounds += 1;
            println!("swept writers, round {}: {} whole reads and {} torn in {:?}; writers {:?}",
                rounds, reads.whole, reads.torn, took, tallies);
            let other: u64 = tallies.iter().map(|t| t.other).sum();
            let ok: u64 = tallies.iter().map(|t| t.ok).sum();
            if reads.torn > 0 || other > 0 || reads.whole == 0 || ok == 0 {
                return Err(err!(
                    "Swept writers, round {}: {} of {} reads of the key were not one payload, \
                    whole (the first: {:?}), and the writers came to {:?}, where only a \
                    Missing Err may stand beside Ok.",
                    rounds, reads.torn, reads.torn + reads.whole, reads.first, tallies;
                    Test, Mismatch));
            }
            swept += tallies.iter().map(|t| t.swept).sum::<u64>();
        }
        if swept == 0 {
            return Err(err!(
                "swept path not exercised in {} rounds over {:?}.", rounds, start.elapsed();
                Test, Missing));
        }
        Ok(())
    }

    // The atomic replacement of ordinary files, with a real kill as the oracle.

    /// `Keep` leaves a replaced file at the permission bits it held, which
    /// `fs::write` also does and a rename over it would otherwise not.
    #[test]
    fn test_save_atomic_keep_holds_the_mode_of_the_file_it_replaces() -> Outcome<()> {
        for bits in [0o600u32, 0o640, 0o664, 0o444, 0o755] {
            let path = scratch_path("keep_mode");
            res!(fs::write(&path, b"old"));
            res!(fs::set_permissions(&path, fs::Permissions::from_mode(bits)));
            let saved = save_atomic(&path, b"new bytes", SaveMode::Keep);
            let mode = mode_of(&path);
            let contents = fs::read(&path);
            let _ = fs::remove_file(&path);
            res!(saved);
            let mode = res!(mode);
            let contents = res!(contents);
            if mode != bits || contents != b"new bytes" {
                return Err(err!(
                    "A file at {:04o} held {:?} at {:04o} after a Keep save, not the new \
                    bytes at the same mode.", bits, contents, mode;
                    Test, Mismatch));
            }
        }
        Ok(())
    }

    /// A first save by `Keep` makes the file `fs::write` would have made, so
    /// the bits are whatever the process's umask leaves, and the oracle is
    /// `fs::write` itself.
    #[test]
    fn test_save_atomic_keep_makes_a_new_file_as_fs_write_does() -> Outcome<()> {
        let theirs = scratch_path("keep_new_theirs");
        let ours = scratch_path("keep_new_ours");
        res!(fs::write(&theirs, b"x"));
        let saved = save_atomic(&ours, b"x", SaveMode::Keep);
        let (want, got) = (mode_of(&theirs), mode_of(&ours));
        let _ = fs::remove_file(&theirs);
        let _ = fs::remove_file(&ours);
        res!(saved);
        let (want, got) = (res!(want), res!(got));
        if want != got {
            return Err(err!(
                "fs::write made a file at {:04o} and a Keep save made one at {:04o}.", want, got;
                Test, Mismatch));
        }
        Ok(())
    }

    /// `Owner` ends at 0600 whatever it replaces, the way `save_secret` does.
    #[test]
    fn test_save_atomic_owner_ends_at_0600_over_a_wider_file() -> Outcome<()> {
        let path = scratch_path("owner_mode");
        res!(fs::write(&path, b"old"));
        res!(fs::set_permissions(&path, fs::Permissions::from_mode(0o664)));
        let saved = save_atomic(&path, b"new bytes", SaveMode::Owner);
        let mode = mode_of(&path);
        let _ = fs::remove_file(&path);
        res!(saved);
        let mode = res!(mode);
        if mode != 0o600 {
            return Err(err!("An Owner save left {:04o}, not 0600.", mode; Test, Mismatch));
        }
        Ok(())
    }

    /// The sweep after a save takes this file's stale temporaries and nothing
    /// else, and in particular not `<name>.tmp`, which an ordinary neighbour may
    /// be using and only the old secret saves ever wrote.
    #[test]
    fn test_save_atomic_sweeps_only_its_own_stale_tmps() -> Outcome<()> {
        let dir = scratch_path("atomic_sweep");
        res!(fs::create_dir(&dir));
        let outcome = sweeps_its_own(&dir);
        let _ = fs::remove_dir_all(&dir);
        outcome
    }

    fn sweeps_its_own(dir: &Path) -> Outcome<()> {
        let file = dir.join("data");
        let old = res!(SystemTime::now()
            .checked_sub(TMP_SWEEP_AGE + Duration::from_secs(60))
            .ok_or_else(|| err!("The clock is too early to backdate a file."; Test, Invalid)));
        // Name, backdated past the bound, swept.
        let cases = [
            ("data.tmp",        true,   false),
            ("data.1.2.tmp",    true,   true),
            ("data.3.4.tmp",    false,  false),
            ("other.1.2.tmp",   true,   false),
        ];
        for (name, aged, _) in cases {
            let p = dir.join(name);
            res!(fs::write(&p, b"leftover"));
            if aged {
                let f = res!(OpenOptions::new().write(true).open(&p));
                res!(f.set_modified(old));
            }
        }
        res!(save_atomic(&file, b"new bytes", SaveMode::Keep));
        for (name, aged, swept) in cases {
            if dir.join(name).exists() == swept {
                return Err(err!(
                    "After a save, {:?} (backdated past the bound: {}) was {}.",
                    name, aged, if swept { "left, but should have been swept" }
                        else { "swept, but should have been left" };
                    Test, Mismatch));
            }
        }
        if res!(fs::read(&file)) != b"new bytes" {
            return Err(err!("{:?} did not hold the new bytes after the save.", file; Test, Mismatch));
        }
        Ok(())
    }

    /// A save leaves no temporary behind, and a symlink at the path is replaced
    /// by a file and not written through, so what it pointed at is untouched.
    #[test]
    fn test_save_atomic_replaces_a_symlink_and_leaves_its_target_alone() -> Outcome<()> {
        let dir = scratch_path("atomic_link");
        res!(fs::create_dir(&dir));
        let outcome = replaces_a_link(&dir);
        let _ = fs::remove_dir_all(&dir);
        outcome
    }

    fn replaces_a_link(dir: &Path) -> Outcome<()> {
        let target = dir.join("target");
        let link = dir.join("link");
        res!(fs::write(&target, b"the target"));
        res!(std::os::unix::fs::symlink(&target, &link));
        res!(save_atomic(&link, b"new bytes", SaveMode::Keep));
        let is_link = res!(fs::symlink_metadata(&link)).file_type().is_symlink();
        if is_link || res!(fs::read(&link)) != b"new bytes" || res!(fs::read(&target)) != b"the target" {
            return Err(err!(
                "After a save over a symlink, the path was still a link ({}) or held the wrong \
                bytes, or its target was written through.", is_link; Test, Mismatch));
        }
        let mut names: Vec<String> = Vec::new();
        for entry in res!(fs::read_dir(dir)) {
            names.push(res!(entry).file_name().to_string_lossy().into_owned());
        }
        names.sort();
        if names != ["link", "target"] {
            return Err(err!("A save left {:?} in the directory, not just the two files.", names;
                Test, Mismatch));
        }
        Ok(())
    }

    /// Two threads saving one path with `Keep` never leave it torn.
    #[test]
    fn test_save_atomic_two_threads_never_tear_the_file() -> Outcome<()> {
        let dir = scratch_path("atomic_threads");
        res!(fs::create_dir(&dir));
        let outcome = race_threads(&dir, keep);
        let _ = fs::remove_dir_all(&dir);
        outcome
    }

    fn keep(path: &Path, data: &[u8]) -> Outcome<()> {
        save_atomic(path, data, SaveMode::Keep)
    }

    // The kill tests: a writer child saves a 1 MiB payload over and over and the
    // parent SIGKILLs it at a random moment, which is what the kernel does to a
    // command that is killed. The control is `fs::write`, which truncates and
    // then writes.
    const KILL_ROLE_ENV:    &str    = "FE2O3_CORE_TEST_KILL_ROLE";   // set only in a writer child
    const KILLS:            u64     = 100;                           // kills of the atomic writer
    const KILLS_CAP:        u64     = 400;                           // kills of the control before giving up

    /// The body of a killed writer: save, announce the first save, and go on
    /// saving, until it is killed or has waited for the kill too long.
    fn kill_child(role: &str) -> Outcome<()> {
        let dir = PathBuf::from(res!(std::env::var(RACE_DIR_ENV)));
        let key = dir.join("key");
        let begun = Instant::now();
        let mut n = 0u64;
        while begun.elapsed() < RACE_HUNG {
            let payload = vec![if n % 2 == 0 { 0xA5u8 } else { 0x5A }; RACE_LEN];
            match role {
                "atomic"    => res!(keep(&key, &payload)),
                "plain"     => res!(fs::write(&key, &payload)),
                _           => return Err(err!("Unknown kill writer role {:?}.", role; Test, Invalid)),
            }
            if n == 0 {
                res!(fs::write(dir.join("ready"), b""));
            }
            n += 1;
        }
        Err(err!("Writer {:?} was never killed.", role; Test, Timeout))
    }

    /// Spawns a writer child, kills it `after` into its saving and says what the
    /// key held then: `None` for one payload, whole, else what it held.
    fn kill_once(exe: &Path, test: &str, role: &str, after: Duration) -> Outcome<Option<String>> {
        let dir = scratch_path("kill");
        res!(fs::create_dir(&dir));
        let outcome = kill_once_in(exe, &dir, test, role, after);
        let _ = fs::remove_dir_all(&dir);
        outcome
    }

    fn kill_once_in(
        exe:    &Path,
        dir:    &Path,
        test:   &str,
        role:   &str,
        after:  Duration,
    )
        -> Outcome<Option<String>>
    {
        let kid = match Command::new(exe)
            .arg("--exact")
            .arg(test)
            .env(KILL_ROLE_ENV, role)
            .env(RACE_DIR_ENV, dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(k) => k,
            Err(e) => return Err(err!(e,
                "Could not spawn the writer child of {:?} --exact {}.", exe, test; Test, IO)),
        };
        let mut writers = Writers(vec![kid]);
        let begun = Instant::now();
        while !dir.join("ready").exists() {
            if let Some(status) = res!(writers.0[0].try_wait()) {
                return Err(err!("The writer child of {} exited {:?} before its first save.",
                    test, status; Test, Mismatch));
            }
            if begun.elapsed() > RACE_HUNG {
                return Err(err!("The writer child of {} never made its first save.", test;
                    Test, Timeout));
            }
            thread::sleep(Duration::from_micros(200));
        }
        thread::sleep(after);
        // SIGKILL on unix, so the child gets no chance to finish a thing.
        res!(writers.0[0].kill());
        res!(writers.0[0].wait());
        let reads = Reads::new();
        match fs::read(dir.join("key")) {
            Ok(bytes) if bytes == reads.a || bytes == reads.b => Ok(None),
            Ok(bytes)   => Ok(Some(torn(&bytes))),
            Err(e)      => Ok(Some(fmt!("the key could not be read: {}", e))),
        }
    }

    /// A random wait of up to 40 ms, from the clock and a counter, so the kills
    /// fall all through the saves, several to a wait, and not at one moment of one.
    fn kill_after(i: u64) -> Duration {
        let t = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos() as u64);
        let mut x = t ^ i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        x ^= x >> 33;
        x = x.wrapping_mul(0xFF51_AFD7_ED558_CCD);
        x ^= x >> 33;
        Duration::from_micros(x % 40_000)
    }

    /// A SIGKILL at any moment of a `Keep` save leaves the file one whole
    /// payload, never empty and never a prefix of one.
    #[test]
    fn test_save_atomic_a_killed_writer_never_leaves_the_file_empty_or_torn() -> Outcome<()> {
        const TEST: &str =
            "file::tests::test_save_atomic_a_killed_writer_never_leaves_the_file_empty_or_torn";
        if let Ok(role) = std::env::var(KILL_ROLE_ENV) {
            return kill_child(&role);
        }
        let exe = res!(std::env::current_exe());
        for i in 0..KILLS {
            if let Some(what) = res!(kill_once(&exe, TEST, "atomic", kill_after(i))) {
                return Err(err!(
                    "Kill {} of {}: a SIGKILL left the file as {}, not one payload, whole.",
                    i + 1, KILLS, what; Test, Mismatch));
            }
        }
        Ok(())
    }

    /// The same kill, on `fs::write`, does leave a file empty or short, so the
    /// test above can see the fault it says is gone.
    #[test]
    fn test_a_plain_write_killed_the_same_way_leaves_the_file_torn() -> Outcome<()> {
        const TEST: &str = "file::tests::test_a_plain_write_killed_the_same_way_leaves_the_file_torn";
        if let Ok(role) = std::env::var(KILL_ROLE_ENV) {
            return kill_child(&role);
        }
        let exe = res!(std::env::current_exe());
        for i in 0..KILLS_CAP {
            if res!(kill_once(&exe, TEST, "plain", kill_after(i))).is_some() {
                return Ok(());
            }
        }
        Err(err!(
            "{} kills of a plain write left no torn file, so the kill tests cannot tell \
            a torn file from a whole one.", KILLS_CAP; Test, Missing))
    }
}
