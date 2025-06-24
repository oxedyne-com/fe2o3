//! This crate provides a concrete interface for multiple Key Derivation Function algorithms.
//! Currently it uses only one, Argon2.  The Argon2 implementation wrapped here, `rust-argon2`
//! unexpectedly produces different hash values for different lane values, necessitating a lane
//! value of one for portability.  Even so, the Open Worldwide Application Security Project (OWASP)
//! recommends using Argon2id with one lane of parallelism.
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

    pub fn new(
        variant:        &str,
        version:        u32,
        mem_cost:       u32,
        time_cost:      u32,
        hash_length:    u32,
        salt:           Vec<u8>,
    )
        -> Outcome<Self>
    {
        Ok(Self {
            hash_length,
            lanes:          1,
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
    
    /// Unfortunately the argon2::encoding module is private so the decoding functionality must be
    /// replicated here.
    pub fn from_argon2_string(
        &mut self,
        encoded:        &str,
        expect_hash:    bool,
    )
        -> Outcome<()>
    {
        let encoded = match encoded.strip_prefix('$') {
            Some(s) => s,
            None => return Err(err!(
                "Encoded Argon2 string '{}' does not begin with a '$'.", encoded;
            Invalid, Input)),
        };
        let (n_complete, n_v0x10) = if expect_hash { (5, 4) } else { (4, 3) };
        let parts: Vec<&str> = encoded.split('$').collect();
        if parts.len() == n_complete {
            let options = res!(Self::extract_options(parts[2]));
            self.variant = res!(argon2::Variant::from_str(parts[0]));
            let vstr = res!(Self::extract_value(parts[1], "v", "version"));
            self.version = res!(argon2::Version::from_str(vstr));
            let mstr = res!(Self::extract_value(options[0], "m", "mem_cost"));
            self.mem_cost = res!(mstr.parse::<u32>());
            let tstr = res!(Self::extract_value(options[1], "t", "time_cost"));
            self.time_cost = res!(tstr.parse::<u32>());
            self.salt = res!(base64::decode(parts[3]));
            if expect_hash {
                self.hash = Some(res!(base64::decode(parts[4])));
            }
            Ok(())
        } else if parts.len() == n_v0x10 {
            let options = res!(Self::extract_options(parts[1]));
            self.variant = res!(argon2::Variant::from_str(parts[0]));
            self.version = argon2::Version::Version10;
            let mstr = res!(Self::extract_value(options[0], "m", "mem_cost"));
            self.mem_cost = res!(mstr.parse::<u32>());
            let tstr = res!(Self::extract_value(options[1], "t", "time_cost"));
            self.time_cost = res!(tstr.parse::<u32>());
            self.salt = res!(base64::decode(parts[2]));
            if expect_hash {
                self.hash = Some(res!(base64::decode(parts[3])));
            }
            Ok(())
        } else {
            Err(err!(
                "The Argon2 string should have {} or {} parts separated by the '$' \
                character. {} were found.", parts.len() - 1, n_v0x10, n_complete;
            Decode, String, Invalid, Input))
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
                    "The Argon2 {} substring key must be '{}'.", err_str, name;
                Missing, Decode, String, Invalid, Input))
            }
        } else {
            Err(err!(
                "The Argon2 {} substring '{}' should have 2 parts separated by the '=' \
                character. {} were found.", err_str, s, parts.len();
            Decode, String, Invalid, Input))
        }
    }

    fn extract_options(s: &str) -> Outcome<Vec<&str>> {
        let parts: Vec<&str> = s.split(',').collect();
        if parts.len() == 3 {
            Ok(parts)
        } else {
            Err(err!(
                "The Argon2 options substring '{}' should have 3 parts separated by the ',' \
                character. {} were found.", s, parts.len();
            Decode, String, Invalid, Input))
        }
    }
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

    fn encode_to_string(&self) -> Outcome<String> {
        match self {
            Self::Argon2(state) => match &state.hash {
                Some(hash) => Ok(fmt!(
                    // Copy the encoding function from argon2 to avoid using argon2::Context
                    // which requires the password.
                    "{}${}",
                    res!(self.encode_cfg_to_string()),
                    base64::encode_config(&hash, base64::STANDARD_NO_PAD),
                )),
                None => Err(err!("Hash has not been created."; Missing)),
            },
        }
    }

    fn encode_cfg_to_string(&self) -> Outcome<String> {
        match self {
            Self::Argon2(state) => Ok(fmt!(
                // Copy the encoding function from argon2 to avoid using argon2::Context
                // which requires the password.
                "${}$v={}$m={},t={},p={}${}",
                state.variant,
                state.version,
                state.mem_cost,
                state.time_cost,
                state.lanes,
                base64::encode_config(&state.salt, base64::STANDARD_NO_PAD),
            )),
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

    #[test]
    fn test_argon2_encode_pass() -> Outcome<()> {
        // Here we store the hash along with the hasher config.
        let pass = b"The meaning is 42";
        let mut kdf = res!(KeyDerivationScheme::default_argon2("Argon2id", 0x13));
        res!(kdf.derive(pass));
        let encoded_cfg = res!(kdf.encode_cfg_to_string());
        let encoded_with_hash = res!(kdf.encode_to_string());
        let hash = res!(kdf.get_hash().ok_or(err!("Missing hash."; Bug, Missing)));
        msg!("hash = '{:02x?}'", hash);
        msg!("encoded cfg = '{}'", encoded_cfg);
        msg!("encoded with hash = '{}'", encoded_with_hash);
        match kdf.verify(b"The meaning is 43") {
            Ok(()) => return Err(err!(
                "The verification should have failed for a differing passphrase.";
            Test, Unexpected)),
            Err(_) => (),
        }
        let mut kdf2 = res!(KeyDerivationScheme::default_argon2("Argon2id", 0x13));
        res!(kdf2.decode_from_string(&encoded_with_hash));
        assert_eq!(kdf, kdf2);
        let mut kdf3 = res!(KeyDerivationScheme::default_argon2("Argon2id", 0x13));
        res!(kdf3.decode_cfg_from_string(&encoded_cfg));
        res!(kdf3.derive(pass));
        assert_eq!(kdf, kdf3);
        Ok(())
    }
    
    /// A simple test of viability for a network packet header proof of work.
    #[test]
    fn test_argon2_pow() -> Outcome<()> {
        let mut kdf = res!(KeyDerivationScheme::default_argon2("Argon2i", 0x13));
        let prefix = [1u8, 2];
        let mut count: usize = 0;
        let t = res!(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH));
        let mut tstamp = t.as_secs().to_be_bytes().to_vec();
        let mut pass = vec![192u8, 168, 0, 1, 127, 0, 0, 1];
        pass.append(&mut tstamp);
        loop { 
            res!(kdf.derive(&pass));
            let hash = res!(kdf.get_hash().ok_or(err!("Missing hash."; Bug, Missing)));
            msg!("count = {} hash = {:02x?}", count, &hash[0..prefix.len()]);
            if hash.starts_with(&prefix) { break; }
            count += 1;
            res!(kdf.set_rand_salt(16));
        }
        msg!("Success.");
        Ok(())
    }

}
