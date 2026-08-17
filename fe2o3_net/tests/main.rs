mod dns;
mod email;
mod http;
mod smtp;
mod smtp_submit;

use oxedyne_fe2o3_core::prelude::*;


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

    // Every case here is tagged "all", so this runs them. It was "dns" until
    // 2026-08-17, which ran the one DNS case and silently skipped the rest --
    // `test_it` matches a tag by prefix, and no other tag begins with "dns".
    // The suite still printed "running 1 test ... ok" in 0.00s, so a filter that
    // switched off four SMTP submission cases read exactly like a passing one.
    let filter = "all";

    res!(dns::test_dns(filter));
    res!(email::test_email(filter));
    res!(http::test_http(filter));
    res!(smtp::test_smtp(filter));
    res!(smtp_submit::test_smtp_submit(filter));

    Ok(())
}
