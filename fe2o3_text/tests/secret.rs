//! Every credential in this file is spelled in two pieces and joined at run time, so that the
//! scanners which read this very file -- the git hook, and this crate's own scanner under a
//! version control system that cannot forget -- find nothing in it to refuse.
//!
//! The DER fixtures are the one thing here written out whole, and they are not credentials. Each
//! is the structural head of a key -- the outer length, the version, the algorithm's object
//! identifier and the tag that opens the private bytes -- read off a key generated with `openssl
//! genpkey` or `ring` for that purpose, and stopping exactly where the secret would begin. The
//! body is filler put there at run time. That is enough to ask the detector the only question it
//! asks, and it means no key was written into a file that is pushed to a public repository.

use oxedyne_fe2o3_text::{
	base2x,
	secret::{
		self,
		Find,
		Kind,
	},
};

use oxedyne_fe2o3_core::{
	prelude::*,
	test::test_it,
};


// One credential of each shape, as an opening and the rest of it.
const SHAPED: &[(&str, &str, Kind)] = &[
	("fw",			"_3ZjKq81mAbCdEfGhIjKlMnOpQrSt",			Kind::Fireworks),
	("sk-ant",		"-api03-AbCdEfGhIjKlMnOpQrStUvWx",			Kind::Anthropic),
	("sk-proj",		"-AbCdEfGhIjKlMnOpQrStUvWxYz01",			Kind::OpenAi),
	("sk-or",		"-v1-0123456789abcdef0123456789abcdef",		Kind::OpenAi),
	("sk-",			"AbCdEfGhIjKlMnOpQrStUvWxYz0123456789",		Kind::OpenAiOld),
	("AKIA",		"IOSFODNN7EXAMPLE",							Kind::Aws),
	("ghp",			"_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789",	Kind::GitHub),
	("github_pat",	"_11ABCDEFG0AbCdEfGhIjKlMnOpQrStUvWxYz0123456789",
														Kind::GitHubPat),
	("xoxb",		"-1234567890-abcdefghij",					Kind::Slack),
	("sk_live",		"_AbCdEfGhIjKlMnOpQrStUv",					Kind::Stripe),
	("AIza",		"SyA0123456789abcdefghijklmnopqrstuv",		Kind::Google),
	("-----BEGIN ",	"OPENSSH PRIVATE KEY-----",					Kind::PrivateKey),
];

// The value of a `Kind::Assigned` finding, in two pieces for the same reason.
const LITERAL: (&str, &str) = ("9f3Bq7", "ZmR4tYuIoPkLjHgFdS");

// The structural head of one private key of each form this catches, and the whole size of the key
// it came off. Read off keys generated with `openssl genpkey`, `openssl ecparam -genkey` and
// `ring`, in that order of appearance, and stopping before the private bytes: see the note at the
// head of this file.
const DER: &[(&str, &str, usize)] = &[
	("ed25519, PKCS#8",		"302E020100300506032B657004220420",					48),
	("ed25519, with the public key",
							"3051020101300506032B657004220420",					83),
	("X25519, PKCS#8",		"302E020100300506032B656E04220420",					48),
	("RSA-2048, PKCS#8",	"308204BD020100300D06092A864886F70D0101010500048204A7",
																				1217),
	("RSA-2048, PKCS#1",	"308204A30201000282010100",							1191),
	("RSA-4096, PKCS#1",	"308209290201000282020100",							2349),
	("P-256, PKCS#8",		"308187020100301306072A8648CE3D020106082A8648CE3D030107",
																				138),
	("P-384, PKCS#8",		"3081B6020100301006072A8648CE3D020106052B8104002204819E",
																				185),
	("P-521, PKCS#8",		"3081EE020100301006072A8648CE3D020106052B81040023",	241),
	("P-256, SEC1",			"30770201010420",									121),
	("P-384, SEC1",			"3081A40201010430",									167),
	("P-521, SEC1",			"3081DC0201010442",									223),
];

// The 83-byte shape the DKIM signing key was in, named on its own because it is the case this
// rule was written for.
const DKIM: (&str, usize) = (DER[1].1, DER[1].2);

// A certificate, which is what a key is most often bundled with, and whose outer sequence opens
// with another sequence where a key's opens with a version.
const CERT: (&str, usize) = ("3082013B3081EEA003020102", 319);

/// The key of that shape, its structure real and its body filler.
fn der(head: &str, len: usize) -> Outcome<Vec<u8>> {
	let mut out = res!(base2x::HEX.from_str(head));
	// Whatever stands where the secret would is beside the point: the detector is being asked a
	// question about the encoding, and it never looks at these bytes.
	while out.len() < len {
		out.push(0x5A);
	}
	out.truncate(len);
	Ok(out)
}


pub fn test_secret(filter: &'static str) -> Outcome<()> {

	res!(test_it(filter, &["Every shape is caught", "all", "secret", "shape"], || {
		for (lead, rest, kind) in SHAPED {
			let line = fmt!("let key = \"{}{}\";\n", lead, rest);
			let found = secret::scan(line.as_bytes());
			req!(found, vec![Find { line: 1, kind: *kind }], "for {:?}", lead);
		}
		Ok(())
	}));

	res!(test_it(filter, &["Nothing is caught in ordinary source", "all", "secret", "shape"], || {
		let text = b"let key = res!(std::env::var(\"FIREWORKS_API_KEY\"),\n\
			\t\"Set FIREWORKS_API_KEY before running this example.\");\n\
			// A short one, sk-nope, and a field with nothing in it, api_key = \"\".\n";
		req!(secret::scan(text), Vec::<Find>::new());
		Ok(())
	}));

	res!(test_it(filter, &["The prefilter admits every opening", "all", "secret", "shape"], || {
		// A shape whose opening the prefilter rejects would match nothing, and every other test
		// here would still pass.
		req!(secret::leads_are_covered(), true);
		Ok(())
	}));

	res!(test_it(filter, &["A named field holding a long literal is caught", "all", "secret",
		"assigned"], ||
	{
		for field in ["api_key", "API_KEY", "secret", "password", "access_token"] {
			let line = fmt!("{} = \"{}{}\"\n", field, LITERAL.0, LITERAL.1);
			req!(secret::scan(line.as_bytes()), vec![Find { line: 1, kind: Kind::Assigned }],
				"for {:?}", field);
		}
		Ok(())
	}));

	res!(test_it(filter, &["A placeholder is not a credential", "all", "secret", "assigned"], || {
		// The rule that decides whether the guard is left switched on. A documentation example
		// refused is a guard somebody turns off, and then it protects nothing.
		for value in [
			"your-key-here",
			"YOUR_API_KEY_GOES_HERE",
			"xxxxxxxxxxxxxxxxxxxxxxxx",
			"placeholder_value_here_ok",
			"changeme_changeme_changeme",
			"example_token_0123456789",
		] {
			let line = fmt!("api_key = \"{}\"\n", value);
			req!(secret::scan(line.as_bytes()), Vec::<Find>::new(), "for {:?}", value);
		}
		Ok(())
	}));

	res!(test_it(filter, &["A short literal is not a credential", "all", "secret", "assigned"],
		||
	{
		let line = fmt!("password: \"{}\"\n", "9f3Bq7ZmR4tYuIoPkLjH");
		req!(secret::scan(line.as_bytes()), vec![Find { line: 1, kind: Kind::Assigned }]);
		let line = fmt!("password: \"{}\"\n", "9f3Bq7ZmR4tYuIoPkLj");
		req!(secret::scan(line.as_bytes()), Vec::<Find>::new());
		Ok(())
	}));

	res!(test_it(filter, &["The marker excuses a line, and only while it is there", "all",
		"secret", "marker"], ||
	{
		let bare = fmt!("let key = \"{}{}\";\n", SHAPED[0].0, SHAPED[0].1);
		req!(secret::scan(bare.as_bytes()), vec![Find { line: 1, kind: Kind::Fireworks }]);
		for marker in ["allowlist secret", "allowlist-secret", "ALLOWLIST SECRET",
			"pragma: allowlist nextline"]
		{
			let line = fmt!("let key = \"{}{}\"; // {}\n", SHAPED[0].0, SHAPED[0].1, marker);
			req!(secret::scan(line.as_bytes()), Vec::<Find>::new(), "for {:?}", marker);
			let above = fmt!("// {}\nlet key = \"{}{}\";\n", marker, SHAPED[0].0, SHAPED[0].1);
			req!(secret::scan(above.as_bytes()), Vec::<Find>::new(), "above, for {:?}", marker);
		}
		// One line above, and no further.
		let far = fmt!("// {}\n\nlet key = \"{}{}\";\n", secret::MARKER, SHAPED[0].0, SHAPED[0].1);
		req!(secret::scan(far.as_bytes()), vec![Find { line: 3, kind: Kind::Fireworks }]);
		Ok(())
	}));

	res!(test_it(filter, &["A finding names the line it is on", "all", "secret", "marker"], || {
		let text = fmt!("one\ntwo\nthree\nlet key = \"{}{}\";\nfive\n",
			SHAPED[0].0, SHAPED[0].1);
		req!(secret::scan(text.as_bytes()), vec![Find { line: 4, kind: Kind::Fireworks }]);
		Ok(())
	}));

	res!(test_it(filter, &["A binary is left alone", "all", "secret", "binary"], || {
		let mut data = fmt!("\0\u{1}\u{2}").into_bytes();
		data.extend_from_slice(fmt!("key = \"{}{}\"\n", SHAPED[0].0, SHAPED[0].1).as_bytes());
		req!(secret::scan(&data), Vec::<Find>::new());
		Ok(())
	}));

	res!(test_it(filter, &["Lockfiles and vendored trees are not scanned", "all", "secret",
		"path"], ||
	{
		for path in ["Cargo.lock", "web/package-lock.json", "go.sum", "node_modules/a/b.js",
			"target/debug/build.rs", "a/vendor/b/c.go", "dist/app.js", ".venv/lib/x.py"]
		{
			req!(secret::skip_path(path.as_bytes()), true, "for {:?}", path);
		}
		for path in ["src/main.rs", "target.rs", "vendor.md", "a/build.rs", "notes/dist.txt"] {
			req!(secret::skip_path(path.as_bytes()), false, "for {:?}", path);
		}
		Ok(())
	}));

	res!(test_it(filter, &["A vendored name below a source tree is not build output", "all",
		"secret", "path"], ||
	{
		// `dist` earns its place on the list because a bundler writes one beside a source tree.
		// Below a `src` the same name is a person's own, and reading it as build output left
		// fourteen hand-written Rust files unscanned by this crate and by the git hook.
		for path in ["fe2o3_o3db_sync/src/dist/cohort.rs", "src/dist/mod.rs", "a/b/src/build/x.rs",
			"src/vendor/x.rs", "crate/src/target/y.rs", "src/node_modules/z.js", "src/.venv/w.py"]
		{
			req!(secret::skip_path(path.as_bytes()), false, "for {:?}", path);
		}
		// A source tree inside a vendored one is still somebody else's.
		for path in ["node_modules/pkg/src/dist/bundle.js", "vendor/dep/src/lib.rs",
			"target/debug/build/dep/src/main.rs"]
		{
			req!(secret::skip_path(path.as_bytes()), true, "for {:?}", path);
		}
		// The name of a lockfile still decides, wherever the file sits.
		req!(secret::skip_path(b"src/dist/Cargo.lock"), true);
		Ok(())
	}));

	res!(test_it(filter, &["A key in a source tree called dist is found", "all", "secret", "path"],
		||
	{
		// The two halves of the guard, put together the way a caller puts them: the path is
		// scanned, and the scan refuses what is in it.
		let path = b"fe2o3_o3db_sync/src/dist/transport.rs";
		req!(secret::skip_path(path), false);
		let line = fmt!("let key = \"{}{}\";\n", SHAPED[0].0, SHAPED[0].1);
		req!(secret::scan(line.as_bytes()), vec![Find { line: 1, kind: Kind::Fireworks }]);
		Ok(())
	}));

	res!(test_it(filter, &["A private key in DER form is caught", "all", "secret", "der"], || {
		for (what, head, len) in DER {
			let key = res!(der(head, *len));
			req!(key.len(), *len, "for {:?}", what);
			req!(secret::scan(&key), vec![Find { line: 1, kind: Kind::DerKey }], "for {:?}", what);
		}
		Ok(())
	}));

	res!(test_it(filter, &["The DKIM key's own shape is caught at 83 bytes", "all", "secret",
		"der"], ||
	{
		// A raw PKCS#8 ed25519 key carrying its public half, which is what `ring` writes and what
		// signed mail for four months from a folder that replicates. No armour, no vendor prefix
		// and no field name beside it.
		let key = res!(der(DKIM.0, DKIM.1));
		req!(key.len(), 83);
		req!(key[1], 0x51);
		req!(secret::scan(&key), vec![Find { line: 1, kind: Kind::DerKey }]);
		// The object identifier is the whole of what says so. One byte off it and this is 83 bytes
		// that nothing else in the module can see -- no shape, no field name, no armour -- which is
		// what the four months were.
		let mut off = key.clone();
		off[11] ^= 0x01;
		req!(secret::scan(&off), Vec::<Find>::new());
		// A real key's bytes are random, so a NUL stands somewhere in most of them, and the binary
		// skip would then stop the scan before it began. This is asked first, and the order is what
		// this line holds in place.
		let mut nulled = key.clone();
		nulled[20] = 0;
		req!(secret::scan(&nulled), vec![Find { line: 1, kind: Kind::DerKey }]);
		Ok(())
	}));

	res!(test_it(filter, &["A newline after the last byte does not hide a DER key", "all",
		"secret", "der"], ||
	{
		for tail in ["\n", "\r\n", "\n\n"] {
			let mut key = res!(der(DER[0].1, DER[0].2));
			key.extend_from_slice(tail.as_bytes());
			req!(secret::scan(&key), vec![Find { line: 1, kind: Kind::DerKey }], "for {:?}", tail);
		}
		Ok(())
	}));

	res!(test_it(filter, &["What is not a DER private key is left alone", "all", "secret",
		"der"], ||
	{
		// A public key, which names the same algorithm and holds nothing worth refusing: the
		// version integer this rule turns on is absent from it.
		let public = res!(der("302A300506032B6570032100", 44));
		req!(secret::scan(&public), Vec::<Find>::new(), "public key");
		// A certificate, whose outer sequence opens with another sequence.
		let cert = res!(der(CERT.0, CERT.1));
		req!(secret::scan(&cert), Vec::<Find>::new(), "certificate");
		// An algorithm nobody has, one object identifier byte away from ed25519.
		let other = res!(der("302E020100300506032B657104220420", 48));
		req!(secret::scan(&other), Vec::<Find>::new(), "unknown algorithm");
		// Truncated: the outer length declares more than is there, so the sequence it names is not
		// in the file. Nor is the key -- openssl reads nothing out of this one.
		let short = res!(der(DER[0].1, 47));
		req!(secret::scan(&short), Vec::<Find>::new(), "truncated");
		// A three-byte sequence holding the version and stopping, with an ed25519 key's algorithm
		// standing immediately after it. Every test below the length is asked inside the declared
		// bytes, and this is the fixture that says so: unbounded, the sequence borrows the seven
		// bytes after itself and this reads as a key.
		let borrowed = res!(der("3003020100300506032B6570", 48));
		req!(secret::scan(&borrowed), Vec::<Find>::new(), "borrowed algorithm");
		// Nothing, and something far too small to be a key.
		req!(secret::scan(b""), Vec::<Find>::new(), "empty");
		req!(secret::scan(&[0x30, 0x02, 0x02, 0x01]), Vec::<Find>::new(), "tiny");
		Ok(())
	}));

	res!(test_it(filter, &["Bytes written after a DER key do not hide it", "all", "secret",
		"der"], ||
	{
		// What this rule asked until 2026-08-23 was that the outer sequence account for the input
		// exactly, so that what it refused was a file that was a key and nothing else. One byte
		// appended walked past the whole of it, and what walked past was a key openssl still read
		// and signed with.
		for tail in [&b"x"[..], b"\0", b" ", b"# the dkim signing key\n"] {
			let mut key = res!(der(DKIM.0, DKIM.1));
			key.extend_from_slice(tail);
			req!(secret::scan(&key), vec![Find { line: 1, kind: Kind::DerKey }], "for {:?}", tail);
		}
		// The shape nobody has to tamper with to produce: a key and the certificate that goes with
		// it in one file, which is what `cat key.der cert.der` writes.
		let mut bundle = res!(der(DER[0].1, DER[0].2));
		bundle.extend_from_slice(&res!(der(CERT.0, CERT.1)));
		req!(bundle.len(), DER[0].2 + CERT.1);
		req!(secret::scan(&bundle), vec![Find { line: 1, kind: Kind::DerKey }], "key and cert");
		// And the marker is not a way out either, wherever it is written: this reads the key's own
		// structure and never the bytes around it, so there is nowhere to put one that it looks at.
		let mut marked = res!(der(DER[0].1, DER[0].2));
		marked.extend_from_slice(fmt!("\n// {}\n", secret::MARKER).as_bytes());
		req!(secret::scan(&marked), vec![Find { line: 1, kind: Kind::DerKey }], "marked");
		Ok(())
	}));

	res!(test_it(filter, &["A DER key away from the front of the file is caught", "all", "secret",
		"der"], ||
	{
		// `cat cert.der key.der`, which is the bundle a person writes without thinking about it:
		// `openssl pkey -inform DER` reads the private key straight out of one, signs with it, and
		// the signature verifies against the original key's public half. Both guards walked past it
		// until 2026-08-23, because each only ever looked at byte 0.
		for (what, head, len) in DER {
			let mut bundle = res!(der(CERT.0, CERT.1));
			bundle.extend_from_slice(&res!(der(head, *len)));
			req!(bundle.len(), CERT.1 + *len, "for {:?}", what);
			req!(secret::scan(&bundle), vec![Find { line: 1, kind: Kind::DerKey }], "for {:?}", what);
		}
		// One byte in front of it does the same, whatever the byte is, the SEQUENCE tag included.
		for lead in [0x41u8, 0x00, 0x30, 0x02] {
			let mut data = vec![lead];
			data.extend_from_slice(&res!(der(DKIM.0, DKIM.1)));
			req!(secret::scan(&data), vec![Find { line: 1, kind: Kind::DerKey }],
				"for a leading {:#04x}", lead);
		}
		// And the finding names the line the key opens on, so that a key written into a file
		// somebody reads is reported where they will find it rather than at the top.
		let mut noted = fmt!("# the dkim signing key\n\n").into_bytes();
		noted.extend_from_slice(&res!(der(DKIM.0, DKIM.1)));
		req!(secret::scan(&noted), vec![Find { line: 3, kind: Kind::DerKey }]);
		Ok(())
	}));

	res!(test_it(filter, &["Only a small file is read at every offset", "all", "secret", "der"],
		||
	{
		// `ring` holds the head of a PKCS#8 key as a `const` template -- these very bytes, which is
		// why the fixture below is the DKIM shape -- and a compiler writes that template into the
		// read-only data of whatever links it. A sweep of every file under ~/usr on 2026-08-23,
		// 623,722 files and 202 GB with nothing skipped, found the structure at 1,729 offsets in 303
		// files, and every one of the 303 was a compiled artefact carrying that template: not one was
		// a key. Nothing separates a template from a key that is not a guess about the bytes around
		// it, and refusing an ordinary build output is how a guard gets switched off. So the offsets
		// are read only while the file is small enough to be a key and what a key is bundled with,
		// and the smallest artefact in that sweep was 101,960 bytes, three times the span.
		let key = res!(der(DKIM.0, DKIM.1));
		let mut inside = vec![0x5A; secret::DER_SPAN - key.len()];
		inside.extend_from_slice(&key);
		req!(inside.len(), secret::DER_SPAN);
		req!(secret::scan(&inside), vec![Find { line: 1, kind: Kind::DerKey }], "at the span");
		// One byte wider and only the front is read, which is where this key is not.
		let mut over = vec![0x5A; secret::DER_SPAN + 1 - key.len()];
		over.extend_from_slice(&key);
		req!(over.len(), secret::DER_SPAN + 1);
		req!(secret::scan(&over), Vec::<Find>::new(), "past the span");
		// The front of a file is still read whatever the file's size, which is the rule as it stood
		// before offsets were looked at and is what catches `cat key.der cert.der`.
		let mut wide = key.clone();
		wide.resize(secret::DER_SPAN * 4, 0x5A);
		req!(secret::scan(&wide), vec![Find { line: 1, kind: Kind::DerKey }], "at the front");
		Ok(())
	}));

	res!(test_it(filter, &["A compiled artefact is not read for a DER key", "all", "secret",
		"der"], ||
	{
		// Size is the whole of the gate, and it is asked of the length the sequence declares, which
		// is read out of the first four bytes and nothing more. fe2o3 has a 22 MB binary in its
		// history, and the estate has ONNX models and video beside it. What the ceiling costs is
		// stated here rather than left to be found: this is a well formed ed25519 key declaring
		// 8996 bytes, which is the size an RSA-16384 key would be, and it goes free.
		let big = res!(der("30822324020100300506032B657004220420", 9000));
		req!(big.len(), 9000);
		req!(secret::scan(&big), Vec::<Find>::new(), "over the ceiling");
		// One byte under the ceiling the same key is caught, so the gate and nothing else is what
		// let the one above through.
		let under = res!(der("30821F3C020100300506032B657004220420", 8000));
		req!(under.len(), 8000);
		req!(secret::scan(&under), vec![Find { line: 1, kind: Kind::DerKey }], "at the ceiling");
		// And an ELF header opens with nothing this rule answers to.
		let mut elf = fmt!("\u{7f}ELF").into_bytes();
		elf.resize(200, 0);
		req!(secret::scan(&elf), Vec::<Find>::new(), "ELF");
		Ok(())
	}));

	Ok(())
}
