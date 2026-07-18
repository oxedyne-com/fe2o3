//! Inter-bot communication primitives.
//!
//! [`msg`] defines the [`OzoneMsg`](msg::OzoneMsg) message enum every bot
//! exchanges, [`channels`] holds the routed channel set used to reach any bot,
//! and [`response`] provides the responder that correlates replies to a
//! request via its ticket.

pub mod channels;
pub mod msg;
pub mod response;
//pub mod server;
