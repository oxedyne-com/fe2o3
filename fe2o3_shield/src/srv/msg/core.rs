use crate::srv::{
    packet::{
        PacketChunkState,
        PacketCount,
        PacketMeta,
        PacketValidator,
    },
};

use oxedize_fe2o3_core::{
    prelude::*,
    byte::{
        ToBytes,
        ToByteArray,
    },
    map::MapMut,
    rand::RanDef,
};
use oxedize_fe2o3_iop_crypto::sign::Signer;
use oxedize_fe2o3_jdat::{
    chunk::{
        Chunker,
        ChunkConfig,
    },
    id::{
        IdDat,
        NumIdDat,
    },
    version::SemVer,
};
use oxedize_fe2o3_hash::{
    map::ShardMap,
    pow::{
        PowCreateParams,
        Pristine,
    },
};
use oxedize_fe2o3_iop_hash::api::{
    Hasher,
    HashForm,
};

use std::{
    clone::Clone,
    collections::BTreeMap,
    fmt::Debug,
    net::{
        SocketAddr,
        UdpSocket,
    },
    sync::RwLock,
    time::{
        Duration,
        Instant,
    },
};


pub trait IdTypes<
    const ML: usize,
    const SL: usize,
    const UL: usize,
> {
    type M: NumIdDat<ML>;
    type S: NumIdDat<SL>;
    type U: NumIdDat<UL>;
}

pub type MsgType = u16;
//pub const MSG_TYPE_BYTE_LEN: usize = 2;
//pub const MSG_TYPE_USER_START: MsgType = 1_024;
//pub type MsgId = u64;
//pub const MSG_ID_BYTE_LEN: usize = 8;

/// The MsgFmt captures the syntax protocol against which incoming and outgoing messages are
/// validated, and the encoding for any outgoing messages.
#[derive(Clone, Debug, Default)]
pub struct MsgFmt {
    pub syntax:     SyntaxRef,
    pub encoding:   Encoding,
}

/// Capture the required (when receiving) and expected (when sending) Proof of Work parameters.
#[derive(Clone, Debug, Default)]
pub struct MsgPow {
    pub zbits:  ZeroBits,
}

impl MsgPow {

    pub fn from_msg(msg: &mut SyntaxMsg) -> Outcome<Self> {
        let zbits = match msg.get_arg_vals_mut("-zb") {
            Some(v) => try_extract_dat_as!(v[0].extract(), ZeroBits, U8, U16, U32),
            None => return Err(err!(
                "No proof of work zero bits specified in message arguments (-zb).";
                Input, Missing)),
        };
        Ok(Self {
            zbits,
        })
    }
}

/// Capture the user id and possibly the session id.
#[derive(Clone, Debug, Default)]
pub struct MsgIds<
    const SIDL: usize,
    const UIDL: usize,
    SID:    NumIdDat<SIDL>,
    UID:    NumIdDat<UIDL>,
> {
    pub sid_opt:    Option<IdDat<SIDL, SID>>,
    pub uid:        IdDat<UIDL, UID>,
}

impl<
    const SIDL: usize,
    const UIDL: usize,
    SID:    NumIdDat<SIDL>,
    UID:    NumIdDat<UIDL>,
>
    MsgIds<SIDL, UIDL, SID, UID>
{
    pub fn from_msg(uid: IdDat<UIDL, UID>, msg: &mut SyntaxMsg) -> Outcome<Self> {
        //let uid = match msg.get_arg_vals_mut("-u") {
        //    Some(v) => try_extract_dat_as!(v[0].extract(), IdDat, U128),
        //    None => return Err(err!(
        //        "No user id value in message arguments (-u).",
        //    ), Input, Missing)),
        //};
        let sid_opt = match msg.get_arg_vals_mut("-s") {
            Some(v) => Some(res!(IdDat::<SIDL, SID>::from_dat(v[0].extract()))),
            None => None, // not required
        };
        Ok(Self {
            uid,
            sid_opt,
        })
    }
}

#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum HandshakeType {
    Unknown = 0,
    Req1    = 1,
    Resp1   = 2,
    Req2    = 3,
    Resp2   = 4,
    Req3    = 5,
    Resp3   = 6,
}

impl From<MsgType> for HandshakeType {
    fn from(u: MsgType) -> Self {
        match u {
            1 =>    Self::Req1,
            2 =>    Self::Resp1,
            3 =>    Self::Req2,
            4 =>    Self::Resp2,
            5 =>    Self::Req3,
            6 =>    Self::Resp3,
            _ =>    Self::Unknown,
        }
    }
}

impl HandshakeType {
    pub fn is_hreq2(&self) -> bool {
        match self {
            Self::Req2 => true,
            _ => false,
        }
    }
}

pub trait IdentifiedMessage {
    fn typ(&self) -> MsgType;
    fn name(&self) -> &'static str;
}
