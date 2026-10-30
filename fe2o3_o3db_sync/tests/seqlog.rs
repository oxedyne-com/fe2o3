use oxedyne_fe2o3_o3db_sync::seqlog::{
    SeqLog,
    SeqRead,
    SeqStore,
    SnapReg,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

use std::{
    cell::RefCell,
    collections::BTreeMap,
};

/// A store in memory, keyed by the key's debug form.
#[derive(Default)]
struct Mem {
    map: RefCell<BTreeMap<String, Dat>>,
}

impl SeqStore for Mem {
    fn get(&self, key: &Dat) -> Outcome<Option<Dat>> {
        Ok(self.map.borrow().get(&fmt!("{:?}", key)).cloned())
    }
    fn put(&self, key: Dat, val: Dat) -> Outcome<()> {
        self.map.borrow_mut().insert(fmt!("{:?}", key), val);
        Ok(())
    }
    fn delete(&self, key: &Dat) -> Outcome<bool> {
        Ok(self.map.borrow_mut().remove(&fmt!("{:?}", key)).is_some())
    }
}

fn rows(names: &[&str]) -> Vec<Dat> {
    names.iter().map(|n| dat!(n.to_string())).collect()
}

fn seqs(r: &SeqRead) -> Vec<u64> {
    match r {
        SeqRead::Rows { rows, .. } => rows.iter().map(|(s, _)| *s).collect(),
        SeqRead::Gap { .. }        => Vec::new(),
    }
}

#[test]
fn appends_take_contiguous_seqs_and_survive_a_reload() -> Outcome<()> {
    let st = Mem::default();
    let log = SeqLog::new("clog:a:c", "clbat:a:c:");
    let mut idx = res!(log.load(&st));
    assert_eq!(res!(log.append(&st, &mut idx, rows(&["a", "b"]), 10, 2)), 1);
    assert_eq!(res!(log.append(&st, &mut idx, rows(&["c"]), 11, 1)), 3);
    let idx = res!(log.load(&st));
    assert_eq!((idx.head, idx.low, idx.batches.len(), idx.bytes()), (3, 1, 2, 3));
    assert_eq!(seqs(&res!(log.read(&st, &idx, 0, 100))), vec![1, 2, 3]);
    assert_eq!(seqs(&res!(log.read(&st, &idx, 2, 100))), vec![3]);
    assert_eq!(seqs(&res!(log.read(&st, &idx, 3, 100))), Vec::<u64>::new());
    Ok(())
}

#[test]
fn a_read_stops_at_its_limit_and_says_more() -> Outcome<()> {
    let st = Mem::default();
    let log = SeqLog::new("i", "b:");
    let mut idx = res!(log.load(&st));
    res!(log.append(&st, &mut idx, rows(&["a", "b", "c"]), 1, 3));
    match res!(log.read(&st, &idx, 0, 2)) {
        SeqRead::Rows { rows, more } => {
            assert_eq!(rows.len(), 2);
            assert!(more);
        }
        SeqRead::Gap { .. } => return Err(err!("A full log answered a gap."; Test)),
    }
    Ok(())
}

#[test]
fn a_cursor_below_a_trim_answers_gap() -> Outcome<()> {
    let st = Mem::default();
    let log = SeqLog::new("i", "b:");
    let mut idx = res!(log.load(&st));
    res!(log.append(&st, &mut idx, rows(&["a", "b"]), 1, 2));
    res!(log.append(&st, &mut idx, rows(&["c"]), 2, 1));
    res!(log.register_snap(&st, &mut idx, 3, dat!("s3".to_string())));
    assert_eq!(res!(log.truncate_through(&st, &mut idx, 2)), 1);
    assert_eq!(idx.low, 3);
    assert!(res!(st.get(&log.batch_key(1))).is_none(), "the trimmed batch is deleted");
    match res!(log.read(&st, &idx, 0, 10)) {
        SeqRead::Gap { low, .. } => assert_eq!(low, 3),
        other => return Err(err!("Expected a gap, read {:?}.", other; Test)),
    }
    assert_eq!(seqs(&res!(log.read(&st, &idx, 2, 10))), vec![3]);
    // Trimmed empty: the next append still numbers on from the head.
    res!(log.trim(&st, &mut idx, |_, _| true));
    assert_eq!((idx.low, idx.head), (4, 3));
    assert_eq!(res!(log.append(&st, &mut idx, rows(&["d"]), 3, 1)), 4);
    assert_eq!(idx.low, 4);
    Ok(())
}

#[test]
fn ext_round_trips_and_purge_leaves_nothing() -> Outcome<()> {
    let st = Mem::default();
    let log = SeqLog::new("i", "b:");
    let mut idx = res!(log.load(&st));
    idx.ext = dat!("caller".to_string());
    res!(log.append(&st, &mut idx, rows(&["a"]), 1, 1));
    let back = res!(log.load(&st));
    assert_eq!(back.ext, dat!("caller".to_string()));
    res!(log.purge(&st, &back));
    assert!(st.map.borrow().is_empty());
    Ok(())
}

#[test]
fn a_lock_entry_goes_when_its_last_holder_leaves() -> Outcome<()> {
    let locks = oxedyne_fe2o3_o3db_sync::seqlog::SeqLocks::new();
    let n = res!(locks.with("a:c", || {
        locks.with("a", || Ok(7))
    }));
    assert_eq!(n, 7);
    assert_eq!(res!(locks.len()), 0, "an idle key stays in the map");
    Ok(())
}

fn ext(s: &str) -> Dat { dat!(s.to_string()) }

/// A log of four single-row batches, seqs 1 to 4, appended at times 1 to 4.
fn four(st: &Mem, log: &SeqLog) -> Outcome<oxedyne_fe2o3_o3db_sync::seqlog::SeqIndex> {
    let mut idx = res!(log.load(st));
    for (i, n) in ["a", "b", "c", "d"].iter().enumerate() {
        res!(log.append(st, &mut idx, rows(&[n]), i as i64 + 1, 1));
    }
    Ok(idx)
}

#[test]
fn a_snapshot_registers_and_survives_a_reload() -> Outcome<()> {
    let st = Mem::default();
    let log = SeqLog::new("i", "b:");
    let mut idx = res!(four(&st, &log));
    match res!(log.register_snap(&st, &mut idx, 2, ext("s2"))) {
        SnapReg::Registered { prev: None } => (),
        other => return Err(err!("Expected a first registration, got {:?}.", other; Test)),
    }
    let back = res!(log.load(&st));
    let snap = res!(back.snap.ok_or_else(|| err!("The snap did not survive a reload."; Test)));
    assert_eq!((snap.seq, snap.ext), (2, ext("s2")));
    // A higher one supersedes it and hands the old one back for its refs to be released.
    match res!(log.register_snap(&st, &mut idx, 4, ext("s4"))) {
        SnapReg::Registered { prev: Some(p) } => assert_eq!((p.seq, p.ext), (2, ext("s2"))),
        other => return Err(err!("Expected a superseding registration, got {:?}.", other; Test)),
    }
    assert_eq!(res!(log.load(&st)).snap.map(|s| s.seq), Some(4));
    Ok(())
}

#[test]
fn a_snapshot_at_or_below_the_current_or_above_the_head_is_refused() -> Outcome<()> {
    let st = Mem::default();
    let log = SeqLog::new("i", "b:");
    let mut idx = res!(four(&st, &log));
    res!(log.register_snap(&st, &mut idx, 3, ext("s3")));
    match res!(log.register_snap(&st, &mut idx, 2, ext("s2"))) {
        SnapReg::Behind { least } => assert_eq!(least, 4),
        other => return Err(err!("A snap below the current one was {:?}.", other; Test)),
    }
    match res!(log.register_snap(&st, &mut idx, 5, ext("s5"))) {
        SnapReg::Ahead { head } => assert_eq!(head, 4),
        other => return Err(err!("A snap above the head was {:?}.", other; Test)),
    }
    // Same seq, same ext: idempotent. Same seq, other ext: a conflict.
    match res!(log.register_snap(&st, &mut idx, 3, ext("s3"))) {
        SnapReg::Same => (),
        other => return Err(err!("A retried snap was {:?}.", other; Test)),
    }
    match res!(log.register_snap(&st, &mut idx, 3, ext("x3"))) {
        SnapReg::Conflict { snap } => assert_eq!(snap.ext, ext("s3")),
        other => return Err(err!("A same-seq different snap was {:?}.", other; Test)),
    }
    assert_eq!(res!(log.load(&st)).snap.map(|s| (s.seq, s.ext)), Some((3, ext("s3"))));
    Ok(())
}

#[test]
fn seq_zero_registers_once_and_only_on_an_empty_log() -> Outcome<()> {
    let st = Mem::default();
    let log = SeqLog::new("i", "b:");
    let mut idx = res!(log.load(&st));
    match res!(log.register_snap(&st, &mut idx, 0, ext("m"))) {
        SnapReg::Registered { prev: None } => (),
        other => return Err(err!("Seq 0 on an empty log was {:?}.", other; Test)),
    }
    match res!(log.register_snap(&st, &mut idx, 0, ext("m"))) {
        SnapReg::Same => (),
        other => return Err(err!("A same-ext repeat at 0 was {:?}.", other; Test)),
    }
    match res!(log.register_snap(&st, &mut idx, 0, ext("other"))) {
        SnapReg::Conflict { .. } => (),
        other => return Err(err!("A different repeat at 0 was {:?}.", other; Test)),
    }
    // On a log with rows and no snap, seq 0 is refused.
    let st2 = Mem::default();
    let mut idx2 = res!(four(&st2, &log));
    match res!(log.register_snap(&st2, &mut idx2, 0, ext("m"))) {
        SnapReg::Behind { least } => assert_eq!(least, 1),
        other => return Err(err!("Seq 0 on a log with rows was {:?}.", other; Test)),
    }
    assert!(idx2.snap.is_none());
    Ok(())
}

#[test]
fn a_read_below_low_answers_the_snapshot() -> Outcome<()> {
    let st = Mem::default();
    let log = SeqLog::new("i", "b:");
    let mut idx = res!(four(&st, &log));
    res!(log.register_snap(&st, &mut idx, 2, ext("s2")));
    assert_eq!(res!(log.trim(&st, &mut idx, |_, _| true)), 2);
    assert_eq!(idx.low, 3);
    match res!(log.read(&st, &idx, 0, 10)) {
        SeqRead::Gap { low, snap: Some(s) } => assert_eq!((low, s.seq, s.ext), (3, 2, ext("s2"))),
        other => return Err(err!("Expected a gap with the snap, read {:?}.", other; Test)),
    }
    // From the snap's seq the tail reads whole.
    assert_eq!(seqs(&res!(log.read(&st, &idx, 2, 10))), vec![3, 4]);
    Ok(())
}

#[test]
fn a_trim_never_cuts_above_the_snapshot_whatever_the_predicate() -> Outcome<()> {
    let st = Mem::default();
    let log = SeqLog::new("i", "b:");
    let mut idx = res!(log.load(&st));
    // Batches 1-5, 6-10, 11-12, 13-15, all old enough for any ttl.
    for n in [5usize, 5, 2, 3] {
        let names: Vec<&str> = std::iter::repeat("r").take(n).collect();
        res!(log.append(&st, &mut idx, rows(&names), 0, n as u64));
    }
    res!(log.register_snap(&st, &mut idx, 10, ext("s10")));
    // A ttl backstop that would cut everything, and that sees the snap.
    let saw = std::cell::Cell::new(None);
    assert_eq!(res!(log.trim(&st, &mut idx, |_, i| {
        saw.set(i.snap.as_ref().map(|s| s.seq));
        true
    })), 2);
    assert_eq!(saw.get(), Some(10), "the predicate sees the snap");
    assert_eq!(idx.low, 11);
    assert_eq!(seqs(&res!(log.read(&st, &idx, 10, 100))), (11..=15).collect::<Vec<u64>>());
    // truncate_through goes through the same bound.
    assert_eq!(res!(log.truncate_through(&st, &mut idx, 15)), 0);
    assert_eq!(idx.low, 11);
    Ok(())
}

#[test]
fn a_trim_with_no_snapshot_keeps_everything() -> Outcome<()> {
    let st = Mem::default();
    let log = SeqLog::new("i", "b:");
    let mut idx = res!(four(&st, &log));
    assert_eq!(res!(log.trim(&st, &mut idx, |_, _| true)), 0);
    assert_eq!(res!(log.truncate_through(&st, &mut idx, 4)), 0);
    assert_eq!((idx.low, idx.batches.len()), (1, 4));
    assert_eq!(seqs(&res!(log.read(&st, &idx, 0, 100))), vec![1, 2, 3, 4]);
    Ok(())
}
