//! Distributed Ozone -- the peer-to-peer replicated mode of the Hematite Ozone
//! key/value database.
//!
//! Every peer runs a local Ozone; this crate layers *replication* on top. It
//! composes the pure primitives from neighbouring crates into one cohesive
//! engine behind a small application API:
//!
//! - [`oxedyne_fe2o3_kademlia`] -- peer identifiers and XOR distance.
//! - [`oxedyne_fe2o3_oam`] -- threshold-XOR placement.
//! - [`oxedyne_fe2o3_hll`] -- network-size estimation.
//! - [`oxedyne_fe2o3_iblt`] -- state-reconciliation sketches.
//!
//! # Architectural layering
//!
//! Distributed Ozone depends on transport (Shield) and storage (local Ozone)
//! through *traits*, not concrete types. The intent is that the two heavy
//! integrations -- Shield's UDP wire and `fe2o3_o3db_sync`'s local engine --
//! plug in as adapters; a pure in-memory mock works just as well for tests
//! and two-node loopback demos. The traits are small on purpose:
//! [`transport::Transport`] sends and receives envelopes, [`storage::Storage`]
//! reads, writes and enumerates records. Neither carries policy.
//!
//! # What this crate owns
//!
//! - [`config::DistOzoneConfig`] and the related table-configuration types
//!   that shape distributed mode at start-up.
//! - [`peer_set::PeerSet`] -- the rolling view of known peers. Updated by
//!   Kademlia lookups (not modelled here) and consulted on every placement
//!   decision.
//! - [`placement::Placement`] -- the service that answers "who holds this
//!   record?" by combining `OamConfig` with the current peer set.
//! - [`dist::DistOzone`] -- the top-level engine. Callers construct one per
//!   process, hand it a [`transport::Transport`] and a [`storage::Storage`],
//!   then invoke [`dist::DistOzone::put`] and [`dist::DistOzone::get`].
//!
//! # What this crate does not own
//!
//! - Any transport implementation. Shield is an external adapter.
//! - Any on-disk storage. `fe2o3_o3db_sync` is an external adapter.
//! - The Oxegen peer. Oxegen is an *application* built on distributed Ozone;
//!   it declares its table schemas (identity, escrow, oxedation, names,
//!   revocation, peer_set, treasury, epoch) and consumes the [`dist::DistOzone`]
//!   API for persistence.
#![forbid(unsafe_code)]

pub mod cohort;
pub mod config;
pub mod dist;
pub mod peer_set;
pub mod placement;
pub mod record;
pub mod storage;
pub mod transport;

pub use crate::{
	config::{
		Consistency,
		DistOzoneConfig,
		TableConfig,
	},
	dist::DistOzone,
	peer_set::PeerSet,
	placement::Placement,
	record::{
		Record,
		RecordId,
	},
};
