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

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_kademlia::id::NodeId;
use oxedyne_fe2o3_oam::config::OamConfig;

use std::time::Duration;


/// The consistency guarantee a table provides under distributed mode.
///
/// *Eventual* tables accept concurrent writes and converge through IBLT
/// anti-entropy. *Cohort-backed* tables serialise writes through a HotStuff
/// consensus cohort and reach strict consistency after three message rounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Consistency {
	/// Writes land locally and at every OAM holder; concurrent writes
	/// reconcile via IBLT anti-entropy. Suitable for append-only or
	/// monotonic-only tables.
	Eventual,
	/// Writes serialise through a consensus cohort of the given size `lambda`,
	/// tolerating up to `floor((lambda - 1) / 3)` Byzantine members. Values
	/// are restricted to `{5, 7, 9}` in the Hematite spec; other values are
	/// rejected by [`TableConfig::new`].
	Cohort {
		/// Cohort size. Must be in `{5, 7, 9}`.
		lambda:	u64,
	},
}


/// Per-table configuration: the name, the consistency model, the
/// anti-entropy cadence, and the IBLT sketch dimensions used for
/// anti-entropy reconciliation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableConfig {
	/// The application-visible table name. Must be unique within a
	/// [`DistOzoneConfig`].
	pub name:				String,
	/// The consistency guarantee.
	pub consistency:		Consistency,
	/// How often a peer initiates an IBLT anti-entropy round for this table.
	/// Ignored for [`Consistency::Cohort`] tables, which reconcile through
	/// consensus rather than anti-entropy.
	pub anti_entropy:		Duration,
	/// Number of IBLT cells used in anti-entropy digests for this table.
	/// Spec rule of thumb: `1.5 × d` cells with three hash functions, where
	/// `d` is the maximum symmetric-difference size the sketch must decode.
	/// Oversized sketches waste bandwidth; undersized sketches force a
	/// bulk-transfer fallback.
	pub iblt_cells:			usize,
}

impl TableConfig {
	/// Default anti-entropy cadence for a non-trivial eventual-consistency
	/// table -- identity directory, oxedation log, name claims.
	pub const DEFAULT_AE: Duration = Duration::from_secs(30);

	/// Cadence for small high-value tables -- the peer set, the revocation
	/// list.
	pub const HIGH_VALUE_AE: Duration = Duration::from_secs(3);

	/// Default IBLT cell count. Tuned for a steady-state symmetric
	/// difference of up to ~160 records (`256 / 1.5`). Larger differences
	/// overload the sketch; the anti-entropy handler falls back to a bulk
	/// transfer when decoding fails.
	pub const DEFAULT_IBLT_CELLS: usize = 256;

	/// The number of hash functions the anti-entropy IBLT uses. Fixed at
	/// three across the crate, matching the sizing rule of thumb in
	/// `fe2o3_iblt`.
	pub const IBLT_NUM_HASHES: usize = 3;

	/// Constructs a table config, validating the consistency model and the
	/// sketch dimensions.
	///
	/// Rejects:
	/// - an empty `name`;
	/// - a [`Consistency::Cohort`] `lambda` outside `{5, 7, 9}`;
	/// - `iblt_cells == 0`.
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

	/// Convenience: an eventual-consistency table at the default cadence
	/// and the default IBLT cell count.
	pub fn eventual<S: Into<String>>(name: S) -> Outcome<Self> {
		Self::new(
			name,
			Consistency::Eventual,
			Self::DEFAULT_AE,
			Self::DEFAULT_IBLT_CELLS,
		)
	}

	/// Convenience: a cohort-backed table with lambda = 5.
	pub fn cohort_default<S: Into<String>>(name: S) -> Outcome<Self> {
		Self::new(
			name,
			Consistency::Cohort { lambda: 5 },
			Self::DEFAULT_AE,
			Self::DEFAULT_IBLT_CELLS,
		)
	}

	/// A deterministic 64-bit seed derived from the table name, used as
	/// the IBLT's splitmix64 salt so that different tables have different
	/// hash functions.
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
	/// The local peer's 256-bit identifier.
	pub local_peer_id:	NodeId,
	/// The initial peer set. Does not need to include the local peer -- it
	/// is filtered out automatically by
	/// [`DistOzone::new`](crate::dist::DistOzone::new).
	pub bootstrap_peers:	Vec<NodeId>,
	/// OAM placement parameters. `network_size` is the *initial* value;
	/// the engine's HyperLogLog estimator updates it at runtime.
	pub oam:				OamConfig,
	/// The table schemas. Must have unique names.
	pub tables:				Vec<TableConfig>,
}

impl DistOzoneConfig {
	/// Constructs and validates a config.
	///
	/// Rejects:
	/// - duplicate table names;
	/// - an empty table list (distributed mode requires at least one table).
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

	/// Returns the table config for the given name, if present.
	pub fn table(&self, name: &str) -> Option<&TableConfig> {
		self.tables.iter().find(|t| t.name == name)
	}
}
