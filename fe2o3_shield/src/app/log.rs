use oxedize_fe2o3_core::{
    prelude::*,
    log::console::LoggerConsole,
};


pub fn switch_to_logger_console<
    L: LoggerConsole<ErrTag>,
>()
    -> Outcome<()>
{
    log_out_finish_wait!();
    let mut log_cfg = log_get_config!();
    let mut logger_console = L::new();
    let logger_console_thread = logger_console.go();

    // Update both channels:
    {
        let mut unlocked_chan_out = lock_write!(LOG.chan_out);
        *unlocked_chan_out = logger_console_thread.clone();
    }
    log_cfg.console = Some(logger_console_thread.chan.clone());

    log_set_config!(log_cfg);
    Ok(())
}
