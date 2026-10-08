//! A buffer cannot make the decoder build more values than the caller allows, and a refusal
//! about a map key does not repeat the key.
//!
//! A one-byte value such as `Dat::EMPTY_CODE` costs a whole `Dat` in memory, so a 16 MiB list of
//! them decodes to some 1 GiB when only the buffer's length is bounded. `DecodeLimits::max_items`
//! counts the values as they are built, containers and scalars alike, and refuses the first one
//! past the cap, so the decoder stops allocating at the cap and not after the buffer is spent.
//!
//! A map refusal once formatted the key it had decoded with `{:?}`. A key is as long as the peer
//! chose, so a megabyte key made an error of several megabytes.
//!
//! Each test names the fault it holds, and turns red when that fault is put back.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
	bdat::DecodeLimits,
	note::NoteConfig,
	prelude::*,
};


// The minimal `c64` of a value: a code that says how many bytes follow, then those bytes.
fn c64(v: u64) -> Vec<u8> {
	let be = v.to_be_bytes();
	let skip = be.iter().take_while(|b| **b == 0).count();
	let mut out = vec![Dat::C64_CODE_START + (8 - skip) as u8];
	out.extend_from_slice(&be[skip..]);
	out
}

// A list of `n` one-byte values, built by hand since the encoder would build `n` `Dat`s first.
fn list_of_empties(n: usize) -> Vec<u8> {
	let mut b = vec![Dat::LIST_CODE];
	b.extend(c64(n as u64));
	b.resize(b.len() + n, Dat::EMPTY_CODE);
	b
}

// The number of values in a tree, counted by the test and not by the decoder.
fn values(dat: &Dat) -> usize {
	1 + match dat {
		Dat::List(v)		=> v.iter().map(values).sum::<usize>(),
		Dat::Map(m)			=> m.iter().map(|(k, v)| values(k) + values(v)).sum::<usize>(),
		Dat::OrdMap(m)		=> m.iter().map(|(k, v)| values(k.dat()) + values(v)).sum::<usize>(),
		Dat::Tup2(t)		=> t.iter().map(values).sum::<usize>(),
		Dat::Tup3(t)		=> t.iter().map(values).sum::<usize>(),
		Dat::Box(d)			=> values(d),
		Dat::Opt(o)			=> match &**o { Some(d) => values(d), None => 0 },
		Dat::ABox(_, d, _)	=> values(d),
		_					=> 0,
	}
}

fn limited(buf: &[u8], max_items: usize) -> Outcome<(Dat, usize)> {
	Dat::from_bytes_limited(buf, &DecodeLimits::default().with_max_items(max_items))
}

fn refusal(buf: &[u8], max_items: usize) -> Outcome<String> {
	match limited(buf, max_items) {
		Ok((d, _))	=> Err(err!(
			"A cap of {} values accepted a {:?}.", max_items, d.kind(); Test, Invalid, Data)),
		Err(e)		=> Ok(fmt!("{}", e)),
	}
}

fn named(msg: &str, parts: &[&str]) {
	for p in parts {
		assert!(msg.contains(p), "The refusal '{}' does not name '{}'.", msg, p);
	}
}


// The value cap.

#[test]
fn a_list_of_values_fits_a_cap_of_its_own_count_and_not_one_less() -> Outcome<()> {
	// The list is a value of its own, so ninety-nine items make a hundred values.
	let (dat, _) = res!(limited(&list_of_empties(99), 100));
	assert_eq!(values(&dat), 100);
	let msg = res!(refusal(&list_of_empties(100), 100));
	named(&msg, &["value number 101", "maximum of 100 values"]);
	Ok(())
}

#[test]
fn a_16_mib_list_of_one_byte_values_is_refused_at_the_cap() -> Outcome<()> {
	let buf = list_of_empties(16 * 1024 * 1024 - 8);
	assert!(buf.len() <= 16 * 1024 * 1024, "The fixture is {} bytes.", buf.len());
	// The count in the refusal is the cap plus one, so the decoder stopped there and did not
	// build the sixteen million values and then count them.
	let msg = res!(refusal(&buf, 1_000));
	named(&msg, &["value number 1001", "maximum of 1000 values"]);
	Ok(())
}

#[test]
fn every_kind_of_container_counts_the_values_inside_it() -> Outcome<()> {
	let tree = Dat::List(vec![
		mapdat!{ "k" => Dat::List(vec![Dat::U8(1), Dat::U8(2)]) },
		omapdat!{ "a" => 1u8, "b" => Dat::Empty },
		Dat::Tup2(Box::new([Dat::U8(1), Dat::Str("x".to_string())])),
		Dat::Tup3(Box::new([Dat::U8(1), Dat::U16(2), Dat::Empty])),
		Dat::Box(Box::new(Dat::Opt(Box::new(Some(Dat::U8(7)))))),
		Dat::ABox(NoteConfig::default(), Box::new(Dat::U8(1)), "note".to_string()),
		Dat::Opt(Box::new(None)),
	]);
	let want = values(&tree);
	let buf = res!(tree.to_bytes(Vec::new()));
	let (back, _) = res!(limited(&buf, want));
	assert_eq!(back, tree);
	let msg = res!(refusal(&buf, want - 1));
	named(&msg, &[&fmt!("value number {}", want)]);
	// The unlimited decode counts too, and never refuses.
	let (back, _) = res!(Dat::from_bytes(&buf));
	assert_eq!(back, tree);
	Ok(())
}

#[test]
fn no_limit_is_set_unless_the_caller_sets_one() {
	assert_eq!(DecodeLimits::default().max_items, usize::MAX);
	assert_eq!(DecodeLimits::new(8, 1024).max_items, usize::MAX);
	assert_eq!(DecodeLimits::text().max_items, usize::MAX);
	assert_eq!(DecodeLimits::UNLIMITED.max_items, usize::MAX);
}


// The map-key refusals.

// A map or ordered map holding one key, a BU64 of `n` bytes, and no value after it.
fn map_with_a_key_and_no_value(code: u8, n: usize) -> Vec<u8> {
	let mut key = vec![Dat::BU64_CODE];
	key.extend_from_slice(&(n as u64).to_be_bytes());
	key.resize(key.len() + n, 0xAB);
	let mut b = vec![code];
	b.extend(c64(key.len() as u64));
	b.extend(key);
	b
}

#[test]
fn a_map_refusal_for_a_megabyte_key_stays_short() -> Outcome<()> {
	let buf = map_with_a_key_and_no_value(Dat::MAP_CODE, 1 << 20);
	let msg = match Dat::from_bytes(&buf) {
		Ok((d, _))	=> return Err(err!("A map with a key and no value decoded to {:?}.", d.kind(); Test, Invalid, Data)),
		Err(e)		=> fmt!("{}", e),
	};
	assert!(msg.len() < 600, "The refusal is {} characters, and should not repeat the key.", msg.len());
	// The key is a code, an eight-byte length and the megabyte.
	named(&msg, &["BU64", "(1048585 bytes)", "decoding 0 key-value pairs"]);
	Ok(())
}

#[test]
fn an_ordered_map_refusal_for_a_megabyte_key_stays_short() -> Outcome<()> {
	let buf = map_with_a_key_and_no_value(Dat::OMAP_CODE, 1 << 20);
	let msg = match Dat::from_bytes(&buf) {
		Ok((d, _))	=> return Err(err!("An ordered map with a key and no value decoded to {:?}.", d.kind(); Test, Invalid, Data)),
		Err(e)		=> fmt!("{}", e),
	};
	assert!(msg.len() < 600, "The refusal is {} characters, and should not repeat the key.", msg.len());
	named(&msg, &["BU64", "(1048585 bytes)", "decoding 0 key-value pairs"]);
	Ok(())
}

#[test]
fn a_short_key_is_still_named_in_full() -> Outcome<()> {
	// A key that is cheap to print is printed, so the refusal says which key lacked its value.
	let mut buf = vec![Dat::MAP_CODE];
	let key = res!(Dat::Str("answer".to_string()).to_bytes(Vec::new()));
	buf.extend(c64(key.len() as u64));
	buf.extend(&key);
	let msg = match Dat::from_bytes(&buf) {
		Ok((d, _))	=> return Err(err!("A map with a key and no value decoded to {:?}.", d.kind(); Test, Invalid, Data)),
		Err(e)		=> fmt!("{}", e),
	};
	named(&msg, &["answer", &fmt!("({} bytes)", key.len())]);
	assert!(!msg.contains("..."), "A short key was cut: {}", msg);
	Ok(())
}
