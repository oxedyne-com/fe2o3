//! A record beaten or displaced only by a record whose sync failed is neither old nor an orphan
//! (R2-1, QA agent 2, 2026-10-09).  When U's sync failed, U may never reach the disk -- a
//! write-back error after which the kernel drops the dirty page, or a power cut -- and the older
//! chunked put P is then the key's value again.  So P must not retire its own set, its head must
//! not be collected, and the sweep must not take its chunks, before a restart settles it.  Two
//! orders: P's head lands after U (P is beaten), and U lands after P (P is displaced).  The loss
//! of U is simulated by cutting U's record, alone in its own data file, from that file.  The
//! hooks are process-wide, so the two run one at a time.

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
    data::cache::{
        CacheEntry,
        is_chunk_key,
    },
    file::{
        core::FileType,
        floc::FileLocation,
    },
    sweep,
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
    fs::OpenOptions,
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

// Writes U, unchunked, into a live file of its own whose barrier fails, and rolls past it.
fn write_unconfirmed(db: &TestDb, root: &Path, k: &Dat) -> Outcome<Outcome<()>> {
    let c = res!(cfg());
    res!(db.api().new_live_files());
    hooks::set_barrier_failure_for(Some(live_fnum(root, &c)));
    let u = db.insert(k.clone(), val(50, 3), Uid::default(), None).map(|_| ());
    res!(db.api().new_live_files());
    hooks::set_barrier_failure_for(None);
    Ok(u)
}

// What the process holds once both stores are done: the key's value, chunk records retired, and
// where the cache holds the key's record.
fn observe(db: &TestDb, k: &Dat) -> Outcome<(String, usize, FileLocation)> {
    res!(db.api().settle_for_test(Duration::from_secs(10)));
    let live = describe(&db.get(k, None));
    let (kbuf, _, _) = res!(db.api().ozone_key_dat(k, None));
    let mut retired = 0usize;
    let mut u_loc = None;
    for (_, cache) in res!(db.api().collect_caches(scan_wait())) {
        let tombs = cache.tomb_tracker().tombs();
        for (kb, e) in cache.map() {
            if let CacheEntry::LocatedValue(mloc, _) = e {
                if *kb == kbuf {
                    u_loc = Some(mloc.file_location().clone());
                } else if is_chunk_key(kb) && tombs.contains_key(kb) {
                    retired += 1;
                }
            }
        }
    }
    let u_loc = match u_loc { Some(l) => l, None => return Err(err!("U is not in the cache"; Test)) };
    Ok((live, retired, u_loc))
}

// Collection and the sweep, before the restart: neither may take P.
fn collect_and_sweep(db: &TestDb) -> Outcome<Outcome<sweep::SweepReport>> {
    res!(db.compact_now(Duration::from_secs(60)));
    Ok(sweep::sweep_orphans(db.api(), Uid::default(), None, scan_wait()))
}

// U never reaches the disk; after a restart P is the key's value.
fn lose_u_and_restart(root: &Path, u_loc: &FileLocation, k: &Dat) -> Outcome<String> {
    let c = res!(cfg());
    let dat = file(root, &c, FileType::Data, u_loc.fnum);
    let len = size(&dat);
    if len < u_loc.start + u_loc.klen + u_loc.vlen {
        return Err(err!("U's file {:?} is {} bytes, short of its record at {:?}.", dat, len, u_loc; Test));
    }
    let f = res!(OpenOptions::new().write(true).open(&dat));
    res!(f.set_len(u_loc.start));
    drop(f);
    let _ = std::fs::remove_file(file(root, &c, FileType::Index, u_loc.fnum));
    let db = res!(start(root, false));
    let after = describe(&db.get(k, None));
    res!(db.close());
    Ok(after)
}

fn fresh(dir: &str) -> Outcome<PathBuf> {
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    Ok(res!(Path::new(dir).canonicalize()))
}

/// P's head lands after U: P is beaten by an unconfirmed winner.  It keeps its set, held from the
/// sweep, and its head is not old.
#[test]
fn a_beaten_head_keeps_its_set_and_its_place() -> Outcome<()> {
    let _serial = SERIAL.lock();
    let dir = "./test_db_unconfirmed_winner_beaten";
    let root = res!(fresh(dir));
    let db = res!(start(&root, true));
    let k = dat!("unconfirmed winner beaten");
    // P: chunked, stamped now, held with its chunks durable and cached until U is in.  Stored
    // through the API, its store runs in this thread rather than a server bot, which U needs.
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
    let (live, retired, u_loc) = res!(observe(&db, &k));
    let swept = res!(collect_and_sweep(&db));
    res!(db.close());
    println!("beaten: p={:?} u={:?} live={} retired={} u_loc={:?} swept={:?}",
        pr.as_ref().map_err(|e| fmt!("{}", e)), u.as_ref().map_err(|e| fmt!("{}", e)),
        live, retired, u_loc, swept.as_ref().map_err(|e| fmt!("{}", e)));
    if let Err(e) = &pr {
        return Err(err!("P's store failed: {}", e; Test));
    }
    if u.is_ok() {
        return Err(err!("U's store succeeded although its sync was failed"; Test));
    }
    assert_eq!(live, "50/3", "the newer record U is the key's value while the process runs");
    assert_eq!(retired, 0, "the older store retired its own set against an unconfirmed winner");
    match &swept {
        Ok(r) => assert_eq!(r.orphans_retired, 0, "the sweep took P's chunks"),
        Err(e) => return Err(err!("the sweep refused although no head was displaced: {}", e; Test)),
    }
    let after = res!(lose_u_and_restart(&root, &u_loc, &k));
    let _ = std::fs::remove_dir_all(dir);
    assert_eq!(after, "1500/2", "with U lost, P's value is the key's value after the restart");
    Ok(())
}

/// U lands after P: P's head is displaced by an unconfirmed record.  It is not old, and the sweep
/// refuses until a restart, since only the store's schemes can name P's set.
#[test]
fn a_displaced_head_keeps_its_place_and_stops_the_sweep() -> Outcome<()> {
    let _serial = SERIAL.lock();
    let dir = "./test_db_unconfirmed_winner_displaced";
    let root = res!(fresh(dir));
    let db = res!(start(&root, true));
    let k = dat!("unconfirmed winner displaced");
    res!(db.insert(k.clone(), val(1_500, 2), Uid::default(), None));
    let u = res!(write_unconfirmed(&db, &root, &k));
    let (live, retired, u_loc) = res!(observe(&db, &k));
    let swept = res!(collect_and_sweep(&db));
    res!(db.close());
    println!("displaced: u={:?} live={} retired={} u_loc={:?} swept={:?}",
        u.as_ref().map_err(|e| e.tags().to_vec()), live, retired, u_loc,
        swept.as_ref().map_err(|e| fmt!("{}", e)));
    if u.is_ok() {
        return Err(err!("U's store succeeded although its sync was failed"; Test));
    }
    assert_eq!(live, "50/3", "the newer record U is the key's value while the process runs");
    assert_eq!(retired, 0, "P's set was retired against an unconfirmed record");
    match &swept {
        Ok(r) => return Err(err!("the sweep ran over a displaced head: {:?}", r; Test)),
        Err(e) => assert!(fmt!("{}", e).contains("Restart the store"),
            "the sweep's refusal does not say what to do: {}", e),
    }
    let after = res!(lose_u_and_restart(&root, &u_loc, &k));
    let _ = std::fs::remove_dir_all(dir);
    assert_eq!(after, "1500/2", "with U lost, P's value is the key's value after the restart");
    Ok(())
}
