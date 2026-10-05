//! The resolver seam: the application's rule for which value a record holds.
//!
//! Every record that reaches an eventual table -- a local put, a
//! `ReplicatePut`, an anti-entropy reply or push -- is offered to a
//! [`Resolver`] together with the value already held, and the engine stores
//! only what the resolver says to. For peers to converge, `resolve` must be a
//! function of `(held, incoming)` and the records its [`ReadView`] returns,
//! must not depend on arrival order or on who sent the record, and must answer
//! [`Verdict::Defer`], never [`Verdict::Refuse`], where only time or a missing
//! record can cure the problem. [`LastVersionWins`] is the default.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use super::{
	record::RecordId,
	storage::Storage,
};

use oxedyne_fe2o3_core::prelude::*;


#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Verdict {
	Take(Vec<u8>),		// store these bytes: the incoming value, or a merge of held and incoming
	Keep,				// the held value stands
	Defer,				// valid once something else arrives; not stored, offered again by anti-entropy
	Refuse(String),		// invalid; not stored, the held value is untouched; the reason is for the log
}

/// Built with [`ResolveCtx::at`], so that fields can be added later.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ResolveCtx {
	pub now_ms:	u64,	// the caller's clock, in milliseconds since the Unix epoch
}

impl ResolveCtx {
	pub fn at(now_ms: u64) -> Self {
		Self { now_ms }
	}
}

/// Point reads only, so a resolver cannot write.
pub trait ReadView {
	fn get(&self, table: &str, id: &RecordId) -> Outcome<Option<Vec<u8>>>;
}

/// What the engine passes to a resolver. It is a wrapper, not a blanket
/// `impl ReadView for S: Storage`, which would make `get` ambiguous for a
/// caller with both traits in scope.
pub struct StorageView<'a, S: Storage>(pub &'a S);

impl<'a, S: Storage> ReadView for StorageView<'a, S> {
	fn get(&self, table: &str, id: &RecordId) -> Outcome<Option<Vec<u8>>> {
		Ok(res!(self.0.get(table, id)).map(|r| r.value))
	}
}

pub trait Resolver {
	/// `held` is `None` on a first arrival, which is validated too. The engine
	/// calls this under its write lock, for one record, only on a peer that
	/// holds the record. An `Err` stores nothing and is reported in
	/// `InboundOutcome::failed`, or returned from a local put.
	fn resolve<V: ReadView>(
		&self,
		ctx:		&ResolveCtx,
		view:		&V,
		table:		&str,
		id:			&RecordId,
		held:		Option<&[u8]>,
		incoming:	&[u8],
	)
		-> Outcome<Verdict>;
}

/// The default resolver: the greater byte string wins. It is a semilattice
/// join, so any arrival order converges, and equal values are a [`Verdict::Keep`].
#[derive(Clone, Copy, Debug, Default)]
pub struct LastVersionWins;

impl LastVersionWins {
	/// A big-endian `u64` version, then the payload. Byte order then sorts by
	/// version first and payload second.
	pub fn value(version: u64, payload: &[u8]) -> Vec<u8> {
		let mut v = Vec::with_capacity(8 + payload.len());
		v.extend_from_slice(&version.to_be_bytes());
		v.extend_from_slice(payload);
		v
	}

	/// `None` when the value is shorter than the eight version bytes.
	pub fn version(value: &[u8]) -> Option<u64> {
		value.first_chunk::<8>().map(|b| u64::from_be_bytes(*b))
	}
}

impl Resolver for LastVersionWins {
	fn resolve<V: ReadView>(
		&self,
		_ctx:		&ResolveCtx,
		_view:		&V,
		_table:		&str,
		_id:		&RecordId,
		held:		Option<&[u8]>,
		incoming:	&[u8],
	)
		-> Outcome<Verdict>
	{
		match held {
			Some(h) if incoming <= h	=> Ok(Verdict::Keep),
			_							=> Ok(Verdict::Take(incoming.to_vec())),
		}
	}
}
