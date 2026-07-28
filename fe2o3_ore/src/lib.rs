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
//!   with a compact varint encoding.
//! - [`op`] is the operation vocabulary, an enum. It is provisional, pending
//!   the sequence-structure design note.
//! - [`log`] is the append-only log, with a per-replica monotonic counter
//!   guard.
//! - [`envelope`] binds an operation's bytes to a public key and a signature.
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

#![forbid(unsafe_code)]

pub mod envelope;
pub mod id;
pub mod log;
pub mod op;
