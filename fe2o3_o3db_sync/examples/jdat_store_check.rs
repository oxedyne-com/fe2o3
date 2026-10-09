//! Checks the JDAT text a COPY of an Ozone store holds against the decoder's refusals, and prints
//! only counts and where each refusal lies, never a value (see
//! `oxedyne_fe2o3_o3db_sync::textscan`).
//!
//! ```ignore
//! jdat_store_check --o3db DIR --key KEYFILE    # Steel or Daimond: DIR is a copy of the Ozone store
//! ```
//!
//! Oxegen's own tables are framed text files, checked by `jdat_check --logs DIR` in
//! `oxedyne_fe2o3_jdat`, which builds static. This one links the Ozone store and is built for the
//! host's glibc.
//!
//! Exit status: 0 when every text read, 1 when any was refused, 2 on a usage error, an unreadable
//! store, or a store that holds nothing to read.
//!
//! # Safety
//! - Scan a `cp -a` COPY, never a live store: a second process on one store is never safe, even
//!   though an open that is only read from writes nothing (since 2026-10-09).
//! - The store is opened for the gateway's parameterisation (16-byte `u128` user ids, AES-256-GCM
//!   at rest, CRC-32), as `o3db_migrate` is (`oxedyne_fe2o3_o3db_sync::gateway`). The at-rest key
//!   is read from the path given and is never defaulted.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    base::constant,
    comm::response::Wait,
    gateway,
    textscan,
};
use oxedyne_fe2o3_jdat::string::scan::TextScan;

use std::{
    path::PathBuf,
    process,
    thread,
    time::Duration,
};


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
    let (dir, key_path) = res!(parse());
    let key = res!(gateway::load_key(&key_path));
    let (db, answered) = res!(gateway::open_store(&dir, None, &key, false, "jdat-store-check"));
    // A report of "clean" from a store that did not answer would be false, so this tool, which
    // only reports, stops here.
    if let Err(e) = gateway::require_answer(answered, "jdat-store-check", &dir) {
        let _ = db.shutdown();
        return Err(e);
    }
    let wait = Wait {
        max_wait:       Duration::from_secs(3600),
        check_interval: constant::CHECK_INTERVAL,
    };
    let mut scan = TextScan::default();
    let outcome = textscan::scan_o3db(db.api(), wait, &mut scan);
    res!(db.shutdown());
    thread::sleep(Duration::from_secs(1));
    res!(outcome);
    if scan.records == 0 {
        return Err(err!(
            "The store at {:?} holds no live key; wrong directory or wrong key?", dir;
            Missing, Input));
    }
    print!("{}", scan.report());
    if scan.is_clean() {
        println!("CLEAN: every text read.");
        Ok(0)
    } else {
        println!("NOT CLEAN: see the REFUSED lines.");
        Ok(1)
    }
}

fn parse() -> Outcome<(PathBuf, PathBuf)> {
    let usage = "Usage: --o3db DIR --key KEYFILE.";
    let mut o3db:   Option<PathBuf> = None;
    let mut key:    Option<PathBuf> = None;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let slot = match arg.as_str() {
            "--o3db"    => &mut o3db,
            "--key"     => &mut key,
            other       => return Err(err!("Unrecognised argument {:?}. {}", other, usage;
                Invalid, Input)),
        };
        *slot = Some(PathBuf::from(res!(it.next().ok_or_else(|| err!(
            "Argument {} needs a value. {}", arg, usage; Missing, Input)))));
    }
    match (o3db, key) {
        (Some(dir), Some(key))  => Ok((dir, key)),
        _ => Err(err!("Give --o3db DIR with --key KEYFILE. {}", usage; Invalid, Input)),
    }
}
