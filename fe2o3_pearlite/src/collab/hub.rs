//! The store a document's operations live in, behind a thin interface.
//!
//! A document's edit stream is a set of signed envelopes keyed by the document's [`DocId`]. The
//! [`Hub`] trait is the whole of what the fold and sign layers ask of a store: append one operation,
//! and load the document's whole log back. Keeping it this small is deliberate -- the o3db backing
//! here can be swapped for a networked one, or a browser's local one, without the layers above
//! noticing.
//!
//! # The o3db representation
//!
//! o3db is a keyed value store, so a document's log is held under one key -- `pearlite-collab:doc:<id>`
//! -- as a list of `[op_id, envelope]` pairs. An append reads that list, adds the pair if the
//! operation is not already there, and writes it back; a scan reads and decodes it. Reading the list
//! whole to append one entry is the simple, correct thing for a single writer, which is what this
//! increment has: the concurrent-append path, where two writers must not lose each other's operation,
//! belongs with the sync transport that carries operations between them, and is that increment's to
//! build. The interface does not change when it arrives -- only this one implementation behind it.

use crate::collab::{
	op,
	sign,
	DocId,
	MAX_OP_BYTES,
	MAX_OPS_PER_DOC,
	MAX_TIME_SKEW_SECS,
};

use oxedyne_fe2o3_ore::{
	envelope::Envelope,
	id::OpId,
	op::{
		Op,
		Record,
	},
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
	prelude::*,
	id::NumIdDat,
};
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_o3db_sync::prelude::{
	Encrypter,
	Hasher,
};

use std::time::{
	SystemTime,
	UNIX_EPOCH,
};

/// The thin store the collaboration layer reaches a document's log through.
///
/// It is deliberately just two operations. A store that can do more -- a networked hub, a local cache
/// -- offers it through this same pair, so nothing above has to know which it holds.
pub trait Hub {
	/// Verifies a signed operation and, if it holds, appends it to a document's log, returning the
	/// identifier it was stored under -- which is taken from the verified record, never from the
	/// caller, so a peer cannot store an envelope under an identity it did not sign for. Appending an
	/// operation the log already holds is a no-op that still returns its identifier, so a redelivery
	/// does not duplicate it. An envelope that does not verify, is oversized, carries an operation of
	/// the wrong voice, an annotation stamped for another document, a time far in the future, or that
	/// would take the log past its size bound, is refused rather than stored.
	fn put(&self, doc: &DocId, env: &Envelope) -> Outcome<OpId>;

	/// Loads a document's whole log: every operation stored under its identity, each with the envelope
	/// that attests to it. The order is not significant -- the fold imposes its own -- and a document
	/// with no operations yet is an empty log, not an error.
	fn scan(&self, doc: &DocId) -> Outcome<Vec<(OpId, Envelope)>>;
}

/// The receiving replica's wall clock in unix epoch seconds, for bounding an author-stated time.
fn now_secs() -> u64 {
	match SystemTime::now().duration_since(UNIX_EPOCH) {
		Ok(d)	=> d.as_secs(),
		Err(_)	=> 0,	// a clock before the epoch bounds nothing, which is safe: it only tightens.
	}
}

/// The author-stated time an operation carries, if it carries one, for the skew bound.
fn op_time(op: &Op) -> Option<u64> {
	match op {
		Op::Proposal { time, .. }	=> Some(*time),
		Op::Said { time, .. }		=> Some(*time),
		Op::Amended { time, .. }	=> Some(*time),
		Op::Settled { time, .. }	=> Some(*time),
		_							=> None,
	}
}

/// Checks a verified record is one this log may hold: a Pearlite annotation operation, of the right
/// voice, whose body -- where it carries one -- decodes as an annotation stamped for this document, and
/// whose time is not implausibly far ahead of the receiving clock.
///
/// This is ingest validation: the fold is poison-proof against a bad operation that is already stored,
/// and this is what keeps one from being stored in the first place. The two are deliberately
/// belt-and-braces, since an append-only log cannot take back what it once accepted.
fn validate_ingest(doc: &DocId, rec: &Record) -> Outcome<()> {
	// The time bound applies to every operation that states one.
	if let Some(time) = op_time(&rec.op) {
		let ceiling = now_secs().saturating_add(MAX_TIME_SKEW_SECS);
		if time > ceiling {
			return Err(err!(
				"The operation {} states a time of {}, more than {} seconds ahead of the receiving \
				clock; a time far in the future is a bid to win last-writer-wins ordering and is \
				refused.", rec.id(), time, MAX_TIME_SKEW_SECS;
				Invalid, Input, Security, Excessive));
		}
	}
	match &rec.op {
		Op::Proposal { voice, body, .. } | Op::Amended { voice, body, .. } => {
			res!(check_voice(rec, voice));
			// The body must decode as an annotation, bounded, and be stamped for this document, or the
			// operation does not belong in this log.
			let ann = res!(op::annotation_from_body(body));
			match &ann.doc_id {
				Some(id) if id == doc.as_str()	=> {},
				Some(id)						=> return Err(err!(
					"The operation {} carries an annotation stamped for document {:?}, not {:?}; it \
					will not be stored here.", rec.id(), id, doc.as_str();
					Invalid, Input, Security, Mismatch)),
				None							=> return Err(err!(
					"The operation {} carries an annotation with no document identity, so it cannot be \
					confirmed to belong to {:?}.", rec.id(), doc.as_str();
					Invalid, Input, Missing)),
			}
			Ok(())
		},
		Op::Said { voice, .. }	=> check_voice(rec, voice),
		// A settlement carries no voice or body of its own; its authority is checked at the fold,
		// against the proposal it settles. Every other operation kind has no business in an annotation
		// log at all.
		Op::Settled { .. }		=> Ok(()),
		other => Err(err!(
			"The operation {} is a {}, which is not an annotation operation and does not belong in a \
			Pearlite collaboration log.", rec.id(), other.name();
			Invalid, Input, Mismatch)),
	}
}

/// Refuses an operation written under any voice but Pearlite's, so the log holds annotation threads and
/// not some other forge's proposals that happen to share the vocabulary.
fn check_voice(rec: &Record, voice: &str) -> Outcome<()> {
	if voice != op::VOICE {
		return Err(err!(
			"The operation {} is written under the voice {:?}, not {:?}; only Pearlite annotation \
			operations belong in this log.", rec.id(), voice, op::VOICE;
			Invalid, Input, Mismatch));
	}
	Ok(())
}

/// A [`Hub`] backed by an o3db instance, reached through the blocking
/// [`Database`](oxedyne_fe2o3_iop_db::api::Database) trait so the store's own generic machinery stays
/// out of here.
pub struct O3dbHub<'a, const UIDL: usize, UID, ENC, KH, D>
where
	UID:	NumIdDat<UIDL> + Copy,
	ENC:	Encrypter,
	KH:		Hasher,
	D:		Database<UIDL, UID, ENC, KH>,
{
	db:		&'a D,
	user:	UID,
	_p:		std::marker::PhantomData<(ENC, KH)>,
}

impl<'a, const UIDL: usize, UID, ENC, KH, D> O3dbHub<'a, UIDL, UID, ENC, KH, D>
where
	UID:	NumIdDat<UIDL> + Copy,
	ENC:	Encrypter,
	KH:		Hasher,
	D:		Database<UIDL, UID, ENC, KH>,
{
	pub fn new(db: &'a D, user: UID) -> Self {
		Self { db, user, _p: std::marker::PhantomData }
	}

	/// The key a document's log is stored under.
	fn key(doc: &DocId) -> Dat {
		dat!(fmt!("pearlite-collab:doc:{}", doc.as_str()))
	}

	/// Reads and decodes a document's log, or an empty one where nothing is stored yet.
	fn load(&self, doc: &DocId) -> Outcome<Vec<(OpId, Envelope)>> {
		let stored = res!(self.db.get(&Self::key(doc), None));
		let entries = match stored {
			None => return Ok(Vec::new()),
			Some((Dat::List(entries), _)) => entries,
			Some((other, _)) => return Err(err!(
				"A Pearlite hub log for {} is stored as a list of entries, found {:?}.",
				doc, other; Input, Invalid, Mismatch)),
		};
		let mut out = Vec::with_capacity(entries.len());
		for entry in entries {
			let pair = try_extract_dat!(entry, List);
			if pair.len() != 2 {
				return Err(err!(
					"A Pearlite hub log entry is an [op_id, envelope] pair, found {} elements.",
					pair.len(); Input, Invalid, Mismatch));
			}
			let op_id	= res!(OpId::from_dat(&pair[0]));
			let env		= res!(Envelope::from_dat(&pair[1]));
			out.push((op_id, env));
		}
		Ok(out)
	}
}

impl<'a, const UIDL: usize, UID, ENC, KH, D> Hub for O3dbHub<'a, UIDL, UID, ENC, KH, D>
where
	UID:	NumIdDat<UIDL> + Copy,
	ENC:	Encrypter,
	KH:		Hasher,
	D:		Database<UIDL, UID, ENC, KH>,
{
	fn put(&self, doc: &DocId, env: &Envelope) -> Outcome<OpId> {
		// A peer does not get to write an unbounded blob into a log: the size of the sealed envelope is
		// bounded before anything is decoded from it.
		if env.payload().len() > MAX_OP_BYTES {
			return Err(err!(
				"A Pearlite operation of {} payload bytes exceeds the {}-byte maximum a single \
				operation may occupy.", env.payload().len(), MAX_OP_BYTES;
				Invalid, Input, Excessive, Size));
		}
		// Verify the signature, decode the record under bounds, and check the header's replica is the
		// one the signer's key derives -- so the identity the operation is stored under is one the
		// signer actually signed for, taken from the record and never from the caller.
		let opened	= res!(sign::open(env));
		let id		= opened.record.id();
		res!(validate_ingest(doc, &opened.record));

		let mut entries = res!(self.load(doc));
		if entries.iter().any(|(stored, _)| *stored == id) {
			return Ok(id);
		}
		// The count is bounded so a peer cannot exhaust the store by appending without end.
		if entries.len() >= MAX_OPS_PER_DOC {
			return Err(err!(
				"The document {} already holds {} operations, the maximum a single log may hold; a \
				further operation is refused.", doc, entries.len();
				Invalid, Input, Excessive, Size));
		}
		entries.push((id, env.clone()));
		let list = Dat::List(entries.iter()
			.map(|(stored, e)| listdat![stored.to_dat(), e.to_dat()])
			.collect());
		res!(self.db.insert(Self::key(doc), list, self.user, None));
		Ok(id)
	}

	fn scan(&self, doc: &DocId) -> Outcome<Vec<(OpId, Envelope)>> {
		self.load(doc)
	}
}
