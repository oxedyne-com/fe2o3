//! Vendor-neutral by rule: no string here names a network or an application,
//! and `tests/presentation.rs` fails if one does; the `v` strings of the shapes
//! are the only protocol words. `shape` must also compile to wasm32 with this
//! crate's default features off, since a member's device builds the very bytes
//! a verifier here rebuilds.

pub mod shape;
pub mod verify;
