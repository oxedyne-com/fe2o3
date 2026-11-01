//! A scan of a zone holds a bounded number of open files, whatever the number of files in it.
//!
//! The scan bot held every data file of a pass open until the pass ended, so that the tombstone
//! check after the walk could read each record from the generation its index named.  A zone with
//! more files than the process may hold open came up short: each index open past the limit
//! failed, the failure was taken for a collection that had removed the file, and the scan
//! refused with "Scan of this zone is short" on a store that was whole (Daimond restore
//! rehearsal under systemd's default soft limit of 1024 open files, 7,939 file pairs,
//! 2026-11-01).  The test runs itself again in a child whose soft limit is below the number of
//! files in the zone, since the limit is per process.
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
    test::setup,
};

use std::{
    collections::BTreeSet,
    env,
    path::{
        Path,
        PathBuf,
    },
    process::Command,
    time::Duration,
};

const N:            usize   = 400;      // Records, one to a data file.
const FD_LIMIT:     u32     = 160;      // The child's soft limit on open files, well below N.
const CHILD_VAR:    &str    = "O3DB_SCAN_FD_BOUND_CHILD";
const TEST_NAME:    &str    = "scan_of_a_zone_with_more_files_than_the_fd_limit_is_whole";

type TestDb = oxedyne_fe2o3_o3db_sync::O3db<
    { setup::UID_LEN },
    setup::Uid,
    EncryptionScheme,
    HashScheme,
    HashScheme,
    ChecksumScheme,
>;

fn schemes() -> RestSchemesInput<EncryptionScheme, HashScheme, HashScheme, ChecksumScheme> {
    RestSchemesInput::new(
        None::<EncryptionScheme>,
        None::<HashScheme>,
        None::<HashScheme>,
        Some(ChecksumScheme::new_crc32()),
    )
}

fn root() -> Outcome<PathBuf> {
    let dir = "./test_db_scan_fd_bound";
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    Ok(res!(Path::new(dir).canonicalize()))
}

fn config() -> Outcome<OzoneConfig> {
    let mut cfg = res!(setup::default_cfg());
    cfg.num_zones           = 1;
    cfg.sync_on_write       = true;
    // Smaller than two records, so that every record starts a data file of its own, with the
    // chunking threshold and chunk size inside the ratios the configuration allows.
    cfg.data_file_max_bytes     = 2_000;
    cfg.rest_chunk_threshold    = 1_500;
    cfg.rest_chunk_bytes        = 500;
    cfg.zone_overrides      = DaticleMap::new();
    Ok(cfg)
}

fn key(i: usize) -> String { fmt!("k:{:04}", i) }

fn scanned(db: &TestDb) -> Outcome<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    for (k, _, _) in res!(db.scan(&ScanOpts::all(), None)) {
        if let Dat::Str(s) = k {
            out.insert(s);
        }
    }
    Ok(out)
}

fn zone_data_files(db: &TestDb) -> Outcome<usize> {
    let mut n = 0;
    for (_, zdir) in res!(db.api().get_zone_dirs()) {
        for entry in res!(std::fs::read_dir(&zdir.dir)) {
            let path = res!(entry).path();
            if path.extension().and_then(|e| e.to_str()) == Some("dat") {
                n += 1;
            }
        }
    }
    Ok(n)
}

// The body, run in the child under the low limit.
fn scan_under_the_limit() -> Outcome<()> {
    log_set_level!("error");
    let root = res!(root());
    let cfg = res!(config());
    let user = setup::Uid::default();
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, true, true));
    let mut want = BTreeSet::new();
    for i in 0..N {
        res!(db.insert(dat!(key(i)), dat!("v".repeat(1_100)), user, None));
        want.insert(key(i));
    }
    res!(db.api().settle_for_test(Duration::from_secs(30)));
    let files = res!(zone_data_files(&db));
    assert!(files > FD_LIMIT as usize,
        "the zone must hold more data files ({}) than the limit on open files ({})", files, FD_LIMIT);
    let got = res!(scanned(&db));
    assert_eq!(got, want, "the live store's scan of {} files must list all {} keys", files, N);
    res!(db.shutdown());
    // Restarted on the index files, as a restored store is.
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg), schemes(), None, true, false));
    let got = res!(scanned(&db));
    assert_eq!(got, want, "the restarted store's scan of {} files must list all {} keys", files, N);
    res!(db.shutdown());
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

#[test]
fn scan_of_a_zone_with_more_files_than_the_fd_limit_is_whole() -> Outcome<()> {
    if env::var_os(CHILD_VAR).is_some() {
        return scan_under_the_limit();
    }
    let exe = res!(env::current_exe());
    // The shell lowers the soft limit and becomes this test binary, running this test alone.
    let out = res!(Command::new("sh")
        .arg("-c")
        .arg("ulimit -Sn \"$1\" && exec \"$2\" --exact \"$3\" --nocapture --test-threads=1")
        .arg("sh")
        .arg(FD_LIMIT.to_string())
        .arg(&exe)
        .arg(TEST_NAME)
        .env(CHILD_VAR, "1")
        .output());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(),
        "the scan failed with at most {} open files: {}\n{}\n{}", FD_LIMIT, out.status, stdout, stderr);
    assert!(stdout.contains("1 passed"), "the child ran no test: {}", stdout);
    Ok(())
}
