//! Regression tests for the deadline a control operation is held to.
//!
//! `OzoneApi::activate_gc` used to wait `constant::USER_REQUEST_WAIT`, six
//! seconds, for every zone bot to acknowledge.  Activation is issued once, at
//! startup, and its message queues behind whatever initialisation the zone bots
//! are still doing, so on a 2.6 GB store none of them answered in time and the
//! database could not be brought up at all.  A user request deadline was being
//! applied to a startup control operation.
//!
//! The zone bots are stood in for here by threads that hold a control message
//! for longer than `USER_REQUEST_TIMEOUT` before acknowledging it, which is the
//! condition the real store produced.  The database itself is not started: the
//! channels are real, the api is real, and only what sits on the far end of them
//! is fabricated, so nothing has to be made big and slow to reproduce a bot that
//! is busy.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::{
    prelude::*,
    channels::Recv,
    rand::RanDef,
};
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_iop_db::api::ScanOpts;
use oxedyne_fe2o3_o3db_sync::{
    api::OzoneApi,
    base::{
        cfg::OzoneConfig,
        constant,
        id::{
            Bid,
            OzoneBotId,
        },
        index::ZoneInd,
    },
    bots::worker::bot::WorkerType,
    comm::{
        channels::BotChannels,
        msg::OzoneMsg,
        response::Wait,
    },
    data::core::RestSchemes,
    test::setup,
};

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{
            AtomicBool,
            Ordering,
        },
    },
    thread,
    time::{
        Duration,
        Instant,
    },
};

type Api = OzoneApi<
    { setup::UID_LEN },
    setup::Uid,
    EncryptionScheme,
    HashScheme,
    HashScheme,
    ChecksumScheme,
>;

type Chans = BotChannels<
    { setup::UID_LEN },
    setup::Uid,
    EncryptionScheme,
    HashScheme,
>;

// How long a stand-in bot holds a message before acknowledging it.  It must exceed
// USER_REQUEST_TIMEOUT, since the whole point is a bot that a user request deadline
// would abandon, and stay well inside CONTROL_REQUEST_TIMEOUT.
const BUSY: Duration = Duration::from_secs(8);
// Ceiling on how long a call bounded by USER_REQUEST_TIMEOUT may take before its
// deadline is not the one it claims.  Below BUSY, so a call that waited for the bot
// cannot pass.
const USER_DEADLINE_CEILING: Duration = Duration::from_secs(7);

#[test]
fn main() -> Outcome<()> {
    log_set_level!("warn");
    let outcome = run();
    log_finish_wait!();
    outcome
}

fn run() -> Outcome<()> {
    res!(busy_zone_bots_do_not_defeat_gc_activation());
    res!(a_request_path_scan_keeps_the_short_deadline());
    res!(a_deliberate_walk_can_ask_for_longer());
    Ok(())
}

/// Channels and an api with no bots behind them, so that the tests can play the bots.
fn harness(nz: u16) -> Outcome<(Api, Chans)> {
    let mut cfg = res!(setup::default_cfg());
    cfg.num_zones           = nz;
    cfg.num_scbots_per_zone = 1;
    cfg.num_wbots_per_zone  = 1;
    // The zone overrides in the shared test config name directories this test never
    // touches, and a stale one would only confuse a failure message.
    cfg.zone_overrides      = OzoneConfig::default().zone_overrides;
    let chans = Chans::new(&cfg);
    let api = Api::new(
        OzoneBotId::Master(Bid::randef()),
        PathBuf::from("."),
        cfg,
        chans.clone(),
        RestSchemes::default(),
    );
    Ok((api, chans))
}

/// A stand-in supervisor and its zone bots: forwards nothing, holds the control message
/// for `BUSY`, then acknowledges once per zone, exactly as `nz` busy zone bots would.
fn busy_supervisor(chans: &Chans, nz: usize, stop: Arc<AtomicBool>) -> thread::JoinHandle<()> {
    let sup = chans.sup().clone();
    thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match sup.recv_timeout(constant::CHECK_INTERVAL) {
                Recv::Empty => continue,
                Recv::Result(Err(_)) => return,
                Recv::Result(Ok(msg)) => {
                    let resp = match msg {
                        OzoneMsg::GcControl(_, resp) => resp,
                        OzoneMsg::NewLiveFile(_, resp) => resp,
                        _ => continue,
                    };
                    thread::sleep(BUSY);
                    for _ in 0..nz {
                        if resp.send(OzoneMsg::Ok).is_err() { return; }
                    }
                },
            }
        }
    })
}

/// A stand-in scan bot for one zone: every walk it is asked for costs `BUSY`, which is
/// what a walk of a large store costs.
fn busy_scan_bot(
    chans:  &Chans,
    z:      usize,
    stop:   Arc<AtomicBool>,
)
    -> Outcome<thread::JoinHandle<()>>
{
    let pool = res!(chans.get_workers_of_type_in_zone(&WorkerType::Scan, &ZoneInd::new(z)));
    let bot = res!(pool.get_bot(0)).clone();
    Ok(thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match bot.recv_timeout(constant::CHECK_INTERVAL) {
                Recv::Empty => continue,
                Recv::Result(Err(_)) => return,
                Recv::Result(Ok(OzoneMsg::ScanRequest { resp, .. })) => {
                    thread::sleep(BUSY);
                    if resp.send(OzoneMsg::ScanEntries(Vec::new())).is_err() { return; }
                },
                Recv::Result(Ok(_)) => continue,
            }
        }
    }))
}

/// Zone bots still busy at startup must not defeat activation, which is what the six
/// second user request deadline made them do.
fn busy_zone_bots_do_not_defeat_gc_activation() -> Outcome<()> {
    let nz = 2usize;
    let (api, chans) = res!(harness(nz as u16));
    let stop = Arc::new(AtomicBool::new(false));
    let handle = busy_supervisor(&chans, nz, stop.clone());

    let t0 = Instant::now();
    let result = api.activate_gc(true);
    let took = t0.elapsed();

    stop.store(true, Ordering::Relaxed);
    let _ = handle.join();

    if let Err(e) = result {
        return Err(err!(e,
            "Activation was defeated by zone bots busy for {:?}, after {:?}.  \
            Activation is a control operation and must wait \
            constant::CONTROL_REQUEST_TIMEOUT ({:?}), not a user request deadline.",
            BUSY, took, constant::CONTROL_REQUEST_TIMEOUT;
            Test));
    }
    // A pass that did not actually outlast the busy period would mean the stand-in bots
    // never held the message, and so would prove nothing.
    if took < BUSY {
        return Err(err!(
            "Activation returned after {:?}, sooner than the {:?} the stand-in zone bots \
            were busy for, so the wait was never tested.", took, BUSY;
            Test, Invalid));
    }
    Ok(())
}

/// A scan is a walk of every index file in every zone, and the short deadline on the
/// request path is what keeps that off it.  Lengthening the control deadline must not
/// have lengthened this one.
fn a_request_path_scan_keeps_the_short_deadline() -> Outcome<()> {
    let nz = 2usize;
    let (api, chans) = res!(harness(nz as u16));
    let stop = Arc::new(AtomicBool::new(false));
    let mut handles = Vec::new();
    for z in 0..nz {
        handles.push(res!(busy_scan_bot(&chans, z, stop.clone())));
    }

    let t0 = Instant::now();
    let result = api.scan(&ScanOpts::all(), None);
    let took = t0.elapsed();

    stop.store(true, Ordering::Relaxed);
    for handle in handles {
        let _ = handle.join();
    }

    let e = match result {
        Err(e) => e,
        Ok(entries) => return Err(err!(
            "A scan served by bots busy for {:?} returned {} entries after {:?}.  \
            The request path deadline, constant::USER_REQUEST_TIMEOUT ({:?}), \
            no longer bounds a scan.",
            BUSY, entries.len(), took, constant::USER_REQUEST_TIMEOUT;
            Test, Invalid)),
    };
    if took > USER_DEADLINE_CEILING {
        return Err(err!(
            "The scan failed, but only after {:?}, beyond the {:?} a call bounded by \
            constant::USER_REQUEST_TIMEOUT ({:?}) may take.",
            took, USER_DEADLINE_CEILING, constant::USER_REQUEST_TIMEOUT;
            Test, Invalid));
    }
    // The failure has to say what it was and what to do about it, since the bare
    // shortfall from the responder cost two hours of wrong diagnoses.
    let text = fmt!("{:?}", e);
    for want in ["scan", "scan_with_wait", "USER_REQUEST_TIMEOUT"] {
        if !text.contains(want) {
            return Err(err!(
                "The scan timeout error does not mention {:?}, so a reader must open \
                api.rs to learn what timed out or what governs it.  It says: {}",
                want, text;
                Test, Missing));
        }
    }
    Ok(())
}

/// A caller that knows it is off the request path names its own deadline, and that
/// deadline, not `USER_REQUEST_WAIT`, is what bounds the walk.
fn a_deliberate_walk_can_ask_for_longer() -> Outcome<()> {
    let nz = 2usize;
    let (api, chans) = res!(harness(nz as u16));
    let stop = Arc::new(AtomicBool::new(false));
    let mut handles = Vec::new();
    for z in 0..nz {
        handles.push(res!(busy_scan_bot(&chans, z, stop.clone())));
    }

    let wait = res!(Wait::new(BUSY + Duration::from_secs(10), constant::CHECK_INTERVAL));
    let t0 = Instant::now();
    let result = api.scan_with_wait(&ScanOpts::all(), None, wait);
    let took = t0.elapsed();

    stop.store(true, Ordering::Relaxed);
    for handle in handles {
        let _ = handle.join();
    }

    if let Err(e) = result {
        return Err(err!(e,
            "A scan given a deadline of its own failed after {:?} against bots busy for \
            {:?}, so scan_with_wait is not honouring the wait it was handed.", took, BUSY;
            Test));
    }
    if took < BUSY {
        return Err(err!(
            "The scan returned after {:?}, sooner than the {:?} the stand-in scan bots \
            were busy for, so the longer deadline was never tested.", took, BUSY;
            Test, Invalid));
    }
    Ok(())
}
