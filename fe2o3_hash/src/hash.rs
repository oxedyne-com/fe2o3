use oxedyne_fe2o3_core::{
    prelude::*,
    alt::{
        Alt,
        DefAlt,
        Override,
        Gnomon,
    },
};
use oxedyne_fe2o3_iop_hash::api::{
    Hash,
    HashForm,
    Hasher,
};
use oxedyne_fe2o3_namex::{
    id::{
        LocalId,
        InNamex,
        NamexId,
    },
};

use std::{
    fmt,
    hash::Hasher as _,
    str,
};

use tiny_keccak::{
    self as keccak,
    Hasher as _,
};

#[derive(Clone)]
pub enum HashScheme {
    // Crypto
    SHA3_256(keccak::Sha3),
    // Non-crypto
    Seahash(seahash::SeaHasher),
}

impl fmt::Debug for HashScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SHA3_256(..) => write!(f, "SHA3_256"),
            Self::Seahash(..) => write!(f, "Seahash"),
        }
    }
}

impl InNamex for HashScheme {

    fn name_id(&self) -> Outcome<NamexId> {
	    Ok(match self {
            Self::SHA3_256(..) =>
                res!(NamexId::try_from("VybbHNWeNXeTqTrXj66TzZScbSTsEFVy0W79QnbroFA=")),
            Self::Seahash(..) =>
                res!(NamexId::try_from("O/3zgxf8/f6mjc0RBau1MMtkfNi9B1eeFB5Q9f6ZfAM=")),
        })
    }

    fn local_id(&self) -> LocalId {
	    match self {
            Self::SHA3_256(..)  => LocalId(1),
            Self::Seahash(..)   => LocalId(2),
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
	            ("SHA3_256", "VybbHNWeNXeTqTrXj66TzZScbSTsEFVy0W79QnbroFA="),
	            ("Seahash", "O/3zgxf8/f6mjc0RBau1MMtkfNi9B1eeFB5Q9f6ZfAM="),
            ],
            _ => return Err(err!(
                "The Namex group name '{}' is not recognised for HashScheme.", gname;
            Invalid, Input)),
        };
        Ok(if ids.len() == 0 {
            None
        } else {
            Some(ids.to_vec())
        })
    }
}

impl Hasher for HashScheme {

    /// Absorbs the input slices in order and the salt last, so that the digest is `H(input ‖
    /// salt)`.  This ordering is fixed, and pinned by test; changing it changes every digest.
    fn hash<const S: usize>(self, input: &[&[u8]], salt: [u8; S]) -> Hash<S> {
        match self {
            Self::SHA3_256(mut hasher) => {
                for slice in input {
                    hasher.update(slice);
                }
                hasher.update(&salt);
                let mut hash = [0u8; 32];
                hasher.finalize(&mut hash);
                Hash::new(HashForm::Bytes32(hash), salt)
            },
            Self::Seahash(mut hasher) => {
                for slice in input {
                    hasher.write(slice);
                }
                hasher.write(&salt);
                let h = hasher.finish();
                Hash::new(HashForm::U64(h), salt)
            }
        }
    }

    fn hash_length(&self) -> Gnomon<usize> {
        match self {
            Self::SHA3_256(..)  => Gnomon::Known(Self::SHA3_256_BYTE_LEN),
            Self::Seahash(..)   => Gnomon::Known(Self::SEAHASH_BYTE_LEN),
        }
    }

    fn is_identity(&self) -> bool { false }
}

impl str::FromStr for HashScheme {
    type Err = Error<ErrTag>;

    fn from_str(name: &str) -> std::result::Result<Self, Self::Err> {
        match name {
            "SHA3_256" => Ok(Self::new_sha3_256()),
            "Seahash" => Ok(Self::new_seahash()),
            _ => Err(err!(
                "The hash scheme '{}' is not recognised.", name;
            Invalid, Input)),
        }
    }
}

impl TryFrom<&str> for HashScheme {
    type Error = Error<ErrTag>;

    fn try_from(s: &str) -> std::result::Result<Self, Self::Error> {
        Self::from_str(s)
    }
}

impl TryFrom<LocalId> for HashScheme {
    type Error = Error<ErrTag>;

    fn try_from(n: LocalId) -> std::result::Result<Self, Self::Error> {
        match n {
            LocalId(1) => Ok(Self::new_sha3_256()),
            LocalId(2) => Ok(Self::new_seahash()),
            _ => Err(err!(
                "The hash scheme with local id {} is not recognised.", n;
            Invalid, Input)),
        }
    }
}

impl HashScheme {

    pub const SEAHASH_BYTE_LEN:     usize   = 8;
    pub const SHA3_256_BYTE_LEN:    usize   = 32;

    pub fn new_sha3_256() -> Self {
        Self::SHA3_256(keccak::Sha3::v256())
    }

    pub fn new_seahash() -> Self {
        Self::Seahash(seahash::SeaHasher::new())
    }
}

#[derive(Clone, Debug, Default)]
pub struct HasherDefAlt<
    D: Hasher,
    G: Hasher,
>(
    pub DefAlt<D, G>
);

impl<
    D: Hasher,
    G: Hasher,
>
    std::ops::Deref for HasherDefAlt<D, G>
{
    type Target = DefAlt<D, G>;
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl<
    D: Hasher,
    G: Hasher,
>
    From<Option<G>> for HasherDefAlt<D, G>
{
    fn from(opt: Option<G>) -> Self {
        Self(
            DefAlt::from(opt),
        )
    }
}

impl<
    D: Hasher,
    G: Hasher,
>
    From<Alt<G>> for HasherDefAlt<D, G>
{
    fn from(alt: Alt<G>) -> Self {
        Self(
            DefAlt::from(alt),
        )
    }
}

impl<
    D: Hasher,
    G: Hasher,
>
    fmt::Display for HasherDefAlt<D, G>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl<
    D: Hasher + InNamex,
    G: Hasher + InNamex,
>
    InNamex for HasherDefAlt<D, G>
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
    D: Hasher,
    G: Hasher,
>
    Hasher for HasherDefAlt<D, G>
{
    fn hash<const S: usize>(self, input: &[&[u8]], salt: [u8; S]) -> Hash<S> {
        match self.0 {
            DefAlt::Default(inner)  => inner.hash(input, salt),
            DefAlt::Given(inner)    => inner.hash(input, salt),
            DefAlt::None            => ().hash(input, salt),
        }
    }

    fn hash_length(&self) -> Gnomon<usize> {
        match &self.0 {
            DefAlt::Default(inner)  => inner.hash_length(),
            DefAlt::Given(inner)    => inner.hash_length(),
            DefAlt::None            => ().hash_length(),
        }
    }

    fn is_identity(&self) -> bool {
        match &self.0 {
            DefAlt::Default(inner)  => inner.is_identity(),
            DefAlt::Given(inner)    => inner.is_identity(),
            DefAlt::None            => true,
        }
    }
}

impl<
    D: Hasher,
    G: Hasher,
>
    HasherDefAlt<D, G>
{
    pub const HASHER_MISSING_MSG: &'static str = "Hasher not specified.";

    /// Possibly override the hasher in `HasherDefAlt`.
    pub fn or_hash<
        const S: usize,
    >(
        &self,
        input:  &[&[u8]],
        salt:   [u8; S],
        or:     Option<&Override<D, G>>,
    )
        -> Hash<S>
    {
        match or {
            None | Some(Override::PassThrough)  => self.clone().hash(input, salt),
            Some(Override::Default(inner))      => inner.clone().hash(input, salt),
            Some(Override::Given(inner))        => inner.clone().hash(input, salt),
            Some(Override::None)                => ().hash(input, salt),
        }
    }

    pub fn or_hash_length(
        &self,
        or: Option<&Override<D, G>>,
    )
        -> Gnomon<usize>
    {
        match or {
            None | Some(Override::PassThrough)  => self.hash_length(),
            Some(Override::Default(inner))      => inner.hash_length(),
            Some(Override::Given(inner))        => inner.hash_length(),
            Some(Override::None)                => ().hash_length(),
        }
    }

    pub fn or_is_identity(
        &self,
        or: Option<&Override<D, G>>,
    )
        -> bool
    {
        match or {
            None | Some(Override::PassThrough)  => self.is_identity(),
            Some(Override::Default(inner))      => inner.is_identity(),
            Some(Override::Given(inner))        => inner.is_identity(),
            Some(Override::None)                => true,
        }
    }

    pub fn or_debug(
        &self,
        or: Option<&Override<D, G>>,
    )
        -> String
    {
        match or {
            None | Some(Override::PassThrough) => fmt!("{:?}", self),
            _ => fmt!("{:?}", or),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use oxedyne_fe2o3_iop_hash::api::HashForm;

    /// The known answer for SHA3-256("hello world").
    const HELLO_WORLD: [u8; 32] = [
        0x64, 0x4b, 0xcc, 0x7e, 0x56, 0x43, 0x73, 0x04,
        0x09, 0x99, 0xaa, 0xc8, 0x9e, 0x76, 0x22, 0xf3,
        0xca, 0x71, 0xfb, 0xa1, 0xd9, 0x72, 0xfd, 0x94,
        0xa3, 0x1c, 0x3b, 0xfb, 0xf2, 0x4e, 0x39, 0x38,
    ];

    /// Unwraps the digest of a SHA3-256 hash, which is always 32 bytes.
    fn digest<const S: usize>(hash: Hash<S>) -> Outcome<[u8; 32]> {
        match hash.as_hashform() {
            HashForm::Bytes32(a32) => Ok(a32),
            other => Err(err!(
                "Expected SHA3-256 to produce a HashForm::Bytes32, found {:?}.", other;
            Test, Mismatch)),
        }
    }

    /// SHA3-256 short message vectors from the NIST Cryptographic Algorithm Validation Programme,
    /// SHA3_256ShortMsg.rsp, plus the widely published digest of "abc".
    #[test]
    fn test_sha3_256_nist_cavp_short_msg() -> Outcome<()> {
        let vectors: [(&[u8], [u8; 32]); 5] = [
            // Len = 0.
            (&[], [
                0xa7, 0xff, 0xc6, 0xf8, 0xbf, 0x1e, 0xd7, 0x66,
                0x51, 0xc1, 0x47, 0x56, 0xa0, 0x61, 0xd6, 0x62,
                0xf5, 0x80, 0xff, 0x4d, 0xe4, 0x3b, 0x49, 0xfa,
                0x82, 0xd8, 0x0a, 0x4b, 0x80, 0xf8, 0x43, 0x4a,
            ]),
            // Len = 8.
            (&[0xe9], [
                0xf0, 0xd0, 0x4d, 0xd1, 0xe6, 0xcf, 0xc2, 0x9a,
                0x44, 0x60, 0xd5, 0x21, 0x79, 0x68, 0x52, 0xf2,
                0x5d, 0x9e, 0xf8, 0xd2, 0x8b, 0x44, 0xee, 0x91,
                0xff, 0x5b, 0x75, 0x9d, 0x72, 0xc1, 0xe6, 0xd6,
            ]),
            // Len = 16.
            (&[0xd4, 0x77], [
                0x94, 0x27, 0x9e, 0x8f, 0x5c, 0xcd, 0xf6, 0xe1,
                0x7f, 0x29, 0x2b, 0x59, 0x69, 0x8a, 0xb4, 0xe6,
                0x14, 0xdf, 0xe6, 0x96, 0xa4, 0x6c, 0x46, 0xda,
                0x78, 0x30, 0x5f, 0xc6, 0xa3, 0x14, 0x6a, 0xb7,
            ]),
            // Len = 32.
            (&[0xb0, 0x53, 0xfa, 0x1d], [
                0xbb, 0x86, 0x2f, 0x25, 0xe1, 0x0d, 0x09, 0x3f,
                0xae, 0x21, 0xea, 0xd5, 0xb4, 0xa2, 0xb3, 0xc5,
                0x4a, 0x41, 0x10, 0x40, 0x51, 0x09, 0x34, 0x82,
                0xf0, 0x15, 0x90, 0xb2, 0xea, 0x36, 0xd2, 0x3a,
            ]),
            // The published digest of "abc".
            (b"abc", [
                0x3a, 0x98, 0x5d, 0xa7, 0x4f, 0xe2, 0x25, 0xb2,
                0x04, 0x5c, 0x17, 0x2d, 0x6b, 0xd3, 0x90, 0xbd,
                0x85, 0x5f, 0x08, 0x6e, 0x3e, 0x9d, 0x52, 0x5b,
                0x46, 0xbf, 0xe2, 0x45, 0x11, 0x43, 0x15, 0x32,
            ]),
        ];
        for (msg, expected) in vectors {
            // `Hasher::hash` takes `self` by value, and `req!` renders its arguments a second time
            // when it fails, so the digest is taken once and compared as a binding.
            let hasher = HashScheme::new_sha3_256();
            let hash = res!(digest(hasher.hash(&[msg], [])));
            req!(hash, expected, "SHA3-256 of {:02x?}", msg);
        }
        Ok(())
    }

    /// The input slices must be absorbed in order, as one message.
    #[test]
    fn test_sha3_256_absorbs_input_slices_in_order() -> Outcome<()> {
        let hasher = HashScheme::new_sha3_256();
        let hash = res!(digest(hasher.hash(&[b"hello", b" ", b"world"], [])));
        req!(hash, HELLO_WORLD);
        Ok(())
    }

    /// The salt is absorbed after the input, giving `H(input ‖ salt)`.  Prepending the salt
    /// instead would produce a different digest, so this pins the convention against a published
    /// value rather than against ourselves.
    #[test]
    fn test_sha3_256_salt_follows_input() -> Outcome<()> {
        let hasher = HashScheme::new_sha3_256();
        // "hello" with the salt " world" must digest as SHA3-256("hello world").
        let hash = res!(digest(hasher.hash(&[b"hello"], *b" world")));
        req!(hash, HELLO_WORLD);
        Ok(())
    }

    #[test]
    fn test_hash_lengths() -> Outcome<()> {
        let sha3 = HashScheme::new_sha3_256();
        req!(*res!(sha3.hash_length().required("SHA3-256 hash length")), 32);
        let len = res!(digest(sha3.hash(&[b"this is a test"], []))).len();
        req!(len, 32);
        let seahash = HashScheme::new_seahash();
        req!(*res!(seahash.hash_length().required("Seahash hash length")), 8);
        Ok(())
    }
}
