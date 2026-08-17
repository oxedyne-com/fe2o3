//! Encrypted multi-admin keystore.
//!
//! A [`Wallet`] holds one symmetric master key protected by any number
//! of password-derived wraps, each owned by a named administrator.
//! Anyone who can supply a password that unwraps one of the entries
//! authenticates as that admin and recovers the master key. The
//! pattern mirrors LUKS key slots, PGP multi-recipient encryption and
//! `age`'s multi-recipient files: one master, many wraps, adding or
//! revoking a password-holder does not disturb the others.
//!
//! The keystore itself is transport-agnostic -- callers supply
//! password bytes from whatever source suits them (interactive stdin,
//! an environment variable, an OS keyring, a signed unlock envelope
//! from a remote administrator). Only the password bytes cross the
//! module boundary; how they were obtained is the caller's concern.
//!
//! # On-disk layout
//!
//! A wallet serialises to a JDAT ordered map:
//!
//! - `metadata` -- application-owned plaintext (app name, root path,
//!   whatever the host wants stamped on the wallet).
//! - `admins` -- a list of [`AdminUser`] entries, each carrying its
//!   own [`WrappedKey`], scope list and expiry.
//! - `app_encrypted_secrets` -- a map of application-owned encrypted
//!   secrets. Each value is independently encrypted; the keystore
//!   does not interpret the bytes.
//!
//! Admin names and scopes are plaintext by design: an attacker with
//! disk access can enumerate admins without decrypting. The threat
//! model protects against stolen files, not against admin
//! enumeration.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::enc::EncryptionScheme;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::kdf::KeyDerivationScheme;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_hash::kdf::KeyDeriver;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    file::JdatFile,
};
use oxedyne_fe2o3_text::base2x;

use std::{
    fmt,
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
};

use rand_core::{
    OsRng,
    RngCore,
};

use secrecy::{
    ExposeSecret,
    Secret,
};


// Matches AES-256-GCM's key size, so the master key goes straight to
// EncryptionScheme with no intermediate derivation.
pub const WALLET_MASTER_KEY_LEN: usize = 32;


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ KEY WRAPPING PRIMITIVES                                                   │
// │                                                                           │
// │ Wrap a symmetric master key with a password-derived key encryption key    │
// │ (KEK) so it can be safely stored on disk alongside the KDF parameters.    │
// │ The caller recovers the master key by re-deriving the KEK from the same   │
// │ password and decrypting the wrap. The pattern is the same as LUKS's key   │
// │ slots, PGP's multiple-recipient encryption, and `age`'s multi-recipient   │
// │ files: one master key, any number of independent wraps, adding or        │
// │ revoking a password-holder does not disturb the others.                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// Every field is a plain string, so a wrap serialises straight into a JDAT
/// file beside the rest of an admin entry.
#[derive(Clone, Debug, Default)]
pub struct WrappedKey {
    pub kdf_name:       String, // e.g. "Argon2id_v0x13"
    // Salt and parameters, no hash; round-trips through
    // KeyDerivationScheme::decode_cfg_from_string.
    pub kdf_cfg:        String,
    pub enc_name:       String, // e.g. "AES-256-GCM"
    // Base2x HEMATITE64 over whatever EncryptionScheme::encrypt produced.  For
    // AES-256-GCM that is ciphertext with the tag embedded and the nonce
    // appended, so the blob is self-contained.
    pub wrapped_key:    String,
}

/// Wraps `master_key` under a key derived from `password`.
///
/// The salt is freshly randomised, so the same master key and the same
/// password wrap differently every time.
pub fn wrap_master_key(
    master_key: &[u8],
    password:   &[u8],
    kdf_name:   &str,
)
    -> Outcome<WrappedKey>
{
    // Derive the KEK from the password with a fresh salt. The KDF is
    // stateful: `derive` stores the output internally, which we then
    // read via `get_hash`.
    let mut kdf = res!(KeyDerivationScheme::from_str(kdf_name));
    res!(kdf.derive(password));
    let kek = res!(kdf.get_hash()).to_vec();

    // Wrap with AES-256-GCM. `EncryptionScheme::encrypt` appends the
    // 12-byte nonce to the ciphertext and embeds the GCM auth tag in
    // the ciphertext, so the returned bytes are self-contained.
    let enc = res!(EncryptionScheme::new_aes_256_gcm_with_key(&kek));
    let wrap_bytes = res!(enc.encrypt(master_key));

    Ok(WrappedKey {
        kdf_name:       kdf_name.to_string(),
        kdf_cfg:        res!(kdf.encode_cfg_to_string()),
        enc_name:       fmt!("{:?}", enc),
        wrapped_key:    base2x::HEMATITE64.to_string(&wrap_bytes),
    })
}

/// Recovers a master key from a wrap.
///
/// A wrong password and a corrupt wrap both fail the GCM tag check and are
/// indistinguishable here, so a caller trying each admin entry in turn should
/// read any error as "not this entry" rather than as a fault.
pub fn unwrap_master_key(
    wrapped:    &WrappedKey,
    password:   &[u8],
)
    -> Outcome<Vec<u8>>
{
    // Re-derive the KEK from the supplied password and the stored
    // KDF config.
    let mut kdf = res!(KeyDerivationScheme::from_str(&wrapped.kdf_name));
    res!(kdf.decode_cfg_from_string(&wrapped.kdf_cfg));
    res!(kdf.derive(password));
    let kek = res!(kdf.get_hash()).to_vec();

    // Decode the wrap blob and decrypt.
    let wrap_bytes = res!(base2x::HEMATITE64.from_str(&wrapped.wrapped_key));
    let enc = res!(EncryptionScheme::new_aes_256_gcm_with_key(&kek));
    let plain = res!(enc.decrypt(&wrap_bytes));
    Ok(plain)
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ADMIN USER                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// A single administrator entry in the wallet.
///
/// Anyone who supplies a password that unwraps an entry has authenticated as
/// that admin and holds the master key; the wallet does not care which entry it
/// was.  Scope and expiry are checked only after the wrap decrypts, so an admin
/// with no scopes still unlocks the wallet and can then invoke nothing.
#[derive(Clone, Debug, Default)]
pub struct AdminUser {
    // Audit output and `list` only; lookup is by which wrap the password opens.
    pub name:       String,
    pub scopes:     Vec<String>,	// verbs, or "*" for all; "admin" manages entries
    pub expires_at: u64,		// seconds since epoch, 0 for never
    pub wrap:       WrappedKey,
}

impl AdminUser {
    /// Creates an admin entry, wrapping `master_key` under `password`.
    pub fn new(
        name:       impl Into<String>,
        password:   &[u8],
        master_key: &[u8],
        kdf_name:   &str,
        scopes:     Vec<String>,
        expires_at: u64,
    )
        -> Outcome<Self>
    {
        let wrap = res!(wrap_master_key(master_key, password, kdf_name));
        Ok(Self {
            name:   name.into(),
            scopes,
            expires_at,
            wrap,
        })
    }

    /// A wrong password is `Ok(None)`, not an error: trying every entry in turn
    /// is the intended use, and a failed wrap is not a fault.
    pub fn try_unwrap(&self, password: &[u8]) -> Outcome<Option<Vec<u8>>> {
        match unwrap_master_key(&self.wrap, password) {
            Ok(k) => Ok(Some(k)),
            Err(_) => Ok(None),
        }
    }

    /// Is this admin past its expiry?
    pub fn is_expired(&self) -> bool {
        if self.expires_at == 0 { return false; }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now >= self.expires_at
    }

    /// Is this admin authorised for `verb`?
    pub fn has_scope(&self, verb: &str) -> bool {
        self.scopes.iter().any(|s| s == "*" || s == verb)
    }
}

impl ToDat for AdminUser {
    fn to_dat(&self) -> Outcome<Dat> {
        let scopes_dat: Vec<Dat> = self.scopes.iter()
            .map(|s| dat!(s.clone()))
            .collect();
        let mut m = DaticleMap::new();
        m.insert(dat!("name"),          dat!(self.name.clone()));
        m.insert(dat!("kdf_name"),      dat!(self.wrap.kdf_name.clone()));
        m.insert(dat!("kdf_cfg"),       dat!(self.wrap.kdf_cfg.clone()));
        m.insert(dat!("enc_name"),      dat!(self.wrap.enc_name.clone()));
        m.insert(dat!("scopes"),        Dat::List(scopes_dat));
        m.insert(dat!("expires_at"),    dat!(self.expires_at));
        m.insert(dat!("wrapped_key"),   dat!(self.wrap.wrapped_key.clone()));
        Ok(Dat::Map(m))
    }
}

impl FromDat for AdminUser {
    fn from_dat(mut dat: Dat) -> Outcome<Self> {
        let name = try_extract_dat!(
            res!(dat.map_remove_must(&dat!("name"))),
            Str,
        );
        let kdf_name = try_extract_dat!(
            res!(dat.map_remove_must(&dat!("kdf_name"))),
            Str,
        );
        let kdf_cfg = try_extract_dat!(
            res!(dat.map_remove_must(&dat!("kdf_cfg"))),
            Str,
        );
        let enc_name = try_extract_dat!(
            res!(dat.map_remove_must(&dat!("enc_name"))),
            Str,
        );
        let scopes_dat = try_extract_dat!(
            res!(dat.map_remove_must(&dat!("scopes"))),
            List,
        );
        let mut scopes = Vec::with_capacity(scopes_dat.len());
        for d in scopes_dat {
            scopes.push(try_extract_dat!(d, Str));
        }
        let expires_at = match res!(dat.map_remove_must(&dat!("expires_at"))) {
            Dat::U64(n) => n,
            Dat::U32(n) => n as u64,
            other => return Err(err!(
                "AdminUser: 'expires_at' must be u64 (got {:?}).", other.kind();
                Input, Mismatch)),
        };
        let wrapped_key = try_extract_dat!(
            res!(dat.map_remove_must(&dat!("wrapped_key"))),
            Str,
        );
        Ok(Self {
            name,
            scopes,
            expires_at,
            wrap: WrappedKey {
                kdf_name,
                kdf_cfg,
                enc_name,
                wrapped_key,
            },
        })
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ UNLOCKED WALLET                                                           │
// └───────────────────────────────────────────────────────────────────────────┘

/// The result of successfully unlocking a wallet.
///
/// The master key sits in a `Secret` so the bytes clear when the struct drops.
/// The matched admin is copied in by value rather than borrowed, so an unlocked
/// wallet can be passed around freely.
///
/// `Clone` is written out by hand because `secrecy::Secret<Vec<u8>>` does not
/// derive it: `CloneableSecret` is not satisfied for `Vec<u8>`.
pub struct UnlockedWallet {
    pub master_key:     Secret<Vec<u8>>,	// 32 bytes
    pub admin_name:     String,
    pub admin_scopes:   Vec<String>,
}

impl Clone for UnlockedWallet {
    fn clone(&self) -> Self {
        Self {
            master_key: Secret::new(self.master_key.expose_secret().clone()),
            admin_name: self.admin_name.clone(),
            admin_scopes: self.admin_scopes.clone(),
        }
    }
}

impl fmt::Debug for UnlockedWallet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnlockedWallet")
            .field("master_key",    &"<redacted>")
            .field("admin_name",    &self.admin_name)
            .field("admin_scopes",  &self.admin_scopes)
            .finish()
    }
}

impl UnlockedWallet {
    /// Is the matched admin authorised for `verb`?
    pub fn has_scope(&self, verb: &str) -> bool {
        self.admin_scopes.iter().any(|s| s == "*" || s == verb)
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ WALLET                                                                    │
// │                                                                           │
// │ Holds public metadata, the list of admin entries (each with its own       │
// │ wrapped copy of the shared master key) and a map of application           │
// │ encrypted secrets. Persisted to disk as a JDAT file via the `JdatFile`    │
// │ trait.                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

// Argon2id at the 0x13 parameter set, which is the OWASP recommendation.
pub const DEFAULT_WALLET_KDF_NAME: &str = "Argon2id_v0x13";

/// Multi-admin wallet: one master key, many wraps.
///
/// The master key never lives on disk; only its per-admin wraps do.  Anyone
/// holding a password that opens one entry recovers it, and with it every
/// application secret in `enc_secs`.
#[derive(Clone, Debug, Default)]
pub struct Wallet {
    metadata:   DaticleMap,
    admins:     Vec<AdminUser>,
    enc_secs:   DaticleMap,
}

impl ToDat for Wallet {
    fn to_dat(&self) -> Outcome<Dat> {
        let mut admins_dat = Vec::with_capacity(self.admins.len());
        for a in &self.admins {
            admins_dat.push(res!(a.to_dat()));
        }
        Ok(omapdat!{
            "metadata"                  => Dat::Map(self.metadata.clone()),
            "admins"                    => Dat::List(admins_dat),
            "app_encrypted_secrets"     => Dat::Map(self.enc_secs.clone()),
        })
    }
}

impl FromDat for Wallet {
    fn from_dat(mut dat: Dat) -> Outcome<Self> {
        if dat.kind() != Kind::OrdMap {
            return Err(err!(
                "Wallet must be a Dat::OrdMap, found a {:?}.", dat.kind();
                Input, Invalid, Mismatch));
        }
        let metadata = try_extract_dat!(
            res!(dat.map_remove_must(&dat!("metadata"))),
            Map,
        );
        let admins_dat = try_extract_dat!(
            res!(dat.map_remove_must(&dat!("admins"))),
            List,
        );
        let mut admins = Vec::with_capacity(admins_dat.len());
        for d in admins_dat {
            admins.push(res!(AdminUser::from_dat(d)));
        }
        let enc_secs = try_extract_dat!(
            res!(dat.map_remove_must(&dat!("app_encrypted_secrets"))),
            Map,
        );
        Ok(Self {
            metadata,
            admins,
            enc_secs,
        })
    }
}

impl JdatFile for Wallet {}

impl Wallet {
    /// Assembles a wallet from parts.  Most callers want
    /// [`Wallet::create_with_first_admin`], which mints the master key too.
    pub fn new(
        metadata:   DaticleMap,
        admins:     Vec<AdminUser>,
        enc_secs:   DaticleMap,
    )
        -> Self
    {
        Self {
            metadata,
            admins,
            enc_secs,
        }
    }

    /// Creates a wallet, minting a random master key and enrolling the first
    /// admin under `password`.
    ///
    /// The wallet comes back already unlocked, so the caller need not prompt
    /// again for the password it has just been given.
    pub fn create_with_first_admin(
        metadata:   DaticleMap,
        admin_name: impl Into<String>,
        password:   &[u8],
        kdf_name:   &str,
    )
        -> Outcome<(Self, UnlockedWallet)>
    {
        let mut master = vec![0u8; WALLET_MASTER_KEY_LEN];
        OsRng.fill_bytes(&mut master);

        let name = admin_name.into();
        let admin = res!(AdminUser::new(
            name.clone(),
            password,
            &master,
            kdf_name,
            vec!["*".to_string()],
            0,
        ));
        let unlocked = UnlockedWallet {
            master_key:     Secret::new(master),
            admin_name:     name,
            admin_scopes:   vec!["*".to_string()],
        };
        let wallet = Self {
            metadata,
            admins:     vec![admin],
            enc_secs:   DaticleMap::new(),
        };
        Ok((wallet, unlocked))
    }

    /// Tries every admin entry against `password`, in declaration order.
    ///
    /// Expiry is checked after the wrap decrypts, not before, so an expired
    /// admin holding the right password is told it has expired rather than
    /// that the password is wrong.  Once you have proved you hold a credential,
    /// the system says why it still refused you.
    pub fn unlock(&self, password: &[u8]) -> Outcome<UnlockedWallet> {
        for admin in &self.admins {
            let key_opt = res!(admin.try_unwrap(password));
            if let Some(key) = key_opt {
                if admin.is_expired() {
                    return Err(err!(
                        "Admin '{}' is past its expiry; refused.",
                        admin.name;
                        Input, Invalid, Security));
                }
                return Ok(UnlockedWallet {
                    master_key:     Secret::new(key),
                    admin_name:     admin.name.clone(),
                    admin_scopes:   admin.scopes.clone(),
                });
            }
        }
        Err(err!(
            "No admin entry accepted the supplied password.";
            Input, Invalid, Security, Input))
    }

    /// Adds an admin entry, wrapping the master key under `new_password`.
    ///
    /// `caller_password` does double duty: it proves the caller holds an
    /// identity, and it is how the master key to be re-wrapped is recovered.
    pub fn add_admin(
        &mut self,
        caller_password:    &[u8],
        new_name:           impl Into<String>,
        new_password:       &[u8],
        new_scopes:         Vec<String>,
        new_expires_at:     u64,
        kdf_name:           &str,
    )
        -> Outcome<()>
    {
        let unlocked = res!(self.unlock(caller_password));
        if !unlocked.has_scope("admin") {
            return Err(err!(
                "Admin '{}' does not have 'admin' scope; cannot add \
                new admin entries.", unlocked.admin_name;
                Input, Invalid, Security));
        }
        let master = unlocked.master_key.expose_secret().clone();
        let new_entry = res!(AdminUser::new(
            new_name,
            new_password,
            &master,
            kdf_name,
            new_scopes,
            new_expires_at,
        ));
        self.admins.push(new_entry);
        Ok(())
    }

    /// Removes the first admin entry named `target_name`.
    pub fn remove_admin(
        &mut self,
        caller_password:    &[u8],
        target_name:        &str,
    )
        -> Outcome<()>
    {
        let unlocked = res!(self.unlock(caller_password));
        if !unlocked.has_scope("admin") {
            return Err(err!(
                "Admin '{}' does not have 'admin' scope; cannot \
                remove admin entries.", unlocked.admin_name;
                Input, Invalid, Security));
        }
        if self.admins.len() <= 1 {
            return Err(err!(
                "Refusing to remove the last remaining admin entry \
                -- a wallet with no admins cannot be unlocked again.";
                Invalid, Input));
        }
        let before = self.admins.len();
        self.admins.retain(|a| a.name != target_name);
        if self.admins.len() == before {
            return Err(err!(
                "No admin entry named '{}'.", target_name;
                Missing, Input));
        }
        Ok(())
    }

    /// Enrols an admin entry against a master key the caller already holds.
    ///
    /// **This authenticates nobody and checks no scope**, unlike
    /// [`Wallet::add_admin`].  It is for a host that unlocked at startup and is
    /// carrying the master key; that host owns the scope check.
    pub fn enrol(
        &mut self,
        master_key: &[u8],
        new_name:   impl Into<String>,
        new_pass:   &[u8],
        new_scopes: Vec<String>,
        new_expires_at: u64,
        kdf_name:   &str,
    )
        -> Outcome<()>
    {
        let new_entry = res!(AdminUser::new(
            new_name, new_pass, master_key, kdf_name, new_scopes, new_expires_at,
        ));
        self.admins.push(new_entry);
        Ok(())
    }

    /// Re-wraps one admin entry under `new_password`, keeping its scopes and
    /// expiry and leaving every other entry alone.
    ///
    /// **This re-authenticates nobody**: holding `master_key` from an earlier
    /// unlock is the whole of the authority required.
    pub fn change_password(
        &mut self,
        admin_name:     &str,
        master_key:     &[u8],
        new_password:   &[u8],
        kdf_name:       &str,
    )
        -> Outcome<()>
    {
        let admin = match self.admins.iter_mut().find(|a| a.name == admin_name) {
            Some(a) => a,
            None => return Err(err!(
                "No admin entry named '{}'.", admin_name;
                Input, Missing)),
        };
        admin.wrap = res!(wrap_master_key(master_key, new_password, kdf_name));
        Ok(())
    }

    /// Removes the first admin entry named `target`.
    ///
    /// **This authenticates nobody**, as with [`Wallet::enrol`].
    pub fn remove_by_name(&mut self, target: &str) -> Outcome<()> {
        if self.admins.len() <= 1 {
            return Err(err!(
                "Refusing to remove the last remaining admin entry \
                -- a wallet with no admins cannot be unlocked again.";
                Input, Invalid));
        }
        let before = self.admins.len();
        self.admins.retain(|a| a.name != target);
        if self.admins.len() == before {
            return Err(err!(
                "No admin entry named '{}'.", target;
                Input, Missing));
        }
        Ok(())
    }

    pub fn metadata(&self)      -> &DaticleMap      { &self.metadata }
    pub fn admins(&self)        -> &[AdminUser]     { &self.admins }
    pub fn enc_secs(&self)      -> &DaticleMap      { &self.enc_secs }

    pub fn metadata_mut(&mut self)      -> &mut DaticleMap      { &mut self.metadata }
    pub fn admins_mut(&mut self)        -> &mut Vec<AdminUser>  { &mut self.admins }
    pub fn enc_secs_mut(&mut self)      -> &mut DaticleMap      { &mut self.enc_secs }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrap_unwrap_roundtrip() -> Outcome<()> {
        let master = vec![0x42u8; WALLET_MASTER_KEY_LEN];
        let wrap = res!(wrap_master_key(
            &master, b"correct horse battery staple", DEFAULT_WALLET_KDF_NAME,
        ));
        let recovered = res!(unwrap_master_key(&wrap, b"correct horse battery staple"));
        req!(master, recovered);
        // Wrong password must fail.
        req!(unwrap_master_key(&wrap, b"wrong password").is_err(), true);
        Ok(())
    }

    #[test]
    fn test_wallet_create_unlock_roundtrip() -> Outcome<()> {
        let (wallet, unlocked) = res!(Wallet::create_with_first_admin(
            DaticleMap::new(),
            "alice",
            b"hunter2",
            DEFAULT_WALLET_KDF_NAME,
        ));
        req!(wallet.admins().len(), 1);
        req!(unlocked.admin_name, "alice".to_string());
        // Reload via JDAT round-trip.
        let dat = res!(wallet.to_dat());
        let wallet2 = res!(Wallet::from_dat(dat));
        let unlocked2 = res!(wallet2.unlock(b"hunter2"));
        req!(unlocked.master_key.expose_secret(), unlocked2.master_key.expose_secret());
        // Wrong password must fail.
        req!(wallet2.unlock(b"notit").is_err(), true);
        Ok(())
    }

    #[test]
    fn test_wallet_change_password() -> Outcome<()> {
        let (mut wallet, unlocked) = res!(Wallet::create_with_first_admin(
            DaticleMap::new(),
            "alice",
            b"oldpass",
            DEFAULT_WALLET_KDF_NAME,
        ));
        let master = unlocked.master_key.expose_secret().clone();
        // Old password works.
        req!(wallet.unlock(b"oldpass").is_ok(), true);
        // Rotate alice's password, master key stays the same.
        res!(wallet.change_password(
            "alice",
            &master,
            b"newpass",
            DEFAULT_WALLET_KDF_NAME,
        ));
        // Old no longer works, new does, master key unchanged.
        req!(wallet.unlock(b"oldpass").is_err(), true);
        let u2 = res!(wallet.unlock(b"newpass"));
        req!(u2.master_key.expose_secret(), &master);
        req!(u2.admin_name, "alice".to_string());
        // Scopes preserved.
        req!(u2.admin_scopes, vec!["*".to_string()]);
        // Rotation on an unknown name fails.
        req!(wallet.change_password(
            "nobody", &master, b"x", DEFAULT_WALLET_KDF_NAME,
        ).is_err(), true);
        Ok(())
    }

    #[test]
    fn test_wallet_add_remove_admin() -> Outcome<()> {
        let (mut wallet, _) = res!(Wallet::create_with_first_admin(
            DaticleMap::new(),
            "alice",
            b"alicepass",
            DEFAULT_WALLET_KDF_NAME,
        ));
        res!(wallet.add_admin(
            b"alicepass",
            "bob",
            b"bobpass",
            vec!["restart".to_string(), "log".to_string()],
            0,
            DEFAULT_WALLET_KDF_NAME,
        ));
        req!(wallet.admins().len(), 2);
        // Bob can unlock but cannot add further admins.
        let bob_unlocked = res!(wallet.unlock(b"bobpass"));
        req!(bob_unlocked.admin_name, "bob".to_string());
        req!(bob_unlocked.has_scope("restart"), true);
        req!(bob_unlocked.has_scope("admin"), false);
        req!(wallet.add_admin(
            b"bobpass",
            "mallory",
            b"mallorypass",
            vec!["*".to_string()],
            0,
            DEFAULT_WALLET_KDF_NAME,
        ).is_err(), true);
        // Alice (with '*') can remove bob.
        res!(wallet.remove_admin(b"alicepass", "bob"));
        req!(wallet.admins().len(), 1);
        // Cannot remove the last admin.
        req!(wallet.remove_admin(b"alicepass", "alice").is_err(), true);
        Ok(())
    }
}
