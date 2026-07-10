//! Peer contact records stored in the routing table.
//!
//! A [`Contact`] is the minimum information needed to route to a peer: its
//! identifier, one or more network addresses, a liveness timestamp, a smoothed
//! round-trip time, and an opaque capability bitfield. Public keys and key
//! rotation live one layer up -- in fe2o3_crypto and the distributed Ozone
//! layer on top of this crate -- because routing itself does not authenticate
//! anything.

use super::id::NodeId;

use std::{
	net::SocketAddr,
	time::Duration,
};


/// Opaque peer capability flags.
///
/// Concrete bit assignments are made by the layer above (distributed Ozone,
/// Oxegen peer type). This crate treats capabilities as an opaque bitfield
/// with set, test and clear operations.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Capabilities(pub u64);

impl Capabilities {
	/// The empty capability set.
	pub const NONE: Self = Self(0);

	/// Returns `true` if every bit of `mask` is set in `self`.
	pub fn has(&self, mask: Capabilities) -> bool {
		self.0 & mask.0 == mask.0
	}

	/// Sets every bit of `mask` in `self`.
	pub fn set(&mut self, mask: Capabilities) {
		self.0 |= mask.0;
	}

	/// Clears every bit of `mask` from `self`.
	pub fn clear(&mut self, mask: Capabilities) {
		self.0 &= !mask.0;
	}
}


/// A routing-table entry for a single peer.
///
/// Cheap to clone -- `addresses` is a short `Vec` and the other fields are
/// `Copy`. The routing table keeps `Contact`s directly rather than behind an
/// `Arc` because they are small and mutated only through the table's own API.
#[derive(Clone, Debug)]
pub struct Contact {
	/// The peer's 256-bit identifier.
	pub node_id:		NodeId,
	/// One or more socket addresses the peer is known to accept traffic on.
	/// The first entry is treated as the primary by higher layers.
	pub addresses:		Vec<SocketAddr>,
	/// Ticks since some epoch the caller chose, supplied at insertion or
	/// touch time. Opaque to this crate.
	pub last_seen:		u64,
	/// Smoothed round-trip time, or `None` if no probe has succeeded yet.
	pub rtt:			Option<Duration>,
	/// Opaque capability bitfield.
	pub capabilities:	Capabilities,
}

impl Contact {
	/// Builds a new contact with `last_seen = 0`, no RTT sample and no
	/// capabilities. Callers are expected to update `last_seen` immediately
	/// on insertion via [`Contact::touch`] or the routing table's insert
	/// path.
	pub fn new(node_id: NodeId, addresses: Vec<SocketAddr>) -> Self {
		Self {
			node_id,
			addresses,
			last_seen:		0,
			rtt:			None,
			capabilities:	Capabilities::NONE,
		}
	}

	/// Updates `last_seen` to `now`.
	pub fn touch(&mut self, now: u64) {
		self.last_seen = now;
	}

	/// Blends a fresh RTT sample into the smoothed estimate.
	///
	/// Uses a simple exponentially weighted moving average with α = 1/8, the
	/// TCP Jacobson smoothing constant. The first sample becomes the initial
	/// estimate unchanged.
	pub fn record_rtt(&mut self, sample: Duration) {
		self.rtt = Some(match self.rtt {
			None => sample,
			Some(prev) => {
				let prev_ns = prev.as_nanos() as u64;
				let samp_ns = sample.as_nanos() as u64;
				let next_ns = prev_ns - (prev_ns / 8) + (samp_ns / 8);
				Duration::from_nanos(next_ns)
			},
		});
	}
}
