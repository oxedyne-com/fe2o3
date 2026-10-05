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
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use super::cohort;
use super::config::{
	Consistency,
	DistOzoneConfig,
	TableConfig,
};
use super::consensus::{
	self,
	CohortInstance,
};
use super::hotstuff::{
	replica::Command as HsCommand,
	types::{
		BlockHash,
		NewView,
		Proposal,
		Vote,
	},
};
use super::peer_set::PeerSet;
use super::placement::Placement;
use super::record::{
	Record,
	RecordId,
};
use super::resolve::{
	LastVersionWins,
	ResolveCtx,
	Resolver,
	StorageView,
	Verdict,
};
use super::storage::Storage;
use super::transport::{
	Envelope,
	MsgKind,
	RequestId,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_data::iblt::{
	DecodeOutcome,
	Iblt,
	IbltConfig,
};
use crate::kademlia::id::{
	ID_LEN,
	NodeId,
};
use crate::oam::config::OamConfig;

use std::collections::{
	HashMap,
	hash_map::Entry,
};
use std::sync::{
	Mutex,
	MutexGuard,
	atomic::{
		AtomicU64,
		Ordering,
	},
};
use std::time::{
	SystemTime,
	UNIX_EPOCH,
};


// Fixed key and value lengths for the anti-entropy IBLT sketches. The key is a
// 32-byte RecordId and the value is the 32-byte content hash produced by
// Storage::digests. Matching lengths across peers is mandatory for IBLT
// subtraction; callers cannot override.
const ANTI_ENTROPY_KEY_LEN:		usize = ID_LEN;
const ANTI_ENTROPY_VALUE_LEN:	usize = 32;


// Choosing more than one peer protects against a single straggler; choosing
// many wastes bandwidth. Three is the operating-point default referenced in the
// spec ("an OAM holder" -- plural in realistic deployments).
pub const DEFAULT_READ_FANOUT: usize = 3;


pub struct DistOzone<S: Storage, R: Resolver = LastVersionWins> {
	cfg:			DistOzoneConfig,
	placement:		Placement,
	peer_set:		PeerSet,
	storage:		S,
	resolver:		R,
	// Held for one record's read, resolve and write, never across an envelope, a
	// sketch build, a bulk read or the caller's own post-write work.
	write:			Mutex<()>,
	next_rid:		AtomicU64,
	pending_gets:	Mutex<HashMap<RequestId, PendingGet>>,
	read_fanout:	usize,
	// An entry is created lazily on the first message, and kept after Decide so
	// duplicate late messages are silently absorbed.
	cohorts:		Mutex<HashMap<(String, RecordId), CohortInstance>>,
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

impl<S: Storage> DistOzone<S, LastVersionWins> {
	pub fn new(cfg: DistOzoneConfig, storage: S) -> Outcome<Self> {
		Self::with_resolver(cfg, storage, LastVersionWins)
	}
}

impl<S: Storage, R: Resolver> DistOzone<S, R> {
	/// The bootstrap peer list is filtered to exclude the local peer, and the
	/// placement service's threshold is precomputed.
	pub fn with_resolver(
		cfg:		DistOzoneConfig,
		storage:	S,
		resolver:	R,
	)
		-> Outcome<Self>
	{
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
			resolver,
			write:			Mutex::new(()),
			next_rid:		AtomicU64::new(1),
			pending_gets:	Mutex::new(HashMap::new()),
			read_fanout:	DEFAULT_READ_FANOUT,
			cohorts:		Mutex::new(HashMap::new()),
		})
	}

	/// One is the minimum; values above the peer-set size are clamped
	/// automatically at request time.
	pub fn set_read_fanout(&mut self, fanout: usize) {
		self.read_fanout = fanout.max(1);
	}

	pub fn config(&self) -> &DistOzoneConfig {
		&self.cfg
	}

	pub fn peer_set(&self) -> &PeerSet {
		&self.peer_set
	}

	pub fn placement(&self) -> &Placement {
		&self.placement
	}

	pub fn storage(&self) -> &S {
		&self.storage
	}

	pub fn resolver(&self) -> &R {
		&self.resolver
	}

	/// Ignores the local peer; true if the peer was new.
	pub fn insert_peer(&mut self, peer: NodeId) -> bool {
		if peer == self.cfg.local_peer_id {
			return false;
		}
		self.peer_set.insert(peer)
	}

	pub fn remove_peer(&mut self, peer: &NodeId) -> bool {
		self.peer_set.remove(peer)
	}

	/// Recomputes the cached placement threshold. The estimate typically comes
	/// from a HyperLogLog merge.
	pub fn update_network_size(&mut self, network_size: u64) -> Outcome<()> {
		let oam = res!(OamConfig::new(self.cfg.oam.replication, network_size));
		self.cfg.oam = oam;
		self.placement.update_oam(oam);
		Ok(())
	}

	fn table_or_err(&self, name: &str) -> Outcome<&TableConfig> {
		match self.cfg.table(name) {
			Some(t) => Ok(t),
			None => Err(err!(
				"Unknown table in DistOzone operation: {}.", name;
				Invalid, Input, Missing)),
		}
	}

	/// [`put_at`](Self::put_at) at the wall-clock time.
	pub fn put(&self, record: Record) -> Outcome<PutOutcome> {
		self.put_at(record, res!(wall_ms()))
	}

	/// On an eventual table, a holder offers the record to the resolver. On
	/// [`Verdict::Take`] the resolver's bytes are stored and a
	/// [`MsgKind::ReplicatePut`] envelope carrying them is emitted for every
	/// remote holder. A `Keep`, `Defer` or `Refuse` stores and sends nothing, and
	/// is returned in [`PutOutcome::verdict`]. A peer that is not a holder
	/// forwards the record as given.
	///
	/// On a cohort-backed table the put enters a HotStuff round. If the
	/// local peer is the cohort's initial leader the engine opens a
	/// [`MsgKind::CohortPropose`] round and returns the proposal envelopes.
	/// Otherwise it emits a [`MsgKind::CohortSubmit`] envelope to the
	/// leader, who drives the round on the submitter's behalf. In both
	/// cases [`PutOutcome::consensus_pending`] is set; the caller learns
	/// when consensus completes through
	/// [`InboundOutcome::completed_consensus_put`].
	pub fn put_at(&self, record: Record, now_ms: u64) -> Outcome<PutOutcome> {
		let tc = res!(self.table_or_err(&record.table));
		match tc.consistency {
			Consistency::Eventual =>
				self.put_eventual(&ResolveCtx::at(now_ms), record),
			Consistency::Cohort { lambda } =>
				self.put_cohort(record, lambda),
		}
	}

	fn put_eventual(
		&self,
		ctx:	&ResolveCtx,
		record:	Record,
	)
		-> Outcome<PutOutcome>
	{
		let decision = self.placement.decide(&record.id, &self.peer_set);
		let verdict = res!(self.apply(ctx, &record));
		// What the remote holders are sent: the stored bytes after a Take, the
		// input where this peer is not a holder, and nothing otherwise.
		let (local_persisted, send) = match &verdict {
			None						=> (false, Some(record)),
			Some(Verdict::Take(bytes))	=> {
				let stored = Record::new(record.id, record.table, bytes.clone());
				(true, Some(stored))
			},
			Some(_)						=> (false, None),
		};

		let mut outbound = Vec::with_capacity(decision.remote_holders.len());
		if let Some(rec) = send {
			for peer in decision.remote_holders {
				outbound.push(Envelope::new(
					self.cfg.local_peer_id,
					*peer,
					MsgKind::ReplicatePut { record: rec.clone() },
				));
			}
		}
		Ok(PutOutcome {
			local_persisted,
			outbound,
			consensus_pending:	None,
			verdict,
		})
	}

	// The write lock, recovered if a panic poisoned it. It guards `()`, so a
	// poison carries no torn state: a panic in a resolver has written nothing,
	// and one in the storage is the storage's own lock to answer for. Failing
	// every later write instead would stop this peer converging until restart.
	// Not lock_mutex_or_recover!, which logs, and dist does not.
	fn write_lock(&self) -> MutexGuard<'_, ()> {
		match self.write.lock() {
			Ok(guard)		=> guard,
			Err(poisoned)	=> poisoned.into_inner(),
		}
	}

	// The one path by which a record reaches storage from an eventual table.
	// None means the local peer is not a holder, so the resolver was not called.
	// A Take of the bytes already held is a Keep: nothing is written, so two
	// peers that agree do not echo the record to each other. A cohort table is
	// an Err whoever sent the record: consensus is its only verdict, so a peer
	// cannot overwrite a decided value by the eventual path.
	fn apply(
		&self,
		ctx:	&ResolveCtx,
		record:	&Record,
	)
		-> Outcome<Option<Verdict>>
	{
		let tc = res!(self.table_or_err(&record.table));
		if !matches!(tc.consistency, Consistency::Eventual) {
			return Err(err!(
				"Table '{}' is a cohort table, which takes writes only through consensus, \
				not as an eventual record.", record.table;
				Invalid, Input, Mismatch));
		}
		if !self.placement.i_am_holder(&record.id) {
			return Ok(None);
		}
		// One record at a time, so the read, the resolve and the write are atomic.
		// The resolver's view takes only the storage's own lock, for one read.
		let _guard = self.write_lock();
		let held = res!(self.storage.get(&record.table, &record.id)).map(|r| r.value);
		let verdict = res!(self.resolver.resolve(
			ctx,
			&StorageView(&self.storage),
			&record.table,
			&record.id,
			held.as_deref(),
			&record.value,
		));
		match verdict {
			Verdict::Take(bytes) => {
				if held.as_deref() == Some(bytes.as_slice()) {
					return Ok(Some(Verdict::Keep));
				}
				res!(self.storage.put(&Record::new(
					record.id,
					record.table.clone(),
					bytes.clone(),
				)));
				Ok(Some(Verdict::Take(bytes)))
			},
			other => Ok(Some(other)),
		}
	}

	fn put_cohort(&self, record: Record, lambda: u64) -> Outcome<PutOutcome> {
		let table = record.table.clone();
		let id = record.id;
		let sel = res!(cohort::select(
			&table,
			&id,
			&self.peer_set,
			&self.cfg.local_peer_id,
			lambda,
		));
		if sel.local_is_leader {
			// Drive the round locally. PutOutcome has no persisted list, so a
			// Decide reached inside this call is not reported.
			let mut persisted = Vec::new();
			let outbound = res!(self.leader_open_round(sel, record, &mut persisted));
			Ok(PutOutcome {
				local_persisted:	false,
				outbound,
				consensus_pending:	Some((table, id)),
				verdict:			None,
			})
		} else {
			// Forward to the leader.
			let env = Envelope::new(
				self.cfg.local_peer_id,
				sel.leader,
				MsgKind::CohortSubmit { record },
			);
			Ok(PutOutcome {
				local_persisted:	false,
				outbound:			vec![env],
				consensus_pending:	Some((table, id)),
				verdict:			None,
			})
		}
	}

	fn leader_open_round(
		&self,
		sel:		cohort::Cohort,
		record:		Record,
		persisted:	&mut Vec<(String, RecordId)>,
	)
		-> Outcome<Vec<Envelope>>
	{
		let table = record.table.clone();
		let id = record.id;
		let block = consensus::encode_record(&record);
		let block_hash = consensus::block_hash(&block);

		let lambda = sel.members.len() as u64;
		let mut cohorts = lock_mutex!(self.cohorts);
		let instance = match cohorts.entry((table.clone(), id)) {
			Entry::Occupied(e)	=> e.into_mut(),
			Entry::Vacant(v)	=> v.insert(res!(CohortInstance::new(
				sel,
				&self.cfg.local_peer_id,
				lambda,
			))),
		};
		if instance.has_decided() {
			return Ok(Vec::new());
		}
		let cmds = res!(instance.replica.propose(block, block_hash));
		let outbound = res!(self.translate_commands(&table, &id, instance, cmds, persisted));
		Ok(outbound)
	}

	/// Applies the local side effects -- persistence on Decide -- as it goes, and
	/// lists each decided record in `persisted`.
	fn translate_commands(
		&self,
		table:		&str,
		id:			&RecordId,
		instance:	&mut CohortInstance,
		cmds:		Vec<HsCommand>,
		persisted:	&mut Vec<(String, RecordId)>,
	)
		-> Outcome<Vec<Envelope>>
	{
		let mut out = Vec::new();
		for cmd in cmds {
			match cmd {
				HsCommand::BroadcastProposal(proposal) => {
					// Broadcast to every cohort member *except* ourselves.
					// HotStuff's on_proposal recipe expects the leader to
					// record its own vote via on_proposal too, so we also
					// feed the proposal back into our local replica.
					let self_id = instance.replica.config().self_id;
					for (i, member) in instance.members.iter().enumerate() {
						if (i as u16) == self_id {
							continue;
						}
						out.push(Envelope::new(
							self.cfg.local_peer_id,
							*member,
							MsgKind::CohortPropose {
								table:		table.to_string(),
								id:			*id,
								proposal:	proposal.clone(),
							},
						));
					}
					// Now fold the proposal into the local replica. We
					// recurse on the commands produced, which lets a leader
					// that is also a voter correctly emit its own SendVote.
					let local_cmds = res!(instance.replica.on_proposal(proposal));
					let more = res!(self.translate_commands(
						table, id, instance, local_cmds, persisted,
					));
					out.extend(more);
				},
				HsCommand::SendVote { to, vote } => {
					let target = match instance.node_id(to) {
						Some(n) => n,
						None => return Err(err!(
							"HotStuff SendVote targets replica id {} \
							which is out of cohort range (size = {}).",
							to, instance.members.len();
							Bug, Invalid)),
					};
					// The leader of the current view is the usual target; a
					// vote sent to ourselves needs to be folded in directly
					// rather than sent on the wire.
					if target == self.cfg.local_peer_id {
						let local_cmds = res!(instance.replica.on_vote(vote));
						let more = res!(self.translate_commands(
							table, id, instance, local_cmds, persisted,
						));
						out.extend(more);
					} else {
						out.push(Envelope::new(
							self.cfg.local_peer_id,
							target,
							MsgKind::CohortVote {
								table:	table.to_string(),
								id:		*id,
								vote,
							},
						));
					}
				},
				HsCommand::SendNewView { to, new_view } => {
					let target = match instance.node_id(to) {
						Some(n) => n,
						None => return Err(err!(
							"HotStuff SendNewView targets replica id {} \
							which is out of cohort range (size = {}).",
							to, instance.members.len();
							Bug, Invalid)),
					};
					if target == self.cfg.local_peer_id {
						let local_cmds = res!(instance.replica.on_new_view(new_view));
						let more = res!(self.translate_commands(
							table, id, instance, local_cmds, persisted,
						));
						out.extend(more);
					} else {
						out.push(Envelope::new(
							self.cfg.local_peer_id,
							target,
							MsgKind::CohortNewView {
								table:		table.to_string(),
								id:			*id,
								new_view,
							},
						));
					}
				},
				HsCommand::Decide { view: _, block } => {
					let record = res!(consensus::decode_record(&block));
					if record.table != table || &record.id != id {
						return Err(err!(
							"HotStuff Decide block decoded to a record at \
							({}, {:?}) that does not match the consensus \
							slot ({}, {:?}).",
							record.table, record.id.as_bytes(),
							table, id.as_bytes();
							Bug, Invalid, Mismatch));
					}
					let hash = consensus::block_hash(&block);
					res!(self.store_decided(instance, hash, &record));
					persisted.push((record.table, record.id));
				},
			}
		}
		Ok(out)
	}

	// Stores a decided record, then marks the instance decided, so a failed store
	// never leaves the instance decided with no record. A second Decide on another
	// block is refused before anything is stored. The replica is inert once it has
	// emitted a Decide, so a failed store holds the record in the instance for
	// cohort_timeout to store again. Consensus is the verdict on a cohort table, so
	// there is no resolver; the write lock keeps a resolver's cross-table read from
	// seeing this store half way.
	fn store_decided(
		&self,
		instance:	&mut CohortInstance,
		hash:		BlockHash,
		record:		&Record,
	)
		-> Outcome<()>
	{
		res!(instance.check_decide(hash));
		let stored = {
			let _guard = self.write_lock();
			self.storage.put(record)
		};
		if let Err(e) = stored {
			instance.unstored = Some((hash, record.clone()));
			return Err(e);
		}
		instance.unstored = None;
		instance.mark_decided(hash)
	}

	/// The caller owns the timer; on expiry it calls this and dispatches the
	/// returned envelopes. Empty if the instance is absent -- already decided,
	/// or never created. An instance whose decided record failed to store is
	/// stored again here, and the call is an `Err` while the store still fails,
	/// so the caller keeps its timer until `completed_consensus_put` is reported
	/// or this call succeeds.
	pub fn cohort_timeout(
		&self,
		table:	&str,
		id:		&RecordId,
	)
		-> Outcome<Vec<Envelope>>
	{
		let mut cohorts = lock_mutex!(self.cohorts);
		let instance = match cohorts.get_mut(&(table.to_string(), *id)) {
			Some(i) => i,
			None => return Ok(Vec::new()),
		};
		if let Some((hash, record)) = instance.unstored.take() {
			res!(self.store_decided(instance, hash, &record));
			return Ok(Vec::new());
		}
		if instance.has_decided() {
			return Ok(Vec::new());
		}
		let cmds = res!(instance.replica.on_timeout());
		// The return type has no persisted list, so a Decide reached here is not reported.
		let mut persisted = Vec::new();
		self.translate_commands(table, id, instance, cmds, &mut persisted)
	}

	/// Reads from local storage if the local peer is a holder; otherwise
	/// dispatches a request to the nearest peers and returns the in-flight
	/// request handle.
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

	/// [`handle_envelope_at`](Self::handle_envelope_at) at the wall-clock time.
	pub fn handle_envelope(&self, env: Envelope) -> Outcome<InboundOutcome> {
		self.handle_envelope_at(env, res!(wall_ms()))
	}

	/// Every record in an envelope gets exactly one result: stored (listed in
	/// [`InboundOutcome::persisted`]), kept (listed nowhere), deferred, refused,
	/// or failed. A record this peer does not hold is dropped silently. One
	/// record's result never stops the rest of the envelope being applied.
	pub fn handle_envelope_at(
		&self,
		env:	Envelope,
		now_ms:	u64,
	)
		-> Outcome<InboundOutcome>
	{
		let ctx = ResolveCtx::at(now_ms);
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
				let mut out = InboundOutcome::empty();
				out.note(&record.table, record.id, self.apply(&ctx, &record));
				Ok(out)
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
					outbound:	vec![reply],
					..InboundOutcome::empty()
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
					completed_get:	if completed { Some(request_id) } else { None },
					..InboundOutcome::empty()
				})
			}
			MsgKind::AntiEntropyDigest { table, sketch } => {
				self.handle_anti_entropy_digest(env.from, table, sketch)
			}
			MsgKind::AntiEntropyReply { table, records, requested_ids, bulk } => {
				self.handle_anti_entropy_reply(
					&ctx, env.from, table, records, requested_ids, bulk,
				)
			}
			MsgKind::AntiEntropyPush { table, records } => {
				self.handle_anti_entropy_push(&ctx, table, records)
			}
			MsgKind::CohortSubmit { record } => {
				self.handle_cohort_submit(record)
			}
			MsgKind::CohortPropose { table, id, proposal } => {
				self.handle_cohort_propose(env.from, table, id, proposal)
			}
			MsgKind::CohortVote { table, id, vote } => {
				self.handle_cohort_vote(table, id, vote)
			}
			MsgKind::CohortNewView { table, id, new_view } => {
				self.handle_cohort_new_view(table, id, new_view)
			}
		}
	}

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

	/// Late responses that arrive after cancellation are ignored by the next
	/// [`handle_envelope`](Self::handle_envelope) call.
	pub fn cancel_get(&self, request_id: RequestId) -> Outcome<()> {
		let mut pending = lock_mutex!(self.pending_gets);
		pending.remove(&request_id);
		Ok(())
	}

	/// The envelope carries a serialised IBLT built from the local storage's
	/// [`Storage::digests`] enumeration for that table. The recipient
	/// subtracts it against its own sketch, decodes the symmetric difference
	/// and answers with [`MsgKind::AntiEntropyReply`]. Cohort-backed tables
	/// reconcile through consensus rather than anti-entropy, and are rejected.
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

	/// Factored out so both the request builder and the inbound digest handler
	/// use the same sketch shape.
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

	/// Decodes the symmetric difference against the local sketch and returns an
	/// [`AntiEntropyReply`][ar] envelope carrying records the sender lacks and
	/// a list of record identifiers the recipient lacks. On sketch overload it
	/// falls back to a bulk reply of every record the recipient holds for the
	/// table.
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
			outbound:	vec![reply],
			..InboundOutcome::empty()
		})
	}

	/// Applies the records the recipient was missing, and builds an
	/// [`AntiEntropyPush`][ap] envelope for any records requested in return.
	///
	/// [ap]: MsgKind::AntiEntropyPush
	fn handle_anti_entropy_reply(
		&self,
		ctx:			&ResolveCtx,
		from:			NodeId,
		table:			String,
		records:		Vec<Record>,
		requested_ids:	Vec<RecordId>,
		_bulk:			bool,
	)
		-> Outcome<InboundOutcome>
	{
		res!(self.table_or_err(&table));

		// Offer every record the peer sent us to the resolver. The apply step
		// re-checks placement, so a stale-N sender cannot push a record to a peer
		// that has since stopped considering itself a holder.
		let mut out = InboundOutcome::empty();
		for record in records {
			if record.table != table {
				continue;
			}
			out.note(&record.table, record.id, self.apply(ctx, &record));
		}

		// Build a push for every requested id we actually have. A failed read is
		// skipped and not listed, since `failed` is for received records and a read
		// fault here is this peer's own; it cannot lose the results above, and the
		// peer asks again next round.
		let mut to_push = Vec::with_capacity(requested_ids.len());
		for rid in requested_ids {
			match self.storage.get(&table, &rid) {
				Ok(Some(r))			=> to_push.push(r),
				Ok(None) | Err(_)	=> {},
			}
		}
		if !to_push.is_empty() {
			out.outbound.push(Envelope::new(
				self.cfg.local_peer_id,
				from,
				MsgKind::AntiEntropyPush {
					table,
					records:	to_push,
				},
			));
		}
		Ok(out)
	}

	/// Opens a HotStuff round if the local peer is the initial leader for the
	/// `(table, record_id)` pair. If it is not, the submission is dropped
	/// silently -- almost always a stale-cohort race, where the submitter's
	/// view of the peer set differed from the leader's.
	fn handle_cohort_submit(
		&self,
		record:	Record,
	)
		-> Outcome<InboundOutcome>
	{
		let tc = res!(self.table_or_err(&record.table));
		let lambda = match tc.consistency {
			Consistency::Cohort { lambda } => lambda,
			Consistency::Eventual => return Err(err!(
				"CohortSubmit received for eventual-consistency table '{}'.",
				record.table;
				Invalid, Input, Mismatch)),
		};
		let sel = res!(cohort::select(
			&record.table,
			&record.id,
			&self.peer_set,
			&self.cfg.local_peer_id,
			lambda,
		));
		if !sel.local_is_leader {
			// Not our job -- drop silently.
			return Ok(InboundOutcome::empty());
		}
		let mut persisted = Vec::new();
		let outbound = res!(self.leader_open_round(sel, record, &mut persisted));
		Ok(InboundOutcome {
			outbound,
			persisted,
			..InboundOutcome::empty()
		})
	}

	/// Creates a fresh per-record replica if one does not yet exist. A proposal
	/// addressed to a peer that is not a cohort member is dropped silently.
	fn handle_cohort_propose(
		&self,
		from:		NodeId,
		table:		String,
		id:			RecordId,
		proposal:	Proposal,
	)
		-> Outcome<InboundOutcome>
	{
		let tc = res!(self.table_or_err(&table));
		let lambda = match tc.consistency {
			Consistency::Cohort { lambda } => lambda,
			Consistency::Eventual => return Err(err!(
				"CohortPropose received for eventual-consistency table '{}'.",
				table;
				Invalid, Input, Mismatch)),
		};
		let sel = res!(cohort::select(
			&table,
			&id,
			&self.peer_set,
			&self.cfg.local_peer_id,
			lambda,
		));
		if !sel.local_is_member {
			// Proposal arrived but we are not in the cohort -- drop. This
			// can happen if peer-set views disagree; the sender will retry
			// after its own set updates.
			return Ok(InboundOutcome::empty());
		}
		// Verify the proposal came from someone plausibly in the cohort.
		// Stronger origin checks (view-aware leader matching) live in the
		// HotStuff replica itself.
		if !sel.members.iter().any(|m| m == &from) {
			return Ok(InboundOutcome::empty());
		}
		let mut cohorts = lock_mutex!(self.cohorts);
		let instance = match cohorts.entry((table.clone(), id)) {
			Entry::Occupied(e)	=> e.into_mut(),
			Entry::Vacant(v)	=> v.insert(res!(CohortInstance::new(
				sel,
				&self.cfg.local_peer_id,
				lambda,
			))),
		};
		if instance.has_decided() {
			return Ok(InboundOutcome::empty());
		}
		let before_decided = instance.has_decided();
		let mut persisted = Vec::new();
		let cmds = res!(instance.replica.on_proposal(proposal));
		let outbound = res!(self.translate_commands(&table, &id, instance, cmds, &mut persisted));
		let completed_consensus_put = if !before_decided && instance.has_decided() {
			Some((table, id))
		} else {
			None
		};
		Ok(InboundOutcome {
			outbound,
			completed_consensus_put,
			persisted,
			..InboundOutcome::empty()
		})
	}

	/// Leader-only: a non-leader replica silently ignores a vote, per the
	/// HotStuff spec.
	fn handle_cohort_vote(
		&self,
		table:	String,
		id:		RecordId,
		vote:	Vote,
	)
		-> Outcome<InboundOutcome>
	{
		let mut cohorts = lock_mutex!(self.cohorts);
		let key = (table.clone(), id);
		let instance = match cohorts.get_mut(&key) {
			Some(i) => i,
			None => {
				// Vote arrived before we created an instance. Drop -- a
				// well-behaved cohort member only votes after seeing the
				// leader's Propose, so by the time a vote reaches us we
				// should already have an instance. This case is most
				// likely an adversarial or replayed envelope.
				return Ok(InboundOutcome::empty());
			},
		};
		if instance.has_decided() {
			return Ok(InboundOutcome::empty());
		}
		let before_decided = instance.has_decided();
		let mut persisted = Vec::new();
		let cmds = res!(instance.replica.on_vote(vote));
		let outbound = res!(self.translate_commands(&table, &id, instance, cmds, &mut persisted));
		let completed_consensus_put = if !before_decided && instance.has_decided() {
			Some((table, id))
		} else {
			None
		};
		Ok(InboundOutcome {
			outbound,
			completed_consensus_put,
			persisted,
			..InboundOutcome::empty()
		})
	}

	fn handle_cohort_new_view(
		&self,
		table:		String,
		id:			RecordId,
		new_view:	NewView,
	)
		-> Outcome<InboundOutcome>
	{
		let mut cohorts = lock_mutex!(self.cohorts);
		let key = (table.clone(), id);
		let instance = match cohorts.get_mut(&key) {
			Some(i) => i,
			None => return Ok(InboundOutcome::empty()),
		};
		if instance.has_decided() {
			return Ok(InboundOutcome::empty());
		}
		let mut persisted = Vec::new();
		let cmds = res!(instance.replica.on_new_view(new_view));
		let outbound = res!(self.translate_commands(&table, &id, instance, cmds, &mut persisted));
		Ok(InboundOutcome {
			outbound,
			persisted,
			..InboundOutcome::empty()
		})
	}

	/// Each record is placement-checked, then offered to the resolver.
	fn handle_anti_entropy_push(
		&self,
		ctx:		&ResolveCtx,
		table:		String,
		records:	Vec<Record>,
	)
		-> Outcome<InboundOutcome>
	{
		res!(self.table_or_err(&table));
		let mut out = InboundOutcome::empty();
		for record in records {
			if record.table != table {
				continue;
			}
			out.note(&record.table, record.id, self.apply(ctx, &record));
		}
		Ok(out)
	}
}


/// The result of a [`DistOzone::put`] call.
#[derive(Clone, Debug)]
pub struct PutOutcome {
	// Always false for a cohort-backed write, which persists on Decide and is
	// signalled through InboundOutcome::completed_consensus_put.
	pub local_persisted:	bool,
	pub outbound:			Vec<Envelope>,
	// Set when the put entered a HotStuff consensus round.
	pub consensus_pending:	Option<(String, RecordId)>,
	// None when the local peer is not a holder, or on a cohort table.
	pub verdict:			Option<Verdict>,
}


/// The result of a [`DistOzone::get`] call.
#[derive(Clone, Debug)]
pub enum GetOutcome {
	Local(Record),
	LocalMiss,	// a holder, but no record at that id
	// A remote read has been initiated; completion is reported through
	// DistOzone::poll_get.
	Remote {
		request_id:	RequestId,
		outbound:	Vec<Envelope>,
	},
	NoTargets,	// not a holder, and no remote targets are known
}


/// The result of a [`DistOzone::handle_envelope`] call.
#[derive(Clone, Debug)]
pub struct InboundOutcome {
	pub outbound:		Vec<Envelope>,
	// Set when this envelope completed a pending remote read; the caller polls
	// DistOzone::poll_get to collect the record itself.
	pub completed_get:	Option<RequestId>,
	// Set when this envelope drove a cohort round to Decide and the record was
	// persisted locally.
	pub completed_consensus_put:	Option<(String, RecordId)>,
	// Per-record results of an envelope. A record is in at most one list, and a
	// Keep, or a record this peer does not hold, is in none. The engine does not
	// log: the caller logs refused and failed, with the envelope's sender.
	pub persisted:	Vec<(String, RecordId)>,					// stored on a Take, or a cohort Decide
	pub deferred:	Vec<(String, RecordId)>,					// not stored; anti-entropy offers it again
	pub refused:	Vec<(String, RecordId, String)>,			// not stored; the resolver's reason
	pub failed:		Vec<(String, RecordId, Error<ErrTag>)>,		// resolve or store returned Err
}

impl InboundOutcome {
	fn empty() -> Self {
		Self {
			outbound:					Vec::new(),
			completed_get:				None,
			completed_consensus_put:	None,
			persisted:					Vec::new(),
			deferred:					Vec::new(),
			refused:					Vec::new(),
			failed:						Vec::new(),
		}
	}

	// Files one record's result in exactly one list, or none.
	fn note(
		&mut self,
		table:	&str,
		id:		RecordId,
		result:	Outcome<Option<Verdict>>,
	) {
		match result {
			Ok(None)						=> {},
			Ok(Some(Verdict::Take(_)))		=> self.persisted.push((table.to_string(), id)),
			Ok(Some(Verdict::Keep))			=> {},
			Ok(Some(Verdict::Defer))		=> self.deferred.push((table.to_string(), id)),
			Ok(Some(Verdict::Refuse(why)))	=> self.refused.push((table.to_string(), id, why)),
			Err(e)							=> self.failed.push((table.to_string(), id, e)),
		}
	}
}


// The wall clock behind put and handle_envelope.
fn wall_ms() -> Outcome<u64> {
	let since = res!(SystemTime::now().duration_since(UNIX_EPOCH));
	Ok(since.as_millis() as u64)
}


/// The result of a [`DistOzone::poll_get`] query.
#[derive(Clone, Debug)]
pub enum PollOutcome {
	Pending,
	Record(Record),
	NotFound,	// every outstanding holder replied that the record is absent
	Unknown,	// unknown, or cancelled
}
