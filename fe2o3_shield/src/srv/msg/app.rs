//! The application payload command: opaque bytes a library user carries over
//! the SHIELD wire.
//!
//! Everything else the protocol sends is the protocol's own business -- the
//! handshake stages, and eventually the session they establish. This is the
//! one message type that carries something SHIELD does not read: a caller's
//! bytes, chunked, proof-of-worked, signed and reassembled exactly as any
//! other message is, and handed back whole at the other end.
//!
//! An exchange is a request and at most one reply, correlated by the message
//! identifier in the packet header rather than by a second connection. That is
//! not a stylistic choice: a peer behind a household router can send a
//! datagram and hear the answer on the socket it sent from, and cannot be sent
//! one out of the blue. A reply that needed its own path is a reply half the
//! network never receives.
//!
//! What this does *not* do is establish a session or encrypt anything. The
//! packet signature says the packet was signed by the key travelling with it,
//! and nothing more; whatever identity the payload has is the caller's to put
//! inside it and check at the other end.

use crate::srv::{
    constant,
    msg::{
        core::{
            IdentifiedMessage,
            IdTypes,
            MsgFmt,
            MsgIds,
            MsgPow,
            MsgType,
        },
        encode::ShieldCommand,
    },
};

use oxedyne_fe2o3_core::{
    prelude::*,
    byte::IntoBytes,
    mem::Extract,
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_syntax::msg::{
    Msg,
    MsgCmd,
};


#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AppMsgKind {
    #[default]
    Request,
    Reply,
}

impl AppMsgKind {
    pub fn typ(&self) -> MsgType {
        match self {
            Self::Request	=> constant::MSG_TYPE_APP_REQUEST,
            Self::Reply		=> constant::MSG_TYPE_APP_REPLY,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Request	=> constant::MSG_CMD_APP_REQUEST,
            Self::Reply		=> constant::MSG_CMD_APP_REPLY,
        }
    }

    pub fn from_msg_type(typ: MsgType) -> Option<Self> {
        match typ {
            constant::MSG_TYPE_APP_REQUEST	=> Some(Self::Request),
            constant::MSG_TYPE_APP_REPLY	=> Some(Self::Reply),
            _					=> None,
        }
    }

    pub fn from_cmd_name(name: &str) -> Option<Self> {
        match name {
            constant::MSG_CMD_APP_REQUEST	=> Some(Self::Request),
            constant::MSG_CMD_APP_REPLY		=> Some(Self::Reply),
            _					=> None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct AppMsg<
    const ML: usize,
    const SL: usize,
    const UL: usize,
    ID: IdTypes<ML, SL, UL>,
> {
    pub fmt:        MsgFmt,
    pub pow:        MsgPow,
    pub mid:        MsgIds<SL, UL, ID::S, ID::U>,
    // Command-specific
    pub kind:       AppMsgKind,
    pub payload:    Vec<u8>,
}

impl<
    const ML: usize,
    const SL: usize,
    const UL: usize,
    ID: IdTypes<ML, SL, UL>,
>
    IntoBytes for AppMsg<ML, SL, UL, ID>
{
    fn into_bytes(self, buf: Vec<u8>) -> Outcome<Vec<u8>> {
        res!(self.construct()).into_bytes(buf)
    }
}

impl<
    const ML: usize,
    const SL: usize,
    const UL: usize,
    ID: IdTypes<ML, SL, UL>,
>
    IdentifiedMessage for AppMsg<ML, SL, UL, ID>
{
    fn typ(&self) -> MsgType { self.kind.typ() }
    fn name(&self) -> &'static str { self.kind.name() }
}

impl<
    const ML: usize,
    const SL: usize,
    const UL: usize,
    ID: IdTypes<ML, SL, UL>,
>
    ShieldCommand<ML, SL, UL, ID> for AppMsg<ML, SL, UL, ID>
{
    fn fmt(&self) -> &MsgFmt { &self.fmt }
    fn pow(&self) -> &MsgPow { &self.pow }
    fn mid(&self) -> &MsgIds<SL, UL, ID::S, ID::U> { &self.mid }

    fn inc_sigpk(&self) -> bool { true }

    fn pad_last(&self) -> bool { false }

    fn construct(self) -> Outcome<Msg> {
        let name = self.name();
        let mut msg = Msg::new(self.syntax().clone()); // cloning ref
        msg.set_encoding(*self.encoding());
        if let Some(sid) = self.sid_opt() {
            msg = res!(msg.add_arg_val("-s", Some(res!(sid.to_dat()))));
        }
        msg = res!(msg.add_arg_val("-zb", Some(dat!(self.pow_zbits()))));
        let mut mcmd = res!(msg.new_cmd(name));
        mcmd = res!(mcmd.add_arg_val("-p", Some(Dat::BU64(self.payload))));
        msg = res!(msg.add_cmd(mcmd));
        res!(msg.validate());
        Ok(msg)
    }

    fn deconstruct(&mut self, mcmd: &mut MsgCmd) -> Outcome<()> {
        self.payload = match mcmd.get_arg_vals_mut("-p") {
            Some(vals) => try_extract_dat!(vals[0].extract(), BU64),
            None => return Err(err!(
                "An application message carries no payload argument (-p).";
                Invalid, Input, Missing)),
        };
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum Answer {
    Nothing,
    Reply(Vec<u8>),
}
