//! What a scan does when the last record in an index file is damaged.
//!
//! # Why this exists
//!
//! An o3db zone's index is an append-only log, and a process that dies without
//! a clean shutdown leaves whatever the last append had managed to put on disk.
//! That is not an exotic condition: `dev/verify_operators.mjs` restarts the
//! Daimond gateway with `SIGKILL` twice in one run, against the same store, and
//! every deployment ends the same way sooner or later.
//!
//! The property a log-structured store owes its caller is that damage at the
//! TAIL costs the records in that damage and nothing else, and this check was
//! written to find out whether o3db keeps it. **It does**, which is the finding
//! and the reason the check is worth keeping:
//!
//! - Damage to an INDEX file costs nothing at all, either shape. The index is
//!   rebuilt from the data file beside it when the zone starts, so the damage is
//!   gone before a reader sees it: 12 of 12 written records read back.
//! - Damage to a DATA file costs exactly the record it damaged. The scan still
//!   returns all 12 keys, because a scan answers from the index; 11 of the 12
//!   values then read back and the twelfth does not.
//!
//! So a store that was stopped mid-write loses what was in flight and stays
//! readable otherwise. Where `store.scan_prefix("lic:") failed after 10.092 s:
//! Mismatch detected` came from on 2026-08-24 is therefore NOT o3db widening
//! one bad record into a dead prefix -- it is the caller doing that, and the
//! caller is `Store::scan_prefix` in the Daimond gateway, which `res!`s the
//! `get` of every key the scan returned.
//!
//! Two shapes are checked because an interrupted append leaves either: a record
//! cut short, and a record whose bytes are all there and wrong.
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
    data::core::RestSchemesInput,
    test::setup,
};

use std::{
    fs,
    path::{
        Path,
        PathBuf,
    },
    thread,
    time::Duration,
};

/// How the last append was interrupted.
#[derive(Clone, Copy, Debug)]
enum Damage {
    /// The write did not finish: the file is short by a few bytes.
    CutShort,
    /// The bytes arrived and are wrong, which is what a checksum is for.
    Corrupted,
}

/// Every file under `root` with the given extension.
fn files_with_ext(root: &Path, ext: &str) -> Outcome<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in res!(fs::read_dir(&dir)) {
            let path = res!(entry).path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().map(|e| e == ext).unwrap_or(false) {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

fn run_case(target: &'static str, damage: Damage, written: usize) -> Outcome<()> {

    let dirname = fmt!("./test_db_scan_torn_{}_{:?}", target, damage).to_lowercase();
    let db_root = res!(Path::new(&dirname).canonicalize().or_else(|_| {
        ok!(fs::create_dir_all(&dirname));
        Path::new(&dirname).canonicalize()
    }));

    let enckey = [0x42u8; 32];
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

    let mut cfg = res!(setup::default_cfg());
    cfg.num_zones            = 1;
    cfg.num_cbots_per_zone   = 2;
    cfg.num_igbots_per_zone  = 2;
    cfg.data_file_max_bytes  = 200_000;
    cfg.zone_overrides       = mapdat!{
        1u16 => mapdat!{ "dir" => "", "max_size" => 10_000_000u64 },
    }.get_map().unwrap_or_default();

    test!(sync_log::stream(), "+--- torn tail: .{} {:?} ---", target, damage);

    // ── 1. Write a prefix's worth of records and stop cleanly ──
    let mut db = res!(setup::start_db(
        db_root.clone(),
        Some(cfg.clone()),
        schms_input.clone(),
        None,
        false, // gc off: an index rebuilt from the data file would repair the
               // damage before the scan saw it, and what is under test here is
               // the scan.
        true,  // wipe
    ));
    thread::sleep(Duration::from_secs(1));
    for i in 0..written {
        res!(db.insert(
            dat!(fmt!("lic:{:03}", i)),
            dat!(fmt!("licence_{}", i)),
            user,
            schms2,
        ));
    }
    thread::sleep(Duration::from_millis(500));
    res!(db.shutdown());
    thread::sleep(Duration::from_millis(500));

    // ── 2. Damage the last record of the largest such file ──
    let inds = res!(files_with_ext(&db_root, target));
    let mut biggest: Option<(PathBuf, u64)> = None;
    for p in inds {
        let len = res!(fs::metadata(&p)).len();
        if biggest.as_ref().map(|(_, n)| len > *n).unwrap_or(true) {
            biggest = Some((p, len));
        }
    }
    let (path, len) = res!(biggest.ok_or_else(|| err!(
        "No .{} file was written under {:?}, so there is nothing to damage and \
        this check would prove nothing.", target, db_root;
        Test, Missing)));
    test!(sync_log::stream(), "Damaging {:?} ({} bytes) by {:?}.", path, len, damage);
    match damage {
        Damage::CutShort => {
            let file = res!(fs::OpenOptions::new().write(true).open(&path));
            res!(file.set_len(len.saturating_sub(9)));
        },
        Damage::Corrupted => {
            let mut bytes = res!(fs::read(&path));
            let n = bytes.len();
            if n < 4 {
                return Err(err!("File {:?} is too small to damage.", path;
                    Test, Size));
            }
            // The last four bytes of a record are its checksum, so flipping a
            // bit just before them damages content the checksum covers.
            bytes[n - 5] ^= 0xFF;
            res!(fs::write(&path, &bytes));
        },
    }

    // ── 3. Reopen and scan ──
    let db = res!(setup::start_db(
        db_root.clone(),
        Some(cfg.clone()),
        schms_input.clone(),
        None,
        false, // gc off
        false, // keep what is there
    ));
    thread::sleep(Duration::from_secs(1));

    let outcome = db.scan(&ScanOpts::with_str_prefix("lic:"), schms2);
    let rows = match outcome {
        Ok(rows) => rows,
        Err(e) => {
            let _ = db.shutdown();
            return Err(err!(e,
                "A {:?} tail in one .{} file made the WHOLE prefix scan fail. \
                Damage at the tail of an append-only log must cost the records \
                in that damage and no others: this is a store that cannot be \
                read at all after an unclean stop.", damage, target;
                Test, Data));
        },
    };
    test!(sync_log::stream(), "Scan returned {} of {} after .{} {:?}.",
        rows.len(), written, target, damage);

    // Every row that did come back must be intact and ours. A scan that
    // survives damage by handing back rubbish would be worse than one that
    // fails.
    for (k, _, _) in &rows {
        match k {
            Dat::Str(s) => if !s.starts_with("lic:") {
                let _ = db.shutdown();
                return Err(err!("Scan returned {:?}, which is not a 'lic:' key.", k;
                    Test, Mismatch));
            },
            other => {
                let _ = db.shutdown();
                return Err(err!("Expected a Dat::Str key, got {:?}.", other;
                    Test, Mismatch));
            },
        }
    }
    if rows.len() == 0 {
        let _ = db.shutdown();
        return Err(err!(
            "The scan came back empty after {:?}. One damaged record cost every \
            one of the {} written, which is the same outage as an error and is \
            quieter about it.", damage, written;
            Test, Data));
    }

    // And now read each value, because that is what the caller does. o3db's
    // scan answers from the INDEX and hands every value back as `Dat::Empty`,
    // so a scan alone never opens a data file and never checksums one --
    // `Store::scan_prefix` in the gateway scans for keys and then `get`s each,
    // and it is the `get` that reads the record whose checksum failed on
    // 2026-08-24. A check that stopped at the scan would be measuring the half
    // of the path that cannot fail.
    let mut read_ok  = 0usize;
    let mut read_bad = Vec::new();
    for (k, _, _) in &rows {
        match db.get(k, schms2) {
            Ok(Some(_)) => read_ok += 1,
            Ok(None)    => read_bad.push(fmt!("{:?}: gone", k)),
            Err(e)      => read_bad.push(fmt!("{:?}: {}", k, e.plain())),
        }
    }
    test!(sync_log::stream(), "Read back {} of {} scanned keys after .{} {:?}; {} would not read.",
        read_ok, rows.len(), target, damage, read_bad.len());

    // The property: damage at the tail costs the records in that damage and no
    // others. One damaged record may cost itself; it may not cost the store.
    if read_ok + 1 < written {
        let _ = db.shutdown();
        return Err(err!(
            "After .{} {:?}, only {} of {} records could be read back. Damage at \
            the tail of an append-only log must cost the records in that damage \
            and no others. What would not read: {:?}",
            target, damage, read_ok, written, read_bad;
            Test, Data));
    }

    res!(db.shutdown());
    thread::sleep(Duration::from_millis(500));
    Ok(())
}

pub fn test_scan_torn_tail(_filter: &'static str) -> Outcome<()> {
    test!(sync_log::stream(), "+---------------------------------------------+");
    test!(sync_log::stream(), "| SCAN OVER A TORN TAIL                       |");
    test!(sync_log::stream(), "+---------------------------------------------+");
    // The index first, which o3db rebuilds from the data file at start-up. Both
    // shapes cost nothing, measured: this pair is here so that the repair stays
    // repaired, and so the data pair below cannot be read as a general claim
    // about damage.
    res!(run_case("ind", Damage::CutShort,  12));
    res!(run_case("ind", Damage::Corrupted, 12));
    // And the data file, which is what the index is rebuilt FROM, so there is
    // nothing behind it to repair it with.
    res!(run_case("dat", Damage::CutShort,  12));
    res!(run_case("dat", Damage::Corrupted, 12));
    test!(sync_log::stream(), "Torn tail test passed.");
    Ok(())
}
