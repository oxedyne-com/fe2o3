mod exif;
mod glob;
mod office;
mod tree;
mod pptx;
mod xlsx;
mod zip;

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

    let filter = "all";

    res!(tree::test_tree(filter));
    res!(exif::test_exif(filter));
    res!(glob::test_glob(filter));
    res!(zip::test_zip(filter));
    res!(office::test_office(filter));
    res!(xlsx::test_xlsx(filter));
    res!(pptx::test_pptx(filter));

    Ok(())
}
