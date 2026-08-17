//! Stand-ins the tests bring, because the crate brings none of its own.
//!
//! A hash function and a signature scheme are the caller's to supply, which is
//! what makes the crate a primitive; the price is that its own tests have to
//! supply them too. Neither of these is cryptography and neither claims to be.
//! What they establish is that this crate presents the right bytes to a scheme
//! and does the right thing with what comes back, which is all it is
//! responsible for; the strength of a real scheme is tested where it is
//! implemented.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::alt::Gnomon;
use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::{
	InNamex,
	NamexId,
	keys::KeyManager,
	sign::Signer,
};
use oxedyne_fe2o3_iop_hash::api::{
	Hash,
	HashForm,
	Hasher,
};


/// A stand-in hash function: a 64-bit fold of the input, in eight bytes.
///
/// It is not a cryptographic hash. It is here so that the segment tests have a
/// digest short enough to write down and stable enough to freeze, which is what
/// a golden-bytes test needs and what the identity hasher, whose digest is the
/// whole input again, cannot give.
#[derive(Clone, Copy, Debug, Default)]
pub struct Fold;

impl InNamex for Fold {
	fn name_id(&self) -> Outcome<NamexId> {
		Ok(NamexId::default())
	}
}

impl Hasher for Fold {
	fn hash<const S: usize>(self, input: &[&[u8]], salt: [u8; S])
		-> Hash<S>
	{
		let mut acc: u64 = 0xcbf2_9ce4_8422_2325;
		for slice in input {
			for b in *slice {
				acc ^= *b as u64;
				acc = acc.wrapping_mul(0x0000_0100_0000_01b3);
			}
		}
		for b in salt.iter() {
			acc ^= *b as u64;
			acc = acc.wrapping_mul(0x0000_0100_0000_01b3);
		}
		Hash::new(HashForm::U64(acc), salt)
	}

	fn hash_length(&self) -> Gnomon<usize> {
		Gnomon::Known(8)
	}

	fn is_identity(&self) -> bool {
		false
	}
}


/// A stand-in signature scheme, present only to exercise the marshalling of
/// envelopes and the segments that carry them.
///
/// It is not cryptography and makes no claim to be: the "signature" is the
/// secret key interleaved with a fold of the message. What these tests
/// establish is that the envelope presents the right bytes to the scheme,
/// carries the right public key, and refuses what the scheme rejects. The
/// strength of a real scheme is that scheme's business, and is tested where
/// it is implemented.
#[derive(Clone, Debug, Default)]
pub struct StubSigner {
	pub pk: Vec<u8>,
	pub sk: Vec<u8>,
}

impl StubSigner {
	/// Constructs a key pair from a seed byte.
	pub fn with_seed(seed: u8) -> Self {
		Self {
			pk: vec![seed; 40],					// longer than a BU8 length would matter
			sk: vec![seed.wrapping_add(1); 40],
		}
	}

	/// The stand-in signature: a fold of the message under the secret key.
	pub fn compute(sk: &[u8], msg: &[u8]) -> Vec<u8> {
		let mut acc = vec![0u8; 32];
		for (i, b) in msg.iter().enumerate() {
			acc[i % 32] = acc[i % 32].wrapping_add(*b).rotate_left(1);
		}
		for (i, b) in sk.iter().enumerate() {
			acc[i % 32] ^= *b;
		}
		acc
	}

	/// The public key a given secret key corresponds to, under the stand-in's
	/// trivial relation.
	pub fn public_of(sk: &[u8]) -> Vec<u8> {
		sk.iter().map(|b| b.wrapping_sub(1)).collect()
	}
}

impl InNamex for StubSigner {
	fn name_id(&self) -> Outcome<NamexId> {
		Ok(NamexId::default())
	}
}

impl KeyManager for StubSigner {
	fn clone_with_keys(&self, pk: Option<&[u8]>, sk: Option<&[u8]>)
		-> Outcome<Self>
	{
		Ok(Self {
			pk: match pk {
				Some(b) => b.to_vec(),
				None => Vec::new(),
			},
			sk: match sk {
				Some(b) => b.to_vec(),
				None => Vec::new(),
			},
		})
	}

	fn get_public_key(&self) -> Outcome<Option<&[u8]>> {
		Ok(if self.pk.is_empty() { None } else { Some(&self.pk) })
	}

	fn get_secret_key(&self) -> Outcome<Option<&[u8]>> {
		Ok(if self.sk.is_empty() { None } else { Some(&self.sk) })
	}

	fn set_public_key(mut self, pk: Option<&[u8]>) -> Outcome<Self> {
		self.pk = match pk {
			Some(b) => b.to_vec(),
			None => Vec::new(),
		};
		Ok(self)
	}

	fn set_secret_key(mut self, sk: Option<&[u8]>) -> Outcome<Self> {
		self.sk = match sk {
			Some(b) => b.to_vec(),
			None => Vec::new(),
		};
		Ok(self)
	}
}

impl Signer for StubSigner {
	fn sign(&self, msg: &[u8]) -> Outcome<Vec<u8>> {
		if self.sk.is_empty() {
			return Err(err!("No secret key set."; Missing, Key));
		}
		Ok(Self::compute(&self.sk, msg))
	}

	fn verify(&self, msg: &[u8], sig: &[u8]) -> Outcome<bool> {
		if self.pk.is_empty() {
			return Err(err!("No public key set."; Missing, Key));
		}
		// Recover the secret key the public key implies, and recompute.
		let sk: Vec<u8> = self.pk.iter().map(|b| b.wrapping_add(1)).collect();
		Ok(Self::compute(&sk, msg) == sig)
	}
}
