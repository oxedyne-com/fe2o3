use oxedyne_fe2o3_core::{
    prelude::*,
    alt::Override,
    rand::Rand,
    test::test_it,
    time::Timer,
};
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_iop_db::api::{
    Meta,
    RestSchemesOverride,
};
use oxedyne_fe2o3_jdat::{
    prelude::*,
    usr::{
        UsrKind,
        UsrKindCode,
        UsrKindId,
    },
};
use oxedyne_fe2o3_net::id::Uid;
use oxedyne_fe2o3_o3db_sync::{
    base::{
        constant,
        index::ZoneInd,
    },
    comm::{
        response::Wait,
    },
    dal::doc::{
        Doc,
        DocKey,
    },
    data::core::RestSchemesInput,
    file::zdir::ZoneDir,
    test::{
        dbapi,
        file::{
            delete_all_index_files,
            corrupt_an_index_file,
        },
        setup,
    },
};
use oxedyne_fe2o3_test::error::delayed_error;

use std::{
    collections::BTreeMap,
    path::Path,
    thread,
    time::Duration,
};


const wait: Wait = constant::USER_REQUEST_WAIT;

pub fn test_docs(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["Document data abstraction layer 000", "all", "docs"], || {

        let db_root = res!(Path::new("./test_db").canonicalize());

        let mut enckey = [0u8; 32];
        Rand::fill_u8(&mut enckey);
        let aes_gcm = res!(EncryptionScheme::new_aes_256_gcm_with_key(&enckey[..]));
        let _sha3_256 = HashScheme::new_sha3_256();
        let crc32 = ChecksumScheme::new_crc32();
        let schms2: RestSchemesOverride<EncryptionScheme, HashScheme> =
            RestSchemesOverride::default().set_encrypter(Override::Default(aes_gcm.clone()));
        let schms2 = Some(&schms2);
        let user = setup::Uid::default();
        //let meta = Meta::<{ setup::UID_LEN }, setup::Uid>::new(setup::Uid::default());
        let schms_input = RestSchemesInput::new(
            Some(aes_gcm.clone()),
            None::<HashScheme>,
            None::<HashScheme>,
            Some(crc32.clone()),
        );
        //let user1_name = fmt!("Alice83");
        //let user1_email = fmt!("alice83@gmail.com");
        //let user1_pass = fmt!("alice_pass");

        let mut cfg = res!(setup::default_cfg());
        cfg.cache_size_limit_bytes  = 100_000;
        cfg.rest_chunk_threshold    = 700;
        cfg.num_cbots_per_zone      = 2;
        cfg.num_zones               = 3;
        cfg.zone_overrides          = mapdat!{
            1u16    =>  mapdat!{
                "dir"       =>  "../test_db_zone_container",
                "max_size"  =>  100u64,
            },
            3u16    =>  mapdat!{
                "dir"       =>  "",
                "max_size"  =>  100u64,
            },
        }.get_map().unwrap();

        let error_delay = 2;

        test!(sync_log::stream(), "+---------------------------------------------+");
        test!(sync_log::stream(), "| NEW OZONE SESSION                           |");
        test!(sync_log::stream(), "| Wipe all traces of previous test.           |");
        test!(sync_log::stream(), "| Start database.                             |");
        test!(sync_log::stream(), "| Store and retrieve data using the document  |");
        test!(sync_log::stream(), "| Data Abstraction Layer (DAL).               |");
        test!(sync_log::stream(), "| Gracefully shut down the database.          |");
        test!(sync_log::stream(), "+---------------------------------------------+");
        // Wipe all traces of previous test.
        // Start database.
        let mut db = match setup::start_db(
            db_root.clone(),
            Some(cfg.clone()),
            schms_input.clone(),
            Some(fmt!("./test_db_zone_container")),
            false,//true,
            true,
        ) {
            // These pauses on errors are needed to capture tardy messages from asynchronous
            // logging.
            Err(e) => return Err(delayed_error(e, error_delay)),
            Ok(db) => db,
        };

        test!(sync_log::stream(), "Listing files and cache before any user activity...");
        res!(db.api().list_files(wait));
        res!(db.api().dump_file_states(wait));
        res!(db.api().dump_caches(wait));

        let doc = res!(Doc::new_doc("/dir1/dir2",
            mapdat!{
                "field3"    => "val3",
                "field4"    => "val4",
            },
        ));
        let (key, val) = doc.into_dats();

        let resp = db.responder();
        res!(db.api().store_dat_using_responder(
            key,
            val,
            user,
            schms2.clone(),
            resp.clone(),
        ));
        let mut timer = Timer::new();
        res!(resp.recv_all(wait)); // Wait to ensure it was stored.
        debug!(sync_log::stream(), "Storage confirmation delay: {:?}", res!(timer.split_micros()));

        test!(sync_log::stream(), "Listing files and cache after direct insertion...");
        res!(db.api().list_files(wait));
        res!(db.api().dump_file_states(wait));
        res!(db.api().dump_caches(wait));

        thread::sleep(Duration::from_secs(7));

        let doc = res!(Doc::new_doc("/dir1/dir2/dir3",
            mapdat!{
                "field1"    => "val1",
                "dir4"      => res!(DocKey::new_dir("/dir1/dir2/dir3/dir4")).into_dat(),
                "field2"    => "val2",
            },
        ));
        let (key, val) = doc.clone().into_dats();
        let mut timer = Timer::new();
        let resp = res!(db.api().put(key, val, user, None));
        res!(resp.recv_all(wait)); // Wait to ensure it was stored.
        debug!(sync_log::stream(), "Storage confirmation: {:?}", res!(timer.split_micros()));
        
        test!(sync_log::stream(), "Listing files and cache after insertion via server...");
        res!(db.api().list_files(wait));
        res!(db.api().dump_file_states(wait));
        res!(db.api().dump_caches(wait));
        
        let (key, val) = doc.clone().into_dats();
        timer.reset();
        let resp = res!(db.api().put(key, val, user, None));
        res!(resp.recv_all(wait)); // Wait to ensure it was stored.
        debug!(sync_log::stream(), "Storage confirmation: {:?}", res!(timer.split_micros()));
        
        test!(sync_log::stream(), "Listing files and cache after repeat insertion via server...");
        res!(db.api().list_files(wait));
        res!(db.api().dump_file_states(wait));
        res!(db.api().dump_caches(wait));

        ////test!(sync_log::stream(), "Pausing now to verify performance of health check.");
        ////thread::sleep(Duration::from_secs(30));

        ////// Ping all bots.
        ////let missing = res!(db.api().ping_bots2(wait));
        ////debug!(sync_log::stream(), "Ping not received from: {:?}.", missing);

        let (key, val) = doc.clone().into_dats();
        if let Some((dat, _)) = res!(db.api().get_wait(&key, None)) {
            test!(sync_log::stream(), "Yes! It worked! Daticle is: {:?}", dat);
            req!(val, dat, "(L: expected, R: actual)");
        } else {
            return Err(err!(
                "Expected to find key {:?}, but found nothing.", key;
            Data, Missing));
        }

        //test!(sync_log::stream(), "Listing files...");
        //res!(db.api().list_files(wait));
        //res!(db.api().dump_caches(wait));
        test!(sync_log::stream(), "Shutting db down...");
        // Gracefully shut down the database.
        res!(db.shutdown());

        Ok(())
    }));
    

    Ok(())
}
