//! A canonical-JSON decode refuses every text `Dat::json_canonical` would not have written.
//!
//! Each refusal below is accepted by the plain JSON configuration, which is why the option exists:
//! a field a signature or a hash covers must have exactly one text, so `"\u0041"` and `"A"` cannot
//! both be read as the same record.

use oxedyne_fe2o3_jdat::{
	prelude::*,
	string::dec::DecoderConfig,
};

use oxedyne_fe2o3_core::prelude::*;


fn plain(s: &str) -> Outcome<Dat> {
	Dat::decode_string_with_config(s, &DecoderConfig::<(), ()>::json(None))
}

fn canonical(s: &str) -> Outcome<Dat> {
	Dat::decode_string_with_config(s, &DecoderConfig::<(), ()>::json_canonical(None))
}

/// Texts the plain JSON decoder reads but the canonical form never writes.
const REFUSED: &[(&str, &str)] = &[
	(r#"{"a":"x\/y"}"#,				"escaped solidus"),
	(r#"{"a":"\u0041"}"#,				"printable ASCII as \\u"),
	(r#"{"a":"\u00e9"}"#,				"non-ASCII as \\u"),
	(r#"{"a":"\ud83d\ude00"}"#,			"surrogate pair as \\u"),
	(r#"{"a":"\u000a"}"#,				"newline as \\u, not \\n"),
	(r#"{"a":"\u001F"}"#,				"control escape in upper-case hex"),
	(r#"{"b":1,"a":2}"#,				"keys out of order"),
	(r#"{"a": 1}"#,					"space after a colon"),
	(r#"{"a":1, "b":2}"#,				"space after a comma"),
	(r#"[1,2 ]"#,					"space before a bracket"),
	(" {\"a\":1}",					"leading whitespace"),
	("{\"a\":1}\n",					"trailing newline"),
	(r#"{"a":-0}"#,					"negative zero"),
	(r#"{"a":1.0}"#,				"a float"),
	(r#"{"a":1e2}"#,				"an exponent"),
];

#[test]
fn canonical_refuses_what_the_writer_never_writes() -> Outcome<()> {
	for (text, why) in REFUSED {
		if let Err(e) = plain(text) {
			// Not every case is a gap in the plain decoder, but each must be refused here.
			println!("plain json already refuses {} ({}): {}", text, why, e);
		}
		match canonical(text) {
			Ok(dat) => return Err(err!(
				"The canonical decode accepted {:?} ({}) as {:?}.", text, why, dat;
				Test, Unexpected)),
			Err(e) => println!("refused {} ({}): {}", text, why, e),
		}
	}
	Ok(())
}

/// The gap the option fills: the plain decoder reads these as the same value as their canonical
/// text, so a field has more than one text.
#[test]
fn plain_json_reads_the_escapes_as_their_characters() -> Outcome<()> {
	assert_eq!(res!(plain(r#""\u0041\/""#)), dat!("A/"));
	assert_eq!(res!(plain(r#"{"b":1,"a":2}"#)), res!(plain(r#"{"a":2,"b":1}"#)));
	Ok(())
}

#[test]
fn canonical_refuses_a_duplicate_key() -> Outcome<()> {
	assert!(canonical(r#"{"a":1,"a":1}"#).is_err(), "a duplicate key must be refused");
	Ok(())
}

#[test]
fn canonical_refuses_jdat_forms() -> Outcome<()> {
	for text in ["(u8|1)", "{\"a\":(u8|1)}", "{\"a\":1,}"] {
		assert!(canonical(text).is_err(), "{:?} must be refused", text);
	}
	Ok(())
}

/// Every text the writer produces reads back to the same value, and that value writes the same
/// text again.
#[test]
fn canonical_text_round_trips() -> Outcome<()> {
	let texts = [
		"null",
		"true",
		"-17",
		"9007199254740991",
		"\"\"",
		"[]",
		"{}",
		"\"a\\\"b\\\\c\\nd\\re\\tf\\bg\\fh\\u001fi\\u0000\"",
		"\"Cœur — 日本 😀 /\"",
		"{\"\\n\":\"Newline\",\"\\u000b\":\"Vertical Tab\",\"\\r\":\"CR\",\"1\":\"One\",\
			\"\u{80}\":\"Control\",\"\u{f6}\":\"o\",\"\u{20ac}\":\"Euro\"}",
		"{\"age_band\":\"adult\",\"captures\":{\"face\":\"aG91c2U\",\"hands\":[1,-2,null]},\
			\"evidence\":\"\",\"place\":{\"cell\":null,\"how\":\"device\"}}",
	];
	for text in texts {
		let dat = res!(canonical(text));
		assert_eq!(res!(dat.json_canonical()), text);
		assert_eq!(dat, res!(plain(text)));
	}
	Ok(())
}

#[test]
fn canonical_refusal_names_the_position() -> Outcome<()> {
	match canonical(r#"{"a":"x\/y"}"#) {
		Ok(dat) => Err(err!("Accepted an escaped solidus as {:?}.", dat; Test, Unexpected)),
		Err(e) => {
			let msg = fmt!("{}", e);
			assert!(msg.contains("at character 7"), "the error should name the position: {}", msg);
			Ok(())
		},
	}
}
