//! A chunked value's bunch key is published after its chunks (A3 round 2a, 2026-10-09).  The
//! records of a store go to writers that make them readable one at a time, each when its own disk
//! barrier completes, so a bunch key sent beside its chunks was readable for as long as the
//! slowest chunk's fsync took: a reader found a key whose chunks were "not found".  The chunks go
//! first now and the bunch key follows once every chunk is written and readable.  The hooks used
//! are process-wide, so every test takes one lock and puts each hook back.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::{
    channels::Recv,
    prelude::*,
};
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    base::cfg::OzoneConfig,
    comm::msg::OzoneMsg,
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

fn cfg(cbots: u16) -> Outcome<OzoneConfig> {
    let mut c = res!(setup::default_cfg());
    c.num_zones             = 1;
    c.num_cbots_per_zone    = cbots; // so that a bunch key often has a cache bot to itself
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
    open_with(dir, 8)
}

fn open_with(dir: &str, cbots: u16) -> Outcome<TestDb> {
    log_set_level!("error");
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());
    setup::start_db(root, Some(res!(cfg(cbots))), schemes(), None, false, true)
}

// A reader that comes while the chunks of a new value are slow to become readable finds no value
// yet, or the whole one, and never a bunch key over chunks that are not there.  A cache bot
// enters its records in turn, so a bunch key queued behind a held chunk at the same cache bot is
// held with it; several values are stored over many cache bots so that some bunch key has one to
// itself.  They are stored in turn here; `concurrent_chunked_puts_do_not_queue` stores them at once.
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

// A caller is told its records are written without waiting for the chunks to become readable.  The
// answers of a write are counted against deadlines of silence, 6 s for each record to be written
// and far longer for the disk to make them durable.  The chunks' answers, and the bunch key's
// that waits behind them, were first reported only once the chunks were readable, so a disk
// slower than 6 s (an fsync on a loaded machine) failed a put with "0 of 8 records were confirmed
// written" although nothing was wrong (A3 round 2a, `chunk_leak` under load).
#[test]
fn chunks_are_reported_written_while_they_are_held() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_chunk_publish_told";
    let db = res!(open(dir));
    let k = dat!("chunks told early");
    let hold = Duration::from_millis(2_500);

    hooks::set_chunk_insert_delay(hold);
    let begun = Instant::now();
    let resp = res!(db.api().put(k.clone(), big(5), Uid::default(), None));
    let chan = match resp.channel() {
        Some(chan) => chan,
        None => return Err(err!("A put returned a responder without a channel."; Test, Missing)),
    };
    let (mut records, mut written, mut told) = (0usize, 0usize, None);
    while told.is_none() && begun.elapsed() < Duration::from_secs(10) {
        match chan.recv_timeout(Duration::from_millis(50)) {
            Recv::Result(Ok(OzoneMsg::Chunks(n))) => records = n,
            Recv::Result(Ok(OzoneMsg::Written)) => {
                written += 1;
                if records > 0 && written == records { // every record, the bunch key included
                    told = Some(begun.elapsed());
                }
            },
            _ => (),
        }
    }
    hooks::set_chunk_insert_delay(Duration::ZERO);
    let told = match told {
        Some(t) => t,
        None => return Err(err!(
            "The caller of a put of {} records was told {} written in 10 s.", records, written;
            Test, Missing)),
    };
    msg!("the caller was told {} records written after {:?}; the hook holds each chunk for {:?}.",
        written, told, hold);
    // The chunks are held for 2.5 s each, so a caller told only when they are readable hears
    // nothing for at least that long.
    assert!(told < hold, "the caller was told its records were written only after {:?}, when the \
        chunks became readable", told);
    // The put goes on to complete.
    let end = Instant::now();
    loop {
        match res!(db.get(&k, None)) {
            Some((v, _)) => { assert_eq!(v, big(5), "the stored value reads back changed"); break; },
            None if end.elapsed() < Duration::from_secs(30) => thread::sleep(Duration::from_millis(50)),
            None => return Err(err!("The put never completed."; Test, Missing)),
        }
    }
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

// Chunked puts do not queue behind one another for the disk.  A server bot that waited through
// its put's chunks held one put each, so twelve at once against the two server bots made the
// last of a queue wait, with chunks held for 1.5 s, past the 6 s a request is given to be
// counted (A3 round 2a).  Twelve over two bots leave one with six, whatever the choice: at
// 1.5 s a put, the last would be reached after 7.5 s.
#[test]
fn concurrent_chunked_puts_do_not_queue() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_chunk_publish_many";
    let db = res!(open_with(dir, 32));
    let nputs = 12usize;
    let hold = Duration::from_millis(1_500);

    hooks::set_chunk_insert_delay(hold);
    let begun = Instant::now();
    let workers: Vec<_> = (0..nputs).map(|i| {
        let db = db.clone();
        thread::spawn(move || {
            db.insert(dat!(fmt!("many {}", i)), big(i as u8), Uid::default(), None)
                .map(|_| ()).map_err(|e| fmt!("{}", e))
        })
    }).collect();
    let mut failed = Vec::new();
    for (i, w) in workers.into_iter().enumerate() {
        match w.join() {
            Ok(Ok(())) => (),
            Ok(Err(e)) => failed.push((i, e.chars().take(200).collect::<String>())),
            Err(_) => failed.push((i, fmt!("the thread panicked"))),
        }
    }
    let took = begun.elapsed();
    hooks::set_chunk_insert_delay(Duration::ZERO);
    msg!("{} chunked puts at once took {:?}; {} failed.", nputs, took, failed.len());
    assert!(failed.is_empty(), "concurrent chunked puts failed: {:?}", failed);
    for i in 0..nputs {
        match res!(db.get(&dat!(fmt!("many {}", i)), None)) {
            Some((v, _)) => assert_eq!(v, big(i as u8), "a stored value reads back changed"),
            None => return Err(err!("Chunked put {} reported success but its value is gone.", i;
                Test, Missing)),
        }
    }
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}
