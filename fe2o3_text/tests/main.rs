mod base2x;
mod highlight;
mod pattern;
mod string;

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

    let filter = "match";//"all";
    res!(base2x::test_base2x(filter));
    res!(highlight::test_highlight(filter));
    res!(pattern::test_pattern(filter));
    res!(string::test_string(filter));

    Ok(())
}
