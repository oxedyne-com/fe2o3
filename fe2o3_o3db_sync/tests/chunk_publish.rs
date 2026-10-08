//! A chunked value's bunch key is published after its chunks (A3 round 2a, 2026-10-09).  The
//! records of a store go to writers that make them readable one at a time, each when its own disk
//! barrier completes, so a bunch key sent beside its chunks was readable for as long as the
//! slowest chunk's fsync took: a reader found a key whose chunks were "not found".  The chunks go
//! first now and the bunch key follows once every chunk is written and readable.  The hooks used
//! are process-wide, so every test takes one lock and puts each hook back.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
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
    path::Path,
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
    time::{
        Duration,
        Instant,
    },
};

static HOOKS: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    match HOOKS.lock() { Ok(g) => g, Err(p) => p.into_inner() }
}

const HOLD:     Duration = Duration::from_millis(250); // a slow chunk
const VALUES:   usize = 6;                             // stored in turn

fn cfg() -> Outcome<OzoneConfig> {
    let mut c = res!(setup::default_cfg());
    c.num_zones             = 1;
    c.num_cbots_per_zone    = 8; // so that a bunch key often has a cache bot to itself
    c.num_fbots_per_zone    = 1;
    c.num_wbots_per_zone    = 2;
    c.zone_overrides        = BTreeMap::new();
    c.rest_chunk_threshold  = 1_000;
    c.rest_chunk_bytes      = 400;
    c.sync_on_write         = true;
    Ok(c)
}

fn big(seed: u8) -> Dat {
    Dat::BU32((0..1_100usize).map(|j| seed ^ (j as u8)).collect()) // three chunks
}

fn open(dir: &str) -> Outcome<TestDb> {
    log_set_level!("error");
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());
    setup::start_db(root, Some(res!(cfg())), schemes(), None, false, true)
}

// A reader that comes while the chunks of a new value are slow to become readable finds no value
// yet, or the whole one, and never a bunch key over chunks that are not there.  A cache bot
// enters its records in turn, so a bunch key queued behind a held chunk at the same cache bot is
// held with it; several values are stored over many cache bots so that some bunch key has one to
// itself.  They are stored in turn: a server bot waits for the chunks of the store it is at, and
// a queue of held stores would run past a request's deadline.
#[test]
fn bunch_key_is_not_readable_before_its_chunks() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_chunk_publish_order";
    let db = res!(open(dir));
    let keys: Vec<Dat> = (0..VALUES).map(|i| dat!(fmt!("chunks first {}", i))).collect();

    let done = Arc::new(AtomicBool::new(false));
    let seen = {
        let (db, keys, done) = (db.clone(), keys.clone(), done.clone());
        thread::spawn(move || {
            let (mut n_none, mut n_whole, mut bad) = (0usize, 0usize, 0usize);
            while !done.load(Ordering::Relaxed) {
                for (i, k) in keys.iter().enumerate() {
                    match db.get(k, None) {
                        Ok(None) => n_none += 1,
                        Ok(Some((v, _))) if v == big(i as u8) => n_whole += 1,
                        other => {
                            bad += 1;
                            let s: String = fmt!("{:?}", other).chars().take(300).collect();
                            msg!("a reader found neither no value nor the whole one: {}", s);
                        },
                    }
                }
                thread::sleep(Duration::from_millis(2));
            }
            (n_none, n_whole, bad)
        })
    };
    hooks::set_chunk_insert_delay(HOLD);
    let begun = Instant::now();
    let mut put = Ok(());
    for (i, k) in keys.iter().enumerate() {
        if let Err(e) = db.insert(k.clone(), big(i as u8), Uid::default(), None) {
            put = Err(e);
        }
    }
    let took = begun.elapsed();
    hooks::set_chunk_insert_delay(Duration::ZERO);
    done.store(true, Ordering::Relaxed);
    let (n_none, n_whole, bad) = match seen.join() {
        Ok(t) => t,
        Err(_) => return Err(err!("The reader thread panicked."; Test, Invalid)),
    };
    res!(put);
    msg!("reader saw no value {} times, a whole one {} times, {} broken; the stores took {:?}.",
        n_none, n_whole, bad, took);
    // Positive control: the chunks were held, so a bunch key sent beside them had a window.
    assert!(took >= HOLD, "the hook did not hold the chunks: the stores took {:?}", took);
    assert_eq!(bad, 0, "a reader found a bunch key whose chunks were not readable");
    for (i, k) in keys.iter().enumerate() {
        match res!(db.get(k, None)) {
            Some((v, _)) => assert_eq!(v, big(i as u8), "the stored value reads back changed"),
            None => return Err(err!("A stored value is gone."; Test, Missing)),
        }
    }
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

// Chunks that are not confirmed durable are not published: the caller is told the write failed,
// untagged `Unconfirmed` because nothing of the value is readable, and the key holds no value
// rather than a bunch key over chunks of doubtful standing.
#[test]
fn unconfirmed_chunks_publish_no_bunch_key() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_chunk_publish_failed";
    let db = res!(open(dir));
    let k = dat!("chunks that fail");
    // A value beside it that stays, so the check is of this key and not of a dead store.
    res!(db.insert(key(0), value(0, 9), Uid::default(), None));

    hooks::set_barrier_failure(true);
    let told = db.insert(k.clone(), big(3), Uid::default(), None);
    hooks::set_barrier_failure(false);
    match told {
        Ok(_) => return Err(err!(
            "A chunked store whose chunks failed their barrier was confirmed.";
            Test, Unexpected)),
        Err(e) => {
            if e.tags().contains(&ErrTag::Unconfirmed) {
                return Err(err!(
                    "A chunked store that published nothing was told it was written but \
                    unconfirmed: {}", e;
                    Test, Mismatch));
            }
        },
    }
    match db.get(&k, None) {
        Ok(None) => (),
        other => return Err(err!(
            "A chunked store whose chunks failed left {:?} at its key, where it should hold no \
            value.", other;
            Test, Mismatch)),
    }
    match res!(db.get(&key(0), None)) {
        Some((v, _)) => assert_eq!(v, value(0, 9), "the value beside it changed"),
        None => return Err(err!("The value beside it is gone."; Test, Missing)),
    }
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}
