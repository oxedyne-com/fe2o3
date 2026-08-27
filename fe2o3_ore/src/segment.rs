//! An append-only run of operations, as bytes.
//!
//! A segment is the durable shape of a stretch of history: a short header
//! saying what the bytes are, then one length-prefixed record after another,
//! each with an integrity check. Appending is writing at the end, and nothing
//! already written is ever revisited, so a writer needs no index and a reader
//! needs no seek.
//!
//! Records come in three forms and the format carries all of them, tagged. A
//! bare [`Record`] is what a replica writes for itself, where provenance is not
//! in question; an [`Envelope`] is the same record with a public key and a
//! signature around it, which is what crosses between parties. A segment may
//! hold either or both, so a repository that starts unsigned and later gains
//! signatures does not need a second format.
//!
//! The third form is a [`Veiled`] record, which is one of the other two
//! encrypted whole, with its header left in clear beside the ciphertext. It
//! exists for the case where the machine holding the bytes is not one of the
//! parties: a carrier that has to place an operation needs its identifier and
//! its parents and nothing else, so those are what it is given.
//!
//! # The caller owns the cipher too
//!
//! [`Entry::veil`] and [`Entry::unveil`] take an implementation of [`Encrypter`]
//! for the same reason the digest takes a [`Hasher`]: which cipher, and whose
//! key, are decisions this crate has no business making. It marshals bytes and
//! asks the caller's scheme to encrypt or decrypt them.
//!
//! # The caller owns the hash
//!
//! Each record carries a digest, and which function computes it is not decided
//! here. The caller brings an implementation of [`Hasher`] and a salt, and the
//! same pair must be brought to read the bytes back. That keeps the crate free
//! of any particular hash and lets a browser use what its platform offers while
//! a server uses what its peers have agreed on.
//!
//! The digest covers the record's kind byte and its body. It is a check against
//! damage, not against forgery: anyone who can rewrite a record can rewrite the
//! digest beside it. Forgery is what the signature in an [`Envelope`] is for.
//!
//! # No I/O
//!
//! Bytes in, records out. Nothing here opens a file or names a path; a segment
//! is a byte buffer that a caller may choose to write to disk.
//!
//! # Incremental
//!
//! [`Reader::feed`] takes whatever bytes have arrived and
//! [`Reader::next_entry`] yields records as they complete, so a segment larger
//! than memory can be read a chunk at a time. Memory is bounded by the largest
//! single record rather than by the segment, and feeding a segment one byte at
//! a time yields exactly what feeding it all at once yields.
//!
//! Writing is incremental in the same way, and across runs as well as within
//! one: [`Writer::resume`] continues a segment already written and emits only
//! the records appended to it, so a caller holding a segment on disk adds to it
//! by writing at the end. Resuming reads the existing bytes first, under the
//! hasher and salt it is given, so a segment that a later reader could not get
//! to the end of is refused before anything is added to it.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::envelope::Envelope;
use crate::id::{
	varint_decode,
	varint_encode,
	OpId,
	ReplicaId,
	VARINT_MAX_LEN,
};
use crate::op::{
	Header,
	Record,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::prelude::*;


pub const MAGIC: [u8; 6] = *b"ORESEG"; // the bytes every segment begins with

/// The format version this module writes.
///
/// Raised to 2 when the operation vocabulary changed for file identity: a
/// content operation no longer carries a file, a lifecycle operation names one
/// by identity, and a path is bytes rather than a string. Nothing was ever
/// written in version 1 that needs to be read again.
///
/// Raised to 3 when the vocabulary gained [`crate::op::Op::FileMode`] at wire
/// code 8. The framing did not move a byte; what the version declares is which
/// operations the records inside may be.
///
/// Raised to 4 when the vocabulary gained five codes at once: wire code 9, the
/// second spelling of [`crate::op::Op::Mark`], for a mark carrying what was said
/// and when; and wire codes 10 to 13 for [`crate::op::Op::Proposal`],
/// [`crate::op::Op::Said`], [`crate::op::Op::Settled`] and
/// [`crate::op::Op::Reverts`], which put a proposal, its discussion, its outcome
/// and what a revert undoes into history rather than beside it. The framing did
/// not move a byte this time either, and neither did a mark that carries neither
/// a body nor a time: it is still written at wire code 4 with the two elements it
/// always had, so every mark ever signed still verifies.
///
/// Raised to 5 when the vocabulary gained [`crate::op::Op::Amended`] at wire code
/// 14, so that a proposal's author can state it again without the opening
/// operation being touched. The framing did not move, and the four proposal codes
/// below it did not move either.
pub const VERSION: u8 = 5;

/// The oldest format version this module reads.
///
/// Version 2 stays readable because each version since has been a strict
/// superset of it: the framing is identical and the vocabulary has only ever
/// grown upwards, so every version 2 segment ever written means in version 4
/// exactly what it meant in version 2, and so does every version 3 one. That is
/// what a bump buys -- a reader meeting a version it does not know says which
/// version it met, rather than reporting an operation code it cannot place --
/// and it is why a bump costs a repository nothing.
///
/// The rule the vocabulary grows by is what keeps that true, and it is not open
/// to reinterpretation: a new code goes strictly above the existing ones,
/// [`VERSION`] rises by one, [`highest_code`] gains a branch, and this constant
/// stays where it is.
///
/// Version 1 is not read. Its operations spelled a file as a path and a path as
/// a string, so its records are not the same records under another number.
pub const VERSION_MIN: u8 = 2;

/// The highest operation code a segment of the given format version may carry.
///
/// Version 2 was frozen with [`crate::op::CODE_NOTE`] at the top of the
/// vocabulary; version 3 added [`crate::op::Op::FileMode`] above it and nothing
/// else; version 4 added the five codes from [`crate::op::CODE_MARK_TIMED`] up to
/// [`crate::op::CODE_REVERTS`]; version 5 added [`crate::op::CODE_AMENDED`] above
/// those and nothing else. A writer continuing a segment somebody else wrote
/// asks this rather than assuming, so that "an older version is a strict subset
/// of a newer one" stays true of the bytes and not only of the intention.
///
/// This is the whole mechanism the additive design rests on. A version 3 segment
/// handed an [`crate::op::Op::Reverts`] refuses it here, and the caller starts a
/// segment at the current version instead, which is why nothing already written
/// has to be rewritten and why nothing already written can be misread.
pub const fn highest_code(version: u8) -> u8 {
	if version <= VERSION_MIN {
		crate::op::CODE_NOTE
	} else if version == 3 {
		crate::op::CODE_FILE_MODE
	} else if version == 4 {
		crate::op::CODE_REVERTS
	} else {
		crate::op::CODE_AMENDED
	}
}

/// Kind byte of a record carrying a bare [`Record`].
pub const KIND_BARE:	u8 = 1;
/// Kind byte of a record carrying a signed [`Envelope`].
pub const KIND_SEALED:	u8 = 2;
/// Kind byte of a record whose header is in clear and whose body is encrypted.
///
/// The kind is a separate axis from [`VERSION`], and neither moved for the
/// other. A version says which operations the records inside a segment may be; a
/// veiled record's operation is ciphertext, so there is no code in it for a
/// version to bound, and a segment written at any version this reader knows may
/// carry one. What a reader meeting a form it does not have needs to be told is
/// the form, and the kind byte says so in the record where it is rather than in a
/// header that would condemn every plain record beside it.
pub const KIND_VEILED:	u8 = 3;
/// Kind byte of a record carrying a RUN of records, deflated together.
///
/// The same axis as [`KIND_VEILED`] and for the same reason: it says what was
/// done to the records in the record where they are, rather than in a header
/// that would condemn every plain record beside it. [`VERSION`] does not move
/// for it, a segment may hold packed and plain records side by side, and a
/// reader that meets one and cannot inflate says so by name.
///
/// **A run and not a record.** Compression saves what is redundant *between*
/// records, and a record is about 1.4 kB, which is too small a window to see any
/// of it: measured on a 55 MB segment, deflating each record on its own reached
/// 58.8% where deflating runs of a megabyte reached 38.4%. So one packed record
/// carries a run, each run inflatable on its own, and reaching a record costs
/// its own megabyte rather than the whole segment.
///
/// **What is inside is the plain framing, unchanged.** The inflated bytes are
/// exactly the bytes those records would have occupied unpacked, digests
/// included, so a packed segment and a plain one carrying the same history yield
/// the same records with the same digests in the same order. That is what keeps
/// a fold over those digests -- the thing a repack compares two stores by -- the
/// same on both sides, and it is why packing is revocable.
pub const KIND_PACKED:	u8 = 4;

/// Most bytes a packed run may inflate to.
///
/// A compressed frame is an instruction to allocate, and the instruction arrives
/// from wherever the segment did. The declared run is a megabyte and this is
/// sixty-four, so nothing a writer here produces comes near it and a frame that
/// does is refused by name rather than obeyed.
pub const PACKED_MAX: usize = 64 << 20;

// How much consumed prefix a reader tolerates before it moves the remainder to
// the front of its buffer.
const COMPACT_THRESHOLD: usize = 1 << 16;


/// Whether a reader recomputes each record's digest, or takes the one written
/// beside it.
///
/// Recomputing is what a segment's framing is for and it is the default; there
/// is no way to reach [`Integrity::Vouched`] except by asking for it. The two
/// read the same records out of the same bytes and differ only in what they
/// notice about damage.
///
/// # What vouching costs, and what it is worth
///
/// The digest covers `[kind] || body` of one record, so recomputing it hashes
/// every byte of the segment. Over a 35,408 operation log in 85 MB that is
/// 420 ms of a 574 ms decode -- the file I/O beneath it is 7.7 ms -- which is
/// paid on every read of a history that has not changed since the last one.
///
/// What it buys is the only check that survives a segment nothing else looks
/// at. A signature says who wrote a record and is skippable by a caller that
/// remembers checking it; the digest says the bytes are the bytes, and a body
/// byte that flips on the disk changes neither the file's length nor its
/// modification time nor the digests recorded beside the bodies. So a caller
/// that vouches has stopped looking for bit rot on this read, and something
/// else must look for it on some other one. That is not a trade this crate can
/// make on a caller's behalf, which is why it is an argument.
///
/// **A caller warranting [`Integrity::Vouched`] warrants three things**: that
/// these exact bytes were read under [`Integrity::Checked`] at some point, that
/// nothing has appended to or rewritten the file since, and that something
/// still re-reads them checked on a timescale it has chosen. Vouching for a
/// file still being written, or for one this machine has never checked, throws
/// the check away and puts nothing in its place.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Integrity {
	#[default]
	Checked,	// hash each body and refuse a record whose digest does not match
	Vouched,	// take the recorded digest as read, on the caller's warrant above
}


/// What a segment says about itself before its first record.
///
/// The replica hint is exactly that: a note of who was writing, which lets a
/// reader sort a directory of segments without opening them. Nothing depends on
/// it, and a segment whose records come from several replicas simply leaves it
/// out rather than lying.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Head {
	pub version:	u8,					// format version the segment was written in
	pub replica:	Option<ReplicaId>,	// who was writing, where one replica wrote all of it
}

impl Head {
	/// Constructs a header at the current format version.
	pub fn new(replica: Option<ReplicaId>) -> Self {
		Self { version: VERSION, replica }
	}

	/// The shape is the magic, the version, a byte saying whether a replica hint
	/// follows, and the hint if it does.
	pub fn encode_into(&self, buf: &mut Vec<u8>) {
		buf.extend_from_slice(&MAGIC);
		buf.push(self.version);
		match self.replica {
			Some(r) => {
				buf.push(1);
				r.encode_into(buf);
			},
			None => buf.push(0),
		}
	}

	pub fn encode(&self) -> Vec<u8> {
		let mut buf = Vec::with_capacity(MAGIC.len() + 2 + VARINT_MAX_LEN);
		self.encode_into(&mut buf);
		buf
	}

	/// Yields the header and how many bytes it took. `None` means the bytes so
	/// far are a prefix of a header and more are needed; an error means they are
	/// not a header at all.
	pub fn decode(buf: &[u8])
		-> Outcome<Option<(Self, usize)>>
	{
		if buf.len() < MAGIC.len() {
			// Only refuse what could not become the magic however it continues.
			if buf != &MAGIC[..buf.len()] {
				return Err(err!(
					"A segment begins {:02x?}, which is not the magic {:02x?}.",
					buf, MAGIC;
				Decode, Input, Invalid));
			}
			return Ok(None);
		}
		if buf[..MAGIC.len()] != MAGIC {
			return Err(err!(
				"A segment begins {:02x?}, which is not the magic {:02x?}.",
				&buf[..MAGIC.len()], MAGIC;
			Decode, Input, Invalid));
		}
		let mut at = MAGIC.len();
		if buf.len() <= at {
			return Ok(None);
		}
		let version = buf[at];
		at += 1;
		if !(VERSION_MIN..=VERSION).contains(&version) {
			return Err(err!(
				"A segment declares format version {}, and this reader knows versions \
				{} to {}.", version, VERSION_MIN, VERSION;
			Decode, Input, Version, Mismatch));
		}
		if buf.len() <= at {
			return Ok(None);
		}
		let tag = buf[at];
		at += 1;
		match tag {
			0 => Ok(Some((Self { version, replica: None }, at))),
			1 => match res!(try_varint(&buf[at..])) {
				Some((n, used)) => Ok(Some((
					Self { version, replica: Some(ReplicaId::new(n)) },
					at + used,
				))),
				None => Ok(None),
			},
			other => Err(err!(
				"A segment's replica hint is tagged {}, which is neither 0 for absent \
				nor 1 for present.", other;
			Decode, Input, Invalid)),
		}
	}
}


/// An entry whose header can be read and whose body cannot.
///
/// The header is in clear because a carrier that never reads an operation still
/// has to place one: a frontier walk follows parents, a sketch is keyed by
/// identifiers, and a closure check wants both. None of them wants an operation
/// body, which is what rendering wants, and a carrier does not render. So this is
/// the whole of what a repository gives away to be carried, and the rest of it is
/// ciphertext.
///
/// The signature is inside, over the plaintext record, which puts verification
/// where decryption is: at a reader holding the key, never at the carrier. The
/// clear header duplicates the one sealed inside, and that duplication is what
/// makes a carrier that alters it detectable rather than merely suspected --
/// [`Entry::unveil`] compares the two and refuses the pair if they disagree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Veiled {
	pub head:	Header,		// identifier and parents, in clear
	pub body:	Vec<u8>,	// the whole entry, encrypted
}

impl Veiled {
	/// The shape is `[head, body]`.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			self.head.to_dat(),
			Dat::BU64(self.body.clone()),
		])
	}

	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let v = match dat {
			Dat::List(v) if v.len() == 2 => v,
			_ => return Err(err!(
				"A veiled record expects a 2-element Dat::List, got {:?}.", dat;
			Decode, Input, Mismatch)),
		};
		let head = res!(Header::from_dat(&v[0]));
		// One width and not the narrower ones a shorter body would also fit. Two
		// byte spellings of one veiled entry would both decode to the same thing
		// and hash differently, and a record whose digest depends on which
		// spelling it arrived in is not one a carrier can pass on unaltered.
		let body = match &v[1] {
			Dat::BU64(b) => b.clone(),
			other => return Err(err!(
				"The veiled body of {} is encoded {:?}; it is written under a 64-bit \
				length, whatever its size.", head.id(), other;
			Decode, Input, Mismatch)),
		};
		Ok(Self { head, body })
	}
}


/// One record of a segment: an operation, with or without its provenance, and
/// readable or not.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Entry {
	/// An operation written down as it stands.
	Bare(Record),
	/// An operation with a public key and a signature around it.
	Sealed(Envelope),
	/// An operation whose header is in clear and whose body is encrypted.
	Veiled(Veiled),
}

impl Entry {
	/// Returns the kind byte identifying the form.
	pub fn kind(&self) -> u8 {
		match self {
			Self::Bare(_)	=> KIND_BARE,
			Self::Sealed(_)	=> KIND_SEALED,
			Self::Veiled(_)	=> KIND_VEILED,
		}
	}

	/// The form's name, for messages.
	pub fn name(&self) -> &'static str {
		match self {
			Self::Bare(_)	=> "bare record",
			Self::Sealed(_)	=> "sealed envelope",
			Self::Veiled(_)	=> "veiled record",
		}
	}

	pub fn is_veiled(&self) -> bool {
		matches!(self, Self::Veiled(_))
	}

	/// The header, which every form carries in clear.
	///
	/// This is the one question a carrier may ask of any entry whatever, and it is
	/// what separates carrying a history from reading one. A veiled entry answers
	/// from the clear header beside its ciphertext and the other two from the
	/// record they hold, so a caller that wants the graph and not the content never
	/// wants a key.
	pub fn head(&self)
		-> Outcome<Header>
	{
		Ok(match self {
			Self::Bare(rec)	=> rec.head.clone(),
			Self::Sealed(e)	=> res!(e.peek_record()).head,
			Self::Veiled(v)	=> v.head.clone(),
		})
	}

	/// Opens a sealed record without checking its signature.
	///
	/// Verification is the caller's to do, with the scheme the caller holds; a
	/// segment reader has no key material and makes no claim about provenance.
	///
	/// A veiled entry fails here rather than answering with a stand-in, because
	/// every caller of this asks it in order to read an operation, and a body
	/// nobody can read is not one. The failure names the operation, so a carrier
	/// handed a form it was not built for says which one it was.
	pub fn peek(&self)
		-> Outcome<Record>
	{
		match self {
			Self::Bare(rec)	=> Ok(rec.clone()),
			Self::Sealed(e)	=> e.peek_record(),
			Self::Veiled(v) => Err(err!(
				"The operation {} is veiled: its body is encrypted under a key held by \
				whoever may read this repository, and not by whoever carries it. Its \
				header is readable with `Entry::head`, and its body with \
				`Entry::unveil` and the key.", v.head.id();
			Invalid, Input, Missing, Key)),
		}
	}

	pub fn id(&self)
		-> Outcome<OpId>
	{
		Ok(res!(self.head()).id())
	}

	/// Encrypts an entry whole, leaving its header in clear.
	///
	/// What goes under the cipher is the entry's own tagged form, so the form
	/// travels with it: a sealed envelope unveils to a sealed envelope, signature
	/// and public key intact, and a bare record to a bare record. Nothing about the
	/// entry is re-encoded on the way, so a signature made before it was veiled is
	/// the signature checked after it is unveiled.
	///
	/// Veiling a veiled entry is refused. A second wrapping would hide a header
	/// that is already hidden, which is the one thing the form exists not to do.
	pub fn veil<E: Encrypter>(&self, enc: &E)
		-> Outcome<Self>
	{
		if let Self::Veiled(v) = self {
			return Err(err!(
				"The operation {} is veiled already, and veiling it again would hide \
				the header a carrier places it by.", v.head.id();
			Invalid, Input, Duplicate));
		}
		let head = res!(self.head());
		let plain = res!(self.to_dat().to_bytes(Vec::new()));
		Ok(Self::Veiled(Veiled { head, body: res!(enc.encrypt(&plain)) }))
	}

	/// Decrypts a veiled entry, and refuses one whose clear header is not the
	/// header inside it.
	///
	/// The comparison is the whole of what the duplicated header buys. A carrier
	/// cannot touch the copy inside, which is under the signature, but it can
	/// rewrite the copy in clear, and every peer that never holds the key would
	/// place the operation by the rewritten one. So the first reader with the key
	/// checks the two against each other, and a disagreement is refused by name
	/// with the signed copy named as the one to believe.
	pub fn unveil<E: Encrypter>(&self, enc: &E)
		-> Outcome<Self>
	{
		let veiled = match self {
			Self::Veiled(v) => v,
			other => return Err(err!(
				"A {} is not veiled, so there is nothing to unveil.", other.name();
			Invalid, Input, Mismatch)),
		};
		let plain = match enc.decrypt(&veiled.body) {
			Ok(p) => p,
			Err(e) => return Err(err!(e,
				"The {} byte body of the veiled operation {} did not decrypt. Either \
				this is not the key the repository was veiled under, or the bytes have \
				been altered since.", veiled.body.len(), veiled.head.id();
			Invalid, Input, Decrypt, Key)),
		};
		let (dat, used) = res!(Dat::from_bytes(&plain));
		if used != plain.len() {
			return Err(err!(
				"The veiled operation {} decrypted to {} bytes and decoded from only {} \
				of them.", veiled.head.id(), plain.len(), used;
			Decode, Input, Mismatch));
		}
		let inner = res!(Self::from_dat(&dat));
		if inner.is_veiled() {
			return Err(err!(
				"The veiled operation {} holds another veiled record.", veiled.head.id();
			Decode, Input, Invalid));
		}
		let inside = res!(inner.head());
		if inside != veiled.head {
			return Err(err!(
				"A veiled entry says in clear that it is {} written against {}, and the \
				record inside it is {} written against {}. The clear header is what a \
				carrier places an operation by, so the two disagreeing means the carrier \
				was given one history and shown another; the record inside is the signed \
				one and is what to believe.",
				veiled.head.id(), said_parents(&veiled.head),
				inside.id(), said_parents(&inside);
			Invalid, Input, Security, Mismatch));
		}
		Ok(inner)
	}

	/// What the entry comes to in a carrier, without any of it being encoded.
	///
	/// Exactly the length [`Entry::to_dat`] encodes to, and which form that is
	/// matters: this is the tagged shape a sync message carries, not the untagged
	/// body a segment writes beside a kind byte of its own.
	///
	/// A carrier that bounds what it will send measures every entry it considers
	/// and sends only some of them, so measuring by serialising buys a number at
	/// the price of the history it is about to throw away. On fe2o3's own history
	/// that is a 22,153,680 byte operation encoded and discarded once per clone.
	pub fn dat_len(&self)
		-> Outcome<usize>
	{
		Ok(res!(self.to_dat().byte_len().ok_or_else(|| err!(
			"A {} holds a daticle whose encoded length cannot be known without \
			encoding it, which is a kind no entry was ever built to carry.", self.name();
		Bug, Invalid))))
	}

	/// The shape is `[kind, body]`.
	///
	/// This is the form for a carrier that is itself a daticle, such as a sync
	/// message. A segment does not use it: there the kind is a byte of the frame
	/// and the body stands alone, so that the digest can cover both without
	/// re-encoding.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			Dat::U8(self.kind()),
			match self {
				Self::Bare(rec)	=> rec.to_dat(),
				Self::Sealed(e)	=> e.to_dat(),
				Self::Veiled(v)	=> v.to_dat(),
			},
		])
	}

	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let v = match dat {
			Dat::List(v) if v.len() == 2 => v,
			_ => return Err(err!(
				"An Entry expects a 2-element Dat::List, got {:?}.", dat;
			Decode, Input, Mismatch)),
		};
		let kind = match &v[0] {
			Dat::U8(k) => *k,
			other => return Err(err!(
				"An Entry kind expects Dat::U8, got {:?}.", other;
			Decode, Input, Mismatch)),
		};
		match kind {
			KIND_BARE	=> Ok(Self::Bare(res!(Record::from_dat(&v[1])))),
			KIND_SEALED	=> Ok(Self::Sealed(res!(Envelope::from_dat(&v[1])))),
			KIND_VEILED	=> Ok(Self::Veiled(res!(Veiled::from_dat(&v[1])))),
			other => Err(err!(
				"An Entry is tagged {}, which is none of {} for a bare record, {} for a \
				sealed envelope and {} for a veiled one.",
				other, KIND_BARE, KIND_SEALED, KIND_VEILED;
			Decode, Input, Invalid)),
		}
	}

	/// The daticle form of whichever shape the entry holds.
	fn body(&self)
		-> Outcome<Vec<u8>>
	{
		let dat = match self {
			Self::Bare(rec)	=> rec.to_dat(),
			Self::Sealed(e)	=> e.to_dat(),
			Self::Veiled(v)	=> v.to_dat(),
		};
		Ok(res!(dat.to_bytes(Vec::new())))
	}

	fn from_body(kind: u8, body: &[u8])
		-> Outcome<Self>
	{
		let (dat, used) = res!(Dat::from_bytes(body));
		if used != body.len() {
			return Err(err!(
				"A segment record body of {} bytes decoded from only {} of them.",
				body.len(), used;
			Decode, Input, Mismatch));
		}
		match kind {
			KIND_BARE	=> Ok(Self::Bare(res!(Record::from_dat(&dat)))),
			KIND_SEALED	=> Ok(Self::Sealed(res!(Envelope::from_dat(&dat)))),
			KIND_VEILED	=> Ok(Self::Veiled(res!(Veiled::from_dat(&dat)))),
			other => Err(err!(
				"A segment record is tagged {}, which is none of {} for a bare record, \
				{} for a sealed envelope and {} for a veiled one.",
				other, KIND_BARE, KIND_SEALED, KIND_VEILED;
			Decode, Input, Invalid)),
		}
	}
}


/// Builds a segment, record by record.
///
/// The bytes accumulate in memory; where they go afterwards is the caller's
/// business. A writer is generic over the hasher rather than taking one per
/// call, so that every record of a segment is checked the same way by
/// construction.
///
/// A writer either starts a segment, with [`Writer::new`], or continues one
/// already written, with [`Writer::resume`]. The difference is only whether the
/// header is emitted, since a segment is its header and then records to the end;
/// what a resumed writer hands back is the records alone, to be appended to the
/// bytes they continue.
#[derive(Clone, Debug)]
pub struct Writer<H: Hasher, const S: usize> {
	hasher:		H,			// hash function each record's digest is computed with
	salt:		[u8; S],	// salt each digest is computed under
	version:	u8,			// declared format version, which bounds the vocabulary
	buf:		Vec<u8>,	// bytes written so far
	count:		usize,		// records held, those resumed from included
}

impl<H: Hasher, const S: usize> Writer<H, S> {

	/// Constructs a writer, emitting the segment header at once.
	pub fn new(head: &Head, hasher: H, salt: [u8; S]) -> Self {
		let mut buf = Vec::new();
		head.encode_into(&mut buf);
		Self { hasher, salt, version: head.version, buf, count: 0 }
	}

	/// The bytes handed back afterwards are the new records alone, which a caller
	/// appends to the segment they were resumed from; the header is not emitted a
	/// second time. [`Writer::count`] carries on from the records already there.
	///
	/// `existing` is read through first, under the hasher and the salt given, and
	/// that is what makes appending safe: a different hash function, a different
	/// salt, a segment written in another format version, and a segment left
	/// half-written by an interrupted append all fail here, rather than being
	/// quietly extended into bytes no reader can get to the end of.
	pub fn resume(existing: &[u8], hasher: H, salt: [u8; S])
		-> Outcome<Self>
	{
		let mut reader: Reader<H, S> = Reader::new(hasher.clone(), salt);
		reader.feed(existing);
		reader.end();
		// Every record is decoded and its digest checked, and nothing is kept:
		// what is wanted is the count and the assurance, not the operations.
		while res!(reader.next_entry()).is_some() {}
		let version = match reader.head() {
			Some(head) => head.version,
			None => return Err(err!(
				"A segment of {} bytes carries no header, so there is nothing to \
				continue.", existing.len();
			Decode, Input, Missing)),
		};
		Ok(Self { hasher, salt, version, buf: Vec::new(), count: reader.count() })
	}

	pub const fn version(&self) -> u8 {
		self.version
	}

	/// A record is refused where the segment's declared version has no code for
	/// the operation it carries, which is what keeps an older version a genuine
	/// subset of a newer one rather than a promise the bytes break. A caller with
	/// such a record to write starts a segment at the current version instead;
	/// nothing about a log says its segments share a version.
	///
	/// A veiled record is not asked, because nothing here can ask it: its
	/// operation is ciphertext and carries no code for the version to bound. That
	/// is not a hole in the claim, since the claim is about what a reader of the
	/// segment can be handed, and what a reader is handed here is an opaque body
	/// and the name of a key it does not have.
	pub fn push(&mut self, entry: &Entry)
		-> Outcome<()>
	{
		res!(self.admits(entry));
		let mut framed = Vec::new();
		res!(self.frame_into(entry, &mut framed));
		self.buf.extend_from_slice(&framed);
		self.count += 1;
		Ok(())
	}

	/// Writes a run of entries as one packed record.
	///
	/// What goes under the compressor is the framing those entries would have had
	/// written plainly -- kind, length, body, digest, one after another -- so
	/// inflating yields exactly the bytes a plain segment holds and the records
	/// come back with the digests they always had. Nothing about an entry is
	/// re-encoded on the way in or out, so a signature made before packing is the
	/// signature checked after it.
	///
	/// The outer record carries a digest of the compressed bytes, which is what
	/// catches damage before anything is inflated.
	pub fn push_packed(&mut self, entries: &[Entry])
		-> Outcome<()>
	{
		if entries.is_empty() {
			return Err(err!(
				"A packed record was asked for over no entries. An empty run would be \
				a record carrying nothing, which a reader cannot tell from a damaged \
				one."; Invalid, Input, Missing));
		}
		let mut plain = Vec::new();
		for entry in entries {
			res!(self.admits(entry));
			res!(self.frame_into(entry, &mut plain));
		}
		let body = res!(deflate(&plain));
		let digest = self.hasher.clone().hash(&[&[KIND_PACKED], &body], self.salt).as_vec();
		self.buf.push(KIND_PACKED);
		varint_encode(body.len() as u64, &mut self.buf);
		self.buf.extend_from_slice(&body);
		varint_encode(digest.len() as u64, &mut self.buf);
		self.buf.extend_from_slice(&digest);
		self.count += entries.len();
		Ok(())
	}

	/// Refuses an entry whose operation the declared version has no code for.
	fn admits(&self, entry: &Entry)
		-> Outcome<()>
	{
		if self.version < VERSION && !entry.is_veiled() {
			let op = res!(entry.peek()).op;
			let top = highest_code(self.version);
			if op.code() > top {
				return Err(err!(
					"A segment declaring format version {} cannot carry an {}, whose \
					wire code {} is above the {} that version spells; version {} is \
					where that operation was added.",
					self.version, op.name(), op.code(), top, VERSION;
				Invalid, Input, Version, Mismatch));
			}
		}
		Ok(())
	}

	/// Appends one record's framing, which is the same whether it is going
	/// straight into the segment or into a run about to be packed.
	fn frame_into(&self, entry: &Entry, out: &mut Vec<u8>)
		-> Outcome<()>
	{
		let kind = entry.kind();
		let body = res!(entry.body());
		let digest = self.hasher.clone().hash(&[&[kind], &body], self.salt).as_vec();
		out.push(kind);
		varint_encode(body.len() as u64, out);
		out.extend_from_slice(&body);
		varint_encode(digest.len() as u64, out);
		out.extend_from_slice(&digest);
		Ok(())
	}

	pub fn extend<'a, I>(&mut self, entries: I)
		-> Outcome<()>
	where
		I: IntoIterator<Item = &'a Entry>,
	{
		for entry in entries {
			res!(self.push(entry));
		}
		Ok(())
	}

	/// Counting the records a resumed writer was given as well as those it has
	/// written.
	pub fn count(&self) -> usize {
		self.count
	}

	/// For a resumed writer, the new records alone.
	pub fn bytes(&self) -> &[u8] {
		&self.buf
	}

	/// For a resumed writer, the bytes to append to the segment it continues.
	pub fn finish(self) -> Vec<u8> {
		self.buf
	}
}


/// Reads a segment as its bytes arrive.
///
/// Bytes go in through [`Reader::feed`], records come out through
/// [`Reader::next_entry`], and [`Reader::end`] declares that no more bytes are
/// coming. Until `end` has been called, `next_entry` returning `None` means only
/// that the next record is not yet complete; afterwards it means the segment is
/// finished, and a record left half-written is an error.
#[derive(Clone, Debug)]
pub struct Reader<H: Hasher, const S: usize> {
	hasher:	H,					// hash function each record's digest is checked with
	salt:	[u8; S],			// salt each digest is checked under
	buf:	Vec<u8>,			// bytes fed but not turned into records, consumed prefix included
	pos:	usize,				// how much of `buf` has been consumed
	eof:	bool,				// whether the caller has declared the segment complete
	head:	Option<Head>,		// the header, once it has been read
	count:	usize,				// records handed over
	tally:	Option<Vec<u8>>,	// digests of records handed over since the last take
	check:	Integrity,			// whether each record's digest is recomputed
	// A packed record inflated, and how much of it has been handed over. A run is
	// drained before another byte of the segment is looked at, so the records
	// come out in the order they went in and a caller cannot tell a packed
	// segment from a plain one.
	run:	Vec<u8>,
	ran:	usize,
}

impl<H: Hasher, const S: usize> Reader<H, S> {

	pub fn new(hasher: H, salt: [u8; S]) -> Self {
		Self {
			hasher,
			salt,
			buf:	Vec::new(),
			pos:	0,
			eof:	false,
			head:	None,
			count:	0,
			tally:	None,
			check:	Integrity::Checked,
			run:	Vec::new(),
			ran:	0,
		}
	}

	/// Reads on the caller's warrant that these bytes have already been checked,
	/// rather than checking them again.
	///
	/// Read [`Integrity`] before reaching for this. It takes the segment's only
	/// defence against a body byte that flipped on the disk out of the read, and
	/// it is the caller who has to say where that defence went instead.
	pub fn integrity(mut self, check: Integrity) -> Self {
		self.check = check;
		self
	}

	/// A reader that also keeps the digest of every record it hands over, for a
	/// caller that wants to name the exact bytes it read.
	///
	/// Each record's digest is computed to check it and then dropped, so a caller
	/// that wanted one had no way to ask and would have to hash the segment a
	/// second time -- 205 ms over a 55 MB segment, measured, against the 7 ms of
	/// hashing digests that were paid for already. Take them with
	/// [`Reader::take_digests`] as they accumulate, which is what keeps them from
	/// growing to one digest for every record in the segment.
	pub fn tallying(hasher: H, salt: [u8; S]) -> Self {
		let mut reader = Self::new(hasher, salt);
		reader.tally = Some(Vec::new());
		reader
	}

	/// Takes up a segment part way through, at a byte that is a record boundary.
	///
	/// A reader ordinarily learns the header from the bytes it is fed, and the
	/// only place a header is written is the first bytes of the segment. So a
	/// reader that is to be fed from the middle has to be told two things the
	/// bytes it will see do not carry: the header the segment declared, and how
	/// many records stand before the first one it will be handed. The header is
	/// what lets it place records at all; the count is what lets a damaged record
	/// name its own position in the file rather than its position in the read.
	///
	/// **The caller warrants that the next byte fed begins a record.** Nothing
	/// here can check that: a segment carries no index and a record is found only
	/// by reading the one before it, so the only party that knows where a record
	/// begins is the read that stopped there. A byte offset taken from anywhere
	/// else will be refused as a damaged record, which is the right answer given
	/// the wrong question.
	///
	/// A segment grows only at its end and nothing already written is ever
	/// revisited, which is what makes taking one up worth doing: a reader that
	/// kept where it stopped reads only what has arrived since.
	pub fn take_up(&mut self, head: Head, ordinal: usize) {
		self.head = Some(head);
		self.count = ordinal;
	}

	/// The digests of the records handed over since the last call, in order, for
	/// a reader built by [`Reader::tallying`]. Empty for any other.
	pub fn take_digests(&mut self) -> Vec<u8> {
		match &mut self.tally {
			Some(tally)	=> std::mem::take(tally),
			None		=> Vec::new(),
		}
	}

	/// Chunk boundaries carry no meaning: a record may be split anywhere, and
	/// the same segment delivered in different chunkings yields the same
	/// records.
	pub fn feed(&mut self, chunk: &[u8]) {
		self.buf.extend_from_slice(chunk);
	}

	/// Declares that no further bytes will be fed.
	pub fn end(&mut self) {
		self.eof = true;
	}

	/// `None` until enough bytes have arrived for the header to be read.
	pub fn head(&self)
		-> Option<&Head>
	{
		self.head.as_ref()
	}

	pub fn count(&self) -> usize {
		self.count
	}

	pub fn remaining(&self) -> &[u8] {
		&self.buf[self.pos..]
	}

	/// Has the segment ended with every byte of it turned into a record?
	pub fn is_exhausted(&self) -> bool {
		self.eof && self.pos >= self.buf.len() && self.ran >= self.run.len()
	}

	/// `None` means that the next record is not yet complete, or, once
	/// [`Reader::end`] has been called, that the segment is finished. An error
	/// names the record that could not be read.
	pub fn next_entry(&mut self)
		-> Outcome<Option<Entry>>
	{
		if self.head.is_none() {
			match res!(Head::decode(&self.buf[self.pos..])) {
				Some((head, used)) => {
					self.head = Some(head);
					self.pos += used;
					self.compact();
				},
				None => {
					if self.eof {
						return Err(err!(
							"A segment ends part way through its header, after {} \
							byte{}.", self.buf.len() - self.pos,
							if self.buf.len() - self.pos == 1 { "" } else { "s" };
						Decode, Input, Missing));
					}
					return Ok(None);
				},
			}
		}
		// A run already inflated is drained first, so that the records of a packed
		// segment arrive in the order they were packed and the caller cannot tell
		// which kind of segment it is reading.
		if self.ran < self.run.len() {
			let (entry, used, digest) = {
				let taken = res!(framed(
					&self.hasher, self.salt, &self.run[self.ran..], self.count, self.check));
				match taken {
					Some(f) => {
						if f.kind == KIND_PACKED {
							return Err(err!(
								"Record {} of a packed run is itself packed. A run holds \
								the records it packed and nothing else; a run inside a run \
								would hide the framing a reader places records by.",
								self.count;
							Decode, Input, Invalid));
						}
						(res!(Entry::from_body(f.kind, f.body)), f.used, f.digest.to_vec())
					},
					None => return Err(err!(
						"A packed run ends part way through record {}, with {} byte{} \
						left over. The run inflated and what came out is not the framing \
						that went in.", self.count, self.run.len() - self.ran,
						if self.run.len() - self.ran == 1 { "" } else { "s" };
					Decode, Input, Missing)),
				}
			};
			self.ran += used;
			self.count += 1;
			if let Some(tally) = &mut self.tally {
				tally.extend_from_slice(&digest);
			}
			if self.ran >= self.run.len() {
				self.run = Vec::new();
				self.ran = 0;
			}
			return Ok(Some(entry));
		}
		if self.pos >= self.buf.len() {
			return Ok(None);
		}
		// The outer digest of a packed record covers the compressed bytes and is
		// checked before anything is inflated, which is what stops a damaged frame
		// becoming an instruction to allocate. It is NOT tallied: what a fold over
		// a log names is the records, and a packed segment holds the same records
		// as the plain one it was made from.
		let made = {
			let taken = res!(framed(
				&self.hasher, self.salt, &self.buf[self.pos..], self.count, self.check));
			match taken {
				Some(f) if f.kind == KIND_PACKED	=> Some((Made::Run(res!(
					inflate(f.body, self.count))), f.used)),
				Some(f)								=> Some((Made::One(res!(
					Entry::from_body(f.kind, f.body)), f.digest.to_vec()), f.used)),
				None								=> None,
			}
		};
		match made {
			Some((Made::One(entry, digest), used)) => {
				self.pos += used;
				self.count += 1;
				if let Some(tally) = &mut self.tally {
					tally.extend_from_slice(&digest);
				}
				self.compact();
				Ok(Some(entry))
			},
			Some((Made::Run(run), used)) => {
				self.pos += used;
				self.run = run;
				self.ran = 0;
				self.compact();
				self.next_entry()
			},
			None => {
				if self.eof {
					Err(err!(
						"A segment ends part way through record {}, with {} byte{} \
						left over.", self.count, self.buf.len() - self.pos,
						if self.buf.len() - self.pos == 1 { "" } else { "s" };
					Decode, Input, Missing))
				} else {
					Ok(None)
				}
			},
		}
	}

	/// Drops the consumed prefix of the buffer once it is worth the move.
	fn compact(&mut self) {
		if self.pos == self.buf.len() {
			self.buf.clear();
			self.pos = 0;
		} else if self.pos >= COMPACT_THRESHOLD {
			self.buf.drain(..self.pos);
			self.pos = 0;
		}
	}
}


/// What one step of [`Reader::next_entry`] produced from the segment: a record,
/// or a run to be handed over a record at a time.
enum Made {
	One(Entry, Vec<u8>),	// the entry, and the digest it was checked against
	Run(Vec<u8>),			// a packed record inflated, still framed
}

/// One framed record, with the digest written beside it.
///
/// The kind is handed back rather than interpreted, because what is done next
/// depends on it: a bare, sealed or veiled record becomes an [`Entry`], and a
/// packed one becomes a run of them.
struct Framed<'a> {
	kind:	u8,
	body:	&'a [u8],
	used:	usize,		// bytes of `buf` the whole record occupied
	digest:	&'a [u8],	// the record's digest, which is what a fold wants
}

/// Reads one framed record from the front of `buf`, whether that is a segment
/// being fed or a run just inflated.
///
/// `None` means the bytes so far are a prefix of a record and more are needed;
/// the caller decides whether more can arrive. `ordinal` is only for the
/// messages, so that a damaged record names its own position.
///
/// Under [`Integrity::Vouched`] the body is not hashed and the recorded digest
/// is handed back unexamined, so what comes out of a run of records is the same
/// bytes either way and only the damage a fold could name differs.
fn framed<'a, H: Hasher, const S: usize>(
	hasher:		&H,
	salt:		[u8; S],
	buf:		&'a [u8],
	ordinal:	usize,
	check:		Integrity,
)
	-> Outcome<Option<Framed<'a>>>
{
	if buf.is_empty() {
		return Ok(None);
	}
	let kind = buf[0];
	let mut at = 1usize;
	let (len, used) = match res!(try_varint(&buf[at..])) {
		Some(v)	=> v,
		None	=> return Ok(None),
	};
	at += used;
	let body_end = match at.checked_add(len as usize) {
		Some(e) if (len as u64) <= usize::MAX as u64 => e,
		_ => return Err(err!(
			"Record {} of the segment declares a body of {} bytes, which no buffer \
			can hold.", ordinal, len;
		Decode, Input, Excessive)),
	};
	if buf.len() < body_end {
		return Ok(None);
	}
	let body = &buf[at..body_end];
	at = body_end;
	let (dlen, used) = match res!(try_varint(&buf[at..])) {
		Some(v)	=> v,
		None	=> return Ok(None),
	};
	at += used;
	let digest_end = match at.checked_add(dlen as usize) {
		Some(e) if (dlen as u64) <= usize::MAX as u64 => e,
		_ => return Err(err!(
			"Record {} of the segment declares a digest of {} bytes, which no buffer \
			can hold.", ordinal, dlen;
		Decode, Input, Excessive)),
	};
	if buf.len() < digest_end {
		return Ok(None);
	}
	let digest = &buf[at..digest_end];
	if let Integrity::Vouched = check {
		return Ok(Some(Framed { kind, body, used: digest_end, digest }));
	}
	let want = hasher.clone().hash(&[&[kind], body], salt).as_vec();
	if want != digest {
		// Naming the operation is worth a decode attempt, since a caller with a
		// damaged segment wants to know which edit is at risk. Where the body is
		// too far gone to decode, the ordinal is all there is to say.
		let named = match kind {
			KIND_PACKED => fmt!("a run of packed operations"),
			_ => match Entry::from_body(kind, body) {
				Ok(entry) => match entry.id() {
					Ok(id)	=> fmt!("the operation {}", id),
					Err(_)	=> fmt!("an unreadable operation"),
				},
				Err(_) => fmt!("an unreadable operation"),
			},
		};
		return Err(err!(
			"Record {} of the segment, carrying {}, fails its integrity check: {} \
			bytes of body hash to {:02x?}, and {:02x?} was recorded.",
			ordinal, named, body.len(), want, digest;
		Decode, Input, Checksum, Mismatch));
	}
	Ok(Some(Framed { kind, body, used: digest_end, digest }))
}

/// Compresses a run's plain framing.
fn deflate(plain: &[u8])
	-> Outcome<Vec<u8>>
{
	let mut out = Vec::new();
	let mut enc = flate2::write::DeflateEncoder::new(&mut out, flate2::Compression::new(6));
	match std::io::Write::write_all(&mut enc, plain) {
		Ok(())	=> (),
		Err(e)	=> return Err(err!(e,
			"{} bytes of records could not be compressed.", plain.len();
		Encode, Data)),
	}
	match enc.finish() {
		Ok(_)	=> (),
		Err(e)	=> return Err(err!(e,
			"{} bytes of records could not be compressed.", plain.len();
		Encode, Data)),
	}
	Ok(out)
}

/// Inflates a packed run, refusing one that would not stop.
///
/// A compressed frame is an instruction to allocate and it arrives from wherever
/// the segment did, so the output is bounded by [`PACKED_MAX`] and a frame that
/// reaches it is refused rather than obeyed. The bound is what makes
/// [`Integrity::Vouched`] safe to offer: under [`Integrity::Checked`] the digest
/// over the compressed bytes has held by the time this is called and what
/// remains to guard against is a frame somebody wrote to be obeyed, but a
/// vouched read reaches here with the frame unexamined and the same bound holds
/// it.
fn inflate(body: &[u8], ordinal: usize)
	-> Outcome<Vec<u8>>
{
	let mut out = Vec::new();
	// `std::io::Read::take`, not the iterator's: the bound is on bytes read.
	let mut dec = std::io::Read::take(
		flate2::read::DeflateDecoder::new(body), (PACKED_MAX as u64) + 1);
	match std::io::Read::read_to_end(&mut dec, &mut out) {
		Ok(_)	=> (),
		Err(e)	=> return Err(err!(e,
			"The packed run at record {} carries {} bytes that do not inflate. The \
			digest over them held, so the bytes are the bytes that were written and \
			what is wrong is what they say.", ordinal, body.len();
		Decode, Input, Invalid)),
	}
	if out.len() > PACKED_MAX {
		return Err(err!(
			"The packed run at record {} inflates past {} bytes, which is the most a \
			run may come to. A run this crate writes is a megabyte, so this is not one \
			of them, and it is refused rather than allocated for.",
			ordinal, PACKED_MAX;
		Decode, Input, Excessive));
	}
	if out.is_empty() {
		return Err(err!(
			"The packed run at record {} inflates to nothing. A run carries the \
			records it packed, and an empty one cannot be told from a damaged one.",
			ordinal;
		Decode, Input, Missing));
	}
	Ok(out)
}

/// A header's parents, named the way a reader names them.
fn said_parents(head: &Header) -> String {
	let said: Vec<String> = head.parents().iter().map(|p| fmt!("{}", p)).collect();
	if said.is_empty() {
		fmt!("nothing, as a root")
	} else {
		said.join(", ")
	}
}


/// `None` means the bytes so far are a prefix of a varint and more are needed;
/// an error means they are not a varint at all, whatever follows.
fn try_varint(buf: &[u8])
	-> Outcome<Option<(u64, usize)>>
{
	let ended = buf.iter().take(VARINT_MAX_LEN).any(|b| *b & 0x80 == 0);
	if !ended && buf.len() < VARINT_MAX_LEN {
		return Ok(None);
	}
	let (n, used) = res!(varint_decode(buf));
	Ok(Some((n, used)))
}


/// Writes a whole segment in one go.
pub fn encode<H: Hasher, const S: usize>(
	head:		&Head,
	entries:	&[Entry],
	hasher:		H,
	salt:		[u8; S],
)
	-> Outcome<Vec<u8>>
{
	let mut writer: Writer<H, S> = Writer::new(head, hasher, salt);
	res!(writer.extend(entries));
	Ok(writer.finish())
}

/// Reads a whole segment held in memory.
pub fn decode<H: Hasher, const S: usize>(bytes: &[u8], hasher: H, salt: [u8; S])
	-> Outcome<(Head, Vec<Entry>)>
{
	let mut reader: Reader<H, S> = Reader::new(hasher, salt);
	reader.feed(bytes);
	reader.end();
	let mut entries = Vec::new();
	while let Some(entry) = res!(reader.next_entry()) {
		entries.push(entry);
	}
	let head = match reader.head() {
		Some(h) => *h,
		None => return Err(err!(
			"A segment of {} bytes carries no header.", bytes.len();
		Decode, Input, Missing)),
	};
	Ok((head, entries))
}


#[cfg(test)]
mod tests {
	use super::*;

	use crate::id::{
		Anchor,
		ContentId,
		ContentRange,
	};
	use crate::op::{
		Header,
		Mode,
		Op,
	};
	use crate::op::tests::samples;
	use crate::test_support::{
		Fold,
		StubSigner,
	};

	use oxedyne_fe2o3_iop_crypto::{
		InNamex,
		NamexId,
		keys::KeyManager,
	};

	fn oid(replica: u64, counter: u64) -> OpId {
		OpId::new(ReplicaId::new(replica), counter)
	}

	/// A stand-in cipher: the input under a keystream folded from the key, with
	/// four bytes of tag after it.
	///
	/// It is not cryptography and is offered as none. What these tests need of a
	/// cipher is three things: that the plaintext cannot be found in the output by
	/// searching for it, that only the same key gets it back, and that a wrong key
	/// fails rather than returning rubbish. An Ore repository veils under
	/// AES-256-GCM from `oxedyne_fe2o3_crypto`, which is tested where it is
	/// implemented; what is tested here is that this module puts the right bytes
	/// in front of a cipher and does the right thing with what comes back.
	#[derive(Clone, Debug, Default)]
	struct StubCipher {
		/// The shared key.
		key: Vec<u8>,
	}

	impl StubCipher {
		fn with_seed(seed: u8) -> Self {
			Self { key: vec![seed; 16] }
		}

		/// The fold both the keystream and the tag are drawn from.
		fn fold(key: &[u8], extra: &[u8]) -> u64 {
			let mut acc: u64 = 0xcbf2_9ce4_8422_2325;
			for b in key.iter().chain(extra.iter()) {
				acc ^= *b as u64;
				acc = acc.wrapping_mul(0x0000_0100_0000_01b3);
			}
			acc
		}

		fn stream(&self, data: &[u8]) -> Vec<u8> {
			let mut acc = Self::fold(&self.key, &[]);
			data.iter()
				.map(|b| {
					acc = acc
						.wrapping_mul(6_364_136_223_846_793_005)
						.wrapping_add(1_442_695_040_888_963_407);
					b ^ (acc >> 33) as u8
				})
				.collect()
		}
	}

	impl InNamex for StubCipher {
		fn name_id(&self) -> Outcome<NamexId> {
			Ok(NamexId::default())
		}
	}

	impl KeyManager for StubCipher {
		fn clone_with_keys(&self, _pk: Option<&[u8]>, sk: Option<&[u8]>)
			-> Outcome<Self>
		{
			Ok(Self {
				key: match sk {
					Some(b)	=> b.to_vec(),
					None	=> Vec::new(),
				},
			})
		}

		fn get_public_key(&self) -> Outcome<Option<&[u8]>> { Ok(None) }

		fn get_secret_key(&self) -> Outcome<Option<&[u8]>> { Ok(Some(&self.key)) }

		fn set_public_key(self, _pk: Option<&[u8]>) -> Outcome<Self> { Ok(self) }

		fn set_secret_key(mut self, sk: Option<&[u8]>) -> Outcome<Self> {
			self.key = match sk {
				Some(b)	=> b.to_vec(),
				None	=> Vec::new(),
			};
			Ok(self)
		}
	}

	impl Encrypter for StubCipher {
		fn encrypt(&self, data: &[u8])
			-> Outcome<Vec<u8>>
		{
			let mut out = self.stream(data);
			out.extend_from_slice(&Self::fold(&self.key, data).to_be_bytes()[..4]);
			Ok(out)
		}

		fn decrypt(&self, data: &[u8])
			-> Outcome<Vec<u8>>
		{
			if data.len() < 4 {
				return Err(err!(
					"A body of {} bytes is shorter than the tag.", data.len();
				Decode, Input, Missing));
			}
			let cut = data.len() - 4;
			let plain = self.stream(&data[..cut]);
			if Self::fold(&self.key, &plain).to_be_bytes()[..4] != data[cut..] {
				return Err(err!(
					"The tag does not check out under this key."; Invalid, Input, Decrypt));
			}
			Ok(plain)
		}

		fn is_identity(&self) -> bool { false }
	}

	/// A handful of records spanning the vocabulary, with roots and merges among
	/// their headers.
	fn records() -> Outcome<Vec<Record>> {
		Ok(vec![
			Record::root(oid(1, 1), Op::FileCreate { path: b"notes.md".to_vec() }),
			Record::new(
				res!(Header::new(oid(1, 2), vec![oid(1, 1)])),
				Op::Splice {
					left:	Some(Anchor::origin(oid(1, 1))),
					right:	None,
					remove:	Vec::new(),
					insert:	b"the quick brown fox".to_vec().into(),
				},
			),
			Record::new(
				res!(Header::new(oid(2, 3), vec![oid(1, 2)])),
				Op::Move {
					src:	vec![res!(ContentRange::new(oid(1, 2), 4, 9))],
					left:	Some(Anchor::after(ContentId::new(oid(1, 2), 18))),
					right:	None,
				},
			),
			Record::new(
				res!(Header::new(oid(3, 9), vec![oid(1, 2), oid(2, 3)])),
				Op::Splice {
					left:	Some(Anchor::after(ContentId::new(oid(1, 2), 0))),
					right:	Some(Anchor::before(ContentId::new(oid(1, 2), 1))),
					remove:	vec![res!(ContentRange::new(oid(1, 2), 10, 15))],
					insert:	vec![0x2a; 900].into(),	// beyond a single byte length
				},
			),
			Record::new(
				res!(Header::new(oid(3, 10), vec![oid(3, 9)])),
				Op::FileRename { file: oid(1, 1), path: vec![0xff, 0x2f, 0x00] },
			),
			Record::new(
				res!(Header::new(oid(3, 11), vec![oid(3, 10)])),
				Op::FileMode { file: oid(1, 1), mode: Mode::Executable },
			),
			Record::new(
				res!(Header::new(oid(3, 12), vec![oid(3, 11)])),
				Op::FileDelete { file: oid(1, 1) },
			),
			Record::new(
				res!(Header::new(oid(3, 13), vec![oid(3, 12)])),
				Op::Mark { name: fmt!("release-caf\u{e9}"), body: None, time: None },
			),
			Record::new(
				res!(Header::new(oid(4, 14), vec![oid(3, 13)])),
				Op::Note {
					on:		vec![res!(ContentRange::new(oid(1, 2), 4, 9))],
					text:	b"the fox is doing the work here".to_vec(),
				},
			),
		])
	}

	fn bare() -> Outcome<Vec<Entry>> {
		Ok(res!(records()).into_iter().map(Entry::Bare).collect())
	}

	/// Those records sealed under a stand-in signer, and the signer.
	fn sealed()
		-> Outcome<(Vec<Entry>, StubSigner)>
	{
		let s = StubSigner::with_seed(19);
		let mut out = Vec::new();
		for rec in res!(records()) {
			out.push(Entry::Sealed(res!(Envelope::seal_record(&s, &rec))));
		}
		Ok((out, s))
	}

	#[test]
	fn bare_records_round_trip() -> Outcome<()> {
		let entries = res!(bare());
		let head = Head::new(Some(ReplicaId::new(7)));
		let bytes = res!(encode(&head, &entries, Fold, [0u8; 0]));
		let (got_head, got) = res!(decode(&bytes, Fold, [0u8; 0]));
		assert_eq!(got_head, head);
		assert_eq!(got, entries);
		Ok(())
	}

	/// Reads `want` entries from the front of a segment and says how many bytes
	/// they took, which is the only place a byte offset into a segment may come
	/// from.
	fn up_to(bytes: &[u8], want: usize)
		-> Outcome<(Head, Vec<Entry>, Vec<u8>, usize)>
	{
		// The header is read on the way to the first record, so a caller that
		// wants none of them has to read it for itself.
		if want == 0 {
			let (head, used) = res!(res!(Head::decode(bytes)).ok_or_else(|| err!(
				"The segment yielded no header."; Bug, Missing)));
			return Ok((head, Vec::new(), Vec::new(), used));
		}
		let mut reader: Reader<Fold, 0> = Reader::tallying(Fold, [0u8; 0]);
		reader.feed(bytes);
		let mut got = Vec::new();
		while got.len() < want {
			match res!(reader.next_entry()) {
				Some(entry)	=> got.push(entry),
				None		=> break,
			}
		}
		let head = res!(reader.head().ok_or_else(|| err!(
			"The segment yielded no header."; Bug, Missing)));
		let at = bytes.len() - reader.remaining().len();
		Ok((*head, got, reader.take_digests(), at))
	}

	#[test]
	fn a_reader_taken_up_part_way_yields_what_a_whole_read_yields() -> Outcome<()> {
		let entries = res!(bare());
		let head = Head::new(Some(ReplicaId::new(7)));
		let bytes = res!(encode(&head, &entries, Fold, [0u8; 0]));
		let (_, whole, all_digests, _) = res!(up_to(&bytes, entries.len() + 1));
		assert_eq!(whole, entries);
		// Every boundary, so that no one lucky stopping place carries the test.
		for stopped in 0..entries.len() {
			let (got_head, first, first_digests, at) = res!(up_to(&bytes, stopped));
			assert_eq!(got_head, head);
			let mut reader: Reader<Fold, 0> = Reader::tallying(Fold, [0u8; 0]);
			reader.take_up(got_head, stopped);
			reader.feed(&bytes[at..]);
			reader.end();
			let mut rest = Vec::new();
			while let Some(entry) = res!(reader.next_entry()) {
				rest.push(entry);
			}
			let mut joined = first;
			joined.extend(rest);
			assert_eq!(joined, entries,
				"a read stopped after {} entries and taken up again lost or changed \
				something", stopped);
			let mut digests = first_digests;
			digests.extend(reader.take_digests());
			assert_eq!(digests, all_digests,
				"the digests of a read stopped after {} entries and taken up again are \
				not the digests of one read", stopped);
			assert_eq!(reader.count(), entries.len(),
				"a reader taken up at {} did not end at the count the file holds", stopped);
		}
		Ok(())
	}

	#[test]
	fn a_reader_taken_up_names_a_damaged_record_by_its_place_in_the_file() -> Outcome<()> {
		let entries = res!(bare());
		let bytes = res!(encode(&Head::new(None), &entries, Fold, [0u8; 0]));
		const STOPPED: usize = 4;
		let (head, _, _, at) = res!(up_to(&bytes, STOPPED));
		let mut damaged = bytes.to_vec();
		// Into the body of the record that begins there: past its kind byte and
		// the varint that gives its length.
		damaged[at + 3] ^= 0xff;
		let mut reader: Reader<Fold, 0> = Reader::new(Fold, [0u8; 0]);
		reader.take_up(head, STOPPED);
		reader.feed(&damaged[at..]);
		reader.end();
		match reader.next_entry() {
			Ok(_) => Err(err!(
				"A record whose body was altered was read as though it were sound.";
			Test, Invalid)),
			Err(e) => {
				let said = fmt!("{}", e);
				assert!(said.contains(&fmt!("Record {} of the segment", STOPPED)),
					"the message names the record by its place in the read rather than \
					in the file: {}", said);
				Ok(())
			},
		}
	}

	#[test]
	fn sealed_records_round_trip_and_still_verify() -> Outcome<()> {
		let (entries, signer) = res!(sealed());
		let bytes = res!(encode(&Head::new(None), &entries, Fold, [0u8; 0]));
		let (head, got) = res!(decode(&bytes, Fold, [0u8; 0]));
		assert_eq!(head.replica, None);
		assert_eq!(got, entries);
		for entry in &got {
			match entry {
				Entry::Sealed(e) => {
					assert!(res!(e.verify(&signer)));
					assert!(res!(e.open_record(&signer)).parents().len() <= 2);
				},
				other => return Err(err!(
					"Expected a sealed envelope, got a {}.", other.name();
				Test, Mismatch)),
			}
		}
		Ok(())
	}

	#[test]
	fn both_forms_mix_in_one_segment() -> Outcome<()> {
		let (mut entries, _) = res!(sealed());
		entries.truncate(2);
		entries.extend(res!(bare()));
		let bytes = res!(encode(&Head::new(None), &entries, Fold, [0u8; 0]));
		let (_, got) = res!(decode(&bytes, Fold, [0u8; 0]));
		assert_eq!(got, entries);
		assert_eq!(got[0].kind(), KIND_SEALED);
		assert_eq!(got[2].kind(), KIND_BARE);
		Ok(())
	}

	/// Both forms survive the tagged daticle round trip, which is what a carrier
	/// that is itself a daticle uses, and a tag that is neither is refused.
	#[test]
	fn entries_round_trip_as_daticles() -> Outcome<()> {
		let (mut entries, _) = res!(sealed());
		entries.extend(res!(bare()));
		for entry in &entries {
			assert_eq!(&res!(Entry::from_dat(&entry.to_dat())), entry);
		}
		let odd = Dat::List(vec![Dat::U8(9), Dat::List(Vec::new())]);
		assert!(Entry::from_dat(&odd).is_err());
		assert!(Entry::from_dat(&Dat::List(Vec::new())).is_err());
		Ok(())
	}

	#[test]
	fn an_empty_segment_is_just_a_header() -> Outcome<()> {
		let head = Head::new(Some(ReplicaId::new(0)));
		let bytes = res!(encode(&head, &[], Fold, [0u8; 0]));
		assert_eq!(bytes, head.encode());
		let (got_head, got) = res!(decode(&bytes, Fold, [0u8; 0]));
		assert_eq!(got_head, head);
		assert!(got.is_empty());
		Ok(())
	}

	#[test]
	fn a_byte_at_a_time_reads_the_same() -> Outcome<()> {
		let entries = res!(bare());
		let bytes = res!(encode(&Head::new(None), &entries, Fold, [0u8; 0]));
		let mut reader: Reader<Fold, 0> = Reader::new(Fold, [0u8; 0]);
		let mut got: Vec<Entry> = Vec::new();
		for b in &bytes {
			reader.feed(&[*b]);
			while let Some(entry) = res!(reader.next_entry()) {
				got.push(entry);
			}
		}
		reader.end();
		while let Some(entry) = res!(reader.next_entry()) {
			got.push(entry);
		}
		assert_eq!(got, entries);
		assert!(reader.is_exhausted());
		assert_eq!(reader.count(), entries.len());
		Ok(())
	}

	/// A tallying reader hands back the digests it checked, in order, and hands
	/// back exactly those.
	///
	/// The oracle is the segment itself: every digest the reader reports must be
	/// found in the encoded bytes, and at a higher offset than the one before it.
	/// That is independent of how the reader computed them, which is what makes
	/// it worth asserting -- a tally built by hashing something else, or built in
	/// the wrong order, or one digest short, fails it.
	#[test]
	fn a_tallying_reader_reports_the_digests_it_checked() -> Outcome<()> {
		let entries = res!(bare());
		let bytes = res!(encode(&Head::new(None), &entries, Fold, [0u8; 0]));

		// Taken once at the end, so that the take holds every digest at once and
		// the order they come back in is a thing this can be wrong about. An
		// earlier version took after every record, and a take of one digest is in
		// order whatever the reader does with it.
		let mut whole: Reader<Fold, 0> = Reader::tallying(Fold, [0u8; 0]);
		whole.feed(&bytes);
		whole.end();
		while let Some(_) = res!(whole.next_entry()) {}
		let tally = whole.take_digests();
		assert_eq!(tally.len(), entries.len() * 8, "one eight byte digest per record");
		assert!(whole.take_digests().is_empty(), "and a second take has nothing left");
		assert!(entries.len() > 2, "the fixture must hold enough records to be out of order");

		let mut at = 0usize;
		for (i, digest) in tally.chunks(8).enumerate() {
			let found = match bytes[at..].windows(8).position(|w| w == digest) {
				Some(p)	=> at + p,
				None	=> return Err(err!(
					"The digest reported for record {} is not in the segment after \
					offset {}.", i, at; Test, Mismatch)),
			};
			at = found + 1;
		}

		// The chunking the bytes arrive in changes nothing, as it changes nothing
		// about the records, and neither does draining the tally as it fills,
		// which is what a reader working a batch at a time does.
		let mut dribbled: Reader<Fold, 0> = Reader::tallying(Fold, [0u8; 0]);
		let mut slow = Vec::new();
		for b in &bytes {
			dribbled.feed(&[*b]);
			while let Some(_) = res!(dribbled.next_entry()) {
				slow.extend_from_slice(&dribbled.take_digests());
			}
		}
		dribbled.end();
		while let Some(_) = res!(dribbled.next_entry()) {
			slow.extend_from_slice(&dribbled.take_digests());
		}
		slow.extend_from_slice(&dribbled.take_digests());
		assert_eq!(slow, tally, "a byte at a time tallies what a mouthful tallies");

		// A reader nobody asked reports nothing, and reads the same records.
		let mut plain: Reader<Fold, 0> = Reader::new(Fold, [0u8; 0]);
		plain.feed(&bytes);
		plain.end();
		let mut got = Vec::new();
		while let Some(entry) = res!(plain.next_entry()) {
			got.push(entry);
			assert!(plain.take_digests().is_empty(), "and keeps nothing on the way");
		}
		assert_eq!(got, entries);
		Ok(())
	}

	/// **The invariant packing rests on**: a packed segment yields the same
	/// records, with the same digests, in the same order, as the plain segment it
	/// was made from.
	///
	/// A fold over those digests is what `ore repack` compares two stores by, so
	/// if this were not exact a compressed store and an uncompressed one carrying
	/// one history would disagree about their own shape, and packing would stop
	/// being revocable. The digests are compared as well as the entries, because
	/// the entries could agree while the framing they were checked against did
	/// not.
	#[test]
	fn a_packed_segment_yields_what_the_plain_one_yields() -> Outcome<()> {
		let entries = res!(bare());
		assert!(entries.len() > 2, "the fixture holds enough records to be a run");
		let head = Head::new(Some(ReplicaId::new(4)));

		let plain = res!(encode(&head, &entries, Fold, [0u8; 0]));
		let mut writer: Writer<Fold, 0> = Writer::new(&head, Fold, [0u8; 0]);
		res!(writer.push_packed(&entries));
		let packed = writer.finish();
		assert_ne!(plain, packed, "the two are not the same bytes");

		let read = |bytes: &[u8]| -> Outcome<(Vec<Entry>, Vec<u8>, usize)> {
			let mut reader: Reader<Fold, 0> = Reader::tallying(Fold, [0u8; 0]);
			reader.feed(bytes);
			reader.end();
			let mut got = Vec::new();
			while let Some(entry) = res!(reader.next_entry()) {
				got.push(entry);
			}
			assert!(reader.is_exhausted(), "every byte became a record");
			let tally = reader.take_digests();
			Ok((got, tally, reader.count()))
		};
		let (plain_entries, plain_tally, plain_count) = res!(read(&plain));
		let (packed_entries, packed_tally, packed_count) = res!(read(&packed));

		assert_eq!(plain_entries, entries, "the plain segment reads back");
		assert_eq!(packed_entries, entries, "and so does the packed one");
		assert_eq!(packed_count, plain_count, "the same number of records");
		assert_eq!(packed_count, entries.len(), "which is the number that went in");
		assert_eq!(packed_tally, plain_tally,
			"and the same digests in the same order, which is what a fold over a log \
			names and what makes packing revocable");
		assert!(!packed_tally.is_empty(), "the fixture really tallied something");
		Ok(())
	}

	/// A run's own digest is over the compressed bytes and is NOT among the
	/// digests a reader tallies.
	///
	/// Stated separately because it is the part that would be easy to get right
	/// by accident and wrong on the next change: one packed record yields several
	/// records, and what a fold wants is the several.
	#[test]
	fn the_runs_own_digest_is_not_tallied() -> Outcome<()> {
		let entries = res!(bare());
		let head = Head::new(None);
		let mut writer: Writer<Fold, 0> = Writer::new(&head, Fold, [0u8; 0]);
		res!(writer.push_packed(&entries));
		let packed = writer.finish();

		let mut reader: Reader<Fold, 0> = Reader::tallying(Fold, [0u8; 0]);
		reader.feed(&packed);
		reader.end();
		while res!(reader.next_entry()).is_some() {}
		let tally = reader.take_digests();
		assert_eq!(tally.len(), entries.len() * 8,
			"one eight byte digest per RECORD, not one for the run");
		Ok(())
	}

	/// The framing around a packed run is fixed, and its payload is not frozen.
	///
	/// There is no golden byte array here on purpose. A packed run's payload is
	/// what a compressor made of the records, so freezing it would freeze a
	/// dependency version and call it a format: the first `cargo update` that
	/// moved `miniz_oxide` would redden it, with nothing about Ore having
	/// changed. What a reader elsewhere must agree about is the framing, and that
	/// is what is asserted -- the magic, the declared version, the kind byte, and
	/// that the length and the digest that follow describe what is there.
	#[test]
	fn the_packed_framing_is_fixed() -> Outcome<()> {
		let entries = res!(bare());
		let head = Head::new(None);
		let mut writer: Writer<Fold, 0> = Writer::new(&head, Fold, [0u8; 0]);
		res!(writer.push_packed(&entries));
		let bytes = writer.finish();

		assert_eq!(&bytes[..MAGIC.len()], &MAGIC[..], "a segment begins with the magic");
		assert_eq!(bytes[6], VERSION, "the version sits where it always has");
		assert_eq!(bytes[7], 0, "no replica hint follows");
		assert_eq!(bytes[8], KIND_PACKED, "and the record says it is a run");

		// The length, the payload and the digest, read the way a reader reads them.
		let (len, used) = res!(varint_decode(&bytes[9..]));
		let at = 9 + used;
		let body = &bytes[at..at + len as usize];
		let (dlen, used) = res!(varint_decode(&bytes[at + len as usize..]));
		let dat = at + len as usize + used;
		assert_eq!(bytes.len(), dat + dlen as usize, "and nothing after the digest");
		let want = Fold.hash(&[&[KIND_PACKED], body], [0u8; 0]).as_vec();
		assert_eq!(&bytes[dat..], &want[..],
			"the digest is over the compressed bytes, which is what catches damage \
			before anything is inflated");
		Ok(())
	}

	/// A run inside a run is refused.
	#[test]
	fn a_run_inside_a_run_is_refused() -> Outcome<()> {
		let entries = res!(bare());
		let head = Head::new(None);
		// A run of records, packed, and then that whole framing packed again as if
		// it were a run of its own.
		let mut inner: Writer<Fold, 0> = Writer::new(&head, Fold, [0u8; 0]);
		res!(inner.push_packed(&entries));
		let once = inner.finish();
		let framing = &once[Head::new(None).encode().len()..];

		let mut outer: Vec<u8> = Head::new(None).encode();
		let body = res!(deflate(framing));
		let digest = Fold.hash(&[&[KIND_PACKED], &body], [0u8; 0]).as_vec();
		outer.push(KIND_PACKED);
		varint_encode(body.len() as u64, &mut outer);
		outer.extend_from_slice(&body);
		varint_encode(digest.len() as u64, &mut outer);
		outer.extend_from_slice(&digest);

		let mut reader: Reader<Fold, 0> = Reader::new(Fold, [0u8; 0]);
		reader.feed(&outer);
		reader.end();
		let said = match reader.next_entry() {
			Ok(_)	=> return Err(err!("A run inside a run was read."; Test, Invalid)),
			Err(e)	=> fmt!("{}", e.plain()),
		};
		assert!(said.contains("itself packed"), "and says so: {}", said);
		Ok(())
	}

	/// A damaged run is named by its digest, before a byte of it is inflated.
	#[test]
	fn a_damaged_run_is_named_and_never_inflated() -> Outcome<()> {
		let entries = res!(bare());
		let head = Head::new(None);
		let mut writer: Writer<Fold, 0> = Writer::new(&head, Fold, [0u8; 0]);
		res!(writer.push_packed(&entries));
		let good = writer.finish();

		// Every single-byte change to the compressed payload, which is where damage
		// lands: each is caught, and none of them reaches the decompressor.
		let at = 9 + res!(varint_decode(&good[9..])).1;
		let len = res!(varint_decode(&good[9..])).0 as usize;
		assert!(len > 4, "there is a payload to damage");
		for i in [at, at + 1, at + len / 2, at + len - 1] {
			let mut bad = good.clone();
			bad[i] ^= 0x01;
			let mut reader: Reader<Fold, 0> = Reader::new(Fold, [0u8; 0]);
			reader.feed(&bad);
			reader.end();
			let said = match reader.next_entry() {
				Ok(_)	=> return Err(err!(
					"A run damaged at byte {} was read.", i; Test, Invalid)),
				Err(e)	=> fmt!("{}", e.plain()),
			};
			assert!(said.contains("fails its integrity check"),
				"damage at {} is caught by the digest, not by the decompressor: {}",
				i, said);
			assert!(said.contains("a run of packed operations"),
				"and the message says what the record was: {}", said);
		}
		Ok(())
	}

	/// A vouched read yields exactly what a checked read yields, for every shape
	/// of segment there is.
	///
	/// This is the whole of what the gate may change: which bodies get hashed.
	/// The records that come out, the order they come out in, the header and the
	/// tally a fold is built from must all be the same bytes, because a caller
	/// switching modes is not asking for a different history. The tally matters
	/// most: a vouched read hands back the digest it found rather than one it
	/// computed, and if those two ever differed on sound bytes then every verdict
	/// ever filed would miss.
	#[test]
	fn a_vouched_read_yields_what_a_checked_read_yields() -> Outcome<()> {
		let entries = res!(bare());
		let (signed, _) = res!(sealed());
		let head = Head::new(Some(ReplicaId::new(7)));

		let mut mixed: Writer<Fold, 0> = Writer::new(&head, Fold, [0u8; 0]);
		res!(mixed.push(&entries[0]));
		res!(mixed.push_packed(&entries[1..]));
		res!(mixed.push(&signed[0]));
		let shapes = [
			("plain bare",	res!(encode(&head, &entries, Fold, [0u8; 0]))),
			("plain sealed",	res!(encode(&head, &signed, Fold, [0u8; 0]))),
			("packed and plain together",	mixed.finish()),
		];

		for (shape, bytes) in &shapes {
			let mut want: Reader<Fold, 0> = Reader::tallying(Fold, [0u8; 0]);
			let mut got: Reader<Fold, 0> = Reader::tallying(Fold, [0u8; 0])
				.integrity(Integrity::Vouched);
			let (mut want_out, mut got_out) = (Vec::new(), Vec::new());
			want.feed(bytes);
			want.end();
			got.feed(bytes);
			got.end();
			while let Some(entry) = res!(want.next_entry()) {
				want_out.push(entry);
			}
			while let Some(entry) = res!(got.next_entry()) {
				got_out.push(entry);
			}
			assert!(!want_out.is_empty(), "the {} fixture holds records", shape);
			assert_eq!(got_out, want_out, "a vouched read of a {} segment", shape);
			assert_eq!(got.head(), want.head(), "and reads the same header, {}", shape);
			assert_eq!(got.count(), want.count(), "and the same count, {}", shape);
			let tally = want.take_digests();
			assert_eq!(tally.len(), want_out.len() * 8, "one digest per record, {}", shape);
			assert_eq!(got.take_digests(), tally,
				"and the same tally, so a fold over a vouched read is the fold a \
				verdict was filed under, {}", shape);
		}
		Ok(())
	}

	/// **The trade, written down.** A body byte that flips under a vouched read
	/// is handed over as though nothing happened, and a fold cannot see it
	/// either.
	///
	/// The same damage under a checked read is refused by name. Both halves are
	/// asserted here because the pair is the point: this is not a test that the
	/// gate works, it is a test of what the gate costs, and the cost is what
	/// [`crate::segment::Integrity`] tells a caller to go and cover somewhere
	/// else.
	#[test]
	fn a_vouched_read_lets_a_flipped_body_byte_through() -> Outcome<()> {
		let entries = res!(bare());
		let head = Head::new(None);
		let good = res!(encode(&head, &entries, Fold, [0u8; 0]));

		// One letter of a note's text, which is bit rot as it actually reads: a
		// byte that keeps its record the same length and the same shape, so
		// nothing but the digest over it could ever have noticed.
		let at = res!(good.windows(3).position(|w| w == b"fox").ok_or_else(|| err!(
			"The fixture no longer carries the text this test damages."; Test, Missing)));
		let mut bad = good.clone();
		bad[at] ^= 0x20;
		assert_eq!(bad.len(), good.len(), "the damage moved nothing");
		assert_ne!(bad, good, "and really is damage");

		let mut checked: Reader<Fold, 0> = Reader::tallying(Fold, [0u8; 0]);
		checked.feed(&bad);
		checked.end();
		let said = loop {
			match checked.next_entry() {
				Ok(Some(_))	=> (),
				Ok(None)	=> return Err(err!(
					"A checked read took a segment whose body bytes had been changed.";
				Test, Invalid)),
				Err(e)		=> break fmt!("{}", e.plain()),
			}
		};
		assert!(said.contains("fails its integrity check"),
			"a checked read still says what is wrong: {}", said);

		let mut vouched: Reader<Fold, 0> = Reader::tallying(Fold, [0u8; 0])
			.integrity(Integrity::Vouched);
		vouched.feed(&bad);
		vouched.end();
		let mut got = Vec::new();
		while let Some(entry) = res!(vouched.next_entry()) {
			got.push(entry);
		}
		assert_eq!(got.len(), entries.len(),
			"a vouched read hands over every record of a damaged segment");
		assert_ne!(got, entries,
			"and what it hands over is not what was written");

		// And the fold is blind to it, which is the part that decides where the
		// check has to go instead: the digests are read out of the file, so the
		// same file with a changed body folds to what it folded to before.
		let mut sound: Reader<Fold, 0> = Reader::tallying(Fold, [0u8; 0])
			.integrity(Integrity::Vouched);
		sound.feed(&good);
		sound.end();
		while let Some(_) = res!(sound.next_entry()) {}
		assert_eq!(vouched.take_digests(), sound.take_digests(),
			"a damaged body folds to what the sound one folded to, so nothing \
			downstream of the tally can catch this either");
		Ok(())
	}

	/// A vouched read reaches the decompressor with the frame unexamined, and the
	/// bound still refuses a run that would not stop.
	///
	/// [`Integrity::Vouched`] gives up the digest that used to stand in front of
	/// [`inflate`], so the only thing between a hostile frame and the allocator
	/// is [`PACKED_MAX`]. That was true before and is load bearing now.
	#[test]
	fn a_vouched_read_still_refuses_a_run_that_would_not_stop() -> Outcome<()> {
		let big = vec![0u8; PACKED_MAX + 1024];
		let body = res!(deflate(&big));
		let mut bytes = Head::new(None).encode();
		bytes.push(KIND_PACKED);
		varint_encode(body.len() as u64, &mut bytes);
		bytes.extend_from_slice(&body);
		// A digest that is not the frame's, so that nothing here could be passing
		// because the frame happened to check out.
		varint_encode(8, &mut bytes);
		bytes.extend_from_slice(&[0u8; 8]);

		let mut reader: Reader<Fold, 0> = Reader::new(Fold, [0u8; 0])
			.integrity(Integrity::Vouched);
		reader.feed(&bytes);
		reader.end();
		let said = match reader.next_entry() {
			Ok(_)	=> return Err(err!(
				"A vouched read inflated a run past the bound."; Test, Invalid)),
			Err(e)	=> fmt!("{}", e.plain()),
		};
		assert!(said.contains("inflates past"), "and says why: {}", said);
		Ok(())
	}

	/// A run that would not stop inflating is refused rather than allocated for.
	///
	/// The digest holds, so this is not damage: it is a frame somebody wrote to be
	/// obeyed. What refuses it is the bound and nothing else, which is why the
	/// frame is built to be sound in every other respect.
	#[test]
	fn a_run_that_would_not_stop_is_refused() -> Outcome<()> {
		let big = vec![0u8; PACKED_MAX + 1024];
		let body = res!(deflate(&big));
		assert!(body.len() < 1 << 20, "the fixture really is a small frame: {}", body.len());
		let digest = Fold.hash(&[&[KIND_PACKED], &body], [0u8; 0]).as_vec();
		let mut bytes = Head::new(None).encode();
		bytes.push(KIND_PACKED);
		varint_encode(body.len() as u64, &mut bytes);
		bytes.extend_from_slice(&body);
		varint_encode(digest.len() as u64, &mut bytes);
		bytes.extend_from_slice(&digest);

		let mut reader: Reader<Fold, 0> = Reader::new(Fold, [0u8; 0]);
		reader.feed(&bytes);
		reader.end();
		let said = match reader.next_entry() {
			Ok(_)	=> return Err(err!("A run past the bound was inflated."; Test, Invalid)),
			Err(e)	=> fmt!("{}", e.plain()),
		};
		assert!(said.contains("inflates past"), "and says why: {}", said);
		Ok(())
	}

	/// Packed and plain records sit side by side in one segment.
	///
	/// The kind is per record, so a segment is not one thing or the other. This is
	/// what lets a repack pack the sealed part of a log and leave the tail alone.
	#[test]
	fn a_segment_holds_packed_and_plain_together() -> Outcome<()> {
		let entries = res!(bare());
		let head = Head::new(None);
		let mut writer: Writer<Fold, 0> = Writer::new(&head, Fold, [0u8; 0]);
		res!(writer.push(&entries[0]));
		res!(writer.push_packed(&entries[1..]));
		res!(writer.push(&entries[0]));
		let bytes = writer.finish();

		let mut reader: Reader<Fold, 0> = Reader::new(Fold, [0u8; 0]);
		reader.feed(&bytes);
		reader.end();
		let mut got = Vec::new();
		while let Some(entry) = res!(reader.next_entry()) {
			got.push(entry);
		}
		let mut want = vec![entries[0].clone()];
		want.extend_from_slice(&entries[1..]);
		want.push(entries[0].clone());
		assert_eq!(got, want, "in the order they were written");
		assert!(reader.is_exhausted());
		Ok(())
	}

	/// A byte at a time reads a packed segment as a mouthful does.
	#[test]
	fn a_byte_at_a_time_reads_a_packed_segment_the_same() -> Outcome<()> {
		let entries = res!(bare());
		let head = Head::new(None);
		let mut writer: Writer<Fold, 0> = Writer::new(&head, Fold, [0u8; 0]);
		res!(writer.push_packed(&entries));
		let bytes = writer.finish();

		let mut reader: Reader<Fold, 0> = Reader::tallying(Fold, [0u8; 0]);
		let mut got: Vec<Entry> = Vec::new();
		let mut slow = Vec::new();
		for b in &bytes {
			reader.feed(&[*b]);
			while let Some(entry) = res!(reader.next_entry()) {
				got.push(entry);
			}
			slow.extend_from_slice(&reader.take_digests());
		}
		reader.end();
		while let Some(entry) = res!(reader.next_entry()) {
			got.push(entry);
		}
		slow.extend_from_slice(&reader.take_digests());
		assert_eq!(got, entries, "a run only becomes records once all of it has arrived");
		assert_eq!(slow.len(), entries.len() * 8);
		assert!(reader.is_exhausted());
		Ok(())
	}

	/// An empty run is refused at both ends.
	#[test]
	fn an_empty_run_is_refused() -> Outcome<()> {
		let head = Head::new(None);
		let mut writer: Writer<Fold, 0> = Writer::new(&head, Fold, [0u8; 0]);
		let said = match writer.push_packed(&[]) {
			Ok(())	=> return Err(err!("An empty run was written."; Test, Invalid)),
			Err(e)	=> fmt!("{}", e.plain()),
		};
		assert!(said.contains("over no entries"), "and says so: {}", said);

		// And one that inflates to nothing, which is what a reader could meet.
		let body = res!(deflate(&[]));
		let digest = Fold.hash(&[&[KIND_PACKED], &body], [0u8; 0]).as_vec();
		let mut bytes = Head::new(None).encode();
		bytes.push(KIND_PACKED);
		varint_encode(body.len() as u64, &mut bytes);
		bytes.extend_from_slice(&body);
		varint_encode(digest.len() as u64, &mut bytes);
		bytes.extend_from_slice(&digest);
		let mut reader: Reader<Fold, 0> = Reader::new(Fold, [0u8; 0]);
		reader.feed(&bytes);
		reader.end();
		let said = match reader.next_entry() {
			Ok(_)	=> return Err(err!("An empty run was read."; Test, Invalid)),
			Err(e)	=> fmt!("{}", e.plain()),
		};
		assert!(said.contains("inflates to nothing"), "and says so: {}", said);
		Ok(())
	}

	#[test]
	fn the_header_arrives_before_the_records() -> Outcome<()> {
		let head = Head::new(Some(ReplicaId::new(300)));
		let bytes = res!(encode(&head, &res!(bare()), Fold, [0u8; 0]));
		let mut reader: Reader<Fold, 0> = Reader::new(Fold, [0u8; 0]);
		reader.feed(&bytes[..head.encode().len()]);
		assert!(res!(reader.next_entry()).is_none());
		assert_eq!(reader.head(), Some(&head));
		Ok(())
	}

	/// Truncating a segment anywhere is a typed error or a request for more
	/// bytes, never a panic and never a half-read record.
	#[test]
	fn truncation_at_every_offset_is_clean() -> Outcome<()> {
		let entries = res!(bare());
		let bytes = res!(encode(&Head::new(Some(ReplicaId::new(2))), &entries, Fold, [0u8; 0]));
		for cut in 0..bytes.len() {
			// Declared complete: a partial record is an error.
			let mut reader: Reader<Fold, 0> = Reader::new(Fold, [0u8; 0]);
			reader.feed(&bytes[..cut]);
			reader.end();
			let mut whole = 0usize;
			loop {
				match reader.next_entry() {
					Ok(Some(_))				=> whole += 1,
					Ok(None) | Err(_)		=> break,
				}
			}
			assert!(whole < entries.len(), "cut at {} yielded every record", cut);
			// Not yet declared complete: the reader asks for more rather than
			// failing, unless the bytes are already wrong.
			let mut open: Reader<Fold, 0> = Reader::new(Fold, [0u8; 0]);
			open.feed(&bytes[..cut]);
			loop {
				match open.next_entry() {
					Ok(Some(_))	=> {},
					Ok(None)	=> break,
					Err(e) => return Err(err!(e,
						"Cut at {} of {} failed before the segment was declared \
						complete.", cut, bytes.len(); Test)),
				}
			}
		}
		// The whole segment reads every record.
		let (_, got) = res!(decode(&bytes, Fold, [0u8; 0]));
		assert_eq!(got.len(), entries.len());
		Ok(())
	}

	#[test]
	fn a_damaged_record_is_named() -> Outcome<()> {
		let entries = res!(bare());
		let bytes = res!(encode(&Head::new(None), &entries, Fold, [0u8; 0]));
		// Damage a byte of the first record's file name, so that the record still
		// decodes and the error can say which operation is at risk.
		let at = match bytes.windows(5).position(|w| w == b"notes") {
			Some(i) => i,
			None => return Err(err!(
				"The segment does not contain the file name it was built with.";
			Test, Missing)),
		};
		let mut damaged = bytes.clone();
		damaged[at] ^= 0x20;
		let e = match decode(&damaged, Fold, [0u8; 0]) {
			Ok(_) => return Err(err!(
				"A damaged record was accepted."; Test, Mismatch)),
			Err(e) => e,
		};
		let msg = fmt!("{}", e);
		assert!(msg.contains("integrity check"), "message was {:?}", msg);
		assert!(msg.contains("Record 0"), "message was {:?}", msg);
		assert!(msg.contains("r1:1"), "message was {:?}", msg);
		Ok(())
	}

	#[test]
	fn a_damaged_digest_is_caught() -> Outcome<()> {
		let entries = res!(bare());
		let bytes = res!(encode(&Head::new(None), &entries, Fold, [0u8; 0]));
		let mut damaged = bytes.clone();
		let last = damaged.len() - 1;
		damaged[last] ^= 0xff;
		assert!(decode(&damaged, Fold, [0u8; 0]).is_err());
		Ok(())
	}

	/// A kind byte flipped from bare to sealed is caught by the digest, which
	/// covers it.
	#[test]
	fn the_kind_byte_is_covered_by_the_digest() -> Outcome<()> {
		let entries = res!(bare());
		let bytes = res!(encode(&Head::new(None), &entries, Fold, [0u8; 0]));
		let head_len = Head::new(None).encode().len();
		let mut damaged = bytes.clone();
		assert_eq!(damaged[head_len], KIND_BARE);
		damaged[head_len] = KIND_SEALED;
		assert!(decode(&damaged, Fold, [0u8; 0]).is_err());
		Ok(())
	}

	/// Reading with the wrong hasher or the wrong salt is the same failure as
	/// reading damaged bytes, which is what makes the check the caller's to own.
	#[test]
	fn the_hasher_must_match() -> Outcome<()> {
		let entries = res!(bare());
		let bytes = res!(encode(&Head::new(None), &entries, Fold, [0u8; 0]));
		assert!(decode(&bytes, Fold, [1u8; 4]).is_err(), "a different salt");
		assert!(decode(&bytes, (), [0u8; 0]).is_err(), "a different function");
		// Under the identity hasher the digest is the body itself, and that too
		// round trips.
		let identity = res!(encode(&Head::new(None), &entries, (), [0u8; 0]));
		let (_, got) = res!(decode(&identity, (), [0u8; 0]));
		assert_eq!(got, entries);
		assert!(identity.len() > bytes.len(), "the identity digest costs the body twice");
		Ok(())
	}

	#[test]
	fn a_segment_that_is_not_one_is_refused() -> Outcome<()> {
		assert!(decode(b"not a segment at all", Fold, [0u8; 0]).is_err());
		assert!(decode(b"O", Fold, [0u8; 0]).is_err(), "a truncated header");
		assert!(decode(b"X", Fold, [0u8; 0]).is_err(), "a wrong first byte");
		// The right magic at an unknown version is refused, and says so.
		let mut wrong = MAGIC.to_vec();
		wrong.push(VERSION + 1);
		wrong.push(0);
		let e = match decode(&wrong, Fold, [0u8; 0]) {
			Ok(_) => return Err(err!("An unknown version was accepted."; Test)),
			Err(e) => e,
		};
		assert!(fmt!("{}", e).contains("version"), "message was {}", e);
		// A record tagged with none of the kinds is refused, and the refusal names
		// the ones there are. That is the mechanism by which a build made before a
		// form existed meets it: a reader knowing only the bare and sealed kinds
		// says so about a veiled record in exactly these words, which is the whole
		// of what a new entry form owes an old reader.
		let entries = res!(bare());
		let bytes = res!(encode(&Head::new(None), &entries, (), [0u8; 0]));
		let head_len = Head::new(None).encode().len();
		// Under the identity hasher a record's digest is its kind byte and its body
		// again, so the tag is put right in the digest as well. Without that the
		// integrity check refuses the record before the tag is ever looked at,
		// which is what this assertion was quietly testing instead.
		let mut odd = bytes.clone();
		odd[head_len] = 9;
		let (body_len, used) = res!(varint_decode(&odd[head_len + 1..]));
		let after_body = head_len + 1 + used + body_len as usize;
		let (digest_len, used) = res!(varint_decode(&odd[after_body..]));
		assert_eq!(digest_len, body_len + 1, "the identity digest is the kind and the body");
		odd[after_body + used] = 9;
		let e = match decode(&odd, (), [0u8; 0]) {
			Ok(_) => return Err(err!("A record tagged 9 was accepted."; Test)),
			Err(e) => e,
		};
		let msg = fmt!("{}", e);
		for named in ["bare", "sealed", "veiled"] {
			assert!(msg.contains(named),
				"the refusal does not name the {} form: {}", named, msg);
		}
		Ok(())
	}

	/// A version 2 segment reads, a version 1 segment does not, and the refusal
	/// says which versions this reader knows.
	///
	/// This is the half of the version 3 event that costs a repository nothing:
	/// version 2 held codes 1 to 7 and version 3 holds 1 to 8, so every segment
	/// written before the bump means under the new reader exactly what it meant
	/// under the old one. Version 1 is a different matter -- its operations
	/// spelled a file as a path -- and stays refused.
	#[test]
	fn a_version_two_segment_still_reads() -> Outcome<()> {
		// Everything but the FileMode, which is the one operation version 2 has
		// no code for.
		let entries: Vec<Entry> = res!(bare())
			.into_iter()
			.filter(|e| !matches!(e.peek(), Ok(Record { op: Op::FileMode { .. }, .. })))
			.collect();
		let old = Head { version: VERSION_MIN, replica: Some(ReplicaId::new(7)) };
		let bytes = res!(encode(&old, &entries, Fold, [0u8; 0]));
		assert_eq!(bytes[MAGIC.len()], VERSION_MIN, "the segment declares version 2");
		let (head, got) = res!(decode(&bytes, Fold, [0u8; 0]));
		assert_eq!(head, old, "the header reads back at the version it was written");
		assert_eq!(got, entries, "and every record with it");
		// Version 1 is below what this reader knows, and the message says so.
		let mut ancient = bytes.clone();
		ancient[MAGIC.len()] = VERSION_MIN - 1;
		let e = match decode(&ancient, Fold, [0u8; 0]) {
			Ok(_) => return Err(err!("Version 1 was accepted."; Test)),
			Err(e) => e,
		};
		let msg = fmt!("{}", e);
		assert!(msg.contains(&fmt!("{}", VERSION_MIN)), "message was {}", msg);
		assert!(msg.contains(&fmt!("{}", VERSION)), "message was {}", msg);
		Ok(())
	}

	/// A segment declaring an older version will not be given an operation that
	/// version has no code for.
	///
	/// Without this the subset claim would hold of the intention and not of the
	/// bytes: appending a FileMode to a version 2 segment would leave a file
	/// whose header promises a vocabulary its records exceed.
	#[test]
	fn an_old_segment_refuses_a_newer_operation() -> Outcome<()> {
		assert_eq!(highest_code(VERSION_MIN), crate::op::CODE_NOTE);
		assert_eq!(highest_code(3), crate::op::CODE_FILE_MODE);
		assert_eq!(highest_code(4), crate::op::CODE_REVERTS);
		assert_eq!(highest_code(VERSION), crate::op::CODE_AMENDED);
		// Every rung is named above, so a bump that forgot one would be caught here
		// rather than by a segment somebody could not read.
		assert!(highest_code(4) < highest_code(VERSION),
			"the vocabulary grows upwards, and version 5 must admit more than version 4");
		let old = Head { version: VERSION_MIN, replica: None };
		let mode = Entry::Bare(Record::root(
			oid(1, 1),
			Op::FileMode { file: oid(1, 1), mode: Mode::Symlink },
		));
		let mark = Entry::Bare(Record::root(oid(1, 2), Op::Mark { name: fmt!("v1"), body: None, time: None }));
		// Starting one.
		let mut writer: Writer<Fold, 0> = Writer::new(&old, Fold, [0u8; 0]);
		assert_eq!(writer.version(), VERSION_MIN);
		// An operation version 2 does spell goes in without complaint.
		res!(writer.push(&mark));
		let e = match writer.push(&mode) {
			Ok(()) => return Err(err!("A FileMode was written into a version 2 \
				segment."; Test)),
			Err(e) => e,
		};
		let msg = fmt!("{}", e);
		assert!(msg.contains("FileMode"), "message was {}", msg);
		assert!(msg.contains(&fmt!("{}", VERSION_MIN)), "message was {}", msg);
		// And continuing one, which is where a real repository would meet it.
		let bytes = res!(encode(&old, &[mark], Fold, [0u8; 0]));
		let mut writer: Writer<Fold, 0> = res!(Writer::resume(&bytes, Fold, [0u8; 0]));
		assert_eq!(writer.version(), VERSION_MIN);
		assert!(writer.push(&mode).is_err());
		// A segment at the current version takes it.
		let mut writer: Writer<Fold, 0> = Writer::new(&Head::new(None), Fold, [0u8; 0]);
		assert_eq!(writer.version(), VERSION);
		res!(writer.push(&mode));
		Ok(())
	}

	/// A version 4 segment refuses an Amended, and a version 5 one takes it.
	///
	/// The same boundary as the version 3 test below, at the rung this change
	/// added, and it is worth its own test for the reason that one is: version 4
	/// is the version every repository written before this change is sitting in.
	/// Every existing store is therefore a version 4 store, and what a version 4
	/// segment does when handed code 14 is what decides whether an amendment costs
	/// anybody a migration. It does not -- the operation is refused by name and
	/// the caller opens a segment at the current version beside it.
	#[test]
	fn a_version_four_segment_refuses_an_amendment() -> Outcome<()> {
		let v4 = Head { version: 4, replica: None };
		let amended = Op::Amended {
			on:		oid(3, 4),
			title:	fmt!("Say it again"),
			body:	b"and say it better".to_vec(),
			voice:	fmt!("wren"),
			time:	1_755_400_300,
		};
		assert_eq!(amended.code(), crate::op::CODE_AMENDED);
		assert!(amended.code() > highest_code(4),
			"an amendment must sit above the version 4 vocabulary");
		let entry = Entry::Bare(Record::root(oid(9, 1), amended.clone()));
		// An operation version 4 does spell still goes into a version 4 segment, so
		// what follows is a refusal of this operation and not of the segment.
		let settled = Entry::Bare(Record::root(oid(9, 2), Op::Settled {
			on:		oid(3, 4),
			state:	crate::op::Settled::Accepted,
			mark:	None,
			time:	1_755_400_301,
		}));
		let mut writer: Writer<Fold, 0> = Writer::new(&v4, Fold, [0u8; 0]);
		res!(writer.push(&settled));
		let e = match writer.push(&entry) {
			Ok(()) => return Err(err!(
				"An Amended at code {} was written into a version 4 segment.",
				amended.code(); Test)),
			Err(e) => e,
		};
		let msg = fmt!("{}", e);
		assert!(msg.contains("Amended"), "message was {}", msg);
		assert!(msg.contains(&fmt!("{}", amended.code())), "message was {}", msg);
		// And continuing a version 4 segment somebody else wrote, which is where a
		// real repository meets this rather than at a fresh one.
		let bytes = res!(encode(&v4, &[settled], Fold, [0u8; 0]));
		let mut writer: Writer<Fold, 0> = res!(Writer::resume(&bytes, Fold, [0u8; 0]));
		assert_eq!(writer.version(), 4);
		assert!(writer.push(&entry).is_err());
		// A segment at the current version takes it, and reads back what went in.
		let mut writer: Writer<Fold, 0> = Writer::new(&Head::new(None), Fold, [0u8; 0]);
		assert_eq!(writer.version(), VERSION);
		res!(writer.push(&entry));
		let back = res!(Op::from_dat(&amended.to_dat()));
		assert_eq!(back, amended, "an amendment did not survive its own encoding");
		Ok(())
	}

	/// A version 3 segment refuses every operation version 4 added, and takes
	/// every operation version 3 spelled.
	///
	/// This is the mechanism the whole additive design rests on, at the boundary
	/// it was built for. Version 3 is the version every repository written before
	/// this change is sitting in, so what a version 3 segment does when it is
	/// handed a code above 8 is what decides whether the change costs a store a
	/// migration. It does not: the operation is refused, and the caller starts a
	/// segment at the current version rather than writing bytes into a file whose
	/// header promises a smaller vocabulary.
	#[test]
	fn a_version_three_segment_refuses_the_version_four_vocabulary() -> Outcome<()> {
		let v3 = Head { version: 3, replica: None };
		let newer = [
			// A mark carrying a time is the second spelling, at code 9.
			Op::Mark {
				name:	fmt!("v1"),
				body:	None,
				time:	Some(1_755_000_000),
			},
			Op::Proposal {
				title:	fmt!("Carry a body on a mark"),
				body:	b"the case".to_vec(),
				voice:	fmt!("someone"),
				time:	1_755_000_001,
			},
			Op::Said {
				on:		oid(1, 1),
				text:	b"agreed".to_vec(),
				voice:	fmt!("someone else"),
				time:	1_755_000_002,
			},
			Op::Settled {
				on:		oid(1, 1),
				state:	crate::op::Settled::Accepted,
				mark:	None,
				time:	1_755_000_003,
			},
			Op::Reverts { undone: vec![oid(1, 1), oid(2, 1)] },
		];
		for (i, op) in newer.iter().enumerate() {
			let code = op.code();
			assert!(code > highest_code(3), "{} is at code {}", op.name(), code);
			let entry = Entry::Bare(Record::root(oid(1, i as u64 + 1), op.clone()));
			let mut writer: Writer<Fold, 0> = Writer::new(&v3, Fold, [0u8; 0]);
			let e = match writer.push(&entry) {
				Ok(()) => return Err(err!(
					"A {} at code {} was written into a version 3 segment.",
					op.name(), code; Test)),
				Err(e) => e,
			};
			let msg = fmt!("{}", e);
			assert!(msg.contains(op.name()), "message was {}", msg);
			assert!(msg.contains(&fmt!("{}", code)), "message was {}", msg);
			// And a segment at the current version takes it.
			let mut writer: Writer<Fold, 0> = Writer::new(&Head::new(None), Fold, [0u8; 0]);
			res!(writer.push(&entry));
		}
		// The code the whole design turns on, said plainly: an operation at 13 in
		// a segment declaring 3.
		let reverts = Entry::Bare(Record::root(
			oid(9, 1),
			Op::Reverts { undone: vec![oid(1, 1)] },
		));
		assert_eq!(res!(reverts.peek()).op.code(), 13);
		let mut writer: Writer<Fold, 0> = Writer::new(&v3, Fold, [0u8; 0]);
		assert!(writer.push(&reverts).is_err());
		// A mark carrying neither a body nor a time is version 2 vocabulary and
		// goes into a version 3 segment, and a version 2 one, exactly as before.
		let plain = Entry::Bare(Record::root(
			oid(9, 2),
			Op::Mark { name: fmt!("v1"), body: None, time: None },
		));
		assert_eq!(res!(plain.peek()).op.code(), crate::op::CODE_MARK);
		let mut writer: Writer<Fold, 0> = Writer::new(&v3, Fold, [0u8; 0]);
		res!(writer.push(&plain));
		let old = Head { version: VERSION_MIN, replica: None };
		let mut writer: Writer<Fold, 0> = Writer::new(&old, Fold, [0u8; 0]);
		res!(writer.push(&plain));
		Ok(())
	}

	/// The bytes of a one-record segment carrying a FileMode, frozen.
	///
	/// The operation that the version 3 bump exists for, pinned in the encoding
	/// it was added in on 12026-07-30. It is shaped like a FileRename -- a code,
	/// an identifier, a field -- and the field is a single tagged byte.
	///
	/// The version 4 and version 5 bumps each moved the version byte here and
	/// nothing else, which is the point: the operation this test pins was written
	/// in version 3 and is spelled in version 5 by the same bytes, so a segment
	/// full of them needs no migration.
	#[test]
	fn the_file_mode_bytes_are_frozen() -> Outcome<()> {
		let rec = Record::root(oid(1, 1), Op::FileMode {
			file:	oid(1, 1),
			mode:	Mode::Executable,
		});
		let bytes = res!(encode(&Head::new(None), &[Entry::Bare(rec)], Fold, [0u8; 0]));
		let want: &[u8] = &[
			// The magic, version 5, and no replica hint.
			0x4f, 0x52, 0x45, 0x53, 0x45, 0x47,
			0x05,
			0x00,
			// The record: a bare one, and 57 bytes of body.
			0x01,
			0x39,
			// The body is a daticle list of 54 bytes: the header, the operation.
			0x33, 0x21, 0x36,
				// The header, 23 bytes: the identifier r1:1, and no parents.
				0x33, 0x21, 0x17,
					0x33, 0x21, 0x12,
						0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
						0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
					0x33, 0x20,
				// The operation, 25 bytes: the FileMode code 8, the file it
				// names, and the mode, which is 1 for executable.
				0x33, 0x21, 0x19,
					0x0a, 0x08,
					0x33, 0x21, 0x12,
						0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
						0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
					0x0a, 0x01,
			// The digest: eight bytes of the folding hasher, and its length.
			0x08,
			0xd4, 0x4c, 0xdb, 0x41, 0x34, 0x92, 0x1e, 0xe8,
		];
		assert_eq!(bytes, want, "the FileMode encoding has changed");
		let (_, got) = res!(decode(want, Fold, [0u8; 0]));
		assert_eq!(got.len(), 1);
		match res!(got[0].peek()).op {
			Op::FileMode { file, mode } => {
				assert_eq!(file, oid(1, 1));
				assert_eq!(mode, Mode::Executable);
			},
			other => return Err(err!(
				"Expected a FileMode, got a {}.", other.name(); Test, Mismatch)),
		}
		Ok(())
	}

	/// A replica hint of any size survives, and its absence is distinguishable
	/// from a hint of zero.
	#[test]
	fn the_replica_hint_round_trips() -> Outcome<()> {
		for hint in [None, Some(0u64), Some(1), Some(127), Some(128), Some(u64::MAX)] {
			let head = Head::new(hint.map(ReplicaId::new));
			let buf = head.encode();
			match res!(Head::decode(&buf)) {
				Some((got, used)) => {
					assert_eq!(got, head);
					assert_eq!(used, buf.len());
				},
				None => return Err(err!(
					"A whole header of {} bytes was read as a prefix.", buf.len();
				Test, Missing)),
			}
		}
		assert!(Head::new(None) != Head::new(Some(ReplicaId::new(0))));
		Ok(())
	}

	#[test]
	fn the_writer_and_the_convenience_agree() -> Outcome<()> {
		let entries = res!(bare());
		let head = Head::new(Some(ReplicaId::new(4)));
		let mut writer: Writer<Fold, 0> = Writer::new(&head, Fold, [0u8; 0]);
		for entry in &entries {
			res!(writer.push(entry));
		}
		assert_eq!(writer.count(), entries.len());
		assert_eq!(writer.finish(), res!(encode(&head, &entries, Fold, [0u8; 0])));
		Ok(())
	}

	#[test]
	fn a_segment_resumes_where_it_left_off() -> Outcome<()> {
		let entries = res!(bare());
		let head = Head::new(Some(ReplicaId::new(11)));
		// The first go: a header and the first two records.
		let mut writer: Writer<Fold, 0> = Writer::new(&head, Fold, [0u8; 0]);
		res!(writer.extend(&entries[..2]));
		let mut file = writer.finish();
		// The second go, which starts by reading what is already there.
		let mut more: Writer<Fold, 0> = res!(Writer::resume(&file, Fold, [0u8; 0]));
		assert_eq!(more.count(), 2, "the records it was resumed from");
		res!(more.extend(&entries[2..]));
		assert_eq!(more.count(), entries.len());
		let tail = more.finish();
		file.extend_from_slice(&tail);
		// Which is the segment written in one go, byte for byte.
		assert_eq!(file, res!(encode(&head, &entries, Fold, [0u8; 0])));
		let (got_head, got) = res!(decode(&file, Fold, [0u8; 0]));
		assert_eq!(got_head, head);
		assert_eq!(got, entries);
		Ok(())
	}

	#[test]
	fn an_empty_segment_resumes() -> Outcome<()> {
		let head = Head::new(None);
		let mut file = head.encode();
		let mut writer: Writer<Fold, 0> = res!(Writer::resume(&file, Fold, [0u8; 0]));
		assert_eq!(writer.count(), 0);
		let entries = res!(bare());
		res!(writer.push(&entries[0]));
		file.extend_from_slice(&writer.finish());
		let (_, got) = res!(decode(&file, Fold, [0u8; 0]));
		assert_eq!(got, entries[..1]);
		Ok(())
	}

	/// Resuming under a hasher or a salt the segment was not written with is
	/// refused, because appending would leave a segment nobody could read whole.
	#[test]
	fn resuming_a_segment_written_otherwise_is_refused() -> Outcome<()> {
		let entries = res!(bare());
		let bytes = res!(encode(&Head::new(None), &entries, Fold, [0u8; 0]));
		assert!(Writer::<Fold, 4>::resume(&bytes, Fold, [1u8; 4]).is_err(), "a different salt");
		assert!(Writer::<(), 0>::resume(&bytes, (), [0u8; 0]).is_err(), "a different function");
		// A segment left half-written by an interrupted append, which is the
		// failure a resumed writer exists to avoid compounding. Every cut that
		// is not a record boundary is refused, and every cut that is one is a
		// shorter segment and resumes as such.
		let mut ends: Vec<usize> = Vec::new();
		let mut probe: Writer<Fold, 0> = Writer::new(&Head::new(None), Fold, [0u8; 0]);
		ends.push(probe.bytes().len());
		for entry in &entries {
			res!(probe.push(entry));
			ends.push(probe.bytes().len());
		}
		for cut in 1..bytes.len() {
			match ends.iter().position(|e| *e == cut) {
				Some(n) => {
					let w: Writer<Fold, 0> = res!(Writer::resume(&bytes[..cut], Fold, [0u8; 0]));
					assert_eq!(w.count(), n, "a segment of {} records cut at {}", n, cut);
				},
				None => if Writer::<Fold, 0>::resume(&bytes[..cut], Fold, [0u8; 0]).is_ok() {
					return Err(err!(
						"A segment cut at {} of {}, part way through a record, was \
						resumed.", cut, bytes.len();
					Test, Mismatch));
				},
			}
		}
		// And what is not a segment at all, including nothing.
		assert!(Writer::<Fold, 0>::resume(b"", Fold, [0u8; 0]).is_err());
		assert!(Writer::<Fold, 0>::resume(b"not a segment", Fold, [0u8; 0]).is_err());
		let mut wrong = MAGIC.to_vec();
		wrong.push(VERSION + 1);
		wrong.push(0);
		assert!(Writer::<Fold, 0>::resume(&wrong, Fold, [0u8; 0]).is_err(), "another version");
		Ok(())
	}

	#[test]
	fn random_segments_round_trip() -> Outcome<()> {
		// A small linear congruential generator, so a failure can be reproduced.
		let mut state = 0x1234_5678_9abc_def0u64;
		let mut next = move || {
			state = state
				.wrapping_mul(6_364_136_223_846_793_005)
				.wrapping_add(1_442_695_040_888_963_407);
			(state >> 33) as usize
		};
		let signer = StubSigner::with_seed(5);
		for trial in 0..40 {
			let n = next() % 12;
			let mut entries: Vec<Entry> = Vec::new();
			let mut ids: Vec<OpId> = Vec::new();
			for k in 0..n {
				let id = oid((next() % 5) as u64 + 1, k as u64 + 1);
				if ids.contains(&id) {
					continue;
				}
				let mut parents: Vec<OpId> = Vec::new();
				for cand in &ids {
					if next() % 2 == 0 {
						parents.push(*cand);
					}
				}
				let head = res!(Header::new(id, parents));
				let anchored = Some(Anchor::origin(oid((next() % 5) as u64 + 1, 1)));
				let op = match next() % 8 {
					0 => Op::FileCreate { path: fmt!("f{}", next() % 100).into_bytes() },
					1 => Op::Mark { name: fmt!("m{}", next() % 100), body: None, time: None },
					2 => Op::FileRename {
						file:	oid((next() % 5) as u64 + 1, 1),
						path:	vec![(next() % 256) as u8; next() % 40],
					},
					3 => Op::FileDelete { file: oid((next() % 5) as u64 + 1, 1) },
					4 => Op::Splice {
						left:	anchored,
						right:	None,
						remove:	Vec::new(),
						insert:	vec![(next() % 256) as u8; 1 + next() % 700].into(),
					},
					5 => Op::Move {
						src:	vec![res!(ContentRange::new(id, 0, (next() % 50) as u64))],
						left:	anchored,
						right:	None,
					},
					6 => Op::FileMode {
						file:	oid((next() % 5) as u64 + 1, 1),
						mode:	match next() % 3 {
							0	=> Mode::Normal,
							1	=> Mode::Executable,
							_	=> Mode::Symlink,
						},
					},
					_ => Op::Note {
						on:		vec![res!(ContentRange::new(id, 0, (next() % 50) as u64 + 1))],
						text:	fmt!("note {}", next() % 1000).into_bytes(),
					},
				};
				let rec = Record::new(head, op);
				entries.push(if next() % 3 == 0 {
					Entry::Sealed(res!(Envelope::seal_record(&signer, &rec)))
				} else {
					Entry::Bare(rec)
				});
				ids.push(id);
			}
			let head = Head::new(if next() % 2 == 0 {
				Some(ReplicaId::new(next() as u64))
			} else {
				None
			});
			let bytes = res!(encode(&head, &entries, Fold, [0u8; 0]));
			let (got_head, got) = res!(decode(&bytes, Fold, [0u8; 0]));
			assert_eq!(got_head, head, "trial {}", trial);
			assert_eq!(got, entries, "trial {}", trial);
			// And in arbitrary chunks.
			let mut reader: Reader<Fold, 0> = Reader::new(Fold, [0u8; 0]);
			let mut chunked: Vec<Entry> = Vec::new();
			let mut at = 0usize;
			while at < bytes.len() {
				let take = (1 + next() % 37).min(bytes.len() - at);
				reader.feed(&bytes[at..at + take]);
				at += take;
				while let Some(entry) = res!(reader.next_entry()) {
					chunked.push(entry);
				}
			}
			reader.end();
			while let Some(entry) = res!(reader.next_entry()) {
				chunked.push(entry);
			}
			assert_eq!(chunked, entries, "trial {} in chunks", trial);
		}
		Ok(())
	}

	/// The bytes of a one-record segment, frozen.
	///
	/// A format that changes by accident orphans every store already written in
	/// it, and nothing else in this file would notice: every other test encodes
	/// and decodes with the same code. This one is the fixed point. If it fails
	/// and the change was deliberate, the version byte is the thing to raise.
	///
	/// It was raised to 2 for file identity, to 3 on 12026-07-30 for
	/// [`crate::op::Op::FileMode`], and to 4 on 12026-08-17 for the mark's second
	/// spelling and the proposal operations. The record below carries a mark with
	/// neither a body nor a time, whose encoding did not change on any of those
	/// occasions, so the only byte that has ever moved here is the version itself.
	/// That is the whole of each event as the framing sees it: what changed is
	/// which operations may appear inside, and an older segment stays readable
	/// because its operations are a subset of a newer one's.
	///
	/// The seven operation bytes below are therefore the test on the version 4
	/// design rather than a chore it creates. A mark saying nothing beyond its
	/// name is written at code 4 with two elements, as it always was; if it were
	/// re-spelled at code 9, those bytes would move and every mark ever signed
	/// would stop verifying.
	#[test]
	fn the_segment_bytes_are_frozen() -> Outcome<()> {
		let rec = Record::new(
			res!(Header::new(oid(2, 3), vec![oid(1, 7)])),
			Op::Mark { name: fmt!("v1"), body: None, time: None },
		);
		let bytes = res!(encode(
			&Head::new(Some(ReplicaId::new(2))),
			&[Entry::Bare(rec)],
			Fold,
			[0u8; 0],
		));
		let want: &[u8] = &[
			// The segment header: the magic, the version, a hint follows, and the
			// replica it names.
			0x4f, 0x52, 0x45, 0x53, 0x45, 0x47,
			0x05,
			0x01,
			0x02,
			// The record: a bare one, and 61 bytes of body.
			0x01,
			0x3d,
			// The body is a daticle list of 58 bytes: the header, the operation.
			0x33, 0x21, 0x3a,
				// The header, 45 bytes: the identifier, then the parents.
				0x33, 0x21, 0x2d,
					// The identifier r2:3, as two 64-bit integers.
					0x33, 0x21, 0x12,
						0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
						0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03,
					// One parent, r1:7.
					0x33, 0x21, 0x15,
						0x33, 0x21, 0x12,
							0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
							0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07,
				// The operation, 7 bytes: the Mark code, and the name "v1".
				0x33, 0x21, 0x07,
					0x0a, 0x04,
					0x29, 0x21, 0x02, 0x76, 0x31,
			// The digest: eight bytes of the folding hasher, and its length.
			0x08,
			0x1e, 0x1a, 0xbf, 0xae, 0x11, 0xf1, 0xa0, 0xe5,
		];
		assert_eq!(bytes, want, "the segment format has changed");
		// Said again on its own: the mark's ten bytes, a three-byte list header
		// then the code 4 and the name, sitting where they have always sat. The
		// assertion above would catch them moving, but it would report a segment
		// that had changed rather than the thing that had actually gone wrong.
		let mark: &[u8] = &[0x33, 0x21, 0x07, 0x0a, 0x04, 0x29, 0x21, 0x02, 0x76, 0x31];
		assert!(
			bytes.windows(mark.len()).any(|w| w == mark),
			"a mark with neither a body nor a time is no longer written at code 4 \
			with two elements, so every mark ever signed has stopped verifying",
		);
		// And the frozen bytes still read.
		let (_, got) = res!(decode(want, Fold, [0u8; 0]));
		assert_eq!(got.len(), 1);
		assert_eq!(res!(got[0].id()), oid(2, 3));
		Ok(())
	}

	/// The stand-in signer's keys are longer than a single byte length, so the
	/// sealed form exercises the wide byte fields.
	#[test]
	fn a_sealed_entry_carries_its_key() -> Outcome<()> {
		let (entries, _) = res!(sealed());
		match &entries[0] {
			Entry::Sealed(e) => assert!(e.signer().len() > 8),
			other => return Err(err!(
				"Expected a sealed envelope, got a {}.", other.name(); Test, Mismatch)),
		}
		let _ = StubSigner::default().clone_with_keys(None, None);
		Ok(())
	}

	/// A veiled entry hands a carrier the header and nothing else, and gives a
	/// reader with the key back exactly what went in.
	///
	/// This is the whole of the form in one test. The identifier and the parents
	/// are readable without a key, because a carrier has to place the operation;
	/// the content is not, and is not in the segment's bytes to be found by
	/// searching for it; and what unveils is the same sealed envelope, whose
	/// signature still checks out, because nothing was re-encoded on the way.
	#[test]
	fn a_veiled_entry_carries_its_header_and_hides_the_rest() -> Outcome<()> {
		let signer = StubSigner::with_seed(3);
		let cipher = StubCipher::with_seed(11);
		let secret: &[u8] = b"the merger closes on Friday";
		let rec = Record::new(
			res!(Header::new(oid(2, 5), vec![oid(1, 3), oid(1, 4)])),
			Op::Splice {
				left:	Some(Anchor::origin(oid(1, 3))),
				right:	None,
				remove:	Vec::new(),
				insert:	secret.to_vec().into(),
			},
		);
		let plain = Entry::Sealed(res!(Envelope::seal_record(&signer, &rec)));
		let veiled = res!(plain.veil(&cipher));
		assert!(veiled.is_veiled());
		assert_eq!(veiled.kind(), KIND_VEILED);
		assert_eq!(veiled.name(), "veiled record");

		// What a carrier may ask, and what it may not.
		let head = res!(veiled.head());
		assert_eq!(head.id(), oid(2, 5));
		assert_eq!(head.parents(), vec![oid(1, 3), oid(1, 4)]);
		assert_eq!(res!(veiled.id()), oid(2, 5));
		let refused = match veiled.peek() {
			Ok(_) => return Err(err!("A veiled operation was read."; Test, Security)),
			Err(e) => fmt!("{}", e.plain()),
		};
		assert!(refused.contains("r2:5"), "the refusal names the operation: {}", refused);
		assert!(refused.contains("veiled"), "and says what it met: {}", refused);

		// Through a segment, which is what a carrier keeps.
		let bytes = res!(encode(&Head::new(None), &[veiled.clone()], Fold, [0u8; 0]));
		assert!(
			!bytes.windows(secret.len()).any(|w| w == secret),
			"the segment a carrier holds contains the operation's own content",
		);
		let (_, got) = res!(decode(&bytes, Fold, [0u8; 0]));
		assert_eq!(got, vec![veiled]);

		// And a reader with the key gets back what was veiled, signature and all.
		let back = res!(got[0].unveil(&cipher));
		assert_eq!(back, plain);
		match &back {
			Entry::Sealed(env) => assert!(res!(env.verify(&signer)),
				"the signature made before veiling does not check out after"),
			other => return Err(err!(
				"Expected a sealed envelope, got a {}.", other.name(); Test, Mismatch)),
		}
		assert_eq!(res!(back.peek()), rec);
		Ok(())
	}

	/// A carrier that rewrites the clear header is caught by the first reader
	/// holding the key, and told which copy to believe.
	///
	/// Rewriting it is the one thing a carrier can do to a veiled entry, and it is
	/// not nothing: every peer that never holds the key places the operation by the
	/// clear copy. What stops it mattering is that the copy inside is under the
	/// signature and cannot be made to agree.
	#[test]
	fn a_carrier_that_rewrites_the_clear_header_is_caught() -> Outcome<()> {
		let cipher = StubCipher::with_seed(2);
		let rec = Record::new(
			res!(Header::new(oid(1, 2), vec![oid(1, 1)])),
			Op::Mark { name: fmt!("v1"), body: None, time: None },
		);
		let veiled = res!(Entry::Bare(rec.clone()).veil(&cipher));
		assert_eq!(res!(veiled.unveil(&cipher)), Entry::Bare(rec), "sound as it stands");
		// Re-parented in clear, the ciphertext untouched.
		let lying = match &veiled {
			Entry::Veiled(v) => Entry::Veiled(Veiled {
				head:	Header::root(oid(1, 2)),
				body:	v.body.clone(),
			}),
			other => return Err(err!(
				"Expected a veiled record, got a {}.", other.name(); Test, Mismatch)),
		};
		assert!(res!(lying.head()).parents().is_empty(),
			"which is the graph a carrier would have placed it in");
		let caught = match lying.unveil(&cipher) {
			Ok(_) => return Err(err!(
				"A rewritten clear header was accepted."; Test, Security)),
			Err(e) => fmt!("{}", e.plain()),
		};
		assert!(caught.contains("r1:1"),
			"the refusal names the parent that was dropped: {}", caught);
		assert!(caught.contains("believe"),
			"and says which of the two copies to believe: {}", caught);
		Ok(())
	}

	/// A key that is not the one it was veiled under fails by name rather than
	/// returning rubbish.
	#[test]
	fn the_wrong_key_does_not_unveil() -> Outcome<()> {
		let ours = StubCipher::with_seed(1);
		let theirs = StubCipher::with_seed(2);
		let veiled = res!(Entry::Bare(Record::root(
			oid(1, 1),
			Op::FileCreate { path: b"notes.md".to_vec() },
		)).veil(&ours));
		let refused = match veiled.unveil(&theirs) {
			Ok(_) => return Err(err!("Another key unveiled it."; Test, Security)),
			Err(e) => fmt!("{}", e.plain()),
		};
		assert!(refused.contains("r1:1"), "the refusal names the operation: {}", refused);
		assert!(refused.contains("did not decrypt"), "and says what failed: {}", refused);
		Ok(())
	}

	#[test]
	fn veiling_does_not_nest_and_unveiling_wants_a_veil() -> Outcome<()> {
		let cipher = StubCipher::with_seed(7);
		let plain = Entry::Bare(Record::root(
			oid(1, 1),
			Op::Mark { name: fmt!("v1"), body: None, time: None },
		));
		let veiled = res!(plain.veil(&cipher));
		assert!(veiled.veil(&cipher).is_err(), "a veil over a veil hides the header");
		assert!(plain.unveil(&cipher).is_err(), "a bare record has nothing to unveil");
		Ok(())
	}

	/// Veiled entries sit beside plain ones in one segment, and each comes back as
	/// the form it went in as.
	///
	/// A repository does not become veiled all at once: what a replica veils is
	/// what it hands to a carrier, and the segments either end of that hop hold
	/// whatever they were given. So the three forms have to mix, in a segment and
	/// in the daticle form a sync message carries.
	#[test]
	fn a_segment_mixes_veiled_entries_with_plain_ones() -> Outcome<()> {
		let cipher = StubCipher::with_seed(23);
		let (mut entries, _) = res!(sealed());
		entries.extend(res!(bare()));
		let mut mixed: Vec<Entry> = Vec::new();
		for (i, entry) in entries.iter().enumerate() {
			mixed.push(if i % 2 == 0 {
				res!(entry.veil(&cipher))
			} else {
				entry.clone()
			});
		}
		let bytes = res!(encode(&Head::new(None), &mixed, Fold, [0u8; 0]));
		let (_, got) = res!(decode(&bytes, Fold, [0u8; 0]));
		assert_eq!(got, mixed);
		for (i, entry) in got.iter().enumerate() {
			assert_eq!(&res!(Entry::from_dat(&entry.to_dat())), entry,
				"entry {} did not round trip as a daticle", i);
			assert_eq!(res!(entry.id()), res!(entries[i].id()),
				"entry {} is not the operation it was made from", i);
			let back = if entry.is_veiled() {
				res!(entry.unveil(&cipher))
			} else {
				entry.clone()
			};
			assert_eq!(back, entries[i], "entry {} did not come back as itself", i);
		}
		Ok(())
	}

	/// The bytes of a one-record segment carrying a veiled entry, frozen.
	///
	/// Written under the identity encrypter, so what the array pins is the framing
	/// and not somebody's cipher: the kind byte 3, the header in clear, and a
	/// length prefixed body whose bytes here are the inner entry itself and can be
	/// read in the listing. Under a real cipher everything above the body is
	/// identical and the body is noise of the same length plus whatever the scheme
	/// adds to it.
	///
	/// The inner bytes are the same ten that [`the_segment_bytes_are_frozen`] pins
	/// for a mark, at their own offset, which is the point of veiling the tagged
	/// form rather than a re-encoding of it: what a reader with the key gets back
	/// is the entry that was signed, byte for byte.
	#[test]
	fn the_veiled_bytes_are_frozen() -> Outcome<()> {
		let rec = Record::new(
			res!(Header::new(oid(2, 3), vec![oid(1, 7)])),
			Op::Mark { name: fmt!("v1"), body: None, time: None },
		);
		let veiled = res!(Entry::Bare(rec).veil(&()));
		let bytes = res!(encode(&Head::new(None), &[veiled], Fold, [0u8; 0]));
		let want: &[u8] = &[
			// The magic, version 5, and no replica hint.
			0x4f, 0x52, 0x45, 0x53, 0x45, 0x47,
			0x05,
			0x00,
			// The record: a veiled one, and 126 bytes of body.
			0x03,
			0x7e,
			// The body is a daticle list of 123 bytes: the clear header, the
			// ciphertext.
			0x33, 0x21, 0x7b,
				// The header in clear, 45 bytes: the identifier r2:3 and the one
				// parent r1:7. This is the whole of what a carrier is given.
				0x33, 0x21, 0x2d,
					0x33, 0x21, 0x12,
						0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
						0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03,
					0x33, 0x21, 0x15,
						0x33, 0x21, 0x12,
							0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
							0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07,
				// The body, as bytes under a 64-bit length: 66 of them. The length
				// is that wide because an operation is not bounded by 255 bytes and
				// a narrower field would decide the format by the first small one.
				0x47,
				0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x42,
					// Which under the identity encrypter is the inner entry itself,
					// tagged bare and carrying the record whole: the same header
					// again, then the mark at code 4 with the name "v1".
					0x33, 0x21, 0x3f,
						0x0a, 0x01,
						0x33, 0x21, 0x3a,
							0x33, 0x21, 0x2d,
								0x33, 0x21, 0x12,
									0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
									0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03,
								0x33, 0x21, 0x15,
									0x33, 0x21, 0x12,
										0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
										0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07,
							0x33, 0x21, 0x07,
								0x0a, 0x04,
								0x29, 0x21, 0x02, 0x76, 0x31,
			// The digest: eight bytes of the folding hasher, and its length.
			0x08,
			0xf5, 0x26, 0x35, 0x5c, 0x1f, 0x94, 0xed, 0x5a,
		];
		assert_eq!(bytes, want, "the veiled framing has changed");
		// The mark's own ten bytes, sitting inside the ciphertext at their usual
		// offset, which is what veiling the tagged form rather than a re-encoding
		// of it buys: a signature made before the veil holds after it.
		let mark: &[u8] = &[0x33, 0x21, 0x07, 0x0a, 0x04, 0x29, 0x21, 0x02, 0x76, 0x31];
		assert!(
			bytes.windows(mark.len()).any(|w| w == mark),
			"the entry a veil is put around is no longer the entry that was written",
		);
		// And the frozen bytes still read, and still unveil.
		let (_, got) = res!(decode(want, Fold, [0u8; 0]));
		assert_eq!(got.len(), 1);
		assert!(got[0].is_veiled());
		assert_eq!(res!(got[0].id()), oid(2, 3));
		assert_eq!(res!(res!(got[0].unveil(&())).peek()).head.parents(), vec![oid(1, 7)]);
		Ok(())
	}

	/// Every shape an entry takes, measured and then encoded, and the two numbers
	/// compared.
	///
	/// This is the only test [`Entry::dat_len`] can have. It is a claim about
	/// bytes nobody built, and a carrier that believes it one byte short puts a
	/// reply past a bound it published -- which is a proxy closing a connection
	/// rather than a number being slightly wrong. So the corpus is every
	/// operation variant, in each of the three entry forms, and the payload sizes
	/// that move the compact length prefixes: nothing at all, one byte, and the
	/// 22,153,680 byte operation fe2o3's own history holds.
	///
	/// Proved red by adding one to the answer, and again by measuring the body
	/// alone rather than the tagged form a message carries, which is the very
	/// confusion between the two forms the doc above warns about.
	#[test]
	fn dat_len_is_what_the_entry_encodes_to() -> Outcome<()> {
		let signer = StubSigner::with_seed(3);
		let cipher = StubCipher::with_seed(9);
		let head = res!(Header::new(oid(5, 7), vec![oid(1, 1), oid(2, 2)]));

		let mut ops: Vec<(String, Op)> = samples()
			.into_iter()
			.enumerate()
			.map(|(i, op)| (fmt!("sample {} ({})", i, op.name()), op))
			.collect();
		let payloads: [(&str, usize); 3] = [
			("an empty payload",		0),
			("a one byte payload",		1),
			("the 22,153,680 byte one",	22_153_680),
		];
		for (name, len) in payloads {
			ops.push((name.to_string(), Op::Splice {
				left:	Some(Anchor::origin(oid(1, 1))),
				right:	None,
				remove:	Vec::new(),
				insert:	vec![0x5a; len].into(),
			}));
		}
		assert!(ops.len() > 30, "the corpus is {} operations, which is not every shape", ops.len());

		let mut seen = std::collections::BTreeSet::new();
		for (name, op) in ops {
			seen.insert(op.code());
			let rec = Record::new(head.clone(), op);
			let bare = Entry::Bare(rec.clone());
			let sealed = Entry::Sealed(res!(Envelope::seal_record(&signer, &rec)));
			let veiled = res!(bare.veil(&cipher));
			for (form, entry) in [("bare", bare), ("sealed", sealed), ("veiled", veiled)] {
				let said = res!(entry.dat_len());
				let wrote = res!(entry.to_dat().to_bytes(Vec::new())).len();
				assert_eq!(said, wrote,
					"{}, {}: measured at {} bytes and encoded to {}", name, form, said, wrote);
			}
		}
		assert_eq!(seen.len(), highest_code(VERSION) as usize,
			"the corpus covers {} of the {} operation codes this version writes",
			seen.len(), highest_code(VERSION));
		Ok(())
	}
}
