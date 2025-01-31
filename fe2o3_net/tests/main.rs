mod dns;
mod email;
mod http;
mod smtp;

use oxedize_fe2o3_core::prelude::*;


#[test]
fn main() -> Outcome<()> {
    
    // Separate the tests out to a run_tests function so that we can funnel any outcome, be it an
    // error or ok, back into this function before closing out with a single call to log_finish_wait! to
    // allow logger thread completion.  Otherwise, we may not see all the logger output before the
    // main thread finishes.

    log_set_level!("debug");

    let outcome = run_tests();

    log_finish_wait!();

    outcome
}

fn run_tests() -> Outcome<()> {

    let filter = "dns";

    res!(dns::test_dns(filter));
    res!(email::test_email(filter));
    res!(http::test_http(filter));
    res!(smtp::test_smtp(filter));

    Ok(())
}
