//! An append-only run of operations, as bytes.
//!
//! A segment is the durable shape of a stretch of history: a short header
//! saying what the bytes are, then one length-prefixed record after another,
//! each with an integrity check. Appending is writing at the end, and nothing
//! already written is ever revisited, so a writer needs no index and a reader
//! needs no seek.
//!
//! Records come in two forms and the format carries both, tagged. A bare
//! [`Record`] is what a replica writes for itself, where provenance is not in
//! question; an [`Envelope`] is the same record with a public key and a
//! signature around it, which is what crosses between parties. A segment may
//! hold either or both, so a repository that starts unsigned and later gains
//! signatures does not need a second format.
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
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::envelope::Envelope;
use crate::id::{
	varint_decode,
	varint_encode,
	OpId,
	ReplicaId,
	VARINT_MAX_LEN,
};
use crate::op::Record;

use oxedyne_fe2o3_core::prelude::*;
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
pub const VERSION: u8 = 4;

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
/// [`crate::op::CODE_REVERTS`]. A writer continuing a segment somebody else wrote
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
	} else {
		crate::op::CODE_REVERTS
	}
}

/// Kind byte of a record carrying a bare [`Record`].
pub const KIND_BARE:	u8 = 1;
/// Kind byte of a record carrying a signed [`Envelope`].
pub const KIND_SEALED:	u8 = 2;

// How much consumed prefix a reader tolerates before it moves the remainder to
// the front of its buffer.
const COMPACT_THRESHOLD: usize = 1 << 16;


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


/// One record of a segment: an operation, with or without its provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Entry {
	/// An operation written down as it stands.
	Bare(Record),
	/// An operation with a public key and a signature around it.
	Sealed(Envelope),
}

impl Entry {
	/// Returns the kind byte identifying the form.
	pub fn kind(&self) -> u8 {
		match self {
			Self::Bare(_)	=> KIND_BARE,
			Self::Sealed(_)	=> KIND_SEALED,
		}
	}

	/// The form's name, for messages.
	pub fn name(&self) -> &'static str {
		match self {
			Self::Bare(_)	=> "bare record",
			Self::Sealed(_)	=> "sealed envelope",
		}
	}

	/// Opens a sealed record without checking its signature.
	///
	/// Verification is the caller's to do, with the scheme the caller holds; a
	/// segment reader has no key material and makes no claim about provenance.
	pub fn peek(&self)
		-> Outcome<Record>
	{
		match self {
			Self::Bare(rec)	=> Ok(rec.clone()),
			Self::Sealed(e)	=> e.peek_record(),
		}
	}

	pub fn id(&self)
		-> Outcome<OpId>
	{
		Ok(res!(self.peek()).head.id())
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
			other => Err(err!(
				"An Entry is tagged {}, which is neither {} for a bare record nor {} \
				for a sealed envelope.", other, KIND_BARE, KIND_SEALED;
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
			other => Err(err!(
				"A segment record is tagged {}, which is neither {} for a bare record \
				nor {} for a sealed envelope.", other, KIND_BARE, KIND_SEALED;
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
	pub fn push(&mut self, entry: &Entry)
		-> Outcome<()>
	{
		if self.version < VERSION {
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
		let kind = entry.kind();
		let body = res!(entry.body());
		let digest = self.hasher.clone().hash(&[&[kind], &body], self.salt).as_vec();
		self.buf.push(kind);
		varint_encode(body.len() as u64, &mut self.buf);
		self.buf.extend_from_slice(&body);
		varint_encode(digest.len() as u64, &mut self.buf);
		self.buf.extend_from_slice(&digest);
		self.count += 1;
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
	hasher:	H,				// hash function each record's digest is checked with
	salt:	[u8; S],		// salt each digest is checked under
	buf:	Vec<u8>,		// bytes fed but not turned into records, consumed prefix included
	pos:	usize,			// how much of `buf` has been consumed
	eof:	bool,			// whether the caller has declared the segment complete
	head:	Option<Head>,	// the header, once it has been read
	count:	usize,			// records handed over
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
		self.eof && self.pos >= self.buf.len()
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
		if self.pos >= self.buf.len() {
			return Ok(None);
		}
		match res!(self.read_entry()) {
			Some((entry, used)) => {
				self.pos += used;
				self.count += 1;
				self.compact();
				Ok(Some(entry))
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

	/// `None` unless a whole record is there.
	fn read_entry(&self)
		-> Outcome<Option<(Entry, usize)>>
	{
		let buf = &self.buf[self.pos..];
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
				"Record {} of the segment declares a body of {} bytes, which no \
				buffer can hold.", self.count, len;
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
				"Record {} of the segment declares a digest of {} bytes, which no \
				buffer can hold.", self.count, dlen;
			Decode, Input, Excessive)),
		};
		if buf.len() < digest_end {
			return Ok(None);
		}
		let digest = &buf[at..digest_end];
		let want = self.hasher.clone().hash(&[&[kind], body], self.salt).as_vec();
		if want != digest {
			// Naming the operation is worth a decode attempt, since a caller with a
			// damaged segment wants to know which edit is at risk. Where the body is
			// too far gone to decode, the ordinal is all there is to say.
			let named = match Entry::from_body(kind, body) {
				Ok(entry) => match entry.id() {
					Ok(id)	=> fmt!("the operation {}", id),
					Err(_)	=> fmt!("an unreadable operation"),
				},
				Err(_) => fmt!("an unreadable operation"),
			};
			return Err(err!(
				"Record {} of the segment, carrying {}, fails its integrity check: \
				{} bytes of body hash to {:02x?}, and {:02x?} was recorded.",
				self.count, named, body.len(), want, digest;
			Decode, Input, Checksum, Mismatch));
		}
		let entry = res!(Entry::from_body(kind, body));
		Ok(Some((entry, digest_end)))
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
	use crate::test_support::{
		Fold,
		StubSigner,
	};

	use oxedyne_fe2o3_iop_crypto::keys::KeyManager;

	fn oid(replica: u64, counter: u64) -> OpId {
		OpId::new(ReplicaId::new(replica), counter)
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
					insert:	b"the quick brown fox".to_vec(),
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
					insert:	vec![0x2a; 900],	// beyond a single byte length
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
		// A record tagged with neither kind is refused.
		let entries = res!(bare());
		let bytes = res!(encode(&Head::new(None), &entries, (), [0u8; 0]));
		let head_len = Head::new(None).encode().len();
		let mut odd = bytes.clone();
		odd[head_len] = 9;
		assert!(decode(&odd, (), [0u8; 0]).is_err());
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
		assert_eq!(highest_code(VERSION), crate::op::CODE_REVERTS);
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
	/// The version 4 bump moved the version byte here and nothing else, which is
	/// the point: the operation this test pins was written in version 3 and is
	/// spelled in version 4 by the same bytes, so a segment full of them needs no
	/// migration.
	#[test]
	fn the_file_mode_bytes_are_frozen() -> Outcome<()> {
		let rec = Record::root(oid(1, 1), Op::FileMode {
			file:	oid(1, 1),
			mode:	Mode::Executable,
		});
		let bytes = res!(encode(&Head::new(None), &[Entry::Bare(rec)], Fold, [0u8; 0]));
		let want: &[u8] = &[
			// The magic, version 4, and no replica hint.
			0x4f, 0x52, 0x45, 0x53, 0x45, 0x47,
			0x04,
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
						insert:	vec![(next() % 256) as u8; 1 + next() % 700],
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
			0x04,
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
}
