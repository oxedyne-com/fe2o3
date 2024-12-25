use crate::{
    prelude::*,
    id::ParseId,
    impls_for_native_integer,
    string::{
        parse_hex_char,
        ToHexString,
    },
};

use std::{
    cmp::Ordering,
    fmt,
};

pub fn byte_slices_equal(a: &[u8], b: &[u8]) -> Outcome<()> {
    for (i, ai) in a.iter().enumerate() {
        if *ai != b[i] {
            return Err(err!(errmsg!("Mismatch detected"),
                Input, Mismatch));
        }
    }
    Ok(())
}

new_type!(B32, [u8; 32], Clone, Default);

impl std::marker::Copy for B32 {}

impl fmt::Debug for B32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex_string())
    }
}

impl fmt::Display for B32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl ParseId<32> for B32 {
    fn parse_id(s: &str) -> Outcome<Self> {
        let s = s.trim_start_matches("0x");
        if s.len() != 64 {
            return Err(err!(errmsg!(
                "The hexadecimal string '{}' has length {}, but it should be 64 for a B32.",
                s, s.len(),
            ), Invalid, Input, String, Size));
        }
        if !s.is_ascii() {
            return Err(err!(errmsg!(
                "The hexadecimal string '{}' contains at least one non-ASCII character.", s,
            ), Invalid, Input, String));
        }

        let mut result = [0u8; 32];
        let mut hex_chars = s.chars();

        let mut count: usize = 0;
        for byt in result.iter_mut() {
            count += 1;
            let hc = match hex_chars.next() {
                Some(c) => c,
                None => return Err(err!(errmsg!(
                    "Expecting a character at position {} in the hexadecimal string '{}', \
                    but it was not found.", count, s,
                ), Invalid, Input, String)),
            };
            let high_nibble = res!(parse_hex_char(hc));
            count += 1;
            let hc = match hex_chars.next() {
                Some(c) => c,
                None => return Err(err!(errmsg!(
                    "Expecting a character at position {} in the hexadecimal string '{}', \
                    but it was not found.", count, s,
                ), Invalid, Input, String)),
            };
            let low_nibble = res!(parse_hex_char(hc));
            *byt = high_nibble << 4 | low_nibble;
        }
        Ok(Self(result))
    }
}

impl Eq for B32 {}

impl PartialEq for B32 {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Ord for B32 {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.0).iter().zip((other.0).iter()).fold(Ordering::Equal, |acc, (a, b)| {
            if a < b {
                Ordering::Less
            } else if a > b {
                Ordering::Greater
            } else {
                acc
            }
        })
    }
}

impl PartialOrd for B32 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub trait IntoBytes {
    fn into_bytes(self, buf: Vec<u8>) -> Outcome<Vec<u8>>;
}

pub trait FromBytes {
    fn from_bytes(buf: &[u8]) -> Outcome<(Self, usize)> where Self: Sized;
    fn too_few(
        nbyts:      usize,
        minbyts:    usize,
        desc:       &str,
        file:       &'static str,
        line:       u32,
    )
        -> Error<ErrTag>
    {
        err!(fmt!("{}:{}: Only {} byte{}, require at least {} to decode the {}.",
            file, line, nbyts, if nbyts == 1 { "" } else { "s" }, minbyts, desc,
        ), Bytes, Input, Decode, Missing)
    }
}

pub trait ToBytes {
    fn to_bytes(&self, buf: Vec<u8>) -> Outcome<Vec<u8>>;
}

pub trait FromByteArray: Sized {
    fn from_byte_array<const L: usize>(buf: [u8; L]) -> Outcome<Self>;
}

pub trait ToByteArray<const L: usize> {
    fn to_byte_array(&self) -> [u8; L];
}

impls_for_native_integer!(u8, 1);
impls_for_native_integer!(u16, 2);
impls_for_native_integer!(u32, 4);
impls_for_native_integer!(u64, 8);
impls_for_native_integer!(u128, 16);
//impls_for_native_integer!(usize);
impls_for_native_integer!(i8, 1);
impls_for_native_integer!(i16, 2);
impls_for_native_integer!(i32, 4);
impls_for_native_integer!(i64, 8);
impls_for_native_integer!(i128, 16);
//impls_for_native_integer!(isize);

impl ToBytes for B32 {
    fn to_bytes(&self, mut buf: Vec<u8>) -> Outcome<Vec<u8>> {
        buf.extend_from_slice(&self.0);
        Ok(buf)
    }
}

impl FromBytes for B32 {
    fn from_bytes(buf: &[u8]) -> Outcome<(Self, usize)> {
        const BYTE_LEN: usize = 32;
        if buf.len() < BYTE_LEN {
            return Err(err!(errmsg!(
                "Not enough bytes to decode, require at least {} \
                for a {}, slice is of length {}.",
                BYTE_LEN, std::any::type_name::<Self>(), buf.len(),
            ), Bytes, Invalid, Input, Decode, Missing));
        }
        let n = Self(res!(
            <[u8; BYTE_LEN]>::try_from(&buf[0..BYTE_LEN]),
            Decode, Bytes, Integer,
        ));
        Ok((n, BYTE_LEN))
    }
}

impl FromByteArray for B32 {
    fn from_byte_array<const L: usize>(buf: [u8; L]) -> Outcome<Self> {
        const BYTE_LEN: usize = 32;
        if L < BYTE_LEN {
            return Err(err!(errmsg!(
                "Not enough bytes to decode, require at least {} \
                for a {}, array is of length {}.",
                BYTE_LEN, std::any::type_name::<Self>(), L,
            ), Bytes, Invalid, Input, Decode, Missing));
        }
        Ok(Self(res!(
            <[u8; BYTE_LEN]>::try_from(&buf[0..BYTE_LEN]),
            Decode, Bytes, Integer,
        )))
    }
}

impl ToByteArray<32> for B32 {
    fn to_byte_array(&self) -> [u8; 32] {
        **self
    }
}

/// This mutable version is useful when you want to append another vector to buf, rather than
/// copying.
pub trait ToBytesMut {
    fn to_bytes_mut(&mut self, buf: Vec<u8>) -> Outcome<Vec<u8>>;
}

#[allow(non_camel_case_types)] 
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Encoding {
    Unknown = 0,
    Binary  = 1,
    UTF8    = 2,
}

impl Default for Encoding {
    fn default() -> Self {
        Self::Unknown
    }
}

impl From<u8> for Encoding {
    fn from(b: u8) -> Self {
        match b {
            1 => Self::Binary,
            2 => Self::UTF8, 
            _ => Self::Unknown,
        }
    }
}
