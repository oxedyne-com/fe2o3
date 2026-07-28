//! The operation vocabulary: what a single unit of history can say.
//!
//! History here is a sequence of operations rather than a sequence of
//! snapshots. An operation states an intent -- create this file, replace this
//! run of bytes with these -- so the intent survives into the record and can be
//! reasoned about later, instead of being inferred back out of a diff.
//!
//! # Provisional
//!
//! This set is provisional. It is deliberately small: enough to express file
//! lifecycle and byte-level edits, and no more. Variants may be added, and the
//! wire codes below are not promised stable.

use crate::id::{
	varint_decode,
	varint_encode,
	Anchor,
	ContentRange,
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
/// Wire code for [`Op::Splice`].
pub const CODE_SPLICE:		u8 = 4;
/// Wire code for [`Op::Mark`].
pub const CODE_MARK:		u8 = 5;
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
/// anything is -- the source is content, the destination is an anchor -- which
/// is what lets the sequence structure in [`crate::seq`] resolve the two against
/// each other however the two operations happen to arrive.
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
	/// Replaces a run of bytes in a file with another, which is the single
	/// primitive from which insertion, deletion and replacement all follow: an
	/// insertion has `delete_len` of zero, a deletion has an empty `insert`.
	Splice {
		/// Path of the file edited.
		file: String,
		/// Byte offset at which the replaced run begins.
		at: u64,
		/// Number of bytes removed at that offset.
		delete_len: u64,
		/// Bytes put in their place.
		insert: Vec<u8>,
	},
	/// Names a point in history, so that it can be referred to later.
	Mark {
		/// The name given to this point.
		name: String,
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
			Self::Splice { .. }		=> CODE_SPLICE,
			Self::Mark { .. }		=> CODE_MARK,
			Self::Move { .. }		=> CODE_MOVE,
		}
	}

	/// Returns the variant name, for messages and logs.
	pub fn name(&self) -> &'static str {
		match self {
			Self::FileCreate { .. }	=> "FileCreate",
			Self::FileDelete { .. }	=> "FileDelete",
			Self::FileRename { .. }	=> "FileRename",
			Self::Splice { .. }		=> "Splice",
			Self::Mark { .. }		=> "Mark",
			Self::Move { .. }		=> "Move",
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
			Self::Splice { file, at, delete_len, insert } => Dat::List(vec![
				Dat::U8(CODE_SPLICE),
				Dat::Str(file.clone()),
				Dat::U64(*at),
				Dat::U64(*delete_len),
				Dat::BU64(insert.clone()),
			]),
			Self::Mark { name } => Dat::List(vec![
				Dat::U8(CODE_MARK),
				Dat::Str(name.clone()),
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
			CODE_SPLICE => {
				res!(expect_len(v, 5, "Splice"));
				Ok(Self::Splice {
					file:		res!(as_str(&v[1], "Splice file")),
					at:			res!(as_u64(&v[2], "Splice at")),
					delete_len:	res!(as_u64(&v[3], "Splice delete_len")),
					insert:		res!(as_bytes(&v[4], "Splice insert")),
				})
			},
			CODE_MARK => {
				res!(expect_len(v, 2, "Mark"));
				Ok(Self::Mark {
					name: res!(as_str(&v[1], "Mark name")),
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
		let (len, hdr) = res!(varint_decode(buf));
		let len = len as usize;
		let end = match hdr.checked_add(len) {
			Some(e) => e,
			None => return Err(err!(
				"An Op declares a length of {} bytes, which overflows the buffer \
				offset.", len;
			Decode, Input, Overflow)),
		};
		if end > buf.len() {
			return Err(err!(
				"An Op declares {} bytes of body but only {} remain.",
				len, buf.len() - hdr;
			Decode, Input, Missing));
		}
		let (dat, used) = res!(Dat::from_bytes(&buf[hdr..end]));
		if used != len {
			return Err(err!(
				"An Op body of {} bytes decoded from only {} of them.", len, used;
			Decode, Input, Mismatch));
		}
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

/// Extracts an unsigned integer field, naming it if the kind is wrong.
fn as_u64(dat: &Dat, what: &str)
	-> Outcome<u64>
{
	match dat {
		Dat::U64(n) => Ok(*n),
		other => Err(err!(
			"An Op {} expects Dat::U64, got {:?}.", what, other;
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
		OpId,
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

	/// One of every variant, including payloads that stress the encoding.
	fn samples() -> Vec<Op> {
		vec![
			Op::FileCreate { path: fmt!("src/lib.rs") },
			Op::FileDelete { path: fmt!("src/old.rs") },
			Op::FileRename {
				from:	fmt!("a/b.txt"),
				to:		fmt!("c/d.txt"),
			},
			// An insertion.
			Op::Splice {
				file:		fmt!("notes.md"),
				at:			0,
				delete_len:	0,
				insert:		b"hello".to_vec(),
			},
			// A deletion.
			Op::Splice {
				file:		fmt!("notes.md"),
				at:			12,
				delete_len:	5,
				insert:		Vec::new(),
			},
			// A replacement whose payload exceeds what a BU8 length can hold.
			Op::Splice {
				file:		fmt!("big.bin"),
				at:			u64::MAX / 2,
				delete_len:	u64::MAX,
				insert:		vec![0xa5; 1000],
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
				file:		fmt!("f"),
				at:			0,
				delete_len:	0,
				insert:		vec![0x5a; len],
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
			Dat::U64(0),
			Dat::U64(0),
			Dat::Str(fmt!("not bytes")),
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
			file:		fmt!("notes.md"),
			at:			13,
			delete_len:	3,
			insert:		b"abc".to_vec(),
		};
		assert!(res!(other.hash((), [0u8; 0])).as_vec() != want);
		Ok(())
	}

	/// The operation used by the hashing test.
	fn sample_op_for_hashing() -> Op {
		Op::Splice {
			file:		fmt!("notes.md"),
			at:			12,
			delete_len:	3,
			insert:		b"abc".to_vec(),
		}
	}

	/// A truncated byte encoding is refused rather than half read.
	#[test]
	fn op_decode_rejects_truncation() -> Outcome<()> {
		let op = Op::Splice {
			file:		fmt!("notes.md"),
			at:			1,
			delete_len:	2,
			insert:		b"abcdef".to_vec(),
		};
		let buf = res!(op.encode());
		for cut in 1..buf.len() {
			assert!(Op::decode(&buf[..cut]).is_err(), "cut at {}", cut);
		}
		Ok(())
	}
}
