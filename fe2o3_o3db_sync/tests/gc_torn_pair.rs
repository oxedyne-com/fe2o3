//! A collection whose cache bot is slow to answer must still leave its data and index files a
//! matching pair.  Until 2026-09-24 the collector renamed the rebuilt index over the old one,
//! waited five seconds for the cache, gave up, and never renamed the data file: the next start read
//! the new index beside the old data file and filled the cache with offsets from the one for the
//! other.  Before o3i that returned other keys' values for a whole session, and after it the same
//! reads failed.  Found by QA of lane o3i (D-C, 2026-09-24).  `test::hooks` holds the one cache
//! bot and the collection, and the hooks are process-wide, which is why this is a test binary of
//! its own with a single test.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    O3db,
    base::cfg::OzoneConfig,
    data::core::RestSchemesInput,
    file::{
        core::FileType,
        zdir::ZoneDir,
    },
    test::{
        hooks,
        setup::{
            self,
            Uid,
            UID_LEN,
        },
    },
};

use std::{
    collections::BTreeMap,
    path::{
        Path,
        PathBuf,
    },
    thread,
    time::Duration,
};

type TestDb = O3db<
    { UID_LEN },
    Uid,
    (),
    HashScheme,
    HashScheme,
    ChecksumScheme,
>;

const NKEYS: usize = 12;

fn config(max: u64) -> Outcome<OzoneConfig> {
    let mut cfg = res!(setup::default_cfg());
    cfg.num_zones               = 1;
    cfg.num_cbots_per_zone      = 1;
    cfg.num_fbots_per_zone      = 1;
    cfg.num_igbots_per_zone     = 1;
    cfg.num_rbots_per_zone      = 1;
    cfg.num_wbots_per_zone      = 1;
    cfg.data_file_max_bytes     = max;
    cfg.rest_chunk_threshold    = max * 7 / 10;
    cfg.zone_overrides          = BTreeMap::new();
    cfg.sync_on_write           = true;
    Ok(cfg)
}

fn schemes() -> RestSchemesInput<(), HashScheme, HashScheme, ChecksumScheme> {
    RestSchemesInput::new(
        None::<()>,
        None::<HashScheme>,
        None::<HashScheme>,
        Some(ChecksumScheme::new_crc32()),
    )
}

fn key(i: usize) -> Dat { dat!(fmt!("torn pair key {:03}", i)) }

/// Every value is 100 bytes and carries its key's number and version, so a value read back names
/// the record it came from.
fn value(i: usize, ver: u8) -> Dat {
    let mut v = vec![0u8; 100];
    v[0] = i as u8;
    v[1] = ver;
    for j in 2..100 {
        v[j] = (i as u8) ^ ver ^ (j as u8);
    }
    Dat::BU32(v)
}

fn size(p: &Path) -> u64 {
    match std::fs::metadata(p) { Ok(m) => m.len(), Err(_) => 0 }
}

fn file(root: &Path, cfg: &OzoneConfig, typ: FileType, n: u32) -> PathBuf {
    cfg.zone_root(root).join("zone_001").join(ZoneDir::relative_file_path(&typ, n))
}

/// Reads every key and counts the answers that are not the version last written.
fn judge(db: &TestDb, want: &[u8], label: &str) -> usize {
    let mut bad = 0;
    for i in 0..NKEYS {
        match db.get(&key(i), None) {
            Ok(Some((v, _))) if v == value(i, want[i]) => (),
            Ok(Some((v, _))) => {
                bad += 1;
                let head = match v.bytes_ref() {
                    Some(b) if b.len() >= 2 => fmt!("key {} version {}", b[0], b[1]),
                    _ => fmt!("{:?}", v),
                };
                msg!("{}: key {} read back {}, not version {}.", label, i, head, want[i]);
            },
            Ok(None) => {
                bad += 1;
                msg!("{}: key {} is missing.", label, i);
            },
            Err(e) => {
                bad += 1;
                let s: String = fmt!("{}", e).chars().take(300).collect();
                msg!("{}: key {} could not be read: {}", label, i, s);
            },
        }
    }
    msg!("{}: {} of {} keys wrong.", label, bad, NKEYS);
    bad
}

#[test]
fn main() -> Outcome<()> {
    log_set_level!("error");
    let dir = "./test_db_gc_torn_pair";
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());

    // The length of one record, all being one length.
    let pdir = "./test_db_gc_torn_pair_probe";
    let _ = std::fs::remove_dir_all(pdir);
    res!(std::fs::create_dir_all(pdir));
    let probe = res!(Path::new(pdir).canonicalize());
    let pcfg = res!(config(4_000));
    let pdb: TestDb = res!(setup::start_db(probe.clone(), Some(pcfg.clone()), schemes(), None, false, true));
    res!(pdb.insert(key(0), value(0, 1), Uid::default(), None));
    res!(pdb.close());
    let len = size(&file(&probe, &pcfg, FileType::Data, 1));
    assert!(len > 100, "the probe record is {} bytes, shorter than its own value", len);

    // Five records to a file, so that superseding two of file 1's crosses the collection trigger.
    let cfg = res!(config(5 * len + len / 2));
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, true, true));
    let mut want = vec![1u8; NKEYS];
    for i in 0..NKEYS {
        res!(db.insert(key(i), value(i, 1), Uid::default(), None));
    }
    let d1 = file(&root, &cfg, FileType::Data, 1);
    let full = size(&d1);
    assert_eq!(full, 5 * len, "file 1 does not hold the five records it was sized for");

    // The collection of file 1 starts a second after keys 0 and 1 are superseded, and its cache
    // update then waits behind an insert held eight seconds, past the five a collector used to
    // wait for it.
    hooks::set_collect_delay(Duration::from_millis(1_000));
    res!(db.insert(key(0), value(0, 2), Uid::default(), None));
    want[0] = 2;
    res!(db.insert(key(1), value(1, 2), Uid::default(), None));
    want[1] = 2;
    hooks::set_insert_delay(Duration::from_millis(8_000));
    let writing = db.clone();
    let slow = res!(thread::Builder::new().name(fmt!("slow insert")).spawn(move || {
        let _ = writing.insert(key(NKEYS - 1), value(NKEYS - 1, 3), Uid::default(), None);
    }));
    want[NKEYS - 1] = 3;
    thread::sleep(Duration::from_millis(9_000));
    hooks::set_insert_delay(Duration::ZERO);
    hooks::set_collect_delay(Duration::ZERO);
    let _ = slow.join();
    let collected = size(&d1);
    let same = judge(&db, &want, "Same process");
    res!(db.close());

    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, true, false));
    let reopened = judge(&db, &want, "After reopening");

    // Churn so that file 1 is collected again with whatever state the start built.
    for round in 4..7u8 {
        for i in 2..6 {
            res!(db.insert(key(i), value(i, round), Uid::default(), None));
            want[i] = round;
        }
    }
    thread::sleep(Duration::from_secs(3));
    let churned = judge(&db, &want, "After more collection");
    res!(db.close());

    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, true, false));
    let second = judge(&db, &want, "After a second reopening");
    res!(db.close());
    log_finish_wait!();

    assert_eq!(same, 0, "{} keys read wrong in the process that ran the collection", same);
    assert_eq!(reopened, 0, "{} keys read wrong after the first reopening", reopened);
    assert_eq!(churned, 0, "{} keys read wrong after file 1 was collected again", churned);
    assert_eq!(second, 0, "{} keys read wrong after the second reopening", second);
    // The collection has to have happened for any of this to mean anything.
    assert_eq!(collected, 3 * len,
        "file 1 should have lost the two superseded records to its collection, and is {} bytes \
        where it was {}", collected, full);
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(pdir);
    Ok(())
}
