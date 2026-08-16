//! The authoring text form: a document as JDAT text, which is what an author writes.
//!
//! A document reaches the wire as BDAT, and BDAT is not a thing anyone types. The source of a
//! document is therefore its JDAT text form, which is what the fixtures of `SPEC.md` §7 carry in
//! their `doc.jdat`, and what the compiler of the `sbj` binary reads. The text is the source and the
//! bytes are the artefact, exactly as they are for the fixtures.
//!
//! Two of the v0 kind labels, `box` and `list`, are also JDAT's own kind labels, so a node written
//! as `(box|{..})` would read back as a `Dat::Box` and a node written as `(list|[..])` as a
//! `Dat::List`. Every node label therefore carries the prefix `sbj_`, and a heading is written
//! `(sbj_heading|{..})`. Nothing of this reaches the wire: BDAT carries the `u16` kind code and no
//! label at all, and a `UsrKindId` compares by code, so the label is the text form's business alone.
//!
//! A node of a kind the v0 vocabulary does not know (§4.5) is written `(sbj_k<code>|{..})`, e.g.
//! `(sbj_k99|{..})`, since a decoder must be told which code a label names before it reads a byte of
//! the document. A document may instead declare a label of its own choosing through [`KindDecl`],
//! which is what the `--kind` option of the compiler passes.

use crate::{
	kinds::NodeKind,
	limit,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
	prelude::*,
	bdat::DecodeLimits,
	string::{
		dec::DecoderConfig,
		enc::EncoderConfig,
	},
	usr::{
		UsrKind,
		UsrKindCode,
		UsrKindId,
		UsrKinds,
	},
};

use std::collections::BTreeMap;

/// The registry through which the JDAT text codec reads and writes node kinds.
pub type Ukinds = UsrKinds<BTreeMap<UsrKindCode, UsrKind>, BTreeMap<String, UsrKindId>>;

/// The nesting depth the text form of a tree at the node depth limit of §5 reaches.
///
/// The text decoder counts every bracket, brace and kindicle rather than every value, so a node
/// costs more levels in the text than it does in the bytes: the kindicle naming the kind, the
/// kindicle naming its payload map, the map itself, the kindicle naming its children list, and the
/// list. This is an upper bound on the text depth of a tree that obeys the node depth limit of §5,
/// which the validator enforces exactly. It is stated here rather than left to the decoder's default,
/// because the limit is the format's: a document that nests to the ceiling §5 sets must read, and
/// one that nests past it must be refused for that reason and not for a library's.
pub const TEXT_DEPTH: usize = 6 * limit::DEPTH + 4;

/// The greatest length, in bytes, of the text form of a document.
///
/// A tree region is at most 4 MiB (§5) and its text form is larger, since every daticle carries a
/// kindicle the bytes do not, a string may escape one character into six, and the text is indented.
/// Eight times the tree limit is beyond anything a document at the limit can reach in text, and it
/// bounds what a decoder will read from a file nobody has vouched for.
pub const TEXT_BYTES: usize = 8 * limit::TREE_BYTES;

/// The limits the text form of a document is read under. See [`TEXT_DEPTH`] and [`TEXT_BYTES`].
pub fn decode_limits() -> DecodeLimits {
	DecodeLimits::new(TEXT_DEPTH, TEXT_BYTES)
}

/// The prefix every node label carries in the text form.
pub const LABEL_PREFIX: &'static str = "sbj_";

/// The prefix a node of a kind the v0 vocabulary does not know carries, followed by its code.
pub const UNKNOWN_LABEL_PREFIX: &'static str = "sbj_k";

/// The indent one level of the text form is written with.
pub const INDENT: &'static str = "    ";

/// Every v0 node kind, in code order.
pub const KINDS: [NodeKind; 13] = [
	NodeKind::Doc,
	NodeKind::Section,
	NodeKind::Para,
	NodeKind::Heading,
	NodeKind::List,
	NodeKind::Item,
	NodeKind::Boxx,
	NodeKind::Image,
	NodeKind::Text,
	NodeKind::Emph,
	NodeKind::Link,
	NodeKind::Code,
	NodeKind::Quote,
];

/// One node kind the v0 vocabulary does not know: the label the text names it by, and its code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KindDecl {
	/// The label the text form uses, e.g. `sbj_k99`.
	pub label:	String,
	/// The wire code the label names.
	pub code:	u16,
}

/// The label a known node kind carries in the text form, e.g. `sbj_heading`.
pub fn label(kind: NodeKind) -> String {
	fmt!("{}{}", LABEL_PREFIX, kind.label())
}

/// The label a node of an unknown kind carries in the text form, e.g. `sbj_k99` (§4.5).
pub fn unknown_label(code: u16) -> String {
	fmt!("{}{}", UNKNOWN_LABEL_PREFIX, code)
}

/// The code an unknown-kind label names, or `None` if the label is not one.
pub fn unknown_code(label: &str) -> Option<u16> {
	let digits = match label.strip_prefix(UNKNOWN_LABEL_PREFIX) {
		Some(digits) => digits,
		None => return None,
	};
	if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
		return None;
	}
	match digits.parse::<u16>() {
		Ok(code) => Some(code),
		Err(_) => None,
	}
}

/// The user kind id of a known node kind: its code, its label, and the shape of its payload.
///
/// The payload kind is declared because the JDAT text decoder reads a user kind that declares one
/// and drops the payload of one that does not. Nothing of it reaches the wire, where BDAT writes the
/// `u16` code and nothing else.
pub fn ukid(kind: NodeKind) -> UsrKindId {
	let payload = if kind.payload_is_str() {
		Kind::Str
	} else {
		Kind::Map
	};
	UsrKindId::new(kind.code(), Some(&label(kind)), Some(payload))
}

/// The user kind id of a kind the v0 vocabulary does not know, whose payload §4.5 requires to be a
/// map.
pub fn unknown_ukid(decl: &KindDecl) -> UsrKindId {
	UsrKindId::new(decl.code, Some(&decl.label), Some(Kind::Map))
}

/// Builds the registry: the thirteen v0 kinds, and the unknown kinds the caller declares.
///
/// A declaration naming a code the vocabulary already knows is refused, since a node of a known kind
/// is written under its own label and a second label for it would give one document two texts.
pub fn ukinds(decls: &[KindDecl]) -> Outcome<Ukinds> {
	let mut uks = UsrKinds::new(BTreeMap::new(), BTreeMap::new());
	for kind in KINDS {
		res!(uks.add(ukid(kind)));
	}
	for decl in decls {
		if let Ok(known) = NodeKind::from_code(decl.code) {
			return Err(err!(
				"The kind declaration '{}' names the code {}, which is the v0 kind '{}'. A known \
				kind is written under its own label, '{}'.",
				decl.label, decl.code, known.label(), label(known);
			Invalid, Input, Conflict));
		}
		match uks.add(unknown_ukid(decl)) {
			Ok(()) => (),
			Err(e) => return Err(err!(e,
				"The kind declaration '{} = {}' could not be registered.", decl.label, decl.code;
			Invalid, Input)),
		}
	}
	Ok(uks)
}

/// Reads a document tree from its JDAT text form.
///
/// The unknown kinds the text names by the `sbj_k<code>` convention are found by [`scan`] and need
/// no declaring; any other label for an unknown kind must be declared in `decls`, since a decoder
/// cannot guess which code a label it has never seen names.
///
/// The JDAT text decoder is recursive and generous with its frames, spending far more of a stack per
/// level than the BDAT decoder does, so a caller reading a document that nests deeply should give the
/// reading thread a stack to do it on, as the `sbj` binary does.
pub fn decode(
	src:	&str,
	decls:	&[KindDecl],
)
	-> Outcome<Dat>
{
	let uks = res!(ukinds(&declarations(src, decls)));
	let cfg = DecoderConfig::jdat(Some(uks)).with_limits(decode_limits());
	match Dat::decode_string_with_config(src, &cfg) {
		Ok(tree) => Ok(tree),
		Err(e) => Err(err!(e,
			"The source is not readable JDAT. A node is written as its kind label and its payload, \
			e.g. (sbj_para|{{ (str|\"children\"): (list|[(sbj_text|(str|\"...\"))]) }}), and a kind \
			the v0 vocabulary does not know is written (sbj_k<code>|{{..}}).";
		Invalid, Input, Decode)),
	}
}

/// Writes a document tree in JDAT text form.
///
/// Every kindicle is written out, including the ones JDAT would infer, so that the text says what
/// the bytes say and nothing is left to a reader's guess: a `u8` reads as a `u8`, a list as a list,
/// and a map as a map. It is what §3 asks of the bytes, asked of the text. A node of a kind the
/// vocabulary does not know is written under the `sbj_k<code>` label, so that what is written here
/// reads back through [`decode`] without a declaration.
pub fn encode(tree: &Dat) -> Outcome<String> {
	let mut decls = Vec::new();
	collect_unknown(tree, &mut decls);
	let uks = res!(ukinds(&decls));
	let cfg = EncoderConfig::jdat_full_to_lines(Some(uks), INDENT);
	let mut s = res!(tree.encode_string_with_config(&cfg));
	s.push('\n');
	Ok(s)
}

/// Reads a plain daticle, such as a key file, which carries no node kinds.
pub fn decode_plain(src: &str) -> Outcome<Dat> {
	let cfg = DecoderConfig::<
		BTreeMap<UsrKindCode, UsrKind>,
		BTreeMap<String, UsrKindId>,
	>::jdat(None);
	Dat::decode_string_with_config(src, &cfg)
}

/// Writes a plain daticle, such as a key file, in JDAT text form.
pub fn encode_plain(dat: &Dat) -> Outcome<String> {
	let cfg = EncoderConfig::<
		BTreeMap<UsrKindCode, UsrKind>,
		BTreeMap<String, UsrKindId>,
	>::jdat_to_lines(None, INDENT);
	let mut s = res!(dat.encode_string_with_config(&cfg));
	s.push('\n');
	Ok(s)
}

/// The declarations a source needs: the ones the caller gave, and the `sbj_k<code>` labels it uses.
fn declarations(
	src:	&str,
	decls:	&[KindDecl],
)
	-> Vec<KindDecl>
{
	let mut all = decls.to_vec();
	for decl in scan(src) {
		// A caller's declaration wins, and a label already declared is not declared twice, since
		// registering one code under two labels is refused by the registry.
		if all.iter().any(|d| d.code == decl.code || d.label == decl.label) {
			continue;
		}
		all.push(decl);
	}
	all
}

/// Finds the `sbj_k<code>` labels a source uses, so that an unknown kind needs no declaring (§4.5).
///
/// String literals are stepped over rather than read, so that a document whose prose happens to
/// mention a label does not thereby declare a node kind.
pub fn scan(src: &str) -> Vec<KindDecl> {
	let mut out: Vec<KindDecl> = Vec::new();
	let chars: Vec<char> = src.chars().collect();
	let mut i = 0;
	while i < chars.len() {
		match chars[i] {
			'"' => {
				// Step over the string literal, honouring the backslash escape.
				i += 1;
				while i < chars.len() && chars[i] != '"' {
					if chars[i] == '\\' {
						i += 1;
					}
					i += 1;
				}
				i += 1;
			},
			'(' => {
				// A kindicle: the label runs to the vertical bar that ends it.
				i += 1;
				let start = i;
				while i < chars.len() && chars[i] != '|' && chars[i] != ')' && chars[i] != '"' {
					i += 1;
				}
				let word: String = chars[start..i].iter().collect();
				if let Some(code) = unknown_code(word.trim()) {
					let decl = KindDecl {
						label:	word.trim().to_string(),
						code,
					};
					if !out.contains(&decl) {
						out.push(decl);
					}
				}
			},
			_ => i += 1,
		}
	}
	out
}

/// Collects a declaration for every unknown kind code a tree carries, so that it can be written.
fn collect_unknown(
	dat:	&Dat,
	out:	&mut Vec<KindDecl>,
) {
	match dat {
		Dat::Usr(uid, payload) => {
			if NodeKind::from_code(uid.code()).is_err() {
				let decl = KindDecl {
					label:	unknown_label(uid.code()),
					code:	uid.code(),
				};
				if !out.contains(&decl) {
					out.push(decl);
				}
			}
			if let Some(boxd) = payload {
				collect_unknown(boxd, out);
			}
		},
		Dat::Map(map) => {
			for (_, v) in map {
				collect_unknown(v, out);
			}
		},
		Dat::OrdMap(map) => {
			for (_, v) in map {
				collect_unknown(v, out);
			}
		},
		Dat::List(list) => {
			for item in list {
				collect_unknown(item, out);
			}
		},
		Dat::Box(boxd) => collect_unknown(boxd, out),
		Dat::Opt(boxoptd) => {
			if let Some(d) = &**boxoptd {
				collect_unknown(d, out);
			}
		},
		_ => (),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The stack a thread is given before it reads a document.
	///
	/// The JDAT text decoder spends a great deal of a stack on every level of a build with no
	/// optimisation, and a test thread is given two megabytes, which a document of a few levels
	/// exhausts. The format's limits do not move to suit a test, so the test moves.
	const STACK_BYTES: usize = 64 * 1024 * 1024;

	/// Runs a test on a thread with a stack that can hold what the text decoder spends.
	fn on_a_stack<F>(f: F) -> Outcome<()>
	where
		F: FnOnce() -> Outcome<()> + Send + 'static,
	{
		let thread = match std::thread::Builder::new()
			.name("sbj_text".to_string())
			.stack_size(STACK_BYTES)
			.spawn(f)
		{
			Ok(thread) => thread,
			Err(e) => return Err(err!(e,
				"Could not spawn the thread the document is read on."; Test, Init)),
		};
		match thread.join() {
			Ok(outcome) => outcome,
			Err(_) => Err(err!(
				"The thread reading the document did not return."; Test, Panic)),
		}
	}

	/// The tree of the `one_para` fixture, in text.
	const ONE_PARA: &'static str = "\
(sbj_doc|(map|{
    (str|\"children\"): (list|[
        (sbj_para|(map|{
            (str|\"children\"): (list|[
                (sbj_text|(str|\"One paragraph.\")),
            ]),
        })),
    ]),
    (str|\"lang\"): (str|\"en\"),
    (str|\"title\"): (str|\"A document\"),
}))
";

	/// A document whose one child is a kind the v0 vocabulary does not know, carrying a fallback.
	const UNKNOWN_KIND: &'static str = "\
(sbj_doc|(map|{
    (str|\"children\"): (list|[
        (sbj_k99|(map|{
            (str|\"fallback\"): (list|[
                (sbj_para|(map|{
                    (str|\"children\"): (list|[(sbj_text|(str|\"A stand-in.\"))]),
                })),
            ]),
        })),
    ]),
    (str|\"lang\"): (str|\"en\"),
    (str|\"title\"): (str|\"A document\"),
}))
";

	#[test]
	fn test_labels_are_prefixed_00() -> Outcome<()> {
		// The two labels that collide with JDAT's own kinds are what the prefix is for.
		assert_eq!(label(NodeKind::Boxx), "sbj_box");
		assert_eq!(label(NodeKind::List), "sbj_list");
		assert_eq!(label(NodeKind::Heading), "sbj_heading");
		assert_eq!(unknown_label(99), "sbj_k99");
		assert_eq!(unknown_code("sbj_k99"), Some(99));
		assert_eq!(unknown_code("sbj_doc"), None);
		assert_eq!(unknown_code("sbj_k"), None);
		assert_eq!(unknown_code("sbj_k99x"), None);
		Ok(())
	}

	#[test]
	fn test_text_round_trip_01() -> Outcome<()> {
		on_a_stack(|| {
			let tree = res!(decode(ONE_PARA, &[]));
			let text = res!(encode(&tree));
			let again = res!(decode(&text, &[]));
			assert_eq!(tree, again, "A tree did not survive its own text form.");
			// And the text is stable: writing what was read gives the text back.
			assert_eq!(text, res!(encode(&again)), "The text form is not stable.");
			Ok(())
		})
	}

	#[test]
	fn test_unknown_kind_needs_no_declaration_02() -> Outcome<()> {
		on_a_stack(|| {
			let tree = res!(decode(UNKNOWN_KIND, &[]));
			let text = res!(encode(&tree));
			assert!(text.contains("sbj_k99"), "The unknown kind lost its label: {}", text);
			assert_eq!(tree, res!(decode(&text, &[])),
				"An unknown kind did not survive a round trip.");
			Ok(())
		})
	}

	#[test]
	fn test_a_declared_label_is_read_03() -> Outcome<()> {
		on_a_stack(|| {
			// The label a document chooses for an unknown kind is declared, never guessed.
			let src = "(sbj_alien|(map|{ (str|\"rows\"): (u8|1) }))";
			assert!(decode(src, &[]).is_err(), "An undeclared label was read.");
			let decls = vec![KindDecl { label: "sbj_alien".to_string(), code: 99 }];
			let tree = res!(decode(src, &decls));
			match &tree {
				Dat::Usr(uid, _) => assert_eq!(uid.code(), 99),
				d => return Err(err!("Expected a node, found a {:?}.", d.kind(); Test, Invalid)),
			}
			// Written back, it carries the conventional label, which needs no declaring.
			let text = res!(encode(&tree));
			assert!(text.contains("sbj_k99"), "The unknown kind was not written by code: {}", text);
			Ok(())
		})
	}

	#[test]
	fn test_a_declaration_may_not_relabel_a_known_kind_04() -> Outcome<()> {
		let decls = vec![KindDecl { label: "sbj_alien".to_string(), code: 3 }];
		match ukinds(&decls) {
			Ok(_) => Err(err!("A second label for the para kind was registered."; Test, Invalid)),
			Err(e) => {
				let msg = fmt!("{}", e);
				assert!(msg.contains("para"), "The refusal should name the kind: {}", msg);
				Ok(())
			},
		}
	}

	#[test]
	fn test_the_scan_steps_over_strings_05() -> Outcome<()> {
		// A label mentioned in prose is prose, not a declaration, and one naming a known code would
		// otherwise collide with the vocabulary.
		let decls = scan("(sbj_text|(str|\"a mention of (sbj_k3| and of sbj_k99 in a string\"))");
		assert!(decls.is_empty(), "The scan read a label out of a string: {:?}", decls);
		let decls = scan("(sbj_k20|(map|{}))");
		assert_eq!(decls, vec![KindDecl { label: "sbj_k20".to_string(), code: 20 }]);
		Ok(())
	}
}
