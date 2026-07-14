//! This crate provides a concrete interface for multiple Key Derivation Function algorithms.
//! Currently it uses only one, Argon2.  The Argon2 tag depends on the lane count `p` by design, so
//! a tag produced with `p = 4` differs from one produced with `p = 1` for otherwise identical
//! inputs.  Every parameter that changes the tag must therefore survive a round trip through the
//! encoded configuration string, or the derived key will silently differ.  The default here is a
//! single lane, which the Open Worldwide Application Security Project (OWASP) recommends for
//! Argon2id, but the encoding does not assume it.
//!
//! # Encoded strings
//!
//! `encode_to_string` emits the Password Hashing Competition (PHC) string format exactly, as
//! `$argon2id$v=19$m=<mem_cost>,t=<time_cost>,p=<lanes>$<salt>$<hash>`, so that other Argon2
//! implementations can read it.  The tag length is implied by the length of the encoded hash.
//!
//! `encode_cfg_to_string` emits the same string sans hash.  PHC defines no hash-less form, and
//! without the hash the tag length would be lost, so the tag length is carried explicitly in an
//! `l=<hash_length>` option, as `$argon2id$v=19$m=...,t=...,p=...,l=32$<salt>`.  Associated data,
//! when present, is carried in the `data=<base64>` option used by the reference implementation.
//!
//! The secret (pepper) is deliberately never encoded.  A caller using one must set it on the
//! decoded state out of band; the encoders return an error rather than drop it silently.
//!
use oxedyne_fe2o3_core::{
    prelude::*,
    alt::{
        Alt,
        DefAlt,
    },
    mem::Extract,
    rand::Rand,
};
use oxedyne_fe2o3_jdat::{
    prelude::*,
    try_extract_tup2dat,
    tup2dat,
};
use oxedyne_fe2o3_iop_hash::kdf::KeyDeriver;
use oxedyne_fe2o3_namex::id::{
    LocalId,
    InNamex,
    NamexId,
};

use std::{
    fmt,
    str,
};

use argon2;
use base64;


#[derive(Clone, Debug, Default)]
pub struct KeyDerivConfig {
    id:     LocalId, // Key derivation function id.
    cfg:    String, // String encoded configuration information.
}

impl ToDat for KeyDerivConfig {
    fn to_dat(&self) -> Outcome<Dat> {
        Ok(tup2dat![
            res!(self.id.to_dat()),
            Dat::Str(self.cfg.clone()),
        ])
    }
}

impl FromDat for KeyDerivConfig {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        let mut result = Self::default();
        let mut v = try_extract_tup2dat!(dat);
        result.id = res!(LocalId::from_dat(v[0].extract()));
        result.cfg = try_extract_dat!(v[1].extract(), Str);
        Ok(result)
    }
}

impl KeyDerivConfig {

    pub fn new(id: LocalId, cfg: String,) -> Self {
        Self {
            id,
            cfg,
        }
    }
    pub fn id(&self) -> &LocalId { &self.id }
    pub fn config(&self) -> &String { &self.cfg }
}

#[derive(Clone, Eq, PartialEq)]
pub enum KeyDerivationScheme {
    Argon2(Argon2State),
}

impl fmt::Display for KeyDerivationScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl fmt::Debug for KeyDerivationScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Argon2(state) => {
                let suffix = match state.version {
                    argon2::Version::Version10 => "_v0x10",
                    argon2::Version::Version13 => "_v0x13",
                };
                let mut variant = state.variant.as_uppercase_str().to_string();
                variant.push_str(suffix);
                write!(f, "{}", variant)
            },
        }
    }
}
    
impl InNamex for KeyDerivationScheme {

    fn name_id(&self) -> Outcome<NamexId> {
        Ok(match self {
            Self::Argon2(Argon2State {
                variant: argon2::Variant::Argon2d,
                version: argon2::Version::Version10,
                ..
            }) => res!(NamexId::try_from("sl6UqXw8nwjEzDMyO9TMlCzwUKtNDHgpDq+DOYjUukU=")),
            //
            Self::Argon2(Argon2State {
                variant: argon2::Variant::Argon2i,
                version: argon2::Version::Version10,
                ..
            }) => res!(NamexId::try_from("cu9pC9q0wONz80OCjaxtMJiyv4zqMUuljhDKMOiUE4o=")),
            //
            Self::Argon2(Argon2State {
                variant: argon2::Variant::Argon2id,
                version: argon2::Version::Version10,
                ..
            }) => res!(NamexId::try_from("D4NkV8upthkHwlPrkfq/wqo+S4dnXwU7EkM4vg4h4jo=")),
            //
            Self::Argon2(Argon2State {
                variant: argon2::Variant::Argon2d,
                version: argon2::Version::Version13,
                ..
            }) => res!(NamexId::try_from("bbj8wR0HwO6JJoWDWgauLWOacUfLfJZVnoWEyTYIfqI=")),
            //
            Self::Argon2(Argon2State {
                variant: argon2::Variant::Argon2i,
                version: argon2::Version::Version13,
                ..
            }) => res!(NamexId::try_from("6UQjGIfYSHD7/2mcr27mtB8B9IyQBRpye0ABSH9/YCo=")),
            //
            Self::Argon2(Argon2State {
                variant: argon2::Variant::Argon2id,
                version: argon2::Version::Version13,
                ..
            }) => res!(NamexId::try_from("M/dnApVP4FC91bbNYeKy73V6j3NVfwW7qcVspx53zl8=")),
        })
    }

    fn local_id(&self) -> LocalId {
        match self {
            Self::Argon2(Argon2State {
                variant: argon2::Variant::Argon2d,
                version: argon2::Version::Version10,
                ..
            }) => LocalId(1),
            //
            Self::Argon2(Argon2State {
                variant: argon2::Variant::Argon2i,
                version: argon2::Version::Version10,
                ..
            }) => LocalId(2),
            //
            Self::Argon2(Argon2State {
                variant: argon2::Variant::Argon2id,
                version: argon2::Version::Version10,
                ..
            }) => LocalId(3),
            //
            Self::Argon2(Argon2State {
                variant: argon2::Variant::Argon2d,
                version: argon2::Version::Version13,
                ..
            }) => LocalId(4),
            //
            Self::Argon2(Argon2State {
                variant: argon2::Variant::Argon2i,
                version: argon2::Version::Version13,
                ..
            }) => LocalId(5),
            //
            Self::Argon2(Argon2State {
                variant: argon2::Variant::Argon2id,
                version: argon2::Version::Version13,
                ..
            }) => LocalId(6),
        }
    }

    fn assoc_names_base64(
        gname: &'static str,
    )
        -> Outcome<Option<Vec<(
            &'static str,
            &'static str,
        )>>>
    {
        let ids = match gname {
            "schemes" => [
	            ("Argon2d_v0x10", "sl6UqXw8nwjEzDMyO9TMlCzwUKtNDHgpDq+DOYjUukU="),
	            ("Argon2i_v0x10", "cu9pC9q0wONz80OCjaxtMJiyv4zqMUuljhDKMOiUE4o="),
	            ("Argon2id_v0x10", "D4NkV8upthkHwlPrkfq/wqo+S4dnXwU7EkM4vg4h4jo="),
	            ("Argon2d_v0x13", "bbj8wR0HwO6JJoWDWgauLWOacUfLfJZVnoWEyTYIfqI="),
	            ("Argon2i_v0x13", "6UQjGIfYSHD7/2mcr27mtB8B9IyQBRpye0ABSH9/YCo="),
	            ("Argon2id_v0x13", "M/dnApVP4FC91bbNYeKy73V6j3NVfwW7qcVspx53zl8="),
            ],
            _ => return Err(err!(
                "The Namex group name '{}' is not recognised for KeyDerivationScheme.", gname;
            Invalid, Input)),
        };
        Ok(if ids.len() == 0 {
            None
        } else {
            Some(ids.to_vec())
        })
    }
}

impl str::FromStr for KeyDerivationScheme {
    type Err = Error<ErrTag>;

    fn from_str(name: &str) -> std::result::Result<Self, Self::Err> {
        match name {
            "Argon2d_v0x10"     => Self::default_argon2("Argon2d", 0x10),
            "Argon2i_v0x10"     => Self::default_argon2("Argon2i", 0x10),
            "Argon2id_v0x10"    => Self::default_argon2("Argon2id", 0x10),
            "Argon2d_v0x13"     => Self::default_argon2("Argon2d", 0x13),
            "Argon2i_v0x13"     => Self::default_argon2("Argon2i", 0x13),
            "Argon2id_v0x13"    => Self::default_argon2("Argon2id", 0x13),
            _ => Err(err!(
                "The key derivation scheme '{}' is not recognised.", name;
            Invalid, Input)),
        }
    }
}

impl TryFrom<LocalId> for KeyDerivationScheme {
    type Error = Error<ErrTag>;

    fn try_from(n: LocalId) -> std::result::Result<Self, Self::Error> {
        match n {
            LocalId(1) => Self::default_argon2("Argon2d", 0x10),
            LocalId(2) => Self::default_argon2("Argon2i", 0x10),
            LocalId(3) => Self::default_argon2("Argon2id", 0x10),
            LocalId(4) => Self::default_argon2("Argon2d", 0x13),
            LocalId(5) => Self::default_argon2("Argon2i", 0x13),
            LocalId(6) => Self::default_argon2("Argon2id", 0x13),
            _ => Err(err!(
                "The key derivation scheme with local id {} is not recognised.", n;
            Invalid, Input)),
        }
    }
}

impl KeyDerivationScheme {

    /// Default `KeyDerivationScheme::Argon2` includes a new random salt.
    pub fn default_argon2(
        variant: &str,
        version: u32,
    )
        -> Outcome<Self>
    {
        let mut state = Argon2State::default();
        state.variant = res!(argon2::Variant::from_str(variant));
        state.version = res!(argon2::Version::from_u32(version));
        Ok(Self::Argon2(state))
    }

    pub fn new_argon2(
        variant:        &str,
        version:        u32,
        mem_cost:       u32,
        time_cost:      u32,
        salt_length:    usize,
        hash_length:    u32,
    )
        -> Outcome<Self>
    {
        let mut kdf = Self::Argon2(Argon2State {
            hash_length,
            lanes:          1,
            mem_cost,
            time_cost,
            variant:        res!(argon2::Variant::from_str(variant)),
            version:        res!(argon2::Version::from_u32(version)),
            ..Default::default()
        });
        res!(kdf.set_rand_salt(salt_length));
        Ok(kdf)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Argon2State {
    // Direct copy of argon2::Config<'a>
    pub ad:             Vec<u8>,
    pub hash_length:    u32,
    pub lanes:          u32,
    pub mem_cost:       u32,
    pub secret:         Vec<u8>,
    pub time_cost:      u32,
    pub variant:        argon2::Variant,
    pub version:        argon2::Version,
    // Extra
    pub salt:           Vec<u8>,
    pub hash:           Option<Vec<u8>>,
}

impl Default for Argon2State {
    fn default() -> Self {
        let mut salt = [0u8; 16];
        Rand::fill_u8(&mut salt);
        Self {
            ad:             Vec::new(),
            hash_length:    32,
            lanes:          1,
            mem_cost:       65536,
            secret:         Vec::new(),
            time_cost:      5,
            variant:        argon2::Variant::Argon2id,
            version:        argon2::Version::Version13,
            salt:           salt.to_vec(),
            hash:           None,
        }
    }
}

impl Argon2State {

    /// Creates an Argon2 state with the given parameters, all of which change the resulting tag.
    pub fn new(
        variant:        &str,
        version:        u32,
        mem_cost:       u32,
        time_cost:      u32,
        lanes:          u32,
        hash_length:    u32,
        salt:           Vec<u8>,
    )
        -> Outcome<Self>
    {
        Ok(Self {
            hash_length,
            lanes,
            mem_cost,
            time_cost,
            variant:        res!(argon2::Variant::from_str(variant)),
            version:        res!(argon2::Version::from_u32(version)),
            salt,
            ..Default::default()
        })
    }

    pub fn to_argon2_config<'a>(&'a self) -> argon2::Config<'a> {
        argon2::Config {
            ad:             &self.ad,
            hash_length:    self.hash_length,
            lanes:          self.lanes,
            mem_cost:       self.mem_cost,
            secret:         &self.secret,
            time_cost:      self.time_cost,
            variant:        self.variant.clone(),
            version:        self.version.clone(),
        }
    }
    
    /// Decodes an Argon2 string, returning an error rather than silently defaulting any parameter
    /// that changes the tag.  The `argon2::encoding` module is private, so the decoding must be
    /// replicated here.  The secret, which is never encoded, is left untouched on `self`.
    pub fn from_argon2_string(
        &mut self,
        encoded:        &str,
        expect_hash:    bool,
    )
        -> Outcome<()>
    {
        let body = match encoded.strip_prefix('$') {
            Some(s) => s,
            None => return Err(err!(
                "Encoded Argon2 string '{}' does not begin with a '$'.", encoded;
            Invalid, Input)),
        };
        let (n_complete, n_v0x10) = if expect_hash { (5, 4) } else { (4, 3) };
        let parts: Vec<&str> = body.split('$').collect();
        // The version field is absent in the original v0x10 encoding.
        let (variant_str, version, opts_str, salt_str, hash_str) = if parts.len() == n_complete {
            let vstr = res!(Self::extract_value(parts[1], "v", "version"));
            (
                parts[0],
                res!(argon2::Version::from_str(vstr)),
                parts[2],
                parts[3],
                if expect_hash { Some(parts[4]) } else { None },
            )
        } else if parts.len() == n_v0x10 {
            (
                parts[0],
                argon2::Version::Version10,
                parts[1],
                parts[2],
                if expect_hash { Some(parts[3]) } else { None },
            )
        } else {
            return Err(err!(
                "The Argon2 string '{}' should have {} or {} parts separated by the '$' \
                character, {} were found.", encoded, n_v0x10, n_complete, parts.len();
            Decode, String, Invalid, Input));
        };
        let opts = res!(Self::extract_options(opts_str));
        let salt = res!(base64::decode(salt_str));
        let hash = match hash_str {
            Some(s) => Some(res!(base64::decode(s))),
            None => None,
        };
        // The tag length is implied by the hash when there is one, and carried in the 'l' option
        // when there is not.  A configuration string with neither is the format the previous
        // release wrote, which never emitted 'l' at all: those strings are in every wallet in the
        // field, and a wallet cannot be rebuilt from source, so refusing to read one would lock its
        // owner out of their own master key.  The tag length then stays as the state already has
        // it, which is what the old decoder did, and so derives the key that string has always
        // meant.
        let hash_length = match (&hash, opts.hash_length) {
            (Some(h), Some(l)) => {
                if (h.len() as u32) != l {
                    return Err(err!(
                        "The Argon2 string '{}' declares a hash length of {} but encodes a hash \
                        of {} bytes.", encoded, l, h.len();
                    Decode, String, Mismatch, Invalid, Input));
                }
                l
            },
            (Some(h), None)     => h.len() as u32,
            (None, Some(l))     => l,
            (None, None)        => self.hash_length,
        };
        self.variant        = res!(argon2::Variant::from_str(variant_str));
        self.version        = version;
        self.mem_cost       = opts.mem_cost;
        self.time_cost      = opts.time_cost;
        self.lanes          = opts.lanes;
        self.hash_length    = hash_length;
        self.ad             = opts.ad;
        self.salt           = salt;
        self.hash           = hash;
        Ok(())
    }

    /// Returns an error if a secret is present, since the encoded string must never carry it.
    fn require_no_secret(&self) -> Outcome<()> {
        if !self.secret.is_empty() {
            return Err(err!(
                "This Argon2 state carries a secret of {} bytes. A secret is never written to the \
                encoded string, and encoding it away silently would derive a different key on \
                decoding. Supply the secret out of band on the decoded state instead.",
                self.secret.len();
            Invalid, Input, Security));
        }
        Ok(())
    }

    /// Encodes the associated data, when present, as the `data` option used by the reference
    /// implementation, including the leading comma.
    fn encode_ad(&self) -> String {
        if self.ad.is_empty() {
            String::new()
        } else {
            fmt!(",data={}", base64::encode_config(&self.ad, base64::STANDARD_NO_PAD))
        }
    }

    fn extract_value<'a>(
        s:          &'a str,
        name:       &'static str,
        err_str:    &'static str,
    )
        -> Outcome<&'a str>
    {
        let parts: Vec<&str> = s.split('=').collect();
        if parts.len() == 2 {
            if parts[0] == name {
                Ok(parts[1])
            } else {
                Err(err!(
                    "The Argon2 {} substring key must be '{}', found '{}'.", err_str, name, parts[0];
                Missing, Decode, String, Invalid, Input))
            }
        } else {
            Err(err!(
                "The Argon2 {} substring '{}' should have 2 parts separated by the '=' \
                character. {} were found.", err_str, s, parts.len();
            Decode, String, Invalid, Input))
        }
    }

    /// Decodes the comma separated options substring.  The mandatory `m`, `t` and `p` come first,
    /// in that order, and may be followed by the optional `l` and `data`.  Any other option is an
    /// error, because an option we do not understand may well be one that changes the tag.
    fn extract_options(s: &str) -> Outcome<Argon2Options> {
        let parts: Vec<&str> = s.split(',').collect();
        if parts.len() < 3 {
            return Err(err!(
                "The Argon2 options substring '{}' should have at least the 3 parts 'm', 't' and \
                'p' separated by the ',' character. {} were found.", s, parts.len();
            Decode, String, Missing, Invalid, Input));
        }
        let mstr = res!(Self::extract_value(parts[0], "m", "mem_cost"));
        let tstr = res!(Self::extract_value(parts[1], "t", "time_cost"));
        let pstr = res!(Self::extract_value(parts[2], "p", "lanes"));
        let mut result = Argon2Options {
            mem_cost:       res!(mstr.parse::<u32>()),
            time_cost:      res!(tstr.parse::<u32>()),
            lanes:          res!(pstr.parse::<u32>()),
            hash_length:    None,
            ad:             Vec::new(),
        };
        let mut ad_seen = false;
        for part in &parts[3..] {
            let kv: Vec<&str> = part.split('=').collect();
            if kv.len() != 2 {
                return Err(err!(
                    "The Argon2 option '{}' should have 2 parts separated by the '=' character. \
                    {} were found.", part, kv.len();
                Decode, String, Invalid, Input));
            }
            match kv[0] {
                "l" => {
                    if result.hash_length.is_some() {
                        return Err(err!(
                            "The Argon2 options substring '{}' repeats the 'l' option.", s;
                        Decode, String, Duplicate, Invalid, Input));
                    }
                    result.hash_length = Some(res!(kv[1].parse::<u32>()));
                },
                "data" => {
                    if ad_seen {
                        return Err(err!(
                            "The Argon2 options substring '{}' repeats the 'data' option.", s;
                        Decode, String, Duplicate, Invalid, Input));
                    }
                    result.ad = res!(base64::decode(kv[1]));
                    ad_seen = true;
                },
                _ => return Err(err!(
                    "The Argon2 option key '{}' in options substring '{}' is not recognised. An \
                    unrecognised option may change the derived key, so it cannot be ignored.",
                    kv[0], s;
                Decode, String, Unknown, Invalid, Input)),
            }
        }
        Ok(result)
    }
}

/// The decoded options substring of an Argon2 encoded string.
struct Argon2Options {
    mem_cost:       u32,
    time_cost:      u32,
    lanes:          u32,
    hash_length:    Option<u32>, // Absent from a string that carries a hash.
    ad:             Vec<u8>,
}

impl KeyDeriver for KeyDerivationScheme {

    fn get_hash(&self) -> Outcome<&[u8]> {
        match self {
            Self::Argon2(state) => match &state.hash {
                Some(hash) => return Ok(&hash[..]),
                None => (),
            },
        }
        Err(err!(
            "{}: Expected hash to be be present, found none.", self;
        Data, Missing))
    }

    fn set_rand_salt(&mut self, n: usize) -> Outcome<()> {
        let mut salt = vec![0u8; n];
        Rand::fill_u8(&mut salt);
        match self {
            Self::Argon2(state) => state.salt = salt,
        }
        Ok(())
    }

    fn derive(&mut self, pass: &[u8]) -> Outcome<()> {
        match self {
            Self::Argon2(state) => match argon2::hash_raw(
                pass,
                &state.salt,
                &state.to_argon2_config(),
            ) {
                Ok(hash) => {
                    state.hash = Some(hash);
                    Ok(())
                },
                Err(e) => Err(err!(e, "While performing Argon2 hash."; Invalid, Input)),
            },
        }
    }

    fn verify(&self, pass: &[u8]) -> Outcome<bool> {
        match self {
            Self::Argon2(state) => match &state.hash {
                Some(hash) => Ok(res!(argon2::verify_raw(
                    pass,
                    &state.salt,
                    &hash,
                    &state.to_argon2_config(),
                ))),
                None => Err(err!("Hash has not been created."; Missing)),
            },
        }
    }

    /// Encodes the state and its hash in the PHC string format, which other Argon2 implementations
    /// can read.  The tag length is implied by the length of the encoded hash.
    fn encode_to_string(&self) -> Outcome<String> {
        match self {
            Self::Argon2(state) => match &state.hash {
                Some(hash) => {
                    // The encoding function is copied from argon2 to avoid using argon2::Context,
                    // which requires the password.
                    res!(state.require_no_secret());
                    if (hash.len() as u32) != state.hash_length {
                        return Err(err!(
                            "This Argon2 state declares a hash length of {} but holds a hash of \
                            {} bytes.", state.hash_length, hash.len();
                        Bug, Mismatch, Invalid));
                    }
                    Ok(fmt!(
                        "${}$v={}$m={},t={},p={}{}${}${}",
                        state.variant,
                        state.version,
                        state.mem_cost,
                        state.time_cost,
                        state.lanes,
                        state.encode_ad(),
                        base64::encode_config(&state.salt, base64::STANDARD_NO_PAD),
                        base64::encode_config(&hash, base64::STANDARD_NO_PAD),
                    ))
                },
                None => Err(err!("Hash has not been created."; Missing)),
            },
        }
    }

    /// Encodes the state sans hash.  Since PHC defines no hash-less form, and the tag length can
    /// no longer be implied by a hash, it is carried explicitly in the `l` option.
    fn encode_cfg_to_string(&self) -> Outcome<String> {
        match self {
            Self::Argon2(state) => {
                res!(state.require_no_secret());
                Ok(fmt!(
                    "${}$v={}$m={},t={},p={},l={}{}${}",
                    state.variant,
                    state.version,
                    state.mem_cost,
                    state.time_cost,
                    state.lanes,
                    state.hash_length,
                    state.encode_ad(),
                    base64::encode_config(&state.salt, base64::STANDARD_NO_PAD),
                ))
            },
        }
    }

    fn decode_from_string(&mut self, s: &str) -> Outcome<()> {
        match self {
            Self::Argon2(state) => state.from_argon2_string(s, true),
        }
    }

    fn decode_cfg_from_string(&mut self, s: &str) -> Outcome<()> {
        match self {
            Self::Argon2(state) => state.from_argon2_string(s, false),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct KeyDeriverDefAlt<
    D: KeyDeriver,
    G: KeyDeriver,
> (pub DefAlt<D, G>);

impl<
    D: KeyDeriver,
    G: KeyDeriver,
>
    std::ops::Deref for KeyDeriverDefAlt<D, G>
{
    type Target = DefAlt<D, G>;
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl<
    D: KeyDeriver,
    G: KeyDeriver,
>
    From<Option<G>> for KeyDeriverDefAlt<D, G>
{
    fn from(opt: Option<G>) -> Self {
        Self(
            DefAlt::from(opt),
        )
    }
}

impl<
    D: KeyDeriver,
    G: KeyDeriver,
>
    From<Alt<G>> for KeyDeriverDefAlt<D, G>
{
    fn from(alt: Alt<G>) -> Self {
        Self(
            DefAlt::from(alt),
        )
    }
}

impl<
    D: KeyDeriver + InNamex,
    G: KeyDeriver + InNamex,
>
    fmt::Display for KeyDeriverDefAlt<D, G>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl<
    D: KeyDeriver + InNamex,
    G: KeyDeriver + InNamex,
>
    InNamex for KeyDeriverDefAlt<D, G>
{
    fn name_id(&self) -> Outcome<NamexId> {
        match &self.0 {
            DefAlt::Default(inner) => inner.name_id(),
            DefAlt::Given(inner) => inner.name_id(),
            DefAlt::None => Err(err!(
                "No Namex id can be specified for DefAlt::None.";
            Missing, Bug)),
        }
    }

    fn local_id(&self) -> LocalId {
        match &self.0 {
            DefAlt::Default(inner)  => inner.local_id(),
            DefAlt::Given(inner)    => inner.local_id(),
            DefAlt::None            => LocalId::default(),
        }
    }

    fn assoc_names_base64(
        gname: &'static str,
    )
        -> Outcome<Option<Vec<(
            &'static str,
            &'static str,
        )>>>
    {
        match res!(D::assoc_names_base64(gname)) {
            Some(mut vd) => match res!(G::assoc_names_base64(gname)) {
                Some(vg) => {
                    vd.extend(vg);
                    Ok(Some(vd))
                },
                None => Ok(Some(vd)),
            },
            None => match res!(G::assoc_names_base64(gname)) {
                Some(vg) => Ok(Some(vg)),
                None => Ok(None),
            },
        }
    }
}

impl<
    D: KeyDeriver,
    G: KeyDeriver,
>
    KeyDeriver for KeyDeriverDefAlt<D, G>
{
    fn get_hash(&self) -> Outcome<&[u8]> {
        match &self.0 {
            DefAlt::Default(inner) => return inner.get_hash(),
            DefAlt::Given(inner) => return inner.get_hash(),
            DefAlt::None => (),
        }
        Err(err!(
            "{}: Expected hash to be be present, found none.", self;
        Data, Missing))
    }

    fn set_rand_salt(&mut self, n: usize) -> Outcome<()> {
        match &mut self.0 {
            DefAlt::Default(inner) => inner.set_rand_salt(n),
            DefAlt::Given(inner) => inner.set_rand_salt(n),
            DefAlt::None => Err(err!("{}", Self::KDF_MISSING_MSG;
                Configuration, Missing)),
        }
    }

    fn derive(&mut self, pass: &[u8]) -> Outcome<()> {
        match &mut self.0 {
            DefAlt::Default(inner) => inner.derive(pass),
            DefAlt::Given(inner) => inner.derive(pass),
            DefAlt::None => Err(err!("{}", Self::KDF_MISSING_MSG;
                Configuration, Missing)),
        }
    }

    fn verify(&self, pass: &[u8]) -> Outcome<bool> {
        match &self.0 {
            DefAlt::Default(inner) => inner.verify(pass),
            DefAlt::Given(inner) => inner.verify(pass),
            DefAlt::None => Err(err!("{}", Self::KDF_MISSING_MSG;
                Configuration, Missing)),
        }
    }

    fn encode_to_string(&self) -> Outcome<String> {
        match &self.0 {
            DefAlt::Default(inner) => inner.encode_to_string(),
            DefAlt::Given(inner) => inner.encode_to_string(),
            DefAlt::None => Err(err!("{}", Self::KDF_MISSING_MSG;
                Configuration, Missing)),
        }
    }

    fn encode_cfg_to_string(&self) -> Outcome<String> {
        match &self.0 {
            DefAlt::Default(inner) => inner.encode_cfg_to_string(),
            DefAlt::Given(inner) => inner.encode_cfg_to_string(),
            DefAlt::None => Err(err!("{}", Self::KDF_MISSING_MSG;
                Configuration, Missing)),
        }
    }

    fn decode_from_string(&mut self, s: &str) -> Outcome<()> {
        match &mut self.0 {
            DefAlt::Default(inner) => inner.decode_from_string(s),
            DefAlt::Given(inner) => inner.decode_from_string(s),
            DefAlt::None => Err(err!("{}", Self::KDF_MISSING_MSG;
                Configuration, Missing)),
        }
    }

    fn decode_cfg_from_string(&mut self, s: &str) -> Outcome<()> {
        match &mut self.0 {
            DefAlt::Default(inner) => inner.decode_cfg_from_string(s),
            DefAlt::Given(inner) => inner.decode_cfg_from_string(s),
            DefAlt::None => Err(err!("{}", Self::KDF_MISSING_MSG;
                Configuration, Missing)),
        }
    }
}

impl<
    D: KeyDeriver,
    G: KeyDeriver,
>
    KeyDeriverDefAlt<D, G>
{
    pub const KDF_MISSING_MSG: &'static str = "Key deriver function not specified.";

    pub fn or_get_hash<'a, OR: KeyDeriver>(&'a self, alt: &'a Alt<OR>) -> Outcome<&'a [u8]> {
        match &alt {
            Alt::Specific(Some(inner)) => return inner.get_hash(),
            Alt::Specific(None) => (),
            Alt::Unspecified => match &self.0 {
                DefAlt::Default(inner) => return inner.get_hash(),
                DefAlt::Given(inner) => return inner.get_hash(),
                DefAlt::None => (),
            },
        }
        Err(err!(
            "{}: Expected hash to be be present, found none.", self;
        Data, Missing))
    }

    pub fn or_set_rand_salt<OR: KeyDeriver>(&mut self, n: usize, mut alt: &mut Alt<OR>) -> Outcome<()> {
        match &mut alt {
            Alt::Specific(Some(inner)) => inner.set_rand_salt(n),
            Alt::Specific(None) => Err(err!("{}", Self::KDF_MISSING_MSG;
                Configuration, Missing)),
            Alt::Unspecified => match &mut self.0 {
                DefAlt::Default(inner) => inner.set_rand_salt(n),
                DefAlt::Given(inner) => inner.set_rand_salt(n),
                DefAlt::None => Err(err!("{}", Self::KDF_MISSING_MSG;
                    Configuration, Missing)),
            },
        }
    }

    pub fn or_derive<OR: KeyDeriver>(&mut self, pass: &[u8], mut alt: &mut Alt<OR>) -> Outcome<()> {
        match &mut alt {
            Alt::Specific(Some(inner)) => inner.derive(pass),
            Alt::Specific(None) => Err(err!("{}", Self::KDF_MISSING_MSG;
                Configuration, Missing)),
            Alt::Unspecified => match &mut self.0 {
                DefAlt::Default(inner) => inner.derive(pass),
                DefAlt::Given(inner) => inner.derive(pass),
                DefAlt::None => Err(err!("{}", Self::KDF_MISSING_MSG;
                    Configuration, Missing)),
            },
        }
    }

    pub fn or_verify<OR: KeyDeriver>(&self, pass: &[u8], alt: &Alt<OR>) -> Outcome<bool> {
        match &alt {
            Alt::Specific(Some(inner)) => inner.verify(pass),
            Alt::Specific(None) => Err(err!("{}", Self::KDF_MISSING_MSG;
                Configuration, Missing)),
            Alt::Unspecified => match &self.0 {
                DefAlt::Default(inner) => inner.verify(pass),
                DefAlt::Given(inner) => inner.verify(pass),
                DefAlt::None => Err(err!("{}", Self::KDF_MISSING_MSG;
                    Configuration, Missing)),
            },
        }
    }

    pub fn or_encode_to_string<OR: KeyDeriver>(&self, alt: &Alt<OR>) -> Outcome<String> {
        match &alt {
            Alt::Specific(Some(inner)) => inner.encode_to_string(),
            Alt::Specific(None) => Err(err!("{}", Self::KDF_MISSING_MSG;
                Configuration, Missing)),
            Alt::Unspecified => match &self.0 {
                DefAlt::Default(inner) => inner.encode_to_string(),
                DefAlt::Given(inner) => inner.encode_to_string(),
                DefAlt::None => Err(err!("{}", Self::KDF_MISSING_MSG;
                    Configuration, Missing)),
            },
        }
    }

    pub fn or_encode_cfg_to_string<OR: KeyDeriver>(&self, alt: &Alt<OR>) -> Outcome<String> {
        match &alt {
            Alt::Specific(Some(inner)) => inner.encode_cfg_to_string(),
            Alt::Specific(None) => Err(err!("{}", Self::KDF_MISSING_MSG;
                Configuration, Missing)),
            Alt::Unspecified => match &self.0 {
                DefAlt::Default(inner) => inner.encode_cfg_to_string(),
                DefAlt::Given(inner) => inner.encode_cfg_to_string(),
                DefAlt::None => Err(err!("{}", Self::KDF_MISSING_MSG;
                    Configuration, Missing)),
            },
        }
    }

    pub fn or_decode_from_string<OR: KeyDeriver>(&mut self, s: &str, mut alt: &mut Alt<OR>) -> Outcome<()> {
        match &mut alt {
            Alt::Specific(Some(inner)) => inner.decode_from_string(s),
            Alt::Specific(None) => Err(err!("{}", Self::KDF_MISSING_MSG;
                Configuration, Missing)),
            Alt::Unspecified => match &mut self.0 {
                DefAlt::Default(inner) => inner.decode_from_string(s),
                DefAlt::Given(inner) => inner.decode_from_string(s),
                DefAlt::None => Err(err!("{}", Self::KDF_MISSING_MSG;
                    Configuration, Missing)),
            },
        }
    }

    pub fn or_decode_cfg_from_string<OR: KeyDeriver>(&mut self, s: &str, mut alt: &mut Alt<OR>) -> Outcome<()> {
        match &mut alt {
            Alt::Specific(Some(inner)) => inner.decode_from_string(s),
            Alt::Specific(None) => Err(err!("{}", Self::KDF_MISSING_MSG;
                Configuration, Missing)),
            Alt::Unspecified => match &mut self.0 {
                DefAlt::Default(inner) => inner.decode_from_string(s),
                DefAlt::Given(inner) => inner.decode_from_string(s),
                DefAlt::None => Err(err!("{}", Self::KDF_MISSING_MSG;
                    Configuration, Missing)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    // The Argon2id test vector of RFC 9106, section 5.3.  It is the only vector here that pins the
    // lane count, the secret and the associated data, because it is the only one that uses them.
    const RFC9106_PASS:     [u8; 32] = [0x01; 32];
    const RFC9106_SALT:     [u8; 16] = [0x02; 16];
    const RFC9106_SECRET:   [u8; 8]  = [0x03; 8];
    const RFC9106_AD:       [u8; 12] = [0x04; 12];
    const RFC9106_TAG:      [u8; 32] = [
        0x0d, 0x64, 0x0d, 0xf5, 0x8d, 0x78, 0x76, 0x6c,
        0x08, 0xc0, 0x37, 0xa3, 0x4a, 0x8b, 0x53, 0xc9,
        0xd0, 0x1e, 0xf0, 0x45, 0x2d, 0x75, 0xb6, 0x5e,
        0xb5, 0x25, 0x20, 0xe9, 0x6b, 0x01, 0xe6, 0x59,
    ];

    /// The RFC 9106 section 5.3 inputs: 32 KiB of memory, 3 passes and, critically, 4 lanes.
    fn rfc9106_state() -> Argon2State {
        Argon2State {
            ad:             RFC9106_AD.to_vec(),
            hash_length:    32,
            lanes:          4,
            mem_cost:       32,
            secret:         RFC9106_SECRET.to_vec(),
            time_cost:      3,
            variant:        argon2::Variant::Argon2id,
            version:        argon2::Version::Version13,
            salt:           RFC9106_SALT.to_vec(),
            hash:           None,
        }
    }

    /// Pins our Argon2id output against the published RFC 9106 tag.
    #[test]
    fn test_argon2_rfc9106_vector() -> Outcome<()> {
        let mut kdf = KeyDerivationScheme::Argon2(rfc9106_state());
        res!(kdf.derive(&RFC9106_PASS));
        req!(res!(kdf.get_hash()), &RFC9106_TAG[..]);
        Ok(())
    }

    /// Pins the configuration string round trip against the published RFC 9106 tag.  Should the
    /// lane count or the tag length be dropped in encoding or decoding, the derived key differs
    /// from the vector and this fails.
    #[test]
    fn test_argon2_rfc9106_cfg_round_trip() -> Outcome<()> {
        let mut state = rfc9106_state();
        state.secret = Vec::new(); // The secret is never encoded; it is supplied out of band.
        let kdf = KeyDerivationScheme::Argon2(state);
        let encoded_cfg = res!(kdf.encode_cfg_to_string());
        msg!("encoded cfg = '{}'", encoded_cfg);
        // A fresh scheme carrying the defaults, lanes = 1 and hash_length = 32, which the decoded
        // configuration must overwrite.
        let mut kdf2 = res!(KeyDerivationScheme::default_argon2("Argon2id", 0x13));
        res!(kdf2.decode_cfg_from_string(&encoded_cfg));
        match &mut kdf2 {
            KeyDerivationScheme::Argon2(state) => {
                req!(state.lanes, 4, "The lane count did not survive the round trip.");
                req!(state.hash_length, 32);
                req!(state.mem_cost, 32);
                req!(state.time_cost, 3);
                req!(state.version, argon2::Version::Version13);
                req!(&state.ad[..], &RFC9106_AD[..]);
                req!(&state.salt[..], &RFC9106_SALT[..]);
                state.secret = RFC9106_SECRET.to_vec(); // Supplied out of band, as documented.
            },
        }
        res!(kdf2.derive(&RFC9106_PASS));
        req!(res!(kdf2.get_hash()), &RFC9106_TAG[..]);
        Ok(())
    }

    /// A foreign PHC string, produced elsewhere with 4 lanes, must verify against its password.
    /// Dropping the lanes on decoding derives a different tag, and the verification fails.
    #[test]
    fn test_argon2_decode_foreign_phc_string() -> Outcome<()> {
        let encoded = "$argon2i$v=19$m=4096,t=3,p=4$YWJjZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXo\
            $BvBk2OaSofBHfbrUW61nHrWB/43xgfs/QJJ5DkMAd8I";
        let mut kdf = res!(KeyDerivationScheme::default_argon2("Argon2i", 0x13));
        res!(kdf.decode_from_string(encoded));
        match &kdf {
            KeyDerivationScheme::Argon2(state) => {
                req!(state.lanes, 4, "The lane count was dropped on decoding.");
                req!(state.mem_cost, 4096);
                req!(state.time_cost, 3);
                req!(state.hash_length, 32);
                req!(&state.salt[..], &b"abcdefghijklmnopqrstuvwxyz"[..]);
            },
        }
        req!(res!(kdf.verify(b"foo")), true);
        req!(res!(kdf.verify(b"bar")), false);
        Ok(())
    }

    /// Our PHC output must be readable by an independent Argon2 parser, here the wrapped crate's
    /// own, which we otherwise never exercise.
    #[test]
    fn test_argon2_phc_string_is_externally_readable() -> Outcome<()> {
        let pass = b"The meaning is 42";
        let state = res!(Argon2State::new("Argon2id", 0x13, 1024, 2, 4, 32, b"somesalt".to_vec()));
        let mut kdf = KeyDerivationScheme::Argon2(state);
        res!(kdf.derive(pass));
        let encoded = res!(kdf.encode_to_string());
        msg!("encoded with hash = '{}'", encoded);
        req!(encoded.starts_with("$argon2id$v=19$m=1024,t=2,p=4$"), true, "Not a PHC string.");
        match argon2::verify_encoded(&encoded, pass) {
            Ok(true) => (),
            Ok(false) => return Err(err!(
                "An independent Argon2 parser read our encoded string but did not verify \
                the password against it."; Test, Mismatch)),
            Err(e) => return Err(err!(e,
                "An independent Argon2 parser could not read our encoded string."; Test, Decode)),
        }
        Ok(())
    }

    /// A configuration string written by the previous release must still decode, and must still
    /// derive the key it derived then.
    ///
    /// The deployed encoder wrote `$argon2id$v=19$m=..,t=..,p=..$<salt>` and no `l` option, because
    /// it never wrote the tag length at all.  Every `kdf_cfg` in a wallet in the field has that
    /// shape, and a wallet is the one artefact that cannot be rebuilt from source: refusing to read
    /// it would lock every admin out of their own master key.  So a missing `l` is not an error, it
    /// is the old format, and the tag length falls back to the one the state already carries.
    ///
    /// The expected tag comes from the wrapped crate's own hasher rather than from ours, so this
    /// pins the fallback to what the old string actually meant, not to what we now think it means.
    #[test]
    fn test_a_cfg_string_from_the_previous_release_still_decodes() -> Outcome<()> {
        let pass = b"The meaning is 42";
        let salt = b"somesalt";
        // Exactly what the previous release's `encode_cfg_to_string` emitted: no 'l' option.
        let old_cfg = fmt!(
            "$argon2id$v=19$m=1024,t=2,p=1${}",
            base64::encode_config(salt, base64::STANDARD_NO_PAD),
        );

        let mut kdf = res!(KeyDerivationScheme::default_argon2("Argon2id", 0x13));
        res!(kdf.decode_cfg_from_string(&old_cfg));
        res!(kdf.derive(pass));
        let ours = res!(kdf.get_hash()).to_vec();

        // The independent oracle: the wrapped crate, told the same parameters by hand.
        let cfg = argon2::Config {
            variant:        argon2::Variant::Argon2id,
            version:        argon2::Version::Version13,
            mem_cost:       1024,
            time_cost:      2,
            lanes:          1,
            hash_length:    32,
            ..argon2::Config::default()
        };
        let theirs = res!(argon2::hash_raw(pass, salt, &cfg));

        req!(ours, theirs,
            "A cfg string from the previous release derived a different key than it used to, \
            which is how a wallet locks its owner out.");
        Ok(())
    }

    #[test]
    fn test_argon2_encode_pass() -> Outcome<()> {
        // Here we store the hash along with the hasher config.
        let pass = b"The meaning is 42";
        let mut kdf = res!(KeyDerivationScheme::default_argon2("Argon2id", 0x13));
        res!(kdf.derive(pass));
        let encoded_cfg = res!(kdf.encode_cfg_to_string());
        let encoded_with_hash = res!(kdf.encode_to_string());
        let hash = res!(kdf.get_hash());
        msg!("hash = '{:02x?}'", hash);
        msg!("encoded cfg = '{}'", encoded_cfg);
        msg!("encoded with hash = '{}'", encoded_with_hash);
        req!(res!(kdf.verify(pass)), true);
        req!(res!(kdf.verify(b"The meaning is 43")), false,
            "The verification should have failed for a differing passphrase.");
        let mut kdf2 = res!(KeyDerivationScheme::default_argon2("Argon2id", 0x13));
        res!(kdf2.decode_from_string(&encoded_with_hash));
        req!(kdf, kdf2);
        let mut kdf3 = res!(KeyDerivationScheme::default_argon2("Argon2id", 0x13));
        res!(kdf3.decode_cfg_from_string(&encoded_cfg));
        res!(kdf3.derive(pass));
        req!(kdf, kdf3);
        Ok(())
    }

    /// A configuration string carrying neither a hash nor an explicit tag length keeps the tag
    /// length the receiving state already holds.
    ///
    /// That is the old format, and it is what every deployed wallet contains, so it must decode.
    /// New strings do not rely on the fallback: `encode_cfg_to_string` always writes `l`, which is
    /// what makes them self-describing.  See
    /// `test_a_cfg_string_from_the_previous_release_still_decodes` for the key it must derive.
    #[test]
    fn test_argon2_decode_without_a_tag_length_keeps_the_states_own() -> Outcome<()> {
        let mut kdf = res!(KeyDerivationScheme::default_argon2("Argon2id", 0x13));
        let expected = match &kdf {
            KeyDerivationScheme::Argon2(state) => state.hash_length,
        };
        res!(kdf.decode_cfg_from_string("$argon2id$v=19$m=65536,t=5,p=1$c29tZXNhbHQ"));
        match &kdf {
            KeyDerivationScheme::Argon2(state) => {
                req!(state.hash_length, expected,
                    "An old configuration string changed the tag length it derives with.");
                req!(state.lanes, 1);
            },
        }
        Ok(())
    }

    /// An option we do not understand may be one that changes the derived key, so it is rejected.
    #[test]
    fn test_argon2_decode_rejects_unknown_option() -> Outcome<()> {
        let mut kdf = res!(KeyDerivationScheme::default_argon2("Argon2id", 0x13));
        match kdf.decode_cfg_from_string("$argon2id$v=19$m=65536,t=5,p=1,l=32,x=9$c29tZXNhbHQ") {
            Ok(()) => Err(err!(
                "A configuration string with an unknown option should not decode.";
            Test, Unexpected)),
            Err(_) => Ok(()),
        }
    }

    /// The lane count is mandatory, not optional.
    #[test]
    fn test_argon2_decode_rejects_missing_lanes() -> Outcome<()> {
        let mut kdf = res!(KeyDerivationScheme::default_argon2("Argon2id", 0x13));
        match kdf.decode_cfg_from_string("$argon2id$v=19$m=65536,t=5,l=32$c29tZXNhbHQ") {
            Ok(()) => Err(err!(
                "A configuration string with no lane count should not decode."; Test, Unexpected)),
            Err(_) => Ok(()),
        }
    }

    /// A secret is never written to an encoded string, so encoding one must fail loudly rather
    /// than drop it, which would derive a different key on decoding.
    #[test]
    fn test_argon2_encode_refuses_to_drop_secret() -> Outcome<()> {
        let kdf = KeyDerivationScheme::Argon2(rfc9106_state());
        match kdf.encode_cfg_to_string() {
            Ok(s) => Err(err!(
                "Encoding a state holding a secret should have failed, but produced '{}'.", s;
            Test, Unexpected)),
            Err(_) => Ok(()),
        }
    }

    /// A simple test of viability for a network packet header proof of work.
    #[test]
    fn test_argon2_pow() -> Outcome<()> {
        // Deliberately cheap parameters: this measures viability, not key strength.
        let mut kdf = res!(KeyDerivationScheme::new_argon2("Argon2i", 0x13, 512, 1, 16, 32));
        let prefix = [1u8]; // One byte, so around 256 derivations are expected.
        let lim: usize = 20_000;
        let mut count: usize = 0;
        let t = res!(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH));
        let mut tstamp = t.as_secs().to_be_bytes().to_vec();
        let mut pass = vec![192u8, 168, 0, 1, 127, 0, 0, 1];
        pass.append(&mut tstamp);
        loop {
            res!(kdf.derive(&pass));
            let hash = res!(kdf.get_hash());
            if hash.starts_with(&prefix) { break; }
            count += 1;
            if count > lim {
                return Err(err!(
                    "No proof of work found for a {} byte prefix in {} derivations.",
                    prefix.len(), lim;
                Test, Excessive));
            }
            res!(kdf.set_rand_salt(16));
        }
        msg!("Proof of work found after {} failed derivations.", count);
        Ok(())
    }

}
