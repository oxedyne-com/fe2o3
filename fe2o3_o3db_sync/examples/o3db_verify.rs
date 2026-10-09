//! Reads every live value of a COPY of an Ozone store and names each key that does not read back
//! whole: a chunked value torn between two writes fails its AES-GCM tag (see
//! `oxedyne_fe2o3_o3db_sync::verify`).
//!
//! ```ignore
//! o3db_verify --o3db DIR [--key KEYFILE] [--scan-secs N]   # or O3DB_KEY_FILE=KEYFILE in the environment
//! ```
//!
//! Exit status: 0 when every live key read back, 1 when any failed, 2 on a usage error, an
//! unreadable store, or a store that holds no live key.
//!
//! # Safety
//! - Check a `cp -a` COPY taken with the store's own process stopped, never a live store, and
//!   never with a second process on one store.  The check issues no write; the store's bots still
//!   survey its files when it opens.
//! - The store is opened for the gateway's parameterisation (16-byte `u128` user ids, AES-256-GCM
//!   at rest, CRC-32), as `o3db_migrate` is (`oxedyne_fe2o3_o3db_sync::gateway`).  The at-rest key
//!   is read from the file named by `--key` or `O3DB_KEY_FILE`, and is never defaulted.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    base::constant,
    comm::response::Wait,
    gateway,
    verify,
};

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
            eprintln!("o3db_verify FAILED: {}", e);
            process::exit(2);
        },
    }
}

fn run() -> Outcome<i32> {
    log_set_level!("warn");
    let (dir, key_path, secs) = res!(parse());
    let key = res!(gateway::load_key(&key_path));
    let (db, answered) = res!(gateway::open_store(&dir, None, &key, false, "o3db-verify"));
    if let Err(e) = gateway::require_answer(answered, "o3db-verify", &dir) {
        let _ = db.shutdown();
        return Err(e);
    }
    let wait = Wait {
        max_wait:       Duration::from_secs(secs),
        check_interval: constant::CHECK_INTERVAL,
    };
    let outcome = verify::verify_live_set(db.api(), None, wait);
    res!(db.shutdown());
    thread::sleep(Duration::from_secs(1));
    let rep = res!(outcome);
    if rep.keys == 0 {
        return Err(err!(
            "The store at {:?} holds no live key; wrong directory?", dir;
            Missing, Input));
    }
    print!("{}", rep.summary(200));
    if rep.clean() {
        println!("CLEAN: every live key read back whole.");
        Ok(0)
    } else {
        println!("NOT CLEAN: {} key(s) failed to read; see the FAILED lines.", rep.failures.len());
        Ok(1)
    }
}

fn parse() -> Outcome<(PathBuf, PathBuf, u64)> {
    let usage = "Usage: --o3db DIR [--key KEYFILE] [--scan-secs N], or O3DB_KEY_FILE for the key.";
    let mut o3db:   Option<PathBuf> = None;
    let mut key:    Option<PathBuf> = std::env::var_os("O3DB_KEY_FILE").map(PathBuf::from);
    let mut secs:   u64 = 3600;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let val = res!(it.next().ok_or_else(|| err!(
            "Argument {} needs a value. {}", arg, usage; Missing, Input)));
        match arg.as_str() {
            "--o3db"        => o3db = Some(PathBuf::from(val)),
            "--key"         => key = Some(PathBuf::from(val)),
            "--scan-secs"   => secs = res!(val.parse::<u64>().map_err(|_| err!(
                "--scan-secs must be a whole number of seconds, got {:?}.", val; Invalid, Input))),
            other           => return Err(err!("Unrecognised argument {:?}. {}", other, usage;
                Invalid, Input)),
        }
    }
    match (o3db, key) {
        (Some(dir), Some(key))  => Ok((dir, key, secs)),
        (None, _)               => Err(err!("Give --o3db DIR. {}", usage; Missing, Input)),
        (_, None)               => Err(err!("Give --key KEYFILE or set O3DB_KEY_FILE. {}", usage;
            Missing, Input, Key)),
    }
}
