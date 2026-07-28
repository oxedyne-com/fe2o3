//! The operation vocabulary: what a single unit of history can say.
//!
//! History here is a sequence of operations rather than a sequence of
//! snapshots. An operation states an intent -- create this file, put these
//! bytes beside those -- so the intent survives into the record and can be
//! reasoned about later, instead of being inferred back out of a diff.
//!
//! # Nothing names a position
//!
//! Every operation that speaks about bytes speaks about them by name. A splice
//! says which gap its bytes go in, by naming the content either side of that
//! gap, and says which content it removes, by naming that content; a move says
//! the same of the run it relocates. No operation carries a byte offset, a line
//! number or anything else that a concurrent edit could invalidate, which is
//! the property [`crate::seq`] is built on and the reason the two vocabularies
//! are now one.
//!
//! # Every operation carries its parents
//!
//! An operation records the frontier its author could see when they wrote it,
//! in [`Header::parents`]. That is what makes the history a graph rather than a
//! list: with it, [`crate::log::OpLog`] can say whether a set is causally
//! complete, and [`crate::seq`] can say whether two operations that touched the
//! same bytes were concurrent or merely consecutive. Parents live on the header
//! and not on the variants, because causality is a property of every operation
//! alike and duplicating it six times would let the six drift.

use crate::id::{
	varint_decode,
	varint_encode,
	Anchor,
	ContentRange,
	OpId,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_hash::api::{
	Hash,
	Hasher,
};
use oxedyne_fe2o3_jdat::prelude::*;


/// Wire code for [`Op::FileCreate`].
pub const CODE_FILE_CREATE:	u8 = 1;
/// Wire code for [`Op::FileDelete`].
pub const CODE_FILE_DELETE:	u8 = 2;
/// Wire code for [`Op::FileRename`].
pub const CODE_FILE_RENAME:	u8 = 3;
/// Wire code for [`Op::Mark`].
pub const CODE_MARK:		u8 = 4;
/// Wire code for [`Op::Splice`].
pub const CODE_SPLICE:		u8 = 5;
/// Wire code for [`Op::Move`].
pub const CODE_MOVE:		u8 = 6;


/// A single unit of history: one whole edit.
///
/// The vocabulary is an enum rather than a trait object, so a reader can
/// enumerate everything history is able to say and the compiler can insist that
/// every consumer handles all of it.
///
/// [`Op::Move`] is why the vocabulary needs more than a splice. A move is not a
/// delete plus an insert: it names the run it relocates by the identity of the
/// bytes themselves, so an edit made concurrently inside that run lands in the
/// run's new home rather than on a tombstone. Nothing in a move says where
/// anything is -- the source is content, the destination is an anchor -- and
/// since a splice says the same, the sequence structure in [`crate::seq`] can
/// resolve any two of them against each other however they happen to arrive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Op {
	/// Brings a file into existence, empty.
	FileCreate {
		/// Path of the new file.
		path: String,
	},
	/// Removes a file.
	FileDelete {
		/// Path of the file removed.
		path: String,
	},
	/// Renames a file, leaving its contents untouched.
	FileRename {
		/// Path before the rename.
		from: String,
		/// Path after the rename.
		to: String,
	},
	/// Names a point in history, so that it can be referred to later.
	Mark {
		/// The name given to this point.
		name: String,
	},
	/// Puts bytes in a gap and kills runs of existing content, which is the
	/// single primitive from which insertion, deletion and replacement all
	/// follow: an insertion removes nothing, a deletion inserts nothing, and a
	/// replacement does both in one operation.
	///
	/// The gap is named by the content either side of it and the removed runs
	/// are named by their content, so neither half carries a position.
	Splice {
		/// Path of the file edited.
		file: String,
		/// Left origin of the inserted bytes; `None` is the start of the file.
		left: Option<Anchor>,
		/// Right origin of the inserted bytes; `None` is the end of the file.
		right: Option<Anchor>,
		/// What dies.
		remove: Vec<ContentRange>,
		/// What is inserted.
		insert: Vec<u8>,
	},
	/// Relocates runs of existing content, which keep their identity, to a
	/// position named by the content already there.
	///
	/// The source names bytes and not a file, because a byte's identity is
	/// repository-wide: the same operation moves a run within one file or from
	/// one file to another, and only `file` says which file the run lands in.
	Move {
		/// Path of the file the content lands in.
		file: String,
		/// What moves, in the order it lands in.
		src: Vec<ContentRange>,
		/// The gap the content lands in, on its left; `None` is the start of the
		/// file.
		left: Option<Anchor>,
		/// The gap the content lands in, on its right; `None` is the end of the
		/// file.
		right: Option<Anchor>,
	},
}

impl Op {
	/// Returns the wire code identifying the variant.
	pub fn code(&self) -> u8 {
		match self {
			Self::FileCreate { .. }	=> CODE_FILE_CREATE,
			Self::FileDelete { .. }	=> CODE_FILE_DELETE,
			Self::FileRename { .. }	=> CODE_FILE_RENAME,
			Self::Mark { .. }		=> CODE_MARK,
			Self::Splice { .. }		=> CODE_SPLICE,
			Self::Move { .. }		=> CODE_MOVE,
		}
	}

	/// Returns the variant name, for messages and logs.
	pub fn name(&self) -> &'static str {
		match self {
			Self::FileCreate { .. }	=> "FileCreate",
			Self::FileDelete { .. }	=> "FileDelete",
			Self::FileRename { .. }	=> "FileRename",
			Self::Mark { .. }		=> "Mark",
			Self::Splice { .. }		=> "Splice",
			Self::Move { .. }		=> "Move",
		}
	}

	/// Returns the file the operation edits the contents of, if it edits any.
	///
	/// A file lifecycle change names paths but edits no content, and a mark
	/// names no file at all, so both give `None`.
	pub fn file(&self) -> Option<&str> {
		match self {
			Self::Splice { file, .. }	=> Some(file),
			Self::Move { file, .. }		=> Some(file),
			_							=> None,
		}
	}

	/// Serialises the operation to a [`Dat`]. The shape is
	/// `[code, field, ...]`, the fields in declaration order.
	///
	/// Byte payloads use [`Dat::BU64`] rather than [`Dat::BU8`], whose length
	/// field is a single byte and so keeps only the low eight bits of the
	/// length of anything longer than 255 bytes.
	pub fn to_dat(&self) -> Dat {
		match self {
			Self::FileCreate { path } => Dat::List(vec![
				Dat::U8(CODE_FILE_CREATE),
				Dat::Str(path.clone()),
			]),
			Self::FileDelete { path } => Dat::List(vec![
				Dat::U8(CODE_FILE_DELETE),
				Dat::Str(path.clone()),
			]),
			Self::FileRename { from, to } => Dat::List(vec![
				Dat::U8(CODE_FILE_RENAME),
				Dat::Str(from.clone()),
				Dat::Str(to.clone()),
			]),
			Self::Mark { name } => Dat::List(vec![
				Dat::U8(CODE_MARK),
				Dat::Str(name.clone()),
			]),
			Self::Splice { file, left, right, remove, insert } => Dat::List(vec![
				Dat::U8(CODE_SPLICE),
				Dat::Str(file.clone()),
				Anchor::opt_to_dat(left),
				Anchor::opt_to_dat(right),
				Dat::List(remove.iter().map(|r| r.to_dat()).collect()),
				Dat::BU64(insert.clone()),
			]),
			Self::Move { file, src, left, right } => Dat::List(vec![
				Dat::U8(CODE_MOVE),
				Dat::Str(file.clone()),
				Dat::List(src.iter().map(|r| r.to_dat()).collect()),
				Anchor::opt_to_dat(left),
				Anchor::opt_to_dat(right),
			]),
		}
	}

	/// Reconstructs an operation from a [`Dat`] produced by [`Op::to_dat`].
	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let v = match dat {
			Dat::List(v) if !v.is_empty() => v,
			_ => return Err(err!(
				"An Op expects a non-empty Dat::List, got {:?}.", dat;
			Decode, Input, Mismatch)),
		};
		let code = match &v[0] {
			Dat::U8(c) => *c,
			other => return Err(err!(
				"An Op code expects Dat::U8, got {:?}.", other;
			Decode, Input, Mismatch)),
		};
		match code {
			CODE_FILE_CREATE => {
				res!(expect_len(v, 2, "FileCreate"));
				Ok(Self::FileCreate {
					path: res!(as_str(&v[1], "FileCreate path")),
				})
			},
			CODE_FILE_DELETE => {
				res!(expect_len(v, 2, "FileDelete"));
				Ok(Self::FileDelete {
					path: res!(as_str(&v[1], "FileDelete path")),
				})
			},
			CODE_FILE_RENAME => {
				res!(expect_len(v, 3, "FileRename"));
				Ok(Self::FileRename {
					from:	res!(as_str(&v[1], "FileRename from")),
					to:		res!(as_str(&v[2], "FileRename to")),
				})
			},
			CODE_MARK => {
				res!(expect_len(v, 2, "Mark"));
				Ok(Self::Mark {
					name: res!(as_str(&v[1], "Mark name")),
				})
			},
			CODE_SPLICE => {
				res!(expect_len(v, 6, "Splice"));
				Ok(Self::Splice {
					file:	res!(as_str(&v[1], "Splice file")),
					left:	res!(Anchor::opt_from_dat(&v[2])),
					right:	res!(Anchor::opt_from_dat(&v[3])),
					remove:	res!(as_ranges(&v[4], "Splice remove")),
					insert:	res!(as_bytes(&v[5], "Splice insert")),
				})
			},
			CODE_MOVE => {
				res!(expect_len(v, 5, "Move"));
				Ok(Self::Move {
					file:	res!(as_str(&v[1], "Move file")),
					src:	res!(as_ranges(&v[2], "Move src")),
					left:	res!(Anchor::opt_from_dat(&v[3])),
					right:	res!(Anchor::opt_from_dat(&v[4])),
				})
			},
			other => Err(err!(
				"Op code {} is not recognised.", other;
			Decode, Input, Invalid)),
		}
	}

	/// Appends the byte encoding of the operation to `buf`, as a varint length
	/// followed by the binary daticle form.
	///
	/// The length prefix lets a consumer skip an operation it does not need to
	/// read, and lets several be laid end to end in one buffer.
	pub fn encode_into(&self, buf: &mut Vec<u8>)
		-> Outcome<()>
	{
		let body = res!(self.to_dat().to_bytes(Vec::new()));
		varint_encode(body.len() as u64, buf);
		buf.extend_from_slice(&body);
		Ok(())
	}

	/// Returns the byte encoding of the operation.
	pub fn encode(&self)
		-> Outcome<Vec<u8>>
	{
		let mut buf = Vec::new();
		res!(self.encode_into(&mut buf));
		Ok(buf)
	}

	/// Decodes an operation from the front of `buf`, returning it and the
	/// number of bytes consumed.
	pub fn decode(buf: &[u8])
		-> Outcome<(Self, usize)>
	{
		let (dat, end) = res!(decode_framed(buf, "Op"));
		Ok((res!(Self::from_dat(&dat)), end))
	}

	/// Hashes the operation's canonical encoding under a hasher the caller
	/// supplies.
	///
	/// The choice of hash function is deliberately not made here. Which
	/// function is right depends on what else has to compute the same value --
	/// a browser limited to what its platform offers, a peer group that has
	/// already agreed on one -- so the caller brings it.
	pub fn hash<H: Hasher, const S: usize>(&self, hasher: H, salt: [u8; S])
		-> Outcome<Hash<S>>
	{
		let bytes = res!(self.encode());
		Ok(hasher.hash(&[&bytes], salt))
	}

	/// Decodes an operation that must occupy the whole of `buf`.
	pub fn decode_all(buf: &[u8])
		-> Outcome<Self>
	{
		let (op, len) = res!(Self::decode(buf));
		if len != buf.len() {
			return Err(err!(
				"An Op consumed {} of {} bytes, leaving {} trailing.",
				len, buf.len(), buf.len() - len;
			Decode, Input, Excessive));
		}
		Ok(op)
	}
}


/// What every operation carries whatever it says: its own name, and the names
/// of the operations its author had already seen.
///
/// The parents are the author's frontier at the moment of writing, which is
/// what turns a heap of operations into a partial order. Two operations are
/// concurrent exactly when neither is reachable from the other by following
/// parents, and that question -- not the accident of which arrived first -- is
/// what decides whether two edits to the same bytes were a conflict or a
/// sequence.
///
/// Parents are held sorted and without repetition, so that a set of parents has
/// exactly one byte spelling. Two spellings would both verify against a
/// signature, which is not a property a provenance chain can afford.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Header {
	/// The operation's own name.
	pub id:			OpId,
	/// The author's frontier when the operation was written, ascending.
	pub parents:	Vec<OpId>,
}

impl Header {
	/// Constructs a header, sorting the parents and dropping repetitions.
	///
	/// Fails if the operation names itself as its own parent, which no frontier
	/// can contain.
	pub fn new(id: OpId, parents: Vec<OpId>)
		-> Outcome<Self>
	{
		let mut parents = parents;
		parents.sort();
		parents.dedup();
		if parents.binary_search(&id).is_ok() {
			return Err(err!(
				"The operation {} names itself as one of its own parents.", id;
			Invalid, Input, Conflict));
		}
		Ok(Self { id, parents })
	}

	/// Constructs the header of a root operation, one written against nothing.
	pub fn root(id: OpId) -> Self {
		Self { id, parents: Vec::new() }
	}

	/// Reports whether the operation was written against nothing, which is what
	/// the first operation of a history looks like.
	pub fn is_root(&self) -> bool {
		self.parents.is_empty()
	}

	/// Serialises the header to a [`Dat`]. The shape is `[id, [parent, ...]]`.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			self.id.to_dat(),
			Dat::List(self.parents.iter().map(|p| p.to_dat()).collect()),
		])
	}

	/// Reconstructs a header from a [`Dat`] produced by [`Header::to_dat`].
	///
	/// Parents out of order, repeated, or naming the operation itself are
	/// refused rather than normalised, so that the encoding stays canonical.
	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let v = match dat {
			Dat::List(v) if v.len() == 2 => v,
			_ => return Err(err!(
				"A Header expects a 2-element Dat::List, got {:?}.", dat;
			Decode, Input, Mismatch)),
		};
		let id = res!(OpId::from_dat(&v[0]));
		let listed = match &v[1] {
			Dat::List(p) => p,
			other => return Err(err!(
				"A Header's parents expect Dat::List, got {:?}.", other;
			Decode, Input, Mismatch)),
		};
		let mut parents = Vec::with_capacity(listed.len());
		for item in listed {
			let p = res!(OpId::from_dat(item));
			if let Some(last) = parents.last() {
				if p <= *last {
					return Err(err!(
						"A Header of {} lists the parent {} after {}; parents are \
						encoded ascending and without repetition.", id, p, last;
					Decode, Input, Order));
				}
			}
			if p == id {
				return Err(err!(
					"A Header of {} names itself as one of its own parents.", id;
				Decode, Input, Conflict));
			}
			parents.push(p);
		}
		Ok(Self { id, parents })
	}
}


/// One whole operation as history records it: the header that names it and
/// places it in the graph, and the operation itself.
///
/// This is the unit that is logged, sealed into an [`crate::envelope::Envelope`]
/// and written into a segment. The header is not part of the operation because
/// the same edit written by two authors is two operations, and the vocabulary
/// should not have to say so six times over.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Record {
	/// Who this operation is and what it was written against.
	pub head:	Header,
	/// What it says.
	pub op:		Op,
}

impl Record {
	/// Constructs a record from a header and an operation.
	pub fn new(head: Header, op: Op) -> Self {
		Self { head, op }
	}

	/// Constructs a record of an operation written against nothing.
	pub fn root(id: OpId, op: Op) -> Self {
		Self { head: Header::root(id), op }
	}

	/// Returns the operation's name.
	pub fn id(&self) -> OpId {
		self.head.id
	}

	/// Returns the author's frontier when the operation was written.
	pub fn parents(&self) -> &[OpId] {
		&self.head.parents
	}

	/// Serialises the record to a [`Dat`]. The shape is `[head, op]`.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			self.head.to_dat(),
			self.op.to_dat(),
		])
	}

	/// Reconstructs a record from a [`Dat`] produced by [`Record::to_dat`].
	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let v = match dat {
			Dat::List(v) if v.len() == 2 => v,
			_ => return Err(err!(
				"A Record expects a 2-element Dat::List, got {:?}.", dat;
			Decode, Input, Mismatch)),
		};
		Ok(Self {
			head:	res!(Header::from_dat(&v[0])),
			op:		res!(Op::from_dat(&v[1])),
		})
	}

	/// Appends the byte encoding of the record to `buf`, as a varint length
	/// followed by the binary daticle form.
	pub fn encode_into(&self, buf: &mut Vec<u8>)
		-> Outcome<()>
	{
		let body = res!(self.to_dat().to_bytes(Vec::new()));
		varint_encode(body.len() as u64, buf);
		buf.extend_from_slice(&body);
		Ok(())
	}

	/// Returns the byte encoding of the record.
	pub fn encode(&self)
		-> Outcome<Vec<u8>>
	{
		let mut buf = Vec::new();
		res!(self.encode_into(&mut buf));
		Ok(buf)
	}

	/// Decodes a record from the front of `buf`, returning it and the number of
	/// bytes consumed.
	pub fn decode(buf: &[u8])
		-> Outcome<(Self, usize)>
	{
		let (dat, end) = res!(decode_framed(buf, "Record"));
		Ok((res!(Self::from_dat(&dat)), end))
	}

	/// Decodes a record that must occupy the whole of `buf`.
	pub fn decode_all(buf: &[u8])
		-> Outcome<Self>
	{
		let (rec, len) = res!(Self::decode(buf));
		if len != buf.len() {
			return Err(err!(
				"A Record consumed {} of {} bytes, leaving {} trailing.",
				len, buf.len(), buf.len() - len;
			Decode, Input, Excessive));
		}
		Ok(rec)
	}

	/// Hashes the record's canonical encoding under a hasher the caller
	/// supplies, so that the parents are covered along with the operation.
	pub fn hash<H: Hasher, const S: usize>(&self, hasher: H, salt: [u8; S])
		-> Outcome<Hash<S>>
	{
		let bytes = res!(self.encode());
		Ok(hasher.hash(&[&bytes], salt))
	}
}


/// Reads a varint length prefix and the daticle it frames, returning the
/// daticle and the offset just past it.
fn decode_framed(buf: &[u8], what: &str)
	-> Outcome<(Dat, usize)>
{
	let (len, hdr) = res!(varint_decode(buf));
	let len = len as usize;
	let end = match hdr.checked_add(len) {
		Some(e) => e,
		None => return Err(err!(
			"A {} declares a length of {} bytes, which overflows the buffer \
			offset.", what, len;
		Decode, Input, Overflow)),
	};
	if end > buf.len() {
		return Err(err!(
			"A {} declares {} bytes of body but only {} remain.",
			what, len, buf.len() - hdr;
		Decode, Input, Missing));
	}
	let (dat, used) = res!(Dat::from_bytes(&buf[hdr..end]));
	if used != len {
		return Err(err!(
			"A {} body of {} bytes decoded from only {} of them.", what, len, used;
		Decode, Input, Mismatch));
	}
	Ok((dat, end))
}

/// Checks that a decoded operation list has exactly the expected length.
fn expect_len(v: &[Dat], want: usize, what: &str)
	-> Outcome<()>
{
	if v.len() != want {
		return Err(err!(
			"An Op::{} expects {} list elements, got {}.", what, want, v.len();
		Decode, Input, Mismatch));
	}
	Ok(())
}

/// Extracts a string field, naming it if the kind is wrong.
fn as_str(dat: &Dat, what: &str)
	-> Outcome<String>
{
	match dat {
		Dat::Str(s) => Ok(s.clone()),
		other => Err(err!(
			"An Op {} expects Dat::Str, got {:?}.", what, other;
		Decode, Input, Mismatch)),
	}
}

/// Extracts a list of content ranges, naming it if the kind is wrong.
fn as_ranges(dat: &Dat, what: &str)
	-> Outcome<Vec<ContentRange>>
{
	match dat {
		Dat::List(v) => {
			let mut out = Vec::with_capacity(v.len());
			for item in v {
				out.push(res!(ContentRange::from_dat(item)));
			}
			Ok(out)
		},
		other => Err(err!(
			"An Op {} expects Dat::List, got {:?}.", what, other;
		Decode, Input, Mismatch)),
	}
}

/// Extracts a byte vector field, naming it if the kind is wrong.
fn as_bytes(dat: &Dat, what: &str)
	-> Outcome<Vec<u8>>
{
	match dat {
		Dat::BU64(b) => Ok(b.clone()),
		other => Err(err!(
			"An Op {} expects Dat::BU64, got {:?}.", what, other;
		Decode, Input, Mismatch)),
	}
}


#[cfg(test)]
mod tests {
	use super::*;

	use crate::id::{
		ContentId,
		ReplicaId,
	};

	/// A content range of the given replica's first operation.
	fn range(replica: u64, from: u64, to: u64) -> ContentRange {
		ContentRange { op: OpId::new(ReplicaId::new(replica), 1), from, to }
	}

	/// A content identifier of the given replica's first operation.
	fn content(replica: u64, off: u64) -> ContentId {
		ContentId::new(OpId::new(ReplicaId::new(replica), 1), off)
	}

	/// An operation identifier.
	fn oid(replica: u64, counter: u64) -> OpId {
		OpId::new(ReplicaId::new(replica), counter)
	}

	/// One of every variant, including payloads that stress the encoding.
	fn samples() -> Vec<Op> {
		vec![
			Op::FileCreate { path: fmt!("src/lib.rs") },
			Op::FileDelete { path: fmt!("src/old.rs") },
			Op::FileRename {
				from:	fmt!("a/b.txt"),
				to:		fmt!("c/d.txt"),
			},
			// An insertion at the start of a file.
			Op::Splice {
				file:	fmt!("notes.md"),
				left:	None,
				right:	None,
				remove:	Vec::new(),
				insert:	b"hello".to_vec(),
			},
			// A deletion, which places nothing and so has no origins.
			Op::Splice {
				file:	fmt!("notes.md"),
				left:	None,
				right:	None,
				remove:	vec![range(1, 12, 17)],
				insert:	Vec::new(),
			},
			// A replacement whose payload exceeds what a BU8 length can hold,
			// killing several fragmented runs at once.
			Op::Splice {
				file:	fmt!("big.bin"),
				left:	Some(Anchor::after(content(2, u64::MAX))),
				right:	Some(Anchor::before(content(3, 0))),
				remove:	vec![
					range(1, 0, u64::MAX),
					range(4, 7, 9),
				],
				insert:	vec![0xa5; 1000],
			},
			// Empty strings and non-ASCII paths.
			Op::FileCreate { path: String::new() },
			Op::Mark { name: fmt!("release-caf\u{e9}") },
			// A move of one run into the middle of a file.
			Op::Move {
				file:	fmt!("src/lib.rs"),
				src:	vec![range(1, 0, 40)],
				left:	Some(Anchor::after(content(2, 3))),
				right:	Some(Anchor::before(content(2, 4))),
			},
			// A move to the very start of a file, of a run fragmented by the
			// edits it has already survived.
			Op::Move {
				file:	fmt!("src/lib.rs"),
				src:	vec![
					range(1, 0, 4),
					range(3, 7, 9),
					range(1, 4, 40),
				],
				left:	None,
				right:	Some(Anchor::before(content(1, 41))),
			},
			// A move to the end of a file, whose destination is bounded by
			// nothing on the right.
			Op::Move {
				file:	String::new(),
				src:	vec![range(u64::MAX, 0, u64::MAX)],
				left:	Some(Anchor::after(content(7, u64::MAX))),
				right:	None,
			},
			// A move of nothing, to nowhere in particular: the degenerate shape
			// the codec still has to spell.
			Op::Move {
				file:	fmt!("empty"),
				src:	Vec::new(),
				left:	None,
				right:	None,
			},
		]
	}

	/// Headers spanning no parents, one, and many.
	fn sample_heads() -> Outcome<Vec<Header>> {
		Ok(vec![
			Header::root(oid(1, 1)),
			res!(Header::new(oid(2, 9), vec![oid(1, 1)])),
			res!(Header::new(oid(3, 4), vec![
				oid(1, 1),
				oid(2, 9),
				oid(9, u64::MAX),
			])),
			res!(Header::new(oid(4, u64::MAX), (1..=200)
				.map(|i| oid(i % 13, i))
				.collect())),
		])
	}

	/// Every variant survives a [`Dat`] round trip.
	#[test]
	fn op_dat_round_trip() -> Outcome<()> {
		for op in samples() {
			let back = res!(Op::from_dat(&op.to_dat()));
			assert_eq!(op, back, "variant {}", op.name());
		}
		Ok(())
	}

	/// Every variant survives a byte round trip.
	#[test]
	fn op_byte_round_trip() -> Outcome<()> {
		for op in samples() {
			let buf = res!(op.encode());
			let back = res!(Op::decode_all(&buf));
			assert_eq!(op, back, "variant {}", op.name());
		}
		Ok(())
	}

	/// A payload longer than 255 bytes keeps its full length, which a
	/// `Dat::BU8` length field could not express.
	#[test]
	fn splice_payload_survives_beyond_a_byte_length() -> Outcome<()> {
		for len in [255usize, 256, 257, 4096, 70_000] {
			let op = Op::Splice {
				file:	fmt!("f"),
				left:	None,
				right:	None,
				remove:	Vec::new(),
				insert:	vec![0x5a; len],
			};
			let back = res!(Op::decode_all(&res!(op.encode())));
			match back {
				Op::Splice { insert, .. } => assert_eq!(insert.len(), len),
				other => return Err(err!(
					"Expected a Splice, got {}.", other.name(); Test, Mismatch)),
			}
		}
		Ok(())
	}

	/// Operations laid end to end each decode in turn, consuming only their
	/// own bytes.
	#[test]
	fn ops_decode_back_to_back() -> Outcome<()> {
		let ops = samples();
		let mut buf = Vec::new();
		for op in &ops {
			res!(op.encode_into(&mut buf));
		}
		let mut at = 0;
		for want in &ops {
			let (got, used) = res!(Op::decode(&buf[at..]));
			assert_eq!(&got, want);
			at += used;
		}
		assert_eq!(at, buf.len());
		Ok(())
	}

	/// Wire codes are distinct and match the variants they name.
	#[test]
	fn codes_are_distinct() -> Outcome<()> {
		let mut seen = Vec::new();
		for op in samples() {
			let code = op.code();
			assert!(code != 0, "variant {} has a zero code", op.name());
			if !seen.contains(&(code, op.name())) {
				seen.push((code, op.name()));
			}
		}
		for (i, (code, name)) in seen.iter().enumerate() {
			for (other_code, other_name) in seen.iter().skip(i + 1) {
				assert!(
					code != other_code,
					"{} and {} share code {}", name, other_name, code,
				);
			}
		}
		Ok(())
	}

	/// Only the two content operations name a file whose bytes they edit.
	#[test]
	fn only_content_operations_name_a_file() -> Outcome<()> {
		assert_eq!(
			Op::Splice {
				file:	fmt!("a.txt"),
				left:	None,
				right:	None,
				remove:	Vec::new(),
				insert:	b"x".to_vec(),
			}.file(),
			Some("a.txt"),
		);
		assert_eq!(
			Op::Move {
				file:	fmt!("b.txt"),
				src:	Vec::new(),
				left:	None,
				right:	None,
			}.file(),
			Some("b.txt"),
		);
		assert_eq!(Op::FileCreate { path: fmt!("c.txt") }.file(), None);
		assert_eq!(Op::FileDelete { path: fmt!("c.txt") }.file(), None);
		assert_eq!(Op::Mark { name: fmt!("v1") }.file(), None);
		Ok(())
	}

	/// An unrecognised code, a wrong shape or a wrong field kind is refused.
	#[test]
	fn op_from_dat_rejects_rubbish() -> Outcome<()> {
		assert!(Op::from_dat(&Dat::U8(CODE_MARK)).is_err());
		assert!(Op::from_dat(&Dat::List(vec![])).is_err());
		assert!(Op::from_dat(&Dat::List(vec![Dat::U8(200), Dat::Str(fmt!("x"))])).is_err());
		// Right code, wrong arity.
		assert!(Op::from_dat(&Dat::List(vec![Dat::U8(CODE_FILE_RENAME)])).is_err());
		// Right arity, wrong field kind.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_FILE_CREATE),
			Dat::U64(3),
		])).is_err());
		// A Splice whose payload is not a byte vector.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_SPLICE),
			Dat::Str(fmt!("f")),
			Anchor::opt_to_dat(&None),
			Anchor::opt_to_dat(&None),
			Dat::List(vec![]),
			Dat::Str(fmt!("not bytes")),
		])).is_err());
		// A Splice whose removed runs are not ranges.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_SPLICE),
			Dat::Str(fmt!("f")),
			Anchor::opt_to_dat(&None),
			Anchor::opt_to_dat(&None),
			Dat::Str(fmt!("not ranges")),
			Dat::BU64(Vec::new()),
		])).is_err());
		// A Move whose source is not a list of ranges.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_MOVE),
			Dat::Str(fmt!("f")),
			Dat::Str(fmt!("not ranges")),
			Anchor::opt_to_dat(&None),
			Anchor::opt_to_dat(&None),
		])).is_err());
		// A Move whose anchor is bare rather than optional.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_MOVE),
			Dat::Str(fmt!("f")),
			Dat::List(vec![range(1, 0, 2).to_dat()]),
			Anchor::after(content(1, 0)).to_dat(),
			Anchor::opt_to_dat(&None),
		])).is_err());
		Ok(())
	}

	/// A move carrying many source runs keeps every one of them, in order,
	/// through both round trips.
	#[test]
	fn move_keeps_its_source_runs_in_order() -> Outcome<()> {
		let src: Vec<ContentRange> = (0..300u64)
			.map(|i| range(i % 7 + 1, i, i + 5))
			.collect();
		let op = Op::Move {
			file:	fmt!("src/seq.rs"),
			src:	src.clone(),
			left:	Some(Anchor::after(content(1, 9))),
			right:	None,
		};
		for back in [
			res!(Op::from_dat(&op.to_dat())),
			res!(Op::decode_all(&res!(op.encode()))),
		] {
			match back {
				Op::Move { src: got, .. } => assert_eq!(got, src),
				other => return Err(err!(
					"Expected a Move, got {}.", other.name(); Test, Mismatch)),
			}
		}
		Ok(())
	}

	/// Both destination anchors keep their side, which decides whether an
	/// insertion abutting the moved run travels with it.
	#[test]
	fn move_anchors_keep_their_side() -> Outcome<()> {
		let cid = content(2, 11);
		for (left, right) in [
			(None, None),
			(Some(Anchor::after(cid)), None),
			(None, Some(Anchor::before(cid))),
			(Some(Anchor::after(cid)), Some(Anchor::before(cid))),
			// The sides the sequence structure refuses are still spelled
			// faithfully; refusing them is the structure's business, not the
			// codec's.
			(Some(Anchor::before(cid)), Some(Anchor::after(cid))),
		] {
			let op = Op::Move {
				file:	fmt!("f"),
				src:	vec![range(1, 0, 3)],
				left,
				right,
			};
			assert_eq!(op, res!(Op::decode_all(&res!(op.encode()))));
			assert_eq!(op, res!(Op::from_dat(&op.to_dat())));
			// A splice carrying the same origins spells them the same way.
			let sp = Op::Splice {
				file:	fmt!("f"),
				left,
				right,
				remove:	vec![range(1, 0, 3)],
				insert:	b"x".to_vec(),
			};
			assert_eq!(sp, res!(Op::decode_all(&res!(sp.encode()))));
			assert_eq!(sp, res!(Op::from_dat(&sp.to_dat())));
		}
		Ok(())
	}

	/// Hashing goes through the canonical encoding: under the identity hasher
	/// the result is exactly those bytes, and equal operations hash equal while
	/// differing ones do not.
	#[test]
	fn op_hashes_its_canonical_encoding() -> Outcome<()> {
		let op = sample_op_for_hashing();
		let want = res!(op.encode());
		let got = res!(op.hash((), [0u8; 0])).as_vec();
		assert_eq!(got, want);
		// An operation differing in one field hashes differently.
		let other = Op::Splice {
			file:	fmt!("notes.md"),
			left:	None,
			right:	None,
			remove:	vec![range(1, 12, 16)],
			insert:	b"abc".to_vec(),
		};
		assert!(res!(other.hash((), [0u8; 0])).as_vec() != want);
		Ok(())
	}

	/// The operation used by the hashing test.
	fn sample_op_for_hashing() -> Op {
		Op::Splice {
			file:	fmt!("notes.md"),
			left:	None,
			right:	None,
			remove:	vec![range(1, 12, 15)],
			insert:	b"abc".to_vec(),
		}
	}

	/// A truncated byte encoding is refused rather than half read.
	#[test]
	fn op_decode_rejects_truncation() -> Outcome<()> {
		let op = Op::Splice {
			file:	fmt!("notes.md"),
			left:	Some(Anchor::after(content(1, 0))),
			right:	None,
			remove:	vec![range(1, 1, 3)],
			insert:	b"abcdef".to_vec(),
		};
		let buf = res!(op.encode());
		for cut in 1..buf.len() {
			assert!(Op::decode(&buf[..cut]).is_err(), "cut at {}", cut);
		}
		Ok(())
	}

	/// A header survives both round trips with no parents, with one, and with
	/// many.
	#[test]
	fn header_round_trips_at_every_arity() -> Outcome<()> {
		for head in res!(sample_heads()) {
			assert_eq!(head, res!(Header::from_dat(&head.to_dat())));
			let rec = Record::new(head.clone(), Op::Mark { name: fmt!("m") });
			assert_eq!(rec, res!(Record::decode_all(&res!(rec.encode()))));
		}
		Ok(())
	}

	/// Parents are sorted and deduplicated on construction, so the same frontier
	/// given in any order has one encoding.
	#[test]
	fn parents_are_canonical() -> Outcome<()> {
		let a = res!(Header::new(oid(9, 1), vec![oid(1, 2), oid(3, 1), oid(1, 2)]));
		let b = res!(Header::new(oid(9, 1), vec![oid(3, 1), oid(1, 2)]));
		assert_eq!(a, b);
		assert_eq!(a.parents, vec![oid(1, 2), oid(3, 1)]);
		assert_eq!(res!(a.to_dat().to_bytes(Vec::new())), res!(b.to_dat().to_bytes(Vec::new())));
		// The decoder refuses the non-canonical spellings the constructor fixes.
		let unsorted = Dat::List(vec![
			oid(9, 1).to_dat(),
			Dat::List(vec![oid(3, 1).to_dat(), oid(1, 2).to_dat()]),
		]);
		assert!(Header::from_dat(&unsorted).is_err());
		let repeated = Dat::List(vec![
			oid(9, 1).to_dat(),
			Dat::List(vec![oid(1, 2).to_dat(), oid(1, 2).to_dat()]),
		]);
		assert!(Header::from_dat(&repeated).is_err());
		Ok(())
	}

	/// An operation may not be its own parent, on the way in or on the way out.
	#[test]
	fn an_operation_is_not_its_own_parent() -> Outcome<()> {
		assert!(Header::new(oid(1, 4), vec![oid(2, 1), oid(1, 4)]).is_err());
		let itself = Dat::List(vec![
			oid(1, 4).to_dat(),
			Dat::List(vec![oid(1, 4).to_dat()]),
		]);
		assert!(Header::from_dat(&itself).is_err());
		Ok(())
	}

	/// A root header carries no parents and says so.
	#[test]
	fn a_root_header_has_no_parents() -> Outcome<()> {
		let head = Header::root(oid(1, 1));
		assert!(head.is_root());
		assert!(head.parents.is_empty());
		assert!(!res!(Header::new(oid(1, 2), vec![oid(1, 1)])).is_root());
		Ok(())
	}

	/// A record round trips whatever operation it carries, and a malformed one
	/// is refused.
	#[test]
	fn record_round_trips_every_variant() -> Outcome<()> {
		let head = res!(Header::new(oid(5, 7), vec![oid(1, 1), oid(2, 2)]));
		for op in samples() {
			let rec = Record::new(head.clone(), op);
			assert_eq!(rec, res!(Record::from_dat(&rec.to_dat())));
			assert_eq!(rec, res!(Record::decode_all(&res!(rec.encode()))));
		}
		assert!(Record::from_dat(&Dat::U8(1)).is_err());
		assert!(Record::from_dat(&Dat::List(vec![Dat::U8(1)])).is_err());
		Ok(())
	}

	/// The parents are inside what a record hashes, so an operation re-parented
	/// hashes differently.
	#[test]
	fn the_parents_are_covered_by_the_hash() -> Outcome<()> {
		let op = Op::Mark { name: fmt!("v1") };
		let one = Record::new(res!(Header::new(oid(1, 5), vec![oid(2, 1)])), op.clone());
		let two = Record::new(res!(Header::new(oid(1, 5), vec![oid(2, 2)])), op);
		assert!(
			res!(one.hash((), [0u8; 0])).as_vec() != res!(two.hash((), [0u8; 0])).as_vec(),
			"re-parenting must change the hash",
		);
		Ok(())
	}

	/// A truncated record is refused at every cut.
	#[test]
	fn record_decode_rejects_truncation() -> Outcome<()> {
		let rec = Record::new(
			res!(Header::new(oid(2, 3), vec![oid(1, 1), oid(1, 2)])),
			Op::Splice {
				file:	fmt!("f"),
				left:	None,
				right:	None,
				remove:	Vec::new(),
				insert:	b"abcdef".to_vec(),
			},
		);
		let buf = res!(rec.encode());
		for cut in 1..buf.len() {
			assert!(Record::decode(&buf[..cut]).is_err(), "cut at {}", cut);
		}
		Ok(())
	}

	/// Records laid end to end each decode in turn.
	#[test]
	fn records_decode_back_to_back() -> Outcome<()> {
		let heads = res!(sample_heads());
		let recs: Vec<Record> = samples()
			.into_iter()
			.enumerate()
			.map(|(i, op)| Record::new(heads[i % heads.len()].clone(), op))
			.collect();
		let mut buf = Vec::new();
		for rec in &recs {
			res!(rec.encode_into(&mut buf));
		}
		let mut at = 0;
		for want in &recs {
			let (got, used) = res!(Record::decode(&buf[at..]));
			assert_eq!(&got, want);
			at += used;
		}
		assert_eq!(at, buf.len());
		Ok(())
	}
}
