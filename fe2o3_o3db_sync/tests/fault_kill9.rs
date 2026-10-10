//! A process killed with SIGKILL at each step of a barrier and of the stop after a failed one
//! (2026-10-10, QA arm d).  The child writes under the every-write policy, then stops at a
//! `hooks::Halt`, says so through a flag file, and waits to be killed.  On the reopen every
//! confirmed value reads back, the unconfirmed one is never served once the fault record
//! existed, a marker left before the kill refuses the open until it is cleared, and every file
//! the record names ends at or below its mark.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    base::{
        cfg::OzoneConfig,
        constant,
    },
    file::{
        core::FileType,
        refusal::Refusal,
        zdir::ZoneDir,
    },
    test::{
        hooks::{
            self,
            Halt,
        },
        setup::{
            self,
            Uid,
        },
    },
};

use std::{
    collections::BTreeMap,
    fs,
    path::{
        Path,
        PathBuf,
    },
    process::{
        Command,
        Stdio,
    },
    thread,
    time::Duration,
};

const ROOT: &str = "FAULT_KILL9_ROOT";
const HALT: &str = "FAULT_KILL9_HALT";
const FLAG: &str = "FAULT_KILL9_FLAG";
const NKEPT: usize = 10;

const HALTS: [Halt; 5] = [
    Halt::Synced,
    Halt::RecordSynced,
    Halt::Recorded,
    Halt::Quarantined,
    Halt::DataCut,
];

fn cfg() -> Outcome<OzoneConfig> {
    let mut c = res!(setup::default_cfg());
    c.num_zones             = 1;
    c.num_wbots_per_zone    = 1;
    c.zone_overrides        = BTreeMap::new();
    c.sync_on_write         = true;
    c.sync_every_n_writes   = 0;
    c.sync_interval_ms      = 0;
    Ok(c)
}

fn val(seed: u8) -> Dat {
    Dat::BU32((0..100).map(|j| seed ^ (j as u8)).collect())
}

fn kept(i: usize) -> Dat {
    dat!(fmt!("kill9 kept {}", i))
}

fn halt_named(name: &str) -> Outcome<Halt> {
    match HALTS.iter().find(|h| fmt!("{:?}", h) == name) {
        Some(h) => Ok(*h),
        None    => Err(err!("No halt is named {:?}.", name; Test, Invalid)),
    }
}

// Writes the kept values, each confirmed durable, then one more that stops at `halt`: under
// `Synced` its barrier succeeds, otherwise it fails.
fn child(root: PathBuf, halt: Halt, flag: PathBuf) -> Outcome<()> {
    log_set_level!("error");
    let db: TestDb = res!(setup::start_db(root, Some(res!(cfg())), schemes(), None, false, true));
    for i in 0..NKEPT {
        res!(db.insert(kept(i), val(i as u8 + 1), Uid::default(), None));
    }
    hooks::set_halt(Some((halt, flag)));
    if halt != Halt::Synced {
        hooks::set_barrier_failure(true);
    }
    let _ = db.insert(dat!("kill9 last"), val(99), Uid::default(), None);
    // A child that is not killed is a failure of the parent, and ends itself.
    thread::sleep(Duration::from_secs(120));
    std::process::exit(3);
}

// The cuts a fault record names, as (file number, data mark, index mark).
fn record_cuts(root: &Path) -> Outcome<Option<Vec<(u32, u64, u64)>>> {
    let text = match fs::read_to_string(root.join(constant::FAULT_FILENAME)) {
        Ok(text)    => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e)      => return Err(err!(e, "Cannot read the fault record."; Test, IO)),
    };
    let mut cuts = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("cut ") {
            let n: Vec<u64> = rest.split(' ').filter_map(|f| f.parse().ok()).collect();
            if n.len() < 4 {
                return Err(err!("A cut line is short: {:?}", line; Test, Invalid));
            }
            cuts.push((n[1] as u32, n[2], n[3]));
        }
    }
    Ok(Some(cuts))
}

fn kill_at(halt: Halt) -> Outcome<()> {
    let label = fmt!("{:?}", halt);
    let dir = fmt!("./test_db_fault_kill9_{}", label.to_lowercase());
    let _ = fs::remove_dir_all(&dir);
    res!(fs::create_dir_all(&dir));
    let root = res!(Path::new(&dir).canonicalize());
    let flag = root.with_extension("halted");
    let log = root.with_extension("log");
    let _ = fs::remove_file(&flag);

    let out = res!(fs::File::create(&log));
    let err = res!(out.try_clone());
    let mut kid = res!(Command::new(res!(std::env::current_exe()))
        .args(["main", "--exact", "--nocapture", "--test-threads=1"])
        .env(ROOT, &root)
        .env(HALT, &label)
        .env(FLAG, &flag)
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn());
    let halted = wait_until(Duration::from_secs(120), || flag.exists());
    res!(kid.kill());
    let _ = kid.wait();
    assert!(halted, "{}: the child never reached its halt; see {:?}", label, log);

    let recorded = halt != Halt::Synced && halt != Halt::RecordSynced;
    let marked = halt == Halt::Quarantined || halt == Halt::DataCut;
    let cuts = res!(record_cuts(&root));
    assert_eq!(cuts.is_some(), recorded, "{}: the fault record is {} after the kill", label,
        if cuts.is_some() { "there" } else { "missing" });
    assert_eq!(Refusal::path(&root).exists(), marked, "{}: the refusal marker is {} after the kill",
        label, if marked { "missing" } else { "there" });

    let c = res!(cfg());
    let open = || -> Outcome<TestDb> {
        log_set_level!("error");
        setup::start_db(root.clone(), Some(c.clone()), schemes(), None, false, false)
    };
    if marked {
        match open() {
            Ok(db)  => {
                let _ = db.close();
                return Err(err!("{}: the store opened over its refusal marker.", label; Test));
            },
            Err(e)  => assert!(fmt!("{}", e).contains(constant::REFUSAL_FILENAME),
                "{}: the refusal does not name its marker: {}", label, e),
        }
        assert!(res!(Refusal::clear(&root)));
    }
    let db = res!(open());
    for i in 0..NKEPT {
        let got = res!(db.get(&kept(i), None)).map(|(v, _)| v);
        assert_eq!(got, Some(val(i as u8 + 1)), "{}: confirmed key {} did not read back", label, i);
    }
    let last = res!(db.get(&dat!("kill9 last"), None)).map(|(v, _)| v);
    if recorded {
        assert_eq!(last, None, "{}: the unconfirmed value was served after its fault was recorded", label);
    } else {
        assert!(last.is_none() || last == Some(val(99)), "{}: the last key read torn", label);
    }
    res!(db.close());
    assert!(!root.join(constant::FAULT_FILENAME).exists(), "{}: the record outlived the cut", label);
    let zdir = c.zone_root(&root).join("zone_001");
    for (f, d, i) in cuts.unwrap_or_default() {
        let dl = fs::metadata(zdir.join(ZoneDir::relative_file_path(&FileType::Data, f))).map(|m| m.len()).unwrap_or(0);
        let il = fs::metadata(zdir.join(ZoneDir::relative_file_path(&FileType::Index, f))).map(|m| m.len()).unwrap_or(0);
        assert!(dl <= d && il <= i, "{}: file {} is ({}, {}), past its mark ({}, {})", label, f, dl, il, d, i);
    }
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_file(&flag);
    let _ = fs::remove_file(&log);
    Ok(())
}

#[test]
fn main() -> Outcome<()> {
    match (std::env::var(ROOT), std::env::var(HALT), std::env::var(FLAG)) {
        (Ok(root), Ok(halt), Ok(flag)) =>
            child(PathBuf::from(root), res!(halt_named(&halt)), PathBuf::from(flag)),
        _ => {
            for halt in HALTS {
                res!(kill_at(halt));
            }
            Ok(())
        },
    }
}
