//! A presence query gives `get`'s answer, whatever the key's history.
//!
//! Over a seeded mix of keys -- small values of every length up to and past a tombstone's, values
//! large enough to be chunked, deleted keys, keys deleted and written again, and keys never
//! written -- `contains` must agree with `get(..).is_some()` for every key: with the values
//! cached, with them cleared from the cache so a tombstone-length record is read, and after a
//! reopen.  The store is unencrypted, so a small value can have a tombstone's length exactly.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

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
        constant,
        id::usr_kind_id_deleted,
    },
    comm::response::Wait,
    data::core::RestSchemesInput,
    test::setup,
};

use std::{
    path::PathBuf,
    time::Duration,
};


type Db = O3db<{ setup::UID_LEN }, setup::Uid, (), HashScheme, HashScheme, ChecksumScheme>;

const SEED: u64     = 0x5eed_0f_0a_be5e;
const KEYS: usize   = 600;
const OPS:  usize   = 1_500;

// A xorshift, so the mix is the same on every run and a failure names its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn key(i: usize) -> Dat {
    dat!(fmt!("k{:04}", i))
}

fn wait() -> Wait {
    Wait {
        max_wait:       Duration::from_secs(30),
        check_interval: constant::CHECK_INTERVAL,
    }
}

fn open(root: &PathBuf, wipe: bool) -> Outcome<Db> {
    let schms_input = RestSchemesInput::new(
        None::<()>,
        None::<HashScheme>,
        None::<HashScheme>,
        Some(ChecksumScheme::new_crc32()),
    );
    let mut cfg = res!(setup::default_cfg());
    cfg.zone_overrides = DaticleMap::new();
    // The test geometry: a value past 1,500 bytes is chunked.
    setup::start_db(root.clone(), Some(cfg), schms_input, None, false, wipe)
}

// A string whose encoding is `len` bytes, or the nearest under it.
fn value(rng: &mut Rng, len: usize) -> Outcome<Dat> {
    let mut n = len;
    loop {
        let v = Dat::Str((0..n).map(|_| (b'a' + (rng.next() % 26) as u8) as char).collect());
        if n == 0 || res!(v.as_bytes()).len() <= len {
            return Ok(v);
        }
        n -= 1;
    }
}

fn agree(db: &Db, when: &str) -> Outcome<usize> {
    let all: Vec<Dat> = (0..KEYS).map(key).collect();
    let held = res!(db.contains_many(&all, None));
    let mut n = 0;
    for (i, k) in all.iter().enumerate() {
        let got = res!(db.get(k, None)).is_some();
        if held[i] != got {
            return Err(err!(
                "{}, seed {:#x}: for {:?}, contains says {} and get says {}.",
                when, SEED, k, held[i], got;
                Test, Mismatch));
        }
        if res!(db.contains(k, None)) != got {
            return Err(err!(
                "{}, seed {:#x}: for {:?}, contains of one key says {} and get says {}.",
                when, SEED, k, !got, got;
                Test, Mismatch));
        }
        if got {
            n += 1;
        }
    }
    println!("{}: {} of {} keys held, contains agreeing with get on all", when, n, KEYS);
    Ok(n)
}

#[test]
fn presence_agrees_with_get() -> Outcome<()> {
    log_set_level!("warn");
    let outcome = run();
    log_finish_wait!();
    outcome
}

fn run() -> Outcome<()> {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("test_db_presence_agree");
    let _ = std::fs::remove_dir_all(&root);
    res!(std::fs::create_dir_all(&root));
    let root = res!(root.canonicalize());

    let tomb = res!(Dat::Usr(usr_kind_id_deleted(), Some(Box::new(Dat::Empty))).as_bytes());
    let user = setup::Uid::default();
    let mut rng = Rng(SEED);
    {
        let db = res!(open(&root, true));
        let (mut puts, mut dels, mut at_tomb) = (0, 0, 0);
        for _ in 0..OPS {
            // Keys past 5/6 of the range are never written.
            let i = rng.below(KEYS * 5 / 6);
            match rng.below(10) {
                0..=2 => {
                    res!(db.delete(&key(i), user, None));
                    dels += 1;
                },
                _ => {
                    let len = match rng.below(5) {
                        0       => tomb.len(),          // a tombstone's length exactly
                        1       => 1_600 + rng.below(4_000), // chunked
                        _       => 2 + rng.below(40),
                    };
                    let v = res!(value(&mut rng, len));
                    if res!(v.as_bytes()).len() == tomb.len() {
                        at_tomb += 1;
                    }
                    res!(db.insert(key(i), v, user, None));
                    puts += 1;
                },
            }
        }
        println!("seed {:#x}: {} puts ({} of a tombstone's length), {} deletes over {} keys",
            SEED, puts, at_tomb, dels, KEYS);
        if at_tomb == 0 {
            return Err(err!("No value of a tombstone's length was written."; Test, Missing));
        }
        let cached = res!(agree(&db, "values cached"));
        res!(db.api().clear_cache_values(wait()));
        let cleared = res!(agree(&db, "values cleared"));
        if cached != cleared {
            return Err(err!("Clearing the cache changed the keys held, {} to {}.", cached, cleared;
                Test, Mismatch));
        }
        res!(db.close());
    }
    {
        let db = res!(open(&root, false));
        res!(agree(&db, "reopened"));
        res!(db.close());
    }
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}
