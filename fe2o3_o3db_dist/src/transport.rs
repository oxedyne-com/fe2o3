//! Wire-format envelopes and message kinds for distributed Ozone.
//!
//! Transport itself lives *outside* this crate: the engine emits envelopes
//! as a [`Commands`](crate::dist::Commands) return value, the caller
//! dispatches them. The production adapter is Shield (UDP, signed-hash
//! datagrams, AddressGuard rate-limiting); the test adapter is whatever the
//! caller builds from a channel or a mock.
//!
//! Keeping transport out of the engine means distributed Ozone is a pure
//! state machine: every decision it makes is a function of its inputs with
//! no hidden I/O, which is the property that made the primitive crates
//! (Kademlia, OAM, IBLT, HotStuff) easy to test and reason about.

use crate::record::{
	Record,
	RecordId,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_kademlia::id::NodeId;


/// A correlation identifier for request/response pairs.
///
/// Used to match [`MsgKind::GetRequest`] with its eventual
/// [`MsgKind::GetResponse`]. Values are 64-bit opaque tokens; the engine
/// picks them monotonically per process.
pub type RequestId = u64;


/// An envelope wraps a message body with its sender and intended recipient.
///
/// The engine consumes envelopes via [`DistOzone::handle_envelope`] and
/// emits them via [`Commands::outbound`]. Signing, encryption and on-wire
/// encoding are the transport adapter's responsibility -- the engine treats
/// envelopes as opaque authenticated structures.
///
/// [`DistOzone::handle_envelope`]: crate::dist::DistOzone::handle_envelope
/// [`Commands::outbound`]: crate::dist::Commands::outbound
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Envelope {
	/// The sending peer's 256-bit identifier.
	pub from:	NodeId,
	/// The intended recipient's 256-bit identifier.
	pub to:		NodeId,
	/// The message body.
	pub body:	MsgKind,
}

impl Envelope {
	/// Constructs a new envelope.
	pub fn new(from: NodeId, to: NodeId, body: MsgKind) -> Self {
		Self { from, to, body }
	}
}


/// The distributed-Ozone message kinds.
///
/// This enum intentionally covers only the messages exchanged by the pieces
/// of distributed mode delivered so far. Anti-entropy digest exchanges,
/// cohort-level HotStuff messages, and brickyard backup messages are deferred
/// until those layers land and will be added as further variants.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MsgKind {
	/// A write: "persist this record if you consider yourself a holder".
	///
	/// Emitted by the originating peer for every remote holder and handled by
	/// each recipient by re-checking its own placement decision -- a peer
	/// with a slightly different view of `N` may decline a put that the
	/// sender chose it for, in which case the record is dropped and the
	/// next anti-entropy round fills the gap.
	ReplicatePut {
		/// The record to persist.
		record:	Record,
	},
	/// A read request: "do you have this record?".
	///
	/// Emitted when the local peer is not itself a holder of the record it
	/// wants to read. The recipient answers with [`MsgKind::GetResponse`].
	GetRequest {
		/// Correlation identifier for the eventual response.
		request_id:	RequestId,
		/// The table the record belongs to.
		table:		String,
		/// The record's 256-bit identifier.
		id:			RecordId,
	},
	/// A read response.
	GetResponse {
		/// Correlation identifier matching the original request.
		request_id:	RequestId,
		/// The record, or `None` if the recipient did not have it.
		record:		Option<Record>,
	},
}

impl MsgKind {
	/// A short human-readable label for logging.
	pub fn label(&self) -> &'static str {
		match self {
			Self::ReplicatePut { .. }	=> "ReplicatePut",
			Self::GetRequest { .. }		=> "GetRequest",
			Self::GetResponse { .. }	=> "GetResponse",
		}
	}
}
