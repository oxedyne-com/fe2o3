#![forbid(unsafe_code)]
pub mod app;
pub mod srv;

use crate::{
    app::tui,
};

use oxedize_fe2o3_core::{
    prelude::*,
};


fn main() -> Outcome<()> {
    
    let mut log_cfg = get_log_config!();
    log_cfg.file = None;
    set_log_config!(log_cfg);
    set_log_level!("debug");

    let outcome = tui::run();

    std::thread::sleep(std::time::Duration::from_secs(1));

    log_finish_wait!();

    outcome
}
