//! A deleted key must not take the rest of the database with it.
//!
//! A deletion is appended to the data file as a tombstone under the deleted key.  On the next
//! start, with no index to read, the database rebuilds its index by replaying that file from the
//! first byte, and it reads every record the same way: a cache hash, the key, the chunk index, the
//! metadata, a checksum, then the value and its checksum.  A tombstone written to any other shape
//! desynchronises the replay at the point it appears, and every record after it -- however many,
//! however recent -- is silently lost.
//!
//! `basic` already deletes the index files and restarts, but it never deletes a *key* first, so a
//! tombstone never reached the replay under test.  This test writes on both sides of a deletion,
//! forces the rebuild, and insists that everything except the deleted key comes back.

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
use oxedyne_fe2o3_iop_db::api::{
    Database,
    RestSchemesOverride,
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    base::index::ZoneInd,
    data::core::RestSchemesInput,
    file::zdir::ZoneDir,
    test::{
        file::delete_all_index_files,
        setup,
    },
};

use std::{
    collections::BTreeMap,
    path::Path,
    thread,
    time::Duration,
};


/// Keys written before the deletion.
const BEFORE: [&str; 3] = ["before:1", "before:2", "before:3"];
/// The key that is deleted.
const DOOMED: &str = "doomed:1";
/// Keys written after the deletion.  These are the ones a badly framed tombstone loses.
const AFTER: [&str; 3] = ["after:1", "after:2", "after:3"];

pub fn test_delete_replay(_filter: &'static str) -> Outcome<()> {

    let db_root = res!(Path::new("./test_db_delete_replay").canonicalize());

    let mut enckey = [0u8; 32];
    Rand::fill_u8(&mut enckey);
    let aes_gcm = res!(EncryptionScheme::new_aes_256_gcm_with_key(&enckey[..]));
    let crc32 = ChecksumScheme::new_crc32();
    let schms2: RestSchemesOverride<EncryptionScheme, HashScheme> =
        RestSchemesOverride::default().set_encrypter(Override::Default(aes_gcm.clone()));
    let schms2 = Some(&schms2);
    let user = setup::Uid::default();
    let schms_input = RestSchemesInput::new(
        Some(aes_gcm.clone()),
        None::<HashScheme>,
        None::<HashScheme>,
        Some(crc32.clone()),
    );

    let mut cfg = res!(setup::default_cfg());
    // Force every write to stable storage before it is acknowledged, so that what this test
    // measures is the replay of the data file, not the buffering policy in front of it.
    cfg.sync_on_write = true;
    // Keep every zone inside this test's own root.  The default configuration sends zone 1 to a
    // container shared with the other tests, where this test's records would mix with theirs.
    cfg.zone_overrides = DaticleMap::new();
    let zdirs: BTreeMap<ZoneInd, ZoneDir>;

    // ── Session 1: write, delete, write again ────────────────────
    {
        test!(sync_log::stream(), "Session 1: writing on both sides of a deletion.");
        let mut db = res!(setup::start_db(
            db_root.clone(),
            Some(cfg.clone()),
            schms_input.clone(),
            None,
            true,
            true,        // wipe: start from nothing
        ));

        for k in BEFORE {
            res!(db.insert(dat!(k), dat!(fmt!("value of {}", k)), user, schms2));
        }
        res!(db.insert(dat!(DOOMED), dat!("this value is about to be deleted"), user, schms2));

        // The tombstone. Everything written after this point is what a misframed one destroys.
        let was_present = res!(db.delete(&dat!(DOOMED), user, schms2));
        req!(true, was_present);

        for k in AFTER {
            res!(db.insert(dat!(k), dat!(fmt!("value of {}", k)), user, schms2));
        }

        zdirs = res!(db.api().get_zone_dirs());
        res!(db.shutdown());
    }

    thread::sleep(Duration::from_secs(1));

    // Force the rebuild: with no index files, the data file is replayed from the first byte.
    res!(delete_all_index_files(&zdirs));

    // ── Session 2: everything but the deleted key must return ────
    {
        test!(sync_log::stream(), "Session 2: index files removed, so the data file is replayed.");
        let db = res!(setup::start_db(
            db_root.clone(),
            Some(cfg.clone()),
            schms_input.clone(),
            None,
            true,
            false,       // do not wipe: read what session 1 left
        ));

        for k in BEFORE {
            match res!(db.get(&dat!(k), schms2)) {
                Some((v, _)) => req!(dat!(fmt!("value of {}", k)), v),
                None => return Err(err!(
                    "The key {:?} was written before the deletion and is now missing: the \
                    replay of the data file did not reach it.", k;
                    Test, Missing, Data)),
            }
        }

        // The point of the test: a tombstone must not swallow what follows it.
        for k in AFTER {
            match res!(db.get(&dat!(k), schms2)) {
                Some((v, _)) => req!(dat!(fmt!("value of {}", k)), v),
                None => return Err(err!(
                    "The key {:?} was written AFTER the deletion and is now missing. The \
                    tombstone is framed differently from an insertion, so the replay lost its \
                    place at the tombstone and never read the records beyond it.", k;
                    Test, Missing, Data)),
            }
        }

        // And the deleted key must stay deleted.
        if res!(db.get(&dat!(DOOMED), schms2)).is_some() {
            return Err(err!(
                "The key {:?} was deleted, but came back after the rebuild.", DOOMED;
                Test, Invalid, Data));
        }

        res!(db.shutdown());
    }

    Ok(())
}
