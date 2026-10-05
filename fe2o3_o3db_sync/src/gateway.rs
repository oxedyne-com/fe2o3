//! Opening an Ozone store under the gateway's own parameterisation, for an operator tool.
//!
//! Steel and Daimond keep their stores with 16-byte `u128` user ids, AES-256-GCM at rest and CRC-32
//! checksums. A store written under a different parameterisation will not read back. The tools
//! that open such a store (`o3db_migrate`, `o3db_sweep`, `jdat_store_check`) all open it through
//! here, so a key is read, checked and used in one way.

use crate::{
    prelude::*,
    data::core::RestSchemesInput,
};

use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_jdat::id::IdDat;

use std::{
    fs,
    path::Path,
    thread,
    time::Duration,
};


pub type Uid = IdDat<16, u128>;
pub type Db  = O3db<16, Uid, EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>;

pub const DB_KEY_LEN: usize = 32;

/// Reads the 32-byte at-rest key, failing loudly if it is absent or the wrong size. There is
/// deliberately no fallback: a wrong key cannot decrypt a byte.
pub fn load_key(path: &Path) -> Outcome<[u8; DB_KEY_LEN]> {
    if !path.exists() {
        return Err(err!(
            "No database key at {:?}. Supply the store's at-rest key with --key; \
            without it the store cannot be read.", path;
            Missing, Key, Input));
    }
    let bytes = res!(fs::read(path));
    if bytes.len() != DB_KEY_LEN {
        return Err(err!(
            "The database key at {:?} is {} bytes, but must be {}.",
            path, bytes.len(), DB_KEY_LEN;
            Invalid, Input, Key));
    }
    let mut key = [0u8; DB_KEY_LEN];
    key.copy_from_slice(&bytes);
    Ok(key)
}

/// Opens and starts a store, leaving garbage collection off unless asked. `cfg_opt` is the
/// configuration of a store being created, and None reads the store's own.
pub fn open_store(
    root:       &Path,
    cfg_opt:    Option<OzoneConfig>,
    key:        &[u8; DB_KEY_LEN],
    gc_on:      bool,
    label:      &str,
)
    -> Outcome<Db>
{
    let aes_gcm = res!(EncryptionScheme::new_aes_256_gcm_with_key(&key[..]));
    let crc32 = ChecksumScheme::new_crc32();
    let schms_input = RestSchemesInput::new(
        Some(aes_gcm),
        None::<HashScheme>,
        None::<HashScheme>,
        Some(crc32),
    );
    let mut db: Db = res!(O3db::new(root, cfg_opt, schms_input, Uid::default()));
    res!(db.start(label.to_string()));
    // State the collector explicitly, so that a read never collects.
    res!(ok!(db.updated_api()).activate_gc(gc_on));
    thread::sleep(Duration::from_millis(500));
    let (_, msgs) = res!(db.api().ping_bots(constant::USER_REQUEST_WAIT));
    if msgs.is_empty() {
        return Err(err!("{}: no bot of the store at {:?} answered.", label, root; Missing, Data));
    }
    info!("{}: {} bots responded.", label, msgs.len());
    Ok(db)
}
