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

use std::{
	fs,
	io::ErrorKind,
	path::{
		Path,
		PathBuf,
	},
	process::Command,
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


// What a body of base64 is wrapped to by the tools that write one: `openssl` at 64, `ssh-keygen` at
// 70, and MIME at 76.
const WIDTHS: &[usize] = &[64, 70, 76];

// The front of an OpenSSH private key, as `ssh-keygen` writes it: the magic, a NUL, and then the
// cipher, kdf and key counts of an unprotected ed25519 key. The rest is filler.
const OPENSSH_HEAD: &[u8] = b"openssh-key-v1\0\0\0\0\x04none\0\0\0\x04none\0\0\0\0\0\0\0\x01";

/// The bytes as a body of lines, encoded by the `base64` crate and not by the module under test, so
/// that what the scanner is handed is an independent reading of the bytes.
fn wrapped(bytes: &[u8], width: usize, eol: &str) -> String {
	let text = base64::encode(bytes);
	let mut out = String::new();
	for line in text.as_bytes().chunks(width) {
		out.push_str(&String::from_utf8_lossy(line));
		out.push_str(eol);
	}
	out
}

/// The armour lines of a private key, with the words that a scanner looks for put together here so
/// that this file does not hold them.
fn armour(algo: &str) -> (String, String) {
	(
		fmt!("-----BEGIN {}{}-----\n", algo, "PRIVATE KEY"),
		fmt!("-----END {}{}-----\n", algo, "PRIVATE KEY"),
	)
}

/// What a PEM file holds between its armour lines.
fn body_of(pem: &str) -> String {
	let mut out = String::new();
	for line in pem.lines() {
		if !line.starts_with("-----") {
			out.push_str(line);
			out.push('\n');
		}
	}
	out
}

/// How many lines the text has.
fn lines_in(text: &str) -> usize {
	text.lines().count()
}

/// A scratch directory under `TMPDIR` that removes itself, so that a key generated for a test does
/// not outlive it.
struct Dir(PathBuf);

impl Dir {
	fn new(name: &str) -> Outcome<Self> {
		let path = std::env::temp_dir().join(fmt!("{}_{}", name, std::process::id()));
		res!(fs::create_dir_all(&path));
		Ok(Self(path))
	}

	fn path(&self) -> &Path { &self.0 }

	fn read(&self, name: &str) -> Outcome<String> {
		Ok(res!(fs::read_to_string(self.0.join(name))))
	}
}

impl Drop for Dir {
	fn drop(&mut self) {
		let _ = fs::remove_dir_all(&self.0);
	}
}

/// Runs a program in the directory. False where the program is not installed, so that a test
/// reports itself skipped rather than failing for want of a tool.
fn run(dir: &Dir, program: &str, args: &[&str]) -> Outcome<bool> {
	match Command::new(program).args(args).current_dir(dir.path()).output() {
		Ok(out) if out.status.success()	=> Ok(true),
		Ok(out)							=> Err(err!(
			"{} {:?} exited with {}: {}", program, args, out.status,
			String::from_utf8_lossy(&out.stderr); Test, IO)),
		Err(e) if e.kind() == ErrorKind::NotFound	=> {
			test!("{} is not installed, so what it makes is not tried.", program);
			Ok(false)
		},
		Err(e) => Err(err!("{} could not be run: {}", program, e; Test, IO)),
	}
}

/// A deterministic stream of 64 bit words, so that a test of random input is the same test every
/// time.
fn xorshift(state: &mut u64) -> u64 {
	let mut x = *state;
	x ^= x << 13;
	x ^= x >> 7;
	x ^= x << 17;
	*state = x;
	x
}

/// A line of `n` characters from the base64 alphabet, at random.
fn noise(state: &mut u64, n: usize) -> String {
	const ALPHABET: &[u8] =
		b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
	let mut out = String::with_capacity(n);
	for _ in 0..n {
		out.push(ALPHABET[(xorshift(state) % 64) as usize] as char);
	}
	out
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


	res!(test_it(filter, &["A private key body without its armour is caught", "all", "secret",
		"body"], ||
	{
		// What qa1 found on 2026-09-23 (D3): the body of a key, written into a file before the
		// armour is, was recorded by the tick that saw it. Each fixture here is a key of a real
		// form, its structure real and its body filler, encoded by the `base64` crate and wrapped
		// as each tool wraps. The finding stands on the line the key's last byte is on.
		for (what, head, len) in DER {
			let key = res!(der(head, *len));
			for width in WIDTHS {
				for eol in ["\n", "\r\n"] {
					let body = wrapped(&key, *width, eol);
					let want = vec![Find { line: lines_in(&body), kind: Kind::KeyBody }];
					req!(secret::scan(body.as_bytes()), want, "for {:?}, {} wide", what, width);
				}
			}
		}
		// Indented, as it stands in a YAML block, and after a line of prose, and with no newline at
		// the end of the file.
		let key = res!(der(DER[3].1, DER[3].2));
		let body = wrapped(&key, 64, "\n");
		let n = lines_in(&body);
		let mut indented = fmt!("private_key: |\n");
		for line in body.lines() {
			indented.push_str(&fmt!("    {}\n", line));
		}
		indented.push_str("other: 1\n");
		req!(secret::scan(indented.as_bytes()), vec![Find { line: 1 + n, kind: Kind::KeyBody }],
			"indented");
		req!(secret::scan(body.trim_end().as_bytes()),
			vec![Find { line: n, kind: Kind::KeyBody }], "no newline at the end");
		// A word or a label alone on the line above is all alphabet, so it runs into the body and
		// puts every character after it out of step. The body is found all the same, and the same
		// word after it, or a certificate with a label above it, is nothing.
		for label in ["notes", "id", "Key", "ssh", "x"] {
			let text = fmt!("{}\n{}{}\n", label, body, label);
			req!(secret::scan(text.as_bytes()), vec![Find { line: 1 + n, kind: Kind::KeyBody }],
				"after {:?}", label);
			let cert = fmt!("{}\n{}", label, wrapped(&res!(der(CERT.0, CERT.1)), 64, "\n"));
			req!(secret::scan(cert.as_bytes()), Vec::<Find>::new(), "certificate after {:?}", label);
		}
		Ok(())
	}));

	res!(test_it(filter, &["Armour is the finding and its body is not a second one", "all", "secret",
		"body", "marker"], ||
	{
		let key = res!(der(DER[3].1, DER[3].2));
		let body = wrapped(&key, 64, "\n");
		let n = lines_in(&body);
		let (head, tail) = armour("");
		let pem = fmt!("{}{}{}", head, body, tail);
		req!(secret::scan(pem.as_bytes()), vec![Find { line: 1, kind: Kind::PrivateKey }],
			"armoured");
		// A marker above the armour excused the whole of it before bodies were read, and does now.
		let marked = fmt!("// {}\n{}", secret::MARKER, pem);
		req!(secret::scan(marked.as_bytes()), Vec::<Find>::new(), "marker above the armour");
		let open = fmt!("// {}\n{}{}", secret::MARKER, head, body);
		req!(secret::scan(open.as_bytes()), Vec::<Find>::new(), "marker above, no end line");
		// On the armour line itself.
		let beside = fmt!("{}{}{}", head.trim_end(), fmt!(" # {}\n", secret::MARKER), body);
		req!(secret::scan(beside.as_bytes()), Vec::<Find>::new(), "marker on the armour line");
		// With no armour there is no line that says what the body is, and so no marker for it:
		// the rule reads the key's own structure, as it does for a raw DER key. A marker above the
		// body, on it, or below it excuses nothing.
		let above = fmt!("# {}\n{}", secret::MARKER, body);
		req!(secret::scan(above.as_bytes()), vec![Find { line: 1 + n, kind: Kind::KeyBody }],
			"marker above a bare body");
		let below = fmt!("{}# {}\n", body, secret::MARKER);
		req!(secret::scan(below.as_bytes()), vec![Find { line: n, kind: Kind::KeyBody }],
			"marker below");
		let one = fmt!("{} # {}\n", base64::encode(&key), secret::MARKER);
		req!(secret::scan(one.as_bytes()), vec![Find { line: 1, kind: Kind::KeyBody }],
			"marker beside a token");
		// Armour and a whole body on one line are the armour's to speak for, so a marker on the
		// line excuses the lot and without one there is a single finding.
		let flat = fmt!("{}{} {}\n", head.trim_end(), base64::encode(&key), tail.trim_end());
		req!(secret::scan(flat.as_bytes()), vec![Find { line: 1, kind: Kind::PrivateKey }],
			"armour and body on one line");
		let flat = fmt!("{} # {}\n", flat.trim_end(), secret::MARKER);
		req!(secret::scan(flat.as_bytes()), Vec::<Find>::new(), "and a marker on it");
		Ok(())
	}));

	res!(test_it(filter, &["A private key body on one line is caught", "all", "secret", "body",
		"token"], ||
	{
		for (what, head, len) in DER {
			let key = res!(der(head, *len));
			let padded = base64::encode(&key);
			let bare = padded.trim_end_matches('=').to_string();
			for one in [&padded, &bare] {
				for form in [
					fmt!("{}\n", one),
					fmt!("KEY={}\n", one),
					fmt!("\"private_key\": \"{}\",\n", one),
					fmt!("key: {} # the signing key\n", one),
					fmt!("<key>{}</key>\n", one),
				] {
					req!(secret::scan(form.as_bytes()),
						vec![Find { line: 1, kind: Kind::KeyBody }], "for {:?}, {}", what, form.len());
				}
			}
		}
		// A certificate and a key written as one file and encoded as one token: the key is not at
		// the front, and is found where it is.
		let mut both = res!(der(CERT.0, CERT.1));
		both.extend_from_slice(&res!(der(DER[0].1, DER[0].2)));
		let line = fmt!("bundle = {}\n", base64::encode(&both));
		req!(secret::scan(line.as_bytes()), vec![Find { line: 1, kind: Kind::KeyBody }], "bundle");
		Ok(())
	}));

	res!(test_it(filter, &["What is not key material is not caught in base64", "all", "secret", "body",
		"clean"], ||
	{
		// A certificate and a public key, which name the same algorithms and hold nothing worth
		// refusing.
		let cert = res!(der(CERT.0, CERT.1));
		for width in WIDTHS {
			req!(secret::scan(wrapped(&cert, *width, "\n").as_bytes()), Vec::<Find>::new(),
				"certificate, {} wide", width);
		}
		let public = res!(der("302A300506032B6570032100", 44));
		req!(secret::scan(wrapped(&public, 64, "\n").as_bytes()), Vec::<Find>::new(), "public key");
		// The structural head of a key with an algorithm nobody has.
		let other = res!(der("302E020100300506032B657104220420", 48));
		req!(secret::scan(wrapped(&other, 64, "\n").as_bytes()), Vec::<Find>::new(), "unknown");
		// A thousand lines of random base64, as runs of ten at two widths, then as one run, then
		// one to a line among prose: none decodes to a key.
		let mut state = 0x9E3779B97F4A7C15u64;
		for width in [64usize, 76] {
			let mut text = String::new();
			for _ in 0..100 {
				for _ in 0..10 {
					text.push_str(&noise(&mut state, width));
					text.push('\n');
				}
				text.push('\n');
			}
			req!(secret::scan(text.as_bytes()), Vec::<Find>::new(), "runs of ten, {} wide", width);
		}
		let mut run = String::new();
		let mut tokens = String::new();
		for i in 0..1000 {
			run.push_str(&noise(&mut state, 64));
			run.push('\n');
			tokens.push_str(&fmt!("let k{} = \"{}\";\n", i, noise(&mut state, 43 + i % 200)));
		}
		req!(secret::scan(run.as_bytes()), Vec::<Find>::new(), "one run of a thousand");
		req!(secret::scan(tokens.as_bytes()), Vec::<Find>::new(), "a token to a line");
		// Words one to a line are all alphabet and run on into one another, and decode to nothing.
		let words = "alpha\nbravo\ncharlie\ndelta\necho\nfoxtrot\ngolf\nhotel\nindia\njuliet\n".repeat(50);
		req!(secret::scan(words.as_bytes()), Vec::<Find>::new(), "words");
		Ok(())
	}));

	res!(test_it(filter, &["The finding stands on the line the key ends on", "all", "secret", "body"],
		||
	{
		// A key short of its last byte is not a key, so a body saved half way through is a
		// fragment, as a token's first characters are, and the save that completes it is refused.
		let key = res!(der(DER[3].1, DER[3].2));
		let body = wrapped(&key, 64, "\n");
		let n = lines_in(&body);
		let all: Vec<&str> = body.lines().collect();
		let half = all[..n - 1].join("\n");
		req!(secret::scan(half.as_bytes()), Vec::<Find>::new(), "the last line missing");
		req!(secret::scan(all.join("\n").as_bytes()),
			vec![Find { line: n, kind: Kind::KeyBody }], "complete");
		// Bytes after the key in the same run do not move the finding off the line the key ends on:
		// an ed25519 key and thirty more bytes at twenty characters a line, which put the key's last
		// byte in the fourth of six.
		let mut run = res!(der(DER[0].1, DER[0].2));
		run.extend_from_slice(&[0x5A; 30]);
		let text = wrapped(&run, 20, "\n");
		req!(lines_in(&text), 6);
		req!(secret::scan(text.as_bytes()), vec![Find { line: 4, kind: Kind::KeyBody }], "mid run");
		// A padded last line ends a run, so what follows it is a run of its own.
		let more = fmt!("{}{}\n{}\n", body, noise(&mut 7u64, 64), noise(&mut 9u64, 64));
		req!(secret::scan(more.as_bytes()), vec![Find { line: n, kind: Kind::KeyBody }], "after");
		// A finding the run puts on a line above one the line walk already found is put in order:
		// a P-256 key takes no padding, so an AWS key id on the next line is part of its run.
		let p256 = wrapped(&res!(der(DER[4].1, DER[4].2)), 64, "\n");
		let aws = fmt!("{}{}\n", "AKIA", "IOSFODNN7EXAMPLE");
		let text = fmt!("{}{}", p256, aws);
		req!(secret::scan(text.as_bytes()), vec![
			Find { line: lines_in(&p256), kind: Kind::KeyBody },
			Find { line: lines_in(&p256) + 1, kind: Kind::Aws },
		], "in the order the lines hold them");
		// So two bodies in a file, each ending in padding, are two findings.
		let dkim = wrapped(&res!(der(DKIM.0, DKIM.1)), 64, "\n");
		let both = fmt!("{}{}", dkim, body);
		req!(secret::scan(both.as_bytes()), vec![
			Find { line: lines_in(&dkim), kind: Kind::KeyBody },
			Find { line: lines_in(&dkim) + n, kind: Kind::KeyBody },
		], "two bodies");
		// A key's last byte on the last character of a line is on that line, and on the first
		// character of the next when the line is one character short: sixteen to a line puts
		// the last of forty-eight bytes at the end of the fourth, and seventy puts the last of
		// fifty-three at the start of the second.
		let mut run = res!(der(DER[0].1, DER[0].2));
		run.extend_from_slice(&[0x5A; 30]);
		req!(secret::scan(wrapped(&run, 16, "\n").as_bytes()),
			vec![Find { line: 4, kind: Kind::KeyBody }], "at the end of a line");
		let odd = res!(der("3033020100300506032B657004220420", 53));
		req!(secret::scan(wrapped(&odd, 70, "\n").as_bytes()),
			vec![Find { line: 2, kind: Kind::KeyBody }], "at the start of a line");
		Ok(())
	}));

	res!(test_it(filter, &["A run too wide to be read at every offset is read at its front", "all",
		"secret", "body", "der"], ||
	{
		// The span DER keys are held to, put to base64: a key behind a span of other bytes is where
		// a compiled artefact carries one, and it is not looked for there.
		let key = res!(der(DKIM.0, DKIM.1));
		let mut inside = vec![0x5A; secret::DER_SPAN - key.len()];
		inside.extend_from_slice(&key);
		req!(inside.len(), secret::DER_SPAN);
		let found = secret::scan(wrapped(&inside, 64, "\n").as_bytes());
		req!(found.len(), 1, "at the span");
		req!(found[0].kind, Kind::KeyBody);
		let mut over = vec![0x5A; secret::DER_SPAN + 1 - key.len()];
		over.extend_from_slice(&key);
		req!(over.len(), secret::DER_SPAN + 1);
		req!(secret::scan(wrapped(&over, 64, "\n").as_bytes()), Vec::<Find>::new(), "past the span");
		// The front is read whatever the run's size.
		let mut wide = key.clone();
		wide.resize(secret::DER_SPAN * 4, 0x5A);
		let text = wrapped(&wide, 64, "\n");
		req!(secret::scan(text.as_bytes()), vec![Find { line: 2, kind: Kind::KeyBody }], "front");
		// A run far wider than a key is read once and not at every line.
		let mut state = 0x2545F4914F6CDD1Du64;
		let mut big = String::new();
		for _ in 0..20_000 {
			big.push_str(&noise(&mut state, 64));
			big.push('\n');
		}
		req!(secret::scan(big.as_bytes()), Vec::<Find>::new(), "twenty thousand lines");
		Ok(())
	}));

	res!(test_it(filter, &["An OpenSSH private key body is known by its magic", "all", "secret", "body",
		"openssh"], ||
	{
		// ssh-keygen's own format is no DER: it opens with a fixed string, which is as exact a
		// mark as an object identifier and holds a private key whether or not a passphrase wraps it.
		let mut key = OPENSSH_HEAD.to_vec();
		key.resize(400, 0x5A);
		let body = wrapped(&key, 70, "\n");
		req!(secret::scan(body.as_bytes()), vec![Find { line: lines_in(&body), kind: Kind::KeyBody }]);
		let (head, tail) = armour("OPENSSH ");
		let pem = fmt!("{}{}{}", head, body, tail);
		req!(secret::scan(pem.as_bytes()), vec![Find { line: 1, kind: Kind::PrivateKey }], "armoured");
		let labelled = fmt!("id\n{}", body);
		req!(secret::scan(labelled.as_bytes()),
			vec![Find { line: 1 + lines_in(&body), kind: Kind::KeyBody }], "labelled");
		// Behind other bytes in a small run it is still found, and in a wide one it is not.
		let mut behind = vec![0x41; 40];
		behind.extend_from_slice(&key);
		req!(secret::scan(wrapped(&behind, 70, "\n").as_bytes()).len(), 1, "behind");
		let mut far = vec![0x41; 31_000];
		far.extend_from_slice(&key);
		far.resize(34_400, 0x41);
		req!(secret::scan(wrapped(&far, 70, "\n").as_bytes()), Vec::<Find>::new(), "far");
		far.truncate(31_000 + key.len());
		req!(secret::scan(wrapped(&far, 70, "\n").as_bytes()).len(), 1, "as far, and no wider");
		// The public half of the same pair opens with a length and a name instead.
		let mut public = b"\0\0\0\x0bssh-ed25519\0\0\0\x20".to_vec();
		public.resize(51, 0x5A);
		let line = fmt!("ssh-ed25519 {} me@host\n", base64::encode(&public));
		req!(secret::scan(line.as_bytes()), Vec::<Find>::new(), "public key");
		Ok(())
	}));

	res!(test_it(filter, &["A key made by openssl or ssh-keygen is caught without its armour", "all",
		"secret", "body", "real"], ||
	{
		// The oracle is the tools themselves: each key is generated here, in TMPDIR, read, scanned
		// and removed with the directory. Nothing of one is printed or kept.
		let dir = res!(Dir::new("fe2o3_text_secret"));
		let makers: &[(&str, &str, &[&str], &str)] = &[
			("RSA-2048, PKCS#8",	"openssl",		&["genpkey", "-algorithm", "RSA", "-pkeyopt",
				"rsa_keygen_bits:2048", "-out", "rsa.pem"],								"rsa.pem"),
			("P-256, PKCS#8",		"openssl",		&["genpkey", "-algorithm", "EC", "-pkeyopt",
				"ec_paramgen_curve:P-256", "-out", "ec.pem"],							"ec.pem"),
			("P-256, SEC1",			"openssl",		&["ecparam", "-name", "prime256v1", "-genkey",
				"-noout", "-out", "sec1.pem"],											"sec1.pem"),
			("ed25519, PKCS#8",		"openssl",		&["genpkey", "-algorithm", "ED25519", "-out",
				"ed.pem"],																"ed.pem"),
			("ed25519, OpenSSH",	"ssh-keygen",	&["-q", "-t", "ed25519", "-N", "", "-f",
				"ssh_ed"],																"ssh_ed"),
			("RSA-2048, OpenSSH",	"ssh-keygen",	&["-q", "-t", "rsa", "-b", "2048", "-N", "", "-f",
				"ssh_rsa"],																"ssh_rsa"),
			("RSA-2048, PKCS#1",	"ssh-keygen",	&["-q", "-t", "rsa", "-b", "2048", "-m", "PEM",
				"-N", "", "-f", "ssh_pem"],												"ssh_pem"),
		];
		let mut tried = 0;
		for (what, program, args, file) in makers {
			if !res!(run(&dir, program, args)) {
				continue;
			}
			tried += 1;
			let pem = res!(dir.read(file));
			req!(secret::scan(pem.as_bytes()), vec![Find { line: 1, kind: Kind::PrivateKey }],
				"armoured, {}", what);
			let body = body_of(&pem);
			req!(secret::scan(body.as_bytes()),
				vec![Find { line: lines_in(&body), kind: Kind::KeyBody }], "body, {}", what);
			let one = body.replace('\n', "");
			req!(secret::scan(fmt!("{}\n", one).as_bytes()),
				vec![Find { line: 1, kind: Kind::KeyBody }], "one line, {}", what);
			let crlf = body.replace('\n', "\r\n");
			req!(secret::scan(crlf.as_bytes()),
				vec![Find { line: lines_in(&body), kind: Kind::KeyBody }], "CRLF, {}", what);
		}
		test!("{} of {} keys were made and tried.", tried, makers.len());
		// And what is not a key, made the same way: a certificate and the public halves.
		if res!(run(&dir, "openssl", &["req", "-x509", "-newkey", "ec", "-pkeyopt",
			"ec_paramgen_curve:P-256", "-nodes", "-keyout", "throwaway.pem", "-out", "cert.pem",
			"-subj", "/CN=fixture", "-days", "1"]))
		{
			let cert = body_of(&res!(dir.read("cert.pem")));
			req!(secret::scan(cert.as_bytes()), Vec::<Find>::new(), "certificate");
			req!(secret::scan(fmt!("{}\n", cert.replace('\n', "")).as_bytes()), Vec::<Find>::new(),
				"certificate on one line");
		}
		if res!(run(&dir, "openssl", &["pkey", "-in", "rsa.pem", "-pubout", "-out", "rsa_pub.pem"])) {
			let public = body_of(&res!(dir.read("rsa_pub.pem")));
			req!(secret::scan(public.as_bytes()), Vec::<Find>::new(), "RSA public key");
		}
		// The public half ssh-keygen wrote beside the private one is a single line.
		if dir.path().join("ssh_ed.pub").exists() {
			let public = res!(dir.read("ssh_ed.pub"));
			req!(secret::scan(public.as_bytes()), Vec::<Find>::new(), "OpenSSH public key");
		}
		Ok(())
	}));

	res!(test_it(filter, &["A literal still open at the end of its line is caught", "all", "secret",
		"assigned", "open"], ||
	{
		// The other half of D3: a field, its separator, an opening quote and the characters typed
		// so far, with the closing quote not yet there. The end of the line closes the literal.
		let value = fmt!("{}{}", LITERAL.0, LITERAL.1);
		for quote in ['"', '\''] {
			for tail in ["", " ", "\t ", "\r"] {
				for field in ["password", "api_key", "SECRET"] {
					for sep in [" = ", ": ", "="] {
						let line = fmt!("{}{}{}{}{}\n", field, sep, quote, value, tail);
						req!(secret::scan(line.as_bytes()),
							vec![Find { line: 1, kind: Kind::Assigned }], "for {:?}", line.len());
					}
				}
			}
		}
		let end = fmt!("password = \"{}", value);
		req!(secret::scan(end.as_bytes()), vec![Find { line: 1, kind: Kind::Assigned }], "at the end");
		// What follows the run says it is not the end of a literal.
		for after in [" and more", ",", ";", ")", "/x"] {
			let line = fmt!("password = \"{}{}\n", value, after);
			req!(secret::scan(line.as_bytes()), Vec::<Find>::new(), "followed by {:?}", after);
		}
		// A placeholder is as excused open as it is closed, and a short run is as short.
		for open in ["api_key = \"your-key-goes-here-please", "api_key = 'xxxxxxxxxxxxxxxxxxxxxxxx",
			"password: \"9f3Bq7ZmR4tYuIoPkLj"]
		{
			req!(secret::scan(open.as_bytes()), Vec::<Find>::new(), "for {:?}", open);
		}
		let marked = fmt!("password = \"{} // {}\n", value, secret::MARKER);
		req!(secret::scan(marked.as_bytes()), Vec::<Find>::new(), "marked");
		Ok(())
	}));

	res!(test_it(filter, &["A token prefix shorter than its shape stays a fragment", "all", "secret",
		"shape"], ||
	{
		// By design: a shape has a minimum, and what is typed short of it is a few characters that
		// every run of letters resembles. The save that completes it is the one refused.
		let full = fmt!("let key = \"{}{}\";\n", "sk_live", "_AbCdEfGhIjKlMnOpQrSt");
		req!(secret::scan(full.as_bytes()), vec![Find { line: 1, kind: Kind::Stripe }]);
		let part = fmt!("let key = \"{}{}\n", "sk_live", "_AbCdEfGhIjKlMnOpQrS");
		req!(secret::scan(part.as_bytes()), Vec::<Find>::new(), "one short");
		Ok(())
	}));

	Ok(())
}
