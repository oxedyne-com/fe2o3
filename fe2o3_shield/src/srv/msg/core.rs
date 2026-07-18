use oxedyne_fe2o3_core::{
    prelude::*,
    byte::Encoding,
    mem::Extract,
};
use oxedyne_fe2o3_jdat::{
    prelude::*,
    id::NumIdDat,
};
use oxedyne_fe2o3_hash::pow::ZeroBits;
use oxedyne_fe2o3_net::id;
use oxedyne_fe2o3_syntax::{
    SyntaxRef,
    msg::Msg,
};

use std::{
    clone::Clone,
    fmt,
};


/// Bundles the message, session and user identifier types used throughout the
/// protocol, parameterised by their respective byte lengths.
pub trait IdTypes<
    const ML: usize,
    const SL: usize,
    const UL: usize,
>:
    Clone
    + Default
    + fmt::Debug
{
    /// Message identifier type.
    type M: NumIdDat<ML>;
    /// Session identifier type.
    type S: NumIdDat<SL>;
    /// User identifier type.
    type U: NumIdDat<UL>;
}

/// Default [`IdTypes`] binding using the standard `fe2o3_net` identifier types.
#[derive(Clone, Debug, Default)]
pub struct DefaultIdTypes<
    const ML: usize,
    const SL: usize,
    const UL: usize,
>;

impl<
    const ML: usize,
    const SL: usize,
    const UL: usize,
>
    IdTypes<ML, SL, UL> for DefaultIdTypes<ML, SL, UL>
    where
    id::Mid: NumIdDat<ML>,
    id::Sid: NumIdDat<SL>,
    id::Uid: NumIdDat<UL>,
{
    type M = id::Mid;
    type S = id::Sid;
    type U = id::Uid;
}

/// Numeric discriminant identifying a message's kind on the wire.
pub type MsgType = u16;
//pub const MSG_TYPE_BYTE_LEN: usize = 2;
//pub const MSG_TYPE_USER_START: MsgType = 1_024;
//pub type MsgId = u64;
//pub const MSG_ID_BYTE_LEN: usize = 8;

/// The MsgFmt captures the syntax protocol against which incoming and outgoing messages are
/// validated, and the encoding for any outgoing messages.
#[derive(Clone, Debug, Default)]
pub struct MsgFmt {
    /// Syntax protocol messages are validated against.
    pub syntax:     SyntaxRef,
    /// Byte encoding applied to outgoing messages.
    pub encoding:   Encoding,
}

/// Capture the required (when receiving) and expected (when sending) Proof of Work parameters.
#[derive(Clone, Debug, Default)]
pub struct MsgPow {
    /// Number of leading zero bits required of the proof of work.
    pub zbits:  ZeroBits,
}

impl MsgPow {
    /// Extracts the proof-of-work zero-bits parameter from a message's `-zb`
    /// argument, erroring if it is absent.
    pub fn from_msg(msg: &mut Msg) -> Outcome<Self> {
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
    /// Session identifier, if the message belongs to a session.
    pub sid_opt:    Option<SID>,
    /// User identifier of the message's originator.
    pub uid:        UID,
}

impl<
    const SIDL: usize,
    const UIDL: usize,
    SID:    NumIdDat<SIDL>,
    UID:    NumIdDat<UIDL>,
>
    MsgIds<SIDL, UIDL, SID, UID>
{
    /// Builds the identifiers from a message, taking the user identifier as
    /// given and reading the optional session identifier from the `-s`
    /// argument.
    pub fn from_msg(uid: UID, msg: &mut Msg) -> Outcome<Self> {
        //let uid = match msg.get_arg_vals_mut("-u") {
        //    Some(v) => try_extract_dat_as!(v[0].extract(), IdDat, U128),
        //    None => return Err(err!(
        //        "No user id value in message arguments (-u).",
        //    ), Input, Missing)),
        //};
        let sid_opt = match msg.get_arg_vals_mut("-s") {
            Some(v) => Some(res!(SID::from_dat(v[0].extract()))),
            None => None, // not required
        };
        Ok(Self {
            uid,
            sid_opt,
        })
    }
}

/// Implemented by message types that carry a wire discriminant and a name.
pub trait IdentifiedMessage {
    /// Returns the message's numeric wire discriminant.
    fn typ(&self) -> MsgType;
    /// Returns the message's human-readable name.
    fn name(&self) -> &'static str;
}
