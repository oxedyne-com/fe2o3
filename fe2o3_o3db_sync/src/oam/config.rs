//! The OAM configuration block.
//!
//! An [`OamConfig`] is the inputs of the placement inequality: the replication
//! factor `n` and the current estimated network size `N`. It is held by every
//! peer and refreshed when either input changes -- `n` on a configuration
//! reload, `N` on a HyperLogLog-driven estimate update.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use super::threshold::Threshold;

use oxedyne_fe2o3_core::prelude::*;


/// The OAM configuration held by every peer.
///
/// # Invariants
///
/// - `replication` is the target number of holders per record.
/// - `network_size` is the current estimated peer count, `N`.
///
/// Both are allowed to be zero; the resulting threshold will saturate to
/// [`Threshold::None`] or [`Threshold::All`] accordingly. [`OamConfig::new`]
/// rejects the genuinely nonsensical combination `replication > 0 &&
/// network_size == 0` so the caller does not accidentally declare "twenty
/// replicas on nothing" as if it were a routine state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OamConfig {
	pub replication:	u64,	// n in the specification
	pub network_size:	u64,	// N in the specification
}

impl OamConfig {
	// The figure quoted in the Hematite specification.
	pub const DEFAULT_REPLICATION: u64 = 20;

	/// The spec allows `replication == 0` -- no peer holds anything -- as a
	/// well-defined limit. `network_size == 0` with a non-zero replication is
	/// an operator mistake rather than a valid operating point, and is
	/// rejected.
	pub fn new(replication: u64, network_size: u64) -> Outcome<Self> {
		if replication > 0 && network_size == 0 {
			return Err(err!(
				"OAM configuration requires network_size > 0 when \
				replication > 0, got replication={} network_size=0.",
				replication;
			Invalid, Input, Size));
		}
		Ok(Self {
			replication,
			network_size,
		})
	}

	pub fn default_replication(network_size: u64) -> Outcome<Self> {
		Self::new(Self::DEFAULT_REPLICATION, network_size)
	}

	/// A pure function of `(replication, network_size)`, and worth caching when
	/// a peer checks placement against many records.
	pub fn threshold(&self) -> Threshold {
		Threshold::from_params(self.replication, self.network_size)
	}

	/// Per record, clamped to `min(replication, network_size)`.
	pub fn expected_holders(&self) -> u64 {
		self.replication.min(self.network_size)
	}
}
