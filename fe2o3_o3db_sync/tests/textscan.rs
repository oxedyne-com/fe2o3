//! The Ozone store text scan (`textscan`) on synthetic, ENCRYPTED Ozone stores, one with planted
//! bad text and one clean, and the gateway's key and store opening (`gateway`).
//!
//! The scan walks the value tree (a list, a map, a box, a byte string) and finds the planted text
//! where it lies; a store without any reports no refusal. The framed-file scan has its own tests
//! in `oxedyne_fe2o3_jdat` (`tests/textscan.rs`).
//!
//! `TEXTSCAN_KEEP=<dir>` keeps the two stores as `<dir>/planted` and `<dir>/clean`, with their
//! key (32 bytes of 0x5a) as `<dir>/key`, to run the `jdat_store_check` binary on COPIES of them.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_jdat::{
    prelude::*,
    string::scan::TextScan,
};
use oxedyne_fe2o3_o3db_sync::{
    base::{
        cfg::OzoneConfig,
        constant,
    },
    comm::response::Wait,
    data::core::RestSchemesInput,
    gateway,
    test::setup,
    textscan,
};

use std::{
    collections::BTreeMap,
    fs,
    path::{
        Path,
        PathBuf,
    },
    thread,
    time::Duration,
};


type Enc = EncryptionScheme;
type Kh  = HashScheme;
type Cs  = ChecksumScheme;

const SCAN_WAIT: Wait = Wait {
    max_wait:       Duration::from_secs(120),
    check_interval: constant::CHECK_INTERVAL,
};

// Words planted in the bad text, which no report may ever repeat.
const SECRET: &str = "s3cret";

#[test]
fn test_an_ozone_store_scan_finds_planted_text_00() -> Outcome<()> {
    log_set_level!("warn");
    let outcome = run_stores();
    log_finish_wait!();
    outcome
}

fn run_stores() -> Outcome<()> {
    let user = setup::Uid::default();

    // The planted store: clean text, text glued to a text, a comma-less comment in a byte string
    // inside a list inside a map, a fraction in a box, a plain string and a number; the map's own key is a string that is not text.
    let planted_root = res!(store_root("planted"));
    let mut inner = BTreeMap::new();
    inner.insert(Dat::Str(fmt!("k")), Dat::BU8(b"[1 # a note\n 2]".to_vec()));
    let rows = vec![
        ("a:1", Dat::Str(fmt!("{{\"ok\": 1}}"))),
        ("a:2", Dat::Str(fmt!("{{\"k\": 1 \"{}\": 2}}", SECRET))),
        ("a:3", Dat::List(vec![Dat::U8(1), Dat::Map(inner)])),
        ("a:4", Dat::Str(fmt!("hello"))),
        ("a:5", Dat::U64(7)),
        ("a:6", Dat::Box(Box::new(Dat::Str(fmt!("(u8|1.5)"))))),
    ];
    res!(write_store(&planted_root, &rows, user));
    let planted = res!(scan_store(&planted_root));
    let got = |class: &str| planted.classes.get(class).copied().unwrap_or(0);
    req!(planted.records, 6);
    req!(planted.texts, 4);
    req!(planted.skipped, 2);
    req!(planted.refused.len(), 3);
    // Every refusal says where, a line and a column, and the report carries both.
    let report = planted.report();
    for r in &planted.refused {
        req!((r.line >= 1 && r.col >= 1), true);
        req!(report.contains(&fmt!("line={} col={}", r.line, r.col)), true);
    }
    req!(got("no separator"), 1);
    req!(got("no separator after a comment"), 1);
    req!(got("whole-number kind with a fraction"), 1);
    req!(planted.is_clean(), false);
    for r in &planted.refused {
        req!((r.store.as_str(), r.at, r.hash.len()), ("o3db", 0, 16));
    }
    for word in [SECRET, "hello", "a:2", "ok", "a note"] {
        if report.contains(word) {
            return Err(err!("The report repeats {:?}: {}", word, report; Test, Data));
        }
    }
    drop_store(&planted_root);

    // The clean store reports no refusal.
    let clean_root = res!(store_root("clean"));
    let rows = vec![
        ("a:1", Dat::Str(fmt!("{{\"ok\": 1}}"))),
        ("a:2", Dat::List(vec![Dat::Str(fmt!("[1, 2]"))])),
        ("a:3", Dat::U64(7)),
    ];
    res!(write_store(&clean_root, &rows, user));
    let clean = res!(scan_store(&clean_root));
    req!((clean.records, clean.texts, clean.refused.len()), (3, 2, 0));
    req!(clean.is_clean(), true);
    drop_store(&clean_root);
    Ok(())
}

// A 32-byte at-rest key, fixed so the test is deterministic; never a real one.
fn test_key() -> [u8; 32] { [0x5au8; 32] }

fn schemes_input() -> Outcome<RestSchemesInput<Enc, Kh, Kh, Cs>> {
    let aes_gcm = res!(EncryptionScheme::new_aes_256_gcm_with_key(&test_key()[..]));
    let crc32 = ChecksumScheme::new_crc32();
    Ok(RestSchemesInput::new(
        Some(aes_gcm),
        None::<HashScheme>,
        None::<HashScheme>,
        Some(crc32),
    ))
}

fn fresh_root(name: &str) -> Outcome<PathBuf> {
    let _ = fs::remove_dir_all(name);
    res!(fs::create_dir_all(name));
    Ok(res!(Path::new(name).canonicalize()))
}

fn base_cfg() -> Outcome<OzoneConfig> {
    let mut cfg = res!(setup::default_cfg());
    cfg.num_zones               = 3;
    cfg.num_cbots_per_zone      = 1;
    cfg.num_fbots_per_zone      = 1;
    cfg.num_wbots_per_zone      = 1;
    cfg.num_igbots_per_zone     = 1;
    cfg.data_file_max_bytes     = 4_000;
    cfg.rest_chunk_threshold    = 500;
    cfg.rest_chunk_bytes        = 128;
    cfg.cache_size_limit_bytes  = 40_000_000;
    cfg.zone_overrides          = BTreeMap::new();
    Ok(cfg)
}

fn write_store(
    root:   &Path,
    rows:   &[(&str, Dat)],
    user:   setup::Uid,
)
    -> Outcome<()>
{
    let db = res!(setup::start_db(
        root.to_path_buf(),
        Some(res!(base_cfg())),
        res!(schemes_input()),
        None,
        false,
        true,
    ));
    thread::sleep(Duration::from_secs(1));
    for (k, v) in rows {
        let resp = res!(db.api().store(Dat::Str(k.to_string()), v.clone(), user));
        res!(resp.recv_store_ack());
    }
    thread::sleep(Duration::from_secs(1));
    res!(db.shutdown());
    thread::sleep(Duration::from_secs(1));
    Ok(())
}

// Opens the store as the examples do, through the gateway: its own configuration, garbage
// collection off.
fn scan_store(root: &Path) -> Outcome<TextScan> {
    let (db, answered) = res!(gateway::open_store(root, None, &test_key(), false, "textscan-test"));
    // A live store answers, and the requirement the scanning tool makes of it is met.
    req!((answered > 0), true);
    req!(res!(gateway::require_answer(answered, "textscan-test", root)), answered);
    thread::sleep(Duration::from_secs(1));
    let mut out = TextScan::default();
    res!(textscan::scan_o3db(db.api(), SCAN_WAIT, &mut out));
    res!(db.shutdown());
    thread::sleep(Duration::from_secs(1));
    Ok(out)
}

// A store's root: under TEXTSCAN_KEEP, with the key file beside it, or a scratch directory.
fn store_root(name: &str) -> Outcome<PathBuf> {
    match std::env::var("TEXTSCAN_KEEP") {
        Ok(keep) => {
            let dir = Path::new(&keep);
            res!(fs::create_dir_all(dir));
            res!(fs::write(dir.join("key"), test_key()));
            fresh_root(&dir.join(name).to_string_lossy())
        },
        Err(_) => fresh_root(&fmt!("./test_db_textscan_{}", name)),
    }
}

// Removes a scratch store, unless the run was asked to keep them.
fn drop_store(root: &Path) {
    if std::env::var("TEXTSCAN_KEEP").is_err() {
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn test_the_gateway_key_is_read_whole_or_refused_00() -> Outcome<()> {
    let dir = res!(fresh_root("./test_gateway_key"));
    let good = dir.join("good");
    let short = dir.join("short");
    res!(fs::write(&good, test_key()));
    res!(fs::write(&short, &test_key()[..31]));
    req!(res!(gateway::load_key(&good)), test_key());
    req!(gateway::load_key(&short).is_err(), true);
    req!(gateway::load_key(&dir.join("absent")).is_err(), true);
    let _ = fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
fn test_a_store_that_gives_no_answer_is_refused_by_the_requirement_alone_00() -> Outcome<()> {
    // `open_store` hands the store back whatever the count, as it did before the helper, so that
    // `o3db_migrate` and `o3db_sweep` carry on. Only the tool that reports on the store asks for
    // an answer. No store gives a zero count to open (with every bot dead the ping itself fails,
    // and a configuration with no bot divides by zero at start), so the zero is driven here
    // through the requirement.
    let root = Path::new("/nowhere");
    let e = match gateway::require_answer(0, "scan", root) {
        Ok(n) => return Err(err!("A count of 0 should be refused, and gave {}", n; Test, Data)),
        Err(e) => e,
    };
    if !e.plain().contains("no bot of the store") {
        return Err(err!("The refusal should say that no bot answered: {}", e.plain(); Test, Data));
    }
    req!(res!(gateway::require_answer(28, "scan", root)), 28);
    req!(res!(gateway::require_answer(1, "scan", root)), 1);
    Ok(())
}
