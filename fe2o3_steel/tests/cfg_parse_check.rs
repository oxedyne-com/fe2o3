//! Throwaway: parse a real deployed config with this build, the whole way down to its vhosts.
//!
//! Run with `STEEL_CONFIG_CHECK=/path/to/config.jdat cargo test --test cfg_parse_check -- --nocapture`.
//! Unset, it skips.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    cfg::Config,
    file::JdatMapFile,
};
use oxedyne_fe2o3_steel::{
    app::cfg::AppConfig,
    srv::cfg::ServerConfig,
};

use std::path::Path;

#[test]
fn parses_a_real_config() -> Outcome<()> {
    let path = match std::env::var("STEEL_CONFIG_CHECK") {
        Ok(p)   => p,
        Err(_)  => {
            println!("STEEL_CONFIG_CHECK unset; skipping.");
            return Ok(());
        }
    };
    let app_cfg = res!(AppConfig::load(Path::new(&path)));
    let server_cfg = res!(ServerConfig::from_datmap(app_cfg.server_cfg.clone()));
    let vhosts = res!(server_cfg.get_vhosts());
    println!("PARSED OK: {} ({} vhosts)", path, vhosts.len());
    for vh in &vhosts {
        println!(
            "  {:34} publish = {}",
            vh.hostnames.first().map(|s| s.as_str()).unwrap_or("?"),
            match &vh.publish {
                Some(p) => fmt!("path={} title={:?} dir={} css={}",
                    p.path, p.title, p.dir, p.css.len()),
                None    => fmt!("none"),
            },
        );
    }
    Ok(())
}
