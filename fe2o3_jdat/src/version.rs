use crate::{
    prelude::*,
    try_extract_tup3dat,
    tup3dat,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    byte::{
        FromBytes,
        ToBytes,
    },
    mem::Extract,
};

use std::{
    fmt,
    str,
};


#[derive(Clone, Copy, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct SemVer {
    pub major: u8,
    pub minor: u8,
    pub patch: u8,
}

impl fmt::Debug for SemVer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl fmt::Display for SemVer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl str::FromStr for SemVer {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return Err(err!(
                "SemVer expects 3 numbers, found {}.", parts.len();
            Input, Invalid));
        }

        let major = res!(parts[0].parse::<u8>());
        let minor = res!(parts[1].parse::<u8>());
        let patch = res!(parts[2].parse::<u8>());

        Ok(Self { major, minor, patch })
    }
}

impl ToBytes for SemVer {
    fn to_bytes(&self, mut buf: Vec<u8>) -> Outcome<Vec<u8>> {
        buf.push(self.major);
        buf.push(self.minor);
        buf.push(self.patch);
        Ok(buf)
    }
}

impl FromBytes for SemVer {
    fn from_bytes(buf: &[u8]) -> Outcome<(Self, usize)> {
        if buf.len() < Self::BYTE_LEN {
            return Err(err!(
                "Not enough bytes to decode, require at least {}, found only {}.",
                Self::BYTE_LEN, buf.len();
            Bytes, Input, Decode, Missing));
        }
        Ok((Self {
            major: buf[0],
            minor: buf[1],
            patch: buf[2],
        },
        3))
    }
}

impl ToDat for SemVer {
    fn to_dat(&self) -> Outcome<Dat> {
        Ok(tup3dat![
            dat!(self.major),
            dat!(self.minor),
            dat!(self.patch),
        ])
    }
}

impl FromDat for SemVer {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        let mut result = Self::default();
        let mut v = try_extract_tup3dat!(dat);
        result.major  = try_extract_dat!(v[0].extract(), U8);
        result.minor  = try_extract_dat!(v[1].extract(), U8);
        result.patch  = try_extract_dat!(v[2].extract(), U8);
        Ok(result)
    }
}

impl SemVer {

    pub const BYTE_LEN: usize = 3;

    pub const fn new(major: u8, minor: u8, patch: u8) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

}
