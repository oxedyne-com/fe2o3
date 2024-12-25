use oxedize_fe2o3_core::{
    prelude::*,
    alt::Gnomon,
};
use oxedize_fe2o3_namex::id::InNamex;

use std::fmt;


/// Most hash implementations seem to be multi-step. Hashing in one go allows us to include the
/// identity transformation as a no-operation option.  They are also self-consuming, so we have to
/// respect that.
pub trait Hasher:
    Clone
    + std::fmt::Debug
    + InNamex
    + Send
    + Sync
{
    fn hash<const S: usize>(self, input: &[&[u8]], salt: [u8; S]) -> Hash<S>;
    /// The a priori length.  The identity hash length is the input length, but we don't know
    /// that before seeing the input.
    fn hash_length(&self) -> Gnomon<usize>;
    fn is_identity(&self) -> bool;
}

/// The empty hasher should just return the input.
impl Hasher for () {
    fn hash<const S: usize>(self, input: &[&[u8]], _salt: [u8; S]) -> Hash<S> {
        let len = input.iter().map(|slice| slice.len()).sum();
        let mut result = Vec::with_capacity(len);

        for slice in input {
            result.extend_from_slice(slice);
        }

        Hash::new(HashForm::Identity(result), [0u8; S])
    }
    fn hash_length(&self) -> Gnomon<usize> { Gnomon::Unknown }
    fn is_identity(&self) -> bool { true }
}

#[derive(Clone)]
pub struct Hash<
    const S: usize,
> {
    form: HashForm,
    salt: [u8; S],
}

impl<
    const S: usize,
>
    fmt::Debug for Hash<S>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash{{form: {:?}, salt: {:02x?}}}", self.form, self.salt)
    }
}

impl<
    const S: usize,
>
    Hash<S>
{
    pub fn new(form: HashForm, salt: [u8; S]) -> Self {
        Self {
            form,
            salt,
        }
    }

    pub fn as_hashform(self) -> HashForm { self.form }
    
    pub fn as_vec(self) -> Vec<u8> {
        let salt_clone = self.salt.clone();
        let mut result = self.form.as_vec();
        result.extend_from_slice(&salt_clone);
        result
    }
}

/// A way to represent a hash result using more efficient primitives, when possible.
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub enum HashForm {
    Identity(Vec<u8>), // Output = Input
    U32(u32),
    U64(u64),
    U128(u128),
    Bytes32([u8; 32]),
    Bytes(Vec<u8>),
}

impl fmt::Debug for HashForm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Identity(v)   |
            Self::Bytes(v)      => write!(f, "{:02x?}", v),
            Self::Bytes32(a32)  => write!(f, "{:02x?}", a32),
            Self::U128(nu128)   => write!(f, "0x{:032x?}", nu128),
            Self::U64(nu64)     => write!(f, "0x{:016x?}", nu64),
            Self::U32(nu32)     => write!(f, "0x{:08x?}", nu32),
        }
    }
}

impl HashForm {
    /// A priori length of hash when encoded to bytes.
    pub fn len(&self) -> Gnomon<usize> {
        match self {
            Self::Identity(_)   => Gnomon::Unknown,
            Self::Bytes(v)      => Gnomon::Known(v.len()),
            Self::Bytes32(_)    => Gnomon::Known(32),
            Self::U128(_)       => Gnomon::Known(16),
            Self::U64(_)        => Gnomon::Known(8),
            Self::U32(_)        => Gnomon::Known(4),
        }
    }

    /// Consuming conversion to byte vector.
    pub fn as_vec(self) -> Vec<u8> {
        match self {
            Self::Identity(v)   |
            Self::Bytes(v)      => v,
            Self::Bytes32(a32)  => a32.to_vec(),
            Self::U128(n128)    => n128.to_be_bytes().to_vec(),
            Self::U64(n64)      => n64.to_be_bytes().to_vec(),
            Self::U32(n32)      => n32.to_be_bytes().to_vec(),
        }
    }

    /// Truncates the byte encoded form of itself to a `u32`, except when there are not enough
    /// bytes.
    pub fn to_u32(&self) -> Outcome<u32> {
        match self {
            Self::Identity(v) | Self::Bytes(v) => {
                if v.len() < 4 {
                    return Err(err!(errmsg!(
                        "With only {} bytes, the HashForm has too few to create \
                        a u32 (4).", v.len(),
                    ), TooSmall, Conversion));
                }
                Ok(u32::from_be_bytes(res!(<[u8; 4]>::try_from(&v[..4]), Decode, Bytes)))
            },
            Self::Bytes32(a32) => Ok(u32::from_be_bytes(res!(<[u8; 4]>::try_from(&a32[..4]), Decode, Bytes))),
            Self::U128(nu128) => Ok(*nu128 as u32),
            Self::U64(nu64) => Ok(*nu64 as u32),
            Self::U32(nu32) => Ok(*nu32),
        }
    }
}
