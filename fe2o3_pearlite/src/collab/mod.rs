//! The Pearlite collaboration backend: a signed, mergeable edit stream over a `.prl` document's
//! annotations.
//!
//! An annotation anchors to a content-addressed block, not a page (see
//! [`oxedyne_fe2o3_austenite::emit::pearl::Annotation`]), so it already survives re-pagination within
//! one file. What it does not yet survive is being carried between authors: two people annotating the
//! same document need a shared, verifiable record of who said what, and a way to fold that record back
//! into each one's local `.prl`. This module supplies the first increment of that.
//!
//! The pieces, and the seam between them:
//!
//! - A document has a stable [`DocId`], minted by its author and written into the `.prl` header
//!   ([`oxedyne_fe2o3_austenite::emit::pearl::PearlDoc::doc_id`]). It is the sync key: it is the one
//!   name that survives both re-pagination and a rewrite, where a page number or a block address does
//!   not.
//! - Each annotation action is one Ore operation ([`op`]), carrying signed provenance in an Ore
//!   [`Envelope`](oxedyne_fe2o3_ore::envelope::Envelope) ([`sign`]). Creating an annotation opens a
//!   proposal, a reply is a `Said`, an edit an `Amended`, and a resolve or a withdrawal a `Settled` --
//!   the threaded-discussion vocabulary the operation log already speaks.
//! - The signature is P-256 ECDSA over SHA-256, exactly the encoding a browser's WebCrypto key emits,
//!   so an operation a browser signs and one a server signs are checked by the very same verifier
//!   ([`oxedyne_fe2o3_crypto::p256`], which is wasm-clean so the reader checks on-device).
//! - A document's operations live in a [`hub`] store keyed by [`DocId`], and folding that stream
//!   ([`fold`]) reproduces the annotation set deterministically, which is then materialised into a
//!   local `.prl`.
//!
//! # What this increment is not
//!
//! There is no transport here and no browser code. The live Steel WebSocket relay that carries
//! operations between a browser and the hub, the `web/pearl-reader/collab.js` reader UI, and the
//! WebCrypto signing path in the browser are the next increment, deliberately left out so this one
//! stops at a clean seam: sealed operations in, a folded annotation set out, over a store that already
//! works.

pub mod fold;
pub mod hub;
pub mod op;
pub mod sign;

use oxedyne_fe2o3_core::prelude::*;

/// A document's stable identity: the key its edit stream is stored and folded against.
///
/// It is minted once by whoever creates the document, written into the `.prl` header, and never
/// derived from the document's contents -- a content-derived name would change the moment the document
/// did, which is the one thing the sync key must not do.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DocId(String);

impl DocId {
	pub fn new<S: Into<String>>(id: S) -> Self {
		Self(id.into())
	}

	pub fn as_str(&self) -> &str {
		&self.0
	}
}

impl std::fmt::Display for DocId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.0)
	}
}
