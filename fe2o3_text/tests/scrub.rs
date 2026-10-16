//! `secret::scrub`, the content scrubber of Daimond's debug feed.
//!
//! The answers it must give are the JavaScript's, and `fe2o3_net/tests/lens.rs` holds it to the
//! JavaScript over a table of 130 cases. These tests are the edges of that table, one rule at a
//! time: the minimum length of each shape and one byte short of it, the word boundary, the
//! places where JavaScript's `\s` and its code units differ from Rust's, the entropy catch at its
//! threshold, and strings built to make a rule slow.
//!
//! Every credential is built at run time from a generator, never written out, so that the
//! scanners reading this file find nothing in it to refuse.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::secret::{
	self,
	scrub,
};

use std::time::Instant;


const ALNUM:	&str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
const B64:		&str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const STD:		&str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789/+";
const UPPER:	&str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const HEX:		&str = "0123456789abcdef";

/// `n` characters of an alphabet, from a linear congruential generator seeded by `seed`.
fn run(n: usize, seed: u32, alpha: &str) -> String {
	let a: Vec<char> = alpha.chars().collect();
	let mut h = (seed as u64 * 2_654_435_761u64) as u32;
	let mut s = String::new();
	for _ in 0..n {
		h = h.wrapping_mul(1_103_515_245).wrapping_add(12_345);
		s.push(a[((h >> 16) as usize) % a.len()]);
	}
	s
}

/// The scrub of `v`, written in place, which is what a marker is.
fn marked(kind: &str, v: &str) -> String {
	scrub::mark(kind, v)
}

/// Does the scrub of `text` carry a marker of this shape?
fn has(text: &str, kind: &str) -> bool {
	scrub::text(text, false).contains(&fmt!("[redacted {} #", kind))
}

// An opening, what may follow it, how many of those at least, and the marker's name.
const SHAPES: &[(&str, &str, &str, usize)] = &[
	("gh",		"ghp_",			ALNUM,	16),
	("gh",		"gho_",			ALNUM,	16),
	("gh",		"ghu_",			ALNUM,	16),
	("gh",		"ghs_",			ALNUM,	16),
	("gh",		"ghr_",			ALNUM,	16),
	("ghpat",	"github_pat_",	ALNUM,	20),
	("stripe",	"sk_live_",		ALNUM,	10),
	("stripe",	"sk_test_",		ALNUM,	10),
	("stripe",	"rk_live_",		ALNUM,	10),
	("stripe",	"rk_test_",		ALNUM,	10),
	("whsec",	"whsec_",		ALNUM,	16),
	("sk",		"sk-",			B64,	16),
	("aws",		"AKIA",			UPPER,	12),
	("aws",		"ASIA",			UPPER,	12),
	("aws",		"ABIA",			UPPER,	12),
	("aws",		"ACCA",			UPPER,	12),
	("aws",		"AGPA",			UPPER,	12),
	("aws",		"AIDA",			UPPER,	12),
	("aws",		"AIPA",			UPPER,	12),
	("aws",		"ANPA",			UPPER,	12),
	("aws",		"ANVA",			UPPER,	12),
	("aws",		"AROA",			UPPER,	12),
	("aws",		"APKA",			UPPER,	12),
	("gcp",		"AIza",			B64,	30),
	("slack",	"xoxa-",		ALNUM,	10),
	("slack",	"xoxb-",		ALNUM,	10),
	("slack",	"xoxe-",		ALNUM,	10),
	("slack",	"xoxp-",		ALNUM,	10),
	("slack",	"xoxr-",		ALNUM,	10),
	("slack",	"xoxs-",		ALNUM,	10),
];

#[test]
fn a_marker_is_the_shape_a_hash_and_the_length_in_utf16_units() {
	// djb2 over "abc" is 0xb885c8b, the figure `fingerprint` gives the same three characters.
	assert_eq!(marked("sk", "abc"), "[redacted sk #b885c8b/3]");
	assert_eq!(marked("x", ""), "[redacted x #1505/0]");
	// An astral character is two UTF-16 units, so it counts two and hashes as two.
	assert!(marked("x", "\u{1F600}").ends_with("/2]"), "{}", marked("x", "\u{1F600}"));
	assert_ne!(marked("x", "\u{1F600}"), marked("x", "\u{1F601}"));
	assert_ne!(marked("x", "ab"), marked("x", "ba"), "the hash separates two orders");
}

#[test]
fn each_shape_is_replaced_whole_at_its_minimum_and_not_one_byte_short() -> Outcome<()> {
	for (i, (kind, lead, alpha, min)) in SHAPES.iter().enumerate() {
		let at = run(*min, 1000 + i as u32, alpha);
		let cred = fmt!("{}{}", lead, at);
		let text = fmt!("note {} end", cred);
		req!(scrub::text(&text, false), fmt!("note {} end", marked(kind, &cred)), "at the minimum, for {}", lead);
		// A longer run is taken whole.
		let long = fmt!("{}{}", cred, run(9, 2000 + i as u32, alpha));
		req!(scrub::text(&fmt!("<{}>", long), false), fmt!("<{}>", marked(kind, &long)), "longer, for {}", lead);
		// One short is not the shape. A run that long may still be a high-entropy blob, so what is
		// asked is that this shape's marker is absent.
		let short = fmt!("note {}{} end", lead, run(*min - 1, 3000 + i as u32, alpha));
		req!(has(&short, kind), false, "one short, for {}", lead);
	}
	Ok(())
}

#[test]
fn a_tune_key_is_exactly_thirty_two_hex_digits_and_a_jwt_is_three_runs() -> Outcome<()> {
	let key = fmt!("tune-{}", run(32, 41, HEX));
	let more = run(8, 42, HEX);
	// `{32}` takes 32 and no more: what stands after it is left where it was.
	req!(scrub::text(&fmt!("a {}{} b", key, more), false), fmt!("a {}{} b", marked("tune", &key), more));
	req!(scrub::text(&fmt!("a tune-{} b", run(31, 43, HEX)), false), fmt!("a tune-{} b", run(31, 43, HEX)));
	// A JWT: the first run opens with `eyJ` and holds six more, the second six, the third four.
	for (a, b, c, hit) in [(6, 6, 4, true), (5, 6, 4, false), (6, 5, 4, false), (6, 6, 3, false), (30, 90, 43, true)] {
		let jwt = fmt!("eyJ{}.{}.{}", run(a, 51, B64), run(b, 52, B64), run(c, 53, B64));
		let text = fmt!("x {} y", jwt);
		if hit {
			req!(scrub::text(&text, false), fmt!("x {} y", marked("jwt", &jwt)), "for {:?}", (a, b, c));
		} else {
			req!(scrub::hit(&text, false), false, "for {:?}", (a, b, c));
		}
	}
	Ok(())
}

#[test]
fn a_shape_needs_a_word_boundary_before_it_and_no_other_character_will_do() -> Outcome<()> {
	for (i, (kind, lead, alpha, min)) in SHAPES.iter().enumerate() {
		let cred = fmt!("{}{}", lead, run(*min, 4000 + i as u32, alpha));
		// A word character before it, a letter, a digit or an underscore: no boundary.
		for pre in ["x", "9", "_"] {
			let text = fmt!("{}{}", pre, cred);
			req!(has(&text, kind), false, "after {:?}, for {}", pre, lead);
		}
		// A hyphen, a slash or a quote before it is a boundary, however word-like it looks.
		for pre in ["-", "/", "\"", " ", "="] {
			let text = fmt!("a{}{}", pre, cred);
			req!(scrub::text(&text, false), fmt!("a{}{}", pre, marked(kind, &cred)), "after {:?}, for {}", pre, lead);
		}
	}
	Ok(())
}

#[test]
fn a_pem_private_key_goes_to_its_footer_or_to_the_end_of_the_string() -> Outcome<()> {
	let head = |algo: &str| fmt!("{}{}{}{}", "-----BEGIN ", algo, "PRIVATE KEY", "-----");
	let foot = |algo: &str| fmt!("{}{}{}{}", "-----END ", algo, "PRIVATE KEY", "-----");
	let body = fmt!("\n{}\n{}\n", run(64, 61, STD), run(64, 62, STD));
	for algo in ["", "RSA ", "EC ", "OPENSSH ", "ENCRYPTED "] {
		let whole = fmt!("{}{}{}", head(algo), body, foot(algo));
		req!(scrub::text(&fmt!("before {} after", whole), false),
			fmt!("before {} after", marked("pem", &whole)), "closed, for {:?}", algo);
		// No footer, as when a log line was clipped: the rest of the string goes.
		let open = fmt!("{}{}", head(algo), body);
		req!(scrub::text(&fmt!("log {}", open), false), fmt!("log {}", marked("pem", &open)), "open, for {:?}", algo);
	}
	// The first footer ends it, and a second key is a second marker.
	let one = fmt!("{}{}{}", head(""), body, foot(""));
	let two = fmt!("{}\nmid\n{}", one, one);
	req!(scrub::text(&two, false), fmt!("{}\nmid\n{}", marked("pem", &one), marked("pem", &one)));
	// A certificate is no private key.
	let cert = fmt!("{}{}{}\n{}\n{}\n{}\n{}{}{}", "-----BEGIN ", "CERTIFICATE", "-----",
		run(16, 63, STD), run(16, 64, STD), run(8, 65, STD), "-----END ", "CERTIFICATE", "-----");
	req!(scrub::text(&cert, false), cert.clone());
	// The header's words are capitals, digits and spaces only.
	req!(scrub::hit(&fmt!("{}{}{}{}", "-----BEGIN ", "rsa ", "PRIVATE KEY", "-----"), false), false);
	Ok(())
}

#[test]
fn a_pair_keeps_its_name_and_marks_only_the_value() -> Outcome<()> {
	// Bearer, Basic and Token, one or more white spaces, then eight or more of the value's alphabet.
	for word in ["Bearer", "Basic", "Token"] {
		let v = run(20, 71, B64);
		req!(scrub::text(&fmt!("Authorization: {} {}", word, v), false),
			fmt!("Authorization: {} {}", word, marked("bearer", &v)), "for {}", word);
	}
	let v = run(8, 72, B64);
	req!(scrub::text(&fmt!("Bearer \t {}", v), false), fmt!("Bearer \t {}", marked("bearer", &v)));
	let seven = fmt!("Bearer {}", run(7, 73, B64));
	req!(scrub::text(&seven, false), seven.clone());
	// The word is case sensitive, and so is the boundary before it.
	req!(scrub::hit(&fmt!("bearer {}", run(20, 74, B64)), false), false);
	req!(scrub::hit(&fmt!("xBearer {}", run(20, 74, B64)), false), false);
	Ok(())
}

#[test]
fn a_url_argument_is_matched_without_regard_to_case_and_counts_utf16_units() -> Outcome<()> {
	for name in ["access_token", "refresh_token", "id_token", "token", "api_key", "api-key", "apikey", "key",
		"auth", "secret", "password", "passwd", "sig", "signature"]
	{
		for case in [name.to_string(), name.to_uppercase()] {
			let v = run(12, 81, ALNUM);
			for lead in ["?", "&", "#"] {
				req!(scrub::text(&fmt!("https://h.example/p{}{}={}&z=1", lead, case, v), false),
					fmt!("https://h.example/p{}{}={}&z=1", lead, case, marked("urlarg", &v)), "for {} {}", lead, case);
			}
		}
	}
	// Four units or more. An astral character is two, so two of them make four and one makes two.
	let two = "\u{1F600}\u{1F600}";
	req!(scrub::text(&fmt!("u?key={}&z", two), false), fmt!("u?key={}&z", marked("urlarg", two)));
	req!(marked("urlarg", two).ends_with("/4]"), true);
	req!(scrub::hit("u?key=\u{1F600}&z", false), false);
	// The value stops at a quote, an angle bracket, a bracket or any JavaScript white space.
	for stop in ["\"", "'", "<", ">", "[", "]", "#", "&", " ", "\u{00A0}", "\u{FEFF}", "\u{3000}"] {
		let v = run(6, 82, ALNUM);
		req!(scrub::text(&fmt!("?token={}{}tail", v, stop), false), fmt!("?token={}{}tail", marked("urlarg", &v), stop), "stops at {:?}", stop);
	}
	// And a name that is only part of a longer one is not the name.
	req!(scrub::hit(&fmt!("?monkey={}", run(12, 83, ALNUM)), false), false);
	req!(scrub::hit(&fmt!("?keys={}", run(12, 83, ALNUM)), false), false);
	Ok(())
}

#[test]
fn javascripts_white_space_is_not_unicodes() -> Outcome<()> {
	// U+FEFF is white space to JavaScript's `\s`, and U+0085 is not; Rust's `char::is_whitespace`
	// has it the other way round, so the port cannot lean on it.
	let v = run(12, 91, B64);
	req!(scrub::text(&fmt!("Bearer\u{FEFF}{}", v), false), fmt!("Bearer\u{FEFF}{}", marked("bearer", &v)));
	req!(scrub::text(&fmt!("Bearer\u{2028}{}", v), false), fmt!("Bearer\u{2028}{}", marked("bearer", &v)));
	req!(scrub::hit(&fmt!("Bearer\u{0085}{}", v), false), false);
	Ok(())
}

#[test]
fn a_secret_name_against_a_value_is_matched_through_up_to_three_quotes_and_spaces() -> Outcome<()> {
	for name in ["apikey", "api_key", "api-key", "auth_token", "auth-token", "authtoken", "access_token",
		"accesstoken", "token", "secret", "passphrase", "password", "master_key", "masterkey",
		"private-key", "privatekey", "mnemonic", "seed_phrase", "seedphrase", "salt", "sealed", "wrapped",
		"API_KEY", "Password"]
	{
		let v = run(16, 101, ALNUM);
		req!(scrub::text(&fmt!("cfg {}: {} end", name, v), false),
			fmt!("cfg {}: {} end", name, marked("named", &v)), "for {}", name);
		req!(scrub::text(&fmt!("cfg {}=\"{}\"", name, v), false),
			fmt!("cfg {}=\"{}\"", name, marked("named", &v)), "quoted, for {}", name);
	}
	let v = run(10, 102, ALNUM);
	// Three quotes or spaces either side of the colon, and not four.
	req!(scrub::text(&fmt!("token\"  : \"'{}", v), false), fmt!("token\"  : \"'{}", marked("named", &v)));
	req!(scrub::hit(&fmt!("token\"   : {}", v), false), false);
	req!(scrub::hit(&fmt!("token: \"'\" {}", v), false), false);
	// Eight value characters, the alphabet with `.`, `/`, `+` and `=` in it.
	req!(scrub::text("token=ab.cd/e+=", false), fmt!("token={}", marked("named", "ab.cd/e+=")));
	req!(scrub::hit("token=abcdefg", false), false);
	// A name is not a name in the middle of a word it does not end: there is no boundary rule on
	// the front of this one, so a suffix counts.
	req!(scrub::text(&fmt!("mytoken={}", v), false), fmt!("mytoken={}", marked("named", &v)));
	Ok(())
}

#[test]
fn an_aws_secret_is_found_beside_a_key_id_the_shapes_did_not_take_or_beside_its_name() -> Outcome<()> {
	let id = fmt!("AKIA{}", run(16, 111, UPPER));
	let sec = run(40, 112, STD);
	// With a word boundary the id is scrubbed first, and nothing is left for the pair to stand
	// beside: the secret keeps its own bytes, as it does in the JavaScript.
	let plain = fmt!("{} and its secret {} here", id, sec);
	req!(scrub::text(&plain, false), fmt!("{} and its secret {} here", marked("aws", &id), sec));
	// Without one, the shape passes the id by and the pair takes the secret.
	let bound = fmt!("x{} and its secret {} here", id, sec);
	req!(scrub::text(&bound, false), fmt!("x{} and its secret {} here", id, marked("awssec", &sec)));
	// The gap between them is at most 200 units, and the separator is one of a quote, a space, a
	// colon, an equals sign or a comma.
	for (gap, ok) in [(0usize, true), (150, true), (199, true), (200, true), (201, false)] {
		let text = fmt!("x{}{}:{}", id, "-".repeat(gap), sec);
		if ok {
			req!(scrub::text(&text, false), fmt!("x{}{}:{}", id, "-".repeat(gap), marked("awssec", &sec)), "gap {}", gap);
		} else {
			req!(scrub::text(&text, false), text.clone(), "gap {}", gap);
		}
	}
	// Forty exactly: a forty-first character of the value's alphabet means it is no secret.
	let long = fmt!("x{} {}{}", id, sec, "a");
	req!(scrub::hit(&long, false), false);
	// And by its name, with or without `aws`, in any case.
	for name in ["aws_secret_access_key", "AWS_SECRET_ACCESS_KEY", "secret_access_key", "awssecretaccesskey",
		"SecretAccessKey"]
	{
		req!(scrub::text(&fmt!("{} = {}", name, sec), false), fmt!("{} = {}", name, marked("awssec", &sec)), "for {}", name);
		req!(scrub::text(&fmt!("{}: \"{}\"", name, sec), false), fmt!("{}: \"{}\"", name, marked("awssec", &sec)), "quoted, for {}", name);
	}
	req!(scrub::hit(&fmt!("aws_secret_access_key = {}", run(39, 113, STD)), false), true,
		"thirty-nine is no secret to this rule, but it is a long high-entropy run");
	Ok(())
}

#[test]
fn the_entropy_catch_stops_at_its_threshold_and_spares_what_a_feed_carries() -> Outcome<()> {
	req!(scrub::entropy("aaaa"), 0.0);
	req!(scrub::entropy("abab"), 1.0);
	req!(scrub::entropy("abcd"), 2.0);
	req!(scrub::entropy("0123456789abcdef"), 4.0);
	// Sixteen symbols, each twice, is exactly 4.0 bits and is not above it. They are not all hex,
	// all digits or all letters, so the safe-run rules do not decide it.
	let sixteen = "ghijklmn12345678ghijklmn12345678";
	req!(sixteen.len(), 32);
	req!(scrub::entropy(sixteen), 4.0);
	req!(scrub::safe_run(sixteen), false);
	req!(scrub::text(&fmt!("x {} y", sixteen), false), fmt!("x {} y", sixteen));
	// Seventeen, each twice, is above it.
	let seventeen = "ghijklmno12345678ghijklmno12345678";
	req!(scrub::text(&fmt!("x {} y", seventeen), false), fmt!("x {} y", marked("hi", seventeen)));
	// A run of 31 is under the minimum however random it is, and 32 is not.
	req!(scrub::hit(&run(31, 121, B64), false), false);
	req!(scrub::hit(&run(32, 121, B64), false), true);
	// The correlation-id fields are spared the catch alone.
	let r = run(40, 122, B64);
	req!(scrub::text(&r, true), r.clone());
	req!(scrub::text(&r, false), marked("hi", &r));
	let key = fmt!("sk-{}", run(20, 123, B64));
	req!(scrub::text(&key, true), marked("sk", &key), "a shape is not spared");
	Ok(())
}

#[test]
fn what_a_feed_is_meant_to_carry_is_a_safe_run() {
	for s in [
		"0a1b2c3d4e5f60718293a4b5c6d7e8f9",				// hex
		"0A1B-2C3D_4E5F-6071-8293",						// hex joined by separators, case unimportant
		"fb9ba89bc9a6abe785f300ee_83dec58094a8",		// the same with a separator in the middle
		"12345678901234567890123456789012",				// decimal
		"abcdefghijklmnopqrstuvwxyzabcdef",				// letters
		"x_y_zz",										// letters once the separators are gone
		"worker_pool_seat_000412_alpha",				// words and numbers
	] {
		assert!(scrub::safe_run(s), "{:?}", s);
	}
	for s in [
		"build-6911be85c0f7-seq-312-stamp",				// a segment that is hex and no word or number
		"gh_c1_zz",										// `c1` is neither
		"gh--12",										// an empty segment is neither
		"g_1",											// a single letter is not a word
		"",
		"Zm9vYmFy-Z2hpamts_bW5vcA",						// base64url
	] {
		assert!(!scrub::safe_run(s), "{:?}", s);
	}
}

#[test]
fn nothing_is_changed_that_has_nothing_in_it() {
	for s in ["", "short", "       ", "plain prose with no credential in it, only words and 12345",
		"/usr/local/bin/some_tool --flag=value --other 12", "caf\u{e9} \u{65e5}\u{672c}\u{8a9e} \u{1F600}"]
	{
		assert_eq!(scrub::text(s, false), s);
		assert!(!scrub::hit(s, false), "{:?}", s);
	}
}

#[test]
fn scrubbing_twice_changes_nothing_the_second_time() -> Outcome<()> {
	let text = fmt!("first sk_test_{} then eyJ{}.{}.{} then AKIA{}\nAuthorization: Bearer {}\n?token={}&k=1\nblob {}",
		run(24, 131, ALNUM), run(10, 132, B64), run(10, 133, B64), run(10, 134, B64), run(16, 135, UPPER),
		run(20, 136, B64), run(12, 137, ALNUM), run(48, 138, B64));
	let once = scrub::text(&text, false);
	assert!(once != text, "the text held credentials and was not changed");
	req!(scrub::text(&once, false), once.clone());
	req!(scrub::hit(&once, false), false);
	Ok(())
}

#[test]
fn the_hooks_scan_reports_what_it_reported_before() -> Outcome<()> {
	// Daimond's shapes are a table of their own. A shape the scrubber has and the hook has not
	// is no business of a commit.
	let cases = [
		fmt!("eyJ{}.{}.{}", run(8, 141, B64), run(12, 142, B64), run(6, 143, B64)),
		fmt!("whsec_{}", run(20, 144, ALNUM)),
		fmt!("sk_test_{}", run(14, 145, ALNUM)),
		fmt!("tune-{}", run(32, 146, HEX)),
		fmt!("ASIA{}", run(14, 147, UPPER)),
		fmt!("Authorization: Bearer {}", run(20, 148, B64)),
		fmt!("?token={}", run(12, 149, ALNUM)),
		fmt!("blob {}", run(40, 150, B64)),
	];
	for c in &cases {
		req!(scrub::hit(c, false), true, "{:?}", c);
		req!(secret::scan(c.as_bytes()).is_empty(), true, "scan, for {:?}", c);
		req!(secret::holds(c), false, "holds, for {:?}", c);
	}
	req!(secret::leads_are_covered(), true);
	Ok(())
}

#[test]
fn a_hostile_string_does_not_make_a_rule_slow() -> Outcome<()> {
	// A rule that restarts a long run from every opening is quadratic, and a feed's author may be
	// anyone. Each of these is a long run with an opening at every few bytes.
	let n = 30_000;
	let hostile = [
		"eyJ".repeat(n),
		"AKIA".repeat(n),
		"ASIA".repeat(n / 2) + " ,",
		"sk-".repeat(n) + "!",
		"-----BEGIN ".repeat(n / 4),
		fmt!("{}{}", "-----BEGIN ", "PRIVATE KEY-----").repeat(n / 8),
		"token=".repeat(n),
		"?key=".repeat(n),
		"Bearer ".repeat(n),
		"aws_secret_access_key=".repeat(n / 4),
		"a".repeat(n * 4),
		"a_".repeat(n * 2),
		"x-".repeat(n * 2) + &run(40, 160, B64),
		"AKIA".repeat(n) + &" ".repeat(300) + &run(40, 161, STD),
	];
	for h in &hostile {
		let t = Instant::now();
		let _ = scrub::text(h, false);
		let _ = scrub::text(h, true);
		let ms = t.elapsed().as_millis();
		assert!(ms < 10_000, "{} ms on a {}-byte string opening {:?}", ms, h.len(), &h[..h.len().min(24)]);
	}
	Ok(())
}
