//! The Oxegen Allocation Mechanism (OAM) primitive for the Hematite distributed
//! Ozone layer.
//!
//! OAM answers a single question without exchanging any routing messages:
//! *"Does peer $p$ hold record $d$?"* It answers it with a deterministic
//! threshold test on the XOR distance between the peer's identifier and the
//! record's hash.
//!
//! A peer $p$ holds record $d$ if and only if
//!
//! $ "XOR"("peer_id"_p, H(d)) < 2^256 dot n / N $
//!
//! where `n` is the replication factor and `N` is the current estimated
//! network size. Two peers with the same view of `n` and `N` will always
//! agree on which peers hold which records. The expected number of holders
//! per record is `min(n, N)`, with tight concentration when `N` is large.
//!
//! # Identifier space
//!
//! OAM reuses the 256-bit identifier space from [`oxedyne_fe2o3_kademlia`].
//! Record hashes are interpreted as [`NodeId`]s and compared to peer
//! identifiers by XOR distance. This keeps the Kademlia routing layer and the
//! OAM placement layer consistent: the same hash that identifies a record for
//! storage also identifies its neighbourhood for lookup.
//!
//! # What this crate does not do
//!
//! - Hash records. Callers bring a cryptographic hash -- SHA-3, BLAKE3, or
//!   whatever their application dictates -- and hand in the resulting 32 bytes
//!   as a [`NodeId`].
//! - Estimate the network size. OAM consumes an `N` value that the caller
//!   obtains from the HyperLogLog layer (see [`oxedyne_fe2o3_hll`]).
//! - Route, replicate, or transport data. OAM only decides *who should hold*;
//!   the distributed Ozone engine decides *how to get the record there*.
//!
//! # Example
//!
//! ```
//! use oxedyne_fe2o3_core::prelude::*;
//! use oxedyne_fe2o3_kademlia::id::NodeId;
//! use oxedyne_fe2o3_oam::{
//!     config::OamConfig,
//!     placement,
//! };
//!
//! # fn main() -> Outcome<()> {
//! // Twenty replicas on a network of five hundred peers.
//! let cfg = res!(OamConfig::new(20, 500));
//! let threshold = cfg.threshold();
//!
//! // A peer asks: do I hold the record with this hash?
//! let my_peer_id = NodeId::from_bytes([0u8; 32]);
//! let record_hash = NodeId::from_bytes([0u8; 32]);
//! assert!(placement::is_holder(&my_peer_id, &record_hash, &threshold));
//! # Ok(())
//! # }
//! ```
#![forbid(unsafe_code)]

pub mod config;
pub mod placement;
pub mod threshold;

pub use crate::{
	config::OamConfig,
	threshold::Threshold,
};
