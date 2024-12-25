use oxedize_fe2o3_core::{
    prelude::*,
};
use oxedize_fe2o3_hash::{
    hash::HashScheme,
};
use oxedize_fe2o3_iop_hash::{
    api::Hasher,
};

//use std::{
//    collections::BTreeMap,
//};

pub fn test_hash(filter: &'static str) -> Outcome<()> {

    match filter {
        "all" | "hash" => {
            let hasher = HashScheme::new_sha3_256();
            let input = b"this is a test";
            req!(res!(hasher.len()), 32);
            let hash = hasher.hash(&input[..], []);
            msg!("hash = {:02x?}", hash);
        },
        _ => (),
    }

    Ok(())
}
