use crate::{
    prelude::*,
    path::NormalPath,
};

use std::{
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

/// Writes `data` to `path` as key material: atomically, and at mode 0600
/// whatever the caller's umask, so the bytes are never briefly readable by
/// anyone else and a crash never leaves a loose or partial file where the
/// secret should be.
///
/// The write lands on a `.tmp` sibling of `path`, created with mode 0600
/// directly (never `create` then `chmod`, which leaves a window at the
/// creating process's default mode), fsynced, then renamed over `path`. The
/// rename replaces whatever `path` held -- including its mode -- so a
/// pre-existing, more permissive file also ends at 0600. The directory is
/// fsynced too, so the rename cannot survive a crash while the directory
/// entry pointing at it does not; and the `.tmp` is removed on every error
/// path, so a failed save never leaves the whole secret sitting under a
/// name nothing else will read.
#[cfg(unix)]
pub fn save_secret(path: &Path, data: &[u8]) -> Outcome<()> {
    use std::os::unix::fs::OpenOptionsExt;

    let tmp = res!(secret_tmp_path(path));
    // A `.tmp` left behind by an interrupted previous write may already
    // exist at whatever mode that run's environment gave it. `.mode(0o600)`
    // below is only honoured for a file `open` actually creates, so remove
    // any leftover first -- otherwise reopening it would keep its old,
    // possibly wider, permissions instead of the 0600 this call promises.
    match fs::remove_file(&tmp) {
        Ok(())                                              => {},
        Err(e) if e.kind() == std::io::ErrorKind::NotFound  => {},
        Err(e) => return Err(err!(e,
            "Could not remove the stale temporary secret file {:?}.", tmp;
            File, IO, Write)),
    }
    {
        let mut f = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
        {
            Ok(f) => f,
            Err(e) => return Err(err!(e,
                "Could not create the temporary secret file {:?}.", tmp;
                File, IO, Create)),
        };
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
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(err!(e,
            "Could not rename {:?} to secret file {:?}.", tmp, path;
            File, IO, Write));
    }
    res!(sync_secret_parent_dir(path));
    Ok(())
}

/// Writes `data` to `path` atomically. No POSIX mode bits exist to restrict
/// here, so this platform gets the write-then-rename without the 0600
/// guarantee the unix build makes.
#[cfg(not(unix))]
pub fn save_secret(path: &Path, data: &[u8]) -> Outcome<()> {
    let tmp = res!(secret_tmp_path(path));
    if let Err(e) = fs::write(&tmp, data) {
        let _ = fs::remove_file(&tmp);
        return Err(err!(e,
            "Could not write the temporary secret file {:?}.", tmp;
            File, IO, Write));
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(err!(e,
            "Could not rename {:?} to secret file {:?}.", tmp, path;
            File, IO, Write));
    }
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

/// Narrows an existing file's mode to 0600 if it is currently wider, for a
/// key file that predates this codebase's atomic `save_secret` writes, or
/// that arrived by some other route -- a backup restore, an `scp`, a
/// deploy step -- at whatever mode its source held.
///
/// Unlike widening a mode, narrowing one has no window to close: the file
/// already exists at its current mode throughout, and `chmod` only ever
/// removes bits, so there is no intermediate state where the file is any
/// more exposed than it already was. A no-op when the mode is already 0600
/// or narrower, and a no-op entirely off unix, where there are no POSIX mode
/// bits to narrow.
#[cfg(unix)]
pub fn restrict_secret(path: &Path) -> Outcome<()> {
    use std::os::unix::fs::PermissionsExt;

    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) => return Err(err!(e,
            "Could not stat {:?} to check whether its mode needs narrowing.", path;
            File, IO, Read)),
    };
    let mode = meta.permissions().mode() & 0o777;
    if mode & !0o600 == 0 {
        return Ok(());
    }
    warn!("Narrowing key file {:?} from mode {:04o} to 0600.", path, mode);
    if let Err(e) = fs::set_permissions(path, fs::Permissions::from_mode(0o600)) {
        return Err(err!(e,
            "Could not narrow {:?} from mode {:04o} to 0600.", path, mode;
            File, IO, Write));
    }
    Ok(())
}

/// A no-op off unix: there are no POSIX mode bits to narrow.
#[cfg(not(unix))]
pub fn restrict_secret(_path: &Path) -> Outcome<()> {
    Ok(())
}

/// Fsyncs the directory holding `path`, after the rename that lands a secret
/// there. Without this, the rename itself can survive a crash while the
/// directory entry pointing at it does not, which can bring back a file --
/// or the previous contents of one -- that was already reported saved.
#[cfg(unix)]
fn sync_secret_parent_dir(path: &Path) -> Outcome<()> {
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
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

/// The sibling `.tmp` path a secret write lands on before the rename.
fn secret_tmp_path(path: &Path) -> Outcome<PathBuf> {
    let file_name = match path.file_name() {
        Some(n) => n.to_os_string(),
        None => return Err(err!(
            "Path {:?} has no file-name component; cannot save secret material.", path;
            Invalid, Input, Path)),
    };
    // Built as an `OsString`, not via `to_string_lossy`, so a non-UTF-8 file
    // name is not mangled into one that could collide with another file's.
    let mut tmp_name = file_name;
    tmp_name.push(".tmp");
    let mut tmp = path.to_path_buf();
    tmp.set_file_name(tmp_name);
    Ok(tmp)
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
        os::unix::fs::PermissionsExt,
        process::Command,
        sync::atomic::{
            AtomicU64,
            Ordering,
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

    /// A `.tmp` sibling left behind at a wide mode by an interrupted prior
    /// write -- not the target `path` itself -- must not leak that mode into
    /// the finished file: `open`'s `mode(0o600)` is only honoured on
    /// creation, so a stale, reused `.tmp` would otherwise keep its old bits.
    #[test]
    fn test_save_secret_ignores_a_stale_permissive_tmp_file() -> Outcome<()> {
        let path = scratch_path("stale_tmp");
        let tmp = res!(secret_tmp_path(&path));
        if let Err(e) = fs::write(&tmp, b"leftover from a killed run") {
            return Err(err!(e, "Could not pre-seed {:?}.", tmp; Test, File, IO, Write));
        }
        if let Err(e) = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644)) {
            return Err(err!(e, "Could not set 0644 on {:?}.", tmp; Test, File, IO));
        }

        res!(save_secret(&path, b"new secret"));

        let mode = res!(mode_of(&path));
        let contents = fs::read(&path);
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
}
