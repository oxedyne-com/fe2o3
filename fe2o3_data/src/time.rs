use oxedize_fe2o3_core::{
    prelude::*,
    byte::{
        FromBytes,
        ToBytes,
    },
    mem::Extract,
};
use oxedize_fe2o3_jdat::{
    prelude::*,
    try_extract_tup2dat,
    tup2dat,
};

use std::{
    fmt::Debug,
    time::{
        Duration,
        SystemTime,
        UNIX_EPOCH,
    },
};

// Duration since the start of the unix epoch.
new_type!(Timestamp, Duration, Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd);

impl ToBytes for Timestamp {
    fn to_bytes(&self, mut buf: Vec<u8>) -> Outcome<Vec<u8>> {
        buf.extend_from_slice(&self.secs().to_be_bytes());
        buf.extend_from_slice(&self.nanos().to_be_bytes());
        Ok(buf)
    }
}

impl FromBytes for Timestamp {
    fn from_bytes(buf: &[u8]) -> Outcome<(Self, usize)> {
        let secs = u64::from_be_bytes(res!(<[u8; 8]>::try_from(&buf[..8]), Decode, Bytes));
        let nanos = u32::from_be_bytes(res!(<[u8; 4]>::try_from(&buf[8..12]), Decode, Bytes));
        Ok((
            Self::new(secs, nanos),
            Self::BYTE_LEN,
        ))
    }
}

// ToDat for Timestamp automatically implemented by Deref.

impl FromDat for Timestamp {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        Ok(Self(res!(Duration::from_dat(dat))))
    }
}

impl Timestamp {
 
    pub const BYTE_LEN: usize = 12;

    pub fn now() -> Outcome<Self> {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(n) => Ok(Self(n)),
            Err(_) => Err(err!(
                "The current SystemTime {:?} is before the Unix epoch {:?}, \
                there is a problem with the system clock!  Try resetting it.",
                SystemTime::now(), UNIX_EPOCH;
            System)),
        }
    }

    pub fn new(secs: u64, nanos: u32) -> Self {
        Self(Duration::new(secs, nanos))
    }

    pub fn secs(&self) -> u64 {
        self.0.as_secs()
    }

    pub fn nanos(&self) -> u32 {
        self.0.subsec_nanos()
    }

}

#[derive(Clone, Debug, Default)]
pub struct Timestamped<
    D: Clone
    + Debug
    + Default
    + ToDat
    + FromDat
> {
    pub data:   D,
    pub t:      Timestamp,
}

impl<
    D: Clone
    + Debug
    + Default
    + ToDat
    + FromDat
>
    Timestamped<D>
{
    pub fn new(data: D) -> Outcome<Self> {
        Ok(Self {
            data,
            t: res!(Timestamp::now()),
        })
    }
}

impl<
    D: Clone
    + Debug
    + Default
    + ToDat
    + FromDat
>
    ToDat for Timestamped<D>
{
    fn to_dat(&self) -> Outcome<Dat> {
        Ok(tup2dat![
            res!(self.data.to_dat()),
            res!(self.t.to_dat()),
        ])
    }
}

impl<
    D: Clone
    + Debug
    + Default
    + ToDat
    + FromDat
>
    FromDat for Timestamped<D>
{
    fn from_dat(dat: Dat) -> Outcome<Self> {
        let mut v = try_extract_tup2dat!(dat);
        let data = res!(D::from_dat(v[0].extract()));
        let t = res!(Timestamp::from_dat(v[1].extract()));
        Ok(Self {
            data,
            t,
        })
    }
}
