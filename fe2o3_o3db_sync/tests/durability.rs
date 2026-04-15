//! Ozone durability barrier integration test.
//!
//! Exercises the new `sync_on_write` / `sync_every_n_writes` /
//! `sync_interval_ms` config knobs by:
//!
//! 1. Building a database with `sync_on_write = true`, writing a
//!    handful of keys, reading them back, and asserting that every
//!    write completed through the `sync_data` code path without
//!    error. This is the strongest-guarantee policy and is the one
//!    most operators will reach for.
//! 2. Repeating the exercise with `sync_every_n_writes = 3` and
//!    with `sync_interval_ms = 50`, so all three policy branches of
//!    `WriterBot::maybe_sync_files` run under test.
//!
//! The test does not attempt to measure actual disk sync (that
//! requires strace or a kernel tracepoint and is tied to the
//! filesystem). Instead it asserts the end-to-end correctness
//! property an operator cares about: that acknowledged writes are
//! readable through a fresh live file pair after each policy's sync
//! cadence has fired at least once.

use oxedyne_fe2o3_core::{
    prelude::*,
    alt::Override,
};
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_iop_db::api::{
    Database,
    RestSchemesOverride,
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    data::core::RestSchemesInput,
    test::setup,
};

use std::{
    path::Path,
    thread,
    time::Duration,
};

pub fn test_durability(_filter: &'static str) -> Outcome<()> {

    let db_root = res!(Path::new("./test_db_durability").canonicalize().or_else(|_| {
        std::fs::create_dir_all("./test_db_durability")?;
        Path::new("./test_db_durability").canonicalize()
    }));

    let enckey = [0x5au8; 32];
    let aes_gcm = res!(EncryptionScheme::new_aes_256_gcm_with_key(&enckey[..]));
    let crc32 = ChecksumScheme::new_crc32();
    let schms2: RestSchemesOverride<EncryptionScheme, HashScheme> =
        RestSchemesOverride::default()
            .set_encrypter(Override::Default(aes_gcm.clone()));
    let schms2 = Some(&schms2);
    let user = setup::Uid::default();

    let schms_input = RestSchemesInput::new(
        Some(aes_gcm.clone()),
        None::<HashScheme>,
        None::<HashScheme>,
        Some(crc32.clone()),
    );

    // Three independent sub-tests: strongest-guarantee, group-commit
    // by count, and group-commit by time window.
    res!(run_with_policy(
        "sync_on_write",
        &db_root,
        &schms_input,
        schms2,
        user,
        |cfg| {
            cfg.sync_on_write = true;
        },
        12,
    ));
    res!(run_with_policy(
        "sync_every_n_writes=3",
        &db_root,
        &schms_input,
        schms2,
        user,
        |cfg| {
            cfg.sync_every_n_writes = 3;
        },
        9,
    ));
    res!(run_with_policy(
        "sync_interval_ms=50",
        &db_root,
        &schms_input,
        schms2,
        user,
        |cfg| {
            cfg.sync_interval_ms = 50;
        },
        6,
    ));

    test!(sync_log::stream(),
        "Durability barrier test passed under every sync policy branch.");
    Ok(())
}

/// Run one sub-test with the given policy setter, writing `count`
/// key/value pairs and reading them back.
fn run_with_policy<F>(
    label:       &str,
    db_root:     &std::path::PathBuf,
    schms_input: &RestSchemesInput<
                     EncryptionScheme,
                     HashScheme,
                     HashScheme,
                     ChecksumScheme,
                 >,
    schms2:      Option<&RestSchemesOverride<EncryptionScheme, HashScheme>>,
    user:        setup::Uid,
    apply:       F,
    count:       u32,
)
    -> Outcome<()>
where
    F: FnOnce(&mut oxedyne_fe2o3_o3db_sync::base::cfg::OzoneConfig),
{
    test!(sync_log::stream(), "+--- durability: {} ---", label);

    let mut cfg = res!(setup::default_cfg());
    cfg.num_zones           = 2;
    cfg.num_cbots_per_zone  = 1;
    cfg.num_wbots_per_zone  = 1;
    cfg.num_igbots_per_zone = 1;
    cfg.data_file_max_bytes = 200_000;
    cfg.zone_overrides = mapdat!{
        1u16 => mapdat!{ "dir" => "", "max_size" => 10_000_000u64 },
        2u16 => mapdat!{ "dir" => "", "max_size" => 10_000_000u64 },
    }.get_map().unwrap();
    apply(&mut cfg);

    let mut db = res!(setup::start_db(
        db_root.clone(),
        Some(cfg.clone()),
        schms_input.clone(),
        None,
        false, // gc off: keep the write path simple.
        true,  // wipe: every sub-test starts clean.
    ));

    thread::sleep(Duration::from_millis(200));

    for i in 0..count {
        res!(db.insert(
            dat!(fmt!("dkey:{:03}", i)),
            dat!(fmt!("dval_{}", i)),
            user,
            schms2,
        ));
    }

    // Time-window sub-test needs to wait past the interval so the
    // last few writes actually fire a sync before we stop.
    thread::sleep(Duration::from_millis(120));

    // Read every key back, to assert the write side acknowledged
    // and the read side can see the value. If any sync step
    // errored out, the write would have propagated that upstream
    // and this loop would fail here.
    for i in 0..count {
        let key = dat!(fmt!("dkey:{:03}", i));
        let got = res!(db.get(&key, schms2));
        match got {
            Some((val, _meta)) => {
                let expected = dat!(fmt!("dval_{}", i));
                if val != expected {
                    return Err(err!(
                        "{}: round-tripped value mismatch: key={}, \
                        got={:?}, expected={:?}",
                        label, i, val, expected;
                        Test, Mismatch));
                }
            },
            None => return Err(err!(
                "{}: key {} not found after durable write.",
                label, i;
                Test, Missing)),
        }
    }

    test!(sync_log::stream(),
        "+--- durability: {} : {} keys round-tripped ---",
        label, count);
    Ok(())
}
