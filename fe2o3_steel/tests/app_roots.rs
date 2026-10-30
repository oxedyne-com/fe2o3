//! Steel's three roots (SRV-LAYOUT U3, D-20261030-22): with no `state_root` or `secret_root`
//! a config resolves exactly as before, and with them Steel writes under the state root and
//! reads its wallet from the secret root.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    cfg::Config,
};
use oxedyne_fe2o3_steel::{
    app::cfg::AppConfig,
    srv::cfg::ServerConfig,
};

use std::path::{
    Path,
    PathBuf,
};

/// A fresh directory under the test target's own scratch area.
fn scratch(name: &str) -> Outcome<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("app_roots").join(name);
    if dir.exists() {
        res!(std::fs::remove_dir_all(&dir));
    }
    res!(std::fs::create_dir_all(&dir));
    Ok(dir)
}

#[test]
fn a_config_without_the_new_keys_resolves_as_before() -> Outcome<()> {
    let app = res!(scratch("before"));
    let mut map = try_extract_dat!(AppConfig::to_datmap(res!(AppConfig::new())), Map);
    map.remove(&dat!("state_root"));
    map.remove(&dat!("secret_root"));
    map.insert(dat!("app_root"), dat!(app.to_string_lossy().to_string()));
    let mut cfg = res!(AppConfig::from_datmap(map));
    res!(cfg.check_and_fix());

    let roots = res!(cfg.roots());
    assert_eq!(roots.app_root(), app.as_path());
    assert_eq!(roots.state_root(), app.as_path());
    assert_eq!(roots.secret_root(), app.as_path());
    let cfg_dir = Path::new("/somewhere/else");
    assert_eq!(res!(cfg.wallet_dir(cfg_dir)), cfg_dir);

    let server_cfg = ServerConfig::default();
    assert_eq!(res!(roots.state(server_cfg.log_dir())), app.join("www").join("logs"));
    Ok(())
}

#[test]
fn named_roots_place_state_and_the_wallet_apart() -> Outcome<()> {
    let base = res!(scratch("apart"));
    let (app, state, secret) = (base.join("srv"), base.join("var"), base.join("etc"));
    res!(std::fs::create_dir_all(&secret));
    let mut cfg = res!(AppConfig::new());
    cfg.app_root = app.to_string_lossy().to_string();
    cfg.state_root = state.to_string_lossy().to_string();
    cfg.secret_root = secret.to_string_lossy().to_string();
    res!(cfg.check_and_fix());

    assert!(state.is_dir(), "check_and_fix creates the state root");
    let roots = res!(cfg.roots());
    assert_eq!(roots.state_root(), state.as_path());
    assert_eq!(roots.secret_root(), secret.as_path());
    assert_eq!(res!(cfg.wallet_dir(&app)), secret);

    let mut server_cfg = ServerConfig::default();
    assert_eq!(res!(roots.state(server_cfg.log_dir())), state.join("www").join("logs"));
    server_cfg.log_dir_rel = fmt!("./www-logs");
    assert_eq!(res!(roots.state(server_cfg.log_dir())), state.join("www-logs"));
    Ok(())
}

#[test]
fn a_relative_state_root_is_refused() -> Outcome<()> {
    let app = res!(scratch("relative"));
    let mut cfg = res!(AppConfig::new());
    cfg.app_root = app.to_string_lossy().to_string();
    cfg.state_root = fmt!("var/lib/steel");
    match cfg.check_and_fix() {
        Ok(())  => Err(err!("A relative state root was accepted."; Test, Unexpected)),
        Err(_)  => Ok(()),
    }
}
