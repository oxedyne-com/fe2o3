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

use crate::op::Record;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::sign::Signer;
use oxedyne_fe2o3_jdat::prelude::*;


/// An operation's bytes together with the provenance that attests to them.
///
/// The payload is opaque here. [`Envelope::seal_record`] fills it with a whole
/// record -- the operation together with the header that names it and lists its
/// parents -- so that both are inside what was signed: an operation lifted out,
/// re-labelled or re-parented will not verify.
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

	/// Seals a record, header and operation together.
	///
	/// The record is encoded in binary daticle form, so the identifier and the
	/// parents are covered by the signature along with the operation.
	pub fn seal_record<S: Signer>(scheme: &S, rec: &Record)
		-> Outcome<Self>
	{
		Self::seal(scheme, res!(rec.to_dat().to_bytes(Vec::new())))
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

	/// Verifies the envelope and, if it checks out, returns the record it
	/// carries.
	///
	/// Fails rather than returning anything if the signature does not verify,
	/// so a caller cannot use the contents by mistake.
	pub fn open_record<S: Signer>(&self, scheme: &S)
		-> Outcome<Record>
	{
		if !res!(self.verify(scheme)) {
			return Err(err!(
				"The envelope's signature does not verify against its enclosed public \
				key, so its contents are not attributable.";
			Invalid, Input, Security, Mismatch));
		}
		decode_record(&self.payload)
	}

	/// Returns the record without checking the signature.
	///
	/// For a caller that has already verified, or that is inspecting something
	/// it does not intend to trust. Prefer [`Envelope::open_record`].
	pub fn peek_record(&self)
		-> Outcome<Record>
	{
		decode_record(&self.payload)
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

/// Decodes a record from a signable byte string.
fn decode_record(buf: &[u8])
	-> Outcome<Record>
{
	let (dat, used) = res!(Dat::from_bytes(buf));
	if used != buf.len() {
		return Err(err!(
			"An envelope payload of {} bytes decoded from only {} of them.",
			buf.len(), used;
		Decode, Input, Mismatch));
	}
	Record::from_dat(&dat)
}


#[cfg(test)]
mod tests {
	use super::*;
	use crate::id::{
		ContentRange,
		OpId,
		ReplicaId,
	};
	use crate::op::{
		Header,
		Op,
	};
	use crate::test_support::StubSigner;

	use oxedyne_fe2o3_iop_crypto::keys::KeyManager;

	/// An operation identifier.
	fn oid(replica: u64, counter: u64) -> OpId {
		OpId::new(ReplicaId::new(replica), counter)
	}

	/// A representative operation.
	fn sample_op() -> Outcome<Op> {
		Ok(Op::Splice {
			file:	fmt!("notes.md"),
			left:	None,
			right:	None,
			remove:	vec![res!(ContentRange::new(oid(1, 1), 12, 15))],
			insert:	vec![0x7e; 900],	// Beyond what a BU8 length could hold.
		})
	}

	/// A representative record, carrying two parents.
	fn sample_record(id: OpId) -> Outcome<Record> {
		Ok(Record::new(
			res!(Header::new(id, vec![oid(1, 1), oid(2, 4)])),
			res!(sample_op()),
		))
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

	/// A whole record survives sealing and opening.
	#[test]
	fn a_record_survives_seal_and_open() -> Outcome<()> {
		let s = StubSigner::with_seed(11);
		let rec = res!(sample_record(oid(4, 17)));
		let env = res!(Envelope::seal_record(&s, &rec));
		assert_eq!(res!(env.open_record(&s)), rec);
		Ok(())
	}

	/// The identifier is covered by the signature: relabelling an operation
	/// breaks it.
	#[test]
	fn the_identifier_is_covered_by_the_signature() -> Outcome<()> {
		let s = StubSigner::with_seed(11);
		let rec = res!(sample_record(oid(4, 17)));
		let env = res!(Envelope::seal_record(&s, &rec));
		let relabelled = res!(sample_record(oid(4, 18)));
		let forged = Envelope::new(
			res!(relabelled.to_dat().to_bytes(Vec::new())),
			env.signer().to_vec(),
			env.signature().to_vec(),
		);
		assert!(!res!(forged.verify(&s)));
		Ok(())
	}

	/// The parents are covered too: re-parenting an operation breaks the
	/// signature, so a causal claim cannot be forged from a genuine edit.
	#[test]
	fn the_parents_are_covered_by_the_signature() -> Outcome<()> {
		let s = StubSigner::with_seed(11);
		let rec = res!(sample_record(oid(4, 17)));
		let env = res!(Envelope::seal_record(&s, &rec));
		let reparented = Record::new(
			res!(Header::new(oid(4, 17), vec![oid(1, 1)])),
			rec.op.clone(),
		);
		let forged = Envelope::new(
			res!(reparented.to_dat().to_bytes(Vec::new())),
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
		let rec = res!(sample_record(oid(5, 1)));
		let env = res!(Envelope::seal_record(&s, &rec));
		let mut sig = env.signature().to_vec();
		sig[3] ^= 0x01;
		let forged = Envelope::new(env.payload().to_vec(), env.signer().to_vec(), sig);
		assert!(forged.open_record(&s).is_err());
		// Peeking still works, for a caller that knows it is not trusting the result.
		assert_eq!(res!(forged.peek_record()).id(), rec.id());
		Ok(())
	}

	/// An envelope survives a [`Dat`] round trip, keys and signatures intact.
	#[test]
	fn envelope_dat_round_trip() -> Outcome<()> {
		let s = StubSigner::with_seed(11);
		let env = res!(Envelope::seal_record(&s, &res!(sample_record(oid(2, 5)))));
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
		let env = res!(Envelope::seal_record(&s, &res!(sample_record(oid(2, 5)))));
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

	/// A payload that is not a record is refused.
	#[test]
	fn a_payload_that_is_not_a_record_is_refused() -> Outcome<()> {
		let s = StubSigner::with_seed(11);
		let env = res!(Envelope::seal(&s, b"not a daticle at all".to_vec()));
		assert!(env.peek_record().is_err());
		assert!(env.open_record(&s).is_err());
		Ok(())
	}
}
