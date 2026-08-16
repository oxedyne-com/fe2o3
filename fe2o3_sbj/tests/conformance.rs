//! The conformance suite of `SPEC.md` §7.
//!
//! The fixtures are the specification's teeth. They are what stops the binary quietly becoming the
//! definition of the format: every acceptance fixture is rebuilt here from its JDAT text and the
//! committed key, and the bytes that come out must be the bytes that were committed, so a change to
//! the encoder that nobody meant shows up as a different file rather than as nothing at all.
//!
//! Every rejection fixture declares what must go wrong with it, and where: the rule broken, the step
//! of §2 that must catch it, what the error must say, and the node or the byte it must name. A
//! reader that refused everything would pass a suite that only checked for an `Err`, so this checks
//! the reason. It also checks the ordering that §2 exists for: a fixture refused at step 6 or 7 must
//! pass steps 1 to 5 first, and a fixture refused before that must never reach a decoder at all.
//!
//! A fixture directory the suite does not know how to run is a failure rather than a skip, so a
//! fixture cannot be added and silently ignored, and the fixtures §7 requires by name are required
//! here by name, so one cannot be deleted and silently missed.

mod common;

use common::{
	Keys,
	Meta,
	Reject,
	DOC_JDAT,
	DOC_SBJ,
	KEY_FILE,
	META_JDAT,
	README_FILE,
	REJECT_JDAT,
};

use oxedyne_fe2o3_sbj::{
	card::Card,
	doc::{
		self,
		Payload,
	},
	envelope,
	index,
	post::Post,
	validate,
	HEADER_LEN,
	SCHEMA_CARD,
	SCHEMA_POST,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::sign::SignatureScheme;
use oxedyne_fe2o3_jdat::prelude::*;

use std::{
	collections::BTreeSet,
	fs,
	path::Path,
};

/// The fixtures `SPEC.md` §7 requires by name.
///
/// A fixture that is missing is a failure, so that the suite cannot be quietly hollowed out by
/// deleting the ones that fail.
const REQUIRED: [&'static str; 30] = [
	// The list at the end of §7, in its order.
	"empty",	// An empty document.
	"one_para",	// One paragraph.
	"every_kind",	// Every node kind once.
	"depth_at_limit",	// Nesting at the depth limit,
	"depth_over_limit",	// and one past it.
	"size_at_limit",	// A tree at the size limit,
	"size_over_limit",	// and one past it.
	"canon_rule1_undeclared_field",	// Each canonicalisation rule of §3,
	"canon_rule2_ordmap",	// violated
	"canon_rule3_uppercase_key",	// exactly
	"canon_rule3_duplicate_key",	// once.
	"canon_rule4_empty_children",
	"canon_rule4_opt_none",
	"canon_rule5_control_char",
	"canon_rule6_int_width",
	"canon_rule7_vek_children",
	"truncated_tree",	// A truncated tree.
	"tree_longer_than_tree_len",	// A tree one byte longer than tree_len.
	"corrupt_tree_byte",	// A corrupted hash, from the tree's side,
	"bad_hash",	// and from the envelope's.
	"bad_sig",	// A corrupted signature.
	"wrong_key",	// A signature by the wrong key.
	"unknown_kind",	// An unknown node kind.
	"reserved_edit_in_doc",	// A document carrying an edit node,
	"reserved_surface_in_doc",	// one carrying a surface node,
	"reserved_surface_with_fallback_still_refused",	// and one whose surface carries a fallback.
	"para_in_para",	// A forbidden child.
	"heading_level_0",	// A heading with level 0,
	"heading_level_7",	// and one with level 7.
	"bdat_not_sbj",	// Valid BDAT, and not valid SBJ.
];

#[test]
fn test_conformance_suite() -> Outcome<()> {
	// A node costs three daticle levels, so the deepest legal document nests daticles 770 deep, and
	// a recursive decoder spends a frame on each. The limit is the format's, and it does not move to
	// suit a test, so the test moves instead.
	let thread = match std::thread::Builder::new()
		.name("sbj_conformance".to_string())
		.stack_size(common::STACK_BYTES)
		.spawn(suite)
	{
		Ok(thread) => thread,
		Err(e) => return Err(err!(e,
			"Could not spawn the thread the deepest fixture is read on.";
		Test, Init)),
	};
	match thread.join() {
		Ok(outcome) => outcome,
		Err(_) => Err(err!(
			"The thread running the conformance suite did not return.";
		Test, Panic)),
	}
}

/// Walks the fixture directory and runs everything in it.
fn suite() -> Outcome<()> {

	let root = common::fixtures_dir();
	if !root.is_dir() {
		return Err(err!(
			"There is no fixture directory at {}. The fixtures are written by \
			`cargo run -p sbj --example gen_fixtures`.", root.display();
		Test, Missing));
	}
	let keys = res!(Keys::load(&root));

	let mut names: BTreeSet<String> = BTreeSet::new();
	for entry in res!(fs::read_dir(&root), IO, File) {
		let entry = res!(entry, IO, File);
		let path = entry.path();
		let name = match path.file_name().and_then(|s| s.to_str()) {
			Some(name) => name.to_string(),
			None => return Err(err!(
				"The fixture directory holds {}, whose name is not UTF-8.", path.display();
			Test, Invalid)),
		};
		if path.is_file() {
			// The only files beside the fixtures are the key they are signed with and the note
			// saying what they are. Anything else is a fixture nobody runs.
			if name != KEY_FILE && name != README_FILE {
				return Err(err!(
					"The fixture directory holds the file '{}', which is neither the key '{}' nor \
					the note '{}'. A file the suite does not know what to do with is a failure, \
					not a skip.", name, KEY_FILE, README_FILE;
				Test, Invalid, Unexpected));
			}
			continue;
		}
		if !path.is_dir() {
			return Err(err!(
				"The fixture directory holds '{}', which is neither a file nor a directory.", name;
			Test, Invalid, Unexpected));
		}
		res!(fixture(&path, &name, &keys));
		names.insert(name);
	}

	if names.is_empty() {
		return Err(err!(
			"The fixture directory {} holds no fixtures.", root.display();
		Test, Missing));
	}
	for req in REQUIRED {
		if !names.contains(req) {
			return Err(err!(
				"The fixture '{}', which SPEC.md §7 requires, is not in {}.", req, root.display();
			Test, Missing));
		}
	}
	Ok(())
}

/// Runs one fixture, refusing to skip one it does not understand.
fn fixture(
	dir:	&Path,
	name:	&str,
	keys:	&Keys,
)
	-> Outcome<()>
{
	// Every file of a fixture is one of four, so that a fixture cannot carry something the suite
	// silently ignores.
	for entry in res!(fs::read_dir(dir), IO, File) {
		let entry = res!(entry, IO, File);
		let file = match entry.file_name().to_str() {
			Some(file) => file.to_string(),
			None => return Err(err!(
				"The fixture '{}' holds a file whose name is not UTF-8.", name;
			Test, Invalid)),
		};
		match file.as_str() {
			DOC_JDAT | DOC_SBJ | META_JDAT | REJECT_JDAT => (),
			_ => return Err(err!(
				"The fixture '{}' holds the file '{}'. A fixture carries '{}', and then either \
				'{}' or '{}'.", name, file, DOC_SBJ, META_JDAT, REJECT_JDAT;
			Test, Invalid, Unexpected)),
		}
	}

	let has_meta = dir.join(META_JDAT).is_file();
	let has_reject = dir.join(REJECT_JDAT).is_file();
	if !dir.join(DOC_SBJ).is_file() {
		return Err(err!(
			"The fixture '{}' carries no '{}'. Every fixture is an artefact, whether it is one a \
			reader must accept or one it must refuse.", name, DOC_SBJ;
		Test, Missing));
	}
	match (has_meta, has_reject) {
		(true, false)	=> accept(dir, name, keys),
		(false, true)	=> reject(dir, name),
		(true, true)	=> Err(err!(
			"The fixture '{}' carries both '{}' and '{}'. A document is either accepted or \
			refused.", name, META_JDAT, REJECT_JDAT;
		Test, Invalid, Conflict)),
		(false, false)	=> Err(err!(
			"The suite does not know how to run the fixture '{}': it carries neither '{}' nor \
			'{}'. A fixture the suite cannot run is a failure, not a skip, so that a fixture \
			cannot be added and quietly ignored.", name, META_JDAT, REJECT_JDAT;
		Test, Invalid, Unknown)),
	}
}

/// Runs an acceptance fixture: the artefact reads, and it is what `meta.jdat` says it is.
///
/// The artefact is then rebuilt from `doc.jdat` and the committed key, and must come out byte for
/// byte the file that was committed. That is what makes `doc.jdat` the source of truth rather than a
/// comment: a change to the encoder that nobody meant shows up here as a different file.
fn accept(
	dir:	&Path,
	name:	&str,
	keys:	&Keys,
)
	-> Outcome<()>
{
	let buf = res!(common::read_bytes(&dir.join(DOC_SBJ)));
	let meta = res!(Meta::from_dat(&res!(common::from_jdat_plain(
		&res!(common::read_text(&dir.join(META_JDAT)))
	))));

	// Steps 1 to 5 touch no content, and a caller may run them and stop.
	let env = match doc::verify_only(&buf) {
		Ok(env) => env,
		Err(e) => return Err(err!(e,
			"The fixture '{}' does not verify.", name;
		Test, Invalid)),
	};
	let art = match doc::read_artefact(&buf) {
		Ok(art) => art,
		Err(e) => return Err(err!(e,
			"The fixture '{}' does not read.", name;
		Test, Invalid)),
	};
	res!(req(name, "the envelope of verify_only is the envelope of read", &env, art.env()));

	// The envelope says what `meta.jdat` says it says.
	res!(req(name, "schema", &art.env().schema, &meta.schema));
	res!(req(name, "time", &art.env().time, &meta.time));
	res!(req(name, "hash", &art.env().hash, &meta.hash));
	res!(req(name, "tree_len", &art.env().tree_len, &meta.tree_len));
	res!(req(name, "author", &art.env().author, &keys.author.pk));

	// The hash in the envelope is the hash of the region, and the region is the length declared.
	let (_, region) = res!(doc::verify(&buf));
	res!(req(name, "tree region length", &try_into!(u64, region.len()), &meta.tree_len));
	let hash = res!(doc::hash_tree(art.env().hash_scheme, region));
	res!(req(name, "the hash of the payload region", &hash, &meta.hash));

	// `doc.jdat` is the payload, and the payload is `doc.sbj`. What the payload IS decides how it
	// is read back and how it is rebuilt: a node tree carries `usr` nodes and is written by
	// `doc::write`, while a post and a card are flat maps with no node labels in them at all.
	let signer = res!(keys.author.signer());
	let rebuilt = match art.payload() {
		Payload::Tree { tree, .. } => {
			// The tree is the shape `meta.jdat` says it is.
			let stats = res!(validate::validate(tree, &art.env().schema));
			res!(req(name, "node count", &Some(try_into!(u64, stats.nodes)), &meta.nodes));
			res!(req(name, "depth", &Some(try_into!(u64, stats.depth)), &meta.depth));
			let from_text = res!(read_tree(&dir.join(DOC_JDAT), name));
			res!(req(name, "the tree of doc.jdat against the tree of doc.sbj", &from_text, tree));
			res!(rewrite(&from_text, &meta, &signer))
		},
		Payload::Post(post) => {
			res!(req(name, "node count", &None, &meta.nodes));
			let from_text = res!(read_payload(&dir.join(DOC_JDAT), name));
			let back = res!(Post::from_dat(&from_text));
			res!(req(name, "the post of doc.jdat against the post of doc.sbj", &back, post));
			res!(doc::write_artefact(&Payload::Post(back), &signer, meta.time))
		},
		Payload::Card(card) => {
			res!(req(name, "node count", &None, &meta.nodes));
			let from_text = res!(read_payload(&dir.join(DOC_JDAT), name));
			let back = res!(Card::from_dat(&from_text));
			res!(req(name, "the card of doc.jdat against the card of doc.sbj", &back, card));
			res!(doc::write_artefact(&Payload::Card(back), &signer, meta.time))
		},
	};
	if rebuilt != buf {
		return Err(err!(
			"The fixture '{}' does not rebuild: writing the document of '{}' with the committed \
			key gives {} bytes, and the committed '{}' is {} bytes{}. A document written twice is \
			the same document, so either the encoder has changed or the fixture is stale; \
			regenerate the fixtures if the change was meant.",
			name, DOC_JDAT, rebuilt.len(), DOC_SBJ, buf.len(),
			match common::first_diff(&rebuilt, &buf) {
				Some(at) => fmt!(", first differing at byte {}", at),
				None => String::new(),
			};
		Test, Invalid, Mismatch));
	}

	// The index of §1.4 is derived data lying outside the hash, and is never trusted: whatever it
	// says is checked against the tree it claims to describe.
	let rest = res!(doc::index_region(&buf));
	if meta.index {
		if rest.is_empty() {
			return Err(err!(
				"The fixture '{}' declares an index, and carries none.", name;
			Test, Missing));
		}
		let idx = res!(index::parse(rest));
		res!(index::check(region, &idx));
		res!(req(name, "the entries of the index", &Some(try_into!(u64, idx.len())), &meta.nodes));
	} else if !rest.is_empty() {
		return Err(err!(
			"The fixture '{}' declares no index, and carries {} trailing bytes.",
			name, rest.len();
		Test, Invalid, Unexpected));
	}
	Ok(())
}

/// Writes a document as its fixture declares it was written.
fn rewrite(
	tree:	&Dat,
	meta:	&Meta,
	signer:	&SignatureScheme,
)
	-> Outcome<Vec<u8>>
{
	if meta.index {
		doc::write_with_index(tree, &meta.schema, signer, meta.time)
	} else {
		doc::write(tree, &meta.schema, signer, meta.time)
	}
}

/// Runs a rejection fixture: the artefact is refused, at the step declared, for the reason declared.
///
/// "It was rejected" is not the claim, since a reader that refused every document would satisfy it.
/// The claim is that the rule named in `reject.jdat` is the rule that caught it, that the failure
/// names the node or the byte it declares, and that it happened at the step of §2 it declares, which
/// is what holds the implementation to verifying before it parses.
fn reject(
	dir:	&Path,
	name:	&str,
)
	-> Outcome<()>
{
	let buf = res!(common::read_bytes(&dir.join(DOC_SBJ)));
	let dec = res!(Reject::from_dat(&res!(common::from_jdat_plain(
		&res!(common::read_text(&dir.join(REJECT_JDAT)))
	))));

	// A fixture whose content is wrong must verify first: steps 1 to 5 touch no content, and a
	// document that fails them is never decoded. A fixture whose container is wrong must fail them.
	match (dec.stage.verifies(), doc::verify_only(&buf)) {
		(true, Err(e)) => return Err(err!(e,
			"The fixture '{}' declares that it is refused at the '{}' step of SPEC.md §2, which \
			comes after verification, but it does not verify. Either the fixture is wrong about \
			what is wrong with it, or the reader is refusing it for a reason that is not the \
			fixture's.", name, dec.stage.label();
		Test, Invalid)),
		(false, Ok(_)) => return Err(err!(
			"The fixture '{}' declares that it is refused at the '{}' step of SPEC.md §2, which is \
			one of the steps that touch no content, and it verified. A document that fails \
			verification must never be decoded, so a reader that verified this one has already \
			gone further than the format allows.", name, dec.stage.label();
		Test, Invalid)),
		_ => (),
	}

	let msg = match doc::read_artefact(&buf) {
		Ok(_) => return Err(err!(
			"The fixture '{}' was read. It breaks {} A document that fails at any step renders as \
			an error card and is never partially displayed.", name, dec.rule;
		Test, Invalid)),
		Err(e) => fmt!("{}", e),
	};

	// SPEC.md §6: every rejection names the failing thing and the rule broken. "Invalid document" is
	// not an error message.
	if !msg.contains(&dec.says) {
		return Err(err!(
			"The fixture '{}' was refused, and not for the reason it declares. It breaks {} The \
			rejection must say '{}', and says: {}", name, dec.rule, dec.says, msg;
		Test, Invalid, Mismatch));
	}
	if let Some(id) = dec.node {
		let names_it = fmt!("Node {}", id);
		if !msg.contains(&names_it) {
			return Err(err!(
				"The fixture '{}' was refused for the right reason, and did not name the node it \
				is: SPEC.md §6 requires the rejection to name '{}', and it says: {}",
				name, names_it, msg;
			Test, Invalid, Mismatch));
		}
	}
	if let Some(off) = dec.offset {
		let names_it = fmt!("byte {}", off);
		if !msg.contains(&names_it) {
			return Err(err!(
				"The fixture '{}' was refused for the right reason, and did not name the byte it \
				is at: SPEC.md §6 requires the rejection to name '{}', and it says: {}",
				name, names_it, msg;
			Test, Invalid, Mismatch));
		}
	}

	// Where the fault is in the tree rather than in the bytes, the fixture carries the tree, and the
	// tree it carries is the one in the artefact.
	let jdat = dir.join(DOC_JDAT);
	if jdat.is_file() {
		// A node tree is written with the `sbj_` node labels and a record without them, so which
		// codec reads it back is decided by what the envelope says the payload is -- read here
		// WITHOUT verifying, since a fixture that fails at the header has no readable envelope and
		// carries no `doc.jdat` either.
		let payload = match envelope_schema(&buf) {
			Some(schema) if schema == SCHEMA_POST || schema == SCHEMA_CARD =>
				res!(read_payload(&jdat, name)),
			_ => res!(read_tree(&jdat, name)),
		};
		let bytes = res!(common::encode_unchecked(&payload));
		let start = res!(common::tree_start(&buf));
		if bytes.as_slice() != &buf[start..] {
			return Err(err!(
				"The fixture '{}' carries a '{}' that is not the tree region of its '{}': the tree \
				encodes to {} bytes and the region is {} bytes{}.",
				name, DOC_JDAT, DOC_SBJ, bytes.len(), buf.len() - start,
				match common::first_diff(&bytes, &buf[start..]) {
					Some(at) => fmt!(", first differing at byte {}", at),
					None => String::new(),
				};
			Test, Invalid, Mismatch));
		}
	}
	Ok(())
}

/// Reads the flat payload a fixture's `doc.jdat` carries, for a schema that is not a node tree.
///
/// Plain JDAT, with none of the `sbj_` node labels: a post and a card carry no `usr` daticles, so
/// the codec that knows the node vocabulary has nothing to do here and reading with it would only
/// make a fixture depend on a table it does not use.
fn read_payload(
	path:	&Path,
	name:	&str,
)
	-> Outcome<Dat>
{
	let s = res!(common::read_text(path));
	match common::from_jdat_plain(&s) {
		Ok(d) => Ok(d),
		Err(e) => Err(err!(e,
			"The '{}' of the fixture '{}' is not readable JDAT.", DOC_JDAT, name;
		Test, Invalid)),
	}
}

/// The schema a file's envelope declares, or `None` if the file has no readable envelope.
///
/// Deliberately UNVERIFIED, and used for nothing but choosing which text codec reads a fixture's
/// `doc.jdat`. A rejection fixture is a file with something wrong with it, so the envelope may be
/// the wrong thing about it; nothing here believes what it says beyond picking a reader.
fn envelope_schema(buf: &[u8]) -> Option<String> {
	let hdr = match envelope::read_header(buf) {
		Ok(h)  => h,
		Err(_) => return None,
	};
	let end = HEADER_LEN + hdr.env_len as usize;
	if buf.len() < end {
		return None;
	}
	match envelope::Envelope::decode(&buf[HEADER_LEN..end]) {
		Ok(env) => Some(env.schema),
		Err(_)  => None,
	}
}

/// Reads the tree a fixture's `doc.jdat` carries, naming the fixture if it will not read.
fn read_tree(
	path:	&Path,
	name:	&str,
)
	-> Outcome<Dat>
{
	let s = res!(common::read_text(path));
	match common::from_jdat(&s) {
		Ok(tree) => Ok(tree),
		Err(e) => Err(err!(e,
			"The '{}' of the fixture '{}' is not readable JDAT.", DOC_JDAT, name;
		Test, Invalid)),
	}
}

/// Requires two things to be equal, naming the fixture and what was compared.
///
/// A fixture that fails says which fixture, what was compared, what was declared and what was found,
/// since a suite that reports "assertion failed" of a fixture nobody named is as much use as the
/// error message §6 forbids.
fn req<T: PartialEq + std::fmt::Debug>(
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
			"The fixture '{}' declares {} to be {:?}, and it is {:?}.", name, what, want, got;
		Test, Invalid, Mismatch))
	}
}
