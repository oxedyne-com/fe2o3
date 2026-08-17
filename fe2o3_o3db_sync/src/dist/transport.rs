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
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use super::record::{
	Record,
	RecordId,
};
use super::hotstuff::types::{
	NewView,
	Proposal,
	Vote,
};

use oxedyne_fe2o3_core::prelude::*;
use crate::kademlia::id::NodeId;


/// Matches a [`MsgKind::GetRequest`] with its eventual
/// [`MsgKind::GetResponse`]. Opaque 64-bit tokens, picked monotonically per
/// process.
pub type RequestId = u64;


/// An envelope wraps a message body with its sender and intended recipient.
///
/// The engine consumes envelopes via [`DistOzone::handle_envelope`] and emits
/// them in the `outbound` field of its outcome types. Signing, encryption and
/// on-wire encoding are the transport adapter's responsibility -- the engine
/// treats envelopes as opaque authenticated structures.
///
/// [`DistOzone::handle_envelope`]: crate::dist::DistOzone::handle_envelope
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Envelope {
	pub from:	NodeId,
	pub to:		NodeId,
	pub body:	MsgKind,
}

impl Envelope {
	pub fn new(from: NodeId, to: NodeId, body: MsgKind) -> Self {
		Self { from, to, body }
	}
}


/// The distributed-Ozone message kinds.
///
/// This enum covers the replication-broadcast / read-routing cycle
/// (`ReplicatePut`, `GetRequest`, `GetResponse`), the IBLT anti-entropy
/// cycle (`AntiEntropyDigest`, `AntiEntropyReply`, `AntiEntropyPush`),
/// and the HotStuff cohort cycle for strong-consistency tables
/// (`CohortSubmit`, `CohortPropose`, `CohortVote`, `CohortNewView`).
/// Brickyard backup messages are deferred until that layer lands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MsgKind {
	// A write: "persist this record if you consider yourself a holder".
	// Emitted for every remote holder. Each recipient re-checks its own
	// placement decision, so a peer with a slightly different view of N may
	// decline; the record is then dropped and the next anti-entropy round
	// fills the gap.
	ReplicatePut {
		record:	Record,
	},
	// A read request: "do you have this record?". Emitted when the local peer
	// is not itself a holder of the record it wants to read. The recipient
	// answers with GetResponse.
	GetRequest {
		request_id:	RequestId,
		table:		String,
		id:			RecordId,
	},
	GetResponse {
		request_id:	RequestId,
		record:		Option<Record>,	// None if the recipient did not have it
	},

	// Anti-entropy digest: "here is my IBLT for this table; reconcile against
	// yours and reply with the symmetric difference". The sender builds the
	// sketch from its own Storage::digests enumeration. The recipient builds
	// its own sketch with matching parameters, subtracts, decodes, and replies
	// with AntiEntropyReply.
	AntiEntropyDigest {
		table:		String,
		sketch:		Vec<u8>,	// Iblt::to_bytes output, opaque to transport
	},
	// Anti-entropy reply: "these records are what I have and you lack; please
	// send me records with these identifiers". On decode failure -- sketch
	// overload -- the recipient bulk-replies with every record it holds for
	// the table and an empty requested-id list, and the originator absorbs
	// what it lacks. Simple, at the cost of bandwidth on a fresh join.
	AntiEntropyReply {
		table:			String,
		records:		Vec<Record>,	// held here, and lacked by the originator
		requested_ids:	Vec<RecordId>,	// wanted from the originator
		bulk:			bool,			// set when the sketch could not decode
	},
	// Anti-entropy push: "here are the records you requested", sent by the
	// originator in answer to an AntiEntropyReply's requested_ids list.
	AntiEntropyPush {
		table:		String,
		records:	Vec<Record>,
	},

	// Forwarded write: "you are the HotStuff leader for this record; drive
	// consensus on my behalf". Sent by a peer whose own DistOzone::put named a
	// cohort-backed table for which it is not the initial round's leader. The
	// recipient leader creates a CohortInstance if none exists and opens a
	// CohortPropose round.
	CohortSubmit {
		record:	Record,
	},
	// HotStuff leader-to-cohort proposal. The (table, id) pair selects the
	// per-record HotStuff instance; the Proposal itself carries the view,
	// phase, block hash, optional block payload and optional justify QC.
	CohortPropose {
		table:		String,
		id:			RecordId,
		proposal:	Proposal,
	},
	// HotStuff replica-to-leader vote.
	CohortVote {
		table:	String,
		id:		RecordId,
		vote:	Vote,
	},
	// HotStuff view change, sent by a replica to the incoming leader when its
	// local timer fires without seeing progress in the current view.
	CohortNewView {
		table:		String,
		id:			RecordId,
		new_view:	NewView,
	},
}

impl MsgKind {
	pub fn label(&self) -> &'static str {
		match self {
			Self::ReplicatePut { .. }		=> "ReplicatePut",
			Self::GetRequest { .. }			=> "GetRequest",
			Self::GetResponse { .. }		=> "GetResponse",
			Self::AntiEntropyDigest { .. }	=> "AntiEntropyDigest",
			Self::AntiEntropyReply { .. }	=> "AntiEntropyReply",
			Self::AntiEntropyPush { .. }	=> "AntiEntropyPush",
			Self::CohortSubmit { .. }		=> "CohortSubmit",
			Self::CohortPropose { .. }		=> "CohortPropose",
			Self::CohortVote { .. }			=> "CohortVote",
			Self::CohortNewView { .. }		=> "CohortNewView",
		}
	}
}
