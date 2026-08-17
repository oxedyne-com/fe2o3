//! Identifiers for replicas, for the operations they author, and for the bytes
//! those operations create.
//!
//! An operation is named by the replica that authored it together with that
//! replica's own counter. A name can therefore be minted without consulting any
//! other replica and without reading a clock, which is what lets history be
//! written offline and merged later. Names are unique across replicas, stable
//! once minted, and totally ordered within a replica.
//!
//! Identifiers are encoded as a pair of LEB128-style varints, so a small
//! replica number and a small counter cost two bytes. The decoder rejects
//! overlong encodings, giving every identifier exactly one byte spelling --
//! necessary where those bytes are hashed or signed.
//!
//! # Content is named, not located
//!
//! Above the operation identifier sit three more names, and none of them is
//! minted: each is arithmetic over an operation identifier and an offset. A
//! [`ContentId`] names one byte by the splice that created it, a
//! [`ContentRange`] names a run of them, and an [`Anchor`] names a gap by the
//! byte on one side of it. Because a byte's name says what created it rather
//! than where it sits, the name survives the byte being moved, and an edit
//! anchored to it travels with the content it was written against.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

use std::fmt;
use std::ops::Range;


pub const VARINT_MAX_LEN: usize = 10; // bytes a u64 varint may occupy


/// Each byte carries seven value bits, least significant group first, with the
/// high bit set on every byte but the last.
pub fn varint_encode(n: u64, buf: &mut Vec<u8>) {
	let mut v = n;
	loop {
		let byte = (v & 0x7f) as u8;
		v >>= 7;
		if v == 0 {
			buf.push(byte);
			return;
		}
		buf.push(byte | 0x80);
	}
}

/// Yields the value and how many bytes it took.
///
/// Overlong encodings are rejected so that every value has exactly one byte
/// spelling. Two spellings of one value would otherwise both verify against a
/// signature, which is not a property a provenance chain can afford.
pub fn varint_decode(buf: &[u8])
	-> Outcome<(u64, usize)>
{
	let mut result: u64 = 0;
	let mut shift: u32 = 0;
	for (i, byte) in buf.iter().enumerate() {
		if i >= VARINT_MAX_LEN {
			return Err(err!(
				"A varint encoding a u64 occupies at most {} bytes, byte {} continues \
				beyond that.", VARINT_MAX_LEN, i;
			Decode, Input, Excessive));
		}
		let payload = (*byte & 0x7f) as u64;
		// The tenth byte of a maximal encoding carries only the top bit of the u64.
		if i == VARINT_MAX_LEN - 1 && payload > 1 {
			return Err(err!(
				"Varint byte {} is {:#04x}, which overflows a u64.", i, byte;
			Decode, Input, Overflow));
		}
		result |= payload << shift;
		if *byte & 0x80 == 0 {
			// Only the single byte encoding of zero may end in a zero payload; anything
			// longer that does so is an overlong spelling of a smaller value.
			if i > 0 && payload == 0 {
				return Err(err!(
					"Varint of {} bytes ends in a zero payload byte, an overlong \
					encoding.", i + 1;
				Decode, Input, Invalid));
			}
			return Ok((result, i + 1));
		}
		shift += 7;
	}
	Err(err!(
		"Varint is truncated: all {} available byte{} carry the continuation bit.",
		buf.len(), if buf.len() == 1 { "" } else { "s" };
	Decode, Input, Missing))
}


/// Identifies one writer of history.
///
/// A replica is whatever mints operation counters independently: a working
/// copy, a device, a server-side session. The number carries no meaning beyond
/// distinguishing one writer from another, and the caller is responsible for
/// ensuring two live writers never share one.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ReplicaId(u64);

impl ReplicaId {
	pub const fn new(id: u64) -> Self {
		Self(id)
	}

	pub const fn inner(&self) -> u64 {
		self.0
	}

	pub fn encode_into(&self, buf: &mut Vec<u8>) {
		varint_encode(self.0, buf)
	}

	pub fn encode(&self) -> Vec<u8> {
		let mut buf = Vec::with_capacity(VARINT_MAX_LEN);
		self.encode_into(&mut buf);
		buf
	}

	pub fn decode(buf: &[u8])
		-> Outcome<(Self, usize)>
	{
		let (n, len) = res!(varint_decode(buf));
		Ok((Self(n), len))
	}
}

impl fmt::Display for ReplicaId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "r{}", self.0)
	}
}


/// Names a single operation: the replica that authored it, and that replica's
/// own count at the time.
///
/// Counters start at one, so zero is available to mean "no operation yet".
/// Ordering is by replica first and counter second, which gives a stable total
/// order over identifiers; it is not a causal order, and nothing here claims it
/// is.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OpId {
	pub replica:	ReplicaId,	// the replica that authored the operation
	pub counter:	u64,		// that replica's own count, from one
}

impl OpId {
	pub const fn new(replica: ReplicaId, counter: u64) -> Self {
		Self { replica, counter }
	}

	/// Replica then counter, each a varint.
	pub fn encode_into(&self, buf: &mut Vec<u8>) {
		self.replica.encode_into(buf);
		varint_encode(self.counter, buf);
	}

	pub fn encode(&self) -> Vec<u8> {
		let mut buf = Vec::with_capacity(2 * VARINT_MAX_LEN);
		self.encode_into(&mut buf);
		buf
	}

	pub fn decode(buf: &[u8])
		-> Outcome<(Self, usize)>
	{
		let (replica, n1) = res!(ReplicaId::decode(buf));
		let (counter, n2) = res!(varint_decode(&buf[n1..]));
		Ok((Self { replica, counter }, n1 + n2))
	}

	pub fn decode_all(buf: &[u8])
		-> Outcome<Self>
	{
		let (id, len) = res!(Self::decode(buf));
		if len != buf.len() {
			return Err(err!(
				"An OpId consumed {} of {} bytes, leaving {} trailing.",
				len, buf.len(), buf.len() - len;
			Decode, Input, Excessive));
		}
		Ok(id)
	}

	/// The shape is `[replica, counter]`.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			Dat::U64(self.replica.inner()),
			Dat::U64(self.counter),
		])
	}

	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let pair = match dat {
			Dat::List(v) if v.len() == 2 => v,
			_ => return Err(err!(
				"An OpId expects a 2-element Dat::List, got {:?}.", dat;
			Decode, Input, Mismatch)),
		};
		let replica = match &pair[0] {
			Dat::U64(n) => ReplicaId::new(*n),
			other => return Err(err!(
				"An OpId replica expects Dat::U64, got {:?}.", other;
			Decode, Input, Mismatch)),
		};
		let counter = match &pair[1] {
			Dat::U64(n) => *n,
			other => return Err(err!(
				"An OpId counter expects Dat::U64, got {:?}.", other;
			Decode, Input, Mismatch)),
		};
		Ok(Self { replica, counter })
	}
}

impl fmt::Display for OpId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}:{}", self.replica, self.counter)
	}
}

impl std::str::FromStr for OpId {
	type Err = Error<ErrTag>;

	/// Reads back exactly what [`fmt::Display`] wrote: `r<replica>:<counter>`.
	///
	/// It is here because an identifier is the one thing in this vocabulary a
	/// person is ever asked to type. Every command and every page that lets
	/// somebody name an operation -- reverting one, marking a flag reviewed,
	/// settling a proposal -- has to turn that text back into an [`OpId`], and a
	/// reader that each of them wrote for itself is a reader that accepts a
	/// different set of spellings in each of them.
	///
	/// ```
	/// use oxedyne_fe2o3_ore::id::{OpId, ReplicaId};
	///
	/// let id = OpId::new(ReplicaId::new(3065315576), 4);
	/// assert_eq!(format!("{}", id), "r3065315576:4");
	/// assert_eq!("r3065315576:4".parse::<OpId>().unwrap(), id);
	/// ```
	///
	/// The `r` is required, and so is the colon. Both are refused rather than
	/// forgiven, because what a person typed is nearly always what a command
	/// printed, and quietly accepting a second spelling would let one identifier
	/// be written two ways in the very sidecars and messages that exist to be
	/// compared with each other.
	fn from_str(text: &str)
		-> Outcome<Self>
	{
		let body = res!(text.strip_prefix('r').ok_or_else(|| err!(
			"An operation identifier is written {:?}, and {:?} does not begin with \
			{:?}.", "r<replica>:<counter>", text, "r";
		Invalid, Input, Mismatch)));
		let (replica, counter) = res!(body.split_once(':').ok_or_else(|| err!(
			"An operation identifier is written {:?}, and {:?} holds no colon \
			separating the replica from the counter.", "r<replica>:<counter>", text;
		Invalid, Input, Missing)));
		let replica = match replica.parse::<u64>() {
			Ok(n) => n,
			Err(e) => return Err(err!(e,
				"The replica of the operation identifier {:?} is not a number.", text;
			Invalid, Input, Mismatch)),
		};
		let counter = match counter.parse::<u64>() {
			Ok(n) => n,
			Err(e) => return Err(err!(e,
				"The counter of the operation identifier {:?} is not a number.", text;
			Invalid, Input, Mismatch)),
		};
		Ok(Self::new(ReplicaId::new(replica), counter))
	}
}


/// Names one byte of content: the operation that created the run it belongs to,
/// and the byte's offset within that run.
///
/// The name is computed, never minted: a splice inserting a thousand bytes
/// brings a thousand content identifiers into existence at the cost of the one
/// operation identifier it already has. A byte keeps its name for as long as the
/// history does, wherever the byte is later placed.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContentId {
	pub op:		OpId,	// the operation that created the byte
	pub off:	u64,	// offset of the byte within that operation's inserted run
}

impl ContentId {
	pub const fn new(op: OpId, off: u64) -> Self {
		Self { op, off }
	}

	/// Names a file's **origin anchor**: byte zero of the one-byte atom that the
	/// file's creation mints.
	///
	/// The byte is born dead and never renders, so nothing a reader points at can
	/// name it; what it is for is that an empty file is not empty in identifier
	/// space, and a splice into one therefore binds after a byte like every other
	/// splice does. `file` is the identity of the file, which is the identity of
	/// the [`crate::op::Op::FileCreate`] that brought it into existence.
	pub const fn origin(file: OpId) -> Self {
		Self { op: file, off: 0 }
	}

	/// The shape is `[op, off]`.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			self.op.to_dat(),
			Dat::U64(self.off),
		])
	}

	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let pair = match dat {
			Dat::List(v) if v.len() == 2 => v,
			_ => return Err(err!(
				"A ContentId expects a 2-element Dat::List, got {:?}.", dat;
			Decode, Input, Mismatch)),
		};
		let off = match &pair[1] {
			Dat::U64(n) => *n,
			other => return Err(err!(
				"A ContentId offset expects Dat::U64, got {:?}.", other;
			Decode, Input, Mismatch)),
		};
		Ok(Self {
			op:		res!(OpId::from_dat(&pair[0])),
			off,
		})
	}
}

impl fmt::Display for ContentId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}+{}", self.op, self.off)
	}
}


/// Names a half-open run `[from, to)` of content identifiers sharing one
/// creating operation.
///
/// A run is the unit in which content is spoken about: what a splice removes,
/// what a move takes with it. Naming a run costs one operation identifier and
/// two offsets however long the run is, which is why the structure's bookkeeping
/// tracks edits rather than bytes.
///
/// The bounds are private because [`ContentRange::new`] refuses a reversed
/// range, and a public field would let a struct literal build one anyway. The
/// arithmetic below subtracts the start from the end, so a reversed range is a
/// panic in a debug build and a wraparound in a release one; the invariant has
/// to hold for every range that exists, not only for those that came through
/// the constructor.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContentRange {
	op:		OpId,	// the operation that created the bytes
	from:	u64,	// first offset, inclusive
	to:		u64,	// last offset, exclusive
}

impl ContentRange {
	/// An empty range is allowed, because splitting a run at its own edge is
	/// arithmetic that should not have to be special-cased; a reversed one names
	/// nothing and is a mistake.
	pub fn new(op: OpId, from: u64, to: u64)
		-> Outcome<Self>
	{
		if to < from {
			return Err(err!(
				"A ContentRange of {}+{}..{} is reversed; the end may not precede \
				the start.", op, from, to;
			Invalid, Input, Range));
		}
		Ok(Self { op, from, to })
	}

	pub const fn op(&self) -> OpId {
		self.op
	}

	pub const fn from(&self) -> u64 {
		self.from
	}

	pub const fn to(&self) -> u64 {
		self.to
	}

	/// This is how a caller coalescing abutting runs grows one without rebuilding
	/// it, and the only way the end moves from outside this module.
	pub fn set_to(&mut self, to: u64)
		-> Outcome<()>
	{
		if to < self.from {
			return Err(err!(
				"A ContentRange of {}+{}..{} would be reversed; the end may not \
				precede the start.", self.op, self.from, to;
			Invalid, Input, Range));
		}
		self.to = to;
		Ok(())
	}

	pub const fn len(&self) -> u64 {
		self.to - self.from
	}

	pub const fn is_empty(&self) -> bool {
		self.to == self.from
	}

	pub const fn offsets(&self) -> Range<u64> {
		self.from..self.to
	}

	pub fn contains(&self, cid: &ContentId) -> bool {
		cid.op == self.op && cid.off >= self.from && cid.off < self.to
	}

	/// Ranges over different creating operations never intersect.
	pub fn intersects(&self, other: &Self) -> bool {
		self.op == other.op && self.from < other.to && other.from < self.to
	}

	pub fn intersection(&self, other: &Self)
		-> Option<Self>
	{
		if !self.intersects(other) {
			return None;
		}
		Some(Self {
			op:		self.op,
			from:	self.from.max(other.from),
			to:		self.to.min(other.to),
		})
	}

	/// The shape is `[op, from, to]`.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			self.op.to_dat(),
			Dat::U64(self.from),
			Dat::U64(self.to),
		])
	}

	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let v = match dat {
			Dat::List(v) if v.len() == 3 => v,
			_ => return Err(err!(
				"A ContentRange expects a 3-element Dat::List, got {:?}.", dat;
			Decode, Input, Mismatch)),
		};
		let mut bound = [0u64; 2];
		for (i, dat) in v[1..].iter().enumerate() {
			bound[i] = match dat {
				Dat::U64(n) => *n,
				other => return Err(err!(
					"A ContentRange bound expects Dat::U64, got {:?}.", other;
				Decode, Input, Mismatch)),
			};
		}
		Self::new(res!(OpId::from_dat(&v[0])), bound[0], bound[1])
	}
}

impl fmt::Display for ContentRange {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}+{}..{}", self.op, self.from, self.to)
	}
}


/// Which side of a byte a gap lies on.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Side {
	Before,
	After,
}

impl Side {
	pub const fn code(&self) -> u8 {
		match self {
			Self::Before	=> 0,
			Self::After		=> 1,
		}
	}

	pub fn from_code(code: u8)
		-> Outcome<Self>
	{
		match code {
			0 => Ok(Self::Before),
			1 => Ok(Self::After),
			other => Err(err!(
				"A Side code is 0 for Before or 1 for After, got {}.", other;
			Decode, Input, Invalid)),
		}
	}
}

impl fmt::Display for Side {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Before	=> write!(f, "before"),
			Self::After		=> write!(f, "after"),
		}
	}
}


/// Names a gap in a file by the byte on one side of it.
///
/// An anchor is what an edit records instead of a position. Because it names
/// content, a later move of that content carries the anchor with it, and an
/// insertion written against it lands beside the same neighbour it was written
/// beside rather than at the offset that neighbour happened to occupy.
///
/// An absent anchor -- `None` where one is expected -- means the start or the
/// end of the file, which no byte names.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Anchor {
	pub content:	ContentId,	// the byte the gap is named by
	pub side:		Side,		// which side of that byte the gap lies on
}

impl Anchor {
	pub const fn new(content: ContentId, side: Side) -> Self {
		Self { content, side }
	}

	/// The form a left origin takes.
	pub const fn after(content: ContentId) -> Self {
		Self { content, side: Side::After }
	}

	/// The form a right origin takes.
	pub const fn before(content: ContentId) -> Self {
		Self { content, side: Side::Before }
	}

	/// Constructs the anchor naming the start of a file: the gap after that
	/// file's origin anchor.
	///
	/// This is the left origin of a splice into an empty file, and it is an
	/// ordinary anchor over an ordinary content identifier. Nothing new is spelled
	/// on the wire for it; see [`ContentId::origin`].
	pub const fn origin(file: OpId) -> Self {
		Self::after(ContentId::origin(file))
	}

	/// The shape is `[content, side]`.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			self.content.to_dat(),
			Dat::U8(self.side.code()),
		])
	}

	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let pair = match dat {
			Dat::List(v) if v.len() == 2 => v,
			_ => return Err(err!(
				"An Anchor expects a 2-element Dat::List, got {:?}.", dat;
			Decode, Input, Mismatch)),
		};
		let side = match &pair[1] {
			Dat::U8(c) => res!(Side::from_code(*c)),
			other => return Err(err!(
				"An Anchor side expects Dat::U8, got {:?}.", other;
			Decode, Input, Mismatch)),
		};
		Ok(Self {
			content:	res!(ContentId::from_dat(&pair[0])),
			side,
		})
	}

	/// Absence is the start or the end of the file.
	pub fn opt_to_dat(anchor: &Option<Self>) -> Dat {
		Dat::Opt(Box::new(anchor.as_ref().map(|a| a.to_dat())))
	}

	pub fn opt_from_dat(dat: &Dat)
		-> Outcome<Option<Self>>
	{
		match dat {
			Dat::Opt(boxed) => match boxed.as_ref() {
				Some(inner)	=> Ok(Some(res!(Self::from_dat(inner)))),
				None		=> Ok(None),
			},
			other => Err(err!(
				"An optional Anchor expects Dat::Opt, got {:?}.", other;
			Decode, Input, Mismatch)),
		}
	}
}

impl fmt::Display for Anchor {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{} {}", self.side, self.content)
	}
}


#[cfg(test)]
mod tests {
	use super::*;

	/// The values worth testing at the edges of the varint encoding: zero, one,
	/// every seven bit group boundary either side, and the extremes of a `u64`.
	fn boundary_values() -> Vec<u64> {
		let mut v = vec![
			0,
			1,
			127,			// largest single byte value
			128,			// smallest two byte value
			u64::MAX,
			u64::MAX - 1,
			u64::MAX / 2,
		];
		for g in 1..10u32 {
			let edge = 1u64 << (7 * g);
			v.push(edge - 1);
			v.push(edge);
			v.push(edge + 1);
		}
		v
	}

	#[test]
	fn varint_round_trip() -> Outcome<()> {
		for n in boundary_values() {
			let mut buf = Vec::new();
			varint_encode(n, &mut buf);
			assert!(buf.len() <= VARINT_MAX_LEN, "{} encoded to {} bytes", n, buf.len());
			let (got, len) = res!(varint_decode(&buf));
			assert_eq!(got, n);
			assert_eq!(len, buf.len());
		}
		Ok(())
	}

	/// The encoding is a prefix code: a value decodes correctly with trailing
	/// bytes present, consuming only its own.
	#[test]
	fn varint_decodes_as_a_prefix() -> Outcome<()> {
		for n in boundary_values() {
			let mut buf = Vec::new();
			varint_encode(n, &mut buf);
			let used = buf.len();
			buf.extend_from_slice(b"trailing rubbish");
			let (got, len) = res!(varint_decode(&buf));
			assert_eq!(got, n);
			assert_eq!(len, used);
		}
		Ok(())
	}

	/// Lengths grow one byte per seven bits, and no further.
	#[test]
	fn varint_lengths_are_as_expected() -> Outcome<()> {
		let cases = [
			(0u64,		1usize),
			(127,		1),
			(128,		2),
			(16_383,	2),
			(16_384,	3),
			(u64::MAX,	10),
		];
		for (n, want) in cases {
			let mut buf = Vec::new();
			varint_encode(n, &mut buf);
			assert_eq!(buf.len(), want, "value {}", n);
		}
		Ok(())
	}

	#[test]
	fn varint_rejects_truncation() -> Outcome<()> {
		assert!(varint_decode(&[]).is_err());
		assert!(varint_decode(&[0x80]).is_err());
		assert!(varint_decode(&[0x80, 0x80, 0x80]).is_err());
		Ok(())
	}

	/// An overlong encoding of a value is rejected, so each value has exactly
	/// one spelling.
	#[test]
	fn varint_rejects_overlong_encodings() -> Outcome<()> {
		assert!(varint_decode(&[0x80, 0x00]).is_err());			// overlong zero
		assert!(varint_decode(&[0x81, 0x00]).is_err());			// overlong one
		assert!(varint_decode(&[0xff, 0x80, 0x00]).is_err());	// overlong 127
		Ok(())
	}

	#[test]
	fn varint_rejects_overflow() -> Outcome<()> {
		// Ten bytes whose final byte carries more than the one remaining bit.
		let too_big = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02];
		assert!(varint_decode(&too_big).is_err());
		// Eleven bytes, every one continuing.
		let too_long = [0x80; 11];
		assert!(varint_decode(&too_long).is_err());
		Ok(())
	}

	#[test]
	fn varint_encodes_the_maximum() -> Outcome<()> {
		let mut buf = Vec::new();
		varint_encode(u64::MAX, &mut buf);
		assert_eq!(buf.len(), VARINT_MAX_LEN);
		assert_eq!(buf[VARINT_MAX_LEN - 1], 0x01);
		let (got, _) = res!(varint_decode(&buf));
		assert_eq!(got, u64::MAX);
		Ok(())
	}

	#[test]
	fn op_id_byte_round_trip() -> Outcome<()> {
		for r in boundary_values() {
			for c in boundary_values() {
				let id = OpId::new(ReplicaId::new(r), c);
				let buf = id.encode();
				let back = res!(OpId::decode_all(&buf));
				assert_eq!(id, back);
			}
		}
		Ok(())
	}

	#[test]
	fn op_id_decodes_as_a_prefix() -> Outcome<()> {
		let id = OpId::new(ReplicaId::new(300), 70_000);
		let mut buf = id.encode();
		let used = buf.len();
		buf.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
		let (back, len) = res!(OpId::decode(&buf));
		assert_eq!(back, id);
		assert_eq!(len, used);
		assert!(OpId::decode_all(&buf).is_err());
		Ok(())
	}

	#[test]
	fn op_id_dat_round_trip() -> Outcome<()> {
		for r in boundary_values() {
			for c in boundary_values() {
				let id = OpId::new(ReplicaId::new(r), c);
				let back = res!(OpId::from_dat(&id.to_dat()));
				assert_eq!(id, back);
			}
		}
		Ok(())
	}

	#[test]
	fn op_id_from_dat_rejects_rubbish() -> Outcome<()> {
		assert!(OpId::from_dat(&Dat::U64(7)).is_err());
		assert!(OpId::from_dat(&Dat::List(vec![Dat::U64(1)])).is_err());
		assert!(OpId::from_dat(&Dat::List(vec![
			Dat::Str(fmt!("one")),
			Dat::U64(2),
		])).is_err());
		assert!(OpId::from_dat(&Dat::List(vec![
			Dat::U64(1),
			Dat::Str(fmt!("two")),
		])).is_err());
		Ok(())
	}

	#[test]
	fn op_id_orders_by_replica_then_counter() -> Outcome<()> {
		let a = OpId::new(ReplicaId::new(1), 9);
		let b = OpId::new(ReplicaId::new(2), 1);
		let c = OpId::new(ReplicaId::new(1), 10);
		assert!(a < b);
		assert!(a < c);
		assert!(c < b);
		Ok(())
	}

	#[test]
	fn op_id_displays_both_parts() -> Outcome<()> {
		let id = OpId::new(ReplicaId::new(4), 17);
		assert_eq!(fmt!("{}", id), "r4:17");
		Ok(())
	}

	fn an_op() -> OpId {
		OpId::new(ReplicaId::new(3), 9)
	}

	#[test]
	fn content_id_dat_round_trip() -> Outcome<()> {
		for off in boundary_values() {
			let cid = ContentId::new(an_op(), off);
			assert_eq!(cid, res!(ContentId::from_dat(&cid.to_dat())));
		}
		Ok(())
	}

	#[test]
	fn content_range_dat_round_trip() -> Outcome<()> {
		for (from, to) in [(0u64, 0u64), (0, 1), (7, 9), (0, u64::MAX)] {
			let r = res!(ContentRange::new(an_op(), from, to));
			assert_eq!(r, res!(ContentRange::from_dat(&r.to_dat())));
		}
		assert!(ContentRange::from_dat(&Dat::U64(1)).is_err());
		assert!(ContentRange::from_dat(&Dat::List(vec![
			an_op().to_dat(),
			Dat::U64(5),
		])).is_err());
		// A range whose end precedes its start is refused on the way back in.
		assert!(ContentRange::from_dat(&Dat::List(vec![
			an_op().to_dat(),
			Dat::U64(9),
			Dat::U64(2),
		])).is_err());
		Ok(())
	}

	#[test]
	fn content_range_refuses_only_reversal() -> Outcome<()> {
		assert!(ContentRange::new(an_op(), 5, 4).is_err());
		let empty = res!(ContentRange::new(an_op(), 5, 5));
		assert!(empty.is_empty());
		assert_eq!(empty.len(), 0);
		Ok(())
	}

	/// The bounds read back as they went in, since nothing else can now see
	/// them.
	#[test]
	fn content_range_reports_its_bounds() -> Outcome<()> {
		let r = res!(ContentRange::new(an_op(), 4, 11));
		assert_eq!(r.op(), an_op());
		assert_eq!(r.from(), 4);
		assert_eq!(r.to(), 11);
		assert_eq!(r.len(), 7);
		assert_eq!(r.offsets(), 4..11);
		Ok(())
	}

	/// Moving the end keeps the invariant the constructor established: forward
	/// or back to the start is allowed, past it is not, and a refusal leaves the
	/// range as it was.
	#[test]
	fn content_range_end_may_not_pass_its_start() -> Outcome<()> {
		let mut r = res!(ContentRange::new(an_op(), 4, 11));
		res!(r.set_to(20));
		assert_eq!(r.len(), 16);
		res!(r.set_to(4));
		assert!(r.is_empty());
		assert!(r.set_to(3).is_err());
		assert_eq!(r.to(), 4, "a refused move leaves the range alone");
		Ok(())
	}

	/// Containment and intersection read the half-open bounds, and neither
	/// crosses from one creating operation to another.
	#[test]
	fn content_range_arithmetic_is_half_open() -> Outcome<()> {
		let op = an_op();
		let other = OpId::new(ReplicaId::new(4), 1);
		let r = res!(ContentRange::new(op, 10, 20));
		assert!(!r.contains(&ContentId::new(op, 9)));
		assert!(r.contains(&ContentId::new(op, 10)));
		assert!(r.contains(&ContentId::new(op, 19)));
		assert!(!r.contains(&ContentId::new(op, 20)), "the end is exclusive");
		assert!(!r.contains(&ContentId::new(other, 15)),
			"a byte of another atom is never in this range");
		assert!(r.intersects(&res!(ContentRange::new(op, 15, 25))));
		assert!(!r.intersects(&res!(ContentRange::new(op, 20, 30))),
			"abutting ranges do not intersect");
		assert!(!r.intersects(&res!(ContentRange::new(other, 10, 20))));
		assert_eq!(
			r.intersection(&res!(ContentRange::new(op, 15, 25))),
			Some(res!(ContentRange::new(op, 15, 20))),
		);
		assert_eq!(r.intersection(&res!(ContentRange::new(op, 30, 40))), None);
		Ok(())
	}

	#[test]
	fn anchor_dat_round_trip() -> Outcome<()> {
		let cid = ContentId::new(an_op(), 4);
		for a in [Anchor::after(cid), Anchor::before(cid)] {
			assert_eq!(a, res!(Anchor::from_dat(&a.to_dat())));
			let opt = Some(a);
			assert_eq!(opt, res!(Anchor::opt_from_dat(&Anchor::opt_to_dat(&opt))));
		}
		let none: Option<Anchor> = None;
		assert_eq!(none, res!(Anchor::opt_from_dat(&Anchor::opt_to_dat(&none))));
		assert!(Anchor::from_dat(&Dat::List(vec![
			cid.to_dat(),
			Dat::U8(7),
		])).is_err());
		assert!(Anchor::opt_from_dat(&Dat::U8(0)).is_err());
		Ok(())
	}

	/// A file's origin anchor is byte zero of the atom its creation mints, and
	/// the anchor naming the start of that file binds after it.
	#[test]
	fn the_origin_anchor_is_an_ordinary_name() -> Outcome<()> {
		let file = an_op();
		assert_eq!(ContentId::origin(file), ContentId::new(file, 0));
		assert_eq!(Anchor::origin(file), Anchor::after(ContentId::new(file, 0)));
		// Nothing new is spelled: it round trips as any other anchor does.
		let a = Anchor::origin(file);
		assert_eq!(a, res!(Anchor::from_dat(&a.to_dat())));
		Ok(())
	}

	#[test]
	fn content_names_display_readably() -> Outcome<()> {
		let cid = ContentId::new(an_op(), 4);
		assert_eq!(fmt!("{}", cid), "r3:9+4");
		assert_eq!(fmt!("{}", res!(ContentRange::new(an_op(), 4, 7))), "r3:9+4..7");
		assert_eq!(fmt!("{}", Anchor::after(cid)), "after r3:9+4");
		Ok(())
	}

	/// An operation identifier reads back exactly as it was written, and refuses
	/// every spelling it was not.
	///
	/// The round trip is the whole of the contract: what a person types is what
	/// some command printed, so the reader is judged against the writer and not
	/// against a grammar written out beside it. The values include the ones a
	/// digest-minted replica actually produces, which are ten digits long, and the
	/// extremes, where a lenient parser would silently wrap.
	#[test]
	fn an_operation_identifier_reads_back_as_it_was_written() -> Outcome<()> {
		use std::str::FromStr;

		for (replica, counter) in [
			(1u64, 1u64),
			(3_065_315_576, 4),
			(2_215_465_083, 5),
			(u64::MAX, u64::MAX),
			(1, u64::MAX),
		] {
			let id = OpId::new(ReplicaId::new(replica), counter);
			let text = fmt!("{}", id);
			assert_eq!(text, fmt!("r{}:{}", replica, counter));
			assert_eq!(res!(OpId::from_str(&text)), id, "the text {:?}", text);
			// And through the trait, which is how a caller reaches it.
			let parsed: OpId = res!(text.parse());
			assert_eq!(parsed, id);
		}
		// Everything the writer never wrote is refused rather than forgiven. A
		// second accepted spelling of one identifier would let the same operation
		// be written two ways in the very records that exist to be compared.
		for bad in [
			"",
			"3:4",					// the r is not decoration
			"r3",					// no counter
			"r:4",					// no replica
			"r3:",					// an empty counter
			"R3:4",					// the prefix is one character and it is lower case
			" r3:4",				// nothing is trimmed
			"r3:4 ",
			"r3:4:5",				// a counter is a number, and 4:5 is not one
			"r-3:4",				// a replica is not negative
			"r3:-4",
			"r3.4",					// the separator is a colon, which is what Display writes
			"r18446744073709551616:1",	// one past a u64, refused rather than wrapped
		] {
			assert!(OpId::from_str(bad).is_err(), "the text {:?} was accepted", bad);
		}
		Ok(())
	}
}
