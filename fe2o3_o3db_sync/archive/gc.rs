use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync2::{
    base::{
        cfg_db::ConfigInit,
    },
    data::{
        cache::Meta,
        core::StoreSpec,
    },
    test::{
        data::{
            find_unique,
        },
        dbapi::{
            store,
            fetch,
        },
        file::save,
        setup::{
            default_cfg,
            start_db,
        },
    },
};
use oxedyne_fe2o3_test::{
    data::{
        DataArrangement,
        DataFill,
        DataSize,
        DataSpec,
    },
    error::delayed_error,
};

use std::{
    path::PathBuf,
    thread,
    time::Duration,
};

use rand_core::{
    RngCore,
    OsRng,
};

//#[test]
//fn test_gc() -> Result<()> {
//
//    let root_path = fmt!("./test_db");
//    let mut enckey = [0u8; 32];
//    OsRng.fill_bytes(&mut enckey);
//    let aes_gcm = EncryptionScheme::new_aes_gcm_256(&enckey);
//    let store_spec = StoreSpec::new().set_encrypter(Some(aes_gcm));
//    //let store_spec = StoreSpec::new();
//    let mut meta: Meta;
//    let user1_name = fmt!("Alice83");
//    let user1_email = fmt!("alice83@gmail.com");
//    let user1_pass = fmt!("alice_pass");
//
//    let error_delay = 2;
//
//    {
//        for case in &[
//            0,
//            50,
//            100,
//            200,
//            300,
//        ] {
//            let (note, cfg, keyspec, valsize) = match case {
//                0 => {
//                    let mut cfg = res!(default_cfg(&root_path));
//                    cfg.bytes_before_chunking = 500;
//                    cfg.num_zones = 1;
//                    (
//                        fmt!("Small sample, all-unique keys."),
//                        cfg,
//                        DataSpec {
//                            size:   DataSize::Const(10),
//                            fill:   DataFill::Random,
//                            arr:    DataArrangement::PlainFill(15),
//                        },
//                        DataSize::Const(10),
//                    )
//                },
//                50 => {
//                    let mut cfg = res!(default_cfg(&root_path));
//                    cfg.cache_size_limit_bytes = 3_000;
//                    cfg.bytes_before_chunking = 500;
//                    cfg.num_zones = 1;
//                    (
//                        fmt!("Small sample, all-unique keys, with chunking and cache size management."),
//                        cfg,
//                        DataSpec {
//                            size:   DataSize::Const(10),
//                            fill:   DataFill::Random,
//                            arr:    DataArrangement::PlainFill(3),
//                        },
//                        DataSize::Const(600),
//                    )
//                },
//                100 => {
//                    let mut cfg = res!(default_cfg(&root_path));
//                    cfg.bytes_before_chunking = 500;
//                    (
//                        fmt!("The same key repeated."),
//                        cfg,
//                        DataSpec {
//                            size:   DataSize::Const(10),
//                            fill:   DataFill::Const(42),
//                            arr:    DataArrangement::PlainFill(500),
//                        },
//                        DataSize::Const(10),
//                    )
//                },
//                200 => {
//                    let mut cfg = res!(default_cfg(&root_path));
//                    cfg.bytes_before_chunking = 500;
//                    cfg.num_zones = 2;
//                    (
//                        fmt!("Generate lots of files with no gc."),
//                        cfg,
//                        DataSpec {
//                            size:   DataSize::Const(10),
//                            fill:   DataFill::Random,
//                            arr:    DataArrangement::PlainFill(500),
//                        },
//                        DataSize::Const(600),
//                    )
//                },
//                300 => {
//                    let mut cfg = res!(default_cfg(&root_path));
//                    cfg.cache_size_limit_bytes = 20_000_000;
//                    cfg.data_file_max_bytes = 1_000_000;
//                    cfg.bytes_before_chunking = 1_024;
//                    cfg.chunk_byte_size = 1_024;
//                    (
//                        fmt!("Performance run with 2 zones."),
//                        cfg,
//                        DataSpec {
//                            size:   DataSize::RandUniform { lo: 30, hi: 3000 },
//                            fill:   DataFill::Random,
//                            arr:    DataArrangement::RepeatFillAndSeq{
//                                n: 60,
//                                rep: 100,
//                                specbox: Box::new(
//                                    DataSpec {
//                                        size:   DataSize::RandUniform { lo: 30, hi: 3000 },
//                                        fill:   DataFill::Random,
//                                        arr:    DataArrangement::PlainFill(15),
//                                    }
//                                ),
//                            },
//                        },
//                        DataSize::RandUniform { lo: 30, hi: 10_000 },
//                    )
//                },
//                _ => unimplemented!(),
//            };
//            msg!("+---------------------------------------------+");
//            msg!("| NEW OZONE SESSION                           |");
//            msg!("| Wipe all traces of previous test.           |");
//            msg!("| Start database.                             |");
//            msg!("| Create a new user.                          |");
//            msg!("| Store and fetch some simple data.           |");
//            msg!("| Gracefully shut down the database.          |");
//            msg!("+---------------------------------------------+");
//            let mut db = match start_db(
//                ConfigInit::Data(cfg.clone()),
//                &root_path,
//                fmt!("./test_db_zone_container"),
//                true,
//            ) {
//                // These pauses on errors are needed to capture tardy messages from logging to help
//                // with debugging.
//                Err(e) => return Err(delayed_error(e, error_delay)),
//                Ok(db) => db,
//            };
//
//            let user_id = match db.create_user(
//                user1_name.clone(),
//                vec![user1_email.clone()],
//                user1_pass.clone(),
//            ) {
//                Err(e) => return Err(delayed_error(e, error_delay)),
//                Ok(user) => *user.id(),
//            };
//            meta = Meta::new(user_id);
//
//            thread::sleep(Duration::from_secs(1));
//            info!(sync_log::stream(), "Begin...");
//            let n = res!(keyspec.len());
//            let valspec = DataSpec {
//                size:   valsize,
//                fill:   DataFill::Random,
//                arr:    DataArrangement::PlainFill(n),
//            };
//
//            info!(sync_log::stream(), "Generating {} data pairs using:", n);
//            info!(sync_log::stream(), " keyspec: {:?}", keyspec);
//            info!(sync_log::stream(), " valspec: {:?}", valspec);
//            let k = res!(keyspec.generate());
//            let v = res!(valspec.generate());
//            //let n = k.len();
//            let mut kmaxlen = 0;
//            let mut kminlen = usize::MAX;
//            let mut vmaxlen = 0;
//            let mut vminlen = usize::MAX;
//            let mut kbyts = 0;
//            let mut vbyts = 0;
//            let mut kdats = Vec::new();
//            let mut vdats = Vec::new();
//            for i in 0..k.len() {
//                let x = &k[i];
//                if x.len() > kmaxlen { kmaxlen = x.len() }
//                if x.len() < kminlen { kminlen = x.len() }
//                kbyts += x.len();
//                kdats.push(Dat::Byt(k[i].clone()));
//                
//                let x = &v[i];
//                if x.len() > vmaxlen { vmaxlen = x.len() }
//                if x.len() < vminlen { vminlen = x.len() }
//                vbyts += x.len();
//                vdats.push(Dat::Byt(v[i].clone()));
//            }
//            info!(sync_log::stream(), "Completed generation of {} pairs with metrics:", n);
//            info!(sync_log::stream(), "  Key sizes: mean {} range {}..{}", kbyts / n, kminlen, kmaxlen);
//            info!(sync_log::stream(), "  Val sizes: mean {} range {}..{}", vbyts / n, vminlen, vmaxlen);
//            info!(sync_log::stream(), "ozone_keys:");
//            let mut kbufs = Vec::new();
//            let mut zinds = Vec::new();
//            for kdat in &kdats {
//                let (kbuf, zind, _) = res!(db.ozone_key_dat(&kdat, &store_spec));
//                kbufs.push(kbuf);
//                zinds.push(zind);
//            }
//            let mask = find_unique(&kbufs);
//
//            //for (i, kbuf) in kbufs.iter().enumerate() {
//            //    info!(sync_log::stream(), " z={} len={} u={} kbuf={:02x?}", zinds[i], kbuf.len(), mask[i], kbuf);
//            //}
//            //info!(sync_log::stream(), "value dats:");
//            //for vdat in &vdats {
//            //    info!(sync_log::stream(), " vdat={:?}", vdat);
//            //}
//            //
//
//            if *case == 550 {
//
//                let mut path = PathBuf::from(&root_path);
//                path.push("control");
//                info!(sync_log::stream(), "Creating {:?}", path);
//                res!(std::fs::create_dir(&path));
//                match save(
//                    path,
//                    res!(cfg.data_file_max_bytes.try_into()),
//                    &mut db,
//                    &meta,
//                    &store_spec,
//                    kdats.clone(),
//                    vdats.clone(),
//                    kbyts + vbyts,
//                ) {
//                    Err(e) => return Err(delayed_error(e, error_delay)),
//                    _ => (),
//                }
//
//            }
//
//            info!(sync_log::stream(), "Test {} {}", case, note);
//            thread::sleep(Duration::from_secs(3));
//
//            match store(
//                &mut db,
//                &meta,
//                &store_spec,
//                kdats.clone(),
//                vdats.clone(),
//                kbyts + vbyts,
//            ) {
//                Err(e) => return Err(delayed_error(e, error_delay)),
//                _ => (),
//            }
//
//            thread::sleep(Duration::from_secs(5));
//            info!(sync_log::stream(), "Ozone state:");
//            let zstats = res!(db.ozone_state(db.default_wait()));
//            for (i, zstat) in zstats.iter().enumerate() {
//                info!(sync_log::stream(), "Zone {}", i+1);
//                for line in oxedyne_fe2o3_text::string::to_lines(fmt!("{:?}", zstat), "  ") {
//                    info!(sync_log::stream(), "{}", line);
//                }
//            }
//            res!(db.list_files());
//
//            //res!(db.dump_caches(db.default_wait()));
//            thread::sleep(Duration::from_secs(3));
//            
//            match fetch(
//                &mut db,
//                &store_spec,
//                &kdats,
//                &mask,
//                &vdats,
//                kbyts + vbyts,
//            ) {
//                Err(e) => return Err(delayed_error(e, error_delay)),
//                _ => (),
//            }
//
//            thread::sleep(Duration::from_secs(3));
//
//            info!(sync_log::stream(), "Shutting db down...");
//            res!(db.shutdown());
//        }
//    }
//
//    Ok(())
//}
