//! Identifiers for replicas and for the operations they author.
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

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

use std::fmt;


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
}
