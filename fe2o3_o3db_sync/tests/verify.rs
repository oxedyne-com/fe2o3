//! `verify::verify_live_set` over an encrypted store opened as the gateway's tools open it: a
//! clean store reads back whole with the right chunked count and is left byte-identical, and a
//! value with one chunk of another write is named by its key.  `verify::verify_key` on an open
//! store tells a record whose bytes were damaged on disk from a whole one, where a presence lookup
//! reports both held.

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
        KeyCheck,
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
    // as a crash or race between two writes of one key leaves it (A3 QA B1, B2).
    {
        let (db, _) = res!(gateway::open_store(&root, None, &KEY, false, "verify-tear"));
        let api = db.api();
        let resp = api.responder();
        let msgs = res!(api.prepare_write_dat(
            torn.clone(), val(700, 77), Uid::default(), None, resp.clone(), None));
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

#[test]
fn verify_key_finds_a_flipped_byte_that_presence_reports_held() -> Outcome<()> {
    log_set_level!("error");
    let dir = "./test_db_verify_key";
    let _ = fs::remove_dir_all(dir);
    res!(fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());
    let victim = dat!("victim");
    {
        let (db, _) = res!(gateway::open_store(&root, Some(res!(cfg())), &KEY, false, "vk-fill"));
        for i in 0..3u8 {
            res!(db.insert(dat!(fmt!("small {}", i)), val(300, i), Uid::default(), None));
        }
        res!(db.shutdown());
    }
    let before = res!(snapshot(&root));
    {
        let (db, _) = res!(gateway::open_store(&root, None, &KEY, false, "vk-victim"));
        res!(db.insert(victim.clone(), val(300, 42), Uid::default(), None));
        res!(db.shutdown());
    }
    // The victim's record is the tail the last write appended to the file that grew most; one
    // byte well inside it is flipped, as a failing disk would leave it.
    let after = res!(snapshot(&root));
    let mut grown: Option<(PathBuf, usize, usize)> = None;
    for (path, bytes) in &after {
        let old = before.get(path).map(|b| b.len()).unwrap_or(0);
        if bytes.len() > old && grown.as_ref().map_or(true, |(_, o, n)| bytes.len() - old > n - o) {
            grown = Some((path.clone(), old, bytes.len()));
        }
    }
    let (path, old, new) = res!(grown.ok_or_else(|| err!("No file grew with the victim's write."; Missing)));
    assert!(new - old > 300, "the victim's record is {} bytes, not a whole value", new - old);
    let mut bytes = res!(fs::read(&path));
    let at = new - (new - old) / 4;
    bytes[at] ^= 0xff;
    res!(fs::write(&path, &bytes));

    let (db, n) = res!(gateway::open_store(&root, None, &KEY, false, "vk-check"));
    res!(gateway::require_answer(n, "vk-check", &root));
    let held = res!(db.contains_many(&[victim.clone(), dat!("small 1"), dat!("never")], None));
    let damaged = verify::verify_key(db.api(), &victim, None);
    let whole   = verify::verify_key(db.api(), &dat!("small 1"), None);
    let never   = verify::verify_key(db.api(), &dat!("never"), None);
    res!(db.shutdown());
    let _ = fs::remove_dir_all(dir);
    assert_eq!(held, vec![true, true, false], "presence of the victim, a whole key and no key");
    assert!(matches!(damaged, KeyCheck::Failed { chunked: false, .. }),
        "the flipped record read as {:?}", damaged);
    assert_eq!(whole, KeyCheck::Whole { chunked: false });
    assert_eq!(never, KeyCheck::Absent);
    Ok(())
}
