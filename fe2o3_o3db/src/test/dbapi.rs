use crate::{
    prelude::*,
    base::constant,
    comm::msg::OzoneMsg,
    test::{
        data::{
            compare_values,
            stopwatch,
        },
    },
};

use oxedize_fe2o3_iop_db::api::{
    Database,
    RestSchemesOverride,
};
use oxedize_fe2o3_jdat::{
    prelude::*,
    chunk::ChunkConfig,
    id::NumIdDat,
};

use std::{
    thread,
    time::{
        Duration,
        Instant,
    },
};

pub fn simple<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    db:     &mut O3db<UIDL, UID, ENC, KH, PR, CS>,
    schms2: Option<&RestSchemesOverride<ENC, KH>>,
    user:   UID,
)
    -> Outcome<()>
{
    test!("Storing and retrieving some simple data.");

    let resp = db.responder();
    res!(db.api().store_dat_using_responder(
        dat!("Not at post"),
        dat!("TK421"),
        user,
        schms2,
        resp.clone(),
    ));
    match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
        OzoneMsg::Chunks(n) => if n != 1 {
            return Err(err!("There should only be one chunk."; Test, Size));
        },
        msg => return Err(err!(
            "Unrecognised response: {:?}", msg;
            Test, Channel, Read, Unexpected)),
    }
    match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
        OzoneMsg::KeyExists(b) => if b == true {
            return Err(err!("This key should not exist."; Test, Unexpected));
        },
        msg => return Err(err!(
            "Unrecognised response: {:?}", msg;
            Test, Channel, Read, Unexpected)),
    }

    thread::sleep(Duration::from_secs(1));
    //res!(db.dump_caches(db.default_wait()));
    //thread::sleep(Duration::from_secs(1));

    // Store it again.
    let resp = res!(db.api().store_using_schemes(
        dat!("Not at post"),
        dat!("TK421"),
        user,
        schms2,
    ));
    match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
        OzoneMsg::Chunks(n) => if n != 1 {
            return Err(err!("There should only be one chunk."; Test, Size));
        },
        msg => return Err(err!(
            "Unrecognised response: {:?}", msg;
            Test, Channel, Read, Unexpected)),
    }
    match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
        OzoneMsg::KeyExists(b) => if b == false {
            return Err(err!(
                "This key should exist.";
                Test, Data, Missing));
        },
        msg => return Err(err!(
            "Unrecognised response: {:?}", msg;
            Test, Channel, Read, Unexpected)),
    }

    // Now retrieve it.
    let resp = db.responder();
    res!(db.api().fetch_using_responder(&dat!("Not at post"), schms2, resp.clone()));
    let expected = dat!("TK421");

    {
        let enc = db.api().schemes().encrypter();
        let or_enc = schms2.map(|s| s.encrypter());

        let result = res!(resp.recv_daticle(enc, or_enc));
        match result {
            (None, _) => return Err(err!(
                "This key should exist, instead received {:?}.", result;
                Test, Unexpected)),
            (Some((dat, meta2)), _) => {
                if dat != expected {
                    return Err(err!(
                        "Expected value {:?}, received {:?}.", expected, dat;
                        Test, Unexpected));
                }
                if meta2.user != user {
                    return Err(err!(
                        "Expected user {:?}, received {:?}.", user, meta2.user;
                        Test, Unexpected));
                }
            },
        }
    }

    // Store some more data.
    let data = mapdat![
        "mass" => 100,
        "prot" => 25,
        "carb" => 30,
        "fat"  => 45,
    ];
    res!(db.api().store_blindly(
        dat!("oats, uncooked"),
        data,
        user,
        schms2,
    ));
    // It takes time to store the data.  If we attempt to fetch it immediately, we may get nothing.
    thread::sleep(Duration::from_millis(10)); // winding this down to 0 should raise an error below
    // An alternative here is to use a db.responder() and wait until storage is complete.

    // Now retrieve it.
    let resp = res!(db.api().fetch_using_schemes(&dat!("oats, uncooked"), schms2));
    let enc = db.api().schemes().encrypter();
    let or_enc = schms2.map(|s| s.encrypter());
    match res!(resp.recv_daticle(enc, or_enc)) {
        (None, _) => return Err(err!("Could not find oats data."; Test, Data, Missing)),
        (Some((Dat::Map(map), meta2)), _) => {
            match map.get(&dat!("prot")) {
                Some(Dat::I32(25)) => (),
                result => return Err(err!(
                    "Unexpected value for field 'prot' in map: {:?}", result;
                    Test, Unexpected)),
            }
            if meta2.user != user {
                return Err(err!(
                    "Expected user {:?}, received {:?}.", user, meta2.user;
                    Test, Unexpected));
            }
        },
        (Some((dat, meta2)), _) => return Err(err!(
            "Unexpected Dat {:?} and meta {:?} returned.", dat, meta2;
            Test, Unexpected)),
    }

    test!("Wow, it worked");
    Ok(())
}

pub fn simple_api<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    db:     &mut O3db<UIDL, UID, ENC, KH, PR, CS>,
    user:   UID,
)
    -> Outcome<()>
{
    test!("Storing and retrieving some simple data using the Database insert and get api.");

    let k = dat!("Meaning of life");
    let v = dat!(42u8);
    res!(db.insert(k.clone(), v.clone(), user, None));
    let result = res!(db.get(&k, None));
    if let Some((v2, _meta2)) = result {
        req!(v, v2);
    } else {
        return Err(err!("Expected value."; Test, Missing, Data)); 
    }

    Ok(())
}

pub fn store_chunked_data<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    db:     &mut O3db<UIDL, UID, ENC, KH, PR, CS>,
    schms2: Option<&RestSchemesOverride<ENC, KH>>,
    user:   UID,
    k:      Dat,
    v:      Dat,
)
    -> Outcome<()>
{
    const CHNK_CFG: ChunkConfig = ChunkConfig {
        threshold_bytes:    0,
        chunk_size:         123,
        dat_wrap:           true,
        pad_last:           true,
    };

    test!("Store and read data that is chunked.");

    let schms2 = match schms2 {
        Some(schms2) => schms2.clone(),
        None => RestSchemesOverride::<ENC, KH>::default(),
    }.set_chunk_config(Some(CHNK_CFG));

    let (resp, num_chunks) = res!(db.api().store_chunked(
        k,
        v,
        user,
        Some(&schms2),
    ));
    let (_, msgs) = res!(resp.recv_number(num_chunks, constant::USER_REQUEST_WAIT));
    for msg in msgs {
        match msg {
            OzoneMsg::KeyChunkExists(b, 0) => {
                if b != false {
                    return Err(err!("This key should not exist."; Test, Unexpected));
                }
                break;
            },
            _ => (),
        }
    }
    Ok(())
}

pub fn fetch_chunked_data<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    db:     &mut O3db<UIDL, UID, ENC, KH, PR, CS>,
    key:    &Dat,
    val:    &Vec<u8>,
    user:   UID,
    schms2: Option<&RestSchemesOverride<ENC, KH>>,
)
    -> Outcome<()>
{
    // Now retrieve it, twice.
    // 1. Retrieve values from caches,
    // 2. Retrieve values from files.
    for src in &["cache", "files"] {
        test!("Retrieve values from {}.", src);
        // First, fetch the bunch key.
        let resp = res!(db.api().fetch_using_schemes(key, schms2));
        let enc = db.api().schemes().encrypter();
        let or_enc = schms2.map(|s| s.encrypter());
        match res!(resp.recv_daticle(enc, or_enc)) {
            (None, _) => return Err(err!(
                "Could not find data for key {:?} {:02x?}.", key,key.as_bytes();
                Test, Data, Missing)),
            (Some((Dat::Tup5u64(tup), meta2)), _) => {
                // Quick check on the metadata
                if meta2.user != user {
                    return Err(err!(
                        "Expected user {:?}, received {:?}.", user, meta2.user;
                        Unexpected, Data, Mismatch));
                }
                // Fetch the chunks.
                match res!(db.api().fetch_chunks(&Dat::Tup5u64(tup), schms2)) {
                    Dat::BU8(v)   |
                    Dat::BU16(v)  |
                    Dat::BU32(v)  |
                    Dat::BU64(v)  => {
                        if val.len() != v.len() {
                            return Err(err!(
                                "Original length = {}, retrieved length = {}.",
                                val.len(), v.len();
                                Test, Size, Mismatch)); 
                        }
                        for j in 0..val.len() {
                            if val[j] != v[j] {
                                return Err(err!(
                                    "Failed at byte {} of {}. Original value {}, \
                                    retrieved value {}.",
                                    j, val.len(), val[j], v[j];
                                    Test, Data, Mismatch)); 
                            }
                        }
                    },
                    dat => return Err(err!(
                        "Unexpected Dat {:?} returned.", dat;
                        Unexpected, Data)),
                }
            },
            (Some((Dat::BU8(v), _)), _)   |
            (Some((Dat::BU16(v), _)), _)  |
            (Some((Dat::BU32(v), _)), _)  |
            (Some((Dat::BU64(v), _)), _)  => {
                // Should only get here if the data was not chunked.
                if val.len() != v.len() {
                    return Err(err!(
                        "Original length = {}, retrieved length = {}.",
                        val.len(), v.len();
                        Test, Size, Mismatch)); 
                }
            },
            (Some((dat, meta2)), _) => return Err(err!(
                "Unexpected Dat {:?} and meta {:?} returned.", dat, meta2;
                Test, Data, Unexpected)),
        }
        res!(db.api().clear_cache_values(constant::USER_REQUEST_WAIT));
    }

    test!("Wow, it worked");

    Ok(())
}

pub fn store<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    db:     &mut O3db<UIDL, UID, ENC, KH, PR, CS>,
    user:   UID,
    schms2: Option<&RestSchemesOverride<ENC, KH>>,
    ks:     Vec<Dat>,
    vs:     Vec<Dat>,
    byts:   usize, // total bytes in data
)
    -> Outcome<(u64, u64)>
{
    test!("Starting some new live files to give us a clean slate...");
    test!("  RestSchemes: {:?}", schms2);
    res!(db.api().new_live_files());

    let n = ks.len();

    // Almost all the data is stored in "firehose mode", where we don't ask for any database
    // feedback on the result.
    test!("Storing {} key-value data pairs.", n);
    let start = Instant::now();
    for i in 0..(n - 1) {
        res!(db.api().store_blindly(
            ks[i].clone(),
            vs[i].clone(),
            user,
            schms2,
        ));
    }
    // Store one final pair in order to get a responder to tell us when all writes have completed.
    res!(res!(db.api().store_using_schemes(
        ks[n - 1].clone(),
        vs[n - 1].clone(),
        user,
        schms2,
    )).recv_timeout(constant::USER_REQUEST_TIMEOUT));

    let elapsed = start.elapsed().as_secs_f64();
    Ok(stopwatch(elapsed, n, byts))
}

pub fn fetch<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    db:     &mut O3db<UIDL, UID, ENC, KH, PR, CS>,
    schms2: Option<&RestSchemesOverride<ENC, KH>>,
    ks:     &Vec<Dat>,
    mask:   &Vec<bool>,
    vs:     &Vec<Dat>,
    byts:   usize,
)
    -> Outcome<Vec<(String, u64, u64)>>
{
    test!("Fetching data.");
    // Now retrieve it, twice.
    // 1. Retrieve values from caches,
    // 2. Retrieve values from files.
    let n = ks.len();
    if mask.len() != n {
        return Err(err!(
            "Mask length {} does not match number of keys {}",
            mask.len(), n;
            Mismatch)); 
    }

    let mut result = Vec::new();
    for src in &["cache", "files"] {
        test!("Retrieve values from {}.", src);
        let mut err_cnt: usize = 0;
        let start = Instant::now();
        for i in 0..n {
            if mask[i] {
                //debug!("Attempt to fetch key {}.", i);
                let resp = res!(db.api().fetch_using_schemes(&ks[i], schms2));
                let enc = db.api().schemes().encrypter();
                let or_enc = schms2.map(|s| s.encrypter());
                match res!(resp.recv_daticle(enc, or_enc)) {
                    (None, _) => err_cnt += 1,
                    (Some((Dat::Tup5u64(tup), _)), _) => {
                        // Fetch the chunks.
                        let dat = res!(db.api().fetch_chunks(&Dat::Tup5u64(tup.clone()), schms2));
                        let v1 = try_extract_dat!(dat, BU8, BU16, BU32, BU64);
                        res!(compare_values(i, &v1, &vs[i]));
                    },
                    (Some((dat, _)), _) => {
                        let v1 = try_extract_dat!(dat, BU8, BU16, BU32, BU64);
                        res!(compare_values(i, &v1, &vs[i]));
                    },
                }
            }
        }
        if err_cnt > 0 {
            error!(err!("Could not find data for {} items.", err_cnt; Test, Data, Missing));
        } else {
            test!("All data retrieved successfully.");
        }
        let elapsed = start.elapsed().as_secs_f64();
        let (tps, bw) = stopwatch(elapsed, n, byts);
        result.push((src.to_uppercase().to_string(), tps, bw));
        if *src == "cache" {
            res!(db.api().clear_cache_values(constant::USER_REQUEST_WAIT));
        }
    }
    
    //res!(db.dump_file_states());

    Ok(result)
}
