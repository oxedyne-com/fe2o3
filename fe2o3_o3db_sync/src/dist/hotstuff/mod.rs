//! A HotStuff Byzantine-fault-tolerant consensus primitive for small
//! cohorts, intended for the strong-consistency tables of the Hematite
//! distributed Ozone layer.
//!
//! # Protocol
//!
//! This is Basic HotStuff: a fixed leader drives a single view through
//! `Prepare -> PreCommit -> Commit -> Decide`, and the cohort recovers from
//! a silent or misbehaving leader through a view-change round. The primitive
//! is a pure, deterministic state machine; it has no transport, no crypto,
//! and no time. Signatures are opaque byte vectors supplied by the caller
//! and aggregated without inspection.
//!
//! # Safety predicate
//!
//! Every `Prepare` proposal in view `v > 1` must either carry a justify QC
//! that endorses the locally-locked block or carry a justify QC from a view
//! strictly newer than the locally-locked QC. A fresh proposal with no
//! justify is only legal when the replica holds no lock -- which happens
//! when the NewView quorum collected by the new leader saw no prior prepare
//! QC anywhere in the cohort. [`replica::Replica::on_proposal`] enforces
//! this.
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
//! [`replica::Config::validate`] enforces the `λ >= 3z + 1` requirement.
//!
//! # Deferred
//!
//! - Checkpointing of multiple decisions (this primitive is one-decision).
//! - Byzantine-fault simulation tests beyond the single unsafe-proposal case
//!   currently in the integration-test suite.
//! - Signature aggregation (we pass individual signatures through the QC;
//!   callers implementing threshold signatures can swap the aggregation
//!   logic at their integration layer).

pub mod replica;
pub mod types;
