//! A value too large for one record must come back as the value that was stored.
//!
//! A value whose encoding exceeds the chunk size is not written whole: it is split, each chunk is
//! stored under its own part key, and the key the caller used holds a `Tup5u64` part key pointing
//! at them.  On the way back out, `fetch_chunks` collects the chunks, rejoins the bytes, decrypts
//! them and decodes them -- and hands back the caller's original `Dat`, fully formed.
//!
//! `get_wait` then treated that `Dat` as though it were still raw bytes and tried to decode it a
//! second time, so a chunked value only survived the round trip if it happened to be a byte string
//! -- and even then it came back wrong, its payload decoded as though it were an encoding.  Every
//! other kind fell through to a catch-all and returned `Unexpected Dat ... returned`.
//!
//! Nothing in the suite stored a value large enough to be chunked, so the read path was never run.
//! The values that grow past the threshold in practice are the accumulating ones -- a ledger, an
//! append-only list, a document -- which is to say the ones a caller least expects to lose.  This
//! test stores each of the kinds a caller actually stores, at a size that forces chunking, and
//! insists on getting back what it put in.

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
use oxedyne_fe2o3_jdat::{
    prelude::*,
    file::JdatMapFile,
};
use oxedyne_fe2o3_o3db_sync::{
    base::cfg::OzoneConfig,
    data::core::RestSchemesInput,
    test::setup,
};

use std::{
    path::Path,
    thread,
    time::Duration,
};


/// Enough records that the encoded list comfortably exceeds any chunk size the database is
/// configured with, so the write path is certain to have split it.
const RECORDS: usize = 400;


/// A list of maps, which is the shape of every accumulating value a caller keeps under one key:
/// a ledger, an audit log, a list of bindings.
fn ledger() -> Dat {
    let mut out = Vec::with_capacity(RECORDS);
    for i in 0..RECORDS {
        out.push(create_dat_ordmap(vec![
            (dat!("account_id"),   dat!(fmt!("acct_{:08}", i))),
            (dat!("kind"),         dat!(if i == 0 { "topup" } else { "spend" })),
            (dat!("delta_minor"),  Dat::I64(if i == 0 { 5_000 } else { -1 })),
            (dat!("ref"),          dat!(fmt!("mail:someone{}@example.com", i))),
        ]));
    }
    Dat::List(out)
}

/// A long string, the other everyday large value: a document, a brief, a blob of text.
fn document() -> Dat {
    let mut s = String::new();
    for i in 0..RECORDS {
        s.push_str(&fmt!("line {} of a document long enough to be split across chunks.\n", i));
    }
    dat!(s)
}

/// A large byte string.  This is the one kind the old code did not reject outright -- it decoded
/// the payload a second time, as though the bytes were themselves an encoding -- so it came back
/// as something else entirely, or failed.  Silent corruption is worse than the error, and this
/// insists on the bytes.
fn blob() -> Dat {
    let mut v = vec![0u8; 64 * 1024];
    Rand::fill_u8(&mut v);
    Dat::BU32(v)
}

pub fn test_chunked_value(_filter: &'static str) -> Outcome<()> {

    let db_root = res!(Path::new("./test_db_chunked_value").canonicalize());

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
    cfg.sync_on_write = true;
    cfg.zone_overrides = DaticleMap::new();

    let cases: Vec<(&str, Dat)> = vec![
        ("chunked:ledger",   ledger()),
        ("chunked:document", document()),
        ("chunked:blob",     blob()),
    ];

    // ── Session 1: store values large enough to be chunked ───────
    {
        test!(sync_log::stream(), "Session 1: storing values that exceed the chunk size.");
        let mut db = res!(setup::start_db(
            db_root.clone(),
            Some(cfg.clone()),
            schms_input.clone(),
            None,
            true,
            true,        // wipe: start from nothing
        ));

        for (k, v) in &cases {
            res!(db.insert(dat!(*k), v.clone(), user, schms2));
        }

        // Read them back in the same session, before any restart: the fault is in the read path,
        // not in what reaches the disk, so it shows up immediately.
        for (k, v) in &cases {
            match res!(db.get(&dat!(*k), schms2)) {
                Some((got, _)) => if got != *v {
                    return Err(err!(
                        "The value stored under {:?} did not survive the round trip. A value \
                        larger than the chunk size is split on the way in and rejoined on the \
                        way out, and what came back is not what went in.", k;
                        Test, Invalid, Data));
                },
                None => return Err(err!(
                    "The key {:?} was just written and cannot be read back.", k;
                    Test, Missing, Data)),
            }
        }

        res!(db.shutdown());
    }

    thread::sleep(Duration::from_secs(1));

    // ── Session 2: and they must survive a restart ───────────────
    {
        test!(sync_log::stream(), "Session 2: the chunked values must still read back.");
        let db = res!(setup::start_db(
            db_root.clone(),
            Some(cfg.clone()),
            schms_input.clone(),
            None,
            true,
            false,       // do not wipe: read what session 1 left
        ));

        for (k, v) in &cases {
            match res!(db.get(&dat!(*k), schms2)) {
                Some((got, _)) => if got != *v {
                    return Err(err!(
                        "After a restart, the value under {:?} came back changed.", k;
                        Test, Invalid, Data));
                },
                None => return Err(err!(
                    "After a restart, the chunked value under {:?} is gone.", k;
                    Test, Missing, Data)),
            }
        }

        res!(db.shutdown());
    }

    // ── Session 3: the chunk configuration changes underneath them ──
    //
    // An operator raising a store's chunking threshold -- which is what a store carrying a
    // too-small one needs -- must not thereby orphan everything already written under the old
    // one. It does not, and this is the reason: a chunked value's geometry (how many chunks, how
    // big) travels in the part key stored with the value, so the reader reconstructs it from what
    // it finds, never from what the configuration currently says. The claim is worth a test
    // rather than an assurance, because a wrong answer here is an unreadable production store.
    {
        test!(sync_log::stream(), "Session 3: the chunk configuration is raised on an \
            existing store; what was written under the old one must still read.");

        // Change the chunk geometry the store writes under. The values already on disk were split
        // into 64-byte chunks; from now on the store splits into much larger ones, so anything
        // read back that took its geometry from the configuration rather than from the value
        // would be reassembled at the wrong stride and come back as rubbish.
        //
        // (The threshold is raised only as far as the store's own validation allows -- it must
        // stay under 80% of the data file size, which these test files make small. That check is
        // the same one that will refuse an ill-judged production setting.)
        let cfg_path = OzoneConfig::config_path(&db_root);
        let mut stored = res!(<OzoneConfig as JdatMapFile>::load(&cfg_path));
        stored.rest_chunk_bytes = 500;
        res!(stored.save(&cfg_path, "  ", true));

        let db = res!(setup::start_db(
            db_root.clone(),
            Some(cfg.clone()),
            schms_input.clone(),
            None,
            true,
            false,       // do not wipe: read what the earlier sessions left
        ));

        for (k, v) in &cases {
            match res!(db.get(&dat!(*k), schms2)) {
                Some((got, _)) => if got != *v {
                    return Err(err!(
                        "The value under {:?} was chunked under one configuration and read back \
                        under another, and it came back changed. A value's chunk geometry must \
                        come from the part key stored with it, not from the current settings.", k;
                        Test, Invalid, Data));
                },
                None => return Err(err!(
                    "Changing the chunk geometry lost the value under {:?}, which was written \
                    under the old one.", k;
                    Test, Missing, Data)),
            }
        }

        // And a value written under the NEW geometry reads back too, so the two coexist in one
        // store rather than the store having to be all of one or all of the other.
        let mut db = db;
        let fresh = ledger();
        res!(db.insert(dat!("chunked:after"), fresh.clone(), user, schms2));
        match res!(db.get(&dat!("chunked:after"), schms2)) {
            Some((got, _)) => if got != fresh {
                return Err(err!(
                    "A value written under the new chunk geometry did not survive the round \
                    trip."; Test, Invalid, Data));
            },
            None => return Err(err!(
                "A value written under the new chunk geometry cannot be read back.";
                Test, Missing, Data)),
        }

        res!(db.shutdown());
    }

    Ok(())
}
