//! No record is ever beaten or displaced by an unconfirmed winner (durable mark, commit A).
//! Before it, a record U whose sync failed could stand in the cache over an older chunked put P,
//! and P had to be held from retiring, collection and the sweep until a restart (R2-1).  Now the
//! failed sync stops the store: U is refused, a store in flight is refused, a read of what no
//! mark covers is refused, compaction and the sweep are refused, and the restart cuts U from the
//! disk.  Two orders, as before: P's head waits while U fails (P would be beaten), and U fails
//! after P (P would be displaced).  The hooks are process-wide, so the two run one at a time.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::{
    Database,
    Meta,
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    base::{
        cfg::OzoneConfig,
        constant,
    },
    comm::response::Wait,
    file::core::FileType,
    sweep,
    test::{
        hooks,
        setup::{
            self,
            Uid,
            UID_LEN,
        },
    },
    verify,
};

use std::{
    collections::BTreeMap,
    path::{
        Path,
        PathBuf,
    },
    sync::Mutex,
    thread,
    time::Duration,
};

fn cfg() -> Outcome<OzoneConfig> {
    let mut c = res!(setup::default_cfg());
    c.num_zones             = 1;
    c.num_cbots_per_zone    = 4;
    c.num_fbots_per_zone    = 1;
    c.num_wbots_per_zone    = 1;
    c.zone_overrides        = BTreeMap::new();
    c.rest_chunk_threshold  = 1_000;
    c.rest_chunk_bytes      = 400;
    c.sync_on_write         = true;
    Ok(c)
}

fn val(len: usize, seed: u8) -> Dat {
    Dat::BU32((0..len).map(|j| seed ^ (j as u8)).collect())
}

fn start(root: &Path, wipe: bool) -> Outcome<TestDb> {
    log_set_level!("error");
    setup::start_db(root.to_path_buf(), Some(res!(cfg())), schemes(), None, false, wipe)
}

fn scan_wait() -> Wait {
    Wait { max_wait: Duration::from_secs(60), check_interval: constant::CHECK_INTERVAL }
}

// "L/S" for a whole value of length L and seed S as `val` writes it, else what was read.
fn describe(r: &Outcome<Option<(Dat, Meta<{ UID_LEN }, Uid>)>>) -> String {
    match r {
        Ok(None) => fmt!("none"),
        Ok(Some((Dat::BU32(b), _))) | Ok(Some((Dat::BU8(b), _))) | Ok(Some((Dat::BU16(b), _))) |
        Ok(Some((Dat::BU64(b), _))) if !b.is_empty()
            && b.iter().enumerate().all(|(j, x)| *x == b[0] ^ (j as u8)) => fmt!("{}/{}", b.len(), b[0]),
        Ok(Some((d, _))) => fmt!("TORN:{:?}", d).chars().take(100).collect(),
        Err(e) => fmt!("ERR:{}", fmt!("{}", e).chars().take(200).collect::<String>()),
    }
}

static SERIAL: Mutex<()> = Mutex::new(());

// The highest numbered data file, the live one just after `new_live_files`.
fn live_fnum(root: &Path, c: &OzoneConfig) -> u32 {
    let mut n = 1;
    while file(root, c, FileType::Data, n + 1).exists() {
        n += 1;
    }
    n
}

// Writes U, unchunked, into a live file of its own whose barrier fails.  The roll after it is
// refused once the store has stopped, and that is not what is tested here.
fn write_unconfirmed(db: &TestDb, root: &Path, k: &Dat) -> Outcome<Outcome<()>> {
    let c = res!(cfg());
    res!(db.api().new_live_files());
    hooks::set_barrier_failure_for(Some(live_fnum(root, &c)));
    let u = db.insert(k.clone(), val(50, 3), Uid::default(), None).map(|_| ());
    let _ = db.api().new_live_files();
    hooks::set_barrier_failure_for(None);
    Ok(u)
}

// What the stopped store still does: a read of the key, compaction and the sweep.
fn stopped(db: &TestDb, k: &Dat) -> Outcome<(bool, String, Outcome<()>, Outcome<sweep::SweepReport>)> {
    res!(db.api().settle_for_test(Duration::from_secs(10)));
    let faulted = db.api().fault().is_some();
    let live = describe(&db.get(k, None));
    let compact = db.compact_now(Duration::from_secs(60)).map(|_| ());
    let swept = sweep::sweep_orphans(db.api(), Uid::default(), None, scan_wait());
    Ok((faulted, live, compact, swept))
}

// The restart cuts U.  Returns the key's value, whether the fault is gone, and whether the store
// verifies clean after a sweep.
fn restart(root: &Path, k: &Dat) -> Outcome<(String, bool, bool)> {
    let db = res!(start(root, false));
    let after = describe(&db.get(k, None));
    let cleared = db.api().fault().is_none();
    res!(sweep::sweep_orphans(db.api(), Uid::default(), None, scan_wait()));
    let v = res!(verify::verify_live_set(db.api(), None, scan_wait()));
    res!(db.close());
    Ok((after, cleared, v.clean()))
}

fn fresh(dir: &str) -> Outcome<PathBuf> {
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    Ok(res!(Path::new(dir).canonicalize()))
}

fn assert_stopped(
    faulted:    bool,
    live:       &str,
    compact:    &Outcome<()>,
    swept:      &Outcome<sweep::SweepReport>,
)
    -> Outcome<()>
{
    assert!(faulted, "a failed sync did not stop the store");
    assert!(live.starts_with("ERR:"), "a read of U, which no mark covers, was served: {}", live);
    assert!(compact.is_err(), "compaction ran on a stopped store");
    match swept {
        Ok(r) => return Err(err!("the sweep ran on a stopped store: {:?}", r; Test)),
        Err(e) => assert!(fmt!("{}", e).contains("Restart the store"),
            "the sweep's refusal does not say what to do: {}", e),
    }
    Ok(())
}

/// P's head waits while U's sync fails.  The store stops, so P's head is refused rather than
/// beaten, and after the restart neither U nor P is the key's value.
#[test]
fn a_store_in_flight_when_a_sync_fails_is_refused() -> Outcome<()> {
    let _serial = SERIAL.lock();
    let dir = "./test_db_unconfirmed_winner_beaten";
    let root = res!(fresh(dir));
    let db = res!(start(&root, true));
    let k = dat!("unconfirmed winner beaten");
    // P: chunked, held with its chunks in until U is in.  Stored through the API, its store
    // runs in this thread rather than a server bot.
    hooks::set_head_hold(true);
    let p = { let (db, k) = (db.clone(), k.clone());
        thread::spawn(move || -> Outcome<()> {
            let resp = db.api().responder();
            res!(db.api().store_dat_using_responder(k, val(1_500, 2), Uid::default(), None, resp.clone()));
            resp.recv_store_ack().map(|_| ())
        }) };
    if !wait_until(Duration::from_secs(30), || hooks::heads_held() >= 1) {
        hooks::set_head_hold(false);
        return Err(err!("P's store never reached its head"; Test));
    }
    let u = res!(write_unconfirmed(&db, &root, &k));
    hooks::set_head_hold(false);
    let pr = match p.join() { Ok(r) => r, Err(_) => Err(err!("P panicked"; Test)) };
    let (faulted, live, compact, swept) = res!(stopped(&db, &k));
    let _ = db.close();
    println!("beaten: p={:?} u={:?} faulted={} live={} compact={:?} swept={:?}",
        pr.as_ref().map_err(|e| fmt!("{}", e)), u.as_ref().map_err(|e| fmt!("{}", e)),
        faulted, live, compact.as_ref().map_err(|e| fmt!("{}", e)),
        swept.as_ref().map_err(|e| fmt!("{}", e)));
    assert!(u.is_err(), "U's store succeeded although its sync was failed");
    assert!(pr.is_err(), "P's head was written on a stopped store");
    res!(assert_stopped(faulted, &live, &compact, &swept));
    let (after, cleared, clean) = res!(restart(&root, &k));
    let _ = std::fs::remove_dir_all(dir);
    assert_eq!(after, "none", "after the restart's cut the key holds neither U nor P");
    assert!(cleared, "the restart left the fault standing");
    assert!(clean, "the store does not verify clean after the restart and a sweep");
    Ok(())
}

/// U's sync fails after P has landed.  U never displaces P: the store stops, U is not served, and
/// the restart's cut leaves P as the key's value.
#[test]
fn a_failed_sync_never_displaces_a_durable_head() -> Outcome<()> {
    let _serial = SERIAL.lock();
    let dir = "./test_db_unconfirmed_winner_displaced";
    let root = res!(fresh(dir));
    let db = res!(start(&root, true));
    let k = dat!("unconfirmed winner displaced");
    res!(db.insert(k.clone(), val(1_500, 2), Uid::default(), None));
    res!(db.api().sync_now(scan_wait()));
    let u = res!(write_unconfirmed(&db, &root, &k));
    let (faulted, live, compact, swept) = res!(stopped(&db, &k));
    let _ = db.close();
    println!("displaced: u={:?} faulted={} live={} compact={:?} swept={:?}",
        u.as_ref().map_err(|e| e.tags().to_vec()), faulted, live,
        compact.as_ref().map_err(|e| fmt!("{}", e)), swept.as_ref().map_err(|e| fmt!("{}", e)));
    assert!(u.is_err(), "U's store succeeded although its sync was failed");
    res!(assert_stopped(faulted, &live, &compact, &swept));
    let (after, cleared, clean) = res!(restart(&root, &k));
    let _ = std::fs::remove_dir_all(dir);
    assert_eq!(after, "1500/2", "after the restart's cut P is the key's value");
    assert!(cleared, "the restart left the fault standing");
    assert!(clean, "the store does not verify clean after the restart and a sweep");
    Ok(())
}
