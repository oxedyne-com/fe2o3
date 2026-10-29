use oxedyne_fe2o3_o3db_sync::seqlog::{
    SeqLog,
    SeqRead,
    SeqStore,
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
    assert_eq!(res!(log.truncate_through(&st, &mut idx, 2)), 1);
    assert_eq!(idx.low, 3);
    assert!(res!(st.get(&log.batch_key(1))).is_none(), "the trimmed batch is deleted");
    match res!(log.read(&st, &idx, 0, 10)) {
        SeqRead::Gap { low } => assert_eq!(low, 3),
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
