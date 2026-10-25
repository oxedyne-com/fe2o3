//! Opening an Ozone store under the gateway's own parameterisation, for an operator tool, and that
//! parameterisation for a process that starts its own store ([`rest_schemes`], [`Ns`]).
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
/// A path namespace over a [`Db`].
pub type Ns  = crate::ns::Ns<16, Uid, EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>;

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

/// The schemes a store is kept under, from its at-rest `key`: AES-256-GCM at rest, CRC-32
/// checksums, and the default hashers.  For a process that starts a store of its own, so it keeps
/// it exactly as the gateway and its tools read it.
pub fn rest_schemes(key: &[u8; DB_KEY_LEN])
    -> Outcome<RestSchemesInput<EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>>
{
    let aes_gcm = res!(EncryptionScheme::new_aes_256_gcm_with_key(&key[..]));
    let crc32 = ChecksumScheme::new_crc32();
    Ok(RestSchemesInput::new(
        Some(aes_gcm),
        None::<HashScheme>,
        None::<HashScheme>,
        Some(crc32),
    ))
}

/// Opens and starts a store, leaving garbage collection off unless asked, and says how many of its
/// bots answered a ping. `cfg_opt` is the configuration of a store being created, and None reads
/// the store's own. The store is returned whatever the count, as `o3db_migrate` and `o3db_sweep`
/// opened it before this helper existed; a tool that reads and reports on the store calls
/// [`require_answer`] with the count.
pub fn open_store(
    root:       &Path,
    cfg_opt:    Option<OzoneConfig>,
    key:        &[u8; DB_KEY_LEN],
    gc_on:      bool,
    label:      &str,
)
    -> Outcome<(Db, usize)>
{
    let schms_input = res!(rest_schemes(key));
    let mut db: Db = res!(O3db::new(root, cfg_opt, schms_input, Uid::default()));
    res!(db.start(label.to_string()));
    // State the collector explicitly, so that a read never collects.
    res!(ok!(db.updated_api()).activate_gc(gc_on));
    thread::sleep(Duration::from_millis(500));
    let (_, msgs) = res!(db.api().ping_bots(constant::USER_REQUEST_WAIT));
    info!("{}: {} bots responded.", label, msgs.len());
    Ok((db, msgs.len()))
}

/// Fails when no bot answered (`n` from [`open_store`]). A scan of a store that does not answer
/// finds nothing, and a report that says "clean" of it would be false.
pub fn require_answer(
    n:      usize,
    label:  &str,
    root:   &Path,
)
    -> Outcome<usize>
{
    if n == 0 {
        return Err(err!("{}: no bot of the store at {:?} answered.", label, root; Missing, Data));
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A caller that starts its own store gets the gateway's schemes from the key alone: AES-256-GCM
    /// at rest, CRC-32 checksums, and the default hashers.
    #[test]
    fn rest_schemes_are_the_gateways() {
        let schemes = match rest_schemes(&[7u8; DB_KEY_LEN]) {
            Ok(s)   => s,
            Err(e)  => panic!("the gateway's schemes from a key: {}", e),
        };
        assert!(schemes.enc.is_some(), "an encrypter");
        assert!(schemes.hash.is_none() && schemes.prnd.is_none(), "the default hashers");
        assert!(schemes.csum.is_some(), "a checksummer");
    }
}
