mod core;
mod msg;

use oxedize_fe2o3_core::prelude::*;

#[test]
fn main() -> Outcome<()> {
    
    // Separate the tests out to a run_tests function so that we can funnel any outcome, be it an
    // error or ok, back into this function before closing out with a single call to log_finish_wait! to
    // allow logger thread completion.  Otherwise, we may not see all the logger output before the
    // main thread finishes.

    set_log_level!("debug");

    let outcome = run_tests();

    log_finish_wait!();

    outcome
}

fn run_tests() -> Outcome<()> {

    let filter = "all";

    res!(core::test_core(filter));
    res!(msg::test_msg(filter));

    Ok(())
}
