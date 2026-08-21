//! Version control whose unit of history is the operation, carrying signed
//! provenance.
//!
//! A conventional version control system records states and recovers intent by
//! comparing them: it stores what the tree looked like before and after, and a
//! diff is computed later, as a guess. Here the intent is the record. A whole
//! edit -- create this file, replace this run of bytes with these -- is one
//! operation, named, appended, and signed. What the author meant is preserved
//! rather than reconstructed, and who wrote it can be checked rather than
//! taken on trust.
//!
//! # Layout
//!
//! - [`id`] names operations: a replica identifier and that replica's counter,
//!   with a compact varint encoding. Above it sit the names for content, which
//!   are arithmetic over an operation identifier rather than minted.
//! - [`op`] is the operation vocabulary, an enum, together with the header that
//!   names an operation and records the frontier it was written against.
//!   Everything that speaks about bytes speaks about them by name.
//! - [`log`] is the append-only log, with a per-replica monotonic counter guard
//!   and a causal one: an operation may not arrive before its parents.
//! - [`envelope`] binds a record's bytes to a public key and a signature.
//! - [`seq`] is the convergent sequence: it consumes the operation vocabulary
//!   directly and renders a file's bytes from an operation set.
//! - [`segment`] is the durable form of a run of operations, read incrementally
//!   and checked with a hasher the caller brings.
//! - [`fastexport`] parses git's fast-import stream, which is the one interface
//!   git keeps for foreign consumers of a repository.
//! - [`gitexport`] emits it, so a history written here becomes a git repository
//!   without this crate ever learning what a packfile is.
//! - [`diff`] recovers the edit between two versions of a file's bytes, for the
//!   author who worked in a filesystem rather than in an editor and so has no
//!   intent to record.
//! - [`sync`] brings two logs into agreement: a peer-symmetric state machine
//!   over typed messages, which delivers causal closures and never subsets.
//!
//! # A pure primitive
//!
//! Nothing here does I/O, opens a socket, touches a filesystem or reads a
//! clock. Hashing, key material and transport belong to the caller, reached
//! through the interoperability traits in `oxedyne_fe2o3_iop_hash` and
//! `oxedyne_fe2o3_iop_crypto`, so the choice of algorithm is never made on the
//! caller's behalf. That discipline is also what keeps the crate portable: it
//! compiles for `wasm32-unknown-unknown` unchanged, so the same history logic
//! runs in a browser and on a server without a second implementation.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

#![forbid(unsafe_code)]

pub mod diff;
pub mod envelope;
pub mod fastexport;
pub mod gitexport;
pub mod id;
pub mod log;
pub mod op;
pub mod segment;
pub mod seq;
pub mod sync;

#[cfg(test)]
mod test_support;
