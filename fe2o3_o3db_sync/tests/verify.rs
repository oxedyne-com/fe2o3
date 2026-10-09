//! `verify::verify_live_set` over an encrypted store opened as the gateway's tools open it: a
//! clean store reads back whole with the right chunked count and is left byte-identical, and a
//! value with one chunk of another write is named by its key.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    base::{
        cfg::OzoneConfig,
        constant,
    },
    comm::response::Wait,
    gateway::{
        self,
        Uid,
    },
    test::setup,
    verify::{
        self,
        VerifyReport,
    },
};

use std::{
    collections::BTreeMap,
    fs,
    path::{
        Path,
        PathBuf,
    },
    time::Duration,
};

// A throwaway store's key; a real one is only ever read from a file at runtime.
const KEY: [u8; 32] = [0x3cu8; 32];

fn cfg() -> Outcome<OzoneConfig> {
    let mut c = res!(setup::default_cfg());
    c.num_zones             = 2;
    c.num_cbots_per_zone    = 1;
    c.num_fbots_per_zone    = 1;
    c.num_wbots_per_zone    = 1;
    c.num_igbots_per_zone   = 1;
    c.rest_chunk_threshold  = 500;
    c.rest_chunk_bytes      = 128;
    c.zone_overrides        = BTreeMap::new();
    Ok(c)
}

fn wait() -> Wait {
    Wait { max_wait: Duration::from_secs(120), check_interval: constant::CHECK_INTERVAL }
}

fn val(len: usize, seed: u8) -> Dat {
    Dat::BU32((0..len).map(|j| seed ^ (j as u8)).collect())
}

fn snapshot(root: &Path) -> Outcome<BTreeMap<PathBuf, Vec<u8>>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in res!(fs::read_dir(&dir)) {
            let entry = res!(entry);
            let path = entry.path();
            let meta = res!(entry.metadata());
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                out.insert(path.clone(), res!(fs::read(&path)));
            }
        }
    }
    Ok(out)
}

// Opens the store at `root` as `o3db_verify` does, checks it and closes it.
fn check(root: &Path) -> Outcome<VerifyReport> {
    let (db, n) = res!(gateway::open_store(root, None, &KEY, false, "verify-test"));
    res!(gateway::require_answer(n, "verify-test", root));
    let rep = verify::verify_live_set(db.api(), None, wait());
    res!(db.shutdown());
    rep
}

#[test]
fn verify_finds_a_torn_value_and_writes_nothing() -> Outcome<()> {
    log_set_level!("error");
    let dir = "./test_db_verify";
    let _ = fs::remove_dir_all(dir);
    res!(fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());
    let torn = dat!("chunked 1");
    {
        let (db, _) = res!(gateway::open_store(&root, Some(res!(cfg())), &KEY, false, "verify-fill"));
        for i in 0..5u8 {
            res!(db.insert(dat!(fmt!("small {}", i)), val(40, i), Uid::default(), None));
        }
        for i in 0..3u8 {
            res!(db.insert(dat!(fmt!("chunked {}", i)), val(700, i), Uid::default(), None));
        }
        res!(db.insert(dat!("gone"), val(40, 9), Uid::default(), None));
        res!(db.delete(&dat!("gone"), Uid::default(), None));
        res!(db.shutdown());
    }

    // Clean: every key reads, the chunked ones are counted, and not a byte changes.
    let before = res!(snapshot(&root));
    let rep = res!(check(&root));
    let after = res!(snapshot(&root));
    msg!("clean store: {}", rep.summary(80));
    assert!(rep.clean(), "a clean store reported failures: {:?}", rep.failures);
    assert_eq!((rep.keys, rep.present, rep.absent, rep.chunked), (9, 8, 1, 3));
    assert_eq!(before.keys().collect::<Vec<_>>(), after.keys().collect::<Vec<_>>(),
        "the check added or removed a file");
    for (path, bytes) in &before {
        assert!(after.get(path) == Some(bytes), "the check changed {:?}", path);
    }

    // Torn: the first chunk of another value of the same length lands under the same chunk key,
    // as a crash or race between two writes of one key left it while every write of a key used
    // one chunk set (A3 QA B1, B2).  Writes have sets of their own now, so the set is forced.
    {
        let (db, _) = res!(gateway::open_store(&root, None, &KEY, false, "verify-tear"));
        let api = db.api();
        let set = match res!(api.get_head_wait(&torn, None)) {
            Some((Dat::Tup5u64(t), _)) => t[0],
            other => return Err(err!("{:?} is not chunked: {:?}", torn, other; Test, Invalid)),
        };
        let resp = api.responder();
        let msgs = res!(api.prepare_write_dat(
            torn.clone(), val(700, 77), Uid::default(), None, resp.clone(), Some(set)));
        assert!(msgs.len() > 2, "the value was not chunked");
        let chunk1 = msgs.into_iter().nth(1).into_iter().collect::<Vec<_>>();
        res!(api.store_bytes(chunk1));
        let _ = resp.recv_timeout(Duration::from_secs(10)); // written, then shut down after it
        res!(db.shutdown());
    }
    let rep = res!(check(&root));
    msg!("torn store: {}", rep.summary(80));
    let _ = fs::remove_dir_all(dir);
    assert_eq!(rep.failures.len(), 1, "expected one failure: {:?}", rep.failures);
    assert_eq!(rep.failures[0].key, torn);
    assert!(rep.failures[0].chunked);
    assert_eq!((rep.keys, rep.present, rep.chunked), (9, 7, 3));
    Ok(())
}
