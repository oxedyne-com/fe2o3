//! A plain record's period under a long put's chunks (Ozone chunk barriers, QA B A1, 2026-10-12).
//! A store's chunks owe no timed barrier of their own, so under `Interval` a plain record that
//! lands when the period is not yet due is covered by the syncer's timer alone.  A syncer that
//! lags its writer through a long put always finds a chunk waiting, so the timer never runs out,
//! and the plain record waited for the put's own `SyncNow`, seconds later.  The syncer now checks
//! what is owed after each item it takes in.  First seen by QA B as a code reading; the probe
//! became this test.  The hooks are process-wide, so the tests run one at a time.

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
    comm::response::Wait,
    test::{
        hooks,
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
    sync::{
        Mutex,
        MutexGuard,
    },
    thread,
    time::{
        Duration,
        Instant,
    },
};

static HOOKS: Mutex<()> = Mutex::new(());

const PERIOD_MS:    u64 = 200;
const CHUNK:        u64 = 400;
const GIANT:        usize = 600_000;    // 1,500 chunks
const LAG:          Duration = Duration::from_millis(2); // per chunk, so about 3 s of syncer time
const CUT_AFTER:    Duration = Duration::from_millis(800);

fn lock() -> MutexGuard<'static, ()> {
    let g = match HOOKS.lock() { Ok(g) => g, Err(p) => p.into_inner() };
    hooks::set_barrier_delay(Duration::ZERO);
    hooks::set_head_hold(false);
    hooks::set_syncer_chunk_delay(Duration::ZERO);
    g
}

fn cfg() -> Outcome<OzoneConfig> {
    let mut c = res!(setup::default_cfg());
    c.num_zones             = 1;
    c.num_cbots_per_zone    = 2;
    c.num_fbots_per_zone    = 1;
    c.num_wbots_per_zone    = 1;
    c.zone_overrides        = BTreeMap::new();
    c.rest_chunk_threshold  = 300;
    c.rest_chunk_bytes      = CHUNK;
    c.data_file_max_bytes   = 16 * 1_048_576;
    c.sync_on_write         = false;
    c.sync_every_n_writes   = 0;
    c.sync_interval_ms      = PERIOD_MS;
    Ok(c)
}

fn fresh(dir: &str) -> Outcome<PathBuf> {
    let _ = fs::remove_dir_all(dir);
    res!(fs::create_dir_all(dir));
    Ok(res!(Path::new(dir).canonicalize()))
}

fn open(root: &Path, c: &OzoneConfig, wipe: bool) -> Outcome<TestDb> {
    log_set_level!("error");
    setup::start_db(root.to_path_buf(), Some(c.clone()), schemes(), None, false, wipe)
}

fn val(len: usize, seed: u8) -> Dat {
    Dat::BU32((0..len).map(|j| seed ^ (j as u8)).collect())
}

fn wait() -> Wait {
    Wait { max_wait: Duration::from_secs(60), check_interval: constant::CHECK_INTERVAL }
}

// Q lands with the period due and barriers; P follows at once, finds it not due, and is left to
// the timer.  A long put then keeps the syncer fed with chunks, slowed by the lag hook, well past
// P's period.  A cut taken while the put is still in its chunk phase must find P.
#[test]
fn a_plain_records_period_outlasts_a_long_puts_chunks() -> Outcome<()> {
    let _lock = lock();
    let c = res!(cfg());
    let root = res!(fresh("./test_db_chunk_period"));
    let db = res!(open(&root, &c, true));
    res!(db.insert(dat!("warm"), dat!("up"), Uid::default(), None));
    res!(db.api().sync_now(wait()));
    hooks::set_syncer_chunk_delay(LAG);
    thread::sleep(Duration::from_millis(PERIOD_MS + 100));
    res!(db.insert(dat!("plain q"), val(50, 5), Uid::default(), None));
    let k = dat!("plain p");
    let v = val(50, 3);
    res!(db.insert(k.clone(), v.clone(), Uid::default(), None));
    let acked = Instant::now();
    let giant = {
        let db = db.clone();
        thread::spawn(move || db.insert(dat!("giant"), val(GIANT, 9), Uid::default(), None).map(|_| ()))
    };
    thread::sleep(CUT_AFTER);
    let running = !giant.is_finished();
    let cut = res!(fresh("./test_db_chunk_period_cut"));
    let cut_result = instant_cut(&db, &root, &c, &cut);
    let at = acked.elapsed();
    hooks::set_syncer_chunk_delay(Duration::ZERO);
    let giant = match giant.join() { Ok(r) => r, Err(_) => Err(err!("The long put panicked."; Test)) };
    res!(cut_result);
    let got = {
        let db2 = res!(open(&cut, &c, false));
        let got = db2.get(&k, None);
        res!(db2.close());
        got
    };
    let _ = fs::remove_dir_all(&cut);
    res!(giant);
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    if !running {
        return Err(err!(
            "The long put had finished before the cut {} ms after P's ack, so its own SyncNow \
            could have covered P and the test shows nothing.", at.as_millis(); Test));
    }
    match res!(got) {
        Some((d, _)) if d == v => Ok(()),
        Some((d, _)) => Err(err!("P read back as another value at a cut {} ms after its ack: {:?}",
            at.as_millis(), d; Test)),
        None => Err(err!(
            "P, acknowledged under Interval({}), was lost at a cut {} ms after its ack, while a \
            long put's chunks held the syncer.", PERIOD_MS, at.as_millis(); Test)),
    }
}
