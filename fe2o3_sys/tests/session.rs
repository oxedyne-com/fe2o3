//! Tests for `fe2o3_sys::session`: the check after a drop, the identity lookup and the ownership
//! test, against the running process and real files.
//!
//! The drop itself needs root. Its two arms are ignored here and run as root in the VM (BR-4e S6) with
//! `cargo test --features session -- --ignored`; it drops a child copy of this test binary, so the
//! harness keeps its own ids.

#![cfg(all(target_os = "linux", feature = "session"))]

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_sys::session::{
    check_dropped,
    drop_to,
    strict_entry,
    strict_owner,
    Identity,
};

use std::{
    env,
    fs,
    os::unix::fs::{
        symlink,
        MetadataExt,
    },
    path::{
        Path,
        PathBuf,
    },
    process::Command,
};

fn scratch(tag: &str) -> Outcome<PathBuf> {
    let dir = env::temp_dir().join(format!("fe2o3-sys-session-{}-{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    res!(fs::create_dir_all(&dir));
    Ok(dir)
}

fn is_root() -> Outcome<bool> {
    Ok(res!(Identity::current()).uid == 0)
}

#[test]
fn the_current_identity_passes_its_own_check() -> Outcome<()> {
    let me = res!(Identity::current());
    res!(check_dropped(&me));
    Ok(())
}

#[test]
fn a_different_uid_fails_the_check() -> Outcome<()> {
    let me = res!(Identity::current());
    let other = Identity::new(me.uid.wrapping_add(1), me.gid, me.groups.clone());
    match check_dropped(&other) {
        Ok(())  => Err(err!("check_dropped passed uid {} for a process of uid {}.", other.uid, me.uid; Test)),
        Err(e)  => {
            let msg = format!("{}", e);
            if msg.contains("did not hold") { Ok(()) }
            else { Err(err!("Unexpected refusal: {}", msg; Test)) }
        }
    }
}

#[test]
fn a_different_gid_fails_the_check() -> Outcome<()> {
    let me = res!(Identity::current());
    let gid = me.gid.wrapping_add(1);
    let mut groups = me.groups.clone();
    groups.push(gid);
    let other = Identity::new(me.uid, gid, groups);
    if check_dropped(&other).is_ok() {
        return Err(err!("check_dropped passed gid {} for a process of gid {}.", gid, me.gid; Test));
    }
    Ok(())
}

#[test]
fn an_extra_group_fails_the_check() -> Outcome<()> {
    let me = res!(Identity::current());
    let extra = (60000..65000).find(|g| !me.groups.contains(g)).unwrap_or(65001);
    let mut groups = me.groups.clone();
    groups.push(extra);
    let other = Identity::new(me.uid, me.gid, groups);
    if check_dropped(&other).is_ok() {
        return Err(err!("check_dropped passed with group {} the process lacks.", extra; Test));
    }
    Ok(())
}

#[test]
fn a_missing_group_fails_the_check() -> Outcome<()> {
    let me = res!(Identity::current());
    let Some(drop) = me.groups.iter().copied().find(|g| *g != me.gid) else {
        return Ok(()); // Only the primary group: nothing to leave out.
    };
    let other = Identity::new(me.uid, me.gid, me.groups.iter().copied().filter(|g| *g != drop).collect());
    if check_dropped(&other).is_ok() {
        return Err(err!("check_dropped passed without group {} the process has.", drop; Test));
    }
    Ok(())
}

#[test]
fn identity_of_the_running_user_matches_its_ids() -> Outcome<()> {
    let me = res!(Identity::current());
    let name = res!(res!(nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(me.uid)))
        .ok_or_else(|| err!("No password entry for uid {}.", me.uid; Missing)));
    let id = res!(Identity::of_user(&name.name));
    if id.uid != me.uid || id.gid != name.gid.as_raw() || !id.groups.contains(&id.gid) {
        return Err(err!("of_user('{}') gave {:?} for uid {} gid {}.", name.name, id, me.uid, name.gid; Test));
    }
    Ok(())
}

#[test]
fn an_unknown_user_is_refused() -> Outcome<()> {
    if Identity::of_user("fe2o3-no-such-user-q7").is_ok() {
        return Err(err!("of_user found a user that does not exist."; Test));
    }
    Ok(())
}

#[test]
fn identity_groups_hold_the_gid_once_and_sorted() -> Outcome<()> {
    let id = Identity::new(1000, 1000, vec![27, 1000, 27, 4]);
    if id.groups != vec![4, 27, 1000] {
        return Err(err!("Groups were {:?}.", id.groups; Test));
    }
    let id = Identity::new(1000, 1001, vec![4]);
    if id.groups != vec![4, 1001] {
        return Err(err!("The gid was not added: {:?}.", id.groups; Test));
    }
    Ok(())
}

#[test]
fn an_unprivileged_drop_fails_loudly() -> Outcome<()> {
    if res!(is_root()) { return Ok(()); }
    if drop_to(&Identity::new(65534, 65534, vec![65534])).is_ok() {
        return Err(err!("A drop without privilege reported success."; Test));
    }
    // The failed drop left the process as it was.
    res!(check_dropped(&res!(Identity::current())));
    Ok(())
}

#[test]
fn root_owned_system_directories_pass() -> Outcome<()> {
    res!(strict_owner(Path::new("/usr/bin")));
    res!(strict_owner(Path::new("/")));
    Ok(())
}

#[test]
fn a_link_into_root_owned_files_passes_on_both_chains() -> Outcome<()> {
    // /bin is a root-owned link to usr/bin on a merged-/usr system, and plain elsewhere.
    res!(strict_owner(Path::new("/bin/sh")));
    Ok(())
}

#[test]
fn a_root_owned_link_to_a_user_owned_target_is_refused() -> Outcome<()> {
    // /dev/fd is a root-owned link to /proc/self/fd, and /proc/<pid> is owned by the test user:
    // the named chain passes, so only the resolved chain can refuse it.
    let link = Path::new("/dev/fd");
    let m = res!(fs::symlink_metadata(link));
    if !m.file_type().is_symlink() || m.uid() != 0 || res!(is_root()) {
        return Err(err!("This test needs /dev/fd to be a root-owned link and a test user other \
            than root."; Test));
    }
    match strict_owner(link) {
        Ok(())  => Err(err!("strict_owner passed /dev/fd, which resolves under /proc/<pid>."; Test)),
        Err(e)  => if e.to_string().contains("/proc/") {
            Ok(())
        } else {
            Err(err!("strict_owner refused /dev/fd for the wrong entry: {}", e; Test))
        },
    }
}

#[test]
fn a_world_writable_root_directory_is_refused() -> Outcome<()> {
    // /tmp is root-owned and mode 1777.
    match strict_owner(Path::new("/tmp")) {
        Ok(())  => Err(err!("strict_owner passed /tmp."; Test)),
        Err(e)  => {
            let msg = format!("{}", e);
            if msg.contains("writable by group or others") && msg.contains("'/tmp'") { Ok(()) }
            else { Err(err!("Unexpected refusal: {}", msg; Test)) }
        }
    }
}

#[test]
fn a_user_owned_file_is_refused_naming_the_first_bad_entry() -> Outcome<()> {
    if res!(is_root()) { return Ok(()); }
    let dir = res!(scratch("owner"));
    let file = dir.join("cfg");
    res!(fs::write(&file, b"x"));
    let out = strict_owner(&file);
    let _ = fs::remove_dir_all(&dir);
    match out {
        Ok(())  => Err(err!("strict_owner passed a file owned by the test user."; Test)),
        // The chain stops at the first entry above the file that fails, /tmp or the user's own.
        Err(e)  => {
            let msg = format!("{}", e);
            let first = file.ancestors().skip(1).find(|a| msg.contains(&format!("'{}'", a.display())));
            if first.is_some() { Ok(()) }
            else { Err(err!("Unexpected refusal: {}", msg; Test)) }
        }
    }
}

#[test]
fn a_user_owned_link_to_a_root_file_is_refused() -> Outcome<()> {
    if res!(is_root()) { return Ok(()); }
    let home = res!(env::var("HOME").map_err(|_| err!("HOME is not set."; Missing)));
    let dir = PathBuf::from(home).join(".cache").join(format!("fe2o3-sys-session-link-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    res!(fs::create_dir_all(&dir));
    let link = dir.join("sh");
    res!(symlink("/usr/bin/env", &link));
    let out = strict_owner(&link);
    let _ = fs::remove_dir_all(&dir);
    match out {
        Ok(())  => Err(err!("strict_owner passed a link in a user-owned directory."; Test)),
        Err(e)  => {
            let msg = format!("{}", e);
            if msg.contains("not root") { Ok(()) }
            else { Err(err!("Unexpected refusal: {}", msg; Test)) }
        }
    }
}

#[test]
fn a_relative_path_or_dotdot_is_refused() -> Outcome<()> {
    if strict_owner(Path::new("usr/bin")).is_ok() {
        return Err(err!("strict_owner passed a relative path."; Test));
    }
    if strict_owner(Path::new("/usr/../usr/bin")).is_ok() {
        return Err(err!("strict_owner passed a path with '..'."; Test));
    }
    Ok(())
}

#[test]
fn the_entry_test_reads_owner_and_mode() -> Outcome<()> {
    let p = Path::new("/x");
    res!(strict_entry(p, 0, 0o755, false));
    res!(strict_entry(p, 0, 0o777, true));     // A link's own mode means nothing.
    for (uid, mode, link) in [(1000, 0o755, false), (0, 0o775, false), (0, 0o757, false), (1000, 0o777, true)] {
        if strict_entry(p, uid, mode, link).is_ok() {
            return Err(err!("strict_entry passed uid {} mode {:o} link {}.", uid, mode, link; Test));
        }
    }
    Ok(())
}

const CHILD: &str = "FE2O3_SESSION_DROP_CHILD";

/// Runs the named root arm again in a child copy of this binary, which does the drop, so the
/// harness keeps its own ids.
fn run_as_child(name: &str) -> Outcome<()> {
    if !res!(is_root()) {
        return Err(err!("This arm must run as root."; Test));
    }
    let exe = res!(env::current_exe());
    let st = res!(Command::new(exe)
        .args(["--exact", name, "--ignored", "--test-threads=1"])
        .env(CHILD, "1")
        .status());
    if !st.success() {
        return Err(err!("The dropped child in '{}' failed: {}.", name, st; Test));
    }
    Ok(())
}

#[test]
#[ignore = "needs root: drops a child process to nobody; BR-4e S6 runs it in the VM"]
fn root_drops_to_nobody_and_cannot_return() -> Outcome<()> {
    if env::var(CHILD).is_ok() {
        let id = res!(Identity::of_user("nobody"));
        res!(drop_to(&id));
        let now = res!(Identity::current());
        if now != id {
            return Err(err!("After the drop the process is {:?}, not {:?}.", now, id; Test));
        }
        if nix::unistd::setuid(nix::unistd::Uid::from_raw(0)).is_ok() {
            return Err(err!("uid 0 was taken back after the drop."; Test));
        }
        return Ok(());
    }
    run_as_child("root_drops_to_nobody_and_cannot_return")
}

#[test]
#[ignore = "needs root: drops a child process; BR-4e S6 runs it in the VM"]
fn root_drop_that_misses_its_identity_is_refused() -> Outcome<()> {
    if env::var(CHILD).is_ok() {
        // Every call succeeds, but the groups asked for omit the gid, which the process then
        // holds; only the check after the calls can see that the drop missed.
        let id = Identity { uid: 65534, gid: 65534, groups: Vec::new() };
        return match drop_to(&id) {
            Ok(())  => Err(err!("drop_to passed a drop the process does not match, {:?}.", id; Test)),
            Err(_)  => Ok(()),
        };
    }
    run_as_child("root_drop_that_misses_its_identity_is_refused")
}

#[test]
fn the_account_of_the_current_uid_names_its_home_and_shell() -> Outcome<()> {
    let id = res!(Identity::current());
    let a = res!(oxedyne_fe2o3_sys::session::Account::of_uid(id.uid));
    assert_eq!(a.uid, id.uid);
    assert_eq!(res!(oxedyne_fe2o3_sys::session::uids())[0], id.uid);
    assert!(!a.name.is_empty());
    assert!(a.home.is_absolute(), "home {:?}", a.home);
    // A uid no system gives out is refused, not answered with a default.
    assert!(oxedyne_fe2o3_sys::session::Account::of_uid(4_000_000_017).is_err());
    Ok(())
}
