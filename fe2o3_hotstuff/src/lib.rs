//! A HotStuff Byzantine-fault-tolerant consensus primitive for small
//! cohorts, intended for the strong-consistency tables of the Hematite
//! distributed Ozone layer.
//!
//! # Scope of this crate
//!
//! This is the *happy-path three-phase skeleton* of Basic HotStuff: a fixed
//! leader drives a single view through `Prepare -> PreCommit -> Commit ->
//! Decide`. The primitive is a pure, deterministic state machine; it has no
//! transport, no crypto, and no time. Signatures are opaque byte vectors
//! supplied by the caller and aggregated without inspection, exactly as in
//! the other fe2o3 primitives.
//!
//! # Deliberately deferred
//!
//! - **View change and leader rotation.** The current implementation panics
//!   in no sense but also cannot recover from a silent or Byzantine leader.
//!   A follow-up commit will add the `NEW_VIEW` flow, leader rotation per
//!   view, and the locked/prepared safety rules.
//! - **Checkpointing.** Multi-decision sequencing beyond a single view.
//! - **Byzantine-fault simulation tests.** The current test matrix exercises
//!   only honest replicas; adversarial tests will land alongside view
//!   change.
//!
//! # Cohort sizes
//!
//! The spec recommends `λ ∈ {5, 7, 9}` with `z = floor((λ - 1) / 3)`
//! tolerated Byzantine members. The quorum threshold is `λ - z`:
//!
//! | `λ` | `z` | quorum |
//! |-----|-----|--------|
//! | 5   | 1   | 4      |
//! | 7   | 2   | 5      |
//! | 9   | 2   | 7      |
//!
//! [`replica::Config::validate`] enforces the `λ >= 3z + 1` safety
//! requirement.
#![forbid(unsafe_code)]

pub mod replica;
pub mod types;
