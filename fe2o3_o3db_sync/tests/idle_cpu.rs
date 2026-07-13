//! Regression test for the idle busy-poll in the Ozone server bot.
//!
//! The `ServerBot` used to wait on its internal channel with a one
//! microsecond `recv_timeout`, so an idle database woke roughly a
//! million times a second per server bot and burned around a fifth of
//! a CPU core doing nothing.  The fix makes the bot block on `recv`
//! instead.  This test starts a database, lets it fall completely
//! idle, and asserts that the whole process consumes only a trivial
//! amount of CPU time over the measurement window.

use oxedyne_fe2o3_core::{
    prelude::*,
    alt::Override,
    rand::Rand,
};
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_iop_db::api::RestSchemesOverride;
use oxedyne_fe2o3_o3db_sync::{
    data::core::RestSchemesInput,
    test::setup,
};
use oxedyne_fe2o3_sys::proc_self::ProcSelf;

use std::{
    path::Path,
    thread,
    time::Duration,
};

#[test]
fn main() -> Outcome<()> {
    log_set_level!("warn");
    let outcome = run();
    log_finish_wait!();
    outcome
}

fn run() -> Outcome<()> {

    // Kernel clock tick rate assumed by /proc accounting on Linux.
    // One tick is therefore 10 ms of CPU time.
    const TICKS_PER_SEC:    u64 = 100;
    // Length of the idle observation window.
    const IDLE_SECS:        u64 = 4;
    // Ceiling on the fraction of a single core the idle database may
    // consume.  The old one microsecond poll burned tens of percent of
    // a core; blocking receives sit far below one percent.  Five
    // percent leaves generous headroom for incidental wake-ups (async
    // logging, the one second zone state updates) while still failing
    // hard if the microsecond spin ever returns.
    const MAX_CORE_FRACTION: f64 = 0.05;

    let db_dir = Path::new("./test_db_idle_cpu");
    res!(std::fs::create_dir_all(db_dir));
    let db_root = res!(db_dir.canonicalize());

    let mut enckey = [0u8; 32];
    Rand::fill_u8(&mut enckey);
    let aes_gcm = res!(EncryptionScheme::new_aes_256_gcm_with_key(&enckey[..]));
    let crc32 = ChecksumScheme::new_crc32();
    let _schms2: RestSchemesOverride<EncryptionScheme, HashScheme> =
        RestSchemesOverride::default().set_encrypter(Override::Default(aes_gcm.clone()));
    let schms_input = RestSchemesInput::new(
        Some(aes_gcm.clone()),
        None::<HashScheme>,
        None::<HashScheme>,
        Some(crc32.clone()),
    );

    let cfg = res!(setup::default_cfg());

    let db = res!(setup::start_db(
        db_root.clone(),
        Some(cfg),
        schms_input,
        Some(fmt!("./test_db_idle_cpu_zone_container")),
        false,
        true,
    ));

    // Let start-up churn settle before measuring.
    thread::sleep(Duration::from_secs(1));

    let ticks_before = res!(ProcSelf::cpu_ticks());
    thread::sleep(Duration::from_secs(IDLE_SECS));
    let ticks_after = res!(ProcSelf::cpu_ticks());

    res!(db.shutdown());

    let ticks_used = ticks_after.saturating_sub(ticks_before);
    // CPU seconds consumed during the idle window.
    let cpu_secs = ticks_used as f64 / TICKS_PER_SEC as f64;
    // Fraction of one core: idle CPU seconds over wall-clock seconds.
    let core_fraction = cpu_secs / IDLE_SECS as f64;

    test!(sync_log::stream(),
        "Idle database consumed {} CPU ticks ({:.3} s) over {} s: {:.2}% of one core.",
        ticks_used, cpu_secs, IDLE_SECS, core_fraction * 100.0);

    if core_fraction > MAX_CORE_FRACTION {
        return Err(err!(
            "Idle database burned {:.2}% of a core, exceeding the {:.2}% ceiling; \
            the server bot is likely busy-polling again.",
            core_fraction * 100.0, MAX_CORE_FRACTION * 100.0;
            Test, Excessive));
    }

    Ok(())
}
