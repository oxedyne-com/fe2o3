use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    id::IdDat,
    kind::Kind,
};

use std::{
    fmt,
    mem,
};


//// Protocol identifier types for defaults and testing.
pub type BidTyp = u64; // Bot Id
pub type SidTyp = u64; // State Id (e.g. for sessions)
pub type UidTyp = u128; // User Id, a default.

pub const UID_KIND: Kind = Kind::U128;

pub const BID_LEN: usize = mem::size_of::<BidTyp>();
pub const SID_LEN: usize = mem::size_of::<SidTyp>();
pub const UID_LEN: usize = mem::size_of::<UidTyp>();

pub type Bid = IdDat<{BID_LEN}, BidTyp>;
pub type Sid = IdDat<{SID_LEN}, SidTyp>;
pub type Uid = IdDat<{UID_LEN}, UidTyp>;

// Message command identifier.
new_type!(McidTyp, u64, Clone, Copy);

pub const MCID_KIND: Kind = Kind::U64;

impl fmt::Display for McidTyp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self)
    }
}

impl fmt::Debug for McidTyp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self)
    }
}

impl fmt::LowerHex for McidTyp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::LowerHex::fmt(&self.0, f)
    }
}
