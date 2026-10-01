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
//!       "argon2id":     "$argon2id$v=19$m=4096,t=3,p=1$<salt>$<hash>",
//!       "send_as":      ["news@example.com", "noreply@example.com"]
//!     }
//!   ]
//! }
//! ```
//!
//! The `argon2id` value is the encoded form produced by
//! `oxedyne_fe2o3_hash::kdf::KeyDerivationScheme`.
//!
//! `send_as`, optional, lists the addresses besides its own that the account may send as on
//! submission: the identities a mail client sends through this one login. The submission server
//! refuses any other sender, in the envelope or in the header `From`, since 2026-09-23. Being
//! hot-reloaded, a change to it takes effect at the next authentication.

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
    send_as:        Vec<String>,    // lower-cased, besides `address`
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
            let send_as = res!(read_send_as(m.get(&dat!("send_as")), &address));
            out.push(PasswdEntry { address, delivery_dir, encoded_hash, send_as });
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
            delivery_key:   e.delivery_dir.clone(),
            send_as:        e.send_as.clone(),
        }
    }
}

/// An entry's `send_as` list, lower-cased. Absent is an empty list. Anything that is not a list
/// of addresses is refused rather than skipped, since a skipped entry is an identity whose mail
/// is refused with no word of why.
fn read_send_as(d: Option<&Dat>, address: &str) -> Outcome<Vec<String>> {
    let items: Vec<Dat> = match d {
        None                => return Ok(Vec::new()),
        Some(Dat::List(l))  => l.clone(),
        Some(Dat::Vek(v))   => v.iter().cloned().collect(),
        Some(other)         => return Err(err!(
            "User entry {}: 'send_as' must be a list of addresses, got {:?}.",
            address, other.kind();
            Invalid, Input, Mismatch)),
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let a = match item {
            Dat::Str(s) => s.trim().to_lowercase(),
            other => return Err(err!(
                "User entry {}: a 'send_as' member is a {:?}, not an address.",
                address, other.kind();
                Invalid, Input, Mismatch)),
        };
        match a.rfind('@') {
            Some(i) if i > 0 && i + 1 < a.len() && !a.contains(char::is_whitespace) => out.push(a),
            _ => return Err(err!(
                "User entry {}: 'send_as' member {:?} is not an address.", address, a;
                Invalid, Input)),
        }
    }
    Ok(out)
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


#[cfg(test)]
mod tests {
    use super::*;

    /// A users file in a fresh scratch directory, removed by the caller.
    fn users_file(tag: &str, body: &str) -> Outcome<PathBuf> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(fmt!(
            "fe2o3_mail_passwd_{}_{}_{}", tag, std::process::id(), nanos));
        res!(fs::create_dir_all(&dir), IO, File);
        let path = dir.join("users.jdat");
        res!(fs::write(&path, body), IO, File);
        Ok(path)
    }

    fn entry(send_as: &str) -> String {
        fmt!("{{ \"users\": [ {{ \"address\": \"Hello@Example.com\", \"delivery_dir\": \"example.com/hello\", \
            \"argon2id\": \"x\"{} }} ] }}", send_as)
    }

    /// The identities an entry lists reach the account, lower-cased, and an entry without the
    /// field sends as itself alone.
    #[test]
    fn send_as_reaches_the_account() -> Outcome<()> {
        let path = res!(users_file("listed", &entry(
            ", \"send_as\": [\"News@Example.com\", \"noreply@example.com\"]")));
        let store = PasswdFileUserStore::new(path.clone());
        let user = match res!(store.lookup("hello@example.com")) {
            Some(u) => u,
            None => return Err(err!("The listed account did not resolve."; Test)),
        };
        assert_eq!(user.send_as, vec![fmt!("news@example.com"), fmt!("noreply@example.com")]);
        assert!(user.may_send_as("hello@example.com"));
        assert!(user.may_send_as("NEWS@example.com"));
        assert!(!user.may_send_as("ceo@example.com"));

        res!(fs::write(&path, entry("")), IO, File);
        let user = match res!(store.lookup("hello@example.com")) {
            Some(u) => u,
            None => return Err(err!("The plain account did not resolve."; Test)),
        };
        assert!(user.send_as.is_empty());
        assert!(!user.may_send_as("news@example.com"), "the reload must drop the identity");
        if let Some(dir) = path.parent() {
            let _ = fs::remove_dir_all(dir);
        }
        Ok(())
    }

    /// A `send_as` that is not a list of addresses fails the load and says which entry.
    #[test]
    fn a_bad_send_as_is_refused_by_entry() -> Outcome<()> {
        for bad in [
            ", \"send_as\": \"news@example.com\"",
            ", \"send_as\": [\"news\"]",
            ", \"send_as\": [\"news @example.com\"]",
            ", \"send_as\": [(u8|3)]",
        ] {
            let path = res!(users_file("bad", &entry(bad)));
            let store = PasswdFileUserStore::new(path.clone());
            match store.lookup("hello@example.com") {
                Ok(u) => return Err(err!("{:?} loaded as {:?}.", bad, u; Test)),
                Err(e) => assert!(fmt!("{}", e).contains("hello@example.com"),
                    "the refusal must name the entry: {}", e),
            }
            if let Some(dir) = path.parent() {
                let _ = fs::remove_dir_all(dir);
            }
        }
        Ok(())
    }
}
