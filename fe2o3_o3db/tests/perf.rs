// Be sure to run using release target, e.g.:
// > clear;clear;cargo test -r --test perf -- --nocapture
use oxedize_fe2o3_core::{
    prelude::*,
    alt::Override,
};
use oxedize_fe2o3_crypto::enc::EncryptionScheme;
use oxedize_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedize_fe2o3_iop_db::api::{
    Meta,
    RestSchemesOverride,
};
use oxedize_fe2o3_jdat::{
    prelude::*,
};
use oxedize_fe2o3_o3db::{
    data::core::RestSchemesInput,
    test::{
        data::{
            find_unique,
        },
        dbapi::{
            store,
            fetch,
        },
        file::{
            append_table,
            save_multiple_files,
            save_single_file,
        },
        setup,
    },
};
use oxedize_fe2o3_test::{
    data::{
        DataArrangement,
        DataFill,
        DataSize,
        DataSpec,
    },
    error::delayed_error,
};

use std::{
    path::{
        Path,
        PathBuf,
    },
    thread,
    time::Duration,
};

use rand_core::{
    RngCore,
    OsRng,
};


fn generate_data(
    keyspec:    DataSpec,
    valspec:    DataSpec,
)
    -> Outcome<(Vec<Dat>, Vec<Dat>, usize, usize)>
{
    let n = res!(keyspec.len());

    test!("Generating {} data pairs using:", n);
    test!(" keyspec: {:?}", keyspec);
    test!(" valspec: {:?}", valspec);
    let k = res!(keyspec.generate());
    let v = res!(valspec.generate());
    //let n = k.len();
    let mut kmaxlen = 0;
    let mut kminlen = usize::MAX;
    let mut vmaxlen = 0;
    let mut vminlen = usize::MAX;
    let mut kbyts = 0;
    let mut vbyts = 0;
    let mut kdats = Vec::new();
    let mut vdats = Vec::new();
    for i in 0..k.len() {
        let x = &k[i];
        if x.len() > kmaxlen { kmaxlen = x.len() }
        if x.len() < kminlen { kminlen = x.len() }
        kbyts += x.len();
        kdats.push(Dat::wrap_dat(k[i].clone()));
        
        let x = &v[i];
        if x.len() > vmaxlen { vmaxlen = x.len() }
        if x.len() < vminlen { vminlen = x.len() }
        vbyts += x.len();
        vdats.push(Dat::wrap_dat(v[i].clone()));
    }
    test!("Completed generation of {} pairs with metrics:", n);
    test!("  Key sizes: mean {} range {}..{}", kbyts / n, kminlen, kmaxlen);
    test!("  Val sizes: mean {} range {}..{}", vbyts / n, vminlen, vmaxlen);
    Ok((kdats, vdats, kbyts, vbyts))
}

pub fn test_perf(_filter: &'static str) -> Outcome<()> {

    let db_root = res!(Path::new("./test_db").canonicalize());
    let mut enckey = [0u8; 32];
    OsRng.fill_bytes(&mut enckey);
    let aes_gcm = res!(EncryptionScheme::new_aes_256_gcm_with_key(&enckey[..]));
    let sha3_256 = HashScheme::new_sha3_256();
    let crc32 = ChecksumScheme::new_crc32();
    let mut schms2: RestSchemesOverride<EncryptionScheme, HashScheme>;
    let schms_input = RestSchemesInput::new(
        Some(aes_gcm.clone()),
        Some(sha3_256.clone()),
        None::<HashScheme>,
        Some(crc32.clone()),
    );
    let user = setup::Uid::default();
    //let meta = Meta::<{ setup::UID_LEN }, setup::Uid>::new(setup::Uid::default());
    let mut table = Vec::new();

    // Generate data.
    let keyspec = DataSpec {
        size:   DataSize::RandUniform { lo: 30, hi: 3000 },
        fill:   DataFill::Random,
        arr:    DataArrangement::RepeatFillAndSeq{
            n: 60,
            rep: 100,
            specbox: Box::new(
                DataSpec {
                    size:   DataSize::RandUniform { lo: 30, hi: 3000 },
                    fill:   DataFill::Random,
                    arr:    DataArrangement::PlainFill(15),
                }
            ),
        },
    };
    let valspec = DataSpec {
        size:   DataSize::RandUniform { lo: 30, hi: 10_000 },
        fill:   DataFill::Random,
        arr:    DataArrangement::PlainFill(res!(keyspec.len())),
    };
    let (kdats, vdats, kbyts, vbyts) = res!(generate_data(keyspec, valspec));
    let mut mask_opt = None;

    let error_delay = 2;
    let data_file_max_bytes: u64 = 1_000_000;
    let bytes_before_chunking: u64 = 100_000;

    {
        for case in &[
            0,
            100,
            200,
        ] {

            let (note, cfg) = match case {
                0 => {
                    schms2 = RestSchemesOverride::default()
                        .set_encrypter(Override::Default(aes_gcm.clone()));
                    let mut cfg = res!(setup::default_cfg());
                    cfg.data_file_max_bytes = data_file_max_bytes;
                    cfg.rest_chunk_threshold = bytes_before_chunking;
                    (
                        fmt!("The kitchen sink, everything on except gc."),
                        cfg,
                    )
                },
                100 => {
                    schms2 = RestSchemesOverride::default();
                    let mut cfg = res!(setup::default_cfg());
                    cfg.data_file_max_bytes = data_file_max_bytes;
                    cfg.rest_chunk_threshold = bytes_before_chunking;
                    (
                        fmt!("Value encryption off."),
                        cfg,
                    )
                },
                200 => {
                    schms2 = RestSchemesOverride::default();
                    let mut cfg = res!(setup::default_cfg());
                    cfg.data_file_max_bytes = data_file_max_bytes;
                    cfg.rest_chunk_threshold = bytes_before_chunking;
                    (
                        fmt!("Value encryption and key hashing off."),
                        cfg,
                    )
                },
                _ => unimplemented!(),
            };

            test!("+---------------------------------------------+");
            test!("| NEW OZONE SESSION                           |");
            test!("| Wipe all traces of previous test.           |");
            test!("| Start database.                             |");
            test!("| No gc and no user filtering.                |");
            test!("| Store and fetch a standard data set.        |");
            test!("| Gracefully shut down the database.          |");
            test!("+---------------------------------------------+");
            let mut db = match setup::start_db(
                db_root.clone(),
                Some(cfg.clone()),
                schms_input.clone(),
                None,
                false,
                true,
            ) {
                // These pauses on errors are needed to capture tardy messages from asynchronous
                // logging.
                Err(e) => return Err(delayed_error(e, error_delay)),
                Ok(db) => db,
            };

            // Can use logger now.
            
            if mask_opt.is_none() {
                test!("Creating mask for unique keys...");
                let mut kbufs = Vec::new();
                for kdat in &kdats {
                    let (kbuf, _, _) = res!(db.api().ozone_key_dat(&kdat, Some(&schms2)));
                    kbufs.push(kbuf);
                }
                mask_opt = Some(find_unique(&kbufs));
                test!("  mask completed.");
            }

            thread::sleep(Duration::from_secs(1));
            test!("Begin...");

            if *case == 0 {

                let mut path = db_root.clone();
                path.push("control_single_file");
                test!("Creating {:?}", path);
                res!(std::fs::create_dir(&path));
                match save_single_file(
                    path,
                    &mut db,
                    user,
                    Some(&schms2),
                    kdats.clone(),
                    vdats.clone(),
                    kbyts + vbyts,
                ) {
                    Err(e) => return Err(delayed_error(e, error_delay)),
                    Ok((tps, bw)) => table.push((fmt!("Save directly to single file"), tps, bw)),
                }

                thread::sleep(Duration::from_secs(3));

                let mut path = db_root.clone();
                path.push("control_multiple_files");
                test!("Creating {:?}", path);
                res!(std::fs::create_dir(&path));
                match save_multiple_files(
                    path,
                    res!(cfg.data_file_max_bytes.try_into()),
                    &mut db,
                    user,
                    Some(&schms2),
                    kdats.clone(),
                    vdats.clone(),
                    kbyts + vbyts,
                ) {
                    Err(e) => return Err(delayed_error(e, error_delay)),
                    Ok((tps, bw)) => table.push((fmt!("Save directly to multiple files"), tps, bw)),
                }

            }

            test!("Test {} {}", case, note);
            thread::sleep(Duration::from_secs(3));

            match store(
                &mut db,
                user,
                Some(&schms2),
                kdats.clone(),
                vdats.clone(),
                kbyts + vbyts,
            ) {
                Err(e) => return Err(delayed_error(e, error_delay)),
                Ok((tps, bw)) => table.push((fmt!("Store: {}", note), tps, bw)),
            }

            thread::sleep(Duration::from_secs(3));
            
            if let Some(mask) = mask_opt.as_ref() {
                match fetch(
                    &mut db,
                    Some(&schms2),
                    &kdats,
                    &mask,
                    &vdats,
                    kbyts + vbyts,
                ) {
                    Err(e) => return Err(delayed_error(e, error_delay)),
                    Ok(result) => {
                        for (s, tps, bw) in result {
                            table.push((fmt!("Fetch {}: {}", s, note), tps, bw));
                        }
                    },
                }
            }

            thread::sleep(Duration::from_secs(3));

            test!("Shutting db down...");
            res!(db.shutdown());
        }
    }

    let path = PathBuf::from("./perf_record.txt");
    res!(append_table(path, table));

    Ok(())
}
