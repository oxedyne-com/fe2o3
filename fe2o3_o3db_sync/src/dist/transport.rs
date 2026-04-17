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
/// This enum covers the replication-broadcast / read-routing cycle
/// (`ReplicatePut`, `GetRequest`, `GetResponse`), the IBLT anti-entropy
/// cycle (`AntiEntropyDigest`, `AntiEntropyReply`, `AntiEntropyPush`),
/// and the HotStuff cohort cycle for strong-consistency tables
/// (`CohortSubmit`, `CohortPropose`, `CohortVote`, `CohortNewView`).
/// Brickyard backup messages are deferred until that layer lands.
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

	/// Anti-entropy digest: "here is my IBLT for this table; reconcile
	/// against yours and reply with the symmetric difference".
	///
	/// The sender builds the sketch from its own [`Storage::digests`][s]
	/// enumeration. The recipient builds its own sketch with matching
	/// parameters, subtracts, decodes, and replies with
	/// [`MsgKind::AntiEntropyReply`].
	///
	/// [s]: crate::storage::Storage::digests
	AntiEntropyDigest {
		/// The table being reconciled.
		table:		String,
		/// The serialised IBLT sketch (the output of
		/// [`Iblt::to_bytes`][tb]). Opaque to the transport layer.
		///
		/// [tb]: oxedyne_fe2o3_iblt::iblt::Iblt::to_bytes
		sketch:		Vec<u8>,
	},
	/// Anti-entropy reply: "these records are what I have and you lack;
	/// please send me records with these identifiers".
	///
	/// On decode failure (sketch overload), the recipient bulk-replies with
	/// every record it holds for the table and an empty requested-id list,
	/// and the originator absorbs the records it lacks. This simplifies
	/// the first-iteration flow at the cost of bandwidth on fresh joins.
	AntiEntropyReply {
		/// The table being reconciled.
		table:			String,
		/// Records the recipient holds that the originator lacked, per the
		/// decoded symmetric difference.
		records:		Vec<Record>,
		/// Record identifiers the recipient wants from the originator.
		requested_ids:	Vec<RecordId>,
		/// `true` if the recipient fell back to a bulk reply because the
		/// sketch could not decode. The originator may choose to skip
		/// sending a follow-up push when it sees a bulk reply.
		bulk:			bool,
	},
	/// Anti-entropy push: "here are the records you requested".
	///
	/// Sent by the originator in response to an
	/// [`AntiEntropyReply`][ar]'s `requested_ids` list.
	///
	/// [ar]: MsgKind::AntiEntropyReply
	AntiEntropyPush {
		/// The table being reconciled.
		table:		String,
		/// The records the recipient asked for.
		records:	Vec<Record>,
	},

	/// Forwarded write: "you are the HotStuff leader for this record;
	/// drive consensus on my behalf".
	///
	/// Sent by a peer whose own [`DistOzone::put`][p] call named a
	/// cohort-backed table but for which the peer is not the initial
	/// round's leader. The recipient leader creates a
	/// [`CohortInstance`][ci] (if it does not yet exist) and opens a
	/// [`MsgKind::CohortPropose`] round.
	///
	/// [p]: crate::dist::DistOzone::put
	/// [ci]: crate::consensus::CohortInstance
	CohortSubmit {
		/// The record to consent on.
		record:	Record,
	},
	/// HotStuff leader-to-cohort proposal for a specific record.
	///
	/// The `(table, id)` pair selects the per-record HotStuff instance; the
	/// [`Proposal`] itself carries the view, phase, block hash, optional
	/// block payload (on [`Phase::Prepare`][ph]) and optional justify QC.
	///
	/// [ph]: oxedyne_fe2o3_hotstuff::types::Phase::Prepare
	CohortPropose {
		/// The consensus-bearing table.
		table:		String,
		/// The record id the cohort is deciding on.
		id:			RecordId,
		/// The HotStuff proposal.
		proposal:	Proposal,
	},
	/// HotStuff replica-to-leader vote for a specific record.
	CohortVote {
		/// The consensus-bearing table.
		table:	String,
		/// The record id the cohort is deciding on.
		id:		RecordId,
		/// The HotStuff vote.
		vote:	Vote,
	},
	/// HotStuff view-change message for a specific record.
	///
	/// Sent by a replica to the incoming leader when its local timer fires
	/// without seeing progress in the current view.
	CohortNewView {
		/// The consensus-bearing table.
		table:		String,
		/// The record id the cohort is deciding on.
		id:			RecordId,
		/// The HotStuff view-change payload.
		new_view:	NewView,
	},
}

impl MsgKind {
	/// A short human-readable label for logging.
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
