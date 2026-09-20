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

use crate::collab::DocId;

use oxedyne_fe2o3_ore::{
	envelope::Envelope,
	id::OpId,
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

/// The thin store the collaboration layer reaches a document's log through.
///
/// It is deliberately just two operations. A store that can do more -- a networked hub, a local cache
/// -- offers it through this same pair, so nothing above has to know which it holds.
pub trait Hub {
	/// Appends a signed operation to a document's log. Appending an operation the log already holds is
	/// a no-op, so a redelivery does not duplicate it.
	fn put(&self, doc: &DocId, op: OpId, env: &Envelope) -> Outcome<()>;

	/// Loads a document's whole log: every operation stored under its identity, each with the envelope
	/// that attests to it. The order is not significant -- the fold imposes its own -- and a document
	/// with no operations yet is an empty log, not an error.
	fn scan(&self, doc: &DocId) -> Outcome<Vec<(OpId, Envelope)>>;
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
	fn put(&self, doc: &DocId, op: OpId, env: &Envelope) -> Outcome<()> {
		let mut entries = res!(self.load(doc));
		if entries.iter().any(|(id, _)| *id == op) {
			return Ok(());
		}
		entries.push((op, env.clone()));
		let list = Dat::List(entries.iter()
			.map(|(id, e)| listdat![id.to_dat(), e.to_dat()])
			.collect());
		res!(self.db.insert(Self::key(doc), list, self.user, None));
		Ok(())
	}

	fn scan(&self, doc: &DocId) -> Outcome<Vec<(OpId, Envelope)>> {
		self.load(doc)
	}
}
