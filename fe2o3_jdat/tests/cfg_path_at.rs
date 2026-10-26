//! `cfg::path_at`, one location value resolved under a root already chosen (kcfg F1, 26 Oct 2026).

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::cfg;

use std::path::{
    Path,
    PathBuf,
};


#[test]
fn path_at_absolute_rerooted() -> Outcome<()> {
    let root = Path::new("/srv/app");
    assert_eq!(res!(cfg::path_at("/var/lib/x", root, None)),    PathBuf::from("/var/lib/x"));
    // NormalPath turns /a/b into ./a/b, so the result must be re-rooted, and stay absolute.
    assert_eq!(res!(cfg::path_at("/var/../etc/./y/", root, None)), PathBuf::from("/etc/y"));
    assert_eq!(res!(cfg::path_at("data/db", root, None)),       PathBuf::from("/srv/app/data/db"));
    assert_eq!(res!(cfg::path_at("./data/../www", root, None)), PathBuf::from("/srv/app/www"));
    assert_eq!(res!(cfg::path_at(".", root, None)),             PathBuf::from("/srv/app"));
    Ok(())
}

#[test]
fn path_at_home_needs_home() -> Outcome<()> {
    let root = Path::new("/srv/app");
    let home = Path::new("/home/runner");
    assert_eq!(res!(cfg::path_at("~/var/x", root, Some(home))), PathBuf::from("/home/runner/var/x"));
    let e = match cfg::path_at("~/var/x", root, None) {
        Ok(p)  => return Err(err!("A '~/' value resolved to {:?} with no home.", p; Test)),
        Err(e) => e,
    };
    assert!(fmt!("{}", e).contains("~/var/x"), "{}", e);
    // The home is a root like any other: it must be absolute and may not be escaped.
    assert!(cfg::path_at("~/x", root, Some(Path::new("home/runner"))).is_err());
    assert!(cfg::path_at("~/../other", root, Some(home)).is_err());
    Ok(())
}

#[test]
fn path_at_refuses_escape_empty_nul_env_relative_root() -> Outcome<()> {
    let root = Path::new("/srv/app");
    for value in [
        "../etc",
        "data/../../etc",
        "",
        "data\0x",
        "{env:DB_DIR}",
        "x/{file:dir.txt}",
        "{file?:dir.txt}",
    ] {
        match cfg::path_at(value, root, None) {
            Ok(p)  => return Err(err!("{:?} resolved to {:?} and should be refused.", value, p; Test)),
            Err(e) => if !value.is_empty() && !value.contains('\0') {
                assert!(fmt!("{}", e).contains(value), "the error does not name {:?}: {}", value, e);
            },
        }
    }
    assert!(cfg::path_at("data", Path::new("srv/app"), None).is_err());
    assert!(cfg::path_at("data", Path::new(""), None).is_err());
    Ok(())
}
