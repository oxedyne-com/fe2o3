//! Sealing an annotation operation into a signed Ore envelope, and checking one, with a P-256
//! (WebCrypto ECDSA / SHA-256) signature.
//!
//! # Where the key lives
//!
//! Nowhere here. An author's private key stays with the author: a browser's WebCrypto `CryptoKey`
//! minted `extractable: false`, or a native P-256 key on a server. This module never sees it. What it
//! is handed is the public key and a detached signature over the operation's canonical bytes -- the
//! two things an [`Envelope`] carries beside the payload -- so the same code path serves a browser and
//! a server, and the signing algorithm is decided by whoever holds the key rather than here.
//!
//! # What is signed
//!
//! The whole record: the operation together with the header naming it and its parents, in the same
//! canonical binary daticle form [`Envelope::seal_record`] uses. An operation lifted out of its
//! history, relabelled or reparented, does not verify.
//!
//! The verifier is [`oxedyne_fe2o3_crypto::p256`], pure Rust and wasm-clean, so an operation is checked
//! by identical code on a server and inside a downloaded reader -- the reader never has to trust a
//! server to have checked provenance for it.

use crate::collab::{
	replica_of,
	DECODE_LIMITS,
};

use oxedyne_fe2o3_ore::{
	envelope::Envelope,
	op::Record,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_crypto::p256::verify_p256_sha256_fixed;

/// A verified record together with the public key that signed it.
///
/// The signer travels with the record because it, and not anything in the body, is the operation's
/// identity: who authored an annotation is who holds the key, and the fold reads the author from here
/// rather than from a free-text field a body could claim anything in. Every [`Opened`] that exists is
/// one whose signature verified and whose header names the replica [`replica_of`] derives from this
/// key, so a caller holding one need not re-establish either.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Opened {
	pub record:	Record,
	pub signer:	Vec<u8>,	// the 65-byte SEC1 public key that signed the record
}

/// The exact bytes an author signs for `rec`: its canonical binary daticle form, identical to what
/// [`Envelope::seal_record`] places in an envelope's payload, so the header is covered along with the
/// operation. A caller with the private key signs these and hands the signature to [`seal`].
pub fn signing_bytes(rec: &Record) -> Outcome<Vec<u8>> {
	Ok(res!(rec.to_dat().to_bytes(Vec::new())))
}

/// Refuses a record whose header names a replica that is not the one its signer's key derives.
///
/// This is the whole of the identity binding: a replica identity is not a peer's to choose, it is
/// [`replica_of`] of the key, so a record claiming another author's replica -- to pre-empt their next
/// identifier or to poison their counter -- carries a replica the impostor's own key does not derive,
/// and is refused here before it can be sealed or folded.
fn check_replica_binding(rec: &Record, pubkey: &[u8]) -> Outcome<()> {
	let named	= rec.id().replica;
	let derived	= replica_of(pubkey);
	if named != derived {
		return Err(err!(
			"The record is identified {}, whose replica {} is not the replica {} derived from the \
			signer's key; a replica identity is bound to the key that signs under it, not chosen.",
			rec.id(), named, derived;
			Invalid, Input, Security, Mismatch));
	}
	Ok(())
}

/// Seals a record into an envelope carrying a P-256 signature.
///
/// `pubkey` is the 65-byte uncompressed SEC1 point and `sig` the 64-byte `r || s` over
/// [`signing_bytes`]`(rec)` -- exactly what a browser's WebCrypto key, or a native P-256 signer,
/// yields. The signature is checked here, and the record's replica identity is checked to be the one
/// the key derives, before the envelope is returned, so a mis-signed or misattributed operation never
/// reaches the hub and every stored envelope is one that verifies against, and is named for, its own
/// enclosed key.
pub fn seal(rec: &Record, pubkey: Vec<u8>, sig: Vec<u8>) -> Outcome<Envelope> {
	let payload = res!(signing_bytes(rec));
	if !res!(verify_p256_sha256_fixed(&pubkey, &payload, &sig)) {
		return Err(err!(
			"The P-256 signature does not verify over the record's bytes, so the envelope would not \
			be attributable to its enclosed public key.";
			Invalid, Input, Security, Mismatch));
	}
	res!(check_replica_binding(rec, &pubkey));
	Ok(Envelope::new(payload, pubkey, sig))
}

/// Checks an envelope's P-256 signature against its own enclosed public key. `false` is a signature
/// that does not hold -- a tampered payload, key or signature -- and an error is a check that could not
/// be made.
pub fn verify(env: &Envelope) -> Outcome<bool> {
	verify_p256_sha256_fixed(env.signer(), env.payload(), env.signature())
}

/// Verifies an envelope and, only if it holds, decodes the record inside it under the collaboration
/// layer's decode bounds and checks the record's replica against its signer.
///
/// An envelope whose signature does not verify, whose payload is a hostile encoding, or whose header
/// claims a replica its key does not derive yields an error rather than a record, so a caller cannot
/// fold in an operation it has not attributed. The signer travels back with the record in the
/// [`Opened`], since that key -- not anything in the body -- is the operation's author.
pub fn open(env: &Envelope) -> Outcome<Opened> {
	if !res!(verify(env)) {
		return Err(err!(
			"The envelope's P-256 signature does not verify against its enclosed public key, so its \
			operation is not attributable and will not be folded.";
			Invalid, Input, Security, Mismatch));
	}
	let record = res!(env.peek_record_limited(&DECODE_LIMITS));
	res!(check_replica_binding(&record, env.signer()));
	Ok(Opened { record, signer: env.signer().to_vec() })
}
