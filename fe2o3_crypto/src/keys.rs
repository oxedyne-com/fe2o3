use crate::{
    enc::EncryptionScheme,
    scheme::SchemeTimestamp,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    mem::Extract,
    rand::RanDef,
};
use oxedyne_fe2o3_hash::kdf::KeyDerivationScheme;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_hash::kdf::KeyDeriver;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    try_extract_tup2dat,
    tup2dat,
    file::JdatFile,
};
use oxedyne_fe2o3_namex::id::LocalId as SchemeLocalId;
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


/// Length of the wallet master key in bytes. Chosen to match
/// AES-256-GCM's key size so the master key can be handed straight
/// to the existing [`EncryptionScheme`] without an intermediate
/// derivation step.
pub const WALLET_MASTER_KEY_LEN: usize = 32;


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ SECRET KEY                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// A private key with `SchemeTimestamp` metadata.  A heap `Vec` is used since the key accomodates
/// different schemes, so the length of the key is not known at compile time.
pub struct SecretKey {
    pub key: Secret<Vec<u8>>,
    pub sts: SchemeTimestamp,
}

impl Clone for SecretKey {
    fn clone(&self) -> Self {
        Self {
            key: {
                let sk = self.key.expose_secret();
                Secret::new(sk.clone())
            },
            sts: self.sts.clone(),
        }
    }
}

impl Default for SecretKey {
    fn default() -> Self {
        Self {
            key: Secret::new(Vec::new()),
            sts: SchemeTimestamp::default(),
        }
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sk = self.key.expose_secret();
        write!(f, "SecretKey{{ key: [{}] bytes, sts: {:?}, k}}", sk.len(), self.sts)
    }
}

impl ToDat for SecretKey {
    fn to_dat(&self) -> Outcome<Dat> {
        let sk = self.key.expose_secret();
        Ok(tup2dat![
            Dat::bytdat(sk.clone()),
            res!(self.sts.to_dat()),
        ])
    }
}

impl FromDat for SecretKey {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        let mut result = SecretKey::default();
        let mut v = try_extract_tup2dat!(dat);
        result.key = Secret::new(try_extract_dat!(v[0].extract(), BU8, BU16, BU32, BU64));
        result.sts =  res!(SchemeTimestamp::from_dat(v[1].extract()));
        Ok(result)
    }
}

impl SecretKey {
    pub fn new(
        sts: SchemeTimestamp,
        key: Secret<Vec<u8>>,
    )
        -> Self
    {
        Self {
            sts,
            key,
        }
    }

    pub fn now(
        id: SchemeLocalId,
        key: Secret<Vec<u8>>,
    )
        -> Outcome<Self>
    {
        Ok(Self {
            sts: res!(SchemeTimestamp::now(id)),
            key,
        })
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ PUBLIC KEY                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// A public key with `SchemeTimestamp` metadata, stored on the heap.
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct PublicKey {
    pub sts: SchemeTimestamp, // Derived ordering starts with the first field here.
    pub key: Vec<u8>,
}

impl Default for PublicKey {
    fn default() -> Self {
        Self {
            key: Vec::new(),
            sts: SchemeTimestamp::default(),
        }
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey{{ key: [{}] bytes, sts: {:?}, k}}", self.key.len(), self.sts)
    }
}

impl ToDat for PublicKey {
    fn to_dat(&self) -> Outcome<Dat> {
        Ok(tup2dat![
            Dat::bytdat(self.key.clone()),
            res!(self.sts.to_dat()),
        ])
    }
}

impl FromDat for PublicKey {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        let mut result = PublicKey::default();
        let mut v = try_extract_tup2dat!(dat);
        result.key = try_extract_dat!(v[0].extract(), BU8, BU16, BU32, BU64);
        result.sts = res!(SchemeTimestamp::from_dat(v[1].extract()));
        Ok(result)
    }
}

impl PublicKey {
    pub fn new(
        sts: SchemeTimestamp,
        key: Vec<u8>,
    )
        -> Self
    {
        Self {
            key,
            sts,
        }
    }

    pub fn now(
        id: SchemeLocalId,
        key: Vec<u8>,
    )
        -> Outcome<Self>
    {
        Ok(Self {
            sts: res!(SchemeTimestamp::now(id)),
            key,
        })
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ KEYS PAIR                                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// An optional public and private key pair with no additional metadata.
#[derive(Default)]
pub struct Keys<
    const PK_LEN: usize,
    const SK_LEN: usize,
> {
    pub pk:     Option<[u8; PK_LEN]>,
    pub sks:    Option<Secret<[u8; SK_LEN]>>,
}

impl<
    const PK_LEN: usize,
    const SK_LEN: usize,
>
    Clone for Keys<PK_LEN, SK_LEN>
{
    fn clone(&self) -> Self {
        Self {
            pk: self.pk.clone(),
            sks: match &self.sks {
                Some(sks) => {
                    let sk = sks.expose_secret();
                    Some(Secret::new(sk.clone()))
                },
                None => None,
            },
        }
    }
}

impl<
    const PK_LEN: usize,
    const SK_LEN: usize,
>
    RanDef for Keys<PK_LEN, SK_LEN>
{
    fn randef() -> Self {
        let mut pk = [0u8; PK_LEN];
        let mut sk = [0u8; SK_LEN];
        OsRng.fill_bytes(&mut pk);
        OsRng.fill_bytes(&mut sk);
        Self {
            pk: Some(pk),
            sks: Some(Secret::new(sk)),
        }
    }
}

impl<
    const PK_LEN: usize,
    const SK_LEN: usize,
>
    Keys<PK_LEN, SK_LEN>
{
    pub fn new(
        pk: Option<[u8; PK_LEN]>,
        sks: Option<Secret<[u8; SK_LEN]>>,
    )
        -> Self
    {
        Self {
            pk,
            sks,
        }
    }

    pub fn randef_sk_only() -> Self {
        let mut sk = [0u8; SK_LEN];
        OsRng.fill_bytes(&mut sk);
        Self {
            pk: None,
            sks: Some(Secret::new(sk)),
        }
    }

    pub fn randef_pk_only() -> Self {
        let mut pk = [0u8; PK_LEN];
        OsRng.fill_bytes(&mut pk);
        Self {
            pk: Some(pk),
            sks: None,
        }
    }
}


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

/// The result of wrapping a master key under a password.
///
/// All four fields are plain strings so they can be serialised directly
/// into a JDAT file alongside the rest of an admin user entry.
#[derive(Clone, Debug, Default)]
pub struct WrappedKey {
    /// Name of the KDF scheme used to derive the KEK, e.g.
    /// `"Argon2id_v0x13"`.
    pub kdf_name:       String,
    /// Encoded KDF configuration (salt + parameters, no hash). Round-
    /// trips through `KeyDerivationScheme::decode_cfg_from_string`.
    pub kdf_cfg:        String,
    /// Name of the encryption scheme used to wrap the master key
    /// under the KEK, e.g. `"AES-256-GCM"`.
    pub enc_name:       String,
    /// Base2x-encoded wrap blob, using `base2x::HEMATITE64`. The
    /// inner bytes are whatever the chosen `EncryptionScheme::encrypt`
    /// produces: for AES-256-GCM this is ciphertext + embedded tag +
    /// appended nonce.
    pub wrapped_key:    String,
}

/// Wrap `master_key` under a KEK derived from `password` via the
/// named KDF scheme. Uses a freshly randomised salt, so every call
/// produces a different wrap even for the same inputs. Returns the
/// wrap plus enough metadata to unwrap it later (KDF name, KDF
/// config, encryption scheme name).
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

/// Attempt to unwrap a master key using `password`. Returns the
/// recovered master key on success, or an error on either decode
/// failure or authentication-tag mismatch (a wrong password is
/// indistinguishable from a wrong key — both fail the GCM tag check).
///
/// Callers that want to "try every admin entry with this password
/// and see which one matches" should treat any error return as
/// "this wasn't the right entry" and move on to the next one.
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
/// Binds a human-readable name, a scope list, an optional expiry and
/// a password-wrapped copy of the wallet master key. Any admin who
/// can supply the password that unwraps their entry has authenticated
/// to the wallet and recovered the master key; the wallet does not
/// care which admin it was.
///
/// Scopes are checked after the wrap decrypts. An admin whose scope
/// list is empty can still unlock the wallet but cannot invoke any
/// verb. An admin past their `expires_at` is refused regardless of
/// password.
#[derive(Clone, Debug, Default)]
pub struct AdminUser {
    /// Display name for this admin. Used in audit log output and
    /// the `list` subcommand; not used for lookup (lookup is by
    /// "which wrap does this password successfully decrypt").
    pub name:       String,
    /// Verbs this admin is authorised to invoke. The wildcard `"*"`
    /// grants every verb. A verb named `"admin"` is required to
    /// manage other admin entries.
    pub scopes:     Vec<String>,
    /// Unix timestamp (seconds since epoch) after which this entry
    /// is refused. A value of `0` means "never expires".
    pub expires_at: u64,
    /// Password-derived wrap of the wallet master key.
    pub wrap:       WrappedKey,
}

impl AdminUser {
    /// Construct a fresh admin entry by wrapping `master_key` with a
    /// KEK derived from `password`. The caller is responsible for
    /// supplying the KDF scheme name (typically read from the host
    /// application's config file).
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

    /// Attempt to unwrap the master key with `password`. Returns
    /// `Ok(Some(master_key))` on successful wrap decryption,
    /// `Ok(None)` if the decryption fails (wrong password), or an
    /// error if a structural problem is encountered.
    pub fn try_unwrap(&self, password: &[u8]) -> Outcome<Option<Vec<u8>>> {
        match unwrap_master_key(&self.wrap, password) {
            Ok(k) => Ok(Some(k)),
            Err(_) => Ok(None),
        }
    }

    /// Returns `true` if this admin has exceeded its `expires_at`.
    /// A zero expiry is treated as "no expiry" and always returns
    /// `false`.
    pub fn is_expired(&self) -> bool {
        if self.expires_at == 0 { return false; }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now >= self.expires_at
    }

    /// Returns `true` if this admin is authorised for `verb`. The
    /// wildcard scope `"*"` matches any verb.
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
/// Holds the recovered master key in a `Secret` so the bytes clear
/// when the struct drops, together with a snapshot of the matched
/// admin's name and scope list. The matched admin is captured by
/// value rather than by reference so callers can hand the unlocked
/// wallet around freely without borrow-checker friction.
///
/// `Clone` is implemented manually because `secrecy::Secret<Vec<u8>>`
/// does not implement `Clone` via derive (the `CloneableSecret`
/// trait bound is not satisfied for `Vec<u8>`).
pub struct UnlockedWallet {
    /// Wallet master key, 32 bytes.
    pub master_key:     Secret<Vec<u8>>,
    /// Name of the admin whose entry matched.
    pub admin_name:     String,
    /// Scope list of the matched admin, copied out of the wallet so
    /// scope checks do not have to re-borrow the original.
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
    /// Returns `true` if the matched admin is authorised for `verb`.
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

/// Default KDF scheme name used when generating fresh admin entries.
/// Argon2id with the 0x13 parameter set is the OWASP-recommended
/// default.
pub const DEFAULT_WALLET_KDF_NAME: &str = "Argon2id_v0x13";

/// Multi-admin wallet: one master key, many wraps.
///
/// The master key itself never lives on disk — only its per-admin
/// wraps do. Anyone who knows a password that unwraps one of the
/// entries can recover the master key and use it to decrypt
/// application secrets stored in `enc_secs` or (in the host
/// application) to decrypt an Ozone database.
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
    /// Construct a wallet from parts. Typical callers use
    /// [`Wallet::create_with_first_admin`] instead, which generates
    /// a random master key and enrols the first admin in one step.
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

    /// Create a brand-new wallet with one admin entry.
    ///
    /// A fresh 32-byte random master key is generated, wrapped under
    /// `password` using `kdf_name`, and stored as the first entry in
    /// the new wallet. The returned `UnlockedWallet` holds the same
    /// master key in memory so the caller does not have to re-prompt
    /// for the password immediately.
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

    /// Try every admin entry in the wallet against `password`, in
    /// declaration order. Returns the recovered master key bundled
    /// with the matched admin's name and scope list on first
    /// success, or an error if every entry rejects the password.
    ///
    /// Expiry is checked **after** the wrap decrypts, so an expired
    /// admin with a valid password sees a clear "expired" error
    /// rather than a generic "wrong password". This is a deliberate
    /// choice: once you prove you hold a credential, the system
    /// tells you why it still refused you.
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

    /// Add a new admin entry wrapping the wallet's master key under
    /// `new_password`. The caller must supply their own password
    /// (`caller_password`) to prove they hold an unlocked identity
    /// and to recover the master key that the new wrap will
    /// protect. The caller must also have the `"admin"` scope or the
    /// `"*"` wildcard.
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

    /// Remove the first admin entry whose name matches
    /// `target_name`. The caller must supply their own password and
    /// must have the `"admin"` scope or `"*"`. Refuses to remove the
    /// last remaining admin, because a wallet with no admins is
    /// irrecoverable.
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

    /// Enrol a new admin entry wrapping the supplied master key.
    ///
    /// Unlike [`Wallet::add_admin`], this method does not prompt
    /// for or verify a caller password. It is used when the caller
    /// is **already** authenticated -- typically because the host
    /// application performed a `unlock` at startup and is now
    /// carrying the recovered master key in memory. The caller is
    /// also responsible for enforcing their own scope check (e.g.
    /// "does the running admin hold 'admin' or '*'?") before
    /// invoking this method.
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

    /// Remove the first admin entry whose name matches `target`.
    ///
    /// Does not authenticate the caller -- analogous to
    /// [`Wallet::enrol`], this is used when the host application
    /// has already proven authority via a prior `unlock`. Refuses
    /// to remove the last remaining admin because a wallet with no
    /// admins is irrecoverable.
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
