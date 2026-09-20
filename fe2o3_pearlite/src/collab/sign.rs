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

use oxedyne_fe2o3_ore::{
	envelope::Envelope,
	op::Record,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_crypto::p256::verify_p256_sha256_fixed;

/// The exact bytes an author signs for `rec`: its canonical binary daticle form, identical to what
/// [`Envelope::seal_record`] places in an envelope's payload, so the header is covered along with the
/// operation. A caller with the private key signs these and hands the signature to [`seal`].
pub fn signing_bytes(rec: &Record) -> Outcome<Vec<u8>> {
	Ok(res!(rec.to_dat().to_bytes(Vec::new())))
}

/// Seals a record into an envelope carrying a P-256 signature.
///
/// `pubkey` is the 65-byte uncompressed SEC1 point and `sig` the 64-byte `r || s` over
/// [`signing_bytes`]`(rec)` -- exactly what a browser's WebCrypto key, or a native P-256 signer,
/// yields. The signature is checked here, before the envelope is returned, so a mis-signed operation
/// never reaches the hub and every stored envelope is one that verifies against its own enclosed key.
pub fn seal(rec: &Record, pubkey: Vec<u8>, sig: Vec<u8>) -> Outcome<Envelope> {
	let payload = res!(signing_bytes(rec));
	if !res!(verify_p256_sha256_fixed(&pubkey, &payload, &sig)) {
		return Err(err!(
			"The P-256 signature does not verify over the record's bytes, so the envelope would not \
			be attributable to its enclosed public key.";
			Invalid, Input, Security, Mismatch));
	}
	Ok(Envelope::new(payload, pubkey, sig))
}

/// Checks an envelope's P-256 signature against its own enclosed public key. `false` is a signature
/// that does not hold -- a tampered payload, key or signature -- and an error is a check that could not
/// be made.
pub fn verify(env: &Envelope) -> Outcome<bool> {
	verify_p256_sha256_fixed(env.signer(), env.payload(), env.signature())
}

/// Verifies an envelope and, only if it holds, decodes the record inside it. An envelope whose
/// signature does not verify yields an error rather than a record, so a caller cannot fold in an
/// operation it has not attributed.
pub fn open(env: &Envelope) -> Outcome<Record> {
	if !res!(verify(env)) {
		return Err(err!(
			"The envelope's P-256 signature does not verify against its enclosed public key, so its \
			operation is not attributable and will not be folded.";
			Invalid, Input, Security, Mismatch));
	}
	env.peek_record()
}
