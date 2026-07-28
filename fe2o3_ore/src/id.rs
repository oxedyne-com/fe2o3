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

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

use std::fmt;
use std::ops::Range;


/// Maximum number of bytes the varint encoding of a `u64` occupies.
pub const VARINT_MAX_LEN: usize = 10;


/// Appends the LEB128-style varint encoding of `n` to `buf`.
///
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

/// Decodes an LEB128-style varint from the front of `buf`, returning the value
/// and the number of bytes consumed.
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
	/// Constructs a replica identifier from its numeric value.
	pub const fn new(id: u64) -> Self {
		Self(id)
	}

	/// Returns the numeric value.
	pub const fn inner(&self) -> u64 {
		self.0
	}

	/// Appends the varint encoding of the identifier to `buf`.
	pub fn encode_into(&self, buf: &mut Vec<u8>) {
		varint_encode(self.0, buf)
	}

	/// Returns the varint encoding of the identifier.
	pub fn encode(&self) -> Vec<u8> {
		let mut buf = Vec::with_capacity(VARINT_MAX_LEN);
		self.encode_into(&mut buf);
		buf
	}

	/// Decodes an identifier from the front of `buf`, returning it and the
	/// number of bytes consumed.
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
	/// The replica that authored the operation.
	pub replica:	ReplicaId,
	/// That replica's own count, starting at one.
	pub counter:	u64,
}

impl OpId {
	/// Constructs an operation identifier.
	pub const fn new(replica: ReplicaId, counter: u64) -> Self {
		Self { replica, counter }
	}

	/// Appends the encoding of the identifier to `buf`, as replica then
	/// counter, each a varint.
	pub fn encode_into(&self, buf: &mut Vec<u8>) {
		self.replica.encode_into(buf);
		varint_encode(self.counter, buf);
	}

	/// Returns the encoding of the identifier.
	pub fn encode(&self) -> Vec<u8> {
		let mut buf = Vec::with_capacity(2 * VARINT_MAX_LEN);
		self.encode_into(&mut buf);
		buf
	}

	/// Decodes an identifier from the front of `buf`, returning it and the
	/// number of bytes consumed.
	pub fn decode(buf: &[u8])
		-> Outcome<(Self, usize)>
	{
		let (replica, n1) = res!(ReplicaId::decode(buf));
		let (counter, n2) = res!(varint_decode(&buf[n1..]));
		Ok((Self { replica, counter }, n1 + n2))
	}

	/// Decodes an identifier that must occupy the whole of `buf`.
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

	/// Serialises the identifier to a [`Dat`]. The shape is
	/// `[replica, counter]`.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			Dat::U64(self.replica.inner()),
			Dat::U64(self.counter),
		])
	}

	/// Reconstructs an identifier from a [`Dat`] produced by [`OpId::to_dat`].
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


/// Names one byte of content: the operation that created the run it belongs to,
/// and the byte's offset within that run.
///
/// The name is computed, never minted: a splice inserting a thousand bytes
/// brings a thousand content identifiers into existence at the cost of the one
/// operation identifier it already has. A byte keeps its name for as long as the
/// history does, wherever the byte is later placed.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContentId {
	/// The operation that created the byte.
	pub op:		OpId,
	/// Offset of the byte within that operation's inserted run.
	pub off:	u64,
}

impl ContentId {
	/// Constructs a content identifier.
	pub const fn new(op: OpId, off: u64) -> Self {
		Self { op, off }
	}

	/// Serialises the identifier to a [`Dat`]. The shape is `[op, off]`.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			self.op.to_dat(),
			Dat::U64(self.off),
		])
	}

	/// Reconstructs an identifier from a [`Dat`] produced by
	/// [`ContentId::to_dat`].
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
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContentRange {
	/// The operation that created the bytes.
	pub op:		OpId,
	/// First offset, inclusive.
	pub from:	u64,
	/// Last offset, exclusive.
	pub to:		u64,
}

impl ContentRange {
	/// Constructs a content range, refusing one whose end precedes its start.
	///
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

	/// Returns the number of bytes the range names.
	pub const fn len(&self) -> u64 {
		self.to - self.from
	}

	/// Reports whether the range names no bytes.
	pub const fn is_empty(&self) -> bool {
		self.to == self.from
	}

	/// Returns the offsets as a half-open range, for interval bookkeeping.
	pub const fn offsets(&self) -> Range<u64> {
		self.from..self.to
	}

	/// Reports whether the range names the given byte.
	pub fn contains(&self, cid: &ContentId) -> bool {
		cid.op == self.op && cid.off >= self.from && cid.off < self.to
	}

	/// Reports whether two ranges name at least one byte in common.
	pub fn intersects(&self, other: &Self) -> bool {
		self.op == other.op && self.from < other.to && other.from < self.to
	}

	/// Returns the bytes two ranges have in common, if any.
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

	/// Serialises the range to a [`Dat`]. The shape is `[op, from, to]`.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			self.op.to_dat(),
			Dat::U64(self.from),
			Dat::U64(self.to),
		])
	}

	/// Reconstructs a range from a [`Dat`] produced by [`ContentRange::to_dat`].
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
	/// The gap immediately preceding the byte.
	Before,
	/// The gap immediately following the byte.
	After,
}

impl Side {
	/// Returns the wire code for the side.
	pub const fn code(&self) -> u8 {
		match self {
			Self::Before	=> 0,
			Self::After		=> 1,
		}
	}

	/// Reconstructs a side from its wire code.
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
	/// The byte the gap is named by.
	pub content:	ContentId,
	/// Which side of that byte the gap lies on.
	pub side:		Side,
}

impl Anchor {
	/// Constructs an anchor.
	pub const fn new(content: ContentId, side: Side) -> Self {
		Self { content, side }
	}

	/// Constructs the anchor immediately following a byte, which is the form a
	/// left origin takes.
	pub const fn after(content: ContentId) -> Self {
		Self { content, side: Side::After }
	}

	/// Constructs the anchor immediately preceding a byte, which is the form a
	/// right origin takes.
	pub const fn before(content: ContentId) -> Self {
		Self { content, side: Side::Before }
	}

	/// Serialises the anchor to a [`Dat`]. The shape is `[content, side]`.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			self.content.to_dat(),
			Dat::U8(self.side.code()),
		])
	}

	/// Reconstructs an anchor from a [`Dat`] produced by [`Anchor::to_dat`].
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

	/// Serialises an optional anchor to a [`Dat`], absence being the start or
	/// the end of the file.
	pub fn opt_to_dat(anchor: &Option<Self>) -> Dat {
		Dat::Opt(Box::new(anchor.as_ref().map(|a| a.to_dat())))
	}

	/// Reconstructs an optional anchor from a [`Dat`] produced by
	/// [`Anchor::opt_to_dat`].
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
			127,			// Largest single byte value.
			128,			// Smallest two byte value.
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

	/// Every boundary value survives a varint round trip and consumes exactly
	/// the bytes it wrote.
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

	/// A truncated varint is an error, not a silent short read.
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
		assert!(varint_decode(&[0x80, 0x00]).is_err());			// Overlong zero.
		assert!(varint_decode(&[0x81, 0x00]).is_err());			// Overlong one.
		assert!(varint_decode(&[0xff, 0x80, 0x00]).is_err());	// Overlong 127.
		Ok(())
	}

	/// A varint too large for a `u64` is rejected rather than wrapped.
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

	/// The maximal `u64` occupies ten bytes with a final byte of one.
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

	/// Every combination of boundary replica and counter survives the byte
	/// round trip.
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

	/// An identifier decodes from the front of a longer buffer, consuming only
	/// its own bytes, and `decode_all` refuses the same buffer.
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

	/// Every combination of boundary replica and counter survives the [`Dat`]
	/// round trip.
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

	/// A malformed [`Dat`] is refused.
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

	/// Identifiers order by replica first, then by counter.
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

	/// The display form names both parts.
	#[test]
	fn op_id_displays_both_parts() -> Outcome<()> {
		let id = OpId::new(ReplicaId::new(4), 17);
		assert_eq!(fmt!("{}", id), "r4:17");
		Ok(())
	}

	/// A sample operation identifier, for the content identifier tests.
	fn an_op() -> OpId {
		OpId::new(ReplicaId::new(3), 9)
	}

	/// Content identifiers survive a [`Dat`] round trip at the boundaries of the
	/// offset.
	#[test]
	fn content_id_dat_round_trip() -> Outcome<()> {
		for off in boundary_values() {
			let cid = ContentId::new(an_op(), off);
			assert_eq!(cid, res!(ContentId::from_dat(&cid.to_dat())));
		}
		Ok(())
	}

	/// Content ranges survive a [`Dat`] round trip, and a malformed one is
	/// refused.
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

	/// A reversed range is refused, and an empty one is not.
	#[test]
	fn content_range_refuses_only_reversal() -> Outcome<()> {
		assert!(ContentRange::new(an_op(), 5, 4).is_err());
		let empty = res!(ContentRange::new(an_op(), 5, 5));
		assert!(empty.is_empty());
		assert_eq!(empty.len(), 0);
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

	/// Anchors, present and absent, survive a [`Dat`] round trip, and a side is
	/// not invented from an unknown code.
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

	/// The display forms name the operation, the offset and the side.
	#[test]
	fn content_names_display_readably() -> Outcome<()> {
		let cid = ContentId::new(an_op(), 4);
		assert_eq!(fmt!("{}", cid), "r3:9+4");
		assert_eq!(fmt!("{}", res!(ContentRange::new(an_op(), 4, 7))), "r3:9+4..7");
		assert_eq!(fmt!("{}", Anchor::after(cid)), "after r3:9+4");
		Ok(())
	}
}
