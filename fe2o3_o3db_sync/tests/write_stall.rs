//! A writer stuck in its append for longer than the caller waits (WSTALL, 2026-10-16: a gateway
//! on a disk that stopped completing for 39 s).  The caller is told the write's outcome is not
//! known, and the record lands once the writer is let go: "reported failed, actually written",
//! pinned down so that a caller can be built to survive it.  `test::hooks` is process-wide, which
//! is why this is a test binary of its own with a single test.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    O3db,
    base::{
        cfg::OzoneConfig,
        constant,
    },
    comm::response::outcome_unknown,
    data::core::RestSchemesInput,
    test::{
        hooks,
        setup::{
            self,
            Uid,
            UID_LEN,
        },
    },
};

use std::{
    collections::BTreeMap,
    path::{
        Path,
        PathBuf,
    },
    thread,
    time::{
        Duration,
        Instant,
    },
};

type TestDb = O3db<
    { UID_LEN },
    Uid,
    (),
    HashScheme,
    HashScheme,
    ChecksumScheme,
>;

#[test]
fn main() -> Outcome<()> {
    log_set_level!("error");
    let outcome = a_stalled_append_is_not_known_and_then_lands();
    hooks::set_write_hold(0, Duration::ZERO);
    log_finish_wait!();
    outcome
}

fn a_stalled_append_is_not_known_and_then_lands() -> Outcome<()> {
    let hold = constant::USER_REQUEST_TIMEOUT + Duration::from_secs(2);
    let root = res!(fresh("./test_db_write_stall"));
    let mut cfg = res!(setup::default_cfg());
    cfg.zone_overrides = BTreeMap::new();
    let db = res!(open(&root, cfg));
    let k = dat!("write stall key");

    hooks::set_write_hold(0, hold);
    let begun = Instant::now();
    let outcome = db.insert(k.clone(), dat!(42u8), Uid::default(), None);
    let took = begun.elapsed();
    if hooks::write_holds_taken() != 1 {
        return Err(err!("The write hold was taken {} times, not once.",
            hooks::write_holds_taken(); Test, Mismatch));
    }
    match outcome {
        Ok(r) => return Err(err!(
            "A write held for {:?} was confirmed in {:?}: {:?}.", hold, took, r;
            Test, Unexpected)),
        Err(e) => {
            if !outcome_unknown(&e) {
                return Err(err!(
                    "A write whose writer said nothing in time is tagged {:?}, which \
                    outcome_unknown does not read as a write that may yet land: {}",
                    e.tags(), e;
                    Test, Mismatch));
            }
            if !fmt!("{}", e).contains("not") || !fmt!("{}", e).contains("known") {
                return Err(err!("The error does not say the outcome is not known: {}", e;
                    Test, Mismatch));
            }
            if took >= hold {
                return Err(err!("The caller waited {:?}, the whole of the {:?} hold.", took, hold;
                    Test, Timeout));
            }
        },
    }

    // Reported failed, and written all the same once the disk returns.
    thread::sleep(hold.saturating_sub(begun.elapsed()) + Duration::from_millis(500));
    let mut found = None;
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if let Some((v, _)) = res!(db.get(&k, None)) {
            found = Some(v);
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    match found {
        Some(v) => req!(v, dat!(42u8), "The held write landed with another value."),
        None => return Err(err!(
            "A write reported as of unknown outcome never landed after its hold.";
            Test, Missing)),
    }
    res!(db.close());
    Ok(())
}

/// An empty directory of this test's own.
fn fresh(dir: &str) -> Outcome<PathBuf> {
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    Ok(res!(Path::new(dir).canonicalize()))
}

fn open(root: &Path, cfg: OzoneConfig) -> Outcome<TestDb> {
    let schms = RestSchemesInput::new(
        None::<()>,
        None::<HashScheme>,
        None::<HashScheme>,
        Some(ChecksumScheme::new_crc32()),
    );
    let mut db = res!(TestDb::new(root.to_path_buf(), Some(cfg), schms, Uid::default()));
    res!(db.start("test"));
    Ok(db)
}
