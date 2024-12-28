use oxedize_fe2o3_core::{
    prelude::*,
    alt::{
        Alt,
        DefAlt,
        Override,
        Gnomon,
    },
};
use oxedize_fe2o3_iop_hash::api::{
    Hash,
    HashForm,
    Hasher,
};
use oxedize_fe2o3_namex::{
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

    //#[test]
    //fn test_hash_scheme_sha3() -> Outcome<()> {
    //    let mut sha3 = res!(sha3::<32>());
    //    let mut output = [0u8; 32];
    //    let expected = b"\
    //        \x64\x4b\xcc\x7e\x56\x43\x73\x04\x09\x99\xaa\xc8\x9e\x76\x22\xf3\
    //        \xca\x71\xfb\xa1\xd9\x72\xfd\x94\xa3\x1c\x3b\xfb\xf2\x4e\x39\x38\
    //    ";
    //
    //    sha3.update(b"hello");
    //    sha3.update(b" ");
    //    sha3.update(b"world");
    //    sha3.finalize(&mut output);
    //
    //    assert_eq!(expected, &output);
    //    Ok(())
    //}

    #[test]
    fn test_hash_01() -> Outcome<()> {
        //let hasher = res!(Hasher::from_str("seahash"));
        let hasher = HashScheme::new_sha3_256();
        let input = b"this is a test";
        msg!("len = {}", res!(hasher.len()));
        let hash = hasher.hash(&input[..], []);
        msg!("hash = {:02x?}", hash);
        Ok(())
    }
}
