//! Configuration types for distributed Ozone.
//!
//! A [`DistOzoneConfig`] block is the caller's one-stop description of
//! distributed mode at start-up. It bundles the local peer's identity, the
//! initial bootstrap peer list, the OAM placement parameters and the
//! per-table consistency / anti-entropy cadence declarations.
//!
//! The config is *static* once a [`DistOzone`](crate::dist::DistOzone) engine
//! is constructed; runtime mutation (peers joining, leaving, network size
//! re-estimated) flows through the engine's own methods rather than through
//! the config.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use crate::kademlia::id::NodeId;
use crate::oam::config::OamConfig;

use std::time::Duration;


/// The consistency guarantee a table provides under distributed mode.
///
/// *Eventual* tables accept concurrent writes and converge through IBLT
/// anti-entropy. *Cohort-backed* tables serialise writes through a HotStuff
/// consensus cohort and reach strict consistency after three message rounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Consistency {
	Eventual,	// writes land locally and at every OAM holder
	// Tolerates up to floor((lambda - 1) / 3) Byzantine members.
	Cohort {
		lambda:	u64,	// 5, 7 or 9
	},
}


/// Per-table configuration: the name, the consistency model, the
/// anti-entropy cadence, and the IBLT sketch dimensions used for
/// anti-entropy reconciliation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableConfig {
	pub name:				String,			// unique within a DistOzoneConfig
	pub consistency:		Consistency,
	pub anti_entropy:		Duration,		// ignored for Cohort tables
	pub iblt_cells:			usize,			// 1.5 x d cells decode a difference of d
}

impl TableConfig {
	// Identity directory, oxedation log, name claims.
	pub const DEFAULT_AE: Duration = Duration::from_secs(30);

	// Small high-value tables -- the peer set, the revocation list.
	pub const HIGH_VALUE_AE: Duration = Duration::from_secs(3);

	// Tuned for a steady-state symmetric difference of up to ~160 records
	// (256 / 1.5). Larger differences overload the sketch; the anti-entropy
	// handler falls back to a bulk transfer when decoding fails.
	pub const DEFAULT_IBLT_CELLS: usize = 256;

	/// The number of hash functions the anti-entropy IBLT uses. Fixed at
	/// three across the crate, matching the sizing rule of thumb in
	/// `fe2o3_data::iblt`.
	pub const IBLT_NUM_HASHES: usize = 3;

	pub fn new<S: Into<String>>(
		name:			S,
		consistency:	Consistency,
		anti_entropy:	Duration,
		iblt_cells:		usize,
	)
		-> Outcome<Self>
	{
		let name = name.into();
		if name.is_empty() {
			return Err(err!(
				"TableConfig requires a non-empty name.";
				Invalid, Input, Missing));
		}
		if let Consistency::Cohort { lambda } = consistency {
			if !matches!(lambda, 5 | 7 | 9) {
				return Err(err!(
					"Cohort lambda must be 5, 7 or 9; got {}.", lambda;
					Invalid, Input, Size));
			}
		}
		if iblt_cells == 0 {
			return Err(err!(
				"TableConfig requires iblt_cells > 0.";
				Invalid, Input, Size));
		}
		Ok(Self { name, consistency, anti_entropy, iblt_cells })
	}

	pub fn eventual<S: Into<String>>(name: S) -> Outcome<Self> {
		Self::new(
			name,
			Consistency::Eventual,
			Self::DEFAULT_AE,
			Self::DEFAULT_IBLT_CELLS,
		)
	}

	/// Cohort size 5.
	pub fn cohort_default<S: Into<String>>(name: S) -> Outcome<Self> {
		Self::new(
			name,
			Consistency::Cohort { lambda: 5 },
			Self::DEFAULT_AE,
			Self::DEFAULT_IBLT_CELLS,
		)
	}

	/// The IBLT's splitmix64 salt, derived from the table name so that
	/// different tables have different hash functions.
	pub fn iblt_seed(&self) -> u64 {
		let mut state: u64 = 0x9E3779B97F4A7C15;
		for byte in self.name.as_bytes() {
			state = state.wrapping_add(*byte as u64);
			state = (state ^ (state >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
			state = (state ^ (state >> 27)).wrapping_mul(0x94D049BB133111EB);
			state ^= state >> 31;
		}
		state
	}
}


/// Top-level configuration for distributed Ozone mode.
#[derive(Clone, Debug)]
pub struct DistOzoneConfig {
	pub local_peer_id:		NodeId,
	pub bootstrap_peers:	Vec<NodeId>,		// the local peer is filtered out
	pub oam:				OamConfig,			// network_size is the initial value only
	pub tables:				Vec<TableConfig>,	// unique names
}

impl DistOzoneConfig {
	pub fn new(
		local_peer_id:		NodeId,
		bootstrap_peers:	Vec<NodeId>,
		oam:				OamConfig,
		tables:				Vec<TableConfig>,
	)
		-> Outcome<Self>
	{
		if tables.is_empty() {
			return Err(err!(
				"DistOzoneConfig requires at least one table.";
				Invalid, Input, Missing));
		}
		// Detect duplicate table names via a pairwise scan. Table counts are
		// small (single-digits to low tens) so quadratic is fine and avoids
		// pulling HashSet into a config type.
		for i in 0..tables.len() {
			for j in (i + 1)..tables.len() {
				if tables[i].name == tables[j].name {
					return Err(err!(
						"Duplicate table name in DistOzoneConfig: {}.",
						tables[i].name;
					Invalid, Input, Duplicate));
				}
			}
		}
		Ok(Self { local_peer_id, bootstrap_peers, oam, tables })
	}

	pub fn table(&self, name: &str) -> Option<&TableConfig> {
		self.tables.iter().find(|t| t.name == name)
	}
}
