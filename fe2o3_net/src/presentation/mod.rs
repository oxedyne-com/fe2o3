//! Presentations, `present/1`: how a relying party, known by its web origin,
//! learns from a signed and fresh answer to its own challenge that a person is
//! a confirmed member of a network of unique humans, either under a public
//! name or under a pseudonym and a linking tag that no other relying party
//! shares.
//!
//! - [`shape`] holds the wire shapes and the bytes each signature covers. It
//!   rests only on the pure-Rust half of Hematite, so it compiles to wasm32
//!   with this crate's default features off, and a member's device builds the
//!   very bytes a verifier rebuilds.
//! - [`verify`] holds the relying party's side: the challenge, issuing and
//!   spending the nonce, freshness, the head, a named Ed25519 signature or a
//!   pairwise `linkring/1` proof over the whole ring at the head, and the
//!   settlement of an invoice.
//!
//! Nothing here names a network or an application. A verifier knows its own
//! origin, the keys whose heads it trusts, and a [`verify::Lookup`] that fetches
//! heads, rings and a name's status for it; the `v` strings of the shapes are
//! the only protocol words it carries.

pub mod shape;
pub mod verify;
