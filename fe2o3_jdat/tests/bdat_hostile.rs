//! Hostile BDAT input is refused and never panics.
//!
//! A decoder that reads bytes from a peer meets lengths and nesting a peer chose. These tests
//! build the bytes by hand, from the layouts in `golden.rs`, and require three things of the
//! decoder:
//!
//! - a declared length near `u64::MAX` is refused, where adding it to an offset would overflow;
//! - an `Adec` payload shorter than its own 8-byte exponent is refused, where it was sliced
//!   backwards;
//! - an item of a list, map or ordered map is cut at the container's declared payload, so a
//!   container cannot read bytes that belong to what follows it and then decode to a value that
//!   encodes to other bytes.
//!
//! Each test names the fault it holds, and turns red when that fault is put back.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
	note::NoteConfig,
	prelude::*,
};
use oxedyne_fe2o3_num::{
	BigDecimal,
	BigInt,
};

use std::panic::{
	catch_unwind,
	AssertUnwindSafe,
};

// Lengths that overflow an offset sum, and some that do not but are far past any buffer.
const HUGE: [u64; 8] = [
	u64::MAX,
	u64::MAX - 1,
	u64::MAX - 7,
	u64::MAX - 8,
	u64::MAX - 9,
	1 << 63,
	(1 << 63) - 1,
	1 << 40,
];

// The minimal `c64` of a value: a code that says how many bytes follow, then those bytes.
fn c64(v: u64) -> Vec<u8> {
	let be = v.to_be_bytes();
	let skip = be.iter().take_while(|b| **b == 0).count();
	let mut out = vec![Dat::C64_CODE_START + (8 - skip) as u8];
	out.extend_from_slice(&be[skip..]);
	out
}

// Decodes under `catch_unwind`, so a panic is a message the test can name and not an abort.
fn decode(buf: &[u8]) -> Result<Outcome<(Dat, usize)>, String> {
	match catch_unwind(AssertUnwindSafe(|| Dat::from_bytes(buf))) {
		Ok(outcome)	=> Ok(outcome),
		Err(panic)	=> {
			let msg = match panic.downcast_ref::<String>() {
				Some(s)	=> s.clone(),
				None	=> match panic.downcast_ref::<&str>() {
					Some(s)	=> s.to_string(),
					None	=> "a panic with no message".to_string(),
				},
			};
			Err(msg)
		},
	}
}

fn refused(what: &str, buf: &[u8]) {
	match decode(buf) {
		Err(msg)		=> panic!("{} made the decoder panic: {}", what, msg),
		Ok(Ok((d, n)))	=> panic!("{} was accepted as {:?} using {} bytes.", what, d, n),
		Ok(Err(_))		=> (),
	}
}

fn accepted(what: &str, buf: &[u8]) -> Dat {
	match decode(buf) {
		Err(msg)		=> panic!("{} made the decoder panic: {}", what, msg),
		Ok(Err(e))		=> panic!("{} was refused: {}", what, e),
		Ok(Ok((d, n)))	=> {
			assert_eq!(n, buf.len(), "{} used {} of {} bytes.", what, n, buf.len());
			d
		},
	}
}

fn encode(dat: &Dat) -> Vec<u8> {
	match dat.to_bytes(Vec::new()) {
		Ok(b)	=> b,
		Err(e)	=> panic!("A fixture daticle did not encode: {}", e),
	}
}

// The encoding of a container with its declared payload length cut by `by` bytes. The bytes
// themselves stay, so the last item or items lie in the buffer but outside the container.
fn shortened(dat: &Dat, by: u8) -> Vec<u8> {
	let mut b = encode(dat);
	// Every fixture's payload is under 256 bytes, so its length is the code 0x21 and one byte.
	assert_eq!(b[1], Dat::C64_CODE_START + 1, "The fixture's length is not one byte.");
	assert!(b[2] > by, "The fixture's payload is shorter than the cut.");
	b[2] -= by;
	b
}

// A kind whose length is a `c64` straight after its code.
fn huge_c64_length(code: u8, what: &str) {
	for v in HUGE {
		let mut b = vec![code];
		b.extend(c64(v));
		b.extend_from_slice(&[0u8; 16]);
		refused(&fmt!("{} declaring {} bytes", what, v), &b);
	}
}

#[test]
fn a_str_length_past_the_maximum_is_refused() {
	huge_c64_length(Dat::STR_CODE, "a Str");
}

#[test]
fn a_bc64_length_past_the_maximum_is_refused() {
	huge_c64_length(Dat::BC64_CODE, "a BC64");
}

#[test]
fn an_aint_length_past_the_maximum_is_refused() {
	huge_c64_length(Dat::AINT_CODE, "an Aint");
}

#[test]
fn an_adec_length_past_the_maximum_is_refused() {
	huge_c64_length(Dat::ADEC_CODE, "an Adec");
}

#[test]
fn an_abox_note_length_past_the_maximum_is_refused() {
	// An annotated box ends in the note's length, so cut the encoding of an empty note, a lone
	// zero byte, and put the huge length in its place.
	let abox = Dat::ABox(NoteConfig::default(), Box::new(Dat::U8(1)), String::new());
	let mut head = encode(&abox);
	assert_eq!(head.pop(), Some(Dat::C64_CODE_START), "The fixture's note is not empty.");
	accepted("an annotated box with an empty note", &encode(&abox));
	for v in HUGE {
		let mut b = head.clone();
		b.extend(c64(v));
		b.extend_from_slice(&[0u8; 16]);
		refused(&fmt!("an annotated box with a note of {} bytes", v), &b);
	}
}

#[test]
fn an_ordered_map_length_past_the_maximum_is_refused() {
	huge_c64_length(Dat::OMAP_CODE, "an ordered map");
}

#[test]
fn a_bu64_length_past_the_maximum_is_refused() {
	// The length is a raw u64 and not a c64, and a value of this kind is how the distributed
	// Ozone codec carries record bytes.
	for v in HUGE {
		let mut b = vec![Dat::BU64_CODE];
		b.extend_from_slice(&v.to_be_bytes());
		b.extend_from_slice(&[0u8; 16]);
		refused(&fmt!("a BU64 declaring {} bytes", v), &b);
	}
	accepted("a BU64 of three bytes", &encode(&Dat::BU64(vec![1, 2, 3])));
}

#[test]
fn an_adec_payload_under_eight_bytes_is_refused() {
	// The payload is the number's bytes, then an 8-byte exponent, so under eight cannot hold it.
	for v in 1..8usize {
		let mut b = vec![Dat::ADEC_CODE];
		b.extend(c64(v as u64));
		b.extend(vec![1u8; v]);
		refused(&fmt!("an Adec payload of {} bytes", v), &b);
		b.extend_from_slice(&[0u8; 16]);
		refused(&fmt!("an Adec payload of {} bytes with bytes after it", v), &b);
	}
	// Eight bytes is a zero number with an exponent, and a longer payload still decodes.
	let mut ok = vec![Dat::ADEC_CODE];
	ok.extend(c64(8));
	ok.extend_from_slice(&[0u8; 8]);
	accepted("an Adec payload of exactly eight bytes", &ok);
	let long = Dat::Adec(BigDecimal::new(BigInt::from(123456789u64), 3));
	assert_eq!(accepted("an Adec with a number and an exponent", &encode(&long)), long);
}

#[test]
fn an_adec_in_a_list_reads_only_its_own_payload() {
	// An Adec followed by another item: its exponent is the last 8 bytes of its own payload, not
	// of the buffer.
	let list = Dat::List(vec![
		Dat::Adec(BigDecimal::new(BigInt::from(5u8), 2)),
		Dat::U8(7),
	]);
	assert_eq!(accepted("a list holding an Adec then a U8", &encode(&list)), list);
}

#[test]
fn a_list_item_cannot_read_past_its_list() {
	let fixtures = [
		(Dat::List(vec![Dat::U16(1)]),							1),	// a fixed width integer
		(Dat::List(vec![Dat::U8(1), Dat::Str("abc".to_string())]),	1),	// a string
		(Dat::List(vec![Dat::List(vec![Dat::U8(1), Dat::U8(2)])]),	1),	// a list in the list
		(Dat::List(vec![Dat::BU64(vec![9, 9, 9])]),				1),	// a byte string
	];
	for (dat, by) in fixtures {
		assert_eq!(accepted("a list of the right length", &encode(&dat)), dat);
		refused(&fmt!("{:?} with its length cut by {}", dat, by), &shortened(&dat, by));
	}
}

#[test]
fn a_map_value_cannot_read_past_its_map() {
	let fixtures = [
		mapdat!{ "k" => 7u16 },
		mapdat!{ "k" => "value" },
		mapdat!{ "a" => 1u8, "k" => Dat::List(vec![Dat::U8(1)]) },
	];
	for dat in fixtures {
		assert_eq!(accepted("a map of the right length", &encode(&dat)), dat);
		refused(&fmt!("{:?} with its length cut by 1", dat), &shortened(&dat, 1));
	}
}

#[test]
fn a_map_key_cannot_read_past_its_map() {
	// A key cut off by the container leaves no room for its value, which the decoder refuses
	// either way, so this holds the refusal and not the bound.
	let dat = Dat::Map(DaticleMap::new());
	assert_eq!(accepted("an empty map", &encode(&dat)), dat);
	let dat = mapdat!{ 7u16 => 1u8 };
	assert_eq!(accepted("a map of the right length", &encode(&dat)), dat);
	for by in 1..=4u8 {
		refused(&fmt!("a map with its length cut by {}", by), &shortened(&dat, by));
	}
}

#[test]
fn an_ordered_map_value_cannot_read_past_its_map() {
	let fixtures = [
		omapdat!{ "k" => 7u16 },
		omapdat!{ "k" => "value" },
		omapdat!{ "a" => 1u8, "k" => Dat::List(vec![Dat::U8(1)]) },
	];
	for dat in fixtures {
		assert_eq!(accepted("an ordered map of the right length", &encode(&dat)), dat);
		refused(&fmt!("{:?} with its length cut by 1", dat), &shortened(&dat, 1));
	}
}

#[test]
fn an_ordered_map_key_cannot_read_past_its_map() {
	let dat = omapdat!{ 7u16 => 1u8 };
	assert_eq!(accepted("an ordered map of the right length", &encode(&dat)), dat);
	for by in 1..=4u8 {
		refused(&fmt!("an ordered map with its length cut by {}", by), &shortened(&dat, by));
	}
}

#[test]
fn a_tuple_item_is_bounded_by_the_container_it_sits_in() {
	// A tuple declares no length of its own, so it cannot overrun itself, but one inside a list
	// is cut at the list's payload like any other item.
	let tup = Dat::Tup2(Box::new([Dat::U8(1), Dat::Str("abc".to_string())]));
	assert_eq!(accepted("a tuple", &encode(&tup)), tup);
	let list = Dat::List(vec![tup]);
	assert_eq!(accepted("a list holding a tuple", &encode(&list)), list);
	refused("a list holding a tuple with its length cut by 1", &shortened(&list, 1));
}

// Valid encodings of every kind the decoder reads with a length, a count or a nest.
fn corpus() -> Vec<Dat> {
	vec![
		Dat::List(vec![Dat::U8(1), Dat::Str("hello".to_string()), Dat::U16(7)]),
		Dat::List(vec![Dat::List(vec![Dat::I8(-1), Dat::Bool(true)]), Dat::Empty]),
		mapdat!{ "k" => 42u8, "key" => "value" },
		omapdat!{ "k" => 42u8, "key" => "value" },
		Dat::Tup2(Box::new([Dat::U8(1), Dat::Str("x".to_string())])),
		Dat::Tup3(Box::new([Dat::U8(1), Dat::U16(2), Dat::U32(3)])),
		Dat::BC64(vec![1, 2, 3]),
		Dat::BU8(vec![1, 2, 3]),
		Dat::BU64(vec![4, 5, 6]),
		Dat::Aint(BigInt::from(-123456789i64)),
		Dat::Adec(BigDecimal::new(BigInt::from(123456789u64), 3)),
		Dat::ABox(NoteConfig::default(), Box::new(Dat::U8(1)), "note".to_string()),
		Dat::Opt(Box::new(Some(Dat::Str("some".to_string())))),
		Dat::Box(Box::new(Dat::List(vec![Dat::U8(1)]))),
	]
}

#[test]
fn every_one_byte_change_is_refused_or_read_within_bounds() {
	for dat in corpus() {
		let good = encode(&dat);
		for at in 0..good.len() {
			for byte in 0..=255u8 {
				let mut b = good.clone();
				b[at] = byte;
				// With bytes after the encoding, so a longer length has something to read.
				for tail in [0usize, 16] {
					let mut c = b.clone();
					c.extend(vec![0xa5u8; tail]);
					match decode(&c) {
						Err(msg)		=> panic!(
							"{:?} with byte {} set to {} and {} bytes after it made the decoder \
							panic: {}", dat, at, byte, tail, msg),
						Ok(Ok((_, n)))	=> assert!(
							n <= c.len(),
							"{:?} with byte {} set to {} used {} of {} bytes.",
							dat, at, byte, n, c.len()),
						Ok(Err(_))		=> (),
					}
				}
			}
		}
	}
}

#[test]
fn every_prefix_of_a_valid_encoding_is_refused_or_complete() {
	for dat in corpus() {
		let good = encode(&dat);
		for n in 0..good.len() {
			if let Ok((_, used)) = match decode(&good[..n]) {
				Err(msg)	=> panic!("{:?} cut to {} bytes made the decoder panic: {}", dat, n, msg),
				Ok(r)		=> r,
			} {
				assert!(used <= n, "{:?} cut to {} bytes used {}.", dat, n, used);
			}
		}
		accepted(&fmt!("{:?}", dat), &good);
	}
}
