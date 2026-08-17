//! Record addressing for distributed Ozone.
//!
//! A distributed-mode record is identified by the pair `(table, key)`. The
//! caller hashes this pair (or just the key, if the table is partitioned by
//! name elsewhere) into a 256-bit [`RecordId`] for OAM placement. This crate
//! does not prescribe the hash function -- the caller hands in a [`RecordId`]
//! that has already been computed, in the same way the underlying primitive
//! crates take pre-computed [`NodeId`]s.
//!
//! [`NodeId`]: crate::kademlia::id::NodeId
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use crate::kademlia::id::{
	ID_LEN,
	NodeId,
};


/// A 256-bit record identifier, shared with the Kademlia / OAM identifier
/// space. This is the caller's pre-computed hash of the record's canonical
/// form -- typically `(table_name, key)` serialised and run through a
/// cryptographic hash such as SHA-3 or BLAKE3.
///
/// A [`RecordId`] is reinterpreted as a [`NodeId`] for placement decisions so
/// that XOR distance against a peer identifier is well-defined.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RecordId(pub [u8; ID_LEN]);

impl RecordId {
	pub const fn from_bytes(bytes: [u8; ID_LEN]) -> Self {
		Self(bytes)
	}

	/// The slice must be exactly [`ID_LEN`] bytes.
	pub fn from_slice(bytes: &[u8]) -> Outcome<Self> {
		if bytes.len() != ID_LEN {
			return Err(err!(
				"RecordId requires exactly {} bytes, got {}.",
				ID_LEN, bytes.len();
			Invalid, Input, Size));
		}
		let mut arr = [0u8; ID_LEN];
		arr.copy_from_slice(bytes);
		Ok(Self(arr))
	}

	pub fn as_node_id(&self) -> NodeId {
		NodeId::from_bytes(self.0)
	}

	pub fn as_bytes(&self) -> &[u8; ID_LEN] {
		&self.0
	}
}

impl From<NodeId> for RecordId {
	fn from(n: NodeId) -> Self {
		Self(*n.as_bytes())
	}
}

impl From<RecordId> for NodeId {
	fn from(r: RecordId) -> Self {
		r.as_node_id()
	}
}


/// A distributed-mode record: the identifier, the application-opaque value,
/// and the table it belongs to. Values are held as byte vectors -- the
/// application is responsible for serialisation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Record {
	pub id:		RecordId,
	pub table:	String,		// matched against a TableConfig
	pub value:	Vec<u8>,
}

impl Record {
	pub fn new<S: Into<String>>(
		id:		RecordId,
		table:	S,
		value:	Vec<u8>,
	)
		-> Self
	{
		Self { id, table: table.into(), value }
	}
}

/// A summary of a [`Record`] suitable for IBLT anti-entropy sketches and
/// replication decisions that do not need to carry the full value payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordDigest {
	pub id:			RecordId,
	// Caller-supplied, so distributed Ozone is tied to no particular hash.
	pub content:	[u8; 32],	// detects divergent copies at the same id
}
