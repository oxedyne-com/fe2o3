//! The sequence lane on a real Ozone store: rising sequence numbers, pages bounded by bytes that
//! answer `more`, holes left by `remove` and `trim_through`, lazy expiry, replayed appends, reader
//! cursors and `stable`, pages bounded by steps, lanes under their own locks side by side, and
//! the lane read back after a restart.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    data::core::RestSchemesInput,
    lane::{
        Lane,
        LaneLocks,
        ReaderState,
    },
    test::setup,
};

use std::{
    path::PathBuf,
    sync::{
        Arc,
        RwLock,
    },
    thread,
};


type TestDb = oxedyne_fe2o3_o3db_sync::O3db<
    { setup::UID_LEN },
    setup::Uid,
    EncryptionScheme,
    HashScheme,
    HashScheme,
    ChecksumScheme,
>;

type TestLane<'a> = Lane<'a, { setup::UID_LEN }, setup::Uid, EncryptionScheme, HashScheme, TestDb>;

const NOW: u64 = 1_760_659_200_000;

fn open(root: &PathBuf, wipe: bool) -> Outcome<TestDb> {
    let schms = RestSchemesInput::new(
        None::<EncryptionScheme>,
        None::<HashScheme>,
        None::<HashScheme>,
        Some(ChecksumScheme::new_crc32()),
    );
    let mut cfg = res!(setup::default_cfg());
    cfg.zone_overrides = DaticleMap::new();
    setup::start_db(root.clone(), Some(cfg), schms, None, false, wipe)
}

fn val(i: u64, bytes: usize) -> Dat {
    let mut v = vec![0u8; bytes];
    v[..8].copy_from_slice(&i.to_be_bytes());
    Dat::bytdat(v)
}

fn seqs(p: &oxedyne_fe2o3_o3db_sync::lane::Page) -> Vec<u64> {
    p.entries.iter().map(|e| e.seq).collect()
}

fn check(ok: bool, what: String) -> Outcome<()> {
    if ok { Ok(()) } else { Err(err!("{}", what; Test, Mismatch)) }
}

#[test]
fn lane() -> Outcome<()> {
    log_set_level!("error");
    let outcome = run();
    log_finish_wait!();
    outcome
}

fn run() -> Outcome<()> {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("test_db_lane");
    let _ = std::fs::remove_dir_all(&root);
    res!(std::fs::create_dir_all(&root));
    let root = res!(root.canonicalize());
    let user = setup::Uid::default();
    {
        let mut db = res!(open(&root, true));
        res!(append_rises(&db, user));
        res!(pages_honour_max_bytes(&db, user));
        res!(holes_are_skipped(&db, user));
        res!(expired_reads_absent_and_is_deleted(&db, user));
        res!(replayed_append_does_not_double(&db, user));
        res!(replay_after_a_torn_append_is_not_held(&db, user));
        res!(cursors_and_stable(&db, user));
        res!(trim_never_passes_the_ask(&db, user));
        res!(steps_bound_a_page(&db, user));
        res!(measured_pages(&db, user));
        res!(a_trim_keeps_live_ephemerals(&db, user));
        res!(the_sweep_is_bounded(&db, user));
        res!(db.shutdown());
    }
    {
        let mut db = res!(open(&root, false));
        res!(survives_a_restart(&db, user));
        res!(db.shutdown());
    }
    res!(replay_under_the_lane_lock(&root, user));
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

fn append_rises(db: &TestDb, user: setup::Uid) -> Outcome<()> {
    let locks = LaneLocks::default();
    let g = locks.lock("t/rise");
    let lane: TestLane = Lane::new(db, user, &g);
    res!(check(res!(lane.head()) == 0, fmt!("a new lane has head {}", res!(lane.head()))));
    for i in 1..=5 {
        let a = res!(lane.append(val(i, 16), None, None));
        res!(check(a.seq == i && !a.replay, fmt!("append {} answered {:?}", i, a)));
    }
    let p = res!(lane.read_after(0, 1 << 20, usize::MAX, NOW));
    res!(check(seqs(&p) == vec![1, 2, 3, 4, 5] && p.through == 5 && !p.more,
        fmt!("read all: {:?} through {} more {}", seqs(&p), p.through, p.more)));
    res!(check(p.entries[2].val == val(3, 16), fmt!("entry 3 reads {:?}", p.entries[2].val)));
    let p = res!(lane.read_after(3, 1 << 20, usize::MAX, NOW));
    res!(check(seqs(&p) == vec![4, 5], fmt!("read after 3: {:?}", seqs(&p))));
    let p = res!(lane.read_after(5, 1 << 20, usize::MAX, NOW));
    res!(check(p.entries.is_empty() && p.through == 5 && !p.more, fmt!("read after head: {:?}", p)));
    // A cursor beyond the head answers the head.
    let p = res!(lane.read_after(99, 1 << 20, usize::MAX, NOW));
    res!(check(p.entries.is_empty() && p.through == 5, fmt!("read after 99: through {}", p.through)));
    Ok(())
}

fn pages_honour_max_bytes(db: &TestDb, user: setup::Uid) -> Outcome<()> {
    let locks = LaneLocks::default();
    let g = locks.lock("t/page");
    let lane: TestLane = Lane::new(db, user, &g);
    for i in 1..=10 {
        res!(lane.append(val(i, 1_000), None, None));
    }
    let size = match val(0, 1_000).byte_len() {
        Some(n) => n,
        None    => return Err(err!("A byte value cannot be measured."; Test, Unexpected)),
    };
    // Room for three and a half: three, and more.
    let mut after = 0;
    let mut got = Vec::new();
    let mut pages = 0;
    loop {
        let p = res!(lane.read_after(after, 3 * size + size / 2, usize::MAX, NOW));
        pages += 1;
        let used: usize = p.entries.len() * size;
        res!(check(used <= 3 * size + size / 2, fmt!("a page carried {} bytes", used)));
        got.extend(seqs(&p));
        after = p.through;
        if !p.more {
            break;
        }
        res!(check(p.entries.len() == 3, fmt!("a full page held {} entries", p.entries.len())));
    }
    res!(check(got == (1..=10).collect::<Vec<u64>>() && pages == 4,
        fmt!("paged {:?} in {} pages", got, pages)));
    // A first entry larger than the page is still answered.
    let p = res!(lane.read_after(0, 10, usize::MAX, NOW));
    res!(check(seqs(&p) == vec![1] && p.more, fmt!("a tiny page: {:?} more {}", seqs(&p), p.more)));
    Ok(())
}

fn holes_are_skipped(db: &TestDb, user: setup::Uid) -> Outcome<()> {
    let locks = LaneLocks::default();
    let g = locks.lock("t/hole");
    let lane: TestLane = Lane::new(db, user, &g);
    for i in 1..=8 {
        res!(lane.append(val(i, 16), None, None));
    }
    res!(check(res!(lane.remove(3)), fmt!("remove 3 found nothing")));
    res!(check(!res!(lane.remove(3)), fmt!("a second remove of 3 found it")));
    res!(check(res!(lane.remove(8)), fmt!("remove 8 found nothing")));
    res!(check(res!(lane.trim_through(2)) == 2, fmt!("trim through 2")));
    let p = res!(lane.read_after(0, 1 << 20, usize::MAX, NOW));
    res!(check(seqs(&p) == vec![4, 5, 6, 7] && p.through == 8 && !p.more,
        fmt!("holes: {:?} through {}", seqs(&p), p.through)));
    // The head is not lowered by removing its entry: the next append is 9.
    let a = res!(lane.append(val(9, 16), None, None));
    res!(check(a.seq == 9, fmt!("after removing the top, append got {}", a.seq)));
    res!(check(res!(lane.depth()) == 7 && res!(lane.floor()) == 2,
        fmt!("depth {} floor {}", res!(lane.depth()), res!(lane.floor()))));
    Ok(())
}

fn expired_reads_absent_and_is_deleted(db: &TestDb, user: setup::Uid) -> Outcome<()> {
    let locks = LaneLocks::default();
    let g = locks.lock("t/exp");
    let lane: TestLane = Lane::new(db, user, &g);
    res!(lane.append(val(1, 16), None, Some(NOW + 1_000)));
    res!(lane.append(val(2, 16), Some(b"two"), Some(NOW + 5_000)));
    res!(lane.append(val(3, 16), None, None));
    let p = res!(lane.read_after(0, 1 << 20, usize::MAX, NOW));
    res!(check(seqs(&p) == vec![1, 2, 3] && p.entries[0].exp_ms == Some(NOW + 1_000),
        fmt!("before expiry: {:?}", p.entries)));
    let p = res!(lane.read_after(0, 1 << 20, usize::MAX, NOW + 1_000));
    res!(check(seqs(&p) == vec![2, 3] && p.through == 3, fmt!("at entry 1's expiry: {:?}", seqs(&p))));
    // Deleted, not merely hidden: a read at an earlier time does not see it again.
    let p = res!(lane.read_after(0, 1 << 20, usize::MAX, NOW));
    res!(check(seqs(&p) == vec![2, 3], fmt!("entry 1 came back: {:?}", seqs(&p))));
    let p = res!(lane.read_after(0, 1 << 20, usize::MAX, NOW + 9_000));
    res!(check(seqs(&p) == vec![3], fmt!("after both expiries: {:?}", seqs(&p))));
    // The expired entry's id went with it, so its replay appends anew.
    let a = res!(lane.append(val(2, 16), Some(b"two"), None));
    res!(check(a.seq == 4 && !a.replay, fmt!("a replay after expiry answered {:?}", a)));
    let held = res!(db.get(&dat!("t/exp/e/1"), None)).is_some()
        || res!(db.get(&dat!("t/exp/e/2"), None)).is_some();
    res!(check(!held, fmt!("an expired entry is still stored")));
    Ok(())
}

fn replayed_append_does_not_double(db: &TestDb, user: setup::Uid) -> Outcome<()> {
    let locks = LaneLocks::default();
    let g = locks.lock("t/replay");
    let lane: TestLane = Lane::new(db, user, &g);
    let a = res!(lane.append(val(1, 16), Some(&[0xab, 0x01]), None));
    let b = res!(lane.append(val(2, 16), Some(&[0xab, 0x02]), None));
    let r = res!(lane.append(val(1, 16), Some(&[0xab, 0x01]), None));
    res!(check(a.seq == 1 && b.seq == 2 && r.seq == 1 && r.replay && res!(lane.head()) == 2,
        fmt!("appends {:?} {:?}, replay {:?}, head {}", a, b, r, res!(lane.head()))));
    // Removed, the id is forgotten with it.
    res!(lane.remove(1));
    let n = res!(lane.append(val(1, 16), Some(&[0xab, 0x01]), None));
    res!(check(n.seq == 3 && !n.replay, fmt!("a replay after remove answered {:?}", n)));
    Ok(())
}

// A crash between the id write and the head write leaves A's id naming a sequence the next
// append, B, takes.  A replay of A must not answer B's entry as its own.
fn replay_after_a_torn_append_is_not_held(db: &TestDb, user: setup::Uid) -> Outcome<()> {
    let a_id: &[u8] = &[0xcd, 0x01];
    res!(db.insert(
        dat!("t/torn/e/1"),
        Dat::List(vec![Dat::U64(0), Dat::bytdat(a_id.to_vec()), val(1, 16)]),
        user,
        None,
    ));
    res!(db.insert(dat!("t/torn/i/cd01"), Dat::U64(1), user, None));
    let locks = LaneLocks::default();
    let g = locks.lock("t/torn");
    let lane: TestLane = Lane::new(db, user, &g);
    let b = res!(lane.append(val(2, 16), Some(&[0xcd, 0x02]), None));
    let r = res!(lane.append(val(1, 16), Some(a_id), None));
    res!(check(b.seq == 1 && r.seq == 2 && !r.replay,
        fmt!("after a torn append, B {:?}, the replay of A {:?}", b, r)));
    let p = res!(lane.read_after(0, 1 << 20, usize::MAX, NOW));
    res!(check(seqs(&p) == vec![1, 2] && p.entries[0].val == val(2, 16) && p.entries[1].val == val(1, 16),
        fmt!("after a torn append the lane holds {:?}", p.entries)));
    // The id now names A's own entry, so a second replay is held.
    let r = res!(lane.append(val(1, 16), Some(a_id), None));
    res!(check(r.seq == 2 && r.replay, fmt!("a second replay of A answered {:?}", r)));
    Ok(())
}

fn cursors_and_stable(db: &TestDb, user: setup::Uid) -> Outcome<()> {
    let locks = LaneLocks::default();
    let g = locks.lock("t/cur");
    let lane: TestLane = Lane::new(db, user, &g);
    for i in 1..=10 {
        res!(lane.append(val(i, 16), None, None));
    }
    res!(check(res!(lane.stable()) == 10, fmt!("with no reader stable is {}", res!(lane.stable()))));
    res!(lane.ack("phone", 4));
    res!(lane.ack("tab", 7));
    res!(lane.ack("laptop", 2));
    res!(check(res!(lane.stable()) == 2, fmt!("stable {} over 4, 7, 2", res!(lane.stable()))));
    // A dormant reader leaves stable, keeping its cursor; acknowledging brings it back.
    res!(check(res!(lane.set_dormant("laptop")), fmt!("laptop unknown")));
    res!(check(res!(lane.stable()) == 4, fmt!("stable {} with laptop dormant", res!(lane.stable()))));
    let rs = res!(lane.readers());
    let laptop = rs.iter().find(|r| r.name == "laptop");
    res!(check(matches!(laptop, Some(r) if r.cursor == 2 && r.state == ReaderState::Dormant),
        fmt!("laptop reads {:?}", laptop)));
    res!(lane.ack("laptop", 3));
    res!(check(res!(lane.stable()) == 3, fmt!("stable {} with laptop back at 3", res!(lane.stable()))));
    // A forgotten reader leaves it too.
    res!(check(res!(lane.forget("laptop")), fmt!("laptop not forgotten")));
    res!(check(!res!(lane.forget("laptop")), fmt!("laptop forgotten twice")));
    res!(check(res!(lane.stable()) == 4, fmt!("stable {} with laptop forgotten", res!(lane.stable()))));
    // A cursor past the head is held at it, so stable never exceeds the head.
    res!(check(res!(lane.ack("phone", 50)) == 10, fmt!("a cursor past the head was kept")));
    res!(lane.ack("tab", 99));
    res!(check(res!(lane.stable()) == 10, fmt!("stable {} with every reader at the head", res!(lane.stable()))));
    // All dormant: the head.
    res!(lane.ack("phone", 1));
    res!(lane.set_dormant("phone"));
    res!(lane.set_dormant("tab"));
    res!(check(res!(lane.stable()) == 10, fmt!("stable {} with no live reader", res!(lane.stable()))));
    res!(check(!res!(lane.set_dormant("nobody")), fmt!("an unknown reader was marked dormant")));
    Ok(())
}

fn trim_never_passes_the_ask(db: &TestDb, user: setup::Uid) -> Outcome<()> {
    let locks = LaneLocks::default();
    let g = locks.lock("t/trim");
    let lane: TestLane = Lane::new(db, user, &g);
    for i in 1..=10 {
        res!(lane.append(val(i, 16), None, None));
    }
    res!(lane.ack("a", 6));
    res!(lane.ack("b", 4));
    let s = res!(lane.stable());
    let f = res!(lane.trim_through(s));
    let p = res!(lane.read_after(0, 1 << 20, usize::MAX, NOW));
    res!(check(f == 4 && seqs(&p) == vec![5, 6, 7, 8, 9, 10],
        fmt!("trim through stable {}: floor {}, left {:?}", s, f, seqs(&p))));
    // A reader below the floor is told of the gap; one at or above it is not.
    res!(check(p.gap && p.floor == 4, fmt!("from 0 over floor 4: gap {} floor {}", p.gap, p.floor)));
    let p = res!(lane.read_after(4, 1 << 20, usize::MAX, NOW));
    res!(check(!p.gap && p.floor == 4 && seqs(&p) == vec![5, 6, 7, 8, 9, 10],
        fmt!("from the floor: gap {} floor {} {:?}", p.gap, p.floor, seqs(&p))));
    // Never lowers the floor, and never passes the head.
    res!(check(res!(lane.trim_through(2)) == 4, fmt!("a lower trim moved the floor")));
    res!(check(res!(lane.trim_through(1_000)) == 10, fmt!("a trim past the head")));
    let p = res!(lane.read_after(0, 1 << 20, usize::MAX, NOW));
    res!(check(p.entries.is_empty() && res!(lane.depth()) == 0, fmt!("after trimming all: {:?}", p)));
    let a = res!(lane.append(val(11, 16), None, None));
    res!(check(a.seq == 11, fmt!("after trimming all, append got {}", a.seq)));
    Ok(())
}

fn survives_a_restart(db: &TestDb, user: setup::Uid) -> Outcome<()> {
    let locks = LaneLocks::default();
    let g = locks.lock("t/cur");
    let lane: TestLane = Lane::new(db, user, &g);
    res!(check(res!(lane.head()) == 10, fmt!("after a restart head {}", res!(lane.head()))));
    let rs = res!(lane.readers());
    res!(check(rs.len() == 2 && rs.iter().all(|r| r.state == ReaderState::Dormant),
        fmt!("after a restart readers {:?}", rs)));
    let locks = LaneLocks::default();
    let g = locks.lock("t/hole");
    let lane: TestLane = Lane::new(db, user, &g);
    let p = res!(lane.read_after(0, 1 << 20, usize::MAX, NOW));
    res!(check(seqs(&p) == vec![4, 5, 6, 7, 9], fmt!("after a restart holes {:?}", seqs(&p))));
    let locks = LaneLocks::default();
    let g = locks.lock("t/replay");
    let r = res!(TestLane::new(db, user, &g).append(val(2, 16), Some(&[0xab, 0x02]), None));
    res!(check(r.seq == 2 && r.replay, fmt!("after a restart a replay answered {:?}", r)));
    Ok(())
}

// Eight threads each append the same eight ids under the store's READ lock and the lane's own
// lock, as a gateway's request handlers do: every id lands once.
fn replay_under_the_lane_lock(root: &PathBuf, user: setup::Uid) -> Outcome<()> {
    let db = Arc::new(RwLock::new(res!(open(root, false))));
    let locks = Arc::new(LaneLocks::default());
    let mut hs = Vec::new();
    for t in 0..8u64 {
        let db = db.clone();
        let locks = locks.clone();
        hs.push(thread::spawn(move || -> Outcome<Vec<u64>> {
            let mut got = Vec::new();
            for k in 0..8u64 {
                let id = ((t + k) % 8).to_be_bytes();
                let shared = match db.read() {
                    Ok(g)   => g,
                    Err(_)  => return Err(err!("The store lock is poisoned."; Test, Lock)),
                };
                let g = locks.lock("t/excl");
                let lane: TestLane = Lane::new(&*shared, user, &g);
                got.push(res!(lane.append(val(k, 16), Some(&id), None)).seq);
            }
            Ok(got)
        }));
    }
    for h in hs {
        match h.join() {
            Ok(Ok(_))   => {},
            Ok(Err(e))  => return Err(e),
            Err(_)      => return Err(err!("An append thread panicked."; Test, Thread)),
        }
    }
    {
        let shared = match db.read() {
            Ok(g)   => g,
            Err(_)  => return Err(err!("The store lock is poisoned."; Test, Lock)),
        };
        let g = locks.lock("t/excl");
        let lane: TestLane = Lane::new(&*shared, user, &g);
        let p = res!(lane.read_after(0, 1 << 20, usize::MAX, NOW));
        res!(check(res!(lane.head()) == 8 && seqs(&p) == (1..=8).collect::<Vec<u64>>(),
            fmt!("64 appends of 8 ids: head {}, entries {:?}", res!(lane.head()), seqs(&p))));
    }
    res!(lanes_run_side_by_side(&db, &locks, user));
    drop(locks);
    let db = match Arc::try_unwrap(db) {
        Ok(l)   => l,
        Err(_)  => return Err(err!("The store is still shared."; Test, Unexpected)),
    };
    let db = match db.into_inner() {
        Ok(d)   => d,
        Err(_)  => return Err(err!("The store lock is poisoned."; Test, Lock)),
    };
    res!(db.shutdown());
    Ok(())
}

// While one lane's lock is held, that lane cannot be taken again, and another lane appends and
// reads on another thread without waiting for it (C8a check F1: one request's lane section must
// not hold up the rest of the store).
fn lanes_run_side_by_side(db: &Arc<RwLock<TestDb>>, locks: &Arc<LaneLocks>, user: setup::Uid)
    -> Outcome<()>
{
    let held = locks.lock("t/side/a");
    res!(check(locks.try_lock("t/side/a").is_none(), fmt!("a held lane was taken again")));
    // Another lane on another stripe (the stripe is drawn under a random key).
    let mut other = None;
    for i in 0..32 {
        let p = fmt!("t/side/b{}", i);
        if locks.try_lock(p.clone()).is_some() {
            other = Some(p);
            break;
        }
    }
    let other = match other {
        Some(p) => p,
        None    => return Err(err!("No lane off the held stripe in 32 tries."; Test, Unexpected)),
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let (db2, locks2) = (db.clone(), locks.clone());
    let h = thread::spawn(move || {
        let run = || -> Outcome<u64> {
            let shared = match db2.read() {
                Ok(g)   => g,
                Err(_)  => return Err(err!("The store lock is poisoned."; Test, Lock)),
            };
            let g = locks2.lock(other);
            let lane: TestLane = Lane::new(&*shared, user, &g);
            res!(lane.append(val(1, 16), None, None));
            let p = res!(lane.read_after(0, 1 << 20, usize::MAX, NOW));
            Ok(p.through)
        };
        let _ = tx.send(run());
    });
    let got = match rx.recv_timeout(std::time::Duration::from_secs(10)) {
        Ok(r)   => r,
        Err(_)  => return Err(err!("Another lane waited 10 s on a held lane."; Test, Unexpected)),
    };
    drop(held);
    if h.join().is_err() {
        return Err(err!("The side lane's thread panicked."; Test, Thread));
    }
    let through = res!(got);
    check(through == 1, fmt!("the side lane read through {}", through))
}

// A page stops after `max_steps` slots, holes counting, with `more` set and `through` at the
// last slot visited (C8a check F1: a walk over thousands of holes held the lane for seconds).
fn steps_bound_a_page(db: &TestDb, user: setup::Uid) -> Outcome<()> {
    let locks = LaneLocks::default();
    let g = locks.lock("t/step");
    let lane: TestLane = Lane::new(db, user, &g);
    for i in 1..=10u64 {
        res!(lane.append(val(i, 16), None, None));
    }
    for s in 1..=6u64 {
        res!(lane.remove(s));
    }
    let p = res!(lane.read_after(0, 1 << 20, 4, NOW));
    res!(check(p.entries.is_empty() && p.through == 4 && p.more,
        fmt!("four steps over six holes: {:?}", p)));
    let p = res!(lane.read_after(p.through, 1 << 20, 4, NOW));
    res!(check(seqs(&p) == vec![7, 8] && p.through == 8 && p.more,
        fmt!("the next four steps: {:?}", p)));
    let p = res!(lane.read_after(p.through, 1 << 20, 4, NOW));
    res!(check(seqs(&p) == vec![9, 10] && p.through == 10 && !p.more,
        fmt!("the last steps: {:?}", p)));
    let p = res!(lane.read_after(0, 1 << 20, 0, NOW));
    res!(check(p.entries.is_empty() && p.through == 0 && p.more,
        fmt!("no steps: {:?}", p)));
    Ok(())
}

// A page measured by the caller's own sizes (a wire form, say) rather than stored bytes.
fn measured_pages(db: &TestDb, user: setup::Uid) -> Outcome<()> {
    let locks = LaneLocks::default();
    let g = locks.lock("t/meas");
    let lane: TestLane = Lane::new(db, user, &g);
    for i in 1..=5u64 {
        res!(lane.append(val(i, 16), None, None));
    }
    let p = res!(lane.read_after_measured(0, 250, usize::MAX, NOW, |_| Some(100)));
    res!(check(seqs(&p) == vec![1, 2] && p.more, fmt!("250 of 100s: {:?}", p)));
    let p = lane.read_after_measured(0, 250, usize::MAX, NOW, |_| None);
    check(p.is_err(), fmt!("an unmeasurable entry answered {:?}", p))
}

// A trim drops durable entries only: an ephemeral at or below the floor (a Stop, a consent) stays
// at its own sequence and reads to a reader below it until it expires, before and after the
// sweep (Daimond C8 check F1, amendment 10).
fn a_trim_keeps_live_ephemerals(db: &TestDb, user: setup::Uid) -> Outcome<()> {
    let locks = LaneLocks::default();
    let g = locks.lock("t/keep");
    let lane: TestLane = Lane::new(db, user, &g);
    for i in 1..=10u64 {
        let exp = if i == 4 || i == 7 { Some(NOW + 60_000) } else { None };
        res!(lane.append(val(i, 16), None, exp));
    }
    res!(check(res!(lane.trim_through(8)) == 8, fmt!("trim through 8")));
    // Before any sweep: the floor is up, nothing deleted, durables below it read absent.
    let p = res!(lane.read_after(3, 1 << 20, usize::MAX, NOW));
    res!(check(seqs(&p) == vec![4, 7, 9, 10] && p.gap && p.through == 10,
        fmt!("unswept, from 3: {:?}", p)));
    res!(check(res!(lane.trim_pending(NOW)), fmt!("nothing pending after a trim")));
    // Swept in steps of 3: three steps reach the floor, the two ephemerals listed.
    let mut n = 0;
    while res!(lane.sweep_trimmed(3, NOW)) {
        n += 1;
        res!(check(n < 10, fmt!("the sweep never ends")));
    }
    res!(check(n == 2, fmt!("swept in {} pending steps", n)));
    let held: Vec<u64> = (1..=8u64).filter(|s| matches!(
        db.get(&dat!(fmt!("t/keep/e/{}", s)), None), Ok(Some(_)))).collect();
    res!(check(held == vec![4, 7], fmt!("held below the floor after the sweep: {:?}", held)));
    let p = res!(lane.read_after(0, 1 << 20, usize::MAX, NOW));
    res!(check(seqs(&p) == vec![4, 7, 9, 10] && p.gap && p.steps == 4,
        fmt!("swept, from 0: {:?}", p)));
    let p = res!(lane.read_after(5, 1 << 20, usize::MAX, NOW));
    res!(check(seqs(&p) == vec![7, 9, 10], fmt!("swept, from 5: {:?}", p)));
    let p = res!(lane.read_after(8, 1 << 20, usize::MAX, NOW));
    res!(check(seqs(&p) == vec![9, 10] && !p.gap, fmt!("from the floor: {:?}", p)));
    // A page of one entry from 0 goes on from the kept one it stopped at.
    let p = res!(lane.read_after(0, 1, usize::MAX, NOW));
    res!(check(seqs(&p) == vec![4] && p.more && p.through == 4, fmt!("a one-entry page: {:?}", p)));
    let p = res!(lane.read_after(p.through, 1, usize::MAX, NOW));
    res!(check(seqs(&p) == vec![7] && p.more, fmt!("the next one-entry page: {:?}", p)));
    // Expired: unread at once, deleted by the next sweep step, which then has nothing left.
    let later = NOW + 60_000;
    let p = res!(lane.read_after(0, 1 << 20, usize::MAX, later));
    res!(check(seqs(&p) == vec![9, 10] && p.dropped == 0, fmt!("after expiry: {:?}", p)));
    res!(check(res!(lane.trim_pending(later)), fmt!("an expired kept entry is not pending")));
    res!(check(!res!(lane.sweep_trimmed(8, later)), fmt!("work left after dropping the kept")));
    let held = (1..=8u64).any(|s| matches!(db.get(&dat!(fmt!("t/keep/e/{}", s)), None), Ok(Some(_))));
    res!(check(!held && !res!(lane.trim_pending(later)), fmt!("an expired kept entry is still stored")));
    // An expired entry above the floor is reported in `dropped`.
    res!(lane.append(val(11, 16), None, Some(NOW + 1)));
    let p = res!(lane.read_after(8, 1 << 20, usize::MAX, later));
    res!(check(seqs(&p) == vec![9, 10] && p.dropped == val(11, 16).byte_len().unwrap_or(0) as u64,
        fmt!("dropped above the floor: {:?}", p)));
    Ok(())
}

// Each sweep step visits at most its budget, and a reader below an unswept floor walks the
// slots itself, bounded by its own steps.
fn the_sweep_is_bounded(db: &TestDb, user: setup::Uid) -> Outcome<()> {
    let locks = LaneLocks::default();
    let g = locks.lock("t/sweep");
    let lane: TestLane = Lane::new(db, user, &g);
    for i in 1..=100u64 {
        res!(lane.append(val(i, 16), Some(&i.to_be_bytes()), None));
    }
    res!(lane.trim_through(100));
    res!(check(res!(lane.sweep_trimmed(10, NOW)), fmt!("ten of a hundred left nothing")));
    let held = (1..=100u64).filter(|s| matches!(
        db.get(&dat!(fmt!("t/sweep/e/{}", s)), None), Ok(Some(_)))).count();
    res!(check(held == 90, fmt!("one step of 10 left {} held", held)));
    let p = res!(lane.read_after(0, 1 << 20, 25, NOW));
    // The swept ten are passed at no step (nothing is kept there), then 25 slots walked.
    res!(check(p.entries.is_empty() && p.more && p.through == 35 && p.steps == 25,
        fmt!("a reader below the unswept floor: {:?}", p)));
    // A replay of a trimmed, unswept entry is held; once swept it appends anew.
    let a = res!(lane.append(val(50, 16), Some(&50u64.to_be_bytes()), None));
    res!(check(a.replay && a.seq == 50, fmt!("unswept replay: {:?}", a)));
    while res!(lane.sweep_trimmed(10, NOW)) {}
    let a = res!(lane.append(val(5, 16), Some(&5u64.to_be_bytes()), None));
    res!(check(!a.replay && a.seq == 101, fmt!("swept replay: {:?}", a)));
    let p = res!(lane.read_after(0, 1 << 20, 3, NOW));
    res!(check(seqs(&p) == vec![101] && p.steps == 1, fmt!("swept, from 0: {:?}", p)));
    Ok(())
}
