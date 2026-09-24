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

// Secret temporary files
pub const SECRET_TMP_SWEEP_AGE: Duration    = Duration::from_secs(10 * 60); // a live writer's tmp is far younger
const SECRET_TMP_TRIES:         u64         = 16;                           // fresh names tried before giving up
static SECRET_TMP_SEQ:          AtomicU64   = AtomicU64::new(0);            // per process, so per thread too

/// Writes `data` to `path` as key material: atomically, and on unix at mode
/// 0600 whatever the caller's umask, so the bytes are never briefly readable by
/// anyone else and a crash never leaves a loose or partial file where the
/// secret should be.
///
/// The write lands on a sibling `<name>.<pid>.<n>.tmp`, where `n` counts saves
/// across the whole process, so no other writer -- another process, or another
/// thread of this one -- ever uses the same name. It is made with `create_new`,
/// at mode 0600 on unix (never `create` then `chmod`, which leaves a window at
/// the process's default mode), fsynced, then renamed over `path`. The rename
/// replaces whatever `path` held -- including its mode -- so a pre-existing,
/// more permissive file also ends at 0600. On unix the directory is fsynced
/// too, so the rename cannot survive a crash while the directory entry pointing
/// at it does not. The tmp is removed on every error path, so a failed save
/// never leaves the whole secret under a name nothing else will read.
///
/// Two writers of one `path` each rename a whole file of their own, so the last
/// rename wins whole, and no reader ever finds a torn key there.
///
/// Once its own rename has landed, a save sweeps, best-effort, what crashed
/// writers left beside `path`: the legacy `<name>.tmp`, which no writer uses any
/// more, and any `<name>.<digits>.<digits>.tmp` older than
/// [`SECRET_TMP_SWEEP_AGE`]. A failed sweep never fails the save. A writer that
/// stalls past that age between create and rename, or whose tmp a writer on
/// older code removes, finds its tmp gone at the rename: it returns an error
/// tagged `Missing` and leaves `path` as this call found it. `Ok` means this
/// call renamed over `path` a file it created and wrote itself.
pub fn save_secret(path: &Path, data: &[u8]) -> Outcome<()> {
    save_secret_aged(path, data, SECRET_TMP_SWEEP_AGE)
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
    let name = res!(secret_file_name(path));
    let (tmp, mut f) = res!(create_secret_tmp(path, &name));
    if let Err(e) = f.write_all(data) {
        let _ = fs::remove_file(&tmp);
        return Err(err!(e,
            "Could not write the temporary secret file {:?}.", tmp;
            File, IO, Write));
    }
    if let Err(e) = f.sync_all() {
        let _ = fs::remove_file(&tmp);
        return Err(err!(e,
            "Could not fsync the temporary secret file {:?}.", tmp;
            File, IO, Write));
    }
    // Closed before the rename, which off unix cannot move a file still open.
    drop(f);
    if let Err(e) = fs::rename(&tmp, path) {
        if e.kind() == std::io::ErrorKind::NotFound {
            // No other writer makes this name, so it is gone because another
            // writer's sweep took it, and the rename moved nothing.
            return Err(err!(e,
                "The temporary secret file {:?} is gone, most likely removed by another \
                writer's cleanup, so {:?} was not changed by this call.", tmp, path;
                File, IO, Missing));
        }
        let _ = fs::remove_file(&tmp);
        return Err(err!(e,
            "Could not rename {:?} to secret file {:?}.", tmp, path;
            File, IO, Write));
    }
    #[cfg(unix)]
    res!(sync_secret_parent_dir(path));
    sweep_secret_tmps(path, &name, sweep_age);
    Ok(())
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
/// `restrict_secret`'s job. A plain `create_dir_all` off unix, where there
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

/// Fsyncs the directory holding `path`, after the rename that lands a secret
/// there. Without this, the rename itself can survive a crash while the
/// directory entry pointing at it does not, which can bring back a file --
/// or the previous contents of one -- that was already reported saved.
#[cfg(unix)]
fn sync_secret_parent_dir(path: &Path) -> Outcome<()> {
    let dir = secret_parent_dir(path);
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

fn secret_parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    }
}

fn secret_file_name(path: &Path) -> Outcome<OsString> {
    match path.file_name() {
        Some(n) => Ok(n.to_os_string()),
        None => Err(err!(
            "Path {:?} has no file-name component; cannot save secret material.", path;
            Invalid, Input, Path)),
    }
}

/// Creates this writer's own temporary sibling of `path`. It is always a new
/// file, never an existing one, so on unix the 0600 asked for is the mode it
/// gets.
fn create_secret_tmp(path: &Path, name: &OsStr) -> Outcome<(PathBuf, File)> {
    let pid = std::process::id();
    for _ in 0..SECRET_TMP_TRIES {
        let n = SECRET_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = path.with_file_name(secret_tmp_name(name, pid, n));
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        match opts.open(&tmp) {
            Ok(f) => return Ok((tmp, f)),
            // Only a crashed writer whose pid has come round again, or a
            // namesake on another host sharing this directory, holds the name.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(err!(e,
                "Could not create the temporary secret file {:?}.", tmp;
                File, IO, Create)),
        }
    }
    Err(err!(
        "All {} fresh temporary names tried for the secret file {:?} were already taken.",
        SECRET_TMP_TRIES, path;
        File, IO, Create, Exists))
}

/// The temporary name writer `n` of process `pid` gives secret file `name`:
/// `<name>.<pid>.<n>.tmp`.
fn secret_tmp_name(name: &OsStr, pid: u32, n: u64) -> OsString {
    // Built as an `OsString`, not via `to_string_lossy`, so a non-UTF-8 file
    // name is not mangled into one that could collide with another file's.
    let mut tmp = name.to_os_string();
    tmp.push(fmt!(".{}.{}.tmp", pid, n));
    tmp
}

/// The one temporary name every writer used before names were made unique.
fn legacy_secret_tmp_name(name: &OsStr) -> OsString {
    let mut tmp = name.to_os_string();
    tmp.push(".tmp");
    tmp
}

/// Is `entry` strictly `<name>.<digits>.<digits>.tmp`, a writer's temporary
/// file for secret file `name`?
fn is_secret_tmp_of(entry: &OsStr, name: &OsStr) -> bool {
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

/// Removes, best-effort, what crashed writers of secret file `name` left beside
/// `path`: the legacy `<name>.tmp`, and each `<name>.<pid>.<n>.tmp` at least
/// `sweep_age` old. A live writer's file is younger than any sensible bound,
/// so it is left alone. Nothing here can fail a save that has already landed.
fn sweep_secret_tmps(path: &Path, name: &OsStr, sweep_age: Duration) {
    let dir = secret_parent_dir(path);
    let entries = match fs::read_dir(dir) {
        Ok(it) => it,
        Err(e) => {
            warn!("Could not list {:?} to sweep leftover temporary copies of {:?}: {}.",
                dir, path, e);
            return;
        },
    };
    let legacy = legacy_secret_tmp_name(name);
    for entry in entries.flatten() {
        let entry_name = entry.file_name();
        let stale = if entry_name == legacy {
            true
        } else if is_secret_tmp_of(&entry_name, name) {
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
                    warn!("Could not remove the leftover temporary secret file {:?}: {}.",
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
        let tmp = path.with_file_name(legacy_secret_tmp_name(&res!(secret_file_name(&path))));
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
            .checked_sub(SECRET_TMP_SWEEP_AGE + Duration::from_secs(60))
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
        let payload = vec![byte; RACE_LEN];
        let mut tally = Tally::default();
        let start = Instant::now();
        let mut n = 0;
        while n < RACE_SAVES && start.elapsed() < RACE_CAP {
            tally.count(match aged {
                true    => save_secret_aged(key, &payload, Duration::ZERO),
                false   => save_secret(key, &payload),
            });
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
        let outcome = race_threads(&dir);
        let _ = fs::remove_dir_all(&dir);
        outcome
    }

    fn race_threads(dir: &Path) -> Outcome<()> {
        let key = dir.join("key");
        let go = Arc::new(Barrier::new(3));
        let mut writers = Vec::new();
        for byte in [0xA5u8, 0x5A] {
            let (key, go) = (key.clone(), go.clone());
            writers.push(thread::spawn(move || {
                go.wait();
                race_writer(&key, byte, false)
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
}
