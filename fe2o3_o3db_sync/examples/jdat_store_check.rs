//! Checks the JDAT text a COPY of a store holds against the decoder's refusals, and prints only
//! counts and where each refusal lies, never a value (see `oxedyne_fe2o3_o3db_sync::textscan`).
//!
//! ```ignore
//! jdat_store_check --logs DIR                  # Oxegen: DIR holds the <table>.log and <table>.tbl files
//! jdat_store_check --o3db DIR --key KEYFILE    # Steel or Daimond: DIR is a copy of the Ozone store
//! ```
//!
//! Exit status: 0 when every text read, 1 when any was refused or a file ends in a broken record,
//! 2 on a usage error, an unreadable store, or a store that holds nothing to read.
//!
//! # Safety
//! - Scan a `cp -a` COPY, never a live store, and never with a second process on one store: a
//!   read-only open of an Ozone store with garbage collection off still rewrites `config.jdat`.
//! - The Ozone mode is wired for the gateway's parameterisation (16-byte `u128` user ids,
//!   AES-256-GCM at rest, CRC-32), as `o3db_migrate` is. The at-rest key is read from the path
//!   given and is never defaulted.
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
    textscan,
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


type Uid = IdDat<16, u128>;
type Db  = O3db<16, Uid, EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>;

const DB_KEY_LEN: usize = 32;

enum Mode {
    Logs(PathBuf),
    O3db(PathBuf, PathBuf),
}

fn main() {
    match run() {
        Ok(code)    => process::exit(code),
        Err(e)      => {
            eprintln!("jdat_store_check FAILED: {}", e);
            process::exit(2);
        },
    }
}

fn run() -> Outcome<i32> {
    log_set_level!("warn");
    let mode = res!(parse());
    let mut scan = textscan::TextScan::default();
    match mode {
        Mode::Logs(dir) => {
            res!(textscan::scan_dir(&dir, &mut scan));
            if scan.files == 0 {
                return Err(err!(
                    "No .log or .tbl file in {:?}; this is not an Oxegen data directory.", dir;
                    Missing, Input));
            }
        },
        Mode::O3db(dir, key_path) => {
            let key = res!(load_key(&key_path));
            let db = res!(open_store(&dir, &key));
            let wait = Wait {
                max_wait:       Duration::from_secs(3600),
                check_interval: constant::CHECK_INTERVAL,
            };
            let outcome = textscan::scan_o3db(db.api(), wait, &mut scan);
            res!(db.shutdown());
            thread::sleep(Duration::from_secs(1));
            res!(outcome);
            if scan.records == 0 {
                return Err(err!(
                    "The store at {:?} holds no live key; wrong directory or wrong key?", dir;
                    Missing, Input));
            }
        },
    }
    print!("{}", scan.report());
    if scan.is_clean() {
        println!("CLEAN: every text read.");
        Ok(0)
    } else {
        println!("NOT CLEAN: see the REFUSED and TRUNCATED lines.");
        Ok(1)
    }
}

// Opens the store with its own configuration and garbage collection off.
fn open_store(
    root:   &Path,
    key:    &[u8; DB_KEY_LEN],
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
    res!(db.start("jdat-store-check".to_string()));
    res!(ok!(db.updated_api()).activate_gc(false));
    thread::sleep(Duration::from_millis(500));
    let (_, msgs) = res!(db.api().ping_bots(constant::USER_REQUEST_WAIT));
    if msgs.is_empty() {
        return Err(err!("No bot of the store at {:?} answered.", root; Missing, Data));
    }
    Ok(db)
}

fn load_key(path: &Path) -> Outcome<[u8; DB_KEY_LEN]> {
    if !path.exists() {
        return Err(err!(
            "No database key at {:?}. Supply the store's at-rest key with --key.", path;
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

fn parse() -> Outcome<Mode> {
    let usage = "Usage: --logs DIR   or   --o3db DIR --key KEYFILE.";
    let mut logs:   Option<PathBuf> = None;
    let mut o3db:   Option<PathBuf> = None;
    let mut key:    Option<PathBuf> = None;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let slot = match arg.as_str() {
            "--logs"    => &mut logs,
            "--o3db"    => &mut o3db,
            "--key"     => &mut key,
            other       => return Err(err!("Unrecognised argument {:?}. {}", other, usage;
                Invalid, Input)),
        };
        *slot = Some(PathBuf::from(res!(it.next().ok_or_else(|| err!(
            "Argument {} needs a value. {}", arg, usage; Missing, Input)))));
    }
    match (logs, o3db, key) {
        (Some(dir), None, None)             => Ok(Mode::Logs(dir)),
        (None, Some(dir), Some(key))        => Ok(Mode::O3db(dir, key)),
        _ => Err(err!("Give --logs DIR alone, or --o3db DIR with --key KEYFILE. {}", usage;
            Invalid, Input)),
    }
}
