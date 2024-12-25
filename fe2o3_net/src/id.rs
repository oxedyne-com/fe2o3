use oxedize_fe2o3_jdat::id::IdDat;

use std::{
    mem,
};

//// Protocol identifier types for defaults and testing.
pub type BidTyp = u64; // Bot Id
pub type MidTyp = u64; // Message Id
pub type SidTyp = u64; // State Id (e.g. for sessions)
pub type UidTyp = u128; // User Id

pub const BID_LEN: usize = mem::size_of::<BidTyp>();
pub const MID_LEN: usize = mem::size_of::<MidTyp>();
pub const SID_LEN: usize = mem::size_of::<SidTyp>();
pub const UID_LEN: usize = mem::size_of::<UidTyp>();

pub type Bid = IdDat<{BID_LEN}, BidTyp>;
pub type Mid = IdDat<{MID_LEN}, MidTyp>;
pub type Sid = IdDat<{SID_LEN}, SidTyp>;
pub type Uid = IdDat<{UID_LEN}, UidTyp>;
