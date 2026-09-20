//! Who a signer is, in a form a reader can show.
//!
//! An operation is attributed to the key that signed it, not to a name written in its body -- a name
//! is free text and any key can claim any name. What a person needs to see, though, is a name and not a
//! key, so a document carries a keyring: a map from a signer's [`fingerprint`](super::fingerprint) to
//! the display name that signer goes by in this document. A signer the keyring does not name is shown
//! by its fingerprint, which is unforgeable and unambiguous even if it is not friendly.
//!
//! # What this is not, yet
//!
//! This is not a membership model. It says what to *call* a signer, not whether that signer is
//! *allowed* to annotate the document -- who may write to a document, and who may say so, is a
//! governance question the next increment answers with a first-class, signed keyring object. Here the
//! rule is only that the signer is the identity and the keyring supplies its name; an unknown signer is
//! displayed, not rejected.

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeMap;


/// The names the signers of a document's operations go by, keyed by signer fingerprint.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Keyring {
	names: BTreeMap<String, String>,	// signer fingerprint -> display name
}

impl Keyring {
	pub fn new() -> Self {
		Self::default()
	}

	/// Records the name a signer, named by its public key, goes by in this document.
	pub fn insert<S: Into<String>>(&mut self, pubkey: &[u8], name: S) {
		self.names.insert(super::fingerprint(pubkey), name.into());
	}

	/// The name to display for the signer whose public key this is: the keyring's name where it holds
	/// one, and otherwise the fingerprint itself, which names the signer unambiguously if not amiably.
	pub fn display(&self, pubkey: &[u8]) -> String {
		let fp = super::fingerprint(pubkey);
		match self.names.get(&fp) {
			Some(name)	=> name.clone(),
			None		=> fp,
		}
	}

	pub fn is_empty(&self) -> bool {
		self.names.is_empty()
	}

	pub fn len(&self) -> usize {
		self.names.len()
	}
}
