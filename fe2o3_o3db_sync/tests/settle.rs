//! The accounting barrier (A3 R2, 2026-10-08): once `settle` returns, every supersession caused by
//! a write acknowledged before the call is in the file bots' states.  A cache bot answers its
//! caller before it tells the file bot of the new record's file, and that file bot tells the file
//! bot of the old record's file, so a request that merely follows the answer can overtake the
//! supersession.  `set_forward_delay` holds each forward and `set_schedule_delay` holds each
//! receipt.  The hooks are process-wide, so the tests here take a lock and put each back.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_o3db_sync::{
    base::constant,
    test::{
        hooks,
        setup,
    },
};

use std::{
    path::Path,
    sync::Mutex,
    time::Duration,
};

static HOOKS: Mutex<()> = Mutex::new(());

const N: usize = 20;

fn old_sum(db: &TestDb) -> Outcome<u64> {
    let states = res!(db.api().collect_file_states(constant::USER_REQUEST_WAIT));
    let mut sum = 0;
    for (_wind, fmap) in states {
        for (_fnum, fstat) in fmap.map() {
            sum += fstat.get_old_sum();
        }
    }
    Ok(sum)
}

#[test]
fn settle_orders_supersessions() -> Outcome<()> {
    let _lock = match HOOKS.lock() { Ok(g) => g, Err(p) => p.into_inner() };
    log_set_level!("error");
    let dir = "./test_db_settle_order";
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());
    let probe = res!(probe("./test_db_settle_order_probe"));
    let mut cfg = res!(config(5 * probe.len + probe.len / 2));
    cfg.num_cbots_per_zone = 2;
    cfg.num_fbots_per_zone = 2;
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg), schemes(), None, false, true));
    let user = setup::Uid::default();
    for i in 0..N {
        res!(db.insert(key(i), value(i, 1), user, None));
    }
    res!(db.api().settle_for_test(Duration::from_secs(10)));
    assert_eq!(res!(old_sum(&db)), 0, "positive control: nothing is old before the overwrite");

    // Two more records put the overwrites a file out of step with the records they replace.  File
    // k belongs to file bot k % 2, and a supersession forwards only when its two files belong to
    // different bots, which five records to a file and twenty keys would never arrange.
    for i in N..(N + 2) {
        res!(db.insert(key(i), value(i, 1), user, None));
    }
    hooks::set_forward_delay(Duration::from_millis(700));
    for i in 0..N {
        res!(db.insert(key(i), value(i, 2), user, None));
    }
    res!(db.api().settle_for_test(Duration::from_secs(60)));
    let at_settle = res!(old_sum(&db));
    hooks::set_forward_delay(Duration::ZERO);
    std::thread::sleep(Duration::from_millis(1500));
    let quiet = res!(old_sum(&db));
    msg!("Old bytes registered at settle {}, once quiet {}, records {} of {} bytes.", at_settle, quiet, N, probe.len);
    assert_eq!(quiet, N as u64 * probe.len, "the quiet total is not the {} superseded records", N);
    assert_eq!(at_settle, quiet, "supersessions were still travelling when settle returned");
    res!(db.close());
    Ok(())
}

// The echo stage: a file bot that has forwarded a supersession has not finished with it, since the
// file bot it went to may be behind.  Every later request queues behind the supersession, so what
// tells a barrier that waits for the receiver from one that stops at the sender is when it returns,
// counted here in the supersessions the receivers had handled by then.  The forwards are slowed as
// well, so that some arrive after the receiver has reached its own request, which it answers.
#[test]
fn settle_waits_for_the_receiving_file_bot() -> Outcome<()> {
    let _lock = match HOOKS.lock() { Ok(g) => g, Err(p) => p.into_inner() };
    log_set_level!("error");
    let dir = "./test_db_settle_receiver";
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());
    let probe = res!(probe("./test_db_settle_receiver_probe"));
    let mut cfg = res!(config(5 * probe.len + probe.len / 2));
    cfg.num_cbots_per_zone = 2;
    cfg.num_fbots_per_zone = 2;
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg), schemes(), None, false, true));
    let user = setup::Uid::default();
    for i in 0..(N + 2) {
        res!(db.insert(key(i), value(i, 1), user, None));
    }
    res!(db.api().settle_for_test(Duration::from_secs(10)));
    assert_eq!(res!(old_sum(&db)), 0, "positive control: nothing is old before the overwrite");

    let base = hooks::schedules_held();
    hooks::set_forward_delay(Duration::from_millis(150));
    hooks::set_schedule_delay(Duration::from_millis(250));
    for i in 0..N {
        res!(db.insert(key(i), value(i, 2), user, None));
    }
    let settled = db.api().settle_for_test(Duration::from_secs(60));
    let at_settle = hooks::schedules_held() - base;
    // The hold stays on until the receivers have caught up, or the count would stop short.
    let mut last = hooks::schedules_held();
    loop {
        std::thread::sleep(Duration::from_millis(800));
        let now = hooks::schedules_held();
        if now == last { break; }
        last = now;
    }
    hooks::set_forward_delay(Duration::ZERO);
    hooks::set_schedule_delay(Duration::ZERO);
    let held = last - base;
    res!(settled);
    msg!("Supersessions held {}, handled when settle returned {}.", held, at_settle);
    assert!(held > 0, "positive control: some supersession was forwarded to another file bot");
    assert_eq!(at_settle, held, "a file bot was still holding a received supersession when settle returned");
    assert_eq!(res!(old_sum(&db)), N as u64 * probe.len, "the superseded records are not all registered");
    res!(db.close());
    Ok(())
}
