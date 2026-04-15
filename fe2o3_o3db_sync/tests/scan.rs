//! Ozone scan integration test.
//!
//! Populates a fresh database with a mix of key prefixes, then
//! exercises [`Database::scan`] through several option shapes
//! (all, prefix, limit, overwrite) and verifies the expected
//! entries come back.

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
    ScanOpts,
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    data::core::RestSchemesInput,
    test::setup,
};

use std::{
    collections::HashSet,
    path::Path,
    thread,
    time::Duration,
};

pub fn test_scan(_filter: &'static str) -> Outcome<()> {

    let db_root = res!(Path::new("./test_db_scan").canonicalize().or_else(|_| {
        std::fs::create_dir_all("./test_db_scan")?;
        Path::new("./test_db_scan").canonicalize()
    }));

    // Fixed key so the test is deterministic.
    let enckey = [0x42u8; 32];
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

    let mut cfg = res!(setup::default_cfg());
    cfg.num_zones            = 3;
    cfg.num_cbots_per_zone   = 2;
    cfg.num_igbots_per_zone  = 2;
    cfg.data_file_max_bytes  = 200_000;
    // Keep every zone inside the per-test db root so this test
    // cannot inherit or contaminate state from sibling tests
    // (basic.rs uses "../test_db_zone_container" for zone 1).
    cfg.zone_overrides       = mapdat!{
        1u16 => mapdat!{ "dir" => "", "max_size" => 10_000_000u64 },
        2u16 => mapdat!{ "dir" => "", "max_size" => 10_000_000u64 },
        3u16 => mapdat!{ "dir" => "", "max_size" => 10_000_000u64 },
    }.get_map().unwrap();

    test!(sync_log::stream(), "+---------------------------------------------+");
    test!(sync_log::stream(), "| SCAN TEST                                   |");
    test!(sync_log::stream(), "+---------------------------------------------+");

    let mut db = res!(setup::start_db(
        db_root.clone(),
        Some(cfg.clone()),
        schms_input.clone(),
        None,
        true,  // gc on
        true,  // wipe
    ));

    thread::sleep(Duration::from_secs(1));

    // 1. Populate with a mix of prefixes.
    //    - 10 under "user:"
    //    - 5 under "sess:"
    //    - 3 under "cfg:"
    test!(sync_log::stream(), "Populating 18 keys across three prefixes.");
    for i in 0..10 {
        res!(db.insert(
            dat!(fmt!("user:{:03}", i)),
            dat!(fmt!("profile_{}", i)),
            user,
            schms2,
        ));
    }
    for i in 0..5 {
        res!(db.insert(
            dat!(fmt!("sess:{:03}", i)),
            dat!(fmt!("token_{}", i)),
            user,
            schms2,
        ));
    }
    for i in 0..3 {
        res!(db.insert(
            dat!(fmt!("cfg:{:03}", i)),
            dat!(fmt!("setting_{}", i)),
            user,
            schms2,
        ));
    }

    thread::sleep(Duration::from_millis(500));

    // 2. Scan everything. 18 entries expected.
    test!(sync_log::stream(), "Scan all: expecting 18 entries.");
    let all = res!(db.scan(&ScanOpts::all(), schms2));
    if all.len() != 18 {
        return Err(err!(
            "Expected 18 entries from scan-all, got {}.", all.len();
            Test, Mismatch));
    }
    let all_keys: HashSet<String> = all.iter()
        .filter_map(|(k, _, _)| match k {
            Dat::Str(s) => Some(s.clone()),
            _ => None,
        })
        .collect();
    if all_keys.len() != 18 {
        return Err(err!(
            "Expected 18 distinct Str keys, got {}.", all_keys.len();
            Test, Mismatch));
    }

    // 3. Prefix scan "user:". 10 expected.
    test!(sync_log::stream(), "Prefix scan 'user:' expecting 10 entries.");
    let user_only = res!(db.scan(
        &ScanOpts::with_str_prefix("user:"),
        schms2,
    ));
    if user_only.len() != 10 {
        return Err(err!(
            "Expected 10 'user:' entries, got {}.", user_only.len();
            Test, Mismatch));
    }
    for (k, _, _) in &user_only {
        match k {
            Dat::Str(s) => {
                if !s.starts_with("user:") {
                    return Err(err!(
                        "Prefix scan returned non-matching key {:?}.", k;
                        Test, Mismatch));
                }
            },
            other => return Err(err!(
                "Expected Dat::Str key, got {:?}.", other;
                Test, Mismatch)),
        }
    }

    // 4. Prefix scan "sess:". 5 expected.
    test!(sync_log::stream(), "Prefix scan 'sess:' expecting 5 entries.");
    let sess_only = res!(db.scan(
        &ScanOpts::with_str_prefix("sess:"),
        schms2,
    ));
    if sess_only.len() != 5 {
        return Err(err!(
            "Expected 5 'sess:' entries, got {}.", sess_only.len();
            Test, Mismatch));
    }

    // 5. Prefix scan "nope:". 0 expected.
    test!(sync_log::stream(), "Prefix scan 'nope:' expecting 0 entries.");
    let nope_only = res!(db.scan(
        &ScanOpts::with_str_prefix("nope:"),
        schms2,
    ));
    if nope_only.len() != 0 {
        return Err(err!(
            "Expected 0 'nope:' entries, got {}.", nope_only.len();
            Test, Mismatch));
    }

    // 6. Limit. 5 expected from a scan-all capped at 5.
    test!(sync_log::stream(), "Scan all with limit=5 expecting 5 entries.");
    let limited = res!(db.scan(
        &ScanOpts::all().limit(5),
        schms2,
    ));
    if limited.len() != 5 {
        return Err(err!(
            "Expected 5 entries under limit=5, got {}.", limited.len();
            Test, Mismatch));
    }

    // 7. Overwrite. Re-insert "user:000" and expect scan still to
    //    return 18 entries total -- one per distinct key -- not 19.
    test!(sync_log::stream(), "Overwrite 'user:000' and rescan; still expecting 18 entries.");
    res!(db.insert(
        dat!("user:000"),
        dat!("profile_0_v2"),
        user,
        schms2,
    ));
    thread::sleep(Duration::from_millis(500));
    let after_overwrite = res!(db.scan(&ScanOpts::all(), schms2));
    if after_overwrite.len() != 18 {
        return Err(err!(
            "Expected 18 entries after overwrite, got {}.", after_overwrite.len();
            Test, Mismatch));
    }

    test!(sync_log::stream(), "Scan test passed.");
    thread::sleep(Duration::from_secs(1));
    res!(db.shutdown());
    Ok(())
}
