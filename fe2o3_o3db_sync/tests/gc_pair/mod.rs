//! What the collection tests share (D-C, 2026-10-01): a one-zone store whose data files hold five
//! records of one length each.  Superseding two of file 1's records then crosses the collection
//! trigger, and the collected file is exactly three records long, so the two generations of its
//! pair differ in size and can be told apart from outside.

#![allow(dead_code)]

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    O3db,
    base::{
        cfg::OzoneConfig,
        constant,
    },
    data::core::RestSchemesInput,
    file::{
        core::FileType,
        zdir::ZoneDir,
    },
    test::setup::{
        self,
        Uid,
        UID_LEN,
    },
};

use std::{
    collections::{
        BTreeMap,
        BTreeSet,
    },
    fs,
    path::{
        Path,
        PathBuf,
    },
    thread,
    time::{
        Duration,
        Instant,
    },
};

pub type TestDb = O3db<
    { UID_LEN },
    Uid,
    (),
    HashScheme,
    HashScheme,
    ChecksumScheme,
>;

pub const NKEYS: usize = 12;

pub fn config(max: u64) -> Outcome<OzoneConfig> {
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

pub fn schemes() -> RestSchemesInput<(), HashScheme, HashScheme, ChecksumScheme> {
    RestSchemesInput::new(
        None::<()>,
        None::<HashScheme>,
        None::<HashScheme>,
        Some(ChecksumScheme::new_crc32()),
    )
}

pub fn key(i: usize) -> Dat { dat!(fmt!("gc pair key {:03}", i)) }

/// Every value is 100 bytes and carries its key's number and version, so a value read back names
/// the record it came from.
pub fn value(i: usize, ver: u8) -> Dat {
    let mut v = vec![0u8; 100];
    v[0] = i as u8;
    v[1] = ver;
    for j in 2..100 {
        v[j] = (i as u8) ^ ver ^ (j as u8);
    }
    Dat::BU32(v)
}

pub fn size(p: &Path) -> u64 {
    match fs::metadata(p) { Ok(m) => m.len(), Err(_) => 0 }
}

pub fn file(root: &Path, cfg: &OzoneConfig, typ: FileType, n: u32) -> PathBuf {
    cfg.zone_root(root).join("zone_001").join(ZoneDir::relative_file_path(&typ, n))
}

pub fn zone_dir(root: &Path, cfg: &OzoneConfig) -> PathBuf {
    cfg.zone_root(root).join("zone_001")
}

/// What the files of a store hold records of this test's size to.  Index entries differ in length
/// with the offset they name, so the length of an index is not its record count times one number.
pub struct Probe {
    pub len:    u64,        // data bytes in one record
    pub ind:    Vec<u64>,   // index bytes for 0 to 5 records
}

/// Measures the records by writing five to a store of their own.
pub fn probe(probe_dir: &str) -> Outcome<Probe> {
    let _ = fs::remove_dir_all(probe_dir);
    res!(fs::create_dir_all(probe_dir));
    let root = res!(Path::new(probe_dir).canonicalize());
    let cfg = res!(config(4_000));
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, false, true));
    let (d1, i1) = (file(&root, &cfg, FileType::Data, 1), file(&root, &cfg, FileType::Index, 1));
    let mut ind = vec![0u64];
    for i in 0..5 {
        res!(db.insert(key(i), value(i, 1), Uid::default(), None));
        ind.push(size(&i1));
    }
    res!(db.close());
    let len = size(&d1) / 5;
    let _ = fs::remove_dir_all(probe_dir);
    assert!(len > 100, "the probe record is {} bytes, shorter than its own value", len);
    assert_eq!(size(&d1) % 5, 0, "the five probe records are not of one length");
    Ok(Probe { len, ind })
}

/// Writes every key at version 1.  File 1 ends up holding keys 0 to 4.
pub fn fill(db: &TestDb) -> Outcome<()> {
    for i in 0..NKEYS {
        res!(db.insert(key(i), value(i, 1), Uid::default(), None));
    }
    Ok(())
}

/// Reads every key and counts the answers that are not the version last written.
pub fn judge(db: &TestDb, want: &[u8], label: &str) -> usize {
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

/// Polls until the condition holds, for at most the limit.
pub fn wait_until<F: FnMut() -> bool>(limit: Duration, mut f: F) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if f() {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    f()
}

/// The temporary files of a collection that are in the directory.
pub fn temporaries(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    if let Ok(list) = fs::read_dir(dir) {
        for entry in list.flatten() {
            if ZoneDir::is_gc_temp_file(&entry.path()) {
                found.push(entry.file_name().to_string_lossy().to_string());
            }
        }
    }
    found
}

/// The bytes in every regular file under the zone root, counted by walking the directories with
/// nothing of the store's own, so that the store's answer has something outside it to be checked
/// against.
pub fn zone_files_len(root: &Path, cfg: &OzoneConfig) -> u64 {
    dir_len(&cfg.zone_root(root))
}

/// The bytes in every regular file under a directory, subdirectories included.
pub fn dir_len(dir: &Path) -> u64 {
    let mut bytes = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(list) = fs::read_dir(&d) {
            for entry in list.flatten() {
                match entry.metadata() {
                    Ok(m) if m.is_dir()     => stack.push(entry.path()),
                    Ok(m) if m.is_file()    => bytes += m.len(),
                    _ => (),
                }
            }
        }
    }
    bytes
}

/// Waits until the zone files have stopped changing, and gives their length then.
pub fn settled_len(root: &Path, cfg: &OzoneConfig) -> u64 {
    let start = Instant::now();
    let mut last = zone_files_len(root, cfg);
    let mut quiet = 0;
    while start.elapsed() < Duration::from_secs(30) {
        thread::sleep(Duration::from_millis(150));
        let now = zone_files_len(root, cfg);
        if now == last {
            quiet += 1;
            if quiet >= 4 {
                break;
            }
        } else {
            quiet = 0;
            last = now;
        }
    }
    last
}

/// The directories a store keeps files in: its root, which holds the zone root and everything
/// else the store writes, and the directory of each zone placed elsewhere by an override.
pub fn store_dirs(root: &Path, cfg: &OzoneConfig) -> Vec<PathBuf> {
    let mut dirs = vec![root.to_path_buf()];
    for zmap in cfg.zone_overrides().values() {
        let dir = match zmap {
            Dat::Map(m) => match m.get(&dat!("dir")) {
                Some(Dat::Str(s))   => s.clone(),
                _                   => String::new(),
            },
            _ => String::new(),
        };
        let base = if dir.is_empty() { root.to_path_buf() } else { root.join(dir) };
        dirs.push(cfg.zone_root(&base));
    }
    dirs
}

/// The files of the store, of whatever extension, that hold the needle anywhere in their bytes,
/// found by reading each whole file under the store root and every overridden zone directory with
/// nothing of the store's own.  Index files, configuration, temporaries and a zone placed outside
/// the root are all searched, so that a copy of a deleted value cannot hide in any of them.
pub fn files_holding(root: &Path, cfg: &OzoneConfig, needle: &[u8]) -> Vec<PathBuf> {
    let mut found = BTreeSet::new();
    let mut stack = store_dirs(root, cfg);
    let mut seen = BTreeSet::new();
    while let Some(d) = stack.pop() {
        if !seen.insert(d.clone()) {
            continue;
        }
        if let Ok(list) = fs::read_dir(&d) {
            for entry in list.flatten() {
                let p = entry.path();
                match entry.metadata() {
                    Ok(m) if m.is_dir() => stack.push(p),
                    Ok(m) if m.is_file() => {
                        if let Ok(bytes) = fs::read(&p) {
                            if needle.len() <= bytes.len() && bytes.windows(needle.len()).any(|w| w == needle) {
                                found.insert(p);
                            }
                        }
                    },
                    _ => (),
                }
            }
        }
    }
    found.into_iter().collect()
}
