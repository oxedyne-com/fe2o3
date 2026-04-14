//! `passwd`-style file-backed `UserStore`.
//!
//! Reads a JDAT file mapping email addresses to Argon2id-hashed
//! passwords plus the relative directory under which the matching
//! Maildir tree lives. Designed to be hand-edited by an administrator
//! and to be hot-reloaded on every authentication so password changes
//! take effect without restarting the server.
//!
//! File format (`users.jdat`):
//!
//! ```jdat
//! {
//!   "users": [
//!     {
//!       "address":      "postmaster@example.com",
//!       "delivery_dir": "example.com/postmaster",
//!       "argon2id":     "$argon2id$v=19$m=4096,t=3,p=1$<salt>$<hash>"
//!     }
//!   ]
//! }
//! ```
//!
//! The `argon2id` value is the encoded form produced by
//! `oxedyne_fe2o3_hash::kdf::KeyDerivationScheme`.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::kdf::KeyDerivationScheme;
use oxedyne_fe2o3_iop_hash::kdf::KeyDeriver;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_net::mail::{
    store::MailUser,
    user::UserStore,
};

use std::{
    fs,
    path::PathBuf,
    sync::Arc,
};


/// One row in the user database.
#[derive(Clone, Debug)]
struct PasswdEntry {
    /// Lowercased full email address (`local@domain`).
    address:        String,
    /// Relative path under the Maildir root that holds this user's
    /// mailbox tree.
    delivery_dir:   String,
    /// Encoded Argon2id hash (output of
    /// `KeyDerivationScheme::encode_to_string`).
    encoded_hash:   String,
}

/// File-backed user store.
///
/// Cheaply cloneable -- the file path is wrapped in an `Arc` and the
/// store reloads on every call. For tens of users this is fast enough
/// and avoids any reload coordination.
#[derive(Clone, Debug)]
pub struct PasswdFileUserStore {
    path: Arc<PathBuf>,
}

impl PasswdFileUserStore {
    /// Build a store backed by the given JDAT file.
    pub fn new(path: PathBuf) -> Self {
        Self { path: Arc::new(path) }
    }

    /// Read and parse every entry in the file.
    fn load(&self) -> Outcome<Vec<PasswdEntry>> {
        let text = match fs::read_to_string(self.path.as_path()) {
            Ok(s) => s,
            Err(e) => return Err(err!(e,
                "Reading user file {:?}.", self.path;
                IO, File, Read)),
        };
        let dat = res!(Dat::decode_string(&text));
        let map = match dat {
            Dat::Map(m) => m,
            _ => return Err(err!(
                "User file {:?} top-level must be a map.", self.path;
                Invalid, Input, Mismatch)),
        };
        let users = match map.get(&dat!("users")) {
            Some(Dat::List(l)) => l.clone(),
            _ => return Err(err!(
                "User file {:?} has no 'users' list.", self.path;
                Invalid, Input, Missing)),
        };
        let mut out = Vec::with_capacity(users.len());
        for entry in users {
            let m = match entry {
                Dat::Map(m) => m,
                _ => return Err(err!(
                    "Each user entry must be a map.";
                    Invalid, Input, Mismatch)),
            };
            let address = match m.get(&dat!("address")) {
                Some(Dat::Str(s)) => s.to_lowercase(),
                _ => return Err(err!(
                    "User entry missing 'address'.";
                    Invalid, Input, Missing)),
            };
            let delivery_dir = match m.get(&dat!("delivery_dir")) {
                Some(Dat::Str(s)) => s.clone(),
                _ => return Err(err!(
                    "User entry missing 'delivery_dir'.";
                    Invalid, Input, Missing)),
            };
            let encoded_hash = match m.get(&dat!("argon2id")) {
                Some(Dat::Str(s)) => s.clone(),
                _ => return Err(err!(
                    "User entry missing 'argon2id'.";
                    Invalid, Input, Missing)),
            };
            out.push(PasswdEntry { address, delivery_dir, encoded_hash });
        }
        Ok(out)
    }

    fn entry_to_user(e: &PasswdEntry) -> MailUser {
        let (local, domain) = match e.address.rfind('@') {
            Some(i) => (e.address[..i].to_string(), e.address[i + 1..].to_string()),
            None    => (e.address.clone(), String::new()),
        };
        MailUser {
            local,
            domain,
            delivery_key: e.delivery_dir.clone(),
        }
    }
}

impl UserStore for PasswdFileUserStore {

    fn authenticate(
        &self,
        address:    &str,
        password:   &str,
    )
        -> Outcome<Option<MailUser>>
    {
        let entries = res!(self.load());
        let lc = address.to_lowercase();
        let entry = match entries.iter().find(|e| e.address == lc) {
            Some(e) => e,
            None => return Ok(None),
        };
        // Decode and verify the Argon2id hash. The encoded form is the
        // same as `KeyDerivationScheme::encode_to_string` and round-
        // trips through `KeyDerivationScheme::from_encoded_string`.
        let mut kdf = res!(KeyDerivationScheme::from_str("Argon2id_v0x13"));
        if let Err(e) = kdf.decode_from_string(&entry.encoded_hash) {
            warn!("Failed to decode Argon2id hash for {}: {}", lc, e);
            return Ok(None);
        }
        let ok = res!(kdf.verify(password.as_bytes()));
        if !ok { return Ok(None); }
        Ok(Some(Self::entry_to_user(entry)))
    }

    fn lookup(&self, address: &str) -> Outcome<Option<MailUser>> {
        let entries = res!(self.load());
        let lc = address.to_lowercase();
        if let Some(e) = entries.iter().find(|e| e.address == lc) {
            return Ok(Some(Self::entry_to_user(e)));
        }
        Ok(None)
    }
}
