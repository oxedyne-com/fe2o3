//! Live-set migration / compaction for an encrypted Ozone store.
//!
//! Copies exactly the live key set of a source store into a fresh target store,
//! dropping orphaned chunk-data records (the leak that grows a churned store
//! without bound). Both stores are opened under the SAME at-rest key and the
//! SAME configuration, so the target holds the same values, freshly encrypted,
//! with matching chunk geometry.
//!
//! ```ignore
//! cargo run -p oxedyne_fe2o3_o3db_sync --example o3db_migrate -- \
//!     --source ./o3db --target ./o3db.new --key ./keys/db/at_rest [--scan-secs 600]
//! ```
//!
//! The scheme parameterisation here MUST match the store's own: this binary is
//! wired for the daimond gateway's store -- 16-byte `u128` user ids, AES-256-GCM
//! at rest, CRC-32 checksums. A store written under a different parameterisation
//! will not read back and the migration will fail loudly rather than silently
//! copy garbage.
//!
//! # Safety
//! - The source is opened with garbage collection OFF and is only ever read;
//!   every write goes to the target. Stop the process that owns the store first,
//!   so the store is quiescent.
//! - The key is read from the path given at runtime. It is NEVER hardcoded and
//!   there is NO default: a missing or wrong-sized key aborts the run.
//! - The run refuses to report success unless the copy verified: the target's
//!   live-key count equals the source's, and every value read back
//!   byte-identical.
//! - The operator still owns the shrink gate: this prints the source and target
//!   byte totals; a target that did not shrink as expected must NOT be swapped
//!   in. See the runbook.
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
    migrate,
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


fn main() {
    if let Err(e) = run() {
        eprintln!("o3db_migrate FAILED: {}", e);
        process::exit(1);
    }
}

fn run() -> Outcome<()> {
    log_set_level!("info");

    let args = res!(Args::parse());

    // Read the at-rest key from disk at runtime. No default, ever.
    let key = res!(load_key(&args.key_path));

    // 1. Open the source read-only: garbage collection off, no writes issued.
    info!("Opening source store at {:?} (gc off, read-only)...", args.source);
    let src = res!(open_store(&args.source, None, &key, false, "migrate-src"));

    // 2. Create the target with the SOURCE'S configuration, so chunk geometry
    //    and encryption parameters match exactly.
    let src_cfg = src.cfg().clone();
    info!("Creating fresh target store at {:?} with the source's configuration...", args.target);
    let tgt = res!(open_store(&args.target, Some(src_cfg), &key, false, "migrate-tgt"));

    // 3. Migrate and verify.
    let scan_wait = Wait {
        max_wait:       Duration::from_secs(args.scan_secs),
        check_interval: constant::CHECK_INTERVAL,
    };
    info!("Migrating live key set (scan deadline {}s)...", args.scan_secs);
    let report = res!(migrate::migrate_live_set(
        src.api(),
        tgt.api(),
        Uid::default(),
        None,
        scan_wait,
    ));

    // 4. Shut both down cleanly so the target's bytes are settled on disk.
    res!(tgt.shutdown());
    res!(src.shutdown());
    thread::sleep(Duration::from_secs(1));

    println!("\n{}", report.summary());

    if !report.verified_ok {
        return Err(err!(
            "Migration did NOT verify; the target must not be swapped in.";
            Data));
    }

    // The shrink gate is the operator's to enforce, but flag the unexpected.
    if report.source_bytes > 0 && report.target_bytes * 2 >= report.source_bytes {
        println!(
            "\nWARNING: the target did NOT shrink much (target is {} of source bytes). \
            If a large reclaim was expected, DO NOT swap; investigate first.",
            fmt!("{:.1}%", (report.target_bytes as f64 / report.source_bytes as f64) * 100.0));
    }

    println!(
        "\nPASS: {} live keys copied and verified byte-identical; {} tombstones dropped.\n\
        Source: {} bytes / {} files.  Target: {} bytes / {} files.\n\
        The source was opened read-only; verify its data/index files are unchanged before swapping.",
        report.copied, report.tombstones,
        report.source_bytes, report.source_files,
        report.target_bytes, report.target_files);

    Ok(())
}

/// Opens and starts a store, leaving garbage collection off unless asked.
fn open_store(
    root:       &Path,
    cfg_opt:    Option<OzoneConfigT>,
    key:        &[u8; DB_KEY_LEN],
    gc_on:      bool,
    label:      &str,
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
    let mut db: Db = res!(O3db::new(root, cfg_opt, schms_input, Uid::default()));
    res!(db.start(label.to_string()));
    // Default is off; state it explicitly so a read never collects.
    res!(ok!(db.updated_api()).activate_gc(gc_on));
    thread::sleep(Duration::from_millis(500));
    let (_, msgs) = res!(db.api().ping_bots(constant::USER_REQUEST_WAIT));
    info!("{}: {} bots responded.", label, msgs.len());
    Ok(db)
}

/// Reads the 32-byte at-rest key, failing loudly if it is absent or the wrong
/// size. There is deliberately no fallback: a wrong key cannot decrypt a byte.
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

// Alias so the signature above does not need the full config path spelled out.
type OzoneConfigT = oxedyne_fe2o3_o3db_sync::base::cfg::OzoneConfig;

struct Args {
    source:     PathBuf,
    target:     PathBuf,
    key_path:   PathBuf,
    scan_secs:  u64,
}

impl Args {
    fn parse() -> Outcome<Self> {
        let mut source:     Option<PathBuf> = None;
        let mut target:     Option<PathBuf> = None;
        let mut key_path:   Option<PathBuf> = None;
        let mut scan_secs:  u64 = 600;

        let mut it = std::env::args().skip(1);
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--source" => source   = Some(PathBuf::from(res!(next(&mut it, "--source")))),
                "--target" => target   = Some(PathBuf::from(res!(next(&mut it, "--target")))),
                "--key"    => key_path = Some(PathBuf::from(res!(next(&mut it, "--key")))),
                "--scan-secs" => {
                    let v = res!(next(&mut it, "--scan-secs"));
                    scan_secs = res!(v.parse::<u64>().map_err(|_| err!(
                        "--scan-secs must be a whole number of seconds, got {:?}.", v;
                        Invalid, Input)));
                },
                other => return Err(err!(
                    "Unrecognised argument {:?}. Usage: --source DIR --target DIR \
                    --key PATH [--scan-secs N].", other;
                    Invalid, Input)),
            }
        }
        Ok(Self {
            source:     res!(source.ok_or_else(||   err!("Missing --source DIR.";   Missing, Input))),
            target:     res!(target.ok_or_else(||   err!("Missing --target DIR.";   Missing, Input))),
            key_path:   res!(key_path.ok_or_else(|| err!("Missing --key PATH.";     Missing, Input))),
            scan_secs,
        })
    }
}

fn next(it: &mut impl Iterator<Item = String>, flag: &str) -> Outcome<String> {
    it.next().ok_or_else(|| err!(
        "Argument {} needs a value.", flag; Missing, Input))
}
