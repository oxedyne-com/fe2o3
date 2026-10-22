//! The orphan sweep reads no clock (Ozone R1-1, 2026-10-10).  After a restart the stamp floor can
//! put every new record past the wall clock, and a sweep that kept "recent" chunks by comparing
//! their stamps with the wall clock kept such an orphan until the clock caught up.  A store in
//! progress is spared by the set registry instead, which a sweep watches for as long as it runs.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_data::time::Timestamp;
use oxedyne_fe2o3_hash::hash::HashScheme;
use oxedyne_fe2o3_iop_db::api::{
    Database,
    RestSchemesOverride,
    ScanOpts,
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    base::constant,
    comm::{
        channels::SetRegistry,
        msg::OzoneMsg,
        response::Wait,
    },
    sweep,
    test::{
        hooks,
        setup::{
            self,
            Uid,
        },
    },
};

use std::{
    sync::{
        Mutex,
        MutexGuard,
    },
    thread,
    time::{
        Duration,
        Instant,
    },
};

const DAY: Duration = Duration::from_secs(86_400);

// The hooks are process-wide, so every test that starts a store takes this lock.
static SERIAL: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|p| p.into_inner())
}

fn scan_wait() -> Wait {
    Wait {
        max_wait:       Duration::from_secs(60),
        check_interval: constant::CHECK_INTERVAL,
    }
}

fn value_of(seed: u8, len: usize) -> Dat {
    Dat::BU32((0..len).map(|i| seed.wrapping_add((i % 251) as u8)).collect())
}

// An overwrite that leaves its predecessor's chunks behind, as a crash between a head and its
// retire does.
fn store_without_reclaim(db: &TestDb, k: Dat, v: Dat) -> Outcome<()> {
    let resp = db.api().responder();
    let msgs = res!(db.api().prepare_write_dat(k, v, Uid::default(),
        None::<&RestSchemesOverride<(), HashScheme>>, resp.clone(), None));
    res!(resp.send(OzoneMsg::Chunks(msgs.len())));
    res!(db.api().store_bytes(msgs));
    res!(resp.recv_store_ack());
    Ok(())
}

#[test]
fn an_orphan_stamped_past_the_wall_clock_is_reclaimed() -> Outcome<()> {
    let _serial = lock();
    log_set_level!("error");
    let dir = "./test_db_sweep_clockless";
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    let root = res!(std::path::Path::new(dir).canonicalize());
    let mut cfg = res!(config(16_000));
    cfg.rest_chunk_threshold    = 3_000;
    cfg.rest_chunk_bytes        = 1_000;
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg), schemes(), None, true, true));

    // The jump: one record stamped a day ahead, as one replayed after the clock stepped back is.
    // Every stamp after it is later still.
    let ahead = *res!(Timestamp::now()) + DAY;
    hooks::set_fixed_stamp(Some(ahead));
    let jumped = db.insert(dat!("jump"), dat!("ahead"), Uid::default(), None);
    hooks::set_fixed_stamp(None);
    res!(jumped);

    let k = dat!("chunked");
    res!(db.insert(k.clone(), value_of(1, 6_000), Uid::default(), None));
    res!(store_without_reclaim(&db, k.clone(), value_of(2, 3_200)));
    res!(db.api().settle_for_test(Duration::from_secs(10)));

    // Positive control: the orphan's chunks are stamped past the wall clock.
    let now = res!(Timestamp::now());
    let chunks = res!(db.api().scan_with_wait(&ScanOpts::all().chunk_data_only(true),
        None::<&RestSchemesOverride<(), HashScheme>>, scan_wait()));
    let ahead_of_clock = chunks.iter().filter(|(_, _, m)| m.time > now).count();

    let report = res!(sweep::sweep_orphans(db.api(), Uid::default(), None, scan_wait()));
    let got = res!(db.get(&k, None));
    res!(db.close());
    log_finish_wait!();
    let _ = std::fs::remove_dir_all(dir);

    msg!("{}", report.summary().replace('\n', " | "));
    assert!(chunks.len() > 3, "the values were not chunked: {} chunk records", chunks.len());
    assert_eq!(ahead_of_clock, chunks.len(), "not every chunk is stamped past the wall clock");
    assert!(report.orphans_found >= 6, "the sweep kept the orphan stamped past the wall clock: {:?}", report);
    assert_eq!(report.orphans_retired, report.orphans_found, "a tombstone did not land: {:?}", report);
    assert!(matches!(&got, Some((g, _)) if *g == value_of(2, 3_200)), "the live value does not read whole");
    Ok(())
}

// A chunked store that starts and finishes while a sweep is between its two scans is in neither
// the live set, read before its head landed, nor the held sets, let go before the chunks were
// classified.  Only the sweep's watch of the registry spares it.
#[test]
fn a_store_made_between_the_sweeps_scans_is_spared() -> Outcome<()> {
    let _serial = lock();
    log_set_level!("error");
    let dir = "./test_db_sweep_between_scans";
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    let root = res!(std::path::Path::new(dir).canonicalize());
    let mut cfg = res!(config(16_000));
    cfg.rest_chunk_threshold    = 3_000;
    cfg.rest_chunk_bytes        = 1_000;
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg), schemes(), None, true, true));
    res!(db.insert(dat!("before"), value_of(3, 6_000), Uid::default(), None));
    res!(db.api().settle_for_test(Duration::from_secs(10)));

    hooks::set_sweep_hold(true);
    let held_before = hooks::sweeps_held();
    let sweeper = {
        let db = db.clone();
        thread::spawn(move || sweep::sweep_orphans(db.api(), Uid::default(), None, scan_wait())
            .map_err(|e| fmt!("{}", e)))
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    while hooks::sweeps_held() == held_before && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    let reached = hooks::sweeps_held() > held_before;
    let k = dat!("between");
    let put = db.insert(k.clone(), value_of(4, 6_000), Uid::default(), None);
    let settled = db.api().settle_for_test(Duration::from_secs(10));
    hooks::set_sweep_hold(false);
    let report = match sweeper.join() {
        Ok(r) => r,
        Err(_) => Err(fmt!("the sweep panicked")),
    };
    let got = db.get(&k, None);
    let pending_empty = lock_mutex!(db.api().chans().pending_sets()).is_empty();
    res!(db.close());
    log_finish_wait!();
    let _ = std::fs::remove_dir_all(dir);

    assert!(reached, "the sweep never reached its hold between the scans");
    res!(put);
    res!(settled);
    assert!(pending_empty, "the store's set was still held, so the watch was not what spared it");
    let report = match report {
        Ok(r) => r,
        Err(e) => return Err(err!("The sweep failed: {}", e; Test)),
    };
    msg!("{}", report.summary().replace('\n', " | "));
    assert_eq!(report.orphans_found, 0, "the sweep took the chunks of a store made between its scans: {:?}", report);
    assert!(report.skipped_pending >= 6, "the store's chunks were not spared as watched: {:?}", report);
    assert!(matches!(&got, Ok(Some((g, _))) if *g == value_of(4, 6_000)),
        "the value stored between the scans does not read whole");
    Ok(())
}

// A set let go while a sweep watches is still reported to it, until the last watch ends.
#[test]
fn a_set_released_during_a_watch_stays_watched() {
    let mut reg = SetRegistry::default();
    reg.hold(1);
    reg.release(&1);
    assert!(!reg.watched().contains(&1), "a set released before any watch is still watched");
    reg.watch();
    reg.hold(2);
    reg.hold(3);
    reg.release(&2);
    let seen = reg.watched();
    assert!(seen.contains(&2), "a set released during the watch was not reported: {:?}", seen);
    assert!(seen.contains(&3), "a held set was not reported: {:?}", seen);
    assert!(!reg.is_held(&2), "a released set is still held");
    reg.watch();
    reg.unwatch();
    assert!(reg.watched().contains(&2), "one watch ending forgot the other's released set");
    reg.unwatch();
    assert_eq!(reg.watched().into_iter().collect::<Vec<_>>(), vec![3],
        "the last watch ending did not forget the released sets");
}
