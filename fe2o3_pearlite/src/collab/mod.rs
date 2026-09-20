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
pub mod keyring;
pub mod op;
pub mod sign;

pub use keyring::Keyring;

use oxedyne_fe2o3_ore::id::ReplicaId;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::bdat::DecodeLimits;

/// The bounds a peer's operation is decoded under.
///
/// A signed operation arriving over a relay is attacker-controlled input, and neither the depth nor
/// the length of what it encodes is the reader's to trust: a few bytes can describe a list nested a
/// million deep, and a decoder that trusts them recurses until its stack is gone. Sixty-four levels is
/// deeper than any annotation the format writes, and a mebibyte is far more than an annotation body
/// needs, so the bound only ever bites a hostile encoding. This is the collaboration layer's policy,
/// passed down to the mechanism in [`oxedyne_fe2o3_ore`], which stays free of any opinion about it.
pub const DECODE_LIMITS: DecodeLimits = DecodeLimits {
	max_depth:	64,
	max_bytes:	1024 * 1024,
};

/// The greatest a single signed operation may be, in bytes of its sealed envelope, before the hub
/// refuses to store it. A relay peer does not get to write an unbounded blob into a document's log.
pub const MAX_OP_BYTES: usize = 1024 * 1024;

/// The greatest number of operations a single document's log may hold before the hub refuses more, so
/// a peer cannot exhaust a store by appending without end.
pub const MAX_OPS_PER_DOC: usize = 100_000;

/// The greatest total size, in bytes of sealed envelope payloads, a single document's log may reach
/// before the hub refuses more. The op-count cap alone leaves a peer room to store a hundred thousand
/// mebibyte operations -- a hundred gibibytes -- so a total-bytes cap bounds the log's real cost
/// rather than only its length.
pub const MAX_DOC_BYTES: usize = 64 * 1024 * 1024;

/// The furthest ahead of the receiving replica's own clock an operation's author-stated time may be,
/// in seconds, before the hub refuses it. An author's clock genuinely differs from a reader's, so some
/// skew is allowed; a `time` far in the future is a bid to win a last-writer-wins ordering forever, and
/// is refused rather than honoured.
pub const MAX_TIME_SKEW_SECS: u64 = 24 * 60 * 60;

/// Derives a replica identity from a signer's public key **and the document it writes in**: the first
/// eight bytes of the SHA-256 of the key followed by the document identity, read big-endian.
///
/// This is what binds an operation's name both to the key that signed it and to the one document it
/// belongs to. A [`ReplicaId`] a peer picks for itself is worthless -- it could pick anyone's -- so it
/// is not picked but computed, here, from the two things a peer cannot forge: the private key behind
/// the public one, and the document being written. A record whose header names a replica this does not
/// derive from its signer *under the document in hand* is refused at [`sign::seal`] and [`sign::open`].
///
/// Folding the document into the replica is what closes cross-document replay for every operation kind
/// at once, the settlement and the reply included, without a wire-format change: the same key signing
/// in document A and in document B mints two different replica identities, so an operation lifted from
/// A and appended to B fails the binding check under B. It also makes an honest counter overlap between
/// two documents impossible -- each document is its own replica space -- and raises identity squatting
/// from guessing a key to guessing a `(key, document)` pair.
pub fn replica_of(pubkey: &[u8], doc: &DocId) -> ReplicaId {
	let mut msg = Vec::with_capacity(pubkey.len() + doc.as_str().len());
	msg.extend_from_slice(pubkey);
	msg.extend_from_slice(doc.as_str().as_bytes());
	let digest = oxedyne_fe2o3_hash::sha256::digest(&msg);
	let mut bytes = [0u8; 8];
	bytes.copy_from_slice(&digest[..8]);
	ReplicaId::new(u64::from_be_bytes(bytes))
}

/// A signer's stable fingerprint for display and for keying a [`Keyring`]: the SHA-256 of the public
/// key, as lower-case hexadecimal.
///
/// The whole digest rather than the eight bytes [`replica_of`] takes, because a fingerprint is shown
/// to a person and compared for identity, where the eight-byte replica identity is only an ordering
/// key: a wider value costs nothing here and leaves no room for a collision to be mistaken for a match.
pub fn fingerprint(pubkey: &[u8]) -> String {
	let digest = oxedyne_fe2o3_hash::sha256::digest(pubkey);
	let mut s = String::with_capacity(digest.len() * 2);
	for b in digest.iter() {
		s.push_str(&fmt!("{:02x}", b));
	}
	s
}

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
