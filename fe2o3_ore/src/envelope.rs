//! Signed provenance for an operation.
//!
//! An envelope binds three things: the bytes of an operation, the public key of
//! whoever authored it, and a detached signature over those bytes. History made
//! of envelopes can be checked rather than trusted -- a reader can establish
//! who wrote each edit without trusting the party that handed the history over.
//!
//! The signature scheme is not chosen here. The caller supplies an
//! implementation of [`Signer`], which is where key material lives and where
//! the algorithm is decided; this module only marshals bytes and asks that
//! implementation to sign or verify. That keeps the crate free of key handling
//! and free of any particular algorithm's baggage.

use crate::id::OpId;
use crate::op::Op;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::sign::Signer;
use oxedyne_fe2o3_jdat::prelude::*;


/// An operation's bytes together with the provenance that attests to them.
///
/// The payload is opaque here. [`Envelope::seal_op`] fills it with an operation
/// and the identifier that names it, encoded together, so that the identifier
/// is inside what was signed: an operation lifted out and re-labelled with a
/// different identifier will not verify.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Envelope {
	/// The signed bytes, exactly as they were presented to the signer.
	payload:	Vec<u8>,
	/// The author's public key, as the signature scheme encodes it.
	signer:		Vec<u8>,
	/// The detached signature over `payload`.
	sig:		Vec<u8>,
}

impl Envelope {
	/// Constructs an envelope from parts already in hand, as when one arrives
	/// over a wire.
	///
	/// Nothing is verified here; call [`Envelope::verify`] for that.
	pub fn new(payload: Vec<u8>, signer: Vec<u8>, sig: Vec<u8>) -> Self {
		Self { payload, signer, sig }
	}

	/// Seals arbitrary payload bytes, taking the public key from the scheme.
	///
	/// Fails if the scheme has no public key set, since an envelope whose
	/// signature no one can attribute is of no use.
	pub fn seal<S: Signer>(scheme: &S, payload: Vec<u8>)
		-> Outcome<Self>
	{
		let sig = res!(scheme.sign(&payload));
		let signer = match res!(scheme.get_public_key()) {
			Some(pk) => pk.to_vec(),
			None => return Err(err!(
				"The signing scheme has no public key set, so a sealed envelope could \
				not be attributed to anyone.";
			Missing, Key, Configuration)),
		};
		Ok(Self { payload, signer, sig })
	}

	/// Seals an operation and the identifier that names it.
	///
	/// The pair is encoded as `[id, op]` in binary daticle form, so the
	/// identifier is covered by the signature.
	pub fn seal_op<S: Signer>(scheme: &S, id: &OpId, op: &Op)
		-> Outcome<Self>
	{
		Self::seal(scheme, res!(encode_pair(id, op)))
	}

	/// Verifies the signature against the payload, using the enclosed public
	/// key.
	///
	/// `scheme` supplies the algorithm only; its own keys are set aside and the
	/// envelope's public key is used, so a caller cannot accidentally check a
	/// signature against the wrong key. Returns `false` for a signature that
	/// does not check out, and an error only where verification could not be
	/// attempted.
	pub fn verify<S: Signer>(&self, scheme: &S)
		-> Outcome<bool>
	{
		let bound = res!(scheme.clone_with_keys(Some(&self.signer), None));
		bound.verify(&self.payload, &self.sig)
	}

	/// Verifies the envelope and, if it checks out, returns the identifier and
	/// operation it carries.
	///
	/// Fails rather than returning anything if the signature does not verify,
	/// so a caller cannot use the contents by mistake.
	pub fn open_op<S: Signer>(&self, scheme: &S)
		-> Outcome<(OpId, Op)>
	{
		if !res!(self.verify(scheme)) {
			return Err(err!(
				"The envelope's signature does not verify against its enclosed public \
				key, so its contents are not attributable.";
			Invalid, Input, Security, Mismatch));
		}
		decode_pair(&self.payload)
	}

	/// Returns the identifier and operation without checking the signature.
	///
	/// For a caller that has already verified, or that is inspecting something
	/// it does not intend to trust. Prefer [`Envelope::open_op`].
	pub fn peek_op(&self)
		-> Outcome<(OpId, Op)>
	{
		decode_pair(&self.payload)
	}

	/// Returns the signed bytes.
	pub fn payload(&self) -> &[u8] {
		&self.payload
	}

	/// Returns the author's public key.
	pub fn signer(&self) -> &[u8] {
		&self.signer
	}

	/// Returns the detached signature.
	pub fn signature(&self) -> &[u8] {
		&self.sig
	}

	/// Serialises the envelope to a [`Dat`]. The shape is
	/// `[payload, signer, signature]`.
	///
	/// All three are [`Dat::BU64`]: keys and signatures readily exceed the 255
	/// bytes a [`Dat::BU8`] length field can express, and a truncated length
	/// there would corrupt silently.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			Dat::BU64(self.payload.clone()),
			Dat::BU64(self.signer.clone()),
			Dat::BU64(self.sig.clone()),
		])
	}

	/// Reconstructs an envelope from a [`Dat`] produced by
	/// [`Envelope::to_dat`].
	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let v = match dat {
			Dat::List(v) if v.len() == 3 => v,
			_ => return Err(err!(
				"An Envelope expects a 3-element Dat::List, got {:?}.", dat;
			Decode, Input, Mismatch)),
		};
		Ok(Self {
			payload:	res!(field_bytes(&v[0], "payload")),
			signer:		res!(field_bytes(&v[1], "signer key")),
			sig:		res!(field_bytes(&v[2], "signature")),
		})
	}

	/// Appends the byte encoding of the envelope to `buf`.
	pub fn encode_into(&self, buf: &mut Vec<u8>)
		-> Outcome<()>
	{
		let body = res!(self.to_dat().to_bytes(Vec::new()));
		buf.extend_from_slice(&body);
		Ok(())
	}

	/// Returns the byte encoding of the envelope.
	pub fn encode(&self)
		-> Outcome<Vec<u8>>
	{
		let mut buf = Vec::new();
		res!(self.encode_into(&mut buf));
		Ok(buf)
	}

	/// Decodes an envelope from the front of `buf`, returning it and the number
	/// of bytes consumed.
	pub fn decode(buf: &[u8])
		-> Outcome<(Self, usize)>
	{
		let (dat, used) = res!(Dat::from_bytes(buf));
		Ok((res!(Self::from_dat(&dat)), used))
	}
}


/// Extracts a byte field, naming it if the kind is wrong.
fn field_bytes(dat: &Dat, what: &str)
	-> Outcome<Vec<u8>>
{
	match dat {
		Dat::BU64(b) => Ok(b.clone()),
		other => Err(err!(
			"An Envelope {} expects Dat::BU64, got {:?}.", what, other;
		Decode, Input, Mismatch)),
	}
}

/// Encodes an identifier and operation as one signable byte string.
fn encode_pair(id: &OpId, op: &Op)
	-> Outcome<Vec<u8>>
{
	let dat = Dat::List(vec![
		id.to_dat(),
		op.to_dat(),
	]);
	Ok(res!(dat.to_bytes(Vec::new())))
}

/// Decodes an identifier and operation from a signable byte string.
fn decode_pair(buf: &[u8])
	-> Outcome<(OpId, Op)>
{
	let (dat, used) = res!(Dat::from_bytes(buf));
	if used != buf.len() {
		return Err(err!(
			"An envelope payload of {} bytes decoded from only {} of them.",
			buf.len(), used;
		Decode, Input, Mismatch));
	}
	let v = match &dat {
		Dat::List(v) if v.len() == 2 => v,
		other => return Err(err!(
			"An envelope payload expects a 2-element Dat::List, got {:?}.", other;
		Decode, Input, Mismatch)),
	};
	let id = res!(OpId::from_dat(&v[0]));
	let op = res!(Op::from_dat(&v[1]));
	Ok((id, op))
}


#[cfg(test)]
mod tests {
	use super::*;
	use crate::id::ReplicaId;

	use oxedyne_fe2o3_iop_crypto::keys::KeyManager;
	use oxedyne_fe2o3_namex::id::{
		InNamex,
		NamexId,
	};

	/// A stand-in signature scheme, present only to exercise the envelope's
	/// marshalling and key binding.
	///
	/// It is not cryptography and makes no claim to be: the "signature" is the
	/// secret key interleaved with a fold of the message. What these tests
	/// establish is that the envelope presents the right bytes to the scheme,
	/// carries the right public key, and refuses what the scheme rejects. The
	/// strength of a real scheme is that scheme's business, and is tested where
	/// it is implemented.
	#[derive(Clone, Debug, Default)]
	struct StubSigner {
		/// Public key bytes.
		pk: Vec<u8>,
		/// Secret key bytes.
		sk: Vec<u8>,
	}

	impl StubSigner {
		/// Constructs a key pair from a seed byte.
		fn with_seed(seed: u8) -> Self {
			Self {
				pk: vec![seed; 40],					// Longer than a BU8 length would matter.
				sk: vec![seed.wrapping_add(1); 40],
			}
		}

		/// The stand-in signature: a fold of the message under the secret key.
		fn compute(sk: &[u8], msg: &[u8]) -> Vec<u8> {
			let mut acc = vec![0u8; 32];
			for (i, b) in msg.iter().enumerate() {
				acc[i % 32] = acc[i % 32].wrapping_add(*b).rotate_left(1);
			}
			for (i, b) in sk.iter().enumerate() {
				acc[i % 32] ^= *b;
			}
			acc
		}

		/// The public key a given secret key corresponds to, under the
		/// stand-in's trivial relation.
		fn public_of(sk: &[u8]) -> Vec<u8> {
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

	/// A representative operation.
	fn sample_op() -> Op {
		Op::Splice {
			file:		fmt!("notes.md"),
			at:			12,
			delete_len:	3,
			insert:		vec![0x7e; 900],	// Beyond what a BU8 length could hold.
		}
	}

	/// The stand-in's key relation holds, so that a failure below is the
	/// envelope's doing and not the stub's.
	#[test]
	fn stub_key_relation_holds() -> Outcome<()> {
		let s = StubSigner::with_seed(3);
		assert_eq!(StubSigner::public_of(&s.sk), s.pk);
		Ok(())
	}

	/// A sealed envelope verifies, and carries the signer's public key.
	#[test]
	fn sealed_envelope_verifies() -> Outcome<()> {
		let s = StubSigner::with_seed(3);
		let env = res!(Envelope::seal(&s, b"the payload".to_vec()));
		assert_eq!(env.signer(), &s.pk[..]);
		assert_eq!(env.payload(), b"the payload");
		assert!(res!(env.verify(&s)));
		Ok(())
	}

	/// Verification uses the envelope's key, not the scheme's, so a scheme
	/// holding a different key still checks the envelope correctly.
	#[test]
	fn verification_uses_the_enclosed_key() -> Outcome<()> {
		let author = StubSigner::with_seed(3);
		let other = StubSigner::with_seed(200);
		let env = res!(Envelope::seal(&author, b"payload".to_vec()));
		// A bystander holding entirely different keys still verifies it.
		assert!(res!(env.verify(&other)));
		Ok(())
	}

	/// An envelope whose public key has been swapped for another's fails.
	#[test]
	fn a_substituted_key_fails() -> Outcome<()> {
		let author = StubSigner::with_seed(3);
		let impostor = StubSigner::with_seed(200);
		let env = res!(Envelope::seal(&author, b"payload".to_vec()));
		let forged = Envelope::new(
			env.payload().to_vec(),
			impostor.pk.clone(),
			env.signature().to_vec(),
		);
		assert!(!res!(forged.verify(&author)));
		Ok(())
	}

	/// Tampering with the payload invalidates the signature.
	#[test]
	fn a_tampered_payload_fails() -> Outcome<()> {
		let s = StubSigner::with_seed(3);
		let env = res!(Envelope::seal(&s, b"payload".to_vec()));
		let tampered = Envelope::new(
			b"paylaod".to_vec(),
			env.signer().to_vec(),
			env.signature().to_vec(),
		);
		assert!(!res!(tampered.verify(&s)));
		Ok(())
	}

	/// Tampering with the signature invalidates it.
	#[test]
	fn a_tampered_signature_fails() -> Outcome<()> {
		let s = StubSigner::with_seed(3);
		let env = res!(Envelope::seal(&s, b"payload".to_vec()));
		let mut sig = env.signature().to_vec();
		sig[0] ^= 0xff;
		let tampered = Envelope::new(env.payload().to_vec(), env.signer().to_vec(), sig);
		assert!(!res!(tampered.verify(&s)));
		Ok(())
	}

	/// Sealing without a public key is refused, since the result could not be
	/// attributed.
	#[test]
	fn sealing_without_a_public_key_is_refused() -> Outcome<()> {
		let s = StubSigner::with_seed(3);
		let keyless = res!(s.clone_with_keys(None, Some(&s.sk)));
		assert!(Envelope::seal(&keyless, b"payload".to_vec()).is_err());
		Ok(())
	}

	/// An operation and its identifier survive sealing and opening.
	#[test]
	fn op_survives_seal_and_open() -> Outcome<()> {
		let s = StubSigner::with_seed(11);
		let id = OpId::new(ReplicaId::new(4), 17);
		let op = sample_op();
		let env = res!(Envelope::seal_op(&s, &id, &op));
		let (got_id, got_op) = res!(env.open_op(&s));
		assert_eq!(got_id, id);
		assert_eq!(got_op, op);
		Ok(())
	}

	/// The identifier is covered by the signature: relabelling an operation
	/// breaks it.
	#[test]
	fn the_identifier_is_covered_by_the_signature() -> Outcome<()> {
		let s = StubSigner::with_seed(11);
		let op = sample_op();
		let env = res!(Envelope::seal_op(&s, &OpId::new(ReplicaId::new(4), 17), &op));
		let relabelled = res!(encode_pair(&OpId::new(ReplicaId::new(4), 18), &op));
		let forged = Envelope::new(
			relabelled,
			env.signer().to_vec(),
			env.signature().to_vec(),
		);
		assert!(!res!(forged.verify(&s)));
		Ok(())
	}

	/// Opening refuses to hand back contents whose signature does not verify.
	#[test]
	fn open_refuses_an_unverified_envelope() -> Outcome<()> {
		let s = StubSigner::with_seed(11);
		let id = OpId::new(ReplicaId::new(1), 1);
		let env = res!(Envelope::seal_op(&s, &id, &sample_op()));
		let mut sig = env.signature().to_vec();
		sig[3] ^= 0x01;
		let forged = Envelope::new(env.payload().to_vec(), env.signer().to_vec(), sig);
		assert!(forged.open_op(&s).is_err());
		// Peeking still works, for a caller that knows it is not trusting the result.
		let (got_id, _) = res!(forged.peek_op());
		assert_eq!(got_id, id);
		Ok(())
	}

	/// An envelope survives a [`Dat`] round trip, keys and signatures intact.
	#[test]
	fn envelope_dat_round_trip() -> Outcome<()> {
		let s = StubSigner::with_seed(11);
		let env = res!(Envelope::seal_op(&s, &OpId::new(ReplicaId::new(2), 5), &sample_op()));
		let back = res!(Envelope::from_dat(&env.to_dat()));
		assert_eq!(env, back);
		assert!(res!(back.verify(&s)));
		Ok(())
	}

	/// An envelope survives a byte round trip, including a payload longer than
	/// a single byte length field could express.
	#[test]
	fn envelope_byte_round_trip() -> Outcome<()> {
		let s = StubSigner::with_seed(11);
		let env = res!(Envelope::seal_op(&s, &OpId::new(ReplicaId::new(2), 5), &sample_op()));
		assert!(env.payload().len() > 255);
		let buf = res!(env.encode());
		let (back, used) = res!(Envelope::decode(&buf));
		assert_eq!(used, buf.len());
		assert_eq!(env, back);
		assert!(res!(back.verify(&s)));
		Ok(())
	}

	/// A malformed [`Dat`] is refused.
	#[test]
	fn envelope_from_dat_rejects_rubbish() -> Outcome<()> {
		assert!(Envelope::from_dat(&Dat::U64(1)).is_err());
		assert!(Envelope::from_dat(&Dat::List(vec![
			Dat::BU64(vec![1]),
			Dat::BU64(vec![2]),
		])).is_err());
		assert!(Envelope::from_dat(&Dat::List(vec![
			Dat::BU64(vec![1]),
			Dat::Str(fmt!("key")),
			Dat::BU64(vec![3]),
		])).is_err());
		Ok(())
	}

	/// A payload that is not an identifier and operation pair is refused.
	#[test]
	fn a_payload_that_is_not_an_op_pair_is_refused() -> Outcome<()> {
		let s = StubSigner::with_seed(11);
		let env = res!(Envelope::seal(&s, b"not a daticle at all".to_vec()));
		assert!(env.peek_op().is_err());
		assert!(env.open_op(&s).is_err());
		Ok(())
	}
}
