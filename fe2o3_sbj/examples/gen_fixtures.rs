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
	card::{
		self,
		Card,
		Role,
	},
	doc::{
		self,
		Payload,
	},
	envelope,
	kinds::{
		NodeKind,
		ReservedKind,
	},
	post::{
		self,
		Post,
		Reference,
		Target,
	},
	share::{
		self,
		Share,
	},
	validate,
	SCHEMA_CARD,
	SCHEMA_DOC,
	SCHEMA_POST,
	SCHEMA_SHARE,
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

	let n = res!(acceptance(&root, &keys)) + res!(rejection(&root, &keys))
		+ res!(payloads(&root, &keys)) + res!(shares(&root, &keys));

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
		nodes:	Some(try_into!(u64, stats.nodes)),
		depth:	Some(try_into!(u64, stats.depth)),
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

    cargo run -p oxedyne_fe2o3_sbj --example gen_fixtures
    cargo test -p oxedyne_fe2o3_sbj

Each fixture is a directory.

**Acceptance** fixtures carry `doc.jdat`, the payload in JDAT text form and the source of truth;
`doc.sbj`, the canonical signed artefact; and `meta.jdat`, what the artefact must turn out to be:
its address, the length of its payload region, and -- where the payload is a node tree -- its node
count and its depth. The suite reads `doc.jdat`, signs it with the committed key, and requires the
bytes it gets back to be `doc.sbj`, byte for byte.

**Not every payload is a node tree.** The container carries any schema (§1.2), and the fixtures
named `post_*`, `card_*` and `share_*` carry `daimond/post/0`, `daimond/card/0` and
`daimond/share/0`, which are flat canonical maps rather than trees. Those declare no node count and
no depth, because they have neither, and their `doc.jdat` is written in plain JDAT with none of the
`sbj_` node labels below. Everything else about them is identical: the same header, the same
envelope, the same address, the same signature, and every rule of §3.

The `share_*` fixtures carry one rule the others do not, and it is the reason that schema exists:
`code` is the sender's SIGNED statement about whether the share carries a program, and it is
checked against the files both ways. `share_code_hidden` is a page under a claim of no code, and
`share_code_claimed_without_code` is the opposite. A share is a COPY the receiver comes to own, so
there is no live view, nothing to revoke, and no third party in the middle of it.

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


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE SCHEMAS THAT ARE NOT NODE TREES                                       │
// └───────────────────────────────────────────────────────────────────────────┘
//
// A post and a card are flat canonical maps, so none of §4 applies to them and the fixtures below
// carry no node count and no depth. Everything else is the same file in the same container, which
// is the point: the envelope, the address, the signature and every rule of §3 are the container's
// and do not change with the payload.

/// A recipient's key, fixed so a fixture written twice is written the same.
const TO_KEY: [u8; post::limit::KEY_BYTES] = [0xA1; post::limit::KEY_BYTES];

/// A message's nonce, fixed for the same reason.
const NONCE: [u8; post::limit::NONCE_BYTES] = [0xB2; post::limit::NONCE_BYTES];

/// A sealing subkey, fixed for the same reason.
const ENC_KEY: [u8; card::limit::KEY_BYTES] = [0xE1; card::limit::KEY_BYTES];

/// The smallest message that is still a message.
fn post_minimal() -> Post {
	Post {
		body:	"The crop is in, and the second field can wait.".to_string(),
		to:	TO_KEY.to_vec(),
		nonce:	NONCE.to_vec(),
		reply_to:	None,
		refs:	Vec::new(),
	}
}

/// The fixtures of the post and card schemas, and the count of them.
fn payloads(
	root:	&Path,
	keys:	&Keys,
)
	-> Outcome<usize>
{
	let author = res!(keys.author.signer());

	// -- What a reader must accept. -------------------------------------------------------------

	res!(accept_payload(root, &author, "post_minimal", &Payload::Post(post_minimal()),
		"The smallest message that is still one: a body, a recipient and a nonce. Neither optional \
		field is present, and neither is encoded as `none` — an absent field is omitted (§3 rule \
		4), because a message written two ways would be a message with two addresses."));

	res!(accept_payload(root, &author, "post_every_target", &Payload::Post(Post {
		body:	"All four of the things a message may point at, once each.".to_string(),
		to:	TO_KEY.to_vec(),
		nonce:	NONCE.to_vec(),
		reply_to:	Some(vec![0xC3; post::limit::ADDR_BYTES]),
		refs:	vec![
			Reference {
				target:	Target::Proposal {
					account:	"oxedyne".to_string(),
					repo:	"daimond".to_string(),
					number:	17,
				},
				fallback:	"the proposal about the panel showing nothing when signed out"
					.to_string(),
			},
			Reference {
				target:	Target::Build { id: "f9f68b75c73b".to_string() },
				fallback:	"the build this was fixed in".to_string(),
			},
			Reference {
				target:	Target::Panel { name: "spend".to_string() },
				fallback:	"the Spending panel".to_string(),
			},
			Reference {
				target:	Target::Guide {
					page:	"improve".to_string(),
					anchor:	Some("voices".to_string()),
				},
				fallback:	"the guide section on voices".to_string(),
			},
		],
	}),
		"Every kind of reference once, at the limit of four, and a reply. All four referents are \
		PUBLIC anchors: each is named globally and can be resolved by anybody holding a session. A \
		reference to something only the sender can reach would draw a pressable chip that always \
		fails, which is why no such kind exists."));

	res!(accept_payload(root, &author, "card_first", &Payload::Card(Card {
		label:	"Jason".to_string(),
		enc:	ENC_KEY.to_vec(),
		role:	Role::Root,
		prev:	None,
	}),
		"A first identity card: a display label, the sealing subkey, and the role. The SIGNING key \
		is not a field here — it is the envelope's `author`, so a card has exactly one place that \
		says which key composed it. Self-signed, which proves the holder of that key composed it \
		and proves nothing whatever about who the holder is."));

	res!(accept_payload(root, &author, "card_rotated", &Payload::Card(Card {
		label:	"Jason".to_string(),
		enc:	ENC_KEY.to_vec(),
		role:	Role::Root,
		prev:	Some(vec![0xD4; card::limit::KEY_BYTES]),
	}),
		"A card naming the key it supersedes. It must not encode as `card_first` does: a rotated \
		key and a first key are different facts, and a reader that could not tell them apart could \
		not tell a replacement from a stranger."));

	// -- What a reader must refuse. -------------------------------------------------------------

	// The re-labelling attack, which is the whole reason §1.3 length-prefixes the schema. The bytes,
	// the hash and the signature are untouched; one word of the envelope is changed. Both payloads
	// are flat maps, so the card's decoder would happily be handed the post's bytes -- the envelope
	// is the only thing that says which this is, and the envelope is signed.
	let bytes = res!(post_minimal().encode());
	let mut env = res!(common::seal(&bytes, SCHEMA_POST, &author, TIME));
	env.schema = SCHEMA_CARD.to_string();
	res!(reject(root, "post_relabelled_as_card", &res!(common::assemble(&env, &bytes)), None,
		Reject {
			stage:	Stage::Sig,
			rule:	"SPEC.md §1.3: the schema is inside the signing input, and is preceded by its \
				length.".to_string(),
			says:	"not a signature by the author".to_string(),
			node:	None,
			offset:	None,
			note:	"A signed post whose envelope has been re-labelled `daimond/card/0` after \
				signing. Everything else is untouched: the payload bytes, the hash of them, and \
				the signature over that hash. The signature covers the schema as well as the \
				address, so the re-labelling is what breaks it. Without that, a payload could be \
				presented to whichever validator would accept it, and an author's signature would \
				vouch for a claim they never made.".to_string(),
		}));

	// A body one byte past the limit. A rejection and never a truncation: a message silently cut
	// short is a message whose sender and reader disagree about what was said.
	let mut long = post_minimal();
	long.body = "x".repeat(post::limit::BODY_BYTES + 1);
	res!(payload_reject(root, &author, "post_body_over_limit", SCHEMA_POST,
		&res!(long.to_dat()),
		"exceeding the limit",
		"SPEC.md §5 and the post schema's own limits: a body is at most 8 KiB of UTF-8."
			.to_string(),
		"A body one byte past the ceiling. The number is revisable on evidence; that there is one \
		is not, since a body with no ceiling is a body that sets the relay's storage.".to_string()));

	// A nonce of the wrong width. Not a shorter nonce: a different thing.
	let short_nonce = {
		let mut m = DaticleMap::new();
		m.insert(dat!(post::KEY_BODY), Dat::BU32(post_minimal().body.into_bytes()));
		m.insert(dat!(post::KEY_NONCE), Dat::BU8(vec![0xB2; post::limit::NONCE_BYTES - 1]));
		m.insert(dat!(post::KEY_TO), Dat::BU8(TO_KEY.to_vec()));
		Dat::Map(m)
	};
	res!(payload_reject(root, &author, "post_nonce_width", SCHEMA_POST, &short_nonce,
		"must carry exactly",
		"The post schema fixes the nonce at 16 bytes.".to_string(),
		"A nonce of fifteen bytes. A key or a nonce of the wrong width is not a shorter one; it \
		is a different thing, and admitting it would let a sender choose how much randomness a \
		message carried.".to_string()));

	// A sealing subkey of the wrong width, for the same reason on the card's side.
	let short_enc = {
		let mut m = DaticleMap::new();
		m.insert(dat!(card::KEY_ENC), Dat::BU8(vec![0xE1; card::limit::KEY_BYTES - 1]));
		m.insert(dat!(card::KEY_LABEL), Dat::Str("Jason".to_string()));
		m.insert(dat!(card::KEY_ROLE), Dat::Str(Role::Root.as_str().to_string()));
		Dat::Map(m)
	};
	res!(payload_reject(root, &author, "card_enc_width", SCHEMA_CARD, &short_enc,
		"must carry exactly",
		"The card schema fixes the sealing subkey at 32 bytes.".to_string(),
		"A sealing subkey of thirty-one bytes. A card is what a correspondent reads a sealing key \
		OFF, so a key of the wrong width here is a key nothing can seal to.".to_string()));

	// A list that is present and empty. Two encodings of one message, and so two addresses.
	let empty_refs = {
		let mut m = DaticleMap::new();
		m.insert(dat!(post::KEY_BODY), Dat::BU32(post_minimal().body.into_bytes()));
		m.insert(dat!(post::KEY_NONCE), Dat::BU8(NONCE.to_vec()));
		m.insert(dat!(post::KEY_REFS), Dat::List(Vec::new()));
		m.insert(dat!(post::KEY_TO), Dat::BU8(TO_KEY.to_vec()));
		Dat::Map(m)
	};
	res!(payload_reject(root, &author, "post_refs_empty_list", SCHEMA_POST, &empty_refs,
		"carries an empty \"refs\" list",
		"SPEC.md §3 rules 4 and 8: an absent optional field is omitted, never encoded as an empty \
		one.".to_string(),
		"A message carrying `refs` as an empty list. A message with no references and a message \
		with an empty list of them are the same message, so admitting both would give it two \
		encodings and therefore two addresses.".to_string()));

	// A duplicate key, which survives only in the bytes: a decoding map collapses it into one entry,
	// so nothing but re-encoding and comparing can catch it.
	res!(reject(root, "post_duplicate_key",
		&res!(payload_file(&author, SCHEMA_POST, &res!(post_duplicate_key_bytes()))), None,
		Reject {
			stage:	Stage::Decode,
			rule:	"SPEC.md §3: a map carries each key once. A duplicate survives only in the \
				bytes.".to_string(),
			says:	"not in canonical form".to_string(),
			node:	None,
			offset:	None,
			note:	"A post whose map carries `to` twice on the wire. Both entries decode, and the \
				second overwrites the first, so the decoded value is indistinguishable from a \
				sound one: the fault exists in the bytes alone. It is caught by re-encoding what \
				was decoded and requiring the same bytes back, which is why that comparison is \
				not an optimisation to skip.".to_string(),
		}));

	// A non-minimal length in the ENVELOPE, over a post. The envelope obeys §3 like everything else
	// the hash is read from, and `tree_len` is the one field written as a variable-width c64.
	res!(reject(root, "envelope_nonminimal_c64",
		&res!(nonminimal_tree_len(&author)), None,
		Reject {
			stage:	Stage::Envelope,
			rule:	"SPEC.md §1.2 and §3: the envelope is canonical, so a length is written in as \
				few bytes as it needs.".to_string(),
			says:	"minimally encoded".to_string(),
			node:	None,
			offset:	None,
			note:	"An envelope whose `tree_len` is written as a wider c64 than the value needs. \
				It decodes to the same number, so nothing about where the payload is or what it \
				says would change; only the bytes differ. Admitted, it would give one artefact \
				more than one envelope encoding, and an envelope is what a reader identifies an \
				artefact from. Caught by the BDAT decoder as it reads the length, which is earlier \
				and more precise than the envelope's own re-encode comparison -- that comparison \
				is the backstop for the faults a decode survives, such as a duplicate key, and \
				this is not one of them.".to_string(),
		}));

	Ok(11)
}

/// Writes an acceptance fixture for a payload that is not a node tree.
fn accept_payload(
	root:	&Path,
	author:	&SignatureScheme,
	name:	&str,
	payload:	&Payload,
	note:	&str,
)
	-> Outcome<()>
{
	let dir = res!(fresh_dir(root, name));
	let buf = res!(doc::write_artefact(payload, author, TIME));
	let env = res!(doc::verify_only(&buf));
	let meta = Meta {
		schema:	env.schema.clone(),
		time:	env.time,
		hash:	env.hash.clone(),
		tree_len:	env.tree_len,
		// A flat record has no nodes and no depth, so it declares neither.
		nodes:	None,
		depth:	None,
		index:	false,
		note:	note.to_string(),
	};
	let as_dat = match payload {
		Payload::Post(p)	=> res!(p.to_dat()),
		Payload::Card(c)	=> res!(c.to_dat()),
		Payload::Share(s)	=> res!(s.to_dat()),
		Payload::Tree { .. }	=> return Err(err!(
			"The fixture '{}' is a node tree, which `accept` writes and this does not.", name;
		Bug, Invalid)),
	};
	res!(common::write_bytes(&dir.join(DOC_JDAT), res!(common::to_jdat_plain(&as_dat)).as_bytes()));
	res!(common::write_bytes(&dir.join(DOC_SBJ), &buf));
	res!(common::write_bytes(
		&dir.join(META_JDAT),
		res!(common::to_jdat_plain(&meta.to_dat())).as_bytes(),
	));
	Ok(())
}

/// Writes a rejection fixture whose payload is a canonical map breaking one of its schema's rules.
///
/// The map is encoded without being checked, so the fixture isolates the rule: the container is
/// sound, the bytes hash to what the envelope says, the signature verifies, and the payload's own
/// decoder is what refuses it.
fn payload_reject(
	root:	&Path,
	author:	&SignatureScheme,
	name:	&str,
	schema:	&str,
	payload:	&Dat,
	says:	&str,
	rule:	String,
	note:	String,
)
	-> Outcome<()>
{
	let bytes = res!(payload.to_bytes(Vec::new()));
	let bad = res!(payload_file(author, schema, &bytes));
	let dir = res!(fresh_dir(root, name));
	res!(common::write_bytes(&dir.join(DOC_SBJ), &bad));
	res!(common::write_bytes(
		&dir.join(REJECT_JDAT),
		res!(common::to_jdat_plain(&Reject {
			stage:	Stage::Decode,
			rule,
			says:	says.to_string(),
			node:	None,
			offset:	None,
			note,
		}.to_dat())).as_bytes(),
	));
	// Written plain, with none of the `sbj_` node labels: there are no `usr` daticles in a record.
	res!(common::write_bytes(&dir.join(DOC_JDAT), res!(common::to_jdat_plain(payload)).as_bytes()));
	Ok(())
}

/// A whole file around payload bytes, correctly hashed and correctly signed.
fn payload_file(
	author:	&SignatureScheme,
	schema:	&str,
	bytes:	&[u8],
)
	-> Outcome<Vec<u8>>
{
	common::assemble(&res!(common::seal(bytes, schema, author, TIME)), bytes)
}

/// The bytes of a post whose map carries the key `to` twice.
///
/// A map is a `BTreeMap` once decoded, so a duplicate key exists only on the wire. The bytes are
/// therefore written entry by entry, in the order a `BTreeMap` puts them in, with one written twice.
fn post_duplicate_key_bytes() -> Outcome<Vec<u8>> {
	let p = post_minimal();
	let mut inner = Vec::new();
	inner = res!(Dat::Str(post::KEY_BODY.to_string()).to_bytes(inner));
	inner = res!(Dat::BU32(p.body.into_bytes()).to_bytes(inner));
	inner = res!(Dat::Str(post::KEY_NONCE.to_string()).to_bytes(inner));
	inner = res!(Dat::BU8(NONCE.to_vec()).to_bytes(inner));
	for _ in 0..2 { // The duplicate.
		inner = res!(Dat::Str(post::KEY_TO.to_string()).to_bytes(inner));
		inner = res!(Dat::BU8(TO_KEY.to_vec()).to_bytes(inner));
	}
	map_bytes(&inner)
}

/// A sound post in a file whose envelope writes `tree_len` in more bytes than it needs.
fn nonminimal_tree_len(author: &SignatureScheme) -> Outcome<Vec<u8>> {
	let bytes = res!(post_minimal().encode());
	let env = res!(common::seal(&bytes, SCHEMA_POST, author, TIME));
	let env_dat = res!(env.to_dat());
	let map = match &env_dat {
		Dat::Map(m) => m,
		other => return Err(err!(
			"An envelope encodes as a map, and this is a {:?}.", other.kind(); Bug, Invalid)),
	};
	// Every entry as the encoder writes it, except `tree_len`, which is written at a wider c64.
	let mut inner = Vec::new();
	for (k, v) in map.iter() {
		inner = res!(k.to_bytes(inner));
		if *k == dat!(envelope::KEY_TREE_LEN) {
			inner.extend_from_slice(&wide_c64(env.tree_len));
		} else {
			inner = res!(v.to_bytes(inner));
		}
	}
	let env_bytes = res!(map_bytes(&inner));
	let mut buf = res!(envelope::write_header(env_bytes.len()));
	buf.extend_from_slice(&env_bytes);
	buf.extend_from_slice(&bytes);
	Ok(buf)
}

/// A BDAT map around already-encoded entries: the map code, the byte length, then the entries.
///
/// Written by hand because every fixture that reaches for it is a fixture whose entries a `Dat`
/// cannot hold -- a duplicate key, or a value written at a width the encoder would never choose.
fn map_bytes(inner: &[u8]) -> Outcome<Vec<u8>> {
	let mut buf = vec![Dat::MAP_CODE];
	buf = res!(Dat::C64(try_into!(u64, inner.len())).to_bytes(buf));
	buf.extend_from_slice(inner);
	Ok(buf)
}

/// A `c64` written in one more byte than the value needs.
///
/// A `c64` is a code byte carrying the number of value bytes that follow, so the same number has as
/// many encodings as there are widths that hold it. Canonical form is the narrowest (§3); this is
/// the next one up, which decodes to exactly the same number and differs only in the bytes.
fn wide_c64(v: u64) -> Vec<u8> {
	let be = v.to_be_bytes();
	// The minimal width, then one more. A zero needs no value bytes, so the wide form of it is one.
	let narrow = be.iter().position(|b| *b != 0).map_or(0, |i| 8 - i);
	// `min(8)` rather than a check: eight is the widest a c64 has, so a value already at it has no
	// wider form and is written as it stands. No caller reaches that, since `tree_len` is capped at
	// four mebibytes by §5.
	let width = (narrow + 1).min(8);
	let mut out = vec![Dat::C64_CODE_START + width as u8];
	out.extend_from_slice(&be[8 - width..]);
	out
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE SHARE SCHEMA                                                          │
// └───────────────────────────────────────────────────────────────────────────┘
//
// `daimond/share/0` is one person sending another a COPY of something they own, and it is the
// third flat record in this container. Its fixtures are here rather than among the post's because
// what they are teeth for is different: a post's rules are about one message having one address,
// and a share's are about that plus a consent bit, which is the one field in this format whose
// whole purpose is that a receiver can check what the SENDER marked before anything runs.

/// A capp's page, standing in for the real thing: what makes a share carry code is the NAME.
const PAGE: &'static [u8] = b"<html><body><p>A page somebody else wrote.</p></body></html>";

/// A share of data alone, fixed so a fixture written twice is written the same.
fn share_data() -> Share {
	Share::new(
		"Sourdough".to_string(),
		TO_KEY.to_vec(),
		NONCE.to_vec(),
		None,
		vec![
			share::File {
				path:	"bakes/2026.jsonl".to_string(),
				body:	b"{\"day\":1,\"loaves\":2}\n".to_vec(),
			},
			share::File {
				path:	"crystal.json".to_string(),
				body:	b"{\"starter\":\"fed Tuesday\"}".to_vec(),
			},
		],
	)
}

/// The same share, carrying a page, and therefore carrying a program.
fn share_capp() -> Share {
	let mut files = share_data().files;
	files.push(share::File { path: "crystal.html".to_string(), body: PAGE.to_vec() });
	Share::new(
		"Life log".to_string(),
		TO_KEY.to_vec(),
		NONCE.to_vec(),
		Some("The food log we talked about.".to_string()),
		files,
	)
}

/// The share fixtures, and the count of them.
fn shares(
	root:	&Path,
	keys:	&Keys,
)
	-> Outcome<usize>
{
	let author = res!(keys.author.signer());

	// -- What a reader must accept. -------------------------------------------------------------

	res!(accept_payload(root, &author, "share_data", &Payload::Share(share_data()),
		"A share of DATA alone: two files, a display name, a recipient and a nonce, and `code` \
		written as false. The bit is present even though nothing here is code — an omitted false \
		and a sender whose build had never heard of the field are the same bytes, and those are \
		the two things a receiver must be able to tell apart. No note, and the absent one is \
		omitted rather than written empty (§3 rules 4 and 8). The files are in path order, which \
		is fixed, because a set of files written two ways would be one Diamond at two addresses."));

	res!(accept_payload(root, &author, "share_capp", &Payload::Share(share_capp()),
		"A share carrying `crystal.html`, and therefore carrying a PROGRAM written by another \
		person. `code` is true, it is inside the payload, and the payload is what the envelope's \
		hash covers and the signature commits to — so a receiver can check that the SENDER marked \
		it, which is the whole point: a flag a relay could add or strip is not a consent flag. It \
		must not encode as `share_data` does, and it carries a covering note besides."));

	// -- What a reader must refuse. -------------------------------------------------------------

	// The central rejection of this schema. Everything else here is a canonicalisation rule; this
	// one is what the consent bit is for.
	let mut hidden = share_capp();
	hidden.code = false;
	res!(payload_reject(root, &author, "share_code_hidden", SCHEMA_SHARE,
		&res!(hidden.to_dat()),
		"crystal.html",
		"The share schema: `code` is the sender's signed claim, and it is checked against the \
		files.".to_string(),
		"A share carrying `crystal.html` under `code: false`. It is signed, correctly hashed and \
		correctly addressed, so nothing in the container catches it: the payload's own decoder \
		does, naming the file. Without that check the bit would be decoration — a sender could \
		ship a page as data and the receiving client, believing the claim, would mount somebody \
		else's program without asking. The refusal is what makes the claim worth reading."
			.to_string()));

	// And the other direction, which is not symmetry for its own sake.
	let mut crying = share_data();
	crying.code = true;
	res!(payload_reject(root, &author, "share_code_claimed_without_code", SCHEMA_SHARE,
		&res!(crying.to_dat()),
		"carries none",
		"The share schema: `code` is checked against the files BOTH ways.".to_string(),
		"A share claiming code and carrying none. Refused rather than waved through as harmless \
		caution: a receiver asked to consent to a program that is not there is a receiver being \
		taught that the question does not mean anything, and the next time it is asked in earnest \
		they will answer the same way.".to_string()));

	// The bit is REQUIRED. An absent one would be read as false by any reader generous enough to
	// default it, which is exactly the generosity a consent flag cannot afford.
	let no_bit = {
		let mut m = match res!(share_data().to_dat()) {
			Dat::Map(m) => m,
			other => return Err(err!(
				"A share encodes as a map, and this is a {:?}.", other.kind(); Bug, Invalid)),
		};
		m.remove(&dat!(share::KEY_CODE));
		Dat::Map(m)
	};
	res!(payload_reject(root, &author, "share_missing_code_bit", SCHEMA_SHARE, &no_bit,
		"missing the required key",
		"The share schema: `code` is required, and is written even when it is false.".to_string(),
		"A share with no `code` key at all. A reader that defaulted it to false would be reading \
		\"they did not say\" as \"they said there is nothing to worry about\", which is the one \
		reading a consent bit must never be given.".to_string()));

	// One Diamond, two addresses: the rule a message's `refs` deliberately does not have, because
	// references are ordered by their author's meaning and a set of files is not.
	let unsorted = {
		let s = share_data();
		let mut m = match res!(s.to_dat()) {
			Dat::Map(m) => m,
			other => return Err(err!(
				"A share encodes as a map, and this is a {:?}.", other.kind(); Bug, Invalid)),
		};
		let mut list = Vec::new();
		for f in s.files.iter().rev() {
			list.push(res!(f.to_dat()));
		}
		m.insert(dat!(share::KEY_FILES), Dat::List(list));
		Dat::Map(m)
	};
	res!(payload_reject(root, &author, "share_files_out_of_order", SCHEMA_SHARE, &unsorted,
		"not in path order",
		"The share schema: the files are ordered by path, so that one set of files has one \
		encoding.".to_string(),
		"The same two files, listed the other way round. They decode to the same Diamond and hash \
		to a different address, so admitting both would give one share two addresses. It is \
		refused rather than sorted: sorting it would be accepting a second encoding and quietly \
		rewriting it, which is what §3 exists to stop.".to_string()));

	// The three paths a share may not carry, one fixture each for the two that are about somebody
	// else's records.
	res!(payload_reject(root, &author, "share_carries_the_log", SCHEMA_SHARE,
		&res!(share_with_path(".daimond/log.jsonl")),
		".daimond/",
		"The share schema: a share may not carry the sender's own `.daimond/` record."
			.to_string(),
		"A share carrying the sender's append-only log — the record of what agents did in THEIR \
		copy. Refused in the format rather than in a client, so that every implementation refuses \
		it: a person sending a recipe does not think to check what travels with it, and the \
		receiver's copy is new, so its record starts empty because nothing has happened in it yet."
			.to_string()));

	res!(payload_reject(root, &author, "share_carries_capp_record", SCHEMA_SHARE,
		&res!(share_with_path("capp.json")),
		"capp.json",
		"The share schema: a share may not carry a capp delivery record.".to_string(),
		"A share carrying `capp.json`, which says which bytes were delivered to that instance and \
		at what template version, and decides which files a future fix may replace. The receiver \
		was never delivered to; they were handed a copy by a person. One carried across from \
		somebody else's machine would pin their copy against updates they never chose, and a \
		doctored one would do it on purpose. A copy with no record is a case the receiving client \
		already knows: it asks.".to_string()));

	res!(payload_reject(root, &author, "share_path_walks", SCHEMA_SHARE,
		&res!(share_with_path("../../notes/private.md")),
		"segment",
		"The share schema: a path is refused rather than resolved, and never walks."
			.to_string(),
		"A share whose file path climbs out of the Diamond. Refused rather than normalised, which \
		is where this parts company with the client-side path guard it otherwise matches: that one \
		is handed an untrusted request and tidies it on the way to a real file, and this is \
		deciding what a SIGNED artefact means, where a path that needed tidying is a path with two \
		spellings.".to_string()));

	// The re-labelling attack over the third schema. The reserved name became a real one, and this
	// is the fixture that shows nothing already signed was weakened by it.
	let bytes = res!(share_data().encode());
	let mut env = res!(common::seal(&bytes, SCHEMA_SHARE, &author, TIME));
	env.schema = SCHEMA_POST.to_string();
	res!(reject(root, "share_relabelled_as_post", &res!(common::assemble(&env, &bytes)), None,
		Reject {
			stage:	Stage::Sig,
			rule:	"SPEC.md §1.3: the schema is inside the signing input, and is preceded by its \
				length.".to_string(),
			says:	"not a signature by the author".to_string(),
			node:	None,
			offset:	None,
			note:	"A signed share whose envelope has been re-labelled `daimond/post/0` after \
				signing. The payload bytes, their hash and the signature over that hash are \
				untouched. This is the same claim as `post_relabelled_as_card`, made over the \
				schema name that was RESERVED when those were signed: the schema reaches the \
				signing input length-prefixed, so a third name coming to exist re-addressed \
				nothing, weakened nothing, and left every fixture already committed byte for byte \
				the file it was.".to_string(),
		}));

	Ok(10)
}

/// A share whose one file sits at `path`, for the fixtures about paths a share may not carry.
///
/// Built as a daticle rather than through `Share::new`, because the point of each is a path the
/// constructor's own validator would refuse, and a fixture that could not be written would prove
/// nothing about what a reader does with one that was.
fn share_with_path(path: &str) -> Outcome<Dat> {
	let mut file = DaticleMap::new();
	file.insert(dat!(share::KEY_BODY),	Dat::BU32(b"whatever is in it".to_vec()));
	file.insert(dat!(share::KEY_PATH),	Dat::Str(path.to_string()));

	let mut m = DaticleMap::new();
	m.insert(dat!(share::KEY_CODE),	Dat::Bool(false));
	m.insert(dat!(share::KEY_FILES),	Dat::List(vec![Dat::Map(file)]));
	m.insert(dat!(share::KEY_NAME),	Dat::Str("A share reaching too far".to_string()));
	m.insert(dat!(share::KEY_NONCE),	Dat::BU8(NONCE.to_vec()));
	m.insert(dat!(share::KEY_TO),	Dat::BU8(TO_KEY.to_vec()));
	Ok(Dat::Map(m))
}
