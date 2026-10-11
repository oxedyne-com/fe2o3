//! A scan lists the keys with a live value, and a deleted key is not one of them.
//!
//! A delete writes a tombstone under the key, and the scan walks the index files, which name
//! every key's newest record without its value.  So the scan listed every deleted key as though
//! it were live, while every read of it came back empty: a pass that counted by the keys alone
//! miscounted (Daimond gateway, G2b session migration, 2026-10-12).  These tests delete keys and
//! scan, in a live store, after a restart, after a rebuild of the index from the data files, and
//! after a compaction.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_iop_db::api::{
    Database,
    ScanOpts,
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    base::cfg::OzoneConfig,
    data::core::RestSchemesInput,
    test::{
        file::delete_all_index_files,
        setup,
    },
};

use std::{
    collections::BTreeSet,
    path::{
        Path,
        PathBuf,
    },
    time::Duration,
};

const N: usize = 24;

type TestDb = oxedyne_fe2o3_o3db_sync::O3db<
    { setup::UID_LEN },
    setup::Uid,
    EncryptionScheme,
    HashScheme,
    HashScheme,
    ChecksumScheme,
>;

// Unencrypted, so that short values are as short as the tombstone itself.
fn schemes() -> RestSchemesInput<EncryptionScheme, HashScheme, HashScheme, ChecksumScheme> {
    RestSchemesInput::new(
        None::<EncryptionScheme>,
        None::<HashScheme>,
        None::<HashScheme>,
        Some(ChecksumScheme::new_crc32()),
    )
}

fn root(name: &str) -> Outcome<PathBuf> {
    let dir = fmt!("./test_db_scan_after_delete_{}", name);
    let _ = std::fs::remove_dir_all(&dir);
    res!(std::fs::create_dir_all(&dir));
    Ok(res!(Path::new(&dir).canonicalize()))
}

fn config(file_max: u64) -> Outcome<OzoneConfig> {
    let mut cfg = res!(setup::default_cfg());
    cfg.sync_on_write       = true;
    cfg.data_file_max_bytes = file_max;
    // Every zone inside this test's own root.
    cfg.zone_overrides      = DaticleMap::new();
    Ok(cfg)
}

fn key(i: usize) -> String { fmt!("k:{:03}", i) }

// Strings of every length from empty to eleven, so that some live value is stored with exactly
// the tombstone's length, and every fifth value large enough to be chunked.
fn val(i: usize) -> Dat {
    if i % 5 == 4 {
        dat!("c".repeat(2_000 + i))
    } else {
        dat!("v".repeat(i % 12))
    }
}

// Every third key, chunked ones among them.
fn doomed(i: usize) -> bool { i % 3 == 1 }

fn fill(db: &TestDb) -> Outcome<BTreeSet<String>> {
    let user = setup::Uid::default();
    for i in 0..N {
        res!(db.insert(dat!(key(i)), val(i), user, None));
    }
    let mut live = BTreeSet::new();
    for i in 0..N {
        if doomed(i) {
            let found = res!(db.delete(&dat!(key(i)), user, None));
            assert!(found, "the delete of {} found nothing", key(i));
        } else {
            live.insert(key(i));
        }
    }
    res!(db.api().settle_for_test(Duration::from_secs(10)));
    Ok(live)
}

// The string keys listed, and any other key, which can only be a retired chunk key.
fn scanned(db: &TestDb, opts: &ScanOpts) -> Outcome<(BTreeSet<String>, Vec<Dat>)> {
    let mut out = BTreeSet::new();
    let mut odd = Vec::new();
    for (k, _, _) in res!(db.scan(opts, None)) {
        match k {
            Dat::Str(s) => { out.insert(s); },
            other       => odd.push(other),
        }
    }
    Ok((out, odd))
}

fn check(db: &TestDb, live: &BTreeSet<String>, phase: &str) -> Outcome<()> {
    let (all, odd) = res!(scanned(db, &ScanOpts::all()));
    let extra: Vec<&String>   = all.difference(live).collect();
    let missing: Vec<&String> = live.difference(&all).collect();
    assert!(extra.is_empty() && missing.is_empty() && odd.is_empty(),
        "{}: the scan must list exactly the {} live keys; it listed {} deleted key(s) {:?}, \
        {} retired chunk key(s) {:?}, and left out {} live key(s) {:?}", phase, live.len(),
        extra.len(), extra, odd.len(), odd, missing.len(), missing);
    let (pre, _) = res!(scanned(db, &ScanOpts::with_str_prefix("k:".to_string())));
    assert_eq!(&pre, live, "{}: the prefix scan differs from the live keys", phase);
    // Every key listed reads, which is the contract a caller counting by keys relies on.
    for k in &all {
        assert!(res!(db.get(&dat!(k.clone()), None)).is_some(), "{}: {} was listed but reads absent", phase, k);
    }
    Ok(())
}

#[test]
fn scan_leaves_deleted_keys_out_live_and_after_restart() -> Outcome<()> {
    log_set_level!("error");
    let root = res!(root("restart"));
    let cfg = res!(config(1_000_000));

    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, true, true));
    let live = res!(fill(&db));
    res!(check(&db, &live, "live store"));
    let zdirs = res!(db.api().get_zone_dirs());
    res!(db.shutdown());

    // Restarted on the index files.
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, true, false));
    res!(check(&db, &live, "after restart"));
    res!(db.shutdown());

    // Restarted with the index rebuilt by replaying the data files.
    res!(delete_all_index_files(&zdirs));
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg), schemes(), None, true, false));
    res!(check(&db, &live, "after replay"));
    res!(db.shutdown());

    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

#[test]
fn scan_leaves_deleted_keys_out_after_compaction() -> Outcome<()> {
    log_set_level!("error");
    let root = res!(root("compact"));
    // Small files, so that the deleted records sit in sealed files a compaction collects.
    let cfg = res!(config(4_000));

    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, true, true));
    let live = res!(fill(&db));
    res!(check(&db, &live, "before compaction"));
    let report = res!(db.compact_now(Duration::from_secs(30)));
    msg!("{:?}", report);
    res!(check(&db, &live, "after compaction"));
    res!(db.shutdown());

    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg), schemes(), None, true, false));
    res!(check(&db, &live, "after compaction and restart"));
    res!(db.shutdown());

    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}
