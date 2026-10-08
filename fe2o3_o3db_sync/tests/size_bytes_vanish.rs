//! A file taken from a zone directory between its listing and its opening must not fail the size
//! (A3 R1, 2026-10-08).  A collection renames its temporary files and deletes a wholly old file's
//! pair, so the listing can name a file that is gone by the time the zone bot opens it.  The
//! listing is held by `test::hooks::set_list_delay`, which is process-wide, which is why this is a
//! test binary of its own with a single test.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_o3db_sync::test::{
    hooks,
    setup,
};

use std::{
    path::Path,
    thread,
    time::Duration,
};

#[test]
fn main() -> Outcome<()> {
    log_set_level!("error");
    let dir = "./test_db_size_bytes_vanish_walk";
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());

    let probe = res!(probe("./test_db_size_bytes_vanish_walk_probe"));
    let cfg = res!(config(5 * probe.len + probe.len / 2));
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, false, true));
    res!(fill(&db));
    let before = zone_files_len(&root, &cfg);

    // A file of the kind a collection leaves while it works, and the positive control: it is in
    // the directory, so it is in the length.
    let decoy = zone_dir(&root, &cfg).join("decoy.tmp");
    res!(std::fs::write(&decoy, vec![7u8; 100]));
    assert_eq!(res!(db.size_bytes()), before + 100, "the control file is not counted");

    // The zone bot reads the directory, names the decoy, and waits.  The decoy is taken away
    // while it waits.
    hooks::set_list_delay(Duration::from_millis(1500));
    let asker = {
        let db = db.clone();
        thread::spawn(move || db.size_bytes())
    };
    thread::sleep(Duration::from_millis(400));
    res!(std::fs::remove_file(&decoy));
    let got = match asker.join() {
        Ok(r) => res!(r),
        Err(_) => return Err(err!("The size_bytes thread panicked."; Test, Thread)),
    };
    hooks::set_list_delay(Duration::ZERO);
    msg!("size_bytes {} with the decoy taken after it was listed, files {}.", got, before);
    assert_eq!(got, before, "a file that vanished mid-walk changed the length");

    res!(db.close());
    log_finish_wait!();
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}
