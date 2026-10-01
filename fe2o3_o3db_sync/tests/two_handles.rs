//! Two handles open on one store at once, as Oregami's `voice` command runs beside its forge.
//!
//! Reproduces the multi-process fault found on 2026-09-23 (M3 in the ore lane's store timeouts
//! report): a zone survey hands its writer any incomplete data file, the live file of a handle
//! that is still running included, without claiming it, so both handles append to one file.  Each
//! writer takes a record's offset from the file length it last saw, and the other handle's appends
//! make that stale.  Ignored until live files are made exclusive across handles, which is a
//! decision still to be taken; run it with `--ignored` to see what is lost.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    O3db,
    base::cfg::OzoneConfig,
    data::core::RestSchemesInput,
    test::setup::{
        self,
        Uid,
        UID_LEN,
    },
};

use std::{
    collections::BTreeMap,
    path::{
        Path,
        PathBuf,
    },
};

type TestDb = O3db<
    { UID_LEN },
    Uid,
    (),
    HashScheme,
    HashScheme,
    ChecksumScheme,
>;

#[test]
#[ignore]
fn main() -> Outcome<()> {
    log_set_level!("error");
    let outcome = run();
    log_finish_wait!();
    outcome
}

fn run() -> Outcome<()> {
    let interleaved = res!(interleaved());
    let beside = res!(beside_a_running_store());
    msg!("Interleaved writes from two handles: {}", interleaved);
    msg!("A second handle opened, written and closed beside a first: {}", beside);
    if interleaved.lost() + beside.lost() > 0 {
        return Err(err!(
            "Two handles on one store lost writes.  Interleaved: {}.  Beside a running \
            store: {}.", interleaved, beside;
            Test, Data, Missing));
    }
    Ok(())
}

/// Both handles open, writing in turn.
fn interleaved() -> Outcome<Tally> {
    let root = res!(fresh("./test_db_two_handles_interleaved"));
    let a = res!(open(&root));
    let b = res!(open(&root));
    let mut written = Vec::new();
    for i in 0..40usize {
        for (h, db) in [("a", &a), ("b", &b)] {
            let (k, v) = kv(h, i);
            res!(db.insert(k.clone(), v.clone(), Uid::default(), None));
            written.push((k, v));
        }
    }
    res!(a.close());
    res!(b.close());
    verify(&root, &written)
}

/// The forge's pattern: a store held open, a command opening a second handle beside it to write
/// a few records and close, and the first writing on.
fn beside_a_running_store() -> Outcome<Tally> {
    let root = res!(fresh("./test_db_two_handles_beside"));
    let forge = res!(open(&root));
    let mut written = Vec::new();
    for i in 0..10usize {
        let (k, v) = kv("forge", i);
        res!(forge.insert(k.clone(), v.clone(), Uid::default(), None));
        written.push((k, v));
    }
    {
        let cli = res!(open(&root));
        for i in 0..5usize {
            let (k, v) = kv("cli", i);
            res!(cli.insert(k.clone(), v.clone(), Uid::default(), None));
            written.push((k, v));
        }
        res!(cli.close());
    }
    for i in 10..20usize {
        let (k, v) = kv("forge", i);
        res!(forge.insert(k.clone(), v.clone(), Uid::default(), None));
        written.push((k, v));
    }
    res!(forge.close());
    verify(&root, &written)
}

#[derive(Debug, Default)]
struct Tally {
    right:      usize,
    wrong:      usize,          // read back as some other value
    missing:    usize,
    failed:     usize,          // the read itself failed
    first:      Option<String>, // what the first bad read found
}

impl Tally {
    fn lost(&self) -> usize { self.wrong + self.missing + self.failed }
}

impl std::fmt::Display for Tally {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        ok!(write!(f, "{} of {} keys read back right, {} wrong, {} missing, {} failed",
            self.right, self.right + self.lost(), self.wrong, self.missing, self.failed));
        match &self.first {
            Some(first) => write!(f, "; first: {}", first),
            None        => Ok(()),
        }
    }
}

/// Reopens the store alone and reads every key written to it.
fn verify(root: &Path, written: &[(Dat, Dat)]) -> Outcome<Tally> {
    let db = res!(open(root));
    let mut tally = Tally::default();
    for (k, v) in written {
        let bad = match db.get(k, None) {
            Ok(Some((got, _))) if &got == v => {
                tally.right += 1;
                None
            },
            Ok(Some((got, _))) => {
                tally.wrong += 1;
                Some(fmt!("{:?} read back as {:?}", k, got))
            },
            Ok(None) => {
                tally.missing += 1;
                Some(fmt!("{:?} is missing", k))
            },
            Err(e) => {
                tally.failed += 1;
                Some(fmt!("{:?} failed: {:?}", k, e))
            },
        };
        if tally.first.is_none() {
            tally.first = bad;
        }
    }
    res!(db.close());
    Ok(tally)
}

/// Values of differing lengths, so that a record read from another's offset cannot pass for it.
fn kv(handle: &str, i: usize) -> (Dat, Dat) {
    (
        dat!(fmt!("two handles {} {}", handle, i)),
        dat!(fmt!("{}:{}:{}", handle, i, "x".repeat(i % 7 + 3))),
    )
}

fn config() -> Outcome<OzoneConfig> {
    let mut cfg = res!(setup::default_cfg());
    cfg.num_zones           = 1;
    cfg.num_wbots_per_zone  = 1;
    // Large enough that nothing rolls over: every record goes to the one live file.
    cfg.data_file_max_bytes = 1_000_000;
    cfg.zone_overrides      = BTreeMap::new();
    Ok(cfg)
}

fn schemes() -> RestSchemesInput<(), HashScheme, HashScheme, ChecksumScheme> {
    RestSchemesInput::new(
        None::<()>,
        None::<HashScheme>,
        None::<HashScheme>,
        Some(ChecksumScheme::new_crc32()),
    )
}

fn fresh(dir: &str) -> Outcome<PathBuf> {
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    Ok(res!(Path::new(dir).canonicalize()))
}

fn open(root: &Path) -> Outcome<TestDb> {
    let mut db = res!(TestDb::new(root.to_path_buf(), Some(res!(config())), schemes(), Uid::default()));
    res!(db.start("test"));
    Ok(db)
}
