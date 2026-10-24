#![cfg(unix)]

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_core::file::{
    effective_uid,
    secure_secret_chain,
    secure_secret_file,
};

use std::{
    fs,
    os::unix::fs::{
        MetadataExt,
        PermissionsExt,
        symlink,
    },
    path::{
        Path,
        PathBuf,
    },
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
};


/// A directory that removes itself however the test ends.
struct Scratch {
    path: PathBuf, // where it is
}

impl Scratch {
    fn new(what: &str) -> Outcome<Self> {
        let stamp = res!(SystemTime::now().duration_since(UNIX_EPOCH));
        let path = std::env::temp_dir().join(fmt!(
            "fe2o3_chain_{}_{}_{}", what, std::process::id(), stamp.as_nanos()));
        res!(fs::create_dir_all(&path));
        Ok(Self { path })
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn mode(path: &Path) -> Outcome<u32> {
    Ok(res!(fs::metadata(path)).permissions().mode() & 0o777)
}

#[test]
fn test_effective_uid_owns_what_this_process_creates() -> Outcome<()> {
    let s = res!(Scratch::new("uid"));
    let file = s.path.join("mine");
    res!(fs::write(&file, b"x"));
    assert_eq!(res!(effective_uid()), res!(fs::metadata(&file)).uid());
    Ok(())
}

#[test]
fn test_secure_secret_chain_narrows_each_existing_component() -> Outcome<()> {
    let s = res!(Scratch::new("narrow"));
    let a = s.path.join("a");
    res!(fs::create_dir_all(a.join("b")));
    res!(fs::set_permissions(&a, fs::Permissions::from_mode(0o755)));
    res!(fs::set_permissions(a.join("b"), fs::Permissions::from_mode(0o777)));
    // The last component is not there yet, which ends the walk.
    res!(secure_secret_chain(&s.path, Path::new("a/b/c/d")));
    assert_eq!(res!(mode(&a)), 0o700);
    assert_eq!(res!(mode(&a.join("b"))), 0o700);
    assert!(!a.join("b").join("c").exists());
    Ok(())
}

#[test]
fn test_secure_secret_chain_refuses_a_symbolic_link() -> Outcome<()> {
    let s = res!(Scratch::new("link"));
    let open = s.path.join("open");
    res!(fs::create_dir_all(&open));
    res!(fs::set_permissions(&open, fs::Permissions::from_mode(0o777)));
    res!(fs::create_dir_all(s.path.join("a")));
    let link = s.path.join("a").join("b");
    res!(symlink(&open, &link));
    let msg = match secure_secret_chain(&s.path, Path::new("a/b/c")) {
        Ok(())  => return Err(err!("A link in the chain was not refused."; Test, Missing)),
        Err(e)  => fmt!("{}", e),
    };
    assert!(msg.contains(&fmt!("{:?}", link)), "{}", msg);
    assert!(msg.contains("symbolic link"), "{}", msg);
    assert_eq!(res!(mode(&open)), 0o777, "nothing beyond the link was touched");
    Ok(())
}

#[test]
fn test_secure_secret_chain_refuses_another_users_directory() -> Outcome<()> {
    // `/tmp` is root's; as root there is nobody else to be, so nothing is asked.
    if res!(effective_uid()) == 0 {
        return Ok(());
    }
    let msg = match secure_secret_chain(Path::new("/"), Path::new("tmp/x")) {
        Ok(())  => return Err(err!("Root's directory was not refused."; Test, Missing)),
        Err(e)  => fmt!("{}", e),
    };
    assert!(msg.contains("\"/tmp\"") && msg.contains("owned by user 0"), "{}", msg);
    Ok(())
}

#[test]
fn test_secure_secret_file_refuses_a_link_and_passes_a_missing_file() -> Outcome<()> {
    let s = res!(Scratch::new("file"));
    res!(secure_secret_file(&s.path.join("absent")));
    let real = s.path.join("real");
    res!(fs::write(&real, b"k"));
    res!(secure_secret_file(&real));
    let link = s.path.join("key");
    res!(symlink(&real, &link));
    assert!(secure_secret_file(&link).is_err());
    if res!(effective_uid()) != 0 {
        assert!(secure_secret_file(Path::new("/etc/passwd")).is_err());
    }
    Ok(())
}
