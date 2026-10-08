//! A delete of a chunked value waits for the tombstones of its chunks and fails if one fails
//! (A3 R2, 2026-10-08).  The chunk tombstones were sent with no responder, so `delete` returned on
//! the bunch key's answer alone, with the chunks possibly unwritten and a failure only logged.
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

#[test]
fn chunk_tombstone_failure_fails_delete() -> Outcome<()> {
    log_set_level!("error");
    let _lock = match HOOKS.lock() { Ok(g) => g, Err(p) => p.into_inner() };
    let db = res!(open("./test_db_chunk_delete_fail"));
    let user = setup::Uid::default();
    let k = dat!("chunk delete fail");
    res!(db.insert(k.clone(), big(9), user, None));
    let cks = res!(chunk_keys(&db, &k));

    // The disk fails every sync, so no tombstone is confirmed.  The bunch key's would fail too, but
    // with words of its own: the error must name a chunk, which only the chunk wait can.
    hooks::set_barrier_failure(true);
    let result = db.delete(&k, user, None);
    hooks::set_barrier_failure(false);
    let e = match result {
        Err(e) => e,
        Ok(b) => return Err(err!("A delete whose chunk tombstones failed returned Ok({}).", b; Test, Invalid)),
    };
    let text = fmt!("{}", e);
    msg!("The delete failed with: {}", text.chars().take(400).collect::<String>());
    assert!(text.contains(&fmt!("chunk 1 of {}", cks.len())),
        "the error does not name the failed chunk: {}", text.chars().take(400).collect::<String>());
    res!(db.close());
    Ok(())
}
