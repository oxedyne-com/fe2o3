//! The binary wire form of distributed Ozone's envelopes.
//!
//! An [`Envelope`] is the JDAT list `[version, from, to, body]`, written in the BDAT binary
//! encoding, and the body is `[kind, fields]`. The layouts are part of the protocol: a peer built
//! from another revision reads exactly the bytes this module writes, so
//!
//! - a message kind keeps its number for ever, and a retired kind leaves a gap;
//! - a change to any layout bumps [`WIRE_VERSION`], and a reader refuses a version it does not
//!   know with a `Mismatch` rather than guess at its fields;
//! - a reader never trusts the bytes: [`Envelope::decode`] bounds their length and nesting before
//!   it decodes, names the field of every refusal, and accepts only a shape that
//!   [`Envelope::encode`] can produce.
//!
//! The first form has never run in a cluster, so there is nothing older to stay compatible with.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use super::hotstuff::types::{
	BlockHash,
	NewView,
	Phase,
	Proposal,
	Qc,
	ReplicaId,
	Vote,
};
use super::record::{
	Record,
	RecordId,
};
use super::transport::{
	Envelope,
	MsgKind,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
	bdat::DecodeLimits,
	prelude::*,
};
use crate::kademlia::id::NodeId;


pub const WIRE_VERSION:			u8		= 1;				// first field of every envelope
pub const MAX_ENVELOPE_BYTES:	usize	= 16 * 1024 * 1024;	// longest envelope written or read
pub const MAX_ENVELOPE_DEPTH:	usize	= 12;				// the deepest shape written is nine

// Message kind numbers. A number is never reused, even after its kind is retired.
const KIND_REPLICATE_PUT:		u8 = 1;
const KIND_GET_REQUEST:			u8 = 2;
const KIND_GET_RESPONSE:		u8 = 3;
const KIND_ANTI_ENTROPY_DIGEST:	u8 = 4;
const KIND_ANTI_ENTROPY_REPLY:	u8 = 5;
const KIND_ANTI_ENTROPY_PUSH:	u8 = 6;
const KIND_COHORT_SUBMIT:		u8 = 7;
const KIND_COHORT_PROPOSE:		u8 = 8;
const KIND_COHORT_VOTE:			u8 = 9;
const KIND_COHORT_NEW_VIEW:		u8 = 10;

// Phase numbers on the wire.
const PHASE_PREPARE:			u8 = 0;
const PHASE_PRE_COMMIT:			u8 = 1;
const PHASE_COMMIT:				u8 = 2;
const PHASE_DECIDE:				u8 = 3;


// The limits every received envelope is decoded under.
fn limits() -> DecodeLimits {
	DecodeLimits::new(MAX_ENVELOPE_DEPTH, MAX_ENVELOPE_BYTES)
}

// A refusal of a decoded shape, naming the shape, the field and what was found. It holds the
// kind of the offending value and never the value, which a peer chooses and may make huge.
fn refuse(what: &str, field: &str, want: &str, found: &Dat) -> Error<ErrTag> {
	err!(
		"{} field '{}' expects {}, found {:?}.",
		what, field, want, found.kind();
	Decode, Input, Mismatch)
}

fn want_u8(dat: Dat, what: &str, field: &str) -> Outcome<u8> {
	match dat {
		Dat::U8(n)	=> Ok(n),
		other		=> Err(refuse(what, field, "Dat::U8", &other)),
	}
}

fn want_u16(dat: Dat, what: &str, field: &str) -> Outcome<u16> {
	match dat {
		Dat::U16(n)	=> Ok(n),
		other		=> Err(refuse(what, field, "Dat::U16", &other)),
	}
}

fn want_u64(dat: Dat, what: &str, field: &str) -> Outcome<u64> {
	match dat {
		Dat::U64(n)	=> Ok(n),
		other		=> Err(refuse(what, field, "Dat::U64", &other)),
	}
}

fn want_bool(dat: Dat, what: &str, field: &str) -> Outcome<bool> {
	match dat {
		Dat::Bool(b)	=> Ok(b),
		other			=> Err(refuse(what, field, "Dat::Bool", &other)),
	}
}

fn want_str(dat: Dat, what: &str, field: &str) -> Outcome<String> {
	match dat {
		Dat::Str(s)	=> Ok(s),
		other		=> Err(refuse(what, field, "Dat::Str", &other)),
	}
}

// Value bytes are a `Dat::BU64`, never a `Dat::BU8`, whose length field holds 255 at most.
fn want_bytes(dat: Dat, what: &str, field: &str) -> Outcome<Vec<u8>> {
	match dat {
		Dat::BU64(v)	=> Ok(v),
		other			=> Err(refuse(what, field, "Dat::BU64", &other)),
	}
}

fn want_b32(dat: Dat, what: &str, field: &str) -> Outcome<[u8; 32]> {
	match dat {
		Dat::B32(b)	=> Ok(b.0),
		other		=> Err(refuse(what, field, "Dat::B32", &other)),
	}
}

fn want_list(dat: Dat, what: &str, field: &str) -> Outcome<Vec<Dat>> {
	match dat {
		Dat::List(v)	=> Ok(v),
		other			=> Err(refuse(what, field, "Dat::List", &other)),
	}
}

fn want_opt(dat: Dat, what: &str, field: &str) -> Outcome<Option<Dat>> {
	match dat {
		Dat::Opt(b)	=> Ok(*b),
		other		=> Err(refuse(what, field, "Dat::Opt", &other)),
	}
}

fn opt_dat(o: Option<Dat>) -> Dat {
	Dat::Opt(Box::new(o))
}

fn maybe_to_dat<T: ToDat>(item: &Option<T>) -> Outcome<Dat> {
	match item {
		Some(t)	=> Ok(opt_dat(Some(res!(t.to_dat())))),
		None	=> Ok(opt_dat(None)),
	}
}

fn list_to_dat<T: ToDat>(items: &[T]) -> Outcome<Dat> {
	let mut v = Vec::with_capacity(items.len());
	for item in items {
		v.push(res!(item.to_dat()));
	}
	Ok(Dat::List(v))
}


// A cursor over the fields of a decoded list. `open` checks the count once, so a shape with a
// field too few or too many is refused before any field is read.
struct Fields {
	what:	&'static str,
	items:	std::vec::IntoIter<Dat>,
}

impl Fields {

	fn open(dat: Dat, what: &'static str, len: usize) -> Outcome<Self> {
		match dat {
			Dat::List(v) if v.len() == len => Ok(Self { what, items: v.into_iter() }),
			Dat::List(v) => Err(err!(
				"{} expects a {}-element Dat::List, found {} elements.",
				what, len, v.len();
			Decode, Input, Mismatch)),
			other => Err(err!(
				"{} expects a {}-element Dat::List, found {:?}.",
				what, len, other.kind();
			Decode, Input, Mismatch)),
		}
	}

	fn next(&mut self, field: &str) -> Outcome<Dat> {
		match self.items.next() {
			Some(dat)	=> Ok(dat),
			None		=> Err(err!(
				"{} has no field '{}'.", self.what, field;
			Decode, Input, Missing)),
		}
	}

	fn u8(&mut self, field: &str) -> Outcome<u8> {
		let dat = res!(self.next(field));
		want_u8(dat, self.what, field)
	}

	fn u16(&mut self, field: &str) -> Outcome<u16> {
		let dat = res!(self.next(field));
		want_u16(dat, self.what, field)
	}

	fn u64(&mut self, field: &str) -> Outcome<u64> {
		let dat = res!(self.next(field));
		want_u64(dat, self.what, field)
	}

	fn bool(&mut self, field: &str) -> Outcome<bool> {
		let dat = res!(self.next(field));
		want_bool(dat, self.what, field)
	}

	fn string(&mut self, field: &str) -> Outcome<String> {
		let dat = res!(self.next(field));
		want_str(dat, self.what, field)
	}

	fn bytes(&mut self, field: &str) -> Outcome<Vec<u8>> {
		let dat = res!(self.next(field));
		want_bytes(dat, self.what, field)
	}

	fn b32(&mut self, field: &str) -> Outcome<[u8; 32]> {
		let dat = res!(self.next(field));
		want_b32(dat, self.what, field)
	}

	fn list(&mut self, field: &str) -> Outcome<Vec<Dat>> {
		let dat = res!(self.next(field));
		want_list(dat, self.what, field)
	}

	fn opt(&mut self, field: &str) -> Outcome<Option<Dat>> {
		let dat = res!(self.next(field));
		want_opt(dat, self.what, field)
	}

	fn id(&mut self, field: &str) -> Outcome<RecordId> {
		Ok(RecordId::from_bytes(res!(self.b32(field))))
	}

	fn phase(&mut self, field: &str) -> Outcome<Phase> {
		let dat = res!(self.next(field));
		Phase::from_dat(dat)
	}

	// A list whose items are each a `T`.
	fn items<T: FromDat>(&mut self, field: &str) -> Outcome<Vec<T>> {
		let dats = res!(self.list(field));
		let mut v = Vec::with_capacity(dats.len());
		for dat in dats {
			v.push(res!(T::from_dat(dat)));
		}
		Ok(v)
	}

	// An optional `T`.
	fn maybe<T: FromDat>(&mut self, field: &str) -> Outcome<Option<T>> {
		match res!(self.opt(field)) {
			Some(dat)	=> Ok(Some(res!(T::from_dat(dat)))),
			None		=> Ok(None),
		}
	}
}


impl ToDat for RecordId {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(Dat::B32(B32(self.0)))
	}
}

impl FromDat for RecordId {
	fn from_dat(dat: Dat) -> Outcome<Self> {
		Ok(Self::from_bytes(res!(want_b32(dat, "RecordId", "id"))))
	}
}

impl ToDat for NodeId {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(Dat::B32(B32(self.0)))
	}
}

impl FromDat for NodeId {
	fn from_dat(dat: Dat) -> Outcome<Self> {
		Ok(Self::from_bytes(res!(want_b32(dat, "NodeId", "id"))))
	}
}

impl ToDat for Record {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(Dat::List(vec![
			res!(self.id.to_dat()),
			Dat::Str(self.table.clone()),
			Dat::BU64(self.value.clone()),
		]))
	}
}

impl FromDat for Record {
	fn from_dat(dat: Dat) -> Outcome<Self> {
		let mut f = res!(Fields::open(dat, "Record", 3));
		Ok(Self {
			id:		res!(f.id("id")),
			table:	res!(f.string("table")),
			value:	res!(f.bytes("value")),
		})
	}
}

impl ToDat for Phase {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(Dat::U8(match self {
			Self::Prepare	=> PHASE_PREPARE,
			Self::PreCommit	=> PHASE_PRE_COMMIT,
			Self::Commit	=> PHASE_COMMIT,
			Self::Decide	=> PHASE_DECIDE,
		}))
	}
}

impl FromDat for Phase {
	fn from_dat(dat: Dat) -> Outcome<Self> {
		match res!(want_u8(dat, "Phase", "phase")) {
			PHASE_PREPARE		=> Ok(Self::Prepare),
			PHASE_PRE_COMMIT	=> Ok(Self::PreCommit),
			PHASE_COMMIT		=> Ok(Self::Commit),
			PHASE_DECIDE		=> Ok(Self::Decide),
			other => Err(err!(
				"Phase number {} names no phase, which are {} to {}.",
				other, PHASE_PREPARE, PHASE_DECIDE;
			Decode, Input, Mismatch)),
		}
	}
}

impl ToDat for Qc {
	fn to_dat(&self) -> Outcome<Dat> {
		let mut sigs = Vec::with_capacity(self.signatures.len());
		for (voter, sig) in &self.signatures {
			sigs.push(Dat::List(vec![
				Dat::U16(*voter),
				Dat::BU64(sig.clone()),
			]));
		}
		Ok(Dat::List(vec![
			Dat::U64(self.view),
			res!(self.phase.to_dat()),
			Dat::B32(B32(self.block_hash)),
			Dat::List(sigs),
		]))
	}
}

impl FromDat for Qc {
	fn from_dat(dat: Dat) -> Outcome<Self> {
		let mut f = res!(Fields::open(dat, "Qc", 4));
		let view		= res!(f.u64("view"));
		let phase		= res!(f.phase("phase"));
		let block_hash	= res!(f.b32("block_hash"));
		let mut signatures = Vec::new();
		for dat in res!(f.list("signatures")) {
			let mut s = res!(Fields::open(dat, "Qc signature", 2));
			let voter: ReplicaId = res!(s.u16("voter"));
			signatures.push((voter, res!(s.bytes("signature"))));
		}
		Ok(Self { view, phase, block_hash, signatures })
	}
}

impl ToDat for Proposal {
	fn to_dat(&self) -> Outcome<Dat> {
		let block = match &self.block {
			Some(b)	=> Some(Dat::BU64(b.clone())),
			None	=> None,
		};
		let justify = match &self.justify {
			Some(qc)	=> Some(res!(qc.to_dat())),
			None		=> None,
		};
		Ok(Dat::List(vec![
			Dat::U64(self.view),
			res!(self.phase.to_dat()),
			Dat::B32(B32(self.block_hash)),
			opt_dat(block),
			opt_dat(justify),
		]))
	}
}

impl FromDat for Proposal {
	fn from_dat(dat: Dat) -> Outcome<Self> {
		let mut f = res!(Fields::open(dat, "Proposal", 5));
		let view		= res!(f.u64("view"));
		let phase		= res!(f.phase("phase"));
		let block_hash: BlockHash = res!(f.b32("block_hash"));
		let block = match res!(f.opt("block")) {
			Some(dat)	=> Some(res!(want_bytes(dat, "Proposal", "block"))),
			None		=> None,
		};
		let justify = res!(f.maybe::<Qc>("justify"));
		Ok(Self { view, phase, block_hash, block, justify })
	}
}

impl ToDat for Vote {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(Dat::List(vec![
			Dat::U64(self.view),
			res!(self.phase.to_dat()),
			Dat::B32(B32(self.block_hash)),
			Dat::U16(self.voter),
			Dat::BU64(self.signature.clone()),
		]))
	}
}

impl FromDat for Vote {
	fn from_dat(dat: Dat) -> Outcome<Self> {
		let mut f = res!(Fields::open(dat, "Vote", 5));
		Ok(Self {
			view:		res!(f.u64("view")),
			phase:		res!(f.phase("phase")),
			block_hash:	res!(f.b32("block_hash")),
			voter:		res!(f.u16("voter")),
			signature:	res!(f.bytes("signature")),
		})
	}
}

impl ToDat for NewView {
	fn to_dat(&self) -> Outcome<Dat> {
		let prepare_qc = match &self.prepare_qc {
			Some(qc)	=> Some(res!(qc.to_dat())),
			None		=> None,
		};
		Ok(Dat::List(vec![
			Dat::U64(self.view),
			Dat::U16(self.sender),
			opt_dat(prepare_qc),
		]))
	}
}

impl FromDat for NewView {
	fn from_dat(dat: Dat) -> Outcome<Self> {
		let mut f = res!(Fields::open(dat, "NewView", 3));
		Ok(Self {
			view:		res!(f.u64("view")),
			sender:		res!(f.u16("sender")),
			prepare_qc:	res!(f.maybe::<Qc>("prepare_qc")),
		})
	}
}

impl ToDat for MsgKind {
	fn to_dat(&self) -> Outcome<Dat> {
		let (kind, body) = match self {
			Self::ReplicatePut { record } => (
				KIND_REPLICATE_PUT,
				res!(record.to_dat()),
			),
			Self::GetRequest { request_id, table, id } => (
				KIND_GET_REQUEST,
				Dat::List(vec![
					Dat::U64(*request_id),
					Dat::Str(table.clone()),
					res!(id.to_dat()),
				]),
			),
			Self::GetResponse { request_id, record } => (
				KIND_GET_RESPONSE,
				Dat::List(vec![
					Dat::U64(*request_id),
					opt_dat(match record {
						Some(r)	=> Some(res!(r.to_dat())),
						None	=> None,
					}),
				]),
			),
			Self::AntiEntropyDigest { table, sketch, after } => (
				KIND_ANTI_ENTROPY_DIGEST,
				Dat::List(vec![
					Dat::Str(table.clone()),
					Dat::BU64(sketch.clone()),
					res!(maybe_to_dat(after)),
				]),
			),
			Self::AntiEntropyReply { table, records, requested_ids, bulk, after, next } => (
				KIND_ANTI_ENTROPY_REPLY,
				Dat::List(vec![
					Dat::Str(table.clone()),
					res!(list_to_dat(records)),
					res!(list_to_dat(requested_ids)),
					Dat::Bool(*bulk),
					res!(maybe_to_dat(after)),
					res!(maybe_to_dat(next)),
				]),
			),
			Self::AntiEntropyPush { table, records } => (
				KIND_ANTI_ENTROPY_PUSH,
				Dat::List(vec![
					Dat::Str(table.clone()),
					res!(list_to_dat(records)),
				]),
			),
			Self::CohortSubmit { record } => (
				KIND_COHORT_SUBMIT,
				res!(record.to_dat()),
			),
			Self::CohortPropose { table, id, proposal } => (
				KIND_COHORT_PROPOSE,
				Dat::List(vec![
					Dat::Str(table.clone()),
					res!(id.to_dat()),
					res!(proposal.to_dat()),
				]),
			),
			Self::CohortVote { table, id, vote } => (
				KIND_COHORT_VOTE,
				Dat::List(vec![
					Dat::Str(table.clone()),
					res!(id.to_dat()),
					res!(vote.to_dat()),
				]),
			),
			Self::CohortNewView { table, id, new_view } => (
				KIND_COHORT_NEW_VIEW,
				Dat::List(vec![
					Dat::Str(table.clone()),
					res!(id.to_dat()),
					res!(new_view.to_dat()),
				]),
			),
		};
		Ok(Dat::List(vec![Dat::U8(kind), body]))
	}
}

impl FromDat for MsgKind {
	fn from_dat(dat: Dat) -> Outcome<Self> {
		let mut f = res!(Fields::open(dat, "MsgKind", 2));
		let kind = res!(f.u8("kind"));
		let body = res!(f.next("body"));
		match kind {
			KIND_REPLICATE_PUT => Ok(Self::ReplicatePut {
				record:	res!(Record::from_dat(body)),
			}),
			KIND_GET_REQUEST => {
				let mut b = res!(Fields::open(body, "GetRequest", 3));
				Ok(Self::GetRequest {
					request_id:	res!(b.u64("request_id")),
					table:		res!(b.string("table")),
					id:			res!(b.id("id")),
				})
			},
			KIND_GET_RESPONSE => {
				let mut b = res!(Fields::open(body, "GetResponse", 2));
				Ok(Self::GetResponse {
					request_id:	res!(b.u64("request_id")),
					record:		res!(b.maybe::<Record>("record")),
				})
			},
			KIND_ANTI_ENTROPY_DIGEST => {
				let mut b = res!(Fields::open(body, "AntiEntropyDigest", 3));
				Ok(Self::AntiEntropyDigest {
					table:	res!(b.string("table")),
					sketch:	res!(b.bytes("sketch")),
					after:	res!(b.maybe::<RecordId>("after")),
				})
			},
			KIND_ANTI_ENTROPY_REPLY => {
				let mut b = res!(Fields::open(body, "AntiEntropyReply", 6));
				Ok(Self::AntiEntropyReply {
					table:			res!(b.string("table")),
					records:		res!(b.items::<Record>("records")),
					requested_ids:	res!(b.items::<RecordId>("requested_ids")),
					bulk:			res!(b.bool("bulk")),
					after:			res!(b.maybe::<RecordId>("after")),
					next:			res!(b.maybe::<RecordId>("next")),
				})
			},
			KIND_ANTI_ENTROPY_PUSH => {
				let mut b = res!(Fields::open(body, "AntiEntropyPush", 2));
				Ok(Self::AntiEntropyPush {
					table:		res!(b.string("table")),
					records:	res!(b.items::<Record>("records")),
				})
			},
			KIND_COHORT_SUBMIT => Ok(Self::CohortSubmit {
				record:	res!(Record::from_dat(body)),
			}),
			KIND_COHORT_PROPOSE => {
				let mut b = res!(Fields::open(body, "CohortPropose", 3));
				let table	= res!(b.string("table"));
				let id		= res!(b.id("id"));
				let dat		= res!(b.next("proposal"));
				Ok(Self::CohortPropose { table, id, proposal: res!(Proposal::from_dat(dat)) })
			},
			KIND_COHORT_VOTE => {
				let mut b = res!(Fields::open(body, "CohortVote", 3));
				let table	= res!(b.string("table"));
				let id		= res!(b.id("id"));
				let dat		= res!(b.next("vote"));
				Ok(Self::CohortVote { table, id, vote: res!(Vote::from_dat(dat)) })
			},
			KIND_COHORT_NEW_VIEW => {
				let mut b = res!(Fields::open(body, "CohortNewView", 3));
				let table	= res!(b.string("table"));
				let id		= res!(b.id("id"));
				let dat		= res!(b.next("new_view"));
				Ok(Self::CohortNewView { table, id, new_view: res!(NewView::from_dat(dat)) })
			},
			other => Err(err!(
				"MsgKind number {} names no message kind that wire version {} knows, \
				which are {} to {}.",
				other, WIRE_VERSION, KIND_REPLICATE_PUT, KIND_COHORT_NEW_VIEW;
			Decode, Input, Mismatch)),
		}
	}
}

impl ToDat for Envelope {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(Dat::List(vec![
			Dat::U8(WIRE_VERSION),
			res!(self.from.to_dat()),
			res!(self.to.to_dat()),
			res!(self.body.to_dat()),
		]))
	}
}

impl FromDat for Envelope {
	fn from_dat(dat: Dat) -> Outcome<Self> {
		let mut f = res!(Fields::open(dat, "Envelope", 4));
		let version = res!(f.u8("version"));
		if version != WIRE_VERSION {
			return Err(err!(
				"Envelope wire version {} is not the version {} this build reads.",
				version, WIRE_VERSION;
			Decode, Input, Mismatch));
		}
		let from	= NodeId::from_bytes(res!(f.b32("from")));
		let to		= NodeId::from_bytes(res!(f.b32("to")));
		let body	= res!(f.next("body"));
		Ok(Self { from, to, body: res!(MsgKind::from_dat(body)) })
	}
}

impl Envelope {

	/// Writes the envelope, refusing one longer than [`MAX_ENVELOPE_BYTES`], which no peer would
	/// accept.
	pub fn encode(&self) -> Outcome<Vec<u8>> {
		let dat = res!(self.to_dat());
		let bytes = res!(dat.to_bytes(Vec::new()));
		if bytes.len() > MAX_ENVELOPE_BYTES {
			return Err(err!(
				"A {} envelope of {} bytes exceeds the maximum of {} bytes.",
				self.body.label(), bytes.len(), MAX_ENVELOPE_BYTES;
			Encode, Excessive, Size));
		}
		Ok(bytes)
	}

	/// Reads an envelope from bytes a peer sent. The length and nesting are bounded before
	/// anything is decoded, and bytes after the envelope are refused.
	pub fn decode(buf: &[u8]) -> Outcome<Self> {
		let (dat, used) = match Dat::from_bytes_limited(buf, &limits()) {
			Ok(pair)	=> pair,
			Err(e)		=> return Err(err!(
				e, "An envelope of {} bytes did not decode.", buf.len();
			Decode, Input, Mismatch)),
		};
		if used != buf.len() {
			return Err(err!(
				"An envelope of {} bytes is followed by {} unread bytes.",
				used, buf.len() - used;
			Decode, Input, Mismatch));
		}
		Self::from_dat(dat)
	}
}
