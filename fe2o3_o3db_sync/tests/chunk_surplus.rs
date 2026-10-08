//! An overwrite that changes a chunked value's geometry retires the chunks it leaves (A3 R4,
//! 2026-10-08).  A chunk is addressed by `(set_id, index, data length, count, size)`, so a value of
//! another length shares no chunk key with its predecessor: nothing supersedes the old chunks, and
//! until now they waited for the orphan sweep, which `compact_now` does not run.  Each erase test
//! finds the old chunks in the data files first (the positive control), overwrites, runs
//! `compact_now`, and counts the bytes of the files from outside the store.  The hooks used are
//! process-wide, so every test takes one lock and puts each hook back.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    chunk::PartKey,
};
use oxedyne_fe2o3_o3db_sync::{
    base::cfg::OzoneConfig,
    test::{
        hooks,
        setup::{
            self,
            Uid,
        },
    },
};

use std::{
    collections::BTreeMap,
    path::{
        Path,
        PathBuf,
    },
    sync::{
        Arc,
        Mutex,
        MutexGuard,
        atomic::{
            AtomicBool,
            Ordering,
        },
    },
    thread,
    time::Duration,
};

static HOOKS: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    match HOOKS.lock() { Ok(g) => g, Err(p) => p.into_inner() }
}

const CSIZE:    usize = 600;    // bytes in a chunk
const FIVE:     usize = 2_900;  // a value of five chunks
const TWO:      usize = 1_100;  // a value of two
const PLAIN:    usize = 500;    // below the threshold, so not chunked

fn chunk_cfg() -> Outcome<OzoneConfig> {
    let mut c = res!(setup::default_cfg());
    c.num_zones             = 1;
    c.num_cbots_per_zone    = 4;
    c.num_fbots_per_zone    = 1;
    c.num_wbots_per_zone    = 2;
    c.zone_overrides        = BTreeMap::new();
    c.rest_chunk_threshold  = 1_000;
    c.rest_chunk_bytes      = CSIZE as u64;
    c.sync_on_write         = true;
    Ok(c)
}

// Bytes that do not repeat inside a window, so that a window is found by its own record only.
fn fill_of(seed: u8, len: usize) -> Vec<u8> {
    let mut x = 0x9E37_79B9_7F4A_7C15u64 ^ ((seed as u64) << 32 | seed as u64);
    (0..len).map(|_| {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (x >> 56) as u8
    }).collect()
}

// A hundred bytes from the middle of each chunk, so that a chunk is found by its own bytes.  The
// encoded value is five bytes longer than the bytes in it, so a window of the middle is inside
// one chunk.
fn windows(seed: u8, len: usize) -> Vec<Vec<u8>> {
    let v = fill_of(seed, len);
    let n = (len + 5 + CSIZE - 1) / CSIZE;
    (0..n).map(|i| v[i * CSIZE + 250..i * CSIZE + 350].to_vec()).collect()
}

struct Store {
    dir:    String,
    root:   PathBuf,
    cfg:    OzoneConfig,
    db:     TestDb,
}

fn store(name: &str) -> Outcome<Store> {
    log_set_level!("error");
    let dir = fmt!("./test_db_surplus_{}", name);
    let _ = std::fs::remove_dir_all(&dir);
    res!(std::fs::create_dir_all(&dir));
    let root = res!(Path::new(&dir).canonicalize());
    let cfg = res!(chunk_cfg());
    // Collection off, so that only the order to compact collects.
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, false, true));
    Ok(Store { dir, root, cfg, db })
}

impl Store {
    fn holds(&self, seed: u8, len: usize) -> Vec<usize> {
        windows(seed, len).iter().map(|w| files_holding(&self.root, &self.cfg, w).len()).collect()
    }

    fn num_chunks(&self, k: &Dat) -> Outcome<u64> {
        let api = self.db.api();
        let resp = res!(api.fetch_using_schemes(k, None));
        match res!(resp.recv_daticle(api.schemes().encrypter(), None)) {
            (Some((Dat::Tup5u64(tup), _)), _) => Ok(PartKey(tup).num_parts()),
            other => Err(err!("Key {:?} holds {:?}, not a bunch key.", k, other; Test, Invalid)),
        }
    }

    fn put(&self, k: &Dat, seed: u8, len: usize) -> Outcome<()> {
        let _ = res!(self.db.insert(k.clone(), Dat::BU32(fill_of(seed, len)), Uid::default(), None));
        Ok(())
    }

    // Five chunks written and found in the files, with a value beside it that stays.
    fn five(&self, k: &Dat, seed: u8) -> Outcome<()> {
        res!(self.db.insert(key(0), value(0, 9), Uid::default(), None));
        res!(self.put(k, seed, FIVE));
        assert_eq!(res!(self.num_chunks(k)), 5, "the value did not split into five chunks");
        res!(self.db.api().settle_for_test(Duration::from_secs(10)));
        assert!(self.holds(seed, FIVE).iter().all(|n| *n >= 1),
            "positive control: every chunk is in a data file: {:?}", self.holds(seed, FIVE));
        Ok(())
    }

    fn compact_and_check(&self, k: &Dat, old: u8, new: u8, len: usize) -> Outcome<()> {
        res!(self.db.api().settle_for_test(Duration::from_secs(10)));
        let report = res!(self.db.compact_now(Duration::from_secs(30)));
        msg!("{:?}", report);
        assert_eq!(self.holds(old, FIVE), vec![0; 5], "old chunks still in data files after compact_now");
        match res!(self.db.get(k, None)) {
            Some((v, _)) => assert_eq!(v, Dat::BU32(fill_of(new, len)), "the new value reads back changed"),
            None => return Err(err!("The new value is gone."; Test, Missing)),
        }
        match res!(self.db.get(&key(0), None)) {
            Some((v, _)) => assert_eq!(v, value(0, 9), "the value beside it changed"),
            None => return Err(err!("The value beside it is gone."; Test, Missing)),
        }
        Ok(())
    }

    fn end(self) -> Outcome<()> {
        res!(self.db.close());
        let _ = std::fs::remove_dir_all(&self.dir);
        Ok(())
    }
}

#[test]
fn fewer_chunks_leave_no_old_chunk() -> Outcome<()> {
    let _lock = lock();
    let s = res!(store("fewer"));
    let k = dat!("shrinks to two");
    res!(s.five(&k, 5));
    res!(s.put(&k, 7, TWO));
    assert_eq!(res!(s.num_chunks(&k)), 2, "the overwrite did not split into two chunks");
    res!(s.compact_and_check(&k, 5, 7, TWO));
    assert!(s.holds(7, TWO).iter().all(|n| *n >= 1), "the new value's own chunks were erased: {:?}", s.holds(7, TWO));
    s.end()
}

#[test]
fn plain_value_leaves_no_old_chunk() -> Outcome<()> {
    let _lock = lock();
    let s = res!(store("plain"));
    let k = dat!("shrinks to plain");
    res!(s.five(&k, 5));
    res!(s.put(&k, 8, PLAIN));
    match res!(s.db.api().fetch_using_schemes(&k, None)).recv_daticle(s.db.api().schemes().encrypter(), None) {
        Ok((Some((Dat::Tup5u64(_), _)), _)) => return Err(err!("The plain value is still chunked."; Test, Invalid)),
        _ => (),
    }
    res!(s.compact_and_check(&k, 5, 8, PLAIN));
    s.end()
}

// The same length reuses every chunk key, which the ordinary supersession retires: the retiring of
// a surplus must keep keys the new value holds, or it erases the value it has just written.
#[test]
fn same_geometry_keeps_the_new_chunks() -> Outcome<()> {
    let _lock = lock();
    let s = res!(store("same"));
    let k = dat!("same size");
    res!(s.five(&k, 5));
    res!(s.put(&k, 6, FIVE));
    res!(s.compact_and_check(&k, 5, 6, FIVE));
    assert!(s.holds(6, FIVE).iter().all(|n| *n >= 1), "the new value's own chunks were erased: {:?}", s.holds(6, FIVE));
    s.end()
}

// The new value is durable before the old chunks go: a reader that comes while the old chunks are
// being retired finds the old value or the new one, never a bunch key with chunks missing.
#[test]
fn reader_during_the_retire_finds_a_whole_value() -> Outcome<()> {
    let _lock = lock();
    let s = res!(store("reader"));
    let k = dat!("read during retire");
    res!(s.five(&k, 5));
    let old = Dat::BU32(fill_of(5, FIVE));
    let new = Dat::BU32(fill_of(9, TWO));

    let done = Arc::new(AtomicBool::new(false));
    let seen = {
        let (db, k, old, new, done) = (s.db.clone(), k.clone(), old.clone(), new.clone(), done.clone());
        thread::spawn(move || {
            let (mut n_old, mut n_new, mut bad) = (0usize, 0usize, 0usize);
            while !done.load(Ordering::Relaxed) {
                match db.get(&k, None) {
                    Ok(Some((v, _))) if v == old => n_old += 1,
                    Ok(Some((v, _))) if v == new => n_new += 1,
                    other => {
                        bad += 1;
                        let s: String = fmt!("{:?}", other).chars().take(200).collect();
                        msg!("a reader found neither value: {}", s);
                    },
                }
                thread::sleep(Duration::from_millis(15));
            }
            (n_old, n_new, bad)
        })
    };
    hooks::set_chunk_tombstone_delay(Duration::from_millis(400));
    let put = s.put(&k, 9, TWO);
    hooks::set_chunk_tombstone_delay(Duration::ZERO);
    done.store(true, Ordering::Relaxed);
    let (n_old, n_new, bad) = match seen.join() {
        Ok(t) => t,
        Err(_) => return Err(err!("The reader thread panicked."; Test, Invalid)),
    };
    res!(put);
    msg!("reader saw the old value {} times, the new {} times, {} broken.", n_old, n_new, bad);
    assert_eq!(bad, 0, "a reader found a value that was neither the old one nor the new one");
    assert!(n_new >= 1, "the reader never saw the new value, so it did not read while the old chunks were retired");
    res!(s.compact_and_check(&k, 5, 9, TWO));
    s.end()
}
