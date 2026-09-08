//! Crash-restart durability of the data-file rebuild (BUG B).
//!
//! # Why this exists
//!
//! An o3db zone keeps an append-only data file and an index file beside it. On
//! restart the index is read to populate the caches; when the index is missing
//! or disagrees with the data file, the zone instead rebuilds the index by
//! walking the data file (`InitGarbageBot::init_cache_data_file`). Under the
//! pre-durability default (never fsync) a crash routinely leaves the data and
//! index flushed to different points, so this rebuild path is the one a real
//! restart falls into.
//!
//! The property a log-structured store owes its caller is that an interrupted
//! final append costs the one record it interrupted and nothing else. The
//! index-driven path keeps that property (see `scan_torn_tail`), but the data
//! rebuild used to `res!`-abort the whole per-file walk on the first torn
//! record. That turned one torn record into a rebuild that returns an error,
//! which fails zone initialisation, which fails `Store::open`: a single torn
//! tail took the WHOLE store offline, and on the full (~1 MiB) live file of the
//! Ochre repro it was ~3k records' worth of data that could not be read back.
//!
//! This test reproduces that shape with many small unchunked records across
//! several rollovers, then simulates a crash by damaging the tail of one sealed
//! data file and removing its index (forcing the data rebuild). Two crash
//! shapes are covered, because an interrupted append leaves either:
//!
//!  - a torn key: a prefix of the next record reached disk (here a lone cache
//!    hash), so `StoredKey::load` reads the hash and then meets the end of the
//!    file loading the key;
//!  - a torn value: the key and the value's length header are whole but the
//!    value body was cut off. `StoredValue::count` walks a value by seeking
//!    over its declared length rather than reading it, so this is invisible at
//!    the key/value decode and shows up only as a record that claims to end
//!    past the file.
//!
//! Before the fix either shape aborts the rebuild and the store will not reopen
//! at all; after it, the rebuild truncates the torn tail and recovers every
//! other record in that file and every record in every other file.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::{
    prelude::*,
    alt::Override,
};
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_iop_db::api::{
    Database,
    RestSchemesOverride,
    ScanOpts,
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    base::cfg::OzoneConfig,
    data::core::RestSchemesInput,
    test::setup,
};

use std::{
    fs,
    io::Write,
    path::{
        Path,
        PathBuf,
    },
    thread,
    time::Duration,
};

/// How the last append to a data file was interrupted.
#[derive(Clone, Copy, Debug)]
enum Damage {
    /// A prefix of the next record (a lone cache hash) reached disk: the key
    /// decode runs off the end of the file.
    TornKey,
    /// The value's length header is whole but its body was cut short: the
    /// record claims to end past the file.
    TornValue,
}

/// Every file under `root` with the given extension, with its byte length.
fn files_with_ext(root: &Path, ext: &str) -> Outcome<Vec<(PathBuf, u64)>> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in res!(fs::read_dir(&dir)) {
            let path = res!(entry).path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().map(|e| e == ext).unwrap_or(false) {
                let len = res!(fs::metadata(&path)).len();
                found.push((path, len));
            }
        }
    }
    found.sort();
    Ok(found)
}

fn run_case(damage: Damage) -> Outcome<()> {

    let written: usize = 300;

    let dirname = fmt!("./test_db_crash_restart_{:?}", damage).to_lowercase();
    let db_root = res!(Path::new(&dirname).canonicalize().or_else(|_| {
        ok!(fs::create_dir_all(&dirname));
        Path::new(&dirname).canonicalize()
    }));

    let enckey = [0x37u8; 32];
    let aes_gcm = res!(EncryptionScheme::new_aes_256_gcm_with_key(&enckey[..]));
    let crc32 = ChecksumScheme::new_crc32();
    let schms2: RestSchemesOverride<EncryptionScheme, HashScheme> =
        RestSchemesOverride::default()
            .set_encrypter(Override::Default(aes_gcm.clone()));
    let schms2 = Some(&schms2);
    let user = setup::Uid::default();

    let schms_input = RestSchemesInput::new(
        Some(aes_gcm.clone()),
        None::<HashScheme>,
        None::<HashScheme>,
        Some(crc32.clone()),
    );

    // Start from the test config, but exercise the real default durability
    // policy (a bounded group commit). The file size is shrunk so a few
    // hundred ~300 B records force many rollovers and leave several full sealed
    // files; the records stay well under `rest_chunk_threshold`, so every write
    // takes the single unchunked path, exactly as the Ochre repro did.
    let mut cfg = res!(setup::default_cfg());
    cfg.num_zones            = 2;
    cfg.num_cbots_per_zone   = 2;
    cfg.num_wbots_per_zone   = 1;
    cfg.num_igbots_per_zone  = 2;
    cfg.data_file_max_bytes  = 8_000;
    cfg.rest_chunk_threshold = 1_500; // 300 B values stay unchunked.
    cfg.rest_chunk_bytes     = 64;
    cfg.sync_interval_ms     = OzoneConfig::default().sync_interval_ms; // the real default floor.
    cfg.zone_overrides       = mapdat!{
        1u16 => mapdat!{ "dir" => "", "max_size" => 10_000_000u64 },
        2u16 => mapdat!{ "dir" => "", "max_size" => 10_000_000u64 },
    }.get_map().unwrap_or_default();

    test!(sync_log::stream(), "+--- crash-restart durability: {:?} ---", damage);

    // ── 1. Write many small records over several rollovers, then stop cleanly ──
    let db = res!(setup::start_db(
        db_root.clone(),
        Some(cfg.clone()),
        schms_input.clone(),
        None,
        false, // gc off: the rebuild path is what is under test.
        true,  // wipe
    ));
    thread::sleep(Duration::from_millis(500));
    let val_body = "x".repeat(300);
    for i in 0..written {
        res!(db.insert(
            dat!(fmt!("crash:{:04}", i)),
            dat!(fmt!("{}:{:04}", val_body, i)),
            user,
            schms2,
        ));
    }
    thread::sleep(Duration::from_millis(500));
    res!(db.shutdown());
    thread::sleep(Duration::from_millis(500));

    // ── 2. Simulate the crash on one sealed data file and remove its index so
    //       the restart must rebuild it from the data and meet the torn tail. A
    //       sealed (not the highest-numbered, live) file is chosen so the tear
    //       is unambiguously a crash artefact rather than a writer appending. ──
    let dats = res!(files_with_ext(&db_root, "dat"));
    if dats.len() < 3 {
        return Err(err!(
            "Only {} data files were written; the config should have forced \
            several rollovers, so this test would prove nothing.", dats.len();
            Test, Size));
    }
    let live = dats[dats.len() - 1].0.clone();
    let mut victim: Option<(PathBuf, u64)> = None;
    for (p, len) in &dats {
        if *p == live {
            continue;
        }
        if victim.as_ref().map(|(_, n)| *len > *n).unwrap_or(true) {
            victim = Some((p.clone(), *len));
        }
    }
    let (victim_path, victim_len) = res!(victim.ok_or_else(|| err!(
        "No sealed data file was found to damage.";
        Test, Missing)));
    test!(sync_log::stream(),
        "Damaging sealed data file {:?} ({} bytes) by {:?} and removing its index.",
        victim_path, victim_len, damage);

    match damage {
        Damage::TornKey => {
            // Append a lone cache hash (CACHE_HASH_BYTES = 4): the key decode
            // reads it, then meets EOF loading the key itself.
            let mut file = res!(fs::OpenOptions::new().append(true).open(&victim_path));
            res!(file.write_all(&[0u8; 4]));
        },
        Damage::TornValue => {
            // Cut the last few bytes so the final value's body is short while
            // its length header survives.
            let file = res!(fs::OpenOptions::new().write(true).open(&victim_path));
            res!(file.set_len(victim_len.saturating_sub(9)));
        },
    }
    // Remove the matching index file to force the data-rebuild path.
    let mut ind_path = victim_path.clone();
    ind_path.set_extension("ind");
    if ind_path.is_file() {
        res!(fs::remove_file(&ind_path));
    }

    // ── 3. Reopen. Before the fix the rebuild aborts and this very open fails;
    //       after the fix it truncates the torn tail and opens. ──
    let db = match setup::start_db(
        db_root.clone(),
        Some(cfg.clone()),
        schms_input.clone(),
        None,
        false, // gc off
        false, // keep what is there
    ) {
        Ok(db) => db,
        Err(e) => return Err(err!(e,
            "The store would not reopen after a single {:?} in one data file. A \
            torn tail on the rebuild path must cost that one record, not the whole \
            store: this is the pre-fix abort-the-whole-file behaviour.", damage;
            Test, Data)),
    };
    thread::sleep(Duration::from_millis(500));

    // ── 4. Scan the store. A scan answers from the index files on disk, so it
    //       is the true measure of the rebuild: before the fix the aborted
    //       rebuild never writes the torn file's index, so every record in that
    //       file is invisible to a scan for the life of the store (and the
    //       rebuild re-aborts on every later restart); after the fix the index
    //       is rebuilt from the records the crash did not take, so they are
    //       scannable again. A `get` by key would hide this difference, because
    //       the rebuild streams each record into the caches as it reads it, so
    //       the records ahead of the tear are cached either way -- it is the
    //       index, and therefore the scan, that the whole-file abort destroys. ──
    let scanned = match db.scan(&ScanOpts::with_str_prefix("crash:"), schms2) {
        Ok(rows) => rows,
        Err(e) => {
            let _ = db.shutdown();
            return Err(err!(e,
                "[{:?}] The prefix scan failed after the crash-restart.", damage;
                Test, Data));
        },
    };
    let scanned_n = scanned.len();
    let scan_lost = written.saturating_sub(scanned_n);
    test!(sync_log::stream(),
        "[{:?}] scan returned {} of {} records; {} not scannable.",
        damage, scanned_n, written, scan_lost);

    // The property: an interrupted final append costs that one record, not a
    // whole file's worth. One tear was injected, so at most a small constant
    // number of records may drop out of the scan; a file's worth missing means
    // the whole-file abort left the torn file with no rebuilt index.
    if scan_lost > 3 {
        let _ = db.shutdown();
        return Err(err!(
            "[{:?}] {} of {} records are missing from the scan after a single torn \
            tail. The aborted rebuild left the torn data file with no index, so a \
            whole file's records became invisible to a scan -- the pre-fix \
            whole-file blast radius.",
            damage, scan_lost, written;
            Test, Data));
    }

    // And every scanned key must read back cleanly: a torn record left behind as
    // a poison pill would error here (the 2026-08-24 "Mismatch detected" scan
    // failure), whereas the fix removes it.
    for (k, _, _) in &scanned {
        match db.get(k, schms2) {
            Ok(Some(_)) => (),
            Ok(None) => {
                let _ = db.shutdown();
                return Err(err!(
                    "[{:?}] Scan returned key {:?} but a get of it found nothing.", damage, k;
                    Test, Data));
            },
            Err(e) => {
                let _ = db.shutdown();
                return Err(err!(e,
                    "[{:?}] A scanned key {:?} would not read back -- a torn record \
                    left as a poison pill.", damage, k;
                    Test, Data));
            },
        }
    }

    res!(db.shutdown());
    thread::sleep(Duration::from_millis(500));
    Ok(())
}

fn run() -> Outcome<()> {
    test!(sync_log::stream(), "+---------------------------------------------+");
    test!(sync_log::stream(), "| CRASH-RESTART DURABILITY (data rebuild)     |");
    test!(sync_log::stream(), "+---------------------------------------------+");
    res!(run_case(Damage::TornKey));
    res!(run_case(Damage::TornValue));
    test!(sync_log::stream(), "Crash-restart durability test passed.");
    Ok(())
}

#[test]
fn main() -> Outcome<()> {
    log_set_level!("trace");
    let outcome = run();
    log_finish_wait!();
    outcome
}
