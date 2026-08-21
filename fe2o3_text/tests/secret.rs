//! Every credential in this file is spelled in two pieces and joined at run time, so that the
//! scanners which read this very file -- the git hook, and this crate's own scanner under a
//! version control system that cannot forget -- find nothing in it to refuse.

use oxedyne_fe2o3_text::secret::{
	self,
	Find,
	Kind,
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

	Ok(())
}
