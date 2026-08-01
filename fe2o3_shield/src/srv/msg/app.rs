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


/// Which half of an application exchange a message is.
///
/// The two travel under different message types rather than under one type
/// with a flag, so that a peer can tell a question from an answer before it
/// has parsed anything: a reply arriving where no question was asked is
/// dropped on the type alone.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AppMsgKind {
    /// A payload sent to a peer, which may be answered.
    #[default]
    Request,
    /// The answer to a request, carrying the request's message identifier.
    Reply,
}

impl AppMsgKind {
    /// The wire discriminant this kind travels under.
    pub fn typ(&self) -> MsgType {
        match self {
            Self::Request	=> constant::MSG_TYPE_APP_REQUEST,
            Self::Reply		=> constant::MSG_TYPE_APP_REPLY,
        }
    }

    /// The syntax command name this kind is carried in.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Request	=> constant::MSG_CMD_APP_REQUEST,
            Self::Reply		=> constant::MSG_CMD_APP_REPLY,
        }
    }

    /// Read a wire discriminant, or `None` if it names something that is not an
    /// application payload.
    pub fn from_msg_type(typ: MsgType) -> Option<Self> {
        match typ {
            constant::MSG_TYPE_APP_REQUEST	=> Some(Self::Request),
            constant::MSG_TYPE_APP_REPLY	=> Some(Self::Reply),
            _					=> None,
        }
    }

    /// Read a syntax command name, or `None` if it names something else.
    pub fn from_cmd_name(name: &str) -> Option<Self> {
        match name {
            constant::MSG_CMD_APP_REQUEST	=> Some(Self::Request),
            constant::MSG_CMD_APP_REPLY		=> Some(Self::Reply),
            _					=> None,
        }
    }
}

/// An opaque application payload, in either direction.
#[derive(Clone, Debug, Default)]
pub struct AppMsg<
    const ML: usize,
    const SL: usize,
    const UL: usize,
    ID: IdTypes<ML, SL, UL>,
> {
    /// Message format: syntax and encoding.
    pub fmt:        MsgFmt,
    /// Proof-of-work parameters for the message.
    pub pow:        MsgPow,
    /// Session and user identifiers.
    pub mid:        MsgIds<SL, UL, ID::S, ID::U>,
    // Command-specific
    /// Whether this is a request or the reply to one.
    pub kind:       AppMsgKind,
    /// The caller's bytes, which the protocol does not read.
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

    /// There is no session yet, so a peer that has never been heard from has no
    /// record of the key a packet was signed with. The key travels with the
    /// packet; what that proves is that the packet was signed by whoever holds
    /// it, which is what the payload's own signature is for, not this one.
    fn inc_sigpk(&self) -> bool { true }

    /// The last chunk is not padded, so what is reassembled is what was sent
    /// and nothing after it.
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

/// What a peer has to say about an application payload that arrived.
///
/// The two are not an `Option<Vec<u8>>` because the difference matters to the
/// sender: news wants no answer and is not left waiting for one, whereas a
/// question that goes unanswered is a question whose asker sits out the
/// timeout. A handler says which it is rather than leaving the wire to guess.
#[derive(Clone, Debug)]
pub enum Answer {
    /// Nothing goes back. The sender was telling, not asking.
    Nothing,
    /// These bytes go back to the sender, under the message identifier that
    /// asked for them.
    Reply(Vec<u8>),
}
