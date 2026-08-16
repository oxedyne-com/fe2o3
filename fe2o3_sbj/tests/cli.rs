//! The authoring toolchain: the text form, the key file, and the `sbj` binary that uses them.
//!
//! The claim the round trip makes is that a document's text form is its source and its bytes are its
//! artefact, and that neither loses anything to the other. It is checked against the conformance
//! fixtures rather than against a document invented here, so that the compiler is held to the same
//! artefacts `SPEC.md` §7 holds the reader to: every fixture's `doc.jdat`, compiled with the
//! committed key at the time its `meta.jdat` declares, must give back the committed `doc.sbj`, byte
//! for byte. Dumped back to text and compiled again, it must give the same bytes once more.
//!
//! The binary is then run as a shell runs it, on a document it wrote itself, since a toolchain that
//! only works when called as a library is not a toolchain.

mod common;

use common::{
	Keys,
	Meta,
	DOC_JDAT,
	DOC_SBJ,
	META_JDAT,
};

use oxedyne_fe2o3_sbj::{
	doc,
	text::{
		self,
		KindDecl,
	},
	validate,
	SCHEMA_DOC,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

use std::{
	fs,
	path::{
		Path,
		PathBuf,
	},
	process::Command,
};

/// The label the fixtures give the one node kind the v0 vocabulary does not know.
///
/// A label a document invents for an unknown kind is declared rather than guessed, which is what the
/// compiler's `--kind` option passes. The conventional `sbj_k99` needs no declaring, and is what a
/// dump writes.
fn alien() -> Vec<KindDecl> {
	vec![KindDecl {
		label:	common::ALIEN_LABEL.to_string(),
		code:	common::ALIEN_CODE,
	}]
}

/// Every fixture's `doc.jdat` compiles to its `doc.sbj`, and dumps back to a document that does too.
#[test]
fn test_round_trip_every_fixture() -> Outcome<()> {
	// The JDAT text decoder is generous with a stack, and a fixture may nest to the depth limit of
	// SPEC.md §5. The limit is the format's and does not move to suit a test, so the test moves.
	let thread = match std::thread::Builder::new()
		.name("sbj_round_trip".to_string())
		.stack_size(common::STACK_BYTES)
		.spawn(round_trip)
	{
		Ok(thread) => thread,
		Err(e) => return Err(err!(e,
			"Could not spawn the thread the deepest fixture is compiled on."; Test, Init)),
	};
	match thread.join() {
		Ok(outcome) => outcome,
		Err(_) => Err(err!(
			"The thread compiling the fixtures did not return."; Test, Panic)),
	}
}

/// Compiles, reads and dumps every acceptance fixture.
fn round_trip() -> Outcome<()> {
	let root = common::fixtures_dir();
	let keys = res!(Keys::load(&root));
	let mut run = 0;
	// Acceptance fixtures the compiler has no business with, counted so the number can be asserted.
	let mut skipped = 0;

	for entry in res!(fs::read_dir(&root), IO, File) {
		let entry = res!(entry, IO, File);
		let dir = entry.path();
		// A rejection fixture carries no `meta.jdat`, and is the conformance suite's business.
		if !dir.is_dir() || !dir.join(META_JDAT).is_file() {
			continue;
		}
		let name = match dir.file_name().and_then(|s| s.to_str()) {
			Some(name) => name.to_string(),
			None => return Err(err!(
				"The fixture directory holds {}, whose name is not UTF-8.", dir.display();
			Test, Invalid)),
		};
		let meta = res!(Meta::from_dat(&res!(common::from_jdat_plain(
			&res!(common::read_text(&dir.join(META_JDAT)))
		))));
		let committed = res!(common::read_bytes(&dir.join(DOC_SBJ)));

		// The compiler is a DOCUMENT tool: it reads the JDAT text of a node tree and writes an
		// oxeweb document. A post and a card travel in the same container and are not node trees,
		// so there is nothing here for it to compile and this is not the suite that tests them.
		//
		// Excluded by an assertion rather than by a `continue`, because a fixture skipped in
		// silence is a fixture that could stop being run without anybody noticing. Every fixture
		// this loop meets is either compiled or is required to be one the compiler refuses.
		if meta.schema != SCHEMA_DOC {
			// The claim is not that the TEXT is unreadable: a flat record is perfectly good JDAT,
			// and the decoder parses it happily into a map. The claim is that the compiler cannot
			// WRITE it. `doc::write` puts every schema through the node-tree validator, so the one
			// route this tool has to an artefact is closed to a record, and it says so.
			let src = res!(common::read_text(&dir.join(DOC_JDAT)));
			let parsed = res!(text::decode(&src, &alien()));
			match compile(&parsed, &meta, &keys) {
				Ok(_) => return Err(err!(
					"The fixture '{}' declares the schema '{}', which is not a node tree, and the \
					document compiler wrote an artefact for it anyway. A record put through the \
					tree writer is a document nobody wrote.", name, meta.schema;
				Test, Invalid, Unexpected)),
				Err(_) => (),
			}
			res!(eq(&name, "the node count of a payload that is not a tree", &meta.nodes, &None));
			skipped += 1;
			continue;
		}

		// The text form of the document is its source: it compiles to the artefact that was
		// committed, byte for byte, or the compiler is not the writer the fixtures were written by.
		let src = res!(common::read_text(&dir.join(DOC_JDAT)));
		let tree = match text::decode(&src, &alien()) {
			Ok(tree) => tree,
			Err(e) => return Err(err!(e,
				"The '{}' of the fixture '{}' is not readable by the compiler.", DOC_JDAT, name;
			Test, Invalid)),
		};
		let built = res!(compile(&tree, &meta, &keys));
		res!(same(&name, "the compiled document", &built, &committed));

		// And what it compiles to is what a reader reads.
		let read = res!(doc::read(&built));
		res!(eq(&name, "the tree that was compiled", read.tree(), &tree));
		let stats = res!(validate::validate(read.tree(), &read.env().schema));
		res!(eq(&name, "the node count", &Some(try_into!(u64, stats.nodes)), &meta.nodes));
		res!(eq(&name, "the depth", &Some(try_into!(u64, stats.depth)), &meta.depth));
		res!(eq(&name, "the address", &read.env().hash, &meta.hash));

		// Dumped back to text, the document is the document: the tree survives, and the text it is
		// written as compiles to the same bytes at the same address. A dump needs no declaration of
		// the unknown kinds it carries, since it writes them under the conventional label.
		let dumped = res!(text::encode(read.tree()));
		let again = match text::decode(&dumped, &[]) {
			Ok(again) => again,
			Err(e) => return Err(err!(e,
				"The dump of the fixture '{}' is not readable by the compiler.", name;
			Test, Invalid)),
		};
		res!(eq(&name, "the dumped tree", &again, &tree));
		let rebuilt = res!(compile(&again, &meta, &keys));
		res!(same(&name, "the recompiled dump", &rebuilt, &committed));

		// The dump is stable: a document dumped, compiled and dumped again is the same text.
		let dumped_again = res!(text::encode(res!(doc::read(&rebuilt)).tree()));
		res!(eq(&name, "the text form", &dumped_again, &dumped));

		run += 1;
	}

	if run == 0 {
		return Err(err!(
			"No acceptance fixture was found in {}, so nothing was round tripped.",
			common::fixtures_dir().display();
		Test, Missing));
	}
	// The four fixtures of the two record schemas. Named as a number rather than left open, so that
	// a fixture which quietly stopped being a document -- or a document that quietly stopped being
	// compiled -- shows up here instead of passing as an exclusion.
	if skipped != 4 {
		return Err(err!(
			"{} acceptance fixtures were excluded from the compiler, and four are expected: the 			two posts and the two cards. A different number means a fixture changed schema, or one 			was added without this count being thought about.", skipped;
		Test, Invalid, Mismatch));
	}
	Ok(())
}

/// Compiles a tree as its fixture declares it was written.
fn compile(
	tree:	&Dat,
	meta:	&Meta,
	keys:	&Keys,
)
	-> Outcome<Vec<u8>>
{
	let signer = res!(keys.author.signer());
	if meta.index {
		doc::write_with_index(tree, &meta.schema, &signer, meta.time)
	} else {
		doc::write(tree, &meta.schema, &signer, meta.time)
	}
}

/// The dump of a fixture is the source it was written from, character for character.
///
/// This is not required of the format, which cares only that the tree survives, but it is what makes
/// a dump worth reading: a document a reader holds and an author's source are the same text, so the
/// two can be compared with a diff rather than a decoder.
#[test]
fn test_a_dump_is_the_source() -> Outcome<()> {
	let thread = match std::thread::Builder::new()
		.name("sbj_dump".to_string())
		.stack_size(common::STACK_BYTES)
		.spawn(dump_is_source)
	{
		Ok(thread) => thread,
		Err(e) => return Err(err!(e,
			"Could not spawn the thread the fixture is dumped on."; Test, Init)),
	};
	match thread.join() {
		Ok(outcome) => outcome,
		Err(_) => Err(err!("The thread dumping the fixture did not return."; Test, Panic)),
	}
}

/// Dumps the fixture that uses every node kind, and compares the text with the fixture's source.
fn dump_is_source() -> Outcome<()> {
	let dir = common::fixtures_dir().join("every_kind");
	let buf = res!(common::read_bytes(&dir.join(DOC_SBJ)));
	let src = res!(common::read_text(&dir.join(DOC_JDAT)));
	let dumped = res!(text::encode(res!(doc::read(&buf)).tree()));
	if dumped != src {
		return Err(err!(
			"The dump of 'every_kind' is not its source. The source is {} characters and the dump \
			is {}:\n{}", src.len(), dumped.len(), dumped;
		Test, Invalid, Mismatch));
	}
	Ok(())
}

/// The binary runs, and does to a document of its own what it does to a fixture.
#[test]
fn test_the_binary_runs() -> Outcome<()> {
	let dir = res!(scratch("binary"));
	let source = dir.join("doc.jdat");
	let keyfile = dir.join("key.jdat");
	let artefact = dir.join("doc.sbj");
	let dumped = dir.join("dumped.jdat");
	let rebuilt = dir.join("rebuilt.sbj");
	res!(write_text(&source, SOURCE));

	// Compile. The key file is not there, so it is generated and saved, and the author is told where.
	let out = res!(sbj(&[
		"compile",	&path(&source),
		"-o",	&path(&artefact),
		"--key",	&path(&keyfile),
		"--time",	"1752000000000",
	]));
	res!(says(&out, "Generated a signing key"));
	res!(says(&out, SCHEMA_DOC));
	// The doc, its heading and that heading's text, its paragraph and that paragraph's text.
	res!(says(&out, "nodes        5, depth 3"));
	if !keyfile.is_file() {
		return Err(err!(
			"The compiler signed a document and saved no key at {}.", keyfile.display();
		Test, Missing));
	}

	// Verify, which touches no content.
	let out = res!(sbj(&["verify", &path(&artefact)]));
	res!(says(&out, "signature    good"));
	res!(says(&out, "not decoded"));

	// Inspect, which reads the document whole.
	let out = res!(sbj(&["inspect", &path(&artefact)]));
	res!(says(&out, "kinds        doc 1, para 1, heading 1, text 2"));
	res!(says(&out, "styles       lede: size = 1"));
	res!(says(&out, "index        none"));

	// Dump, and compile the dump: the same key, the same time, the same document, the same bytes.
	res!(sbj(&["dump", &path(&artefact), "-o", &path(&dumped)]));
	res!(sbj(&[
		"compile",	&path(&dumped),
		"-o",	&path(&rebuilt),
		"--key",	&path(&keyfile),
		"--time",	"1752000000000",
	]));
	let first = res!(common::read_bytes(&artefact));
	let second = res!(common::read_bytes(&rebuilt));
	res!(same("the binary", "the recompiled dump", &second, &first));

	res!(clean(&dir));
	Ok(())
}

/// A document that fails is refused, the shell hears about it, and the refusal names the rule.
#[test]
fn test_the_binary_refuses_and_says_why() -> Outcome<()> {
	let dir = res!(scratch("refuses"));
	let source = dir.join("doc.jdat");
	let keyfile = dir.join("key.jdat");
	let artefact = dir.join("doc.sbj");
	let corrupt = dir.join("corrupt.sbj");

	// A heading of level 7 never reaches a file, so it never gets an address.
	res!(write_text(&source, &SOURCE.replace("(u8|2)", "(u8|7)")));
	let (code, _, err) = res!(sbj_raw(&[
		"compile",	&path(&source),
		"-o",	&path(&artefact),
		"--key",	&path(&keyfile),
	]));
	res!(exits(code, 1, "compiling a heading of level 7"));
	res!(says(&err, "Node 1"));
	res!(says(&err, "1..=6"));

	// A corrupted byte of the tree region is caught by the hash, before anything decodes it.
	res!(write_text(&source, SOURCE));
	res!(sbj(&[
		"compile",	&path(&source),
		"-o",	&path(&artefact),
		"--key",	&path(&keyfile),
	]));
	let mut buf = res!(common::read_bytes(&artefact));
	let last = buf.len() - 1;
	buf[last] ^= 0x01;
	res!(common::write_bytes(&corrupt, &buf));

	let (code, _, err) = res!(sbj_raw(&["verify", &path(&corrupt)]));
	res!(exits(code, 1, "verifying a corrupted tree"));
	res!(says(&err, "hashes to"));

	// And a subcommand that is not one, and an option that is not one.
	let (code, _, err) = res!(sbj_raw(&["render", &path(&artefact)]));
	res!(exits(code, 1, "a subcommand that is not one"));
	res!(says(&err, "is not an sbj subcommand"));
	let (code, _, err) = res!(sbj_raw(&["verify", &path(&artefact), "--quickly"]));
	res!(exits(code, 1, "an option that is not one"));
	res!(says(&err, "is not an sbj option"));

	res!(clean(&dir));
	Ok(())
}

/// The document the binary is run on: a heading, a paragraph, and a style the paragraph names.
const SOURCE: &'static str = "\
(sbj_doc|(map|{
    (str|\"children\"): (list|[
        (sbj_heading|(map|{
            (str|\"children\"): (list|[(sbj_text|(str|\"Style without a cascade\"))]),
            (str|\"level\"): (u8|2),
        })),
        (sbj_para|(map|{
            (str|\"children\"): (list|[(sbj_text|(str|\"A lede paragraph.\"))]),
            (str|\"style\"): (str|\"lede\"),
        })),
    ]),
    (str|\"lang\"): (str|\"en\"),
    (str|\"styles\"): (map|{
        (str|\"lede\"): (map|{
            (str|\"size\"): (i8|1),
        }),
    }),
    (str|\"title\"): (str|\"A document\"),
}))
";

/// Runs the binary, refusing a run that fails.
fn sbj(args: &[&str]) -> Outcome<String> {
	let (code, out, err) = res!(sbj_raw(args));
	if code != 0 {
		return Err(err!(
			"`sbj {}` exited {}, and says: {}", args.join(" "), code, err;
		Test, Invalid));
	}
	Ok(out)
}

/// Runs the binary, returning its exit code, its output, and what it said to the shell.
fn sbj_raw(args: &[&str]) -> Outcome<(i32, String, String)> {
	let out = match Command::new(env!("CARGO_BIN_EXE_sbj")).args(args).output() {
		Ok(out) => out,
		Err(e) => return Err(err!(e,
			"Could not run the sbj binary at {}.", env!("CARGO_BIN_EXE_sbj");
		Test, IO)),
	};
	let code = match out.status.code() {
		Some(code) => code,
		None => return Err(err!(
			"`sbj {}` was killed by a signal rather than exiting.", args.join(" ");
		Test, Invalid)),
	};
	Ok((
		code,
		String::from_utf8_lossy(&out.stdout).to_string(),
		String::from_utf8_lossy(&out.stderr).to_string(),
	))
}

/// Requires an exit code, naming what was run.
fn exits(
	code:	i32,
	want:	i32,
	what:	&str,
)
	-> Outcome<()>
{
	if code == want {
		Ok(())
	} else {
		Err(err!(
			"{} exited {}, where {} was required. A document that fails is refused, and the shell \
			hears about it.", what, code, want;
		Test, Invalid, Mismatch))
	}
}

/// Requires the tool to have said something, naming what it said instead.
fn says(
	out:	&str,
	what:	&str,
)
	-> Outcome<()>
{
	if out.contains(what) {
		Ok(())
	} else {
		Err(err!(
			"The tool was required to say '{}', and says:\n{}", what, out;
		Test, Invalid, Mismatch))
	}
}

/// Requires two byte strings to be equal, naming the first byte at which they part.
fn same(
	name:	&str,
	what:	&str,
	got:	&[u8],
	want:	&[u8],
)
	-> Outcome<()>
{
	if got == want {
		return Ok(());
	}
	Err(err!(
		"For '{}', {} is {} bytes against the {} required{}. A document written twice is the same \
		document.",
		name, what, got.len(), want.len(),
		match common::first_diff(got, want) {
			Some(at) => fmt!(", first differing at byte {}", at),
			None => String::new(),
		};
	Test, Invalid, Mismatch))
}

/// Requires two things to be equal, naming the fixture and what was compared.
fn eq<T: PartialEq + std::fmt::Debug>(
	name:	&str,
	what:	&str,
	got:	&T,
	want:	&T,
)
	-> Outcome<()>
{
	if got == want {
		Ok(())
	} else {
		Err(err!(
			"For '{}', {} is {:?}, where {:?} was required.", name, what, got, want;
		Test, Invalid, Mismatch))
	}
}

/// A directory to work in, emptied of whatever a previous run left there.
fn scratch(what: &str) -> Outcome<PathBuf> {
	let dir = std::env::temp_dir().join(fmt!("sbj_cli_{}_{}", what, std::process::id()));
	if dir.exists() {
		res!(clean(&dir));
	}
	match fs::create_dir_all(&dir) {
		Ok(()) => Ok(dir),
		Err(e) => Err(err!(e,
			"Could not make the working directory {}.", dir.display();
		Test, IO, File)),
	}
}

/// Removes a working directory.
fn clean(dir: &Path) -> Outcome<()> {
	match fs::remove_dir_all(dir) {
		Ok(()) => Ok(()),
		Err(e) => Err(err!(e,
			"Could not remove the working directory {}.", dir.display();
		Test, IO, File)),
	}
}

/// Writes a text file, naming it if it will not.
fn write_text(
	path:	&Path,
	src:	&str,
)
	-> Outcome<()>
{
	common::write_bytes(path, src.as_bytes())
}

/// A path as the shell takes it.
fn path(p: &Path) -> String {
	fmt!("{}", p.display())
}
