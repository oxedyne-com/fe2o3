//! Record addressing for distributed Ozone.
//!
//! A distributed-mode record is identified by the pair `(table, key)`. The
//! caller hashes this pair (or just the key, if the table is partitioned by
//! name elsewhere) into a 256-bit [`RecordId`] for OAM placement. This crate
//! does not prescribe the hash function -- the caller hands in a [`RecordId`]
//! that has already been computed, in the same way the underlying primitive
//! crates take pre-computed [`NodeId`]s.
//!
//! [`NodeId`]: oxedyne_fe2o3_kademlia::id::NodeId

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_kademlia::id::{
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
	/// Constructs a record identifier from a raw 32-byte array.
	pub const fn from_bytes(bytes: [u8; ID_LEN]) -> Self {
		Self(bytes)
	}

	/// Constructs a record identifier from a byte slice.
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

	/// Reinterprets the record identifier as a [`NodeId`] for XOR-distance
	/// comparisons.
	pub fn as_node_id(&self) -> NodeId {
		NodeId::from_bytes(self.0)
	}

	/// Returns the identifier as a byte slice.
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
	/// The 256-bit identifier used for OAM placement.
	pub id:		RecordId,
	/// The table the record belongs to. Matched against a [`TableConfig`].
	pub table:	String,
	/// The application-opaque value bytes.
	pub value:	Vec<u8>,
}

impl Record {
	/// Constructs a record from its parts.
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
	/// The 256-bit identifier used for OAM placement.
	pub id:			RecordId,
	/// A content hash of the value bytes, used to detect divergent copies at
	/// the same id. Caller-supplied so distributed Ozone is not tied to a
	/// particular hash function.
	pub content:	[u8; 32],
}
