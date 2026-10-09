//! A chunked put accepted before a close is stored, or its caller is told it was not.  The server
//! bot sends a chunked value's bunch key only once its chunks are readable, so the close must let
//! the server bots end before it finishes the readers and writers.  Found by Opus QA of A3 round
//! 2a (M2, 2026-10-09): the caller was told `Written`, `close()` returned `Ok`, and the key was
//! gone after reopening.  The hooks are process-wide, hence a binary of its own.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::{
    channels::Recv,
    prelude::*,
};
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    base::cfg::OzoneConfig,
    comm::msg::OzoneMsg,
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
    path::Path,
    thread,
    time::{
        Duration,
        Instant,
    },
};

fn cfg(cbots: u16) -> Outcome<OzoneConfig> {
    let mut c = res!(setup::default_cfg());
    c.num_zones             = 1;
    c.num_cbots_per_zone    = cbots;
    c.num_fbots_per_zone    = 1;
    c.num_wbots_per_zone    = 2;
    c.zone_overrides        = BTreeMap::new();
    c.rest_chunk_threshold  = 1_000;
    c.rest_chunk_bytes      = 400;
    c.sync_on_write         = true;
    Ok(c)
}

fn val(len: usize, seed: u8) -> Dat {
    Dat::BU32((0..len).map(|j| seed ^ (j as u8)).collect())
}

fn start(dir: &str, c: OzoneConfig, wipe: bool) -> Outcome<TestDb> {
    log_set_level!("error");
    if wipe {
        let _ = std::fs::remove_dir_all(dir);
    }
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());
    setup::start_db(root, Some(c), schemes(), None, false, wipe)
}

#[test]
fn chunked_put_in_flight_at_close_is_kept() -> Outcome<()> {
    let dir = "./test_db_close_chunked_put";
    let k = dat!("in flight at close");
    let told: Vec<String>;
    let closed: Outcome<()>;
    {
        let db = res!(start(dir, res!(cfg(4)), true));
        res!(db.insert(key(0), value(0, 1), Uid::default(), None));
        hooks::set_chunk_insert_delay(Duration::from_millis(300));
        let resp = res!(db.api().put(k.clone(), val(1_100, 9), Uid::default(), None));
        thread::sleep(Duration::from_millis(20)); // the server bot has begun the put
        closed = db.close();
        hooks::set_chunk_insert_delay(Duration::ZERO);
        let chan = match resp.channel() {
            Some(c) => c,
            None => return Err(err!("No channel."; Test, Missing)),
        };
        let mut t = Vec::new();
        let end = Instant::now() + Duration::from_secs(8);
        while Instant::now() < end {
            match chan.recv_timeout(Duration::from_millis(100)) {
                Recv::Result(Ok(OzoneMsg::Error(e))) => t.push(fmt!("Error({})", e).chars().take(200).collect()),
                Recv::Result(Ok(m)) => t.push(fmt!("{:?}", m).chars().take(60).collect()),
                _ => (),
            }
        }
        told = t;
    }
    let db = res!(start(dir, res!(cfg(4)), false));
    let got = res!(db.get(&k, None));
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    let kept = matches!(&got, Some((v, _)) if *v == val(1_100, 9));
    msg!("close returned {:?}; kept {}; the caller was told {:?}",
        closed.as_ref().map_err(|e| fmt!("{}", e)), kept, told);
    assert!(kept, "a chunked put accepted before the close was not stored (caller told {:?})", told);
    assert!(closed.is_ok(), "the close failed: {:?}", closed.map_err(|e| fmt!("{}", e)));
    assert!(!told.iter().any(|m| m.starts_with("Error")),
        "the put was stored, but its caller was told an error: {:?}", told);
    Ok(())
}
