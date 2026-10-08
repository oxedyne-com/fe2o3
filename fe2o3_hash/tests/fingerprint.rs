//! The pinned vectors were worked out by an independent SipHash-1-3-128 written in Python from the
//! paper (the same code gives the reference's published `a3817f04ba25a8e66df67214c7550293` for
//! SipHash-2-4-128 of nothing). A change to one of them is a change to every cache key built on it.

use oxedyne_fe2o3_hash::fingerprint::Fingerprint;
use oxedyne_fe2o3_hash::fingerprint::Fingerprinter;
use oxedyne_fe2o3_hash::fingerprint::LazyFingerprint;

use std::cell::Cell;
use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;

fn hex(b: &[u8]) -> String { Fingerprint::of(b).to_string() }

fn ramp(n: usize) -> Vec<u8> { (0..n).map(|i| i as u8).collect() }

#[test]
fn pinned_vectors_of_byte_strings() {
	assert_eq!(hex(b""),				"013030dd6adb62fdbea58827b2bc7ee7");
	assert_eq!(hex(b"a"),				"678aae375dd72f49a84f7c63de373c08");
	assert_eq!(hex(b"abc"),				"01d65f4901d5d4b2e81efa46153f7543");
	assert_eq!(hex(&ramp(15)),			"09017e1eeccd21296c52bdb205557ec1");
	assert_eq!(hex(&ramp(64)),			"c2666aba59a7c84e36a54a4bf2de5312");
	assert_eq!(hex(&ramp(255)),			"e93ebbaed065a4f1a558210ac1a428f5");
}

#[test]
fn pinned_vector_of_typed_writes() {
	// The bytes are 01, 0302, 07060504, 0f0e0d0c0b0a0908, feffffff, 01, 0000c03f, the eight bytes of
	// -0.25, fdffffffffffffff, then the length 6 and "h\xc3\xa9llo": 54 bytes, six blocks and a tail of six.
	let mut f = Fingerprinter::new();
	f.write_u8(1);
	f.write_u16(0x0203);
	f.write_u32(0x0405_0607);
	f.write_u64(0x0809_0a0b_0c0d_0e0f);
	f.write_i32(-2);
	f.write_bool(true);
	f.write_f32(1.5);
	f.write_f64(-0.25);
	f.write_i64(-3);
	f.write_str("h\u{e9}llo");
	assert_eq!(f.finish().to_string(), "c6b9ca07a4088efab5ba8ddb602e2150");
}

#[test]
fn pinned_vector_of_a_nested_part() {
	let mut f = Fingerprinter::new();
	f.write_u8(9);
	f.nest(|p| {
		p.write_str("inner");
		p.write_u32(7);
	});
	f.write_u8(10);
	assert_eq!(f.finish().to_string(), "6301e3b3ab0b3006845b8494dfc82486");

	// The part's own fingerprint, written by hand, is the same stream.
	let mut part = Fingerprinter::new();
	part.write_str("inner");
	part.write_u32(7);
	assert_eq!(part.finish().to_string(), "58c93206e007b54bba1b72eadd64e307");
	let mut g = Fingerprinter::new();
	g.write_u8(9);
	g.write_fingerprint(part.finish());
	g.write_u8(10);
	assert_eq!(g.finish(), f.finish());
}

#[test]
fn a_stream_is_the_same_however_it_is_cut() {
	let all = ramp(70);
	for n in 0..all.len() {
		let whole = Fingerprint::of(&all[..n]);
		// Every single cut.
		for cut in 0..=n {
			let mut f = Fingerprinter::new();
			f.write(&all[..cut]);
			f.write(&all[cut..n]);
			assert_eq!(f.finish(), whole, "length {} cut at {}", n, cut);
		}
		// Byte by byte.
		let mut f = Fingerprinter::new();
		for b in &all[..n] {
			f.write_u8(*b);
		}
		assert_eq!(f.finish(), whole, "length {} one byte at a time", n);
	}
}

#[test]
fn an_integer_is_its_little_endian_bytes_at_any_offset() {
	for skew in 0..9usize {
		let pad = ramp(skew);
		let mut a = Fingerprinter::new();
		a.write(&pad);
		a.write_u64(0x1122_3344_5566_7788);
		a.write_u32(0x99aa_bbcc);
		a.write_u16(0xddee);
		a.write_usize(0x0102_0304);
		a.write_i32(-5);
		a.write_i64(-6);
		a.write_f32(0.5);
		a.write_f64(2.5);
		a.write_bool(false);
		let mut bytes = pad.clone();
		bytes.extend_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
		bytes.extend_from_slice(&0x99aa_bbccu32.to_le_bytes());
		bytes.extend_from_slice(&0xddeeu16.to_le_bytes());
		bytes.extend_from_slice(&0x0102_0304u64.to_le_bytes());
		bytes.extend_from_slice(&(-5i32).to_le_bytes());
		bytes.extend_from_slice(&(-6i64).to_le_bytes());
		bytes.extend_from_slice(&0.5f32.to_bits().to_le_bytes());
		bytes.extend_from_slice(&2.5f64.to_bits().to_le_bytes());
		bytes.push(0);
		assert_eq!(a.finish(), Fingerprint::of(&bytes), "after {} bytes of padding", skew);
	}
}

#[test]
fn finish_does_not_consume_and_the_stream_goes_on() {
	let mut f = Fingerprinter::new();
	f.write(b"abc");
	let first = f.finish();
	assert_eq!(f.finish(), first);
	f.write(b"d");
	assert_eq!(first, Fingerprint::of(b"abc"));
	assert_eq!(f.finish(), Fingerprint::of(b"abcd"));
}

#[test]
fn what_is_written_matters_in_value_in_order_and_in_length() {
	let two = |a: &str, b: &str| {
		let mut f = Fingerprinter::new();
		f.write_str(a);
		f.write_str(b);
		f.finish()
	};
	assert_ne!(two("ab", "c"), two("a", "bc"));
	assert_ne!(two("a", "b"), two("b", "a"));
	assert_ne!(Fingerprint::of(&[0]), Fingerprint::of(&[0, 0]));
	assert_ne!(Fingerprint::of(&[]), Fingerprint::of(&[0]));
	let mut p = Fingerprinter::new();
	p.write_f64(f64::NAN);
	let mut q = Fingerprinter::new();
	q.write_f64(f64::from_bits(f64::NAN.to_bits() ^ 1));
	assert_ne!(p.finish(), q.finish(), "a float goes by its bits");
	// A part nested is not the same as its writes made flat.
	let mut flat = Fingerprinter::new();
	flat.write_u32(7);
	let mut nested = Fingerprinter::new();
	nested.nest(|p| p.write_u32(7));
	assert_ne!(flat.finish(), nested.finish());
}

#[test]
fn many_distinct_inputs_give_distinct_fingerprints_and_halves() {
	let mut all = HashSet::new();
	let mut los = HashSet::new();
	let mut his = HashSet::new();
	for i in 0..200_000u64 {
		let mut f = Fingerprinter::new();
		f.write_u64(i);
		let fp = f.finish();
		all.insert(fp);
		los.insert(fp.as_u128() as u64);
		his.insert((fp.as_u128() >> 64) as u64);
	}
	assert_eq!(all.len(), 200_000);
	assert_eq!(los.len(), 200_000);
	assert_eq!(his.len(), 200_000);
}

#[test]
fn a_flipped_input_bit_flips_about_half_the_output_bits() {
	let mut total = 0u32;
	let runs = 256u32;
	for i in 0..runs {
		let x = (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
		let mut a = Fingerprinter::new();
		a.write_u64(x);
		let mut b = Fingerprinter::new();
		b.write_u64(x ^ (1u64 << (i % 64)));
		total += (a.finish().as_u128() ^ b.finish().as_u128()).count_ones();
	}
	let mean = total as f64 / runs as f64;
	assert!(mean > 56.0 && mean < 72.0, "mean flipped bits {} of 128", mean);
}

#[test]
fn text_value_halves_and_fold() {
	let fp = Fingerprint::from_u128(0x0123_4567_89ab_cdef_fedc_ba98_7654_3210);
	assert_eq!(fp.to_string(), "0123456789abcdeffedcba9876543210");
	assert_eq!(format!("{:?}", fp), "Fingerprint(0123456789abcdeffedcba9876543210)");
	assert_eq!(fp.as_u128(), 0x0123_4567_89ab_cdef_fedc_ba98_7654_3210);
	assert_eq!(fp.fold(), 0x0123_4567_89ab_cdef ^ 0xfedc_ba98_7654_3210);
	assert_eq!(Fingerprint::default().as_u128(), 0);
	assert!(Fingerprint::from_u128(1) < Fingerprint::from_u128(1 << 64));
}

#[test]
fn xor_folds_a_set_in_any_order() {
	let (a, b, c) = (Fingerprint::of(b"a"), Fingerprint::of(b"b"), Fingerprint::of(b"c"));
	let mut one = Fingerprint::default();
	for x in [a, b, c] { one ^= x; }
	let mut two = Fingerprint::default();
	for x in [c, a, b] { two ^= x; }
	assert_eq!(one, two);
	assert_eq!(one, a ^ b ^ c);
	assert_eq!(one.as_u128(), a.as_u128() ^ b.as_u128() ^ c.as_u128());
	assert_ne!(one, a ^ b);
}

#[test]
fn equal_fingerprints_hash_alike() {
	let h = |fp: Fingerprint| {
		let mut s = DefaultHasher::new();
		fp.hash(&mut s);
		s.finish()
	};
	assert_eq!(h(Fingerprint::of(b"x")), h(Fingerprint::of(b"x")));
	assert_ne!(h(Fingerprint::of(b"x")), h(Fingerprint::of(b"y")));
}

#[test]
fn a_lazy_fingerprint_is_worked_out_once() {
	let calls = Cell::new(0);
	let lazy = LazyFingerprint::new();
	assert_eq!(lazy.get(), None);
	let work = || {
		calls.set(calls.get() + 1);
		Fingerprint::of(b"work")
	};
	let a = lazy.get_or_init(work);
	let b = lazy.get_or_init(work);
	assert_eq!(a, Fingerprint::of(b"work"));
	assert_eq!(a, b);
	assert_eq!(calls.get(), 1);
	assert_eq!(lazy.get(), Some(a));
}

#[test]
fn a_clone_of_a_lazy_fingerprint_is_empty_so_a_changed_clone_is_never_stale() {
	// A value with a fingerprint cached beside it.
	#[derive(Clone, Default)]
	struct Value {
		text:	String,
		fp:		LazyFingerprint,
	}
	impl Value {
		fn fingerprint(&self) -> Fingerprint { self.fp.get_or_init(|| Fingerprint::of(self.text.as_bytes())) }
	}
	let a = Value { text: "before".to_string(), fp: LazyFingerprint::new() };
	let before = a.fingerprint();
	let mut b = a.clone();
	assert_eq!(b.fp.get(), None, "the clone starts empty");
	b.text.push_str(" and after");
	assert_ne!(b.fingerprint(), before, "the changed clone reads its own text");
	assert_eq!(a.fingerprint(), before, "the original is untouched");

	// A value changed in place clears its own.
	let mut c = a.clone();
	let first = c.fingerprint();
	c.text = "changed".to_string();
	assert_eq!(c.fingerprint(), first, "nothing told the cell, so it still holds the old one");
	c.fp.clear();
	assert_ne!(c.fingerprint(), first);
}

#[test]
fn a_lazy_fingerprint_takes_no_part_in_equality_hashing_or_debug_output() {
	let a = LazyFingerprint::new();
	let b = LazyFingerprint::new();
	b.get_or_init(|| Fingerprint::of(b"set"));
	assert_eq!(a, b);
	assert_eq!(format!("{:?}", a), format!("{:?}", b));
	let h = |l: &LazyFingerprint| {
		let mut s = DefaultHasher::new();
		l.hash(&mut s);
		s.finish()
	};
	assert_eq!(h(&a), h(&b));
}
