//! A fixed-width byte variant refuses a payload longer than its length field can state.
//!
//! `Dat::BU8` writes its length in one byte, `BU16` in two and `BU32` in four. The encoder once
//! cut the length to that width and wrote the whole payload after it, so a 300-byte `BU8` was
//! written with the length 44 and read back as 44 bytes: no error on either side, and the bytes
//! after the cut were taken for the next item. These tests require the encoder to refuse, at the
//! width's boundary, and `Dat::wrap_dat` to choose a variant that always fits.
//!
//! Each test names the fault it holds, and turns red when that fault is put back.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;


fn payload(n: usize) -> Vec<u8> {
	(0..n).map(|i| (i % 251) as u8).collect()
}

fn encode(dat: &Dat) -> Outcome<Vec<u8>> {
	dat.to_bytes(Vec::new())
}

fn round_trip(dat: &Dat) -> Outcome<Dat> {
	let buf = res!(encode(dat));
	let (back, used) = res!(Dat::from_bytes(&buf));
	if used != buf.len() {
		return Err(err!(
			"Decoding {} encoded bytes used {}.", buf.len(), used;
		Test, Mismatch, Size));
	}
	Ok(back)
}

/// The message of the refusal, or an error when the encoder accepted the value.
fn refusal(dat: &Dat) -> Outcome<String> {
	match encode(dat) {
		Ok(buf)	=> Err(err!(
			"The encoder accepted a payload its length field cannot state, writing {} bytes.",
			buf.len();
		Test, Invalid, Data)),
		Err(e)	=> Ok(fmt!("{}", e)),
	}
}

fn check_names(msg: &str, parts: &[&str]) -> Outcome<()> {
	for p in parts {
		if !msg.contains(p) {
			return Err(err!(
				"The refusal '{}' does not name '{}'.", msg, p;
			Test, Missing, Data));
		}
	}
	Ok(())
}


// A BU8 length field holds 255.

#[test]
fn bu8_holds_255_and_refuses_256() -> Outcome<()> {
	let full = Dat::BU8(payload(255));
	let buf = res!(encode(&full));
	assert_eq!(buf[1], 255, "the length byte of a 255-byte BU8");
	assert_eq!(buf.len(), 1 + 1 + 255);
	assert_eq!(res!(round_trip(&full)), full);

	let msg = res!(refusal(&Dat::BU8(payload(256))));
	res!(check_names(&msg, &["BU8", "256", "255"]));
	Ok(())
}

#[test]
fn bu8_refuses_300_bytes_that_were_read_back_as_44() -> Outcome<()> {
	res!(refusal(&Dat::BU8(payload(300))));
	Ok(())
}

// A BU16 length field holds 65,535.

#[test]
fn bu16_holds_65535_and_refuses_65536() -> Outcome<()> {
	let full = Dat::BU16(payload(65_535));
	let buf = res!(encode(&full));
	assert_eq!(&buf[1..3], &[0xFF, 0xFF], "the length bytes of a 65,535-byte BU16");
	assert_eq!(buf.len(), 1 + 2 + 65_535);
	assert_eq!(res!(round_trip(&full)), full);

	let msg = res!(refusal(&Dat::BU16(payload(65_536))));
	res!(check_names(&msg, &["BU16", "65536", "65535"]));
	Ok(())
}

// A BU32 length field holds 4,294,967,295. The vector is zeroed in one allocation, so its pages
// are mapped and never touched while the encoder reads only its length and refuses.

#[cfg(target_pointer_width = "64")]
#[test]
fn bu32_refuses_a_length_past_four_gibibytes() -> Outcome<()> {
	let n = u32::MAX as usize + 1;
	let msg = res!(refusal(&Dat::BU32(vec![0u8; n])));
	res!(check_names(&msg, &["BU32", "4294967296", "4294967295"]));
	Ok(())
}

#[test]
fn bu32_holds_a_length_a_bu16_cannot() -> Outcome<()> {
	let big = Dat::BU32(payload(70_000));
	assert_eq!(res!(round_trip(&big)), big);
	Ok(())
}

// BU64 states any length a vector can have.

#[test]
fn bu64_holds_what_bu8_cannot() -> Outcome<()> {
	let big = Dat::BU64(payload(300));
	assert_eq!(res!(round_trip(&big)), big);
	Ok(())
}

// The refusal reaches the caller through a container, where a value is usually met.

#[test]
fn a_refusal_inside_a_list_or_map_reaches_the_caller() -> Outcome<()> {
	let in_list = Dat::List(vec![Dat::U8(1), Dat::BU8(payload(256))]);
	res!(refusal(&in_list));

	let mut map = DaticleMap::new();
	map.insert(dat!("k"), Dat::BU16(payload(65_536)));
	res!(refusal(&Dat::Map(map)));
	Ok(())
}

// `wrap_dat` picks the narrowest variant that fits, so a payload it wraps is never refused.

#[test]
fn wrap_dat_chooses_a_width_that_fits() -> Outcome<()> {
	for n in [0usize, 1, 254, 255, 256, 65_534, 65_535, 65_536, 70_000] {
		let dat = Dat::wrap_dat(payload(n));
		assert_eq!(res!(round_trip(&dat)), dat, "a wrapped payload of {} bytes", n);
		let bytes = match &dat {
			Dat::BU8(b) | Dat::BU16(b) | Dat::BU32(b) | Dat::BU64(b)	=> b.clone(),
			other	=> return Err(err!(
				"wrap_dat gave {:?} for {} bytes, which is not a byte variant.", other, n;
			Test, Unexpected, Data)),
		};
		assert_eq!(bytes, payload(n));
	}
	Ok(())
}

#[test]
fn wrap_bytes_var_chooses_a_width_that_fits() -> Outcome<()> {
	for n in [0usize, 254, 255, 256, 65_534, 65_535, 65_536, 70_000] {
		let wrapped = res!(Dat::wrap_bytes_var(payload(n)));
		let (back, used) = res!(Dat::from_bytes(&wrapped));
		assert_eq!(used, wrapped.len());
		let got = match back {
			Dat::BU8(b) | Dat::BU16(b) | Dat::BU32(b) | Dat::BU64(b)	=> b,
			other	=> return Err(err!(
				"wrap_bytes_var gave {:?} for {} bytes, which is not a byte variant.", other, n;
			Test, Unexpected, Data)),
		};
		assert_eq!(got, payload(n), "a wrapped payload of {} bytes", n);
	}
	Ok(())
}


// The text decoder. A text that names a width writes a payload the encoder would refuse, so
// the decoder refuses it too, at the parse and not when the value is stored.

fn text_bytes(kind: &str, n: usize) -> String {
	let items: Vec<String> = payload(n).iter().map(|b| fmt!("{}", b)).collect();
	fmt!("({}|[{}])", kind, items.join(", "))
}

#[test]
fn text_bu8_holds_255_and_refuses_256() -> Outcome<()> {
	let full = res!(Dat::decode_string(text_bytes("bu8", 255)));
	assert_eq!(full, Dat::BU8(payload(255)));
	// What the text decoder accepts, the encoder writes.
	assert_eq!(res!(round_trip(&full)), full);

	let msg = match Dat::decode_string(text_bytes("bu8", 256)) {
		Ok(d)	=> return Err(err!("The text decoder built {:?} wider than a BU8 states.", d.kind(); Test, Invalid, Data)),
		Err(e)	=> fmt!("{}", e),
	};
	res!(check_names(&msg, &["BU8", "256", "255"]));
	Ok(())
}

#[test]
fn text_bu8_refuses_300_bytes_that_the_encoder_would_cut_to_44() -> Outcome<()> {
	assert!(Dat::decode_string(text_bytes("bu8", 300)).is_err());
	Ok(())
}

#[test]
fn text_bu16_holds_65535_and_refuses_65536() -> Outcome<()> {
	let full = res!(Dat::decode_string(text_bytes("bu16", 65_535)));
	assert_eq!(full, Dat::BU16(payload(65_535)));
	assert_eq!(res!(round_trip(&full)), full);

	let msg = match Dat::decode_string(text_bytes("bu16", 65_536)) {
		Ok(d)	=> return Err(err!("The text decoder built {:?} wider than a BU16 states.", d.kind(); Test, Invalid, Data)),
		Err(e)	=> fmt!("{}", e),
	};
	res!(check_names(&msg, &["BU16", "65536", "65535"]));
	Ok(())
}

#[test]
fn text_bu32_and_bu64_take_what_a_narrower_width_refuses() -> Outcome<()> {
	let wide = res!(Dat::decode_string(text_bytes("bu32", 70_000)));
	assert_eq!(wide, Dat::BU32(payload(70_000)));
	assert_eq!(res!(round_trip(&wide)), wide);
	let wide = res!(Dat::decode_string(text_bytes("bu64", 300)));
	assert_eq!(wide, Dat::BU64(payload(300)));
	Ok(())
}
