//! The distributed-Ozone engine.
//!
//! [`DistOzone`] composes the placement service, the peer set, and a
//! [`Storage`] backend into one cohesive replication engine. It is a pure
//! state machine: every public method either reads state or returns the
//! outbound envelopes the caller should dispatch through its transport
//! adapter. No method calls `send` itself.
//!
//! # Write path
//!
//! [`DistOzone::put`] runs the placement decision, persists locally if the
//! local peer is a holder, and returns [`PutOutcome`] with a
//! [`ReplicatePut`](crate::transport::MsgKind::ReplicatePut) envelope for
//! every remote holder. The caller dispatches the envelopes; recipients
//! re-check their own placement decision, so a put that a sender directed
//! at a peer with a slightly different view of `N` can be silently dropped
//! by the recipient without harm -- the next anti-entropy round fills it
//! in.
//!
//! # Read path
//!
//! [`DistOzone::get`] reads from the local store if the local peer is a
//! holder, returning [`GetOutcome::Local`] or [`GetOutcome::LocalMiss`]. If
//! the local peer is *not* a holder, it returns [`GetOutcome::Remote`] with
//! a request id and the
//! [`GetRequest`](crate::transport::MsgKind::GetRequest) envelopes to
//! dispatch. The caller polls [`DistOzone::poll_get`] to learn when a
//! response has landed.

use crate::config::{
	Consistency,
	DistOzoneConfig,
	TableConfig,
};
use crate::peer_set::PeerSet;
use crate::placement::Placement;
use crate::record::{
	Record,
	RecordId,
};
use crate::storage::Storage;
use crate::transport::{
	Envelope,
	MsgKind,
	RequestId,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iblt::iblt::{
	DecodeOutcome,
	Iblt,
	IbltConfig,
};
use oxedyne_fe2o3_kademlia::id::{
	ID_LEN,
	NodeId,
};
use oxedyne_fe2o3_oam::config::OamConfig;

use std::collections::HashMap;
use std::sync::{
	Mutex,
	atomic::{
		AtomicU64,
		Ordering,
	},
};


/// Fixed key and value lengths used by the anti-entropy IBLT sketches.
///
/// The key is a 32-byte [`RecordId`] and the value is the 32-byte content
/// hash produced by [`Storage::digests`]. Matching lengths across peers is
/// mandatory for IBLT subtraction; callers cannot override.
const ANTI_ENTROPY_KEY_LEN:		usize = ID_LEN;
const ANTI_ENTROPY_VALUE_LEN:	usize = 32;


/// The default number of peers to which a remote read request is dispatched.
///
/// Choosing more than one protects against a single straggler; choosing many
/// wastes bandwidth. Three is the operating-point default referenced in the
/// spec ("an OAM holder" -- plural in realistic deployments).
pub const DEFAULT_READ_FANOUT: usize = 3;


/// The distributed-Ozone engine.
pub struct DistOzone<S: Storage> {
	cfg:			DistOzoneConfig,
	placement:		Placement,
	peer_set:		PeerSet,
	storage:		S,
	next_rid:		AtomicU64,
	pending_gets:	Mutex<HashMap<RequestId, PendingGet>>,
	read_fanout:	usize,
}

/// The in-flight state of a remote read.
#[derive(Clone, Debug)]
struct PendingGet {
	#[allow(dead_code)]	// Preserved for diagnostics and future retry logic.
	table:				String,
	#[allow(dead_code)]
	id:					RecordId,
	outstanding:		usize,
	first_response:		Option<Record>,
	resolved_empty:		bool,
}

impl<S: Storage> DistOzone<S> {
	/// Constructs a new engine from the configuration and a storage backend.
	///
	/// The bootstrap peer list is filtered to exclude the local peer, and the
	/// placement service's threshold is precomputed.
	pub fn new(cfg: DistOzoneConfig, storage: S) -> Outcome<Self> {
		let placement = Placement::new(cfg.local_peer_id, cfg.oam);
		let peer_set = PeerSet::from_bootstrap(
			&cfg.local_peer_id,
			cfg.bootstrap_peers.iter().cloned(),
		);
		Ok(Self {
			cfg,
			placement,
			peer_set,
			storage,
			next_rid:		AtomicU64::new(1),
			pending_gets:	Mutex::new(HashMap::new()),
			read_fanout:	DEFAULT_READ_FANOUT,
		})
	}

	/// Overrides the default read fanout. One is the minimum; values above
	/// the peer-set size are clamped automatically at request time.
	pub fn set_read_fanout(&mut self, fanout: usize) {
		self.read_fanout = fanout.max(1);
	}

	/// Returns the configuration.
	pub fn config(&self) -> &DistOzoneConfig {
		&self.cfg
	}

	/// Returns the current peer set.
	pub fn peer_set(&self) -> &PeerSet {
		&self.peer_set
	}

	/// Returns the placement service.
	pub fn placement(&self) -> &Placement {
		&self.placement
	}

	/// Returns the storage backend.
	pub fn storage(&self) -> &S {
		&self.storage
	}

	/// Inserts a peer into the rolling peer set. Returns `true` if the peer
	/// was new.
	pub fn insert_peer(&mut self, peer: NodeId) -> bool {
		if peer == self.cfg.local_peer_id {
			return false;
		}
		self.peer_set.insert(peer)
	}

	/// Removes a peer from the rolling peer set. Returns `true` if the peer
	/// was present.
	pub fn remove_peer(&mut self, peer: &NodeId) -> bool {
		self.peer_set.remove(peer)
	}

	/// Updates the OAM network-size estimate (typically from a HyperLogLog
	/// merge) and recomputes the cached placement threshold.
	pub fn update_network_size(&mut self, network_size: u64) -> Outcome<()> {
		let oam = res!(OamConfig::new(self.cfg.oam.replication, network_size));
		self.cfg.oam = oam;
		self.placement.update_oam(oam);
		Ok(())
	}

	/// Looks up a table config by name, or returns an error if the table is
	/// not declared in the configuration.
	fn table_or_err(&self, name: &str) -> Outcome<&TableConfig> {
		match self.cfg.table(name) {
			Some(t) => Ok(t),
			None => Err(err!(
				"Unknown table in DistOzone operation: {}.", name;
				Invalid, Input, Missing)),
		}
	}

	/// Writes a record, persisting it locally if the local peer is a holder
	/// and returning the outbound replicate envelopes for every remote
	/// holder.
	///
	/// Rejects unknown table names and cohort-backed tables. The latter
	/// route through the consensus path, which is not yet wired in this
	/// layer and would silently bypass the cohort if fall-through were
	/// permitted.
	pub fn put(&self, record: Record) -> Outcome<PutOutcome> {
		let tc = res!(self.table_or_err(&record.table));
		if !matches!(tc.consistency, Consistency::Eventual) {
			return Err(err!(
				"DistOzone::put does not yet support cohort-backed tables \
				(table '{}'). Writes to such tables must go through the \
				consensus path once it is wired in.",
				record.table;
				Invalid, Input, Unimplemented));
		}

		let decision = self.placement.decide(&record.id, &self.peer_set);
		let local_persisted = decision.local_is_holder;
		if local_persisted {
			res!(self.storage.put(&record));
		}

		let mut outbound = Vec::with_capacity(decision.remote_holders.len());
		for peer in decision.remote_holders {
			outbound.push(Envelope::new(
				self.cfg.local_peer_id,
				*peer,
				MsgKind::ReplicatePut { record: record.clone() },
			));
		}
		Ok(PutOutcome { local_persisted, outbound })
	}

	/// Reads a record. Returns immediately from local storage if the local
	/// peer is a holder; otherwise dispatches a request to the nearest
	/// peers and returns the in-flight request handle.
	pub fn get(
		&self,
		table:	&str,
		id:		&RecordId,
	)
		-> Outcome<GetOutcome>
	{
		res!(self.table_or_err(table));

		if self.placement.i_am_holder(id) {
			return Ok(match res!(self.storage.get(table, id)) {
				Some(r) => GetOutcome::Local(r),
				None => GetOutcome::LocalMiss,
			});
		}

		let targets = self.placement.read_targets(
			id,
			&self.peer_set,
			self.read_fanout,
		);
		if targets.is_empty() {
			return Ok(GetOutcome::NoTargets);
		}
		let request_id = self.next_rid.fetch_add(1, Ordering::Relaxed);
		let outbound = targets.iter()
			.map(|peer| Envelope::new(
				self.cfg.local_peer_id,
				**peer,
				MsgKind::GetRequest {
					request_id,
					table:	table.to_string(),
					id:		*id,
				},
			))
			.collect::<Vec<_>>();
		{
			let mut pending = lock_mutex!(self.pending_gets);
			pending.insert(request_id, PendingGet {
				table:				table.to_string(),
				id:					*id,
				outstanding:		outbound.len(),
				first_response:		None,
				resolved_empty:		false,
			});
		}
		Ok(GetOutcome::Remote { request_id, outbound })
	}

	/// Handles an incoming envelope, returning any outbound envelopes the
	/// handling produced and, for responses, the request id that was
	/// resolved.
	pub fn handle_envelope(&self, env: Envelope) -> Outcome<InboundOutcome> {
		if env.to != self.cfg.local_peer_id {
			// An envelope addressed to somebody else; ignore. This is mostly
			// a belt-and-braces guard: the transport adapter should not
			// deliver misaddressed envelopes in the first place.
			return Ok(InboundOutcome::empty());
		}
		match env.body {
			MsgKind::ReplicatePut { record } => {
				// Re-check placement: sender's view of N may disagree with
				// ours near the threshold. Drop silently if we do not
				// consider ourselves a holder.
				if !self.placement.i_am_holder(&record.id) {
					return Ok(InboundOutcome::empty());
				}
				// Reject puts for tables we do not know about.
				if self.cfg.table(&record.table).is_none() {
					return Err(err!(
						"ReplicatePut for unknown table '{}'.", record.table;
						Invalid, Input, Missing));
				}
				res!(self.storage.put(&record));
				Ok(InboundOutcome::empty())
			}
			MsgKind::GetRequest { request_id, table, id } => {
				if self.cfg.table(&table).is_none() {
					return Err(err!(
						"GetRequest for unknown table '{}'.", table;
						Invalid, Input, Missing));
				}
				let record = res!(self.storage.get(&table, &id));
				let reply = Envelope::new(
					self.cfg.local_peer_id,
					env.from,
					MsgKind::GetResponse { request_id, record },
				);
				Ok(InboundOutcome {
					outbound:		vec![reply],
					completed_get:	None,
				})
			}
			MsgKind::GetResponse { request_id, record } => {
				let completed = {
					let mut pending = lock_mutex!(self.pending_gets);
					let Some(slot) = pending.get_mut(&request_id) else {
						// Unknown request id; stale or cancelled response.
						return Ok(InboundOutcome::empty());
					};
					if slot.outstanding > 0 {
						slot.outstanding -= 1;
					}
					match record {
						Some(r) if slot.first_response.is_none() => {
							slot.first_response = Some(r);
						}
						None => {
							slot.resolved_empty = true;
						}
						_ => { /* later non-first response; ignore. */ }
					}
					// A pending get is "resolved" when either a response with
					// a record has landed or every target has replied empty.
					slot.first_response.is_some() || slot.outstanding == 0
				};
				Ok(InboundOutcome {
					outbound:		Vec::new(),
					completed_get:	if completed { Some(request_id) } else { None },
				})
			}
			MsgKind::AntiEntropyDigest { table, sketch } => {
				self.handle_anti_entropy_digest(env.from, table, sketch)
			}
			MsgKind::AntiEntropyReply { table, records, requested_ids, bulk } => {
				self.handle_anti_entropy_reply(
					env.from, table, records, requested_ids, bulk,
				)
			}
			MsgKind::AntiEntropyPush { table, records } => {
				self.handle_anti_entropy_push(table, records)
			}
		}
	}

	/// Reports on the state of a pending remote read.
	///
	/// Returns:
	/// - `PollOutcome::Pending` if the request is still in flight,
	/// - `PollOutcome::Record(r)` if at least one holder returned the record,
	/// - `PollOutcome::NotFound` if every holder reported no record,
	/// - `PollOutcome::Unknown` if the request id is not (or is no longer)
	///   known to the engine.
	pub fn poll_get(&self, request_id: RequestId) -> Outcome<PollOutcome> {
		let pending = lock_mutex!(self.pending_gets);
		let Some(slot) = pending.get(&request_id) else {
			return Ok(PollOutcome::Unknown);
		};
		if let Some(r) = &slot.first_response {
			return Ok(PollOutcome::Record(r.clone()));
		}
		if slot.outstanding == 0 && slot.resolved_empty {
			return Ok(PollOutcome::NotFound);
		}
		Ok(PollOutcome::Pending)
	}

	/// Discards the bookkeeping for a pending remote read.
	///
	/// Late responses that arrive after cancellation are ignored by the next
	/// [`handle_envelope`](Self::handle_envelope) call.
	pub fn cancel_get(&self, request_id: RequestId) -> Outcome<()> {
		let mut pending = lock_mutex!(self.pending_gets);
		pending.remove(&request_id);
		Ok(())
	}

	/// Builds an anti-entropy digest envelope for the given eventual-
	/// consistency table, addressed to a randomly- or caller-chosen target
	/// peer.
	///
	/// The envelope carries a serialised IBLT built from the local storage's
	/// [`Storage::digests`] enumeration for that table. The recipient
	/// subtracts it against its own sketch, decodes the symmetric difference
	/// and answers with [`MsgKind::AntiEntropyReply`].
	///
	/// Rejects unknown tables and cohort-backed tables (those reconcile
	/// through consensus, not anti-entropy).
	pub fn build_anti_entropy_request(
		&self,
		table:	&str,
		target:	NodeId,
	)
		-> Outcome<Envelope>
	{
		let tc = res!(self.table_or_err(table));
		if !matches!(tc.consistency, Consistency::Eventual) {
			return Err(err!(
				"anti-entropy is defined only for Eventual tables \
				(table '{}').", table;
				Invalid, Input, Unimplemented));
		}
		let iblt = res!(self.build_table_iblt(tc));
		let sketch = iblt.to_bytes();
		Ok(Envelope::new(
			self.cfg.local_peer_id,
			target,
			MsgKind::AntiEntropyDigest {
				table:	table.to_string(),
				sketch,
			},
		))
	}

	/// Builds an IBLT sketch of the entire contents of a table from the
	/// local storage backend. Factored out so both the request builder and
	/// the inbound digest handler use the same sketch shape.
	fn build_table_iblt(&self, tc: &TableConfig) -> Outcome<Iblt> {
		let cfg = IbltConfig {
			num_cells:	tc.iblt_cells,
			num_hashes:	TableConfig::IBLT_NUM_HASHES,
			key_len:	ANTI_ENTROPY_KEY_LEN,
			value_len:	ANTI_ENTROPY_VALUE_LEN,
			seed:		tc.iblt_seed(),
		};
		let mut iblt = res!(Iblt::new(cfg));
		let digests = res!(self.storage.digests(&tc.name));
		for d in digests {
			res!(iblt.insert(d.id.as_bytes(), &d.content));
		}
		Ok(iblt)
	}

	/// Handles an incoming anti-entropy digest: decodes the symmetric
	/// difference against the local sketch and returns an
	/// [`AntiEntropyReply`][ar] envelope carrying records the sender lacks
	/// and a list of record identifiers the recipient lacks. On sketch
	/// overload falls back to a bulk reply of every record the recipient
	/// holds for the table.
	///
	/// [ar]: MsgKind::AntiEntropyReply
	fn handle_anti_entropy_digest(
		&self,
		from:		NodeId,
		table:		String,
		sketch:		Vec<u8>,
	)
		-> Outcome<InboundOutcome>
	{
		let tc = res!(self.table_or_err(&table));
		if !matches!(tc.consistency, Consistency::Eventual) {
			return Err(err!(
				"anti-entropy digest received for non-Eventual table '{}'.",
				table;
				Invalid, Input, Unimplemented));
		}
		let expected_cfg = IbltConfig {
			num_cells:	tc.iblt_cells,
			num_hashes:	TableConfig::IBLT_NUM_HASHES,
			key_len:	ANTI_ENTROPY_KEY_LEN,
			value_len:	ANTI_ENTROPY_VALUE_LEN,
			seed:		tc.iblt_seed(),
		};
		let their_iblt = res!(Iblt::from_bytes(&sketch));
		if their_iblt.config() != expected_cfg {
			return Err(err!(
				"anti-entropy sketch config mismatch for table '{}'.",
				table;
				Invalid, Input, Mismatch));
		}
		let mut mine = res!(self.build_table_iblt(tc));
		res!(mine.subtract(&their_iblt));
		let decode = res!(mine.decode());
		let (records_for_sender, requested_ids, bulk) = match decode {
			DecodeOutcome::Complete { inserted, deleted } => {
				// `inserted` = keys in mine not in theirs -> records I
				// should send. `deleted` = keys in theirs not in mine ->
				// ids I should request.
				let mut records_for_sender = Vec::with_capacity(inserted.len());
				for (key_bytes, _value_hash) in inserted {
					let rid = res!(RecordId::from_slice(&key_bytes));
					if let Some(r) = res!(self.storage.get(&table, &rid)) {
						records_for_sender.push(r);
					}
				}
				let mut requested_ids = Vec::with_capacity(deleted.len());
				for (key_bytes, _value_hash) in deleted {
					requested_ids.push(res!(RecordId::from_slice(&key_bytes)));
				}
				(records_for_sender, requested_ids, false)
			}
			DecodeOutcome::Incomplete { .. } => {
				// Sketch overloaded. Fall back to bulk: send everything I
				// have for this table; the sender absorbs what it lacks.
				// This is simple and correct; a later optimisation can
				// teach the sender to retry with a larger sketch.
				let digests = res!(self.storage.digests(&table));
				let mut records = Vec::with_capacity(digests.len());
				for d in digests {
					if let Some(r) = res!(self.storage.get(&table, &d.id)) {
						records.push(r);
					}
				}
				(records, Vec::new(), true)
			}
		};
		let reply = Envelope::new(
			self.cfg.local_peer_id,
			from,
			MsgKind::AntiEntropyReply {
				table,
				records:		records_for_sender,
				requested_ids,
				bulk,
			},
		);
		Ok(InboundOutcome {
			outbound:		vec![reply],
			completed_get:	None,
		})
	}

	/// Handles an incoming anti-entropy reply: applies the records the
	/// recipient was missing, and builds an
	/// [`AntiEntropyPush`][ap] envelope for any records the recipient
	/// requested in return.
	///
	/// [ap]: MsgKind::AntiEntropyPush
	fn handle_anti_entropy_reply(
		&self,
		from:			NodeId,
		table:			String,
		records:		Vec<Record>,
		requested_ids:	Vec<RecordId>,
		_bulk:			bool,
	)
		-> Outcome<InboundOutcome>
	{
		res!(self.table_or_err(&table));

		// Apply every record the peer sent us, re-checking placement so a
		// stale-N sender cannot push a record to a peer that has since
		// stopped considering itself a holder.
		for record in records {
			if record.table != table {
				continue;
			}
			if self.placement.i_am_holder(&record.id) {
				res!(self.storage.put(&record));
			}
		}

		// Build a push for every requested id we actually have.
		let mut to_push = Vec::with_capacity(requested_ids.len());
		for rid in requested_ids {
			if let Some(r) = res!(self.storage.get(&table, &rid)) {
				to_push.push(r);
			}
		}
		if to_push.is_empty() {
			return Ok(InboundOutcome::empty());
		}
		let push = Envelope::new(
			self.cfg.local_peer_id,
			from,
			MsgKind::AntiEntropyPush {
				table,
				records:	to_push,
			},
		);
		Ok(InboundOutcome {
			outbound:		vec![push],
			completed_get:	None,
		})
	}

	/// Handles an incoming anti-entropy push: applies the records the
	/// originator sent in response to our request list. Each record is
	/// placement-checked before persistence.
	fn handle_anti_entropy_push(
		&self,
		table:		String,
		records:	Vec<Record>,
	)
		-> Outcome<InboundOutcome>
	{
		res!(self.table_or_err(&table));
		for record in records {
			if record.table != table {
				continue;
			}
			if self.placement.i_am_holder(&record.id) {
				res!(self.storage.put(&record));
			}
		}
		Ok(InboundOutcome::empty())
	}
}


/// The result of a [`DistOzone::put`] call.
#[derive(Clone, Debug)]
pub struct PutOutcome {
	/// `true` if the record was persisted to local storage.
	pub local_persisted:	bool,
	/// The replicate-put envelopes to dispatch to remote holders.
	pub outbound:			Vec<Envelope>,
}


/// The result of a [`DistOzone::get`] call.
#[derive(Clone, Debug)]
pub enum GetOutcome {
	/// The record was read from local storage.
	Local(Record),
	/// The local peer is a holder but has no record at that id.
	LocalMiss,
	/// The local peer is not a holder; a remote read has been initiated
	/// and the returned envelopes should be dispatched. Completion is
	/// reported through [`DistOzone::poll_get`].
	Remote {
		/// Correlation identifier for the outstanding request.
		request_id:	RequestId,
		/// The envelopes to dispatch.
		outbound:	Vec<Envelope>,
	},
	/// The local peer is not a holder and no remote targets are known
	/// (empty peer set, or a freshly-constructed engine).
	NoTargets,
}


/// The result of a [`DistOzone::handle_envelope`] call.
#[derive(Clone, Debug)]
pub struct InboundOutcome {
	/// Envelopes the caller should dispatch.
	pub outbound:		Vec<Envelope>,
	/// If this envelope completed a pending remote read, the request id of
	/// that read. The caller polls [`DistOzone::poll_get`] to collect the
	/// record itself.
	pub completed_get:	Option<RequestId>,
}

impl InboundOutcome {
	fn empty() -> Self {
		Self { outbound: Vec::new(), completed_get: None }
	}
}


/// The result of a [`DistOzone::poll_get`] query.
#[derive(Clone, Debug)]
pub enum PollOutcome {
	/// The request is still outstanding.
	Pending,
	/// A response has landed carrying the record.
	Record(Record),
	/// Every outstanding holder has replied that the record is not present.
	NotFound,
	/// The request id is unknown or has been cancelled.
	Unknown,
}
