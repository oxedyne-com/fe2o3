//! The sequence lane on a real Ozone store: rising sequence numbers, pages bounded by bytes that
//! answer `more`, holes left by `remove` and `trim_through`, lazy expiry, replayed appends, reader
//! cursors and `stable`, and the lane read back after a restart.

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
        res!(append_rises(&mut db, user));
        res!(pages_honour_max_bytes(&mut db, user));
        res!(holes_are_skipped(&mut db, user));
        res!(expired_reads_absent_and_is_deleted(&mut db, user));
        res!(replayed_append_does_not_double(&mut db, user));
        res!(replay_after_a_torn_append_is_not_held(&mut db, user));
        res!(cursors_and_stable(&mut db, user));
        res!(trim_never_passes_the_ask(&mut db, user));
        res!(db.shutdown());
    }
    {
        let mut db = res!(open(&root, false));
        res!(survives_a_restart(&mut db, user));
        res!(db.shutdown());
    }
    res!(replay_under_one_exclusive_section(&root, user));
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

fn append_rises(db: &mut TestDb, user: setup::Uid) -> Outcome<()> {
    let lane: TestLane = Lane::new(db, user, "t/rise");
    res!(check(res!(lane.head()) == 0, fmt!("a new lane has head {}", res!(lane.head()))));
    for i in 1..=5 {
        let a = res!(lane.append(val(i, 16), None, None));
        res!(check(a.seq == i && !a.replay, fmt!("append {} answered {:?}", i, a)));
    }
    let p = res!(lane.read_after(0, 1 << 20, NOW));
    res!(check(seqs(&p) == vec![1, 2, 3, 4, 5] && p.through == 5 && !p.more,
        fmt!("read all: {:?} through {} more {}", seqs(&p), p.through, p.more)));
    res!(check(p.entries[2].val == val(3, 16), fmt!("entry 3 reads {:?}", p.entries[2].val)));
    let p = res!(lane.read_after(3, 1 << 20, NOW));
    res!(check(seqs(&p) == vec![4, 5], fmt!("read after 3: {:?}", seqs(&p))));
    let p = res!(lane.read_after(5, 1 << 20, NOW));
    res!(check(p.entries.is_empty() && p.through == 5 && !p.more, fmt!("read after head: {:?}", p)));
    // A cursor beyond the head answers the head.
    let p = res!(lane.read_after(99, 1 << 20, NOW));
    res!(check(p.entries.is_empty() && p.through == 5, fmt!("read after 99: through {}", p.through)));
    Ok(())
}

fn pages_honour_max_bytes(db: &mut TestDb, user: setup::Uid) -> Outcome<()> {
    let lane: TestLane = Lane::new(db, user, "t/page");
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
        let p = res!(lane.read_after(after, 3 * size + size / 2, NOW));
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
    let p = res!(lane.read_after(0, 10, NOW));
    res!(check(seqs(&p) == vec![1] && p.more, fmt!("a tiny page: {:?} more {}", seqs(&p), p.more)));
    Ok(())
}

fn holes_are_skipped(db: &mut TestDb, user: setup::Uid) -> Outcome<()> {
    let lane: TestLane = Lane::new(db, user, "t/hole");
    for i in 1..=8 {
        res!(lane.append(val(i, 16), None, None));
    }
    res!(check(res!(lane.remove(3)), fmt!("remove 3 found nothing")));
    res!(check(!res!(lane.remove(3)), fmt!("a second remove of 3 found it")));
    res!(check(res!(lane.remove(8)), fmt!("remove 8 found nothing")));
    res!(check(res!(lane.trim_through(2)) == 2, fmt!("trim through 2")));
    let p = res!(lane.read_after(0, 1 << 20, NOW));
    res!(check(seqs(&p) == vec![4, 5, 6, 7] && p.through == 8 && !p.more,
        fmt!("holes: {:?} through {}", seqs(&p), p.through)));
    // The head is not lowered by removing its entry: the next append is 9.
    let a = res!(lane.append(val(9, 16), None, None));
    res!(check(a.seq == 9, fmt!("after removing the top, append got {}", a.seq)));
    res!(check(res!(lane.depth()) == 7 && res!(lane.floor()) == 2,
        fmt!("depth {} floor {}", res!(lane.depth()), res!(lane.floor()))));
    Ok(())
}

fn expired_reads_absent_and_is_deleted(db: &mut TestDb, user: setup::Uid) -> Outcome<()> {
    let lane: TestLane = Lane::new(db, user, "t/exp");
    res!(lane.append(val(1, 16), None, Some(NOW + 1_000)));
    res!(lane.append(val(2, 16), Some(b"two"), Some(NOW + 5_000)));
    res!(lane.append(val(3, 16), None, None));
    let p = res!(lane.read_after(0, 1 << 20, NOW));
    res!(check(seqs(&p) == vec![1, 2, 3] && p.entries[0].exp_ms == Some(NOW + 1_000),
        fmt!("before expiry: {:?}", p.entries)));
    let p = res!(lane.read_after(0, 1 << 20, NOW + 1_000));
    res!(check(seqs(&p) == vec![2, 3] && p.through == 3, fmt!("at entry 1's expiry: {:?}", seqs(&p))));
    // Deleted, not merely hidden: a read at an earlier time does not see it again.
    let p = res!(lane.read_after(0, 1 << 20, NOW));
    res!(check(seqs(&p) == vec![2, 3], fmt!("entry 1 came back: {:?}", seqs(&p))));
    let p = res!(lane.read_after(0, 1 << 20, NOW + 9_000));
    res!(check(seqs(&p) == vec![3], fmt!("after both expiries: {:?}", seqs(&p))));
    // The expired entry's id went with it, so its replay appends anew.
    let a = res!(lane.append(val(2, 16), Some(b"two"), None));
    res!(check(a.seq == 4 && !a.replay, fmt!("a replay after expiry answered {:?}", a)));
    let held = res!(db.get(&dat!("t/exp/e/1"), None)).is_some()
        || res!(db.get(&dat!("t/exp/e/2"), None)).is_some();
    res!(check(!held, fmt!("an expired entry is still stored")));
    Ok(())
}

fn replayed_append_does_not_double(db: &mut TestDb, user: setup::Uid) -> Outcome<()> {
    let lane: TestLane = Lane::new(db, user, "t/replay");
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
fn replay_after_a_torn_append_is_not_held(db: &mut TestDb, user: setup::Uid) -> Outcome<()> {
    let a_id: &[u8] = &[0xcd, 0x01];
    res!(db.insert(
        dat!("t/torn/e/1"),
        Dat::List(vec![Dat::U64(0), Dat::bytdat(a_id.to_vec()), val(1, 16)]),
        user,
        None,
    ));
    res!(db.insert(dat!("t/torn/i/cd01"), Dat::U64(1), user, None));
    let lane: TestLane = Lane::new(db, user, "t/torn");
    let b = res!(lane.append(val(2, 16), Some(&[0xcd, 0x02]), None));
    let r = res!(lane.append(val(1, 16), Some(a_id), None));
    res!(check(b.seq == 1 && r.seq == 2 && !r.replay,
        fmt!("after a torn append, B {:?}, the replay of A {:?}", b, r)));
    let p = res!(lane.read_after(0, 1 << 20, NOW));
    res!(check(seqs(&p) == vec![1, 2] && p.entries[0].val == val(2, 16) && p.entries[1].val == val(1, 16),
        fmt!("after a torn append the lane holds {:?}", p.entries)));
    // The id now names A's own entry, so a second replay is held.
    let r = res!(lane.append(val(1, 16), Some(a_id), None));
    res!(check(r.seq == 2 && r.replay, fmt!("a second replay of A answered {:?}", r)));
    Ok(())
}

fn cursors_and_stable(db: &mut TestDb, user: setup::Uid) -> Outcome<()> {
    let lane: TestLane = Lane::new(db, user, "t/cur");
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

fn trim_never_passes_the_ask(db: &mut TestDb, user: setup::Uid) -> Outcome<()> {
    let lane: TestLane = Lane::new(db, user, "t/trim");
    for i in 1..=10 {
        res!(lane.append(val(i, 16), None, None));
    }
    res!(lane.ack("a", 6));
    res!(lane.ack("b", 4));
    let s = res!(lane.stable());
    let f = res!(lane.trim_through(s));
    let p = res!(lane.read_after(0, 1 << 20, NOW));
    res!(check(f == 4 && seqs(&p) == vec![5, 6, 7, 8, 9, 10],
        fmt!("trim through stable {}: floor {}, left {:?}", s, f, seqs(&p))));
    // A reader below the floor is told of the gap; one at or above it is not.
    res!(check(p.gap && p.floor == 4, fmt!("from 0 over floor 4: gap {} floor {}", p.gap, p.floor)));
    let p = res!(lane.read_after(4, 1 << 20, NOW));
    res!(check(!p.gap && p.floor == 4 && seqs(&p) == vec![5, 6, 7, 8, 9, 10],
        fmt!("from the floor: gap {} floor {} {:?}", p.gap, p.floor, seqs(&p))));
    // Never lowers the floor, and never passes the head.
    res!(check(res!(lane.trim_through(2)) == 4, fmt!("a lower trim moved the floor")));
    res!(check(res!(lane.trim_through(1_000)) == 10, fmt!("a trim past the head")));
    let p = res!(lane.read_after(0, 1 << 20, NOW));
    res!(check(p.entries.is_empty() && res!(lane.depth()) == 0, fmt!("after trimming all: {:?}", p)));
    let a = res!(lane.append(val(11, 16), None, None));
    res!(check(a.seq == 11, fmt!("after trimming all, append got {}", a.seq)));
    Ok(())
}

fn survives_a_restart(db: &mut TestDb, user: setup::Uid) -> Outcome<()> {
    let lane: TestLane = Lane::new(db, user, "t/cur");
    res!(check(res!(lane.head()) == 10, fmt!("after a restart head {}", res!(lane.head()))));
    let rs = res!(lane.readers());
    res!(check(rs.len() == 2 && rs.iter().all(|r| r.state == ReaderState::Dormant),
        fmt!("after a restart readers {:?}", rs)));
    let lane: TestLane = Lane::new(db, user, "t/hole");
    let p = res!(lane.read_after(0, 1 << 20, NOW));
    res!(check(seqs(&p) == vec![4, 5, 6, 7, 9], fmt!("after a restart holes {:?}", seqs(&p))));
    let r = res!(Lane::new(db, user, "t/replay").append(val(2, 16), Some(&[0xab, 0x02]), None));
    res!(check(r.seq == 2 && r.replay, fmt!("after a restart a replay answered {:?}", r)));
    Ok(())
}

// Eight threads each append the same eight ids under the write lock, as a gateway's request
// handlers would: every id lands once.
fn replay_under_one_exclusive_section(root: &PathBuf, user: setup::Uid) -> Outcome<()> {
    let db = Arc::new(RwLock::new(res!(open(root, false))));
    let mut hs = Vec::new();
    for t in 0..8u64 {
        let db = db.clone();
        hs.push(thread::spawn(move || -> Outcome<Vec<u64>> {
            let mut got = Vec::new();
            for k in 0..8u64 {
                let id = ((t + k) % 8).to_be_bytes();
                let mut guard = match db.write() {
                    Ok(g)   => g,
                    Err(_)  => return Err(err!("The store lock is poisoned."; Test, Lock)),
                };
                let lane: TestLane = Lane::new(&mut *guard, user, "t/excl");
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
    let mut guard = match db.write() {
        Ok(g)   => g,
        Err(_)  => return Err(err!("The store lock is poisoned."; Test, Lock)),
    };
    let lane: TestLane = Lane::new(&mut *guard, user, "t/excl");
    let p = res!(lane.read_after(0, 1 << 20, NOW));
    res!(check(res!(lane.head()) == 8 && seqs(&p) == (1..=8).collect::<Vec<u64>>(),
        fmt!("64 appends of 8 ids: head {}, entries {:?}", res!(lane.head()), seqs(&p))));
    drop(guard);
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
