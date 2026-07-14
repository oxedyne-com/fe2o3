use oxedyne_fe2o3_core::{
    prelude::*,
};
use oxedyne_fe2o3_hash::{
    hash::HashScheme,
};
use oxedyne_fe2o3_iop_hash::{
    api::{
        Hasher,
        HashForm,
    },
};

pub fn test_hash(filter: &'static str) -> Outcome<()> {

    match filter {
        "all" | "hash" => {
            let hasher = HashScheme::new_sha3_256();
            req!(*res!(hasher.hash_length().required("SHA3-256 hash length")), 32);
            let input = b"this is a test";
            let hash = hasher.hash(&[&input[..]], []);
            msg!("hash = {:02x?}", hash);
            match hash.as_hashform() {
                HashForm::Bytes32(a32) => req!(a32.len(), 32),
                other => return Err(err!(
                    "Expected SHA3-256 to produce a HashForm::Bytes32, found {:?}.", other;
                Test, Mismatch)),
            }
        },
        _ => (),
    }

    Ok(())
}
