//! Distributed Ozone -- the peer-to-peer replicated mode of Hematite's Ozone
//! key/value database.
//!
//! Every peer runs a local Ozone (the rest of this crate); this module layers
//! *replication* on top. It composes the pure primitives from the
//! primitive modules into one cohesive engine behind a small application
//! API:
//!
//! - [`crate::kademlia`] -- peer identifiers and XOR distance.
//! - [`crate::oam`] -- threshold-XOR placement.
//! - [`oxedyne_fe2o3_data::hll`] -- network-size estimation.
//! - [`oxedyne_fe2o3_data::iblt`] -- state-reconciliation sketches.
//! - [`hotstuff`] -- Byzantine-fault-tolerant consensus (kept local to this
//!   module because it is consumed nowhere else in the ecosystem today).
//!
//! # Architectural layering
//!
//! Distributed Ozone depends on transport (Shield) and storage (local Ozone)
//! through *traits*, not concrete types. The intent is that the two heavy
//! integrations -- Shield's UDP wire and this crate's local engine -- plug
//! in as adapters; a pure in-memory mock works just as well for tests and
//! two-node loopback demos. The traits are small on purpose:
//! [`transport::Transport`] sends and receives envelopes, [`storage::Storage`]
//! reads, writes and enumerates records. Neither carries policy.
//!
//! # What this module owns
//!
//! - [`config::DistOzoneConfig`] and the related table-configuration types
//!   that shape distributed mode at start-up.
//! - [`peer_set::PeerSet`] -- the rolling view of known peers. Updated by
//!   Kademlia lookups (not modelled here) and consulted on every placement
//!   decision.
//! - [`placement::Placement`] -- the service that answers "who holds this
//!   record?" by combining `OamConfig` with the current peer set.
//! - [`engine::DistOzone`] -- the top-level engine. Callers construct one
//!   per process, hand it a [`transport::Transport`] and a
//!   [`storage::Storage`], then invoke [`engine::DistOzone::put`] and
//!   [`engine::DistOzone::get`].
//!
//! # What this module does not own
//!
//! - Any transport implementation. Shield is an external adapter.
//! - Any on-disk storage. The rest of this crate is an external adapter.
//! - The reference peer application. That is downstream and consumes the
//!   [`engine::DistOzone`] API for persistence across its identity,
//!   escrow, name, revocation, peer-set, treasury and epoch tables.
//!
//! # Feature gate
//!
//! This module is compiled only when the crate is built with the `dist`
//! feature. Local-only callers pay no compile cost for the distributed
//! primitive graph.

pub mod cohort;
pub mod config;
pub mod consensus;
pub mod engine;
pub mod hotstuff;
pub mod o3db_storage;
pub mod peer_set;
pub mod placement;
pub mod record;
pub mod storage;
pub mod transport;

pub use self::{
	config::{
		Consistency,
		DistOzoneConfig,
		TableConfig,
	},
	engine::DistOzone,
	peer_set::PeerSet,
	placement::Placement,
	record::{
		Record,
		RecordId,
	},
};
