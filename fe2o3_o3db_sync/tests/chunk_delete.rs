//! A delete of a chunked value waits for the tombstones of its chunks and fails if one fails
//! (A3 R2, 2026-10-08).  The chunk tombstones were sent with no responder, so `delete` returned on
//! the bunch key's answer alone, with the chunks possibly unwritten and a failure only logged.
//! Since fix A (2026-10-09) a delete is a store with no chunks: the chunks are retired only once
//! the head tombstone is durable, and a chunk tombstone that fails after that is left to the
//! orphan sweep rather than failing the delete.
//! The hooks used are process-wide, so the tests here take a lock and put each back.

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
        setup,
    },
};

use std::{
    collections::BTreeMap,
    path::Path,
    sync::Mutex,
    time::{
        Duration,
        Instant,
    },
};

static HOOKS: Mutex<()> = Mutex::new(());

const CHUNKS: u64 = 3;

fn cfg() -> Outcome<OzoneConfig> {
    let mut c = res!(setup::default_cfg());
    c.num_zones             = 1;
    c.num_cbots_per_zone    = 4;
    c.num_fbots_per_zone    = 1;
    c.num_wbots_per_zone    = 2;
    c.zone_overrides        = BTreeMap::new();
    c.rest_chunk_threshold  = 1_000;
    c.rest_chunk_bytes      = 400;
    c.sync_on_write         = true;
    Ok(c)
}

fn big(seed: u8) -> Dat {
    Dat::BU32((0..1_100usize).map(|j| seed ^ (j as u8)).collect())
}

fn open(dir: &str) -> Outcome<TestDb> {
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());
    setup::start_db(root, Some(res!(cfg())), schemes(), None, false, true)
}

/// The chunk keys of the value held under `k`, read from its bunch key.
fn chunk_keys(db: &TestDb, k: &Dat) -> Outcome<Vec<Dat>> {
    let resp = res!(db.api().fetch_using_schemes(k, None));
    let pkey = match res!(resp.recv_daticle(db.api().schemes().encrypter(), None)) {
        (Some((Dat::Tup5u64(tup), _)), _) => PartKey(tup),
        other => return Err(err!("Key {:?} holds {:?}, not a bunch key.", k, other; Test, Invalid)),
    };
    assert_eq!(pkey.num_parts(), CHUNKS, "the value did not split into {} chunks", CHUNKS);
    Ok((1..(pkey.num_parts() + 1)).map(|i| Dat::Tup5u64([
        pkey.set_id(), i, pkey.data_len(), pkey.num_parts(), pkey.part_size(),
    ])).collect())
}

/// Does the newest record at this chunk key retire the chunk?
fn retired(db: &TestDb, ck: &Dat) -> Outcome<bool> {
    let resp = res!(db.api().fetch_using_schemes(ck, None));
    let (found, _) = res!(resp.recv_daticle(db.api().schemes().encrypter(), None));
    Ok(found.is_none())
}

fn cbot_of(db: &TestDb, k: &Dat) -> Outcome<usize> {
    let (_, wind, _) = res!(db.api().ozone_key_dat(k, None));
    Ok(**wind.bpind())
}

#[test]
fn chunked_delete_waits_for_chunks() -> Outcome<()> {
    log_set_level!("error");
    let _lock = match HOOKS.lock() { Ok(g) => g, Err(p) => p.into_inner() };
    let db = res!(open("./test_db_chunk_delete_wait"));
    let user = setup::Uid::default();

    // A key whose bunch key shares no cache bot with any of its chunks, so that the bunch key's
    // answer is not queued behind the held chunk tombstones.
    let mut found = None;
    for n in 0..200 {
        let k = dat!(fmt!("chunk delete wait {:03}", n));
        res!(db.insert(k.clone(), big(n as u8), user, None));
        let cks = res!(chunk_keys(&db, &k));
        let bunch = res!(cbot_of(&db, &k));
        let mut apart = true;
        for ck in &cks {
            if res!(cbot_of(&db, ck)) == bunch { apart = false; }
        }
        if apart { found = Some((k, cks)); break; }
        res!(db.delete(&k, user, None));
    }
    let (k, cks) = match found {
        Some(f) => f,
        None => return Err(err!("No key kept its bunch key apart from its chunks."; Test, Missing)),
    };
    for ck in &cks {
        assert!(!res!(retired(&db, ck)), "positive control: chunk {:?} is not live before the delete", ck);
    }

    hooks::set_chunk_tombstone_delay(Duration::from_millis(300));
    let t0 = Instant::now();
    let existed = res!(db.delete(&k, user, None));
    let took = t0.elapsed();
    hooks::set_chunk_tombstone_delay(Duration::ZERO);
    assert!(existed, "the delete did not find the value");
    msg!("The delete returned after {:?}.", took);
    // Reads queue behind the held tombstones at their cache bot, so the read-back below holds
    // either way; the time the delete took is what tells a delete that waited from one that did not.
    assert!(took >= Duration::from_millis(300), "delete returned in {:?}, before a held chunk tombstone", took);
    for (i, ck) in cks.iter().enumerate() {
        assert!(res!(retired(&db, ck)), "chunk {} of the value is still live when delete returned", i + 1);
    }

    res!(db.close());
    Ok(())
}

// A delete whose head tombstone is not confirmed retires nothing: the value's chunks stay live,
// so a bunch key that did not land still reads whole.  Every sync fails, so no tombstone is
// confirmed; the old order sent the chunks' beside the head's and took them all.
#[test]
fn head_tombstone_failure_retires_nothing() -> Outcome<()> {
    log_set_level!("error");
    let _lock = match HOOKS.lock() { Ok(g) => g, Err(p) => p.into_inner() };
    let db = res!(open("./test_db_chunk_delete_fail"));
    let user = setup::Uid::default();
    let k = dat!("chunk delete fail");
    res!(db.insert(k.clone(), big(9), user, None));
    let cks = res!(chunk_keys(&db, &k));

    hooks::set_barrier_failure(true);
    let result = db.delete(&k, user, None);
    hooks::set_barrier_failure(false);
    if let Ok(b) = result {
        return Err(err!("A delete whose head tombstone failed returned Ok({}).", b; Test, Invalid));
    }
    for (i, ck) in cks.iter().enumerate() {
        assert!(!res!(retired(&db, ck)), "chunk {} was retired although the head tombstone failed", i + 1);
    }
    res!(db.close());
    Ok(())
}

// A delete removes what existed when it began.  Its tombstones are stamped once, at the start:
// stamped as each is sent, an overwrite of the same length begun while the chunk tombstones are
// becoming durable has chunks newer than theirs and a bunch key older than the bunch tombstone,
// so the delete takes the value and leaves its chunks live for the orphan sweep (QA A2-6).
#[test]
fn delete_does_not_erase_a_later_overwrite() -> Outcome<()> {
    log_set_level!("error");
    let _lock = match HOOKS.lock() { Ok(g) => g, Err(p) => p.into_inner() };
    let db = res!(open("./test_db_chunk_delete_race"));
    let user = setup::Uid::default();
    let k = dat!("chunk delete race");
    res!(db.insert(k.clone(), big(1), user, None));
    let _ = res!(chunk_keys(&db, &k));

    hooks::set_barrier_delay(Duration::from_millis(1_000));
    let del = {
        let (db, k) = (db.clone(), k.clone());
        std::thread::spawn(move || db.delete(&k, user, None))
    };
    std::thread::sleep(Duration::from_millis(300));
    let put = {
        let (db, k) = (db.clone(), k.clone());
        std::thread::spawn(move || db.insert(k, big(2), user, None).map(|_| ()))
    };
    let del = del.join();
    let put = put.join();
    hooks::set_barrier_delay(Duration::ZERO);
    match (del, put) {
        (Ok(a), Ok(b)) => { res!(a); res!(b); },
        _ => return Err(err!("A thread panicked."; Test, Invalid)),
    }
    match db.get(&k, None) {
        Ok(Some((v, _))) => assert_eq!(v, big(2), "the overwrite reads back changed"),
        other => {
            let t: String = fmt!("{:?}", other).chars().take(300).collect();
            panic!("a value begun after the delete began does not read back whole: {}", t);
        },
    }
    res!(db.close());
    Ok(())
}

// A chunked delete sends its head tombstone, waits for it to be durable, and only then sends the
// tombstones of the chunks: sent together, the chunks' could land first and a reader find a chunk
// gone under a current bunch key (fix A).  Counted from the order of the steps, not timed.
#[test]
fn chunked_delete_retires_after_its_head_is_durable() -> Outcome<()> {
    log_set_level!("error");
    let _lock = match HOOKS.lock() { Ok(g) => g, Err(p) => p.into_inner() };
    let db = res!(open("./test_db_chunk_delete_round"));
    let user = setup::Uid::default();
    let k = dat!("chunk delete round");
    res!(db.insert(k.clone(), big(3), user, None));
    let cks = res!(chunk_keys(&db, &k));

    hooks::set_trace(true);
    let existed = res!(db.delete(&k, user, None));
    let steps = hooks::trace_steps();
    hooks::set_trace(false);
    assert!(existed, "the delete did not find the value");
    let sent = steps.iter().filter(|s| **s == hooks::Step::Sent).count();
    assert_eq!(sent, cks.len() + 1, "expected a tombstone for each chunk and the bunch key: {:?}", steps);
    let mut want = vec![hooks::Step::Sent, hooks::Step::Durable];
    want.extend(cks.iter().map(|_| hooks::Step::Sent));
    assert_eq!(steps, want, "the chunks were not retired after the head tombstone was durable");
    for ck in &cks {
        assert!(res!(retired(&db, ck)), "chunk {:?} is still live when delete returned", ck);
    }
    res!(db.close());
    Ok(())
}
