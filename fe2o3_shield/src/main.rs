#![forbid(unsafe_code)]
pub mod app;
pub mod prelude;
pub mod srv;

use crate::{
    app::tui,
};

use oxedize_fe2o3_core::{
    prelude::*,
};


fn main() -> Outcome<()> {
    
    let mut log_cfg = log_get_config!();
    log_cfg.file = None;
    log_set_config!(log_cfg);
    log_set_level!("debug");

    let outcome = tui::run();

    std::thread::sleep(std::time::Duration::from_secs(1));

    log_finish_wait!();

    outcome
}
