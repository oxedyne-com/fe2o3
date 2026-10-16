//! A presence query reads no value.
//!
//! Asking whether a store holds a key was answered by reading the value, decrypting and decoding
//! it, and throwing it away.  The Daimond gateway's chunk presence query asked that of 2,000
//! values of 256 KB at a time, half a gigabyte read and decrypted to answer "which of these do you
//! hold?", and on a one-CPU host the phone gave up after 63 s (2026-10-16).  The zone caches hold
//! every key's record in memory, and a tombstone has one length, so the index answers.
//!
//! This store is the gateway's: 2,000 encrypted values of 256 KB, under a 100 MB cache, so most
//! values are not cached and a read goes to disk.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::{
    prelude::*,
    rand::Rand,
};
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    O3db,
    base::constant,
    comm::response::Wait,
    data::core::RestSchemesInput,
    test::{
        hooks,
        setup,
    },
};

use std::{
    path::PathBuf,
    sync::Arc,
    thread,
    time::{
        Duration,
        Instant,
    },
};


type Db = O3db<{ setup::UID_LEN }, setup::Uid, EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>;

const N:            usize       = 2_000;        // the gateway's HAVE_QUERY_BATCH
const VAL_BYTES:    usize       = 256 * 1024;   // one Daimond chunk
const DELETED:      usize       = 50;           // of the N, deleted after writing
const ABSENT:       usize       = 200;          // never written
const QUERIES:      usize       = 8;            // concurrent queries, as the phone's retries stacked
// Measured on argonaut 2026-10-16: 8 concurrent 2,000-key queries of the index took 8.9 ms in
// all, 33 ms pinned to one CPU, and one query of the old path, reading 524 MB of values, took
// 218-286 s.  Fifteen times the index's one-CPU figure.
const BOUND:        Duration    = Duration::from_millis(500);

fn key(i: usize) -> Dat {
    dat!(fmt!("chunk:{:05}", i))
}

fn wait() -> Wait {
    Wait {
        max_wait:       Duration::from_secs(30),
        check_interval: constant::CHECK_INTERVAL,
    }
}

fn open(root: &PathBuf, enc: &EncryptionScheme, wipe: bool) -> Outcome<Db> {
    let schms_input = RestSchemesInput::new(
        Some(enc.clone()),
        None::<HashScheme>,
        None::<HashScheme>,
        Some(ChecksumScheme::new_crc32()),
    );
    let mut cfg = res!(setup::default_cfg());
    cfg.zone_overrides          = DaticleMap::new();
    cfg.cache_size_limit_bytes  = 100_000_000;      // as the live gateway's
    cfg.data_file_max_bytes     = 64_000_000;
    cfg.rest_chunk_threshold    = 716_800;          // so a chunk is one record, as live
    cfg.rest_chunk_bytes        = 102_400;
    setup::start_db(root.clone(), Some(cfg), schms_input, None, false, wipe)
}

#[test]
fn presence_reads_no_value() -> Outcome<()> {
    log_set_level!("warn");
    let outcome = run();
    log_finish_wait!();
    outcome
}

fn run() -> Outcome<()> {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("test_db_presence");
    let _ = std::fs::remove_dir_all(&root);
    res!(std::fs::create_dir_all(&root));
    let root = res!(root.canonicalize());

    let mut enckey = [0u8; 32];
    Rand::fill_u8(&mut enckey);
    let aes = res!(EncryptionScheme::new_aes_256_gcm_with_key(&enckey[..]));

    let user = setup::Uid::default();
    let mut val = vec![0u8; VAL_BYTES];
    {
        let db = res!(open(&root, &aes, true));
        let t = Instant::now();
        for i in 0..N {
            val[..8].copy_from_slice(&(i as u64).to_le_bytes());
            res!(db.insert(key(i), Dat::BU32(val.clone()), user, None));
        }
        println!("wrote {} values of {} bytes in {:?}", N, VAL_BYTES, t.elapsed());
        res!(db.close());
    }

    // Reopened, so the cache holds what a restarted gateway's does: the index, and no values.
    let db = Arc::new(res!(open(&root, &aes, false)));
    let all: Vec<Dat> = (0..N).map(key).collect();

    // (a) No value is read to answer for present keys.
    let before = hooks::record_bytes_read();
    let t = Instant::now();
    let held = res!(db.contains_many(&all, None));
    let took = t.elapsed();
    let read = hooks::record_bytes_read() - before;
    println!("(a) presence of {} present keys: {} bytes read from data files, {:?}", N, read, took);
    if held.iter().any(|h| !h) {
        return Err(err!("{} of {} present keys were reported absent.",
            held.iter().filter(|h| !**h).count(), N; Test, Missing));
    }
    if read != 0 {
        return Err(err!(
            "Answering whether {} present keys are held read {} bytes of records from the data \
            files, about {} values of {} bytes; the index alone holds the answer.",
            N, read, read / VAL_BYTES as u64, VAL_BYTES;
            Test, Excessive));
    }

    // (b) Eight concurrent queries of all 2,000 finish inside the bound.
    let t = Instant::now();
    let mut handles = Vec::with_capacity(QUERIES);
    for _ in 0..QUERIES {
        let db = db.clone();
        let all = all.clone();
        handles.push(thread::spawn(move || -> Outcome<(usize, Duration)> {
            let t = Instant::now();
            let held = res!(db.contains_many(&all, None));
            Ok((held.iter().filter(|h| **h).count(), t.elapsed()))
        }));
    }
    let mut slowest = Duration::ZERO;
    for h in handles {
        match h.join() {
            Ok(Ok((n, took))) => {
                if n != N {
                    return Err(err!("A concurrent query found {} of {} keys.", n, N; Test, Missing));
                }
                slowest = std::cmp::max(slowest, took);
            },
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err(err!("A query thread panicked."; Test, Thread)),
        }
    }
    let all_took = t.elapsed();
    println!("(b) {} concurrent queries of {} keys: slowest {:?}, all {:?}", QUERIES, N, slowest, all_took);
    if all_took > BOUND {
        return Err(err!(
            "{} concurrent presence queries of {} keys took {:?}, over the {:?} bound.",
            QUERIES, N, all_took, BOUND;
            Test, Excessive));
    }

    // The old path, for the record: `get` on every key, which is what `has` did.
    let before = hooks::record_bytes_read();
    let t = Instant::now();
    let mut found = 0;
    for k in &all {
        if res!(db.get(k, None)).is_some() {
            found += 1;
        }
    }
    println!("old path, get().is_some() on {} keys: found {}, {} bytes read, {:?}",
        N, found, hooks::record_bytes_read() - before, t.elapsed());

    // (c) A deleted key is absent, its tombstone cached and, after the cache is cleared, not.
    for i in 0..DELETED {
        res!(db.delete(&key(i), user, None));
    }
    let gone: Vec<Dat> = (0..DELETED).map(key).collect();
    let held = res!(db.contains_many(&gone, None));
    if let Some(i) = held.iter().position(|h| *h) {
        return Err(err!("The deleted key {:?} is reported held, its tombstone cached.", key(i);
            Test, Mismatch));
    }
    res!(db.api().clear_cache_values(wait()));
    let before = hooks::record_bytes_read();
    let held = res!(db.contains_many(&gone, None));
    let read = hooks::record_bytes_read() - before;
    if let Some(i) = held.iter().position(|h| *h) {
        return Err(err!("The deleted key {:?} is reported held, its tombstone uncached.", key(i);
            Test, Mismatch));
    }
    println!("(c) {} deleted keys absent; uncached, their tombstones cost {} bytes read", DELETED, read);
    if read > (DELETED * 64) as u64 {
        return Err(err!("Reading {} uncached tombstones took {} bytes.", DELETED, read;
            Test, Excessive));
    }
    // Only the deleted ones: their neighbours stay held.
    let held = res!(db.contains_many(&all, None));
    let n = held.iter().filter(|h| **h).count();
    if n != N - DELETED || held[..DELETED].iter().any(|h| *h) {
        return Err(err!("After {} deletes, {} of {} keys are held.", DELETED, n, N; Test, Mismatch));
    }

    // (d) A key never written is absent.
    let never: Vec<Dat> = (N..N + ABSENT).map(key).collect();
    let held = res!(db.contains_many(&never, None));
    if held.iter().any(|h| *h) {
        return Err(err!("A key never written is reported held."; Test, Mismatch));
    }
    if res!(db.contains(&key(N + ABSENT), None)) {
        return Err(err!("A key never written is reported held by contains."; Test, Mismatch));
    }
    println!("(d) {} keys never written absent", ABSENT);

    res!(db.close());
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}
