//! Writes the conformance fixtures of `SPEC.md` §7.
//!
//! Run it with `cargo run -p sbj --example gen_fixtures`. Every artefact under `fixtures/` comes
//! from here, so that when the format changes the fixtures are rebuilt rather than patched, and no
//! byte of them is a byte nobody can reproduce.
//!
//! Two things this deliberately does not do. It does not sign with a fresh key, since a fixture that
//! changes on every run is not a fixture: the key is committed at `fixtures/key.jdat`, and is
//! generated once, the first time this runs against an empty directory. And it does not ask the
//! implementation what it thinks of a bad file: every `reject.jdat` is written from `SPEC.md`, by
//! hand, so that the suite tests the code against the specification rather than against itself.

#[path = "../tests/common/mod.rs"]
mod common;

use common::{
	tree,
	Keys,
	Meta,
	Reject,
	Stage,
	ALIEN_CODE,
	DOC_JDAT,
	DOC_SBJ,
	KEY_FILE,
	META_JDAT,
	README_FILE,
	REJECT_JDAT,
	TIME,
};

use oxedyne_fe2o3_sbj::{
	canon,
	doc,
	envelope,
	kinds::{
		NodeKind,
		ReservedKind,
	},
	validate,
	SCHEMA_DOC,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::sign::SignatureScheme;
use oxedyne_fe2o3_jdat::prelude::*;

use std::{
	fs,
	path::{
		Path,
		PathBuf,
	},
};

/// A schema no validator in this build reads, for the fixture that declares one.
const SCHEMA_FOREIGN: &'static str = "oxeweb/cmd/0";

fn main() {
	// The deepest legal document nests daticles 770 deep, and encoding one costs a stack frame at
	// every level, so the work runs on a thread with a stack that can hold it.
	let thread = match std::thread::Builder::new()
		.name("gen_fixtures".to_string())
		.stack_size(common::STACK_BYTES)
		.spawn(generate)
	{
		Ok(thread) => thread,
		Err(e) => {
			println!("Could not spawn the generator thread: {}", e);
			std::process::exit(1);
		},
	};
	match thread.join() {
		Ok(Ok(n)) => println!("Wrote {} fixtures.", n),
		Ok(Err(e)) => {
			println!("{}", e);
			std::process::exit(1);
		},
		Err(_) => {
			println!("The generator thread did not return.");
			std::process::exit(1);
		},
	}
}

/// Writes every fixture, and returns how many were written.
fn generate() -> Outcome<usize> {

	let root = common::fixtures_dir();
	res!(fs::create_dir_all(&root), IO, File);

	// The key is generated once and committed. A fixture signed by a key that changed would carry a
	// different signature every run, and a suite that had to be regenerated to pass would test
	// nothing.
	let keys = if root.join(KEY_FILE).exists() {
		res!(Keys::load(&root))
	} else {
		let keys = res!(Keys::generate());
		res!(keys.save(&root));
		keys
	};

	res!(common::write_bytes(&root.join(README_FILE), readme().as_bytes()));

	let n = res!(acceptance(&root, &keys)) + res!(rejection(&root, &keys));

	// A fixture directory nothing above wrote is a fixture nobody can regenerate, and the suite will
	// refuse to skip it, so it is caught here rather than there.
	let mut dirs = 0;
	for entry in res!(fs::read_dir(&root), IO, File) {
		if res!(entry, IO, File).path().is_dir() {
			dirs += 1;
		}
	}
	if dirs != n {
		return Err(err!(
			"{} fixtures were written, and {} directories are in {}. A directory nothing here wrote \
			is stale: delete it, or write the fixture that belongs in it.", n, dirs, root.display();
		Invalid, Mismatch));
	}
	Ok(n)
}

/// The fixtures a reader must accept.
fn acceptance(
	root:	&Path,
	keys:	&Keys,
)
	-> Outcome<usize>
{
	let author = res!(keys.author.signer());

	res!(accept(root, &author, "empty", &tree::empty(), false,
		"A document with no content at all: the smallest thing that is still a document."));

	res!(accept(root, &author, "one_para", &tree::one_para(), false,
		"One paragraph of one text run."));

	res!(accept(root, &author, "every_kind", &tree::every_kind(), false,
		"Every v0 node kind once, including code and quote, a link by the typed address form and an \
		image by content hash, an optional field present, another absent, and a node carrying no \
		children."));

	res!(accept(root, &author, "styled", &tree::styled(), false,
		"A document exercising the style table of §4.4: an inherited property (size) and a self-only \
		property (bg), a box naming one style and a paragraph within it naming another, each \
		resolving to a table entry."));

	res!(accept(root, &author, "align_is_local", &tree::align_is_local(), false,
		"A box naming `align: justify`, holding a paragraph that names nothing. `align` is self-only \
		(§4.4), so it aligns the lines of the node that named it, and the box has none: the \
		paragraph inside must be set exactly as it would be with no style at all. The property \
		inherits in CSS and does not in the format, so a reader built on a browser will justify the \
		paragraph unless it is stopped, and then one document says two things."));

	res!(accept(root, &author, "link_by_name", &tree::link_by_name(), false,
		"A link whose typed address (§4.3) is a NAMES name."));

	res!(accept(root, &author, "link_by_hash", &tree::link_by_hash(), false,
		"A link whose typed address (§4.3) is a b32 content hash."));

	res!(accept(root, &author, "unknown_kind_fallback", &tree::unknown_fallback(), false,
		"An unknown node kind carrying a non-empty fallback of known nodes (§4.5), which a reader \
		that does not implement the kind renders and validates in its place, so the document is \
		accepted. The unknown node also carries an uninterpreted field, held only to §3."));

	res!(accept(root, &author, "indexed", &tree::one_para(), true,
		"The document of the one_para fixture, written with the optional index of §1.4 appended. \
		The index lies outside the hash, so this is the same document at the same address, and its \
		hash is the hash of one_para."));

	res!(accept(root, &author, "depth_at_limit", &res!(tree::chain(common::depth_limit())), false,
		"Nesting at the depth limit of §5: a doc at the head of a chain of boxes, 256 nodes deep."));

	res!(accept(root, &author, "size_at_limit", &res!(tree::sized(common::tree_limit())), false,
		"A tree region of exactly the size limit of §5, to the byte."));

	Ok(11)
}

/// Writes an acceptance fixture: the tree, the artefact, and what the artefact must turn out to be.
fn accept(
	root:	&Path,
	author:	&SignatureScheme,
	name:	&str,
	tree:	&Dat,
	index:	bool,
	note:	&str,
)
	-> Outcome<()>
{
	let dir = res!(fresh_dir(root, name));
	let buf = if index {
		res!(doc::write_with_index(tree, SCHEMA_DOC, author, TIME))
	} else {
		res!(doc::write(tree, SCHEMA_DOC, author, TIME))
	};
	let env = res!(doc::verify_only(&buf));
	let stats = res!(validate::validate(tree, SCHEMA_DOC));
	let meta = Meta {
		schema:	env.schema.clone(),
		time:	env.time,
		hash:	env.hash.clone(),
		tree_len:	env.tree_len,
		nodes:	try_into!(u64, stats.nodes),
		depth:	try_into!(u64, stats.depth),
		index,
		note:	note.to_string(),
	};
	res!(common::write_bytes(&dir.join(DOC_JDAT), res!(common::to_jdat(tree)).as_bytes()));
	res!(common::write_bytes(&dir.join(DOC_SBJ), &buf));
	res!(common::write_bytes(
		&dir.join(META_JDAT),
		res!(common::to_jdat_plain(&meta.to_dat())).as_bytes(),
	));
	Ok(())
}

/// The fixtures a reader must refuse, and the reason each must be refused for.
fn rejection(
	root:	&Path,
	keys:	&Keys,
)
	-> Outcome<usize>
{
	let author = res!(keys.author.signer());
	let impostor = res!(keys.impostor.signer());

	// A good document, and the file that carries it. Every fixture below is this file with one
	// thing wrong with it, or a file assembled around one tree that is wrong in one way.
	let good = res!(canon::encode(&tree::every_kind()));
	let file = res!(common::assemble(&res!(common::seal(&good, SCHEMA_DOC, &author, TIME)), &good));

	// -- Step 1: the header. ------------------------------------------------------------------

	// A file that is perfectly good BDAT and is not an SBJ file at all: the encoded tree of a real
	// document, with nothing in front of it.
	res!(reject(root, "bdat_not_sbj", &good, None, Reject {
		stage:	Stage::Header,
		rule:	"SPEC.md §1.1: a reader that does not recognise the magic stops.".to_string(),
		says:	"Not an SBJ file".to_string(),
		node:	None,
		offset:	None,
		note:	"Valid BDAT: the encoded tree of a real document, with no SBJ header in front of \
			it. Nothing about a bare daticle says which format it belongs to, so the magic is \
			what says so, and its absence is a rejection rather than a guess.".to_string(),
	}));

	let mut bad = file.clone();
	bad[1] ^= 0xFF;
	res!(reject(root, "bad_magic", &bad, None, Reject {
		stage:	Stage::Header,
		rule:	"SPEC.md §1.1: the magic is 'SBJ\\0'.".to_string(),
		says:	"Not an SBJ file".to_string(),
		node:	None,
		offset:	None,
		note:	"Byte 1 of the magic is flipped. Everything after it is a valid document."
			.to_string(),
	}));

	let mut bad = file.clone();
	bad[5] = 1;
	res!(reject(root, "bad_version", &bad, None, Reject {
		stage:	Stage::Header,
		rule:	"SPEC.md §1.1: a reader that reads a major version it does not implement stops."
			.to_string(),
		says:	"not implemented here".to_string(),
		node:	None,
		offset:	None,
		note:	"The major version is 1, and this reads version 0. It does not guess.".to_string(),
	}));

	// -- Step 2: the envelope. ----------------------------------------------------------------

	let env = res!(common::seal(&good, SCHEMA_DOC, &author, TIME));
	let mut map = match res!(env.to_dat()) {
		Dat::Map(map) => map,
		d => return Err(err!("An envelope is a map, found a {:?}.", d.kind(); Bug, Invalid)),
	};
	map.remove(&dat!(envelope::KEY_TIME));
	let bad = res!(common::assemble_raw(&Dat::Map(map), &good));
	res!(reject(root, "envelope_missing_key", &bad, Some(&tree::every_kind()), Reject {
		stage:	Stage::Envelope,
		rule:	"SPEC.md §1.2: the envelope carries exactly these keys, all required.".to_string(),
		says:	"missing the required key \"time\"".to_string(),
		node:	None,
		offset:	None,
		note:	"The 'time' key is gone from the envelope map. The tree behind it is a good \
			document, and is never reached.".to_string(),
	}));

	// -- Step 3: the tree region. -------------------------------------------------------------

	let bad = file[..file.len() - 1].to_vec();
	res!(reject(root, "truncated_tree", &bad, None, Reject {
		stage:	Stage::Region,
		rule:	"SPEC.md §2 step 3: a tree region shorter than declared is a rejection, not a \
			truncation.".to_string(),
		says:	"shorter than declared".to_string(),
		node:	None,
		offset:	None,
		note:	"The last byte of the tree region is gone. The envelope still declares the length \
			the region had, and the reader believes neither the bytes nor the envelope: it \
			refuses the file.".to_string(),
	}));

	let over = res!(tree::sized(common::tree_limit() + 1));
	let over_bytes = res!(canon::encode(&over));
	let bad = res!(common::assemble(
		&res!(common::seal(&over_bytes, SCHEMA_DOC, &author, TIME)),
		&over_bytes,
	));
	res!(reject(root, "size_over_limit", &bad, None, Reject {
		stage:	Stage::Region,
		rule:	"SPEC.md §5: the tree region size limit is 4 MiB, enforced before decoding."
			.to_string(),
		says:	"exceeding the limit".to_string(),
		node:	None,
		offset:	None,
		note:	"A tree region of 4 MiB and one byte, correctly hashed and correctly signed. The \
			limit is enforced on the envelope's word alone, before a byte of the region is \
			hashed, let alone decoded.".to_string(),
	}));

	// -- Step 4: the hash. --------------------------------------------------------------------

	let start = res!(common::tree_start(&file));
	let mut bad = file.clone();
	bad[start + 3] ^= 0x01;
	res!(reject(root, "corrupt_tree_byte", &bad, None, Reject {
		stage:	Stage::Hash,
		rule:	"SPEC.md §2 step 4: the tree region hashes to what the envelope declares."
			.to_string(),
		says:	"hashes to".to_string(),
		node:	None,
		offset:	None,
		note:	"One bit of one byte of the tree region is flipped. The hash is the document's \
			address, so a tree that does not hash to it is not this document.".to_string(),
	}));

	// The author signs the corrupted hash, so the signature is sound and the hash alone is wrong.
	let mut env = res!(common::seal(&good, SCHEMA_DOC, &author, TIME));
	env.hash[0] ^= 0x01;
	res!(common::resign(&mut env, &author));
	let bad = res!(common::assemble(&env, &good));
	res!(reject(root, "bad_hash", &bad, Some(&tree::every_kind()), Reject {
		stage:	Stage::Hash,
		rule:	"SPEC.md §2 step 4: a hash that is not the hash of the tree region is a rejection."
			.to_string(),
		says:	"hashes to".to_string(),
		node:	None,
		offset:	None,
		note:	"The hash in the envelope is corrupted, and the author has signed the corrupted \
			hash, so the signature is sound and only the hash is wrong. The reader hashes the \
			region itself rather than taking the envelope's word for it.".to_string(),
	}));

	// -- Step 5: the signature. ---------------------------------------------------------------

	let mut env = res!(common::seal(&good, SCHEMA_DOC, &author, TIME));
	env.sig[0] ^= 0x01;
	let bad = res!(common::assemble(&env, &good));
	res!(reject(root, "bad_sig", &bad, Some(&tree::every_kind()), Reject {
		stage:	Stage::Sig,
		rule:	"SPEC.md §2 step 5: the signature is verified over the signing input, under the \
			author's key.".to_string(),
		says:	"not a signature by the author".to_string(),
		node:	None,
		offset:	None,
		note:	"One bit of the signature is flipped. The hash is right, so the document is at the \
			address it claims to be at; nobody has vouched for it.".to_string(),
	}));

	// A perfectly good signature, made by a key that is not the author the envelope names.
	let mut env = res!(common::seal(&good, SCHEMA_DOC, &impostor, TIME));
	env.author = keys.author.pk.clone();
	let bad = res!(common::assemble(&env, &good));
	res!(reject(root, "wrong_key", &bad, Some(&tree::every_kind()), Reject {
		stage:	Stage::Sig,
		rule:	"SPEC.md §2 step 5: the signature must be the author's.".to_string(),
		says:	"not a signature by the author".to_string(),
		node:	None,
		offset:	None,
		note:	"A sound signature over the right signing input, made by a key that is not the \
			author the envelope names. A signature nobody checks against a key is not a \
			signature.".to_string(),
	}));

	// -- Step 6: decoding, and the canonical encoding rules of §3. ----------------------------

	// The tree region is one byte short of the tree it holds, and the author has hashed and signed
	// the short region, so every step that touches no content passes. The tree is cut off.
	let short = &good[..good.len() - 1];
	let mut env = res!(common::seal(short, SCHEMA_DOC, &author, TIME));
	env.tree_len = try_into!(u64, short.len());
	res!(common::resign(&mut env, &author));
	let bad = res!(common::assemble(&env, short));
	res!(reject(root, "tree_longer_than_tree_len", &bad, None, Reject {
		stage:	Stage::Decode,
		rule:	"SPEC.md §2: a tree region that does not hold a whole tree is a rejection."
			.to_string(),
		says:	SAYS_TRUNCATED.to_string(),
		node:	None,
		offset:	None,
		note:	"'tree_len' understates the tree by one byte, and the author has hashed and signed \
			the region it declares, so the file verifies. The declared region holds a tree cut \
			off one byte from its end, and the decoder says so.".to_string(),
	}));

	let mut region = good.clone();
	region.push(0x00);
	let bad = res!(common::assemble(
		&res!(common::seal(&region, SCHEMA_DOC, &author, TIME)),
		&region,
	));
	res!(reject(root, "bytes_trailing_the_tree", &bad, None, Reject {
		stage:	Stage::Decode,
		rule:	"SPEC.md §3: a document encodes to exactly one byte string.".to_string(),
		says:	"canonical".to_string(),
		node:	None,
		offset:	None,
		note:	"The tree region carries the tree and one byte more, all of it hashed and signed. \
			A byte the tree does not need gives the document a second address, so it is no \
			part of a canonical encoding.".to_string(),
	}));

	res!(canon_reject(root, &author, "canon_rule1_undeclared_field", Some(1), "rule 1",
		"SPEC.md §3 rule 1: field types are fixed by the schema.",
		"The heading carries a field the schema does not declare. A reader that ignored it would \
		accept two byte strings for one document.",
		tree::doc_with_heading(common::map(vec![
			("colour",	Dat::Str("red".to_string())),
			("level",	Dat::U8(2)),
			("children",	Dat::List(vec![common::text("A heading")])),
		])),
	));

	res!(canon_reject(root, &author, "canon_rule2_ordmap", Some(1), "rule 2",
		"SPEC.md §3 rule 2: maps are Dat::Map, never Dat::OrdMap.",
		"The heading's payload is an OrdMap, whose order follows the author's typing rather than \
		its keys, so the same heading typed in another order is a different byte string.",
		tree::doc_with_heading(create_dat_ordmap(vec![
			(Dat::Str("level".to_string()),	Dat::U8(2)),
			(Dat::Str("children".to_string()),	Dat::List(vec![common::text("A heading")])),
		])),
	));

	res!(canon_reject(root, &author, "canon_rule3_uppercase_key", Some(1), "rule 3",
		"SPEC.md §3 rule 3: map keys are strings, lowercase ASCII.",
		"The heading's level is spelled 'Level'. A key that may be spelled two ways is a document \
		with two addresses.",
		tree::doc_with_heading(common::map(vec![
			("Level",	Dat::U8(2)),
			("children",	Dat::List(vec![common::text("A heading")])),
		])),
	));

	// A duplicate key cannot be built as a tree at all: BDAT decodes a map into a BTreeMap, which
	// collapses the duplicate, so the tree that comes out is perfectly canonical. It survives only
	// in the bytes, so the bytes are built by hand here, and the first byte at which they differ
	// from the canonical encoding of what they decode to is computed here too, rather than read out
	// of the error the implementation happens to give.
	let dup = res!(duplicate_key_bytes());
	let (decoded, n) = res!(Dat::from_bytes(&dup));
	if n != dup.len() {
		return Err(err!(
			"The hand-built bytes of the duplicate key fixture do not decode whole."; Bug, Invalid));
	}
	let canonical = res!(canon::encode(&decoded));
	let at = match common::first_diff(&canonical, &dup) {
		Some(at) => at,
		None => return Err(err!("The duplicate key did not change the bytes."; Bug, Invalid)),
	};
	let bad = res!(common::assemble(
		&res!(common::seal(&dup, SCHEMA_DOC, &author, TIME)),
		&dup,
	));
	res!(reject(root, "canon_rule3_duplicate_key", &bad, None, Reject {
		stage:	Stage::Decode,
		rule:	"SPEC.md §3 rule 3: no map key may appear twice.".to_string(),
		says:	"rule 3".to_string(),
		node:	None,
		offset:	Some(try_into!(u64, at)),
		note:	"The doc's payload map carries the key 'lang' twice. The tree it decodes to is \
			perfectly canonical, since a BTreeMap cannot hold a duplicate, so the duplicate \
			survives only in the bytes: one tree, two byte strings, two addresses. The offset \
			is the first byte at which the bytes differ from the canonical encoding of the \
			tree they decode to.".to_string(),
	}));

	res!(canon_reject(root, &author, "canon_rule4_empty_children", Some(1), "rule 4",
		"SPEC.md §3 rule 4: no redundant wrappers, and a node with no children omits the key.",
		"The heading carries an empty children list rather than omitting the key, which gives a \
		childless heading two encodings.",
		tree::doc_with_heading(common::map(vec![
			("level",	Dat::U8(2)),
			("children",	Dat::List(Vec::new())),
		])),
	));

	res!(canon_reject(root, &author, "canon_rule4_opt_none", Some(1), "rule 4",
		"SPEC.md §3 rule 4: an absent optional field is omitted, not encoded as a none.",
		"The section's optional title is written as a none rather than left out, so an untitled \
		section has two encodings.",
		tree::doc_of(vec![
			common::node(NodeKind::Section, common::map(vec![
				("title",	Dat::Opt(Box::new(None))),
				("children", Dat::List(vec![
					common::node(NodeKind::Para, common::map(vec![
						("children", Dat::List(vec![common::text("A paragraph.")])),
					])),
				])),
			])),
		]),
	));

	res!(canon_reject(root, &author, "canon_rule5_control_char", Some(2), "rule 5",
		"SPEC.md §3 rule 5: strings carry no C0 or C1 control characters other than tab and \
		newline.",
		"The text run carries a carriage return, so one line ending would have two encodings.",
		tree::doc_with_heading(common::map(vec![
			("level",	Dat::U8(2)),
			("children",	Dat::List(vec![common::text("a carriage\rreturn")])),
		])),
	));

	res!(canon_reject(root, &author, "canon_rule5_not_nfc", Some(2), "rule 5",
		"SPEC.md §3 rule 5: strings are in Unicode NFC.",
		"The text run spells café with a combining acute accent rather than the composed letter. \
		It displays identically to the composed form and hashes differently, so the one document \
		would have two addresses.",
		tree::doc_with_heading(common::map(vec![
			("level",	Dat::U8(2)),
			("children",	Dat::List(vec![common::text("cafe\u{0301}")])),
		])),
	));

	res!(canon_reject(root, &author, "canon_rule6_int_width", Some(1), "rules 1 and 6",
		"SPEC.md §3 rule 6: integers are exactly the declared width, with no promotion and no \
		demotion.",
		"The heading's level is a u32 where the schema declares a u8. Both decode to the number 2, \
		and they are different byte strings, so they are two addresses for one document.",
		tree::doc_with_heading(common::map(vec![
			("level",	Dat::U32(2)),
			("children",	Dat::List(vec![common::text("A heading")])),
		])),
	));

	let vek = match Vek::try_from(vec![common::text("A heading")]) {
		Ok(vek) => vek,
		Err(e) => return Err(err!(e, "Could not build a Vek of one text run."; Bug, Invalid)),
	};
	res!(canon_reject(root, &author, "canon_rule7_vek_children", Some(1), "rule 7",
		"SPEC.md §3 rule 7: lists are Dat::List, not Dat::Vek, even where every element shares a \
		kind.",
		"The heading's children sit in a Vek. Every child of a heading is a node, so a Vek is \
		always available, and always a second encoding of the same list.",
		tree::doc_with_heading(common::map(vec![
			("level",	Dat::U8(2)),
			("children",	Dat::Vek(vek)),
		])),
	));

	// SPEC §4.5: an unknown kind is canonical enough to decode, so canon accepts it, and the
	// validator is what refuses one whose payload carries no non-empty fallback of known nodes. The
	// alien here carries an ordinary map with an uninterpreted field but no fallback, so it verifies,
	// decodes canonically, and is refused at validation.
	res!(schema_reject(root, &author, "unknown_kind", 1,
		"fallback",
		&fmt!("SPEC.md §4.5: an unknown kind code {} is permitted only with a non-empty fallback of \
			known nodes.", ALIEN_CODE),
		"The document's one child declares a kind code that names no v0 node kind and carries no \
		fallback. The bytes are canonical, so the file verifies and decodes; the vocabulary rule of \
		§4.5 is what refuses it, naming the node and the missing fallback.",
		tree::doc_of(vec![
			common::alien(common::map(vec![
				("rows",	Dat::Str("no fallback here".to_string())),
			])),
		]),
	));

	let deep = res!(tree::chain(common::depth_limit() + 1));
	let deep_bytes = res!(common::encode_unchecked(&deep));
	let bad = res!(common::assemble(
		&res!(common::seal(&deep_bytes, SCHEMA_DOC, &author, TIME)),
		&deep_bytes,
	));
	res!(reject(root, "depth_over_limit", &bad, None, Reject {
		stage:	Stage::Decode,
		rule:	fmt!("SPEC.md §5: the nesting depth limit is {}, enforced during decoding.",
			common::depth_limit()),
		says:	fmt!("past the limit of {}", common::depth_limit()),
		node:	Some(try_into!(u64, common::depth_limit())),
		offset:	None,
		note:	fmt!("A doc at the head of a chain of boxes {} nodes deep, correctly hashed and \
			correctly signed. A tiny file describing a deep nest is the cheapest attack there \
			is against a recursive decoder, so the depth limit is the decoder's rather than \
			the validator's.", common::depth_limit() + 1),
	}));

	// -- Step 7: the schema, and the limits that are not the decoder's. -----------------------

	let foreign = res!(canon::encode(&tree::one_para()));
	let bad = res!(common::assemble(
		&res!(common::seal(&foreign, SCHEMA_FOREIGN, &author, TIME)),
		&foreign,
	));
	res!(reject(root, "foreign_schema", &bad, Some(&tree::one_para()), Reject {
		stage:	Stage::Validate,
		rule:	"SPEC.md §1.2: the envelope declares the schema of its payload.".to_string(),
		says:	SCHEMA_FOREIGN.to_string(),
		node:	None,
		offset:	None,
		note:	"A sound envelope over a sound tree, declaring a schema this build does not \
			validate. The container carries any schema, and a reader that validates one \
			refuses the others rather than reading them as though they were documents."
			.to_string(),
	}));

	res!(schema_reject(root, &author, "para_in_para", 2,
		"admits inline content only",
		"SPEC.md §4.2: a para takes inline content.",
		"A paragraph inside a paragraph. The tree is canonical and the file verifies; the \
		vocabulary is what refuses it.",
		tree::doc_of(vec![
			common::node(NodeKind::Para, common::map(vec![
				("children", Dat::List(vec![
					common::node(NodeKind::Para, common::map(vec![
						("children", Dat::List(vec![common::text("Inner")])),
					])),
				])),
			])),
		]),
	));

	res!(schema_reject(root, &author, "heading_level_0", 1,
		"1..=6",
		"SPEC.md §4.2: a heading's level runs from 1 to 6.",
		"A heading of level 0. The field is a u8, exactly as the schema declares, and 0 is not a \
		heading level.",
		tree::doc_with_heading(common::map(vec![
			("level",	Dat::U8(0)),
			("children",	Dat::List(vec![common::text("A heading")])),
		])),
	));

	res!(schema_reject(root, &author, "heading_level_7", 1,
		"1..=6",
		"SPEC.md §4.2: a heading's level runs from 1 to 6.",
		"A heading of level 7, one past the deepest heading the format has.",
		tree::doc_with_heading(common::map(vec![
			("level",	Dat::U8(7)),
			("children",	Dat::List(vec![common::text("A heading")])),
		])),
	));

	res!(schema_reject(root, &author, "empty_list", 1,
		"must carry at least one",
		"SPEC.md §4.2: a list is marked `+` and carries at least one item.",
		"A list with an ordered field but no items. A document or section may be empty, but an \
		empty list is a construction error rather than intent.",
		tree::doc_of(vec![
			common::node(NodeKind::List, common::map(vec![
				("ordered", Dat::Bool(false)),
			])),
		]),
	));

	// A style field naming an entry the table does not define (§4.4). The table defines 'callout',
	// and the box names 'ghost'. The bytes are canonical, so the fault is the validator's: a style
	// name must resolve to a table entry, named at the node that made the reference.
	res!(schema_reject(root, &author, "style_missing_entry", 1,
		"ghost",
		"SPEC.md §4.4: a node's style field must name an entry the document's style table defines.",
		"A box names the style 'ghost', which the document's style table, defining only 'callout', \
		does not. A style error cannot escape the node that made it, and this one is named at the \
		box.",
		common::node(NodeKind::Doc, common::map(vec![
			("title",	Dat::Str("A style with no entry".to_string())),
			("lang",	Dat::Str("en".to_string())),
			("styles", common::map(vec![
				("callout", common::map(vec![
					("bg",	Dat::Str("muted".to_string())),
				])),
			])),
			("children", Dat::List(vec![
				common::node(NodeKind::Boxx, common::map(vec![
					("style",	Dat::Str("ghost".to_string())),
					("children", Dat::List(vec![
						common::node(NodeKind::Para, common::map(vec![
							("children", Dat::List(vec![common::text("in a box")])),
						])),
					])),
				])),
			])),
		])),
	));

	// A style record whose value is out of its enumeration (§4.4). The 'bg' property is a palette
	// name, and 'purple' is not one. The bytes are canonical, since canon pins only the type of a
	// palette value and not its membership, so the validator is what refuses it, at the doc where the
	// table is validated.
	res!(schema_reject(root, &author, "style_out_of_enum", 0,
		"purple",
		"SPEC.md §4.4: a palette property carries a palette name; the palette is ink, muted, accent, \
		bg.",
		"The style 'callout' declares a background of 'purple', which is not a palette name. The \
		style table is validated at the doc, so the rejection names node 0 and the offending style.",
		common::node(NodeKind::Doc, common::map(vec![
			("title",	Dat::Str("A colour off the palette".to_string())),
			("lang",	Dat::Str("en".to_string())),
			("styles", common::map(vec![
				("callout", common::map(vec![
					("bg",	Dat::Str("purple".to_string())),
				])),
			])),
			("children", Dat::List(vec![
				common::node(NodeKind::Para, common::map(vec![
					("children", Dat::List(vec![common::text("a paragraph")])),
				])),
			])),
		])),
	));

	// A malformed link address carrying two entries (§4.3). An address is a single-entry map, and
	// this one names both a name and a hash. Canon pins each entry's type but not the count, so the
	// bytes are canonical and the validator refuses the address, naming the link node.
	res!(schema_reject(root, &author, "link_two_entries", 2,
		"not a valid link address",
		"SPEC.md §4.3: a link address is a map with exactly one entry.",
		"The link's 'to' address carries both a name and a hash. A typed address selects one kind, \
		so two entries is not an address at all, and the reader refuses it at the door rather than \
		letting the renderer choose.",
		tree::doc_of(vec![
			common::node(NodeKind::Para, common::map(vec![
				("children", Dat::List(vec![
					common::node(NodeKind::Link, common::map(vec![
						("to",	common::map(vec![
							("name",	Dat::Str("news.cricket".to_string())),
							("hash",	Dat::from([0x9fu8; 32])),
						])),
						("children",	Dat::List(vec![common::text("a link")])),
					])),
				])),
			])),
		]),
	));

	// SPEC §4.2: the codes 14 and 15 are reserved to the chrome and to applications, and the document
	// schema admits them nowhere. The bytes of all three fixtures below are canonical, since canon has
	// no schema for a code outside the vocabulary and holds such a node only to §3; the validator is
	// what refuses them, and it refuses them by name.

	res!(schema_reject(root, &author, "reserved_edit_in_doc", 1,
		"may not carry an edit node",
		"SPEC.md §4.2: the kind code 14, `edit`, is reserved to the chrome and to applications, and \
		the document schema admits the kinds 1 to 13 and no others.",
		"A document carrying an editable text field. An `edit` is a facility of the engine, reached \
		by the chrome's address bar and by an application's own form fields, and a document may not \
		ask for one. The refusal names the kind and says that a document may not carry it.",
		tree::doc_of(vec![
			common::reserved(ReservedKind::Edit, common::map(vec![
				("placeholder",	Dat::Str("Search the oxeweb".to_string())),
			])),
		]),
	));

	res!(schema_reject(root, &author, "reserved_surface_in_doc", 1,
		"may not carry a surface node",
		"SPEC.md §4.2: the kind code 15, `surface`, is reserved to applications, and the document \
		schema admits the kinds 1 to 13 and no others.",
		"A document carrying a pane for an application to paint. A `surface` is the one place \
		anything but the author's own data reaches the screen, so a document that could name one \
		would be a program, and the whole design turns on a document never being one.",
		tree::doc_of(vec![
			common::reserved(ReservedKind::Surface, common::map(vec![
				("app",	Dat::Str("app.modeller".to_string())),
			])),
		]),
	));

	// The hole that §4.5 would leave open if a reserved code were treated as merely unknown. The
	// fallback here is everything §4.5 asks of one -- a non-empty list of known nodes, which validate
	// in full -- and it buys the surface nothing. Were it otherwise, an author could put a surface in
	// a document today, under a fallback that renders innocently, and every reader that later learned
	// what code 15 meant would begin honouring it: a document that became a program by waiting.
	res!(schema_reject(root, &author, "reserved_surface_with_fallback_still_refused", 1,
		"whether or not it carries a fallback",
		"SPEC.md §4.5: a fallback admits a kind the reader has never heard of, and never one the \
		reader knows the document schema does not admit.",
		"A document carrying a `surface` that also carries a valid, non-empty fallback of known \
		nodes. A fallback is forward compatibility for an unknown code, and code 15 is not unknown: \
		the reader knows exactly what it has been handed, and knows a document may not carry it, so \
		it refuses it with the fallback and without.",
		tree::doc_of(vec![
			common::reserved(ReservedKind::Surface, common::map(vec![
				("app",	Dat::Str("app.modeller".to_string())),
				("fallback", Dat::List(vec![
					common::node(NodeKind::Para, common::map(vec![
						("children", Dat::List(vec![
							common::text("A picture of a teapot, rendered by nobody."),
						])),
					])),
				])),
			])),
		]),
	));

	Ok(35)
}

/// What the decoder says of a tree region that ends before the tree in it does.
const SAYS_TRUNCATED: &'static str = "Not enough bytes";

/// Writes a fixture whose tree breaks a canonical encoding rule of §3.
///
/// The tree is encoded without being checked, since `canon::encode` refuses to give an address to a
/// document that has no canonical form, which is exactly what the rule being broken means. The bytes
/// are then hashed and signed like any other, so that the rejection can only be the rule's.
fn canon_reject(
	root:	&Path,
	author:	&SignatureScheme,
	name:	&str,
	node:	Option<u64>,
	says:	&str,
	rule:	&str,
	note:	&str,
	tree:	Dat,
)
	-> Outcome<()>
{
	let bytes = res!(common::encode_unchecked(&tree));
	let bad = res!(common::assemble(
		&res!(common::seal(&bytes, SCHEMA_DOC, author, TIME)),
		&bytes,
	));
	reject(root, name, &bad, Some(&tree), Reject {
		stage:	Stage::Decode,
		rule:	rule.to_string(),
		says:	says.to_string(),
		node,
		offset:	None,
		note:	note.to_string(),
	})
}

/// Writes a fixture whose tree is canonical and whose vocabulary is wrong.
///
/// The bytes are canonical, so the fixture isolates the schema: everything up to and including the
/// decoding of the tree succeeds, and validation is what refuses it.
fn schema_reject(
	root:	&Path,
	author:	&SignatureScheme,
	name:	&str,
	node:	u64,
	says:	&str,
	rule:	&str,
	note:	&str,
	tree:	Dat,
)
	-> Outcome<()>
{
	let bytes = res!(canon::encode(&tree));
	let bad = res!(common::assemble(
		&res!(common::seal(&bytes, SCHEMA_DOC, author, TIME)),
		&bytes,
	));
	reject(root, name, &bad, Some(&tree), Reject {
		stage:	Stage::Validate,
		rule:	rule.to_string(),
		says:	says.to_string(),
		node:	Some(node),
		offset:	None,
		note:	note.to_string(),
	})
}

/// Writes a rejection fixture: the bad artefact, the tree behind it where there is one, and the
/// failure the reader must produce.
fn reject(
	root:	&Path,
	name:	&str,
	buf:	&[u8],
	tree:	Option<&Dat>,
	dec:	Reject,
)
	-> Outcome<()>
{
	let dir = res!(fresh_dir(root, name));
	res!(common::write_bytes(&dir.join(DOC_SBJ), buf));
	res!(common::write_bytes(
		&dir.join(REJECT_JDAT),
		res!(common::to_jdat_plain(&dec.to_dat())).as_bytes(),
	));
	// A rejection fixture carries the tree in text form only where the tree region of the artefact
	// is exactly the encoding of that tree. Where the fault is in the bytes rather than in the tree,
	// there is no tree to carry, and the suite requires none.
	if let Some(tree) = tree {
		res!(common::write_bytes(&dir.join(DOC_JDAT), res!(common::to_jdat(tree)).as_bytes()));
	}
	Ok(())
}

/// Empties and returns a fixture's directory, so that a regeneration leaves nothing stale behind.
fn fresh_dir(root: &Path, name: &str) -> Outcome<PathBuf> {
	let dir = root.join(name);
	if dir.exists() {
		res!(fs::remove_dir_all(&dir), IO, File);
	}
	res!(fs::create_dir_all(&dir), IO, File);
	Ok(dir)
}

/// The bytes of a doc whose payload map carries the key `lang` twice.
///
/// A map is a `BTreeMap` once decoded, so a duplicate key exists only on the wire. The bytes are
/// therefore written entry by entry, in the order a `BTreeMap` puts them in, with one entry written
/// twice.
fn duplicate_key_bytes() -> Outcome<Vec<u8>> {

	let kids = Dat::List(vec![
		common::node(NodeKind::Para, common::map(vec![
			("children", Dat::List(vec![common::text("One paragraph.")])),
		])),
	]);

	let mut inner = Vec::new();
	inner = res!(Dat::Str("children".to_string()).to_bytes(inner));
	inner = res!(kids.to_bytes(inner));
	for _ in 0..2 { // The duplicate.
		inner = res!(Dat::Str("lang".to_string()).to_bytes(inner));
		inner = res!(Dat::Str("en".to_string()).to_bytes(inner));
	}
	inner = res!(Dat::Str("title".to_string()).to_bytes(inner));
	inner = res!(Dat::Str("A document".to_string()).to_bytes(inner));

	let mut payload = vec![Dat::MAP_CODE];
	payload = res!(Dat::C64(try_into!(u64, inner.len())).to_bytes(payload));
	payload.extend_from_slice(&inner);

	let mut buf = vec![Dat::USR_CODE];
	buf.extend_from_slice(&NodeKind::Doc.code().to_be_bytes());
	buf.push(Dat::OPT_SOME_CODE);
	buf.extend_from_slice(&payload);
	Ok(buf)
}

/// What `fixtures/README.md` says.
fn readme() -> String {
	fmt!("\
# SBJ conformance fixtures

The teeth of `SPEC.md` §7. Written by `examples/gen_fixtures.rs`, run by `tests/conformance.rs`, and
regenerated rather than patched:

    cargo run -p sbj --example gen_fixtures
    cargo test -p sbj

Each fixture is a directory.

**Acceptance** fixtures carry `doc.jdat`, the document in JDAT text form and the source of truth;
`doc.sbj`, the canonical signed artefact; and `meta.jdat`, what the artefact must turn out to be:
its address, the length of its tree region, its node count and its depth. The suite reads
`doc.jdat`, signs it with the committed key, and requires the bytes it gets back to be `doc.sbj`,
byte for byte.

**Rejection** fixtures carry `doc.sbj`, the bad artefact, and `reject.jdat`, which declares the rule
broken, the step of §2 that must catch it, what the error must say, and the node or the byte it must
name. \"It was rejected\" is not the claim: the claim is that it was rejected for the right reason.
A rejection fixture also carries `doc.jdat` where the tree region is the encoding of a tree that can
be written down; where the fault is in the bytes themselves, there is no tree to write.

Every rejection fixture past the header is correctly hashed and correctly signed, so that the
rejection can only have come from the rule the fixture breaks, and never from a signature that
happened not to check out.

`{}` holds the fixed key every fixture is signed with, and a second key that signs nothing but the
fixture of a signature by the wrong hand. It is committed on purpose: a fixture signed by a fresh
key would be a different file on every run, and a suite that has to be regenerated to pass tests
nothing. It is a test key, published here, and signs nothing else.

Node labels in `doc.jdat` carry an `sbj_` prefix, because two of the v0 kind labels, `box` and
`list`, are JDAT's own kind labels as well: `(box|{{..}})` would read back as a `Dat::Box`. None of
this reaches the wire, where BDAT carries the `u16` kind code and no label at all.

The kind code {} appears in the `unknown_kind` and `unknown_kind_fallback` fixtures. It names no v0
node kind, which is the point of it: the first carries no fallback and is refused, and the second
carries a fallback of known nodes and is accepted (§4.5).

The kind codes {} (`edit`) and {} (`surface`) appear in the three `reserved_*` fixtures. They are
not unknown: §4.2 reserves them to the chrome and to applications, and `oxeweb/doc/0` admits the
kinds 1 to 13 and no others. All three are refused, and the third carries a valid fallback and is
refused anyway, which is the point of it: a fallback admits a code the reader has never heard of,
and never one the reader knows a document may not carry.
", KEY_FILE, ALIEN_CODE, ReservedKind::Edit.code(), ReservedKind::Surface.code())
}
