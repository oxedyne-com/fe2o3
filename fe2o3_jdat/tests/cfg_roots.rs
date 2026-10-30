//! `cfg::Roots`, an app's code, state and secret roots, each chosen by `resolve_root` and each
//! placing its values by `path_at` (SRV-LAYOUT G1, 30 Oct 2026).

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    cfg::{
        self,
        Roots,
    },
};

use std::path::{
    Path,
    PathBuf,
};


#[test]
fn roots_state_and_secret_default_to_app() -> Outcome<()> {
    let roots = res!(Roots::resolve("/srv/app", "", "", "FE2O3_ROOTS_T1"));
    assert_eq!(roots.app_root(),    Path::new("/srv/app"));
    assert_eq!(roots.state_root(),  Path::new("/srv/app"));
    assert_eq!(roots.secret_root(), Path::new("/srv/app"));
    // An unsplit app places every value exactly as path_at under its one root does.
    for value in ["o3db", "./logs", "/var/log/x", "a/../b"] {
        let one = res!(cfg::path_at(value, Path::new("/srv/app"), None));
        assert_eq!(res!(roots.app(value)),    one, "{}", value);
        assert_eq!(res!(roots.state(value)),  one, "{}", value);
        assert_eq!(res!(roots.secret(value)), one, "{}", value);
    }
    Ok(())
}

#[test]
fn roots_split_places_each_value_under_its_own_root() -> Outcome<()> {
    let roots = res!(Roots::resolve(
        "/srv/app", "/var/lib/app", "/etc/oxedyne/app", "FE2O3_ROOTS_T2"));
    assert_eq!(res!(roots.app("www")),          PathBuf::from("/srv/app/www"));
    assert_eq!(res!(roots.state("./o3db")),     PathBuf::from("/var/lib/app/o3db"));
    assert_eq!(res!(roots.secret("ctl.token")), PathBuf::from("/etc/oxedyne/app/ctl.token"));
    // An absolute value is used as written whichever root it is asked of.
    assert_eq!(res!(roots.state("/run/app/ctl.sock")), PathBuf::from("/run/app/ctl.sock"));
    Ok(())
}

#[test]
fn roots_each_spec_is_a_resolve_root_spec_with_its_own_env_var() -> Outcome<()> {
    std::env::set_var("FE2O3_ROOTS_T3_APP_ROOT",    "/srv/a");
    std::env::set_var("FE2O3_ROOTS_T3_STATE_ROOT",  "/var/lib/a");
    std::env::set_var("FE2O3_ROOTS_T3_SECRET_ROOT", "/etc/a");
    let roots = res!(Roots::resolve("env", "env", "env", "FE2O3_ROOTS_T3"));
    assert_eq!(roots.app_root(),    Path::new("/srv/a"));
    assert_eq!(roots.state_root(),  Path::new("/var/lib/a"));
    assert_eq!(roots.secret_root(), Path::new("/etc/a"));

    let cwd = res!(std::env::current_dir());
    let home = PathBuf::from(res!(std::env::var("HOME")));
    let roots = res!(Roots::resolve(".", "./state", "~/keys", "FE2O3_ROOTS_T3"));
    assert_eq!(roots.app_root(),    cwd.as_path());
    assert_eq!(roots.state_root(),  cwd.join("state").as_path());
    assert_eq!(roots.secret_root(), home.join("keys").as_path());

    std::env::remove_var("FE2O3_ROOTS_T3B_STATE_ROOT");
    let e = match Roots::resolve("/srv/a", "env", "", "FE2O3_ROOTS_T3B") {
        Ok(r)  => return Err(err!("An unset state env var resolved to {:?}.", r; Test)),
        Err(e) => e,
    };
    assert!(fmt!("{}", e).contains("FE2O3_ROOTS_T3B_STATE_ROOT"), "{}", e);
    Ok(())
}

#[test]
fn roots_refuse_a_relative_root_and_keep_path_at_checks() -> Outcome<()> {
    // resolve_root takes "rel/x" as written, so a relative root is refused here, by name.
    for (app, state, secret, which) in [
        ("rel/app", "",          "",        "app"),
        ("",        "",          "",        "app"),
        ("/srv/a",  "rel/state", "",        "state"),
        ("/srv/a",  "",          "rel/sec", "secret"),
    ] {
        match Roots::resolve(app, state, secret, "FE2O3_ROOTS_T4") {
            Ok(r)  => return Err(err!("{:?} resolved with a relative root: {:?}.",
                (app, state, secret), r; Test)),
            Err(e) => assert!(fmt!("{}", e).contains(which), "{}: {}", which, e),
        }
    }
    let roots = res!(Roots::resolve("/srv/a", "/var/lib/a", "/etc/a", "FE2O3_ROOTS_T4"));
    for value in ["../etc", "", "x\0y", "{env:X}", "{file:k}"] {
        assert!(roots.state(value).is_err(),  "state {:?}", value);
        assert!(roots.secret(value).is_err(), "secret {:?}", value);
    }
    // '~/' needs a home, given or from HOME; one given replaces it.
    let roots = roots.with_home(Some(PathBuf::from("/home/runner")));
    assert_eq!(res!(roots.state("~/x")), PathBuf::from("/home/runner/x"));
    let roots = roots.with_home(None);
    assert!(roots.state("~/x").is_err());
    Ok(())
}

#[test]
fn roots_resolve_file_references_under_the_secret_root() -> Outcome<()> {
    let base = std::env::temp_dir().join(fmt!("fe2o3_cfg_roots_{}", std::process::id()));
    let secret = base.join("secret");
    res!(std::fs::create_dir_all(&secret));
    res!(std::fs::write(secret.join("api.key"), "s3cret\n"));
    let roots = res!(Roots::resolve(
        &base.join("app").to_string_lossy(), "", &secret.to_string_lossy(), "FE2O3_ROOTS_T5"));
    let dat = Dat::List(vec![Dat::Str(fmt!("{{file:api.key}}")), Dat::U8(7)]);
    assert_eq!(res!(roots.resolve_secrets(&dat)),
        Dat::List(vec![Dat::Str(fmt!("s3cret")), Dat::U8(7)]));
    res!(std::fs::remove_dir_all(&base));
    Ok(())
}
