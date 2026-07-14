//! Golden byte vectors for the BDAT binary encoding.
//!
//! `byte.rs` encodes eighty-seven daticles and decodes them again with our own decoder.  That
//! proves the two agree with each other, which they would go on doing if every length prefix were
//! little-endian, or if the code for a `u16` changed tomorrow: the suite would stay green while the
//! format underneath it moved.
//!
//! The bytes are not an implementation detail.  o3db hashes the encoding of a key to decide which
//! node owns the record, so a byte that changes is a record that moves; a value written by one
//! release and read by the next has to encode identically or it is simply lost.  Nothing outside
//! this crate can be consulted about that -- BDAT is ours -- so the vectors below are written out
//! by hand from the format's own rules, and the encoder is checked against them rather than the
//! other way round:
//!
//! - a kind code from `constant.rs`, one byte, then the payload;
//! - fixed-width integers big-endian, in two's complement where signed;
//! - a `c64` count as `C64_CODE_START + n`, where `n` is how many significant bytes follow, most
//!   significant first, so the value zero is the bare code `0x20` and no bytes at all;
//! - a string as its code, then a `c64` byte length, then UTF-8;
//! - a list as its code, then a `c64` giving the byte length of its encoded items -- their *bytes*,
//!   not their number -- then the items.
//!
//! If a vector here fails, either the encoding changed or this description of it is wrong.  Both
//! are worth stopping for.

use oxedyne_fe2o3_jdat::prelude::*;

use oxedyne_fe2o3_core::{
	prelude::*,
	test::test_it,
};

/// Encode the daticle, require the exact bytes, then decode those bytes and require the daticle
/// back.  Both directions matter: an encoder alone could be pinned by a decoder that shares its
/// mistake.
fn golden(dat: Dat, expected: &[u8], what: &str) -> Outcome<()> {
	let encoded = res!(dat.clone().to_bytes(Vec::new()));
	req!(encoded, expected.to_vec(), "Encoding {}.", what);

	let (decoded, n) = res!(Dat::from_bytes(expected));
	req!(decoded, dat, "Decoding {}.", what);
	req!(n, expected.len(), "Bytes consumed decoding {}.", what);
	Ok(())
}

pub fn test_golden_func(filter: &'static str) -> Outcome<()> {

	res!(test_it(filter, &["Golden logic", "all", "golden"], || {
		res!(golden(Dat::Empty,          &[0x00], "the empty daticle"));
		res!(golden(dat!(true),          &[0x01], "true"));
		res!(golden(dat!(false),         &[0x02], "false"));
		res!(golden(Dat::Opt(Box::new(None)), &[0x03], "none"));
		Ok(())
	}));

	res!(test_it(filter, &["Golden fixed width integers", "all", "golden"], || {
		// The code, then the value big-endian, in two's complement where the kind is signed.
		res!(golden(dat!(0u8),      &[0x0a, 0x00], "u8 zero"));
		res!(golden(dat!(255u8),    &[0x0a, 0xff], "u8 max"));
		res!(golden(dat!(1u16),     &[0x0b, 0x00, 0x01], "u16 one"));
		res!(golden(dat!(65535u16), &[0x0b, 0xff, 0xff], "u16 max"));
		res!(golden(dat!(1u32),     &[0x0c, 0x00, 0x00, 0x00, 0x01], "u32 one"));
		res!(golden(dat!(1u64),     &[0x0d, 0, 0, 0, 0, 0, 0, 0, 0x01], "u64 one"));

		res!(golden(dat!(-1i8),         &[0x10, 0xff], "i8 minus one"));
		res!(golden(dat!(i8::MIN),      &[0x10, 0x80], "i8 min"));
		res!(golden(dat!(-2i16),        &[0x11, 0xff, 0xfe], "i16 minus two"));
		res!(golden(dat!(i16::MIN),     &[0x11, 0x80, 0x00], "i16 min"));
		res!(golden(dat!(i32::MIN),     &[0x12, 0x80, 0x00, 0x00, 0x00], "i32 min"));
		Ok(())
	}));

	res!(test_it(filter, &["Golden c64", "all", "golden"], || {
		// The whole point of a c64: the code carries the length, so a small number is small.  The
		// value zero occupies one byte in total, and no payload byte at all.
		res!(golden(Dat::C64(0),            &[0x20], "c64 zero"));
		res!(golden(Dat::C64(1),            &[0x21, 0x01], "c64 one"));
		res!(golden(Dat::C64(255),          &[0x21, 0xff], "c64 255, still one byte"));
		res!(golden(Dat::C64(256),          &[0x22, 0x01, 0x00], "c64 256, now two"));
		res!(golden(Dat::C64(65_535),       &[0x22, 0xff, 0xff], "c64 65535"));
		res!(golden(Dat::C64(65_536),       &[0x23, 0x01, 0x00, 0x00], "c64 65536, now three"));
		res!(golden(Dat::C64(u64::MAX),
			&[0x28, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff], "c64 max, all eight"));
		Ok(())
	}));

	res!(test_it(filter, &["Golden strings", "all", "golden"], || {
		// The code, a c64 byte length, then UTF-8.  The length counts bytes, not characters, which
		// is why the accented character below declares two.
		res!(golden(dat!(""),       &[0x29, 0x20], "the empty string"));
		res!(golden(dat!("abc"),    &[0x29, 0x21, 0x03, 0x61, 0x62, 0x63], "the string 'abc'"));
		res!(golden(dat!("é"),      &[0x29, 0x21, 0x02, 0xc3, 0xa9], "a two byte character"));
		Ok(())
	}));

	res!(test_it(filter, &["Golden c64 minimality", "all", "golden"], || {
		// A c64 code says how many bytes follow, so the value 5 can be written 0x21 0x05, and it
		// can also be written 0x22 0x00 0x05, and 0x23 0x00 0x00 0x05, and so on.  The encoder only
		// ever writes the first.  The decoder used to read all of them, which gave one value an
		// unbounded number of valid encodings.
		//
		// That is not a tidiness complaint.  o3db hashes the encoded bytes of a key to choose the
		// node that owns the record, so a key re-encoded non-minimally by a peer hashes to a
		// different node: the same record written in one place and sought in another.  A signature
		// over a canonical encoding has the same problem from the other end.
		//
		// So a leading zero is refused, and the value keeps exactly one encoding.
		let minimal: &[u8] = &[0x21, 0x05];
		let (dat, n) = res!(Dat::from_bytes(minimal));
		req!(dat, Dat::C64(5), "The minimal encoding of five.");
		req!(n, 2);

		for padded in [
			vec![0x22, 0x00, 0x05],
			vec![0x23, 0x00, 0x00, 0x05],
			vec![0x28, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05],
		] {
			match Dat::from_bytes(&padded) {
				Ok((dat, _)) => return Err(err!(
					"The non-minimal encoding {:02x?} of five was accepted, decoding to {:?}. It \
					has a shorter encoding, so it is a second set of bytes for one value.",
					padded, dat;
				Test, Invalid)),
				Err(_) => (),
			}
		}
		Ok(())
	}));

	res!(test_it(filter, &["Golden lists", "all", "golden"], || {
		// The c64 after the list code is the byte length of the encoded items, not their number.
		res!(golden(Dat::List(vec![]), &[0x33, 0x20], "the empty list"));
		res!(golden(
			Dat::List(vec![dat!(1u8), dat!(2u8)]),
			// Two u8 daticles, two bytes each, so four bytes of items.
			&[0x33, 0x21, 0x04, 0x0a, 0x01, 0x0a, 0x02],
			"a list of two u8s",
		));
		res!(golden(
			Dat::List(vec![Dat::List(vec![dat!(1u8)])]),
			// The inner list is 0x33 0x21 0x02 0x0a 0x01, which is five bytes.
			&[0x33, 0x21, 0x05, 0x33, 0x21, 0x02, 0x0a, 0x01],
			"a list holding a list",
		));
		Ok(())
	}));

	Ok(())
}
