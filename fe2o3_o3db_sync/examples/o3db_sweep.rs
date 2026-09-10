//! In-place orphan sweep for an encrypted Ozone store.
//!
//! Reclaims the orphaned chunk-data records a churned store leaks -- chunk records left behind when
//! a chunked value was overwritten at a new geometry, or, on a pre-fix build, on any overwrite --
//! by tombstoning each orphaned chunk key so the running collector reclaims its bytes.  Unlike the
//! migration, this runs against a LIVE store with garbage collection ON and needs no second store
//! and no downtime.
//!
//! ```ignore
//! cargo run -p oxedyne_fe2o3_o3db_sync --example o3db_sweep -- \
//!     --source ./o3db --key ./keys/db/at_rest [--scan-secs 600] [--skew-secs 5]
//! ```
//!
//! The scheme parameterisation here MUST match the store's own: this binary is wired for the
//! daimond gateway's store -- 16-byte `u128` user ids, AES-256-GCM at rest, CRC-32 checksums.  A
//! store written under a different parameterisation will not read back and the sweep will fail
//! loudly rather than silently retire the wrong keys.
//!
//! # Safety
//! - The store is opened with garbage collection ON: the sweep relies on the collector to reclaim
//!   what it tombstones.  It issues no write to any value; it only tombstones chunk keys it has
//!   proven orphaned.
//! - The key is read from the path given at runtime.  It is NEVER hardcoded and there is NO default:
//!   a missing or wrong-sized key aborts the run.
//! - `--skew-secs` guards concurrent writers: only chunk records stamped more than this many seconds
//!   before the sweep started are retired.  The default is deliberately generous.  Pass `0` only for
//!   a store nothing is writing.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_jdat::id::IdDat;
use oxedyne_fe2o3_o3db_sync::{
    base::constant,
    comm::response::Wait,
    data::core::RestSchemesInput,
    db::O3db,
    sweep,
};

use std::{
    fs,
    path::{
        Path,
        PathBuf,
    },
    process,
    thread,
    time::Duration,
};


// The gateway's own parameterisation. Change these only to match a store
// written by a differently configured application.
type Uid = IdDat<16, u128>;
type Db  = O3db<16, Uid, EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>;

const DB_KEY_LEN: usize = 32;

// How long to let the running collector settle after the sweep returns, before measuring the
// reclaimed footprint. Reclamation is asynchronous; this is the operator tool's own settle.
const GC_SETTLE_SECS: u64 = 5;


fn main() {
    if let Err(e) = run() {
        eprintln!("o3db_sweep FAILED: {}", e);
        process::exit(1);
    }
}

fn run() -> Outcome<()> {
    log_set_level!("info");

    let args = res!(Args::parse());

    // Read the at-rest key from disk at runtime. No default, ever.
    let key = res!(load_key(&args.key_path));

    // Open the store with garbage collection ON: the sweep needs the collector running to reclaim
    // what it tombstones.
    info!("Opening store at {:?} (gc on)...", args.source);
    let db = res!(open_store(&args.source, &key, true, "sweep"));

    let scan_wait = Wait {
        max_wait:       Duration::from_secs(args.scan_secs),
        check_interval: constant::CHECK_INTERVAL,
    };
    info!(
        "Sweeping orphaned chunk records (scan deadline {}s, epoch skew {}s)...",
        args.scan_secs, args.skew_secs);
    let report = res!(sweep::sweep_orphans(
        db.api(),
        Uid::default(),
        None,
        scan_wait,
        Duration::from_secs(args.skew_secs),
    ));

    if report.orphans_retired != report.orphans_found {
        res!(db.shutdown());
        return Err(err!(
            "Sweep retired {} of {} orphans found; a tombstone write did not land. Investigate \
            before relying on the reclaim.", report.orphans_retired, report.orphans_found;
            Data));
    }

    // The sweep returns as soon as every tombstone is acknowledged; reclamation is asynchronous, so
    // the reclaimed footprint is the caller's to measure. Let the collector -- still running -- settle,
    // then measure the root before shutting down.
    info!("Letting the collector settle for {}s before measuring the reclaimed footprint...",
        GC_SETTLE_SECS);
    thread::sleep(Duration::from_secs(GC_SETTLE_SECS));
    let bytes_after = res!(zone_data_bytes(&args.source));

    res!(db.shutdown());
    thread::sleep(Duration::from_secs(1));

    println!("\n{}", report.summary());

    let pct = if report.bytes_before > 0 {
        (bytes_after as f64 / report.bytes_before as f64) * 100.0
    } else {
        100.0
    };
    println!(
        "\nDONE: {} orphaned chunk records retired, {} skipped as too recent to be sure.\n\
        Data bytes {} -> {} ({:.2}% of the pre-sweep footprint) after a {}s settle.",
        report.orphans_retired, report.skipped_recent,
        report.bytes_before, bytes_after, pct, GC_SETTLE_SECS);

    Ok(())
}

/// Total bytes of the store's data files (`.dat`) under a root, walked recursively; index files and
/// anything else are not counted. The reclaim the sweep drives shows up as data-file bytes.
fn zone_data_bytes(root: &Path) -> Outcome<u64> {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for entry in rd {
            let entry = res!(entry);
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().map(|e| e == constant::DATA_FILE_EXT).unwrap_or(false) {
                let meta = res!(entry.metadata());
                total += meta.len();
            }
        }
    }
    Ok(total)
}

/// Opens and starts a store.
fn open_store(
    root:   &Path,
    key:    &[u8; DB_KEY_LEN],
    gc_on:  bool,
    label:  &str,
)
    -> Outcome<Db>
{
    let aes_gcm = res!(EncryptionScheme::new_aes_256_gcm_with_key(&key[..]));
    let crc32 = ChecksumScheme::new_crc32();
    let schms_input = RestSchemesInput::new(
        Some(aes_gcm),
        None::<HashScheme>,
        None::<HashScheme>,
        Some(crc32),
    );
    let mut db: Db = res!(O3db::new(root, None, schms_input, Uid::default()));
    res!(db.start(label.to_string()));
    res!(ok!(db.updated_api()).activate_gc(gc_on));
    thread::sleep(Duration::from_millis(500));
    let (_, msgs) = res!(db.api().ping_bots(constant::USER_REQUEST_WAIT));
    info!("{}: {} bots responded.", label, msgs.len());
    Ok(db)
}

/// Reads the 32-byte at-rest key, failing loudly if it is absent or the wrong size. There is
/// deliberately no fallback: a wrong key cannot decrypt a byte.
fn load_key(path: &Path) -> Outcome<[u8; DB_KEY_LEN]> {
    if !path.exists() {
        return Err(err!(
            "No database key at {:?}. Supply the store's at-rest key with --key; \
            without it the store cannot be read.", path;
            Missing, Key, Input));
    }
    let bytes = res!(fs::read(path));
    if bytes.len() != DB_KEY_LEN {
        return Err(err!(
            "The database key at {:?} is {} bytes, but must be {}.",
            path, bytes.len(), DB_KEY_LEN;
            Invalid, Input, Key));
    }
    let mut key = [0u8; DB_KEY_LEN];
    key.copy_from_slice(&bytes);
    Ok(key)
}

struct Args {
    source:     PathBuf,
    key_path:   PathBuf,
    scan_secs:  u64,
    skew_secs:  u64,
}

impl Args {
    fn parse() -> Outcome<Self> {
        let mut source:     Option<PathBuf> = None;
        let mut key_path:   Option<PathBuf> = None;
        let mut scan_secs:  u64 = 600;
        let mut skew_secs:  u64 = 5;

        let mut it = std::env::args().skip(1);
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--source" => source   = Some(PathBuf::from(res!(next(&mut it, "--source")))),
                "--key"    => key_path = Some(PathBuf::from(res!(next(&mut it, "--key")))),
                "--scan-secs" => {
                    let v = res!(next(&mut it, "--scan-secs"));
                    scan_secs = res!(v.parse::<u64>().map_err(|_| err!(
                        "--scan-secs must be a whole number of seconds, got {:?}.", v;
                        Invalid, Input)));
                },
                "--skew-secs" => {
                    let v = res!(next(&mut it, "--skew-secs"));
                    skew_secs = res!(v.parse::<u64>().map_err(|_| err!(
                        "--skew-secs must be a whole number of seconds, got {:?}.", v;
                        Invalid, Input)));
                },
                other => return Err(err!(
                    "Unrecognised argument {:?}. Usage: --source DIR --key PATH \
                    [--scan-secs N] [--skew-secs N].", other;
                    Invalid, Input)),
            }
        }
        Ok(Self {
            source:     res!(source.ok_or_else(||   err!("Missing --source DIR.";   Missing, Input))),
            key_path:   res!(key_path.ok_or_else(|| err!("Missing --key PATH.";     Missing, Input))),
            scan_secs,
            skew_secs,
        })
    }
}

fn next(it: &mut impl Iterator<Item = String>, flag: &str) -> Outcome<String> {
    it.next().ok_or_else(|| err!(
        "Argument {} needs a value.", flag; Missing, Input))
}
