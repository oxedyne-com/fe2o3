//! Canonical encoding: one document, one byte string. See `SPEC.md` §3.
//!
//! The hash of the tree region is the document's address, so a tree must encode to exactly one byte
//! string or it has more than one address. BDAT is happy to encode the same logical value several
//! ways, which is why every rule of §3 exists, and why non-canonical bytes are rejected here rather
//! than quietly re-encoded.
//!
//! Two of the rules cannot be checked on a decoded tree at all, because the decoder has already
//! thrown the evidence away. A duplicate map key (§3 rule 3) collapses into one entry when BDAT
//! builds its `BTreeMap`, and a `c64` length written in more bytes than it needs decodes to the
//! same number. Both survive only in the bytes, so [`decode`] re-encodes the tree it decoded and
//! insists on getting the same bytes back. That single comparison enforces every byte-level rule at
//! once, including the ones nobody has thought of yet.

use crate::{
	kinds::{
		known_style_field,
		Content,
		FieldType,
		NodeKind,
		ReservedKind,
		StyleCheck,
		ADDR_HASH,
		ADDR_NAME,
		KEY_ALT,
		KEY_CHILDREN,
		KEY_FALLBACK,
		KEY_STYLE,
		KEY_STYLES,
	},
	limit,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
	prelude::*,
	bdat::DecodeLimits,
};
use oxedyne_fe2o3_text::unicode::norm::{
	self,
	Form,
};

/// Greatest daticle nesting depth a tree region may reach, given the node depth limit of §5.
///
/// The decoder counts daticles, not nodes, and a node costs up to three daticle levels: the `usr`
/// itself, its payload map, and the list its children sit in. This is therefore an upper bound on
/// the daticle depth of a tree that obeys the node depth limit, and the exact node depth is
/// enforced by the validator.
pub const DAT_DEPTH: usize = 3 * limit::DEPTH + 2;

/// The limits a tree region is decoded under. See `SPEC.md` §5.
pub fn decode_limits() -> DecodeLimits {
	DecodeLimits::new(DAT_DEPTH, limit::TREE_BYTES)
}

/// Checks a decoded tree against every canonical encoding rule, naming the node and the rule broken.
///
/// The rules that survive decoding are all checked here: maps are `Dat::Map` (rule 2), keys are
/// lowercase ASCII strings (rule 3), no `Dat::Box` and no `Dat::Opt` (rule 4), strings carry no
/// forbidden control characters (rule 5), integers are exactly the width the schema declares
/// (rules 1 and 6), and children sit in a `Dat::List` (rule 7). The rules that do not survive
/// decoding are checked by [`decode`].
pub fn check(tree: &Dat) -> Outcome<()> {
	let mut next: usize = 0;
	res!(check_node(tree, &mut next, 1));
	Ok(())
}

/// Encodes a tree canonically, having first checked that it obeys §3.
///
/// A tree that fails [`check`] is never encoded, since encoding it would mint an address for a
/// document that has no canonical form.
pub fn encode(tree: &Dat) -> Outcome<Vec<u8>> {
	res!(check(tree));
	Ok(res!(tree.to_bytes(Vec::new())))
}

/// Decodes a tree region, rejecting bytes that are not the canonical encoding of the tree they
/// decode to.
///
/// The buffer must hold the tree and nothing else. After decoding under the limits of §5 and
/// checking the tree against §3, the tree is re-encoded and the bytes compared, which catches the
/// non-canonicities the decoder cannot preserve: duplicate map keys, over-long `c64` lengths, and
/// any other encoding of the same value that is not the one the encoder produces.
pub fn decode(buf: &[u8]) -> Outcome<Dat> {
	let (tree, n) = res!(Dat::from_bytes_limited(buf, &decode_limits()));
	if n != buf.len() {
		return Err(err!(
			"The tree region is {} bytes, but the tree in it occupies {}. Bytes trailing a \
			tree are not part of a canonical encoding.", buf.len(), n;
		Invalid, Input, Excessive));
	}
	res!(check(&tree));
	let reenc = res!(tree.to_bytes(Vec::new()));
	if reenc.as_slice() != buf {
		let at = match reenc.iter().zip(buf.iter()).position(|(a, b)| a != b) {
			Some(i)	=> fmt!("first differing at byte {}", i),
			None	=> fmt!("one is a prefix of the other"),
		};
		return Err(err!(
			"The bytes are not the canonical encoding of the tree they decode to (SPEC.md §3): \
			re-encoding gives {} bytes against the {} supplied, {}. A duplicate map key \
			(rule 3), or a length written in more bytes than it needs, encodes one tree two \
			ways, and is rejected rather than silently re-encoded.",
			reenc.len(), buf.len(), at;
		Invalid, Input, Mismatch));
	}
	Ok(tree)
}

/// Checks a string against §3 rule 5: no forbidden control characters, and Unicode NFC.
///
/// UTF-8 well-formedness and the absence of unpaired surrogates come free: BDAT rejects a string
/// whose bytes are not UTF-8, and a Rust `String` cannot hold a surrogate. What remains is the
/// control characters, and this rejects the whole Unicode `Cc` category, which is C0, C1 and
/// delete, save for tab and newline. Carriage return is rejected too, so one line ending has one
/// encoding.
///
/// The NFC requirement closes the last way a single logical document could hold two addresses. The
/// letter é can be written as one code point or as an e followed by a combining accent. The two
/// display identically, mean the same thing, and hash differently, so without this rule one
/// document has two addresses and neither is wrong. Requiring the composed form makes the mapping
/// from meaning to address a function again.
pub fn check_string(s: &str) -> Outcome<()> {
	for (i, ch) in s.chars().enumerate() {
		if ch.is_control() && ch != '\t' && ch != '\n' {
			return Err(err!(
				"SPEC.md §3 rule 5: character {} of the string is U+{:04X}, a control \
				character. Only tab and newline are permitted.", i, ch as u32;
			Invalid, Input));
		}
	}
	if !norm::is_normalised(s, Form::Nfc) {
		return Err(err!(
			"SPEC.md §3 rule 5: the string is not in Unicode NFC. Text that displays identically \
			must encode identically, or one document has two addresses. Normalise the text to NFC \
			before signing it.";
		Invalid, Input));
	}
	Ok(())
}

/// Checks a map key against §3 rule 3: a lowercase ASCII string.
pub fn check_key_string(k: &str) -> Outcome<()> {
	if k.is_empty() {
		return Err(err!(
			"SPEC.md §3 rule 3: a map key must not be empty.";
		Invalid, Input));
	}
	for (i, ch) in k.chars().enumerate() {
		if !ch.is_ascii() {
			return Err(err!(
				"SPEC.md §3 rule 3: map key '{}' is not ASCII: character {} is U+{:04X}.",
				k, i, ch as u32;
			Invalid, Input));
		}
		if ch.is_ascii_uppercase() {
			return Err(err!(
				"SPEC.md §3 rule 3: map key '{}' is not lowercase: character {} is '{}'.",
				k, i, ch;
			Invalid, Input));
		}
	}
	Ok(())
}

/// Checks one node, and recurses into its children in depth-first, pre-order, so that `next` names
/// nodes exactly as §4.6 does.
fn check_node(
	node:	&Dat,
	next:	&mut usize,
	depth:	usize,
)
	-> Outcome<()>
{
	let id = *next;
	*next += 1;
	if depth > limit::DEPTH {
		return Err(err!(
			"Node {}: nesting reaches depth {}, past the limit of {} (SPEC.md §5).",
			id, depth, limit::DEPTH;
		Invalid, Input, Excessive));
	}
	let (ukid, payload) = match node {
		Dat::Usr(ukid, Some(boxd)) => (ukid, &**boxd),
		Dat::Usr(ukid, None) => return Err(err!(
			"Node {}: the usr daticle of kind code {} carries no payload.", id, ukid.code();
		Invalid, Input, Missing)),
		_ => return Err(err!(
			"Node {}: a node is a usr daticle (SPEC.md §4.1), found {:?}.", id, node.kind();
		Invalid, Input)),
	};

	// A node whose kind code this version does not know is still canonicalised: §4.5 lets an unknown
	// kind carry a fallback of known nodes, and its bytes obey §3 either way. Whether the kind is
	// legal at all, and whether its fallback is present, are the validator's call, not canon's.
	match NodeKind::from_code(ukid.code()) {
		Ok(kind)	=> check_known_node(kind, payload, id, next, depth),
		Err(_)	=> match ReservedKind::from_code(ukid.code()) {
			Some(reserved)	=> check_reserved_node(reserved, payload, id, next, depth),
			None	=> check_unknown_node(payload, ukid.code(), id, next, depth),
		},
	}
}

/// Canonicalises a node whose kind this version knows, pinning each field to the type and width the
/// schema declares before recursing into its children.
fn check_known_node(
	kind:	NodeKind,
	payload:	&Dat,
	id:	usize,
	next:	&mut usize,
	depth:	usize,
)
	-> Outcome<()>
{
	// A text run carries a bare string, the one node whose payload is not a map.
	if kind.payload_is_str() {
		return match payload {
			Dat::Str(s) => check_str_at(s, id, kind.label(), "text payload"),
			_ => Err(err!(
				"Node {} ({}): the payload of a text node is a str (SPEC.md §4.2), \
				found {:?}.", id, kind.label(), payload.kind();
			Invalid, Input)),
		};
	}

	let map = match payload {
		Dat::Map(map) => map,
		Dat::OrdMap(_) => return Err(err!(
			"Node {} ({}): SPEC.md §3 rule 2: a map is a Dat::Map, whose order follows its \
			keys, never a Dat::OrdMap, whose order follows the author's typing.",
			id, kind.label();
		Invalid, Input)),
		Dat::Box(_) => return Err(err!(
			"Node {} ({}): SPEC.md §3 rule 4: no redundant wrappers, so a payload is not \
			wrapped in a Dat::Box.", id, kind.label();
		Invalid, Input)),
		_ => return Err(err!(
			"Node {} ({}): the payload of this node is a map (SPEC.md §4.1), found {:?}.",
			id, kind.label(), payload.kind();
		Invalid, Input)),
	};

	// Check the payload's own keys and values before descending, so that the children of this node
	// take the ids immediately after it.
	let mut kids: Option<&Vec<Dat>> = None;
	for (k, v) in map {
		let key = match k {
			Dat::Str(s) => s,
			_ => return Err(err!(
				"Node {} ({}): SPEC.md §3 rule 3: a map key is a Dat::Str, found {:?}.",
				id, kind.label(), k.kind();
			Invalid, Input)),
		};
		res!(check_key_at(key, id, kind.label()));
		if key == KEY_CHILDREN {
			kids = Some(res!(check_children(v, id, kind)));
		} else if key == KEY_STYLE {
			// The universal style field (§4.4), permitted on any map-payload node.
			res!(check_style_name(v, id, kind.label()));
		} else if key == KEY_STYLES {
			// The document style table (§4.4); its placement is the validator's call.
			res!(check_styles_table(v, id, kind.label()));
		} else {
			res!(check_field(key, v, id, kind));
		}
	}

	if let Some(list) = kids {
		for child in list {
			res!(check_node(child, next, depth + 1));
		}
	}

	Ok(())
}

/// Canonicalises a node of a kind the format reserves (§4.2): an `edit` or a `surface`.
///
/// Canon does not pin a reserved kind's field widths, and this is deliberate. Whether a reserved kind
/// is admitted at all depends on the schema the envelope declares, which canon is not told and must
/// not consult: a document carrying a surface is a tree the *validator* refuses, and it must reach the
/// validator to be refused by name. A reserved node the schema does not admit may therefore carry
/// anything at all -- including a `fallback`, which is exactly the smuggling attempt §4.5 exists to
/// close -- and holding it to a field table here would refuse it as a canonicity fault rather than as
/// what it is. The fields of a reserved node the schema *does* admit are pinned by the validator,
/// which knows the schema.
///
/// What canon does own is the bytes. Every field is held to the structural rules of §3, and the two
/// fields that carry nodes -- a surface's `alt` (§4.2) and a fallback (§4.5) -- are walked as nodes,
/// so that the content standing in for an application is canonicalised exactly as the content around
/// it is, and takes its node ids in the same pre-order.
fn check_reserved_node(
	reserved:	ReservedKind,
	payload:	&Dat,
	id:	usize,
	next:	&mut usize,
	depth:	usize,
)
	-> Outcome<()>
{
	let label = reserved.label();
	let map = match payload {
		Dat::Map(map) => map,
		Dat::OrdMap(_) => return Err(err!(
			"Node {} ({}): SPEC.md §3 rule 2: a map is a Dat::Map, never a Dat::OrdMap.", id, label;
		Invalid, Input)),
		Dat::Box(_) => return Err(err!(
			"Node {} ({}): SPEC.md §3 rule 4: no redundant wrappers, so a payload is not wrapped in \
			a Dat::Box.", id, label;
		Invalid, Input)),
		// A non-map payload is what the validator refuses; canon still holds its bytes to §3.
		other => return check_struct(other, id),
	};

	// The alternative, then the fallback: both are lists of nodes, and both are walked. A `BTreeMap`
	// hands the keys back in order, and 'alt' precedes 'fallback', so the ids fall in the order the
	// bytes carry them.
	let mut alt: Option<&Vec<Dat>> = None;
	let mut fallback: Option<&Vec<Dat>> = None;
	for (k, v) in map {
		let key = match k {
			Dat::Str(s) => s,
			_ => return Err(err!(
				"Node {} ({}): SPEC.md §3 rule 3: a map key is a Dat::Str, found {:?}.",
				id, label, k.kind();
			Invalid, Input)),
		};
		res!(check_key_at(key, id, label));
		if key == KEY_ALT && reserved == ReservedKind::Surface {
			alt = Some(res!(check_node_list(v, id, label, KEY_ALT)));
		} else if key == KEY_FALLBACK {
			fallback = Some(res!(check_node_list(v, id, label, KEY_FALLBACK)));
		} else {
			res!(check_struct(v, id));
		}
	}

	if let Some(list) = alt {
		for child in list {
			res!(check_node(child, next, depth + 1));
		}
	}
	if let Some(list) = fallback {
		for child in list {
			res!(check_node(child, next, depth + 1));
		}
	}

	Ok(())
}

/// Canonicalises a node whose kind code this version does not know (§4.5).
///
/// Canon has no schema for an unknown kind, so it cannot pin the widths of its fields; it enforces
/// only the structural rules of §3 that hold regardless of type. The one field it recognises is the
/// `fallback` list, whose elements are known nodes and take node ids in pre-order like any other
/// children. Whether the fallback is present and non-empty is the validator's call.
fn check_unknown_node(
	payload:	&Dat,
	code:	u16,
	id:	usize,
	next:	&mut usize,
	depth:	usize,
)
	-> Outcome<()>
{
	let label = fmt!("unknown kind {}", code);
	let map = match payload {
		Dat::Map(map) => map,
		Dat::OrdMap(_) => return Err(err!(
			"Node {} ({}): SPEC.md §3 rule 2: a map is a Dat::Map, never a Dat::OrdMap.",
			id, label;
		Invalid, Input)),
		Dat::Box(_) => return Err(err!(
			"Node {} ({}): SPEC.md §3 rule 4: no redundant wrappers, so a payload is not \
			wrapped in a Dat::Box.", id, label;
		Invalid, Input)),
		// A non-map payload cannot carry a fallback, which the validator rejects; canon still holds
		// its bytes to §3.
		other => return check_struct(other, id),
	};

	let mut fallback: Option<&Vec<Dat>> = None;
	for (k, v) in map {
		let key = match k {
			Dat::Str(s) => s,
			_ => return Err(err!(
				"Node {} ({}): SPEC.md §3 rule 3: a map key is a Dat::Str, found {:?}.",
				id, label, k.kind();
			Invalid, Input)),
		};
		res!(check_key_at(key, id, &label));
		if key == KEY_FALLBACK {
			fallback = Some(res!(check_node_list(v, id, &label, KEY_FALLBACK)));
		} else {
			// Canon does not know this field's width, so it holds only its structure to §3.
			res!(check_struct(v, id));
		}
	}

	if let Some(list) = fallback {
		for child in list {
			res!(check_node(child, next, depth + 1));
		}
	}

	Ok(())
}

/// Checks the value under the `children` key, returning the list it must be.
fn check_children<'a>(
	v:	&'a Dat,
	id:	usize,
	kind:	NodeKind,
)
	-> Outcome<&'a Vec<Dat>>
{
	if kind.content() == Content::None {
		return Err(err!(
			"Node {} ({}): this kind takes no children (SPEC.md §4.2), so the '{}' key is \
			omitted rather than carried empty (SPEC.md §3 rule 4).",
			id, kind.label(), KEY_CHILDREN;
		Invalid, Input));
	}
	match v {
		Dat::List(list) => {
			if list.is_empty() {
				Err(err!(
					"Node {} ({}): SPEC.md §3 rule 4: a node with no children omits the '{}' \
					key rather than carrying an empty list, which would give one document two \
					encodings.", id, kind.label(), KEY_CHILDREN;
				Invalid, Input))
			} else {
				Ok(list)
			}
		},
		Dat::Vek(_) => Err(err!(
			"Node {} ({}): SPEC.md §3 rule 7: children sit in a Dat::List, never a Dat::Vek, \
			even where every child shares a kind.", id, kind.label();
		Invalid, Input)),
		Dat::Opt(_) | Dat::Box(_) => Err(err!(
			"Node {} ({}): SPEC.md §3 rule 4: no redundant wrappers, so the '{}' key carries a \
			bare list.", id, kind.label(), KEY_CHILDREN;
		Invalid, Input)),
		_ => Err(err!(
			"Node {} ({}): the '{}' key carries a list of nodes (SPEC.md §4.2), found {:?}.",
			id, kind.label(), KEY_CHILDREN, v.kind();
		Invalid, Input)),
	}
}

/// Checks one field of a node's payload map against the type the schema declares for it.
fn check_field(
	key:	&str,
	v:	&Dat,
	id:	usize,
	kind:	NodeKind,
)
	-> Outcome<()>
{
	let field = match kind.fields().iter().find(|f| f.name == key) {
		Some(field) => field,
		None => return Err(err!(
			"Node {} ({}): SPEC.md §3 rule 1: field types are fixed by the schema, and the \
			schema for this kind declares no field '{}'.", id, kind.label(), key;
		Invalid, Input)),
	};

	// Rule 4 before rule 1, so that a wrapper is named as a wrapper rather than as a type error.
	res!(check_no_wrapper(v, id, kind.label(), &fmt!("field '{}'", key)));

	// A typed address is a structural map (§4.3), canonicalised entry by entry rather than pinned to
	// a single daticle width.
	if field.typ == FieldType::Address {
		return check_address_map(v, id, kind.label());
	}

	let ok = match (field.typ, v) {
		(FieldType::Str,	Dat::Str(_))	=> true,
		(FieldType::U8,	Dat::U8(_))	=> true,
		(FieldType::I8,	Dat::I8(_))	=> true,
		(FieldType::U32,	Dat::U32(_))	=> true,
		(FieldType::Bool,	Dat::Bool(_))	=> true,
		(FieldType::Hash32,	Dat::B32(_))	=> true,
		_	=> false,
	};
	if !ok {
		return Err(err!(
			"Node {} ({}): SPEC.md §3 rules 1 and 6: field '{}' is declared {} by the schema, \
			with no promotion and no demotion, but carries a {:?}.",
			id, kind.label(), key, type_name(field.typ), v.kind();
		Invalid, Input));
	}
	if let Dat::Str(s) = v {
		res!(check_str_at(s, id, kind.label(), key));
	}
	Ok(())
}

/// The wire type a field type names, as the schema writes it.
fn type_name(typ: FieldType) -> &'static str {
	match typ {
		FieldType::Str	=> "str",
		FieldType::U8	=> "u8",
		FieldType::I8	=> "i8",
		FieldType::U32	=> "u32",
		FieldType::Bool	=> "bool",
		FieldType::Hash32	=> "b32",
		FieldType::Address	=> "address",
		FieldType::Nodes	=> "a non-empty list of nodes",
	}
}

/// Pins the universal `style` field (§4.4) to a `str`, permitted on any map-payload node.
fn check_style_name(
	v:	&Dat,
	id:	usize,
	label:	&str,
)
	-> Outcome<()>
{
	res!(check_no_wrapper(v, id, label, "the 'style' field"));
	match v {
		Dat::Str(s) => check_str_at(s, id, label, "the 'style' field"),
		_ => Err(err!(
			"Node {} ({}): SPEC.md §3 rule 1: the 'style' field names a style entry as a str, \
			found {:?}.", id, label, v.kind();
		Invalid, Input)),
	}
}

/// Canonicalises the document `styles` table (§4.4): a map from style name to a style record.
fn check_styles_table(
	v:	&Dat,
	id:	usize,
	label:	&str,
)
	-> Outcome<()>
{
	res!(check_no_wrapper(v, id, label, "the 'styles' table"));
	let table = match v {
		Dat::Map(map) => map,
		Dat::OrdMap(_) => return Err(err!(
			"Node {} ({}): SPEC.md §3 rule 2: the 'styles' table is a Dat::Map, whose order \
			follows its keys, never a Dat::OrdMap.", id, label;
		Invalid, Input)),
		_ => return Err(err!(
			"Node {} ({}): the 'styles' table is a map from style name to record (SPEC.md §4.4), \
			found {:?}.", id, label, v.kind();
		Invalid, Input)),
	};
	// An empty table defines nothing, exactly like an absent one, so accepting it would give one
	// document two encodings and two addresses. It must be omitted rather than written empty.
	if table.is_empty() {
		return Err(err!(
			"Node {} ({}): SPEC.md §3: an empty 'styles' table defines nothing and must be omitted, \
			since it would otherwise give one document two encodings.", id, label;
		Invalid, Input));
	}
	for (k, rec) in table {
		let name = match k {
			Dat::Str(s) => s,
			_ => return Err(err!(
				"Node {} ({}): SPEC.md §3 rule 3: a style name is a Dat::Str, found {:?}.",
				id, label, k.kind();
			Invalid, Input)),
		};
		res!(check_key_at(name, id, label));
		res!(check_style_record(rec, id, label, name));
	}
	Ok(())
}

/// Canonicalises one style record: a map whose property values are pinned by their `StyleCheck`.
fn check_style_record(
	rec:	&Dat,
	id:	usize,
	label:	&str,
	name:	&str,
)
	-> Outcome<()>
{
	let record = match rec {
		Dat::Map(map) => map,
		Dat::OrdMap(_) => return Err(err!(
			"Node {} ({}): SPEC.md §3 rule 2: style record '{}' is a Dat::Map, never a \
			Dat::OrdMap.", id, label, name;
		Invalid, Input)),
		Dat::Box(_) => return Err(err!(
			"Node {} ({}): SPEC.md §3 rule 4: no redundant wrappers, so style record '{}' is not \
			wrapped in a Dat::Box.", id, label, name;
		Invalid, Input)),
		_ => return Err(err!(
			"Node {} ({}): style record '{}' is a map (SPEC.md §4.4), found {:?}.",
			id, label, name, rec.kind();
		Invalid, Input)),
	};
	// An empty record sets no property and so has no effect, the same two-encodings trap as an empty
	// table: a style worth naming defines at least one property.
	if record.is_empty() {
		return Err(err!(
			"Node {} ({}): SPEC.md §3: style record '{}' is empty and sets nothing, so it must be \
			removed rather than written empty.", id, label, name;
		Invalid, Input));
	}
	for (k, v) in record {
		let prop = match k {
			Dat::Str(s) => s,
			_ => return Err(err!(
				"Node {} ({}): SPEC.md §3 rule 3: a style property key is a Dat::Str, found {:?}.",
				id, label, k.kind();
			Invalid, Input)),
		};
		res!(check_key_at(prop, id, label));
		res!(check_no_wrapper(v, id, label, &fmt!("style property '{}'", prop)));
		// Canon asks what a property IS, and never whether this tree may name it: a property's wire
		// type is the same in every schema, so `grid` is a u8 wherever it is legal, and pinning that
		// width is the same work in a document as in a chrome. Whether a document may name `grid` at
		// all is the validator's question (§4.4), and it is asked whatever these bytes say.
		let sf = match known_style_field(prop) {
			Some(sf) => sf,
			// An unknown style property has no declared width; canon holds its bytes to §3 and the
			// validator rejects the property itself.
			None => {
				res!(check_struct(v, id));
				continue;
			},
		};
		// A border is the one style property that is not a scalar, so its shape is pinned by its own
		// routine rather than by the width table below.
		if sf.check == StyleCheck::Border {
			res!(check_style_border(v, id, label, prop));
			continue;
		}
		let ok = match (sf.check, v) {
			(StyleCheck::Palette,	Dat::Str(_))	=> true,
			(StyleCheck::Lang,	Dat::Str(_))	=> true,
			(StyleCheck::Direction,	Dat::Str(_))	=> true,
			(StyleCheck::Alignment,	Dat::Str(_))	=> true,
			(StyleCheck::ScaleStep,	Dat::I8(_))	=> true,
			(StyleCheck::Spacing,	Dat::U8(_))	=> true,
			(StyleCheck::Tile,	Dat::U16(_))	=> true,
			(StyleCheck::Share,	Dat::U8(_))	=> true,
			(StyleCheck::Elevation,	Dat::U8(_))	=> true,
			_	=> false,
		};
		if !ok {
			return Err(err!(
				"Node {} ({}): SPEC.md §3 rules 1 and 6: style property '{}' is declared \
				{} by the schema, with no promotion and no demotion, but carries a {:?}.",
				id, label, prop, style_check_type(sf.check), v.kind();
			Invalid, Input));
		}
		if let Dat::Str(s) = v {
			res!(check_str_at(s, id, label, prop));
		}
	}
	Ok(())
}

/// Canonicalises a style's `border`: a two-element list of a palette name and a width in pixels.
///
/// Its parts are pinned exactly as any other style value is. The width is the `u8` the schema
/// declares, with no promotion and no demotion (§3 rules 1 and 6); the list is a `Dat::List` and
/// never a `Dat::Vek` (rule 7); and the name obeys the string rules (rule 5). Neither part can be
/// wrapped, since a wrapper is not a `Dat::Str` or a `Dat::U8` and the match below takes nothing else
/// (rule 4).
///
/// Whether the name is one the palette holds is the validator's question and not canon's, exactly as
/// with `fill` and `bg`: a colour outside the palette is a well-formed encoding of a style that means
/// nothing, and it is refused for meaning nothing.
fn check_style_border(
	v:	&Dat,
	id:	usize,
	label:	&str,
	prop:	&str,
)
	-> Outcome<()>
{
	let list = match v {
		Dat::List(list) => list,
		Dat::Vek(_) => return Err(err!(
			"Node {} ({}): SPEC.md §3 rule 7: style property '{}' is a Dat::List, never a Dat::Vek.",
			id, label, prop;
		Invalid, Input)),
		_ => return Err(err!(
			"Node {} ({}): style property '{}' is a palette name and a width in pixels, written as \
			a two-element list (SPEC.md §4.4), found {:?}.", id, label, prop, v.kind();
		Invalid, Input)),
	};
	match list.as_slice() {
		[Dat::Str(colour), Dat::U8(_)] => check_str_at(colour, id, label, prop),
		_ => Err(err!(
			"Node {} ({}): SPEC.md §3 rules 1 and 6: style property '{}' is declared {} by the \
			schema, with no promotion and no demotion.",
			id, label, prop, style_check_type(StyleCheck::Border);
		Invalid, Input)),
	}
}

/// The wire type a style property's check names, as the schema writes it.
fn style_check_type(check: StyleCheck) -> &'static str {
	match check {
		StyleCheck::Palette	=> "str",
		StyleCheck::Lang	=> "str",
		StyleCheck::Direction	=> "str",
		StyleCheck::Alignment	=> "str",
		StyleCheck::ScaleStep	=> "i8",
		StyleCheck::Spacing	=> "u8",
		StyleCheck::Tile	=> "u16",
		StyleCheck::Share	=> "u8",
		StyleCheck::Elevation	=> "u8",
		StyleCheck::Border	=> "a str and a u8, in a two-element list",
	}
}

/// Canonicalises a `link` address map (§4.3), pinning `name` to a `str` and `hash` to a `b32`.
///
/// Canon enforces only the byte-canonicity of whatever entries are present; whether the map holds
/// exactly one, and whether its key is one [`check_address`](crate::kinds::check_address) knows, is
/// the validator's call.
fn check_address_map(
	v:	&Dat,
	id:	usize,
	label:	&str,
)
	-> Outcome<()>
{
	let map = match v {
		Dat::Map(map) => map,
		Dat::OrdMap(_) => return Err(err!(
			"Node {} ({}): SPEC.md §3 rule 2: a link address is a Dat::Map, whose order follows \
			its keys, never a Dat::OrdMap.", id, label;
		Invalid, Input)),
		_ => return Err(err!(
			"Node {} ({}): a link address is a single-entry map (SPEC.md §4.3), found {:?}.",
			id, label, v.kind();
		Invalid, Input)),
	};
	for (k, av) in map {
		let key = match k {
			Dat::Str(s) => s,
			_ => return Err(err!(
				"Node {} ({}): SPEC.md §3 rule 3: an address key is a Dat::Str, found {:?}.",
				id, label, k.kind();
			Invalid, Input)),
		};
		res!(check_key_at(key, id, label));
		res!(check_no_wrapper(av, id, label, "an address value"));
		if key == ADDR_NAME {
			match av {
				Dat::Str(s) => res!(check_str_at(s, id, label, "the address name")),
				_ => return Err(err!(
					"Node {} ({}): SPEC.md §3 rules 1 and 6: an address '{}' is a str, found {:?}.",
					id, label, ADDR_NAME, av.kind();
				Invalid, Input)),
			}
		} else if key == ADDR_HASH {
			match av {
				Dat::B32(_) => (),
				_ => return Err(err!(
					"Node {} ({}): SPEC.md §3 rules 1 and 6: an address '{}' is a b32, found {:?}.",
					id, label, ADDR_HASH, av.kind();
				Invalid, Input)),
			}
		} else {
			// An unknown address key has no declared width; canon holds only its bytes to §3.
			res!(check_struct(av, id));
		}
	}
	Ok(())
}

/// Checks the value under a key that carries nodes, returning the list it must be.
///
/// Two keys do: an unknown kind's `fallback` (§4.5), and a surface's `alt` (§4.2). Both stand in for
/// something the reader is not showing, and both are lists of ordinary nodes, canonicalised as such.
fn check_node_list<'a>(
	v:	&'a Dat,
	id:	usize,
	label:	&str,
	key:	&str,
)
	-> Outcome<&'a Vec<Dat>>
{
	match v {
		Dat::List(list) => Ok(list),
		Dat::Vek(_) => Err(err!(
			"Node {} ({}): SPEC.md §3 rule 7: the '{}' list is a Dat::List, never a Dat::Vek.",
			id, label, key;
		Invalid, Input)),
		Dat::Opt(_) | Dat::Box(_) => Err(err!(
			"Node {} ({}): SPEC.md §3 rule 4: no redundant wrappers, so the '{}' key carries a \
			bare list.", id, label, key;
		Invalid, Input)),
		_ => Err(err!(
			"Node {} ({}): the '{}' key carries a list of nodes, found {:?}.",
			id, label, key, v.kind();
		Invalid, Input)),
	}
}

/// Rejects a value wrapped in a redundant `Dat::Opt` or `Dat::Box` (§3 rule 4), naming the node.
fn check_no_wrapper(
	v:	&Dat,
	id:	usize,
	label:	&str,
	what:	&str,
)
	-> Outcome<()>
{
	match v {
		Dat::Opt(_) => Err(err!(
			"Node {} ({}): SPEC.md §3 rule 4: {} carries a Dat::Opt. An optional field carries \
			its bare value when present and is omitted when absent, so an optional never reaches \
			the wire as a none, and never as a redundant some.", id, label, what;
		Invalid, Input)),
		Dat::Box(_) => Err(err!(
			"Node {} ({}): SPEC.md §3 rule 4: no redundant wrappers, so {} is not wrapped in a \
			Dat::Box.", id, label, what;
		Invalid, Input)),
		_ => Ok(()),
	}
}

/// Canonicalises a value whose field width the schema does not fix, enforcing the structural rules
/// of §3 that hold regardless of type.
///
/// It rejects a `Dat::OrdMap` (rule 2), a `Dat::Box` (rule 4), and a `Dat::Vek` (rule 7), insists on
/// lowercase ASCII string keys (rule 3), and forbids control characters in strings (rule 5),
/// recursing through maps and lists. It is used for the fields of an unknown kind (§4.5) and for any
/// entry of an address map or style record whose key the schema does not name.
fn check_struct(
	v:	&Dat,
	id:	usize,
)
	-> Outcome<()>
{
	match v {
		Dat::Map(map) => {
			for (k, val) in map {
				let key = match k {
					Dat::Str(s) => s,
					_ => return Err(err!(
						"Node {}: SPEC.md §3 rule 3: a map key is a Dat::Str, found {:?}.",
						id, k.kind();
					Invalid, Input)),
				};
				match check_key_string(key) {
					Ok(()) => (),
					Err(e) => return Err(err!(e,
						"Node {}: the map carries a key the format does not permit (§3 rule 3).",
						id;
					Invalid, Input)),
				}
				res!(check_struct(val, id));
			}
			Ok(())
		},
		Dat::OrdMap(_) => Err(err!(
			"Node {}: SPEC.md §3 rule 2: a map is a Dat::Map, never a Dat::OrdMap.", id;
		Invalid, Input)),
		Dat::Box(_) => Err(err!(
			"Node {}: SPEC.md §3 rule 4: no redundant wrappers, so a value is not wrapped in a \
			Dat::Box.", id;
		Invalid, Input)),
		Dat::Vek(_) => Err(err!(
			"Node {}: SPEC.md §3 rule 7: a homogeneous sequence is still a Dat::List, never a \
			Dat::Vek.", id;
		Invalid, Input)),
		Dat::List(list) => check_seq(list, id),
		// A tuple, a user-tagged value, and an option each enclose further daticles. §4.5 holds an
		// unknown kind's uninterpreted fields to §3, so the recursion must reach inside them, or a
		// forbidden string or an OrdMap could hide in a field the schema does not name. Leaving these
		// to the catch-all was the gap that let `(20|{...,"rows":[bad\rstring, 0]})` earn an address.
		Dat::Tup2(a)	=> check_seq(&a[..], id),
		Dat::Tup3(a)	=> check_seq(&a[..], id),
		Dat::Tup4(a)	=> check_seq(&a[..], id),
		Dat::Tup5(a)	=> check_seq(&a[..], id),
		Dat::Tup6(a)	=> check_seq(&a[..], id),
		Dat::Tup7(a)	=> check_seq(&a[..], id),
		Dat::Tup8(a)	=> check_seq(&a[..], id),
		Dat::Tup9(a)	=> check_seq(&a[..], id),
		Dat::Tup10(a)	=> check_seq(&a[..], id),
		Dat::Usr(_, Some(boxd))	=> check_struct(boxd, id),
		Dat::Usr(_, None)	=> Ok(()),
		Dat::Opt(boxoptd) => match &**boxoptd {
			Some(d) => check_struct(d, id),
			None => Ok(()),
		},
		Dat::Str(s) => match check_string(s) {
			Ok(()) => Ok(()),
			Err(e) => Err(err!(e,
				"Node {}: a string carries a character the format does not permit (§3 rule 5).",
				id;
			Invalid, Input)),
		},
		_ => Ok(()),
	}
}

/// Runs [`check_struct`] over every element of a sequence, naming the node it sits in.
fn check_seq(
	items:	&[Dat],
	id:	usize,
)
	-> Outcome<()>
{
	for item in items {
		res!(check_struct(item, id));
	}
	Ok(())
}

/// Checks a string, naming the node it sits in if it fails.
fn check_str_at(
	s:	&str,
	id:	usize,
	label:	&str,
	what:	&str,
)
	-> Outcome<()>
{
	match check_string(s) {
		Ok(()) => Ok(()),
		Err(e) => Err(err!(e,
			"Node {} ({}): the {} carries a character the format does not permit.",
			id, label, what;
		Invalid, Input)),
	}
}

/// Checks a map key, naming the node it sits in if it fails.
fn check_key_at(
	key:	&str,
	id:	usize,
	label:	&str,
)
	-> Outcome<()>
{
	match check_key_string(key) {
		Ok(()) => Ok(()),
		Err(e) => Err(err!(e,
			"Node {} ({}): the payload map carries a key the format does not permit.",
			id, label;
		Invalid, Input)),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	use oxedyne_fe2o3_jdat::usr::UsrKindId;

	/// Builds a node of the given kind around the given payload.
	fn node(kind: NodeKind, payload: Dat) -> Dat {
		Dat::Usr(
			UsrKindId::new(kind.code(), Some(kind.label()), None),
			Some(Box::new(payload)),
		)
	}

	/// Builds a payload map from string keys.
	fn map(kv: Vec<(&str, Dat)>) -> Dat {
		create_dat_map(
			kv.into_iter().map(|(k, v)| (Dat::Str(k.to_string()), v)).collect()
		)
	}

	/// Builds a text node.
	fn text(s: &str) -> Dat {
		node(NodeKind::Text, Dat::Str(s.to_string()))
	}

	/// A document exercising every v0 node kind once, with a style table, an optional field present,
	/// an optional field absent, a node with no children, an inherited style property (`size`), a
	/// self-only property (`bg`), a link by name, and a link by hash.
	fn valid_doc() -> Dat {
		node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("Style without a cascade".to_string())),
			("lang",	Dat::Str("en".to_string())),
			(KEY_STYLES, map(vec![
				("callout", map(vec![
					("bg",	Dat::Str("muted".to_string())),
					("pad",	Dat::U8(3)),
					("fill",	Dat::Str("ink".to_string())),
				])),
				("lede", map(vec![
					("size",	Dat::I8(1)),
				])),
			])),
			(KEY_CHILDREN, Dat::List(vec![
				node(NodeKind::Heading, map(vec![
					("level",	Dat::U8(2)),
					(KEY_CHILDREN,	Dat::List(vec![text("Style without a cascade")])),
				])),
				node(NodeKind::Section, map(vec![
					("title",	Dat::Str("A section".to_string())),
					(KEY_CHILDREN, Dat::List(vec![
						node(NodeKind::Para, map(vec![
							(KEY_STYLE,	Dat::Str("lede".to_string())),
							(KEY_CHILDREN, Dat::List(vec![
								text("A run\twith a tab\nand a newline."),
								node(NodeKind::Emph, map(vec![
									("strong",	Dat::Bool(true)),
									(KEY_CHILDREN,	Dat::List(vec![text("loud")])),
								])),
								node(NodeKind::Link, map(vec![
									("to",	map(vec![
										("name",	Dat::Str("news.cricket".to_string())),
									])),
									(KEY_CHILDREN,	Dat::List(vec![text("a link")])),
								])),
								node(NodeKind::Link, map(vec![
									("to",	map(vec![
										("hash",	Dat::from([0x9fu8; 32])),
									])),
									(KEY_CHILDREN,	Dat::List(vec![text("a hash link")])),
								])),
							])),
						])),
						// A paragraph with no children omits the key entirely.
						node(NodeKind::Para, map(vec![])),
						node(NodeKind::Code, map(vec![
							("lang",	Dat::Str("rust".to_string())),
							("text",	Dat::Str("fn main() {}".to_string())),
						])),
						node(NodeKind::Quote, map(vec![
							("cite",	Dat::Str("a source".to_string())),
							(KEY_CHILDREN, Dat::List(vec![
								node(NodeKind::Para, map(vec![
									(KEY_CHILDREN, Dat::List(vec![text("quoted")])),
								])),
							])),
						])),
						node(NodeKind::List, map(vec![
							("ordered",	Dat::Bool(false)),
							(KEY_CHILDREN, Dat::List(vec![
								node(NodeKind::Item, map(vec![
									(KEY_CHILDREN, Dat::List(vec![
										node(NodeKind::Para, map(vec![
											(KEY_CHILDREN, Dat::List(vec![text("one")])),
										])),
									])),
								])),
							])),
						])),
						// A box naming a style entry, with an image whose optional dimensions are
						// present.
						node(NodeKind::Boxx, map(vec![
							(KEY_STYLE,	Dat::Str("callout".to_string())),
							(KEY_CHILDREN, Dat::List(vec![
								node(NodeKind::Image, map(vec![
									("hash",	Dat::from([0x01u8; 32])),
									("alt",	Dat::Str("a picture".to_string())),
									("w",	Dat::U32(640)),
									("h",	Dat::U32(480)),
								])),
							])),
						])),
					])),
				])),
			])),
		]))
	}

	/// Replaces the payload map of the heading, which is node 1 of `valid_doc`.
	fn doc_with_heading(payload: Dat) -> Dat {
		node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("T".to_string())),
			("lang",	Dat::Str("en".to_string())),
			(KEY_CHILDREN, Dat::List(vec![
				Dat::Usr(
					UsrKindId::new(NodeKind::Heading.code(), Some("heading"), None),
					Some(Box::new(payload)),
				),
			])),
		]))
	}

	/// Asserts that checking the tree fails, and that the message names the given rule and node.
	fn rejects(tree: &Dat, rule: &str, node_id: &str) {
		match check(tree) {
			Ok(()) => assert!(false, "Expected a rejection naming {}, but the tree passed.", rule),
			Err(e) => {
				let msg = fmt!("{}", e);
				assert!(msg.contains(rule), "Expected {} in the error, got: {}", rule, msg);
				assert!(msg.contains(node_id), "Expected {} in the error, got: {}", node_id, msg);
			},
		}
		// A tree that fails the check is never encoded.
		assert!(encode(tree).is_err(), "A tree that fails check() was encoded anyway.");
	}

	#[test]
	fn test_valid_tree_passes() -> Outcome<()> {
		res!(check(&valid_doc()));
		Ok(())
	}

	#[test]
	fn test_round_trip() -> Outcome<()> {
		let tree = valid_doc();
		let enc1 = res!(encode(&tree));
		let dec = res!(decode(&enc1));
		let enc2 = res!(encode(&dec));
		assert_eq!(enc1, enc2, "Encoding is not stable across a decode.");
		// And the decoded tree is the tree.
		assert_eq!(tree, dec, "A tree did not survive its own encoding.");
		Ok(())
	}

	#[test]
	fn test_rule_1_undeclared_field() {
		let tree = doc_with_heading(map(vec![
			("level",	Dat::U8(2)),
			("colour",	Dat::Str("red".to_string())),
			(KEY_CHILDREN,	Dat::List(vec![text("h")])),
		]));
		rejects(&tree, "rule 1", "Node 1");
	}

	#[test]
	fn test_rule_2_ordmap_payload() {
		let payload = create_dat_ordmap(vec![
			(Dat::Str("level".to_string()), Dat::U8(2)),
		]);
		rejects(&doc_with_heading(payload), "rule 2", "Node 1");
	}

	#[test]
	fn test_rule_3_key_not_a_string() {
		let mut m = DaticleMap::new();
		m.insert(Dat::U8(1), Dat::U8(2));
		rejects(&doc_with_heading(Dat::Map(m)), "rule 3", "Node 1");
	}

	#[test]
	fn test_rule_3_uppercase_key() {
		let tree = doc_with_heading(map(vec![
			("Level",	Dat::U8(2)),
		]));
		rejects(&tree, "rule 3", "Node 1");
	}

	#[test]
	fn test_rule_3_duplicate_key_survives_only_in_the_bytes() -> Outcome<()> {
		// A BTreeMap cannot hold a duplicate key, so a duplicate can only be written by hand, and
		// can only be caught by comparing the bytes with the re-encoding of what they decode to.
		let mut inner = Vec::new();
		for _ in 0..2 {
			inner = res!(Dat::Str("level".to_string()).to_bytes(inner));
			inner = res!(Dat::U8(2).to_bytes(inner));
		}
		let mut payload = Vec::new();
		payload.push(Dat::MAP_CODE);
		payload = res!(Dat::C64(inner.len() as u64).to_bytes(payload));
		payload.extend_from_slice(&inner);

		let mut buf = Vec::new();
		buf.push(Dat::USR_CODE);
		buf.extend_from_slice(&NodeKind::Heading.code().to_be_bytes());
		buf.push(Dat::OPT_SOME_CODE);
		buf.extend_from_slice(&payload);

		// The tree the bytes decode to is perfectly canonical, which is the point.
		let (tree, n) = res!(Dat::from_bytes(&buf));
		assert_eq!(n, buf.len());
		res!(check(&tree));
		assert!(res!(encode(&tree)).len() < buf.len(), "The duplicate key did not collapse.");

		match decode(&buf) {
			Ok(_) => assert!(false, "Bytes carrying a duplicate map key were accepted."),
			Err(e) => {
				let msg = fmt!("{}", e);
				assert!(msg.contains("§3"), "Expected a canonicity rejection, got: {}", msg);
			},
		}
		Ok(())
	}

	#[test]
	fn test_rule_4_box_payload() {
		let payload = Dat::Box(Box::new(map(vec![("level", Dat::U8(2))])));
		rejects(&doc_with_heading(payload), "rule 4", "Node 1");
	}

	#[test]
	fn test_rule_4_optional_encoded_as_none() {
		// A section's title is optional, and an absent one is omitted, never written as a none.
		let tree = node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("T".to_string())),
			("lang",	Dat::Str("en".to_string())),
			(KEY_CHILDREN, Dat::List(vec![
				node(NodeKind::Section, map(vec![
					("title",	Dat::Opt(Box::new(None))),
					(KEY_CHILDREN,	Dat::List(vec![
						node(NodeKind::Para, map(vec![
							(KEY_CHILDREN, Dat::List(vec![text("p")])),
						])),
					])),
				])),
			])),
		]));
		rejects(&tree, "rule 4", "Node 1");
	}

	#[test]
	fn test_rule_4_optional_wrapped_in_some() {
		let tree = node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("T".to_string())),
			("lang",	Dat::Str("en".to_string())),
			(KEY_CHILDREN, Dat::List(vec![
				node(NodeKind::Boxx, map(vec![
					("style",	Dat::Opt(Box::new(Some(Dat::Str("note".to_string()))))),
					(KEY_CHILDREN,	Dat::List(vec![
						node(NodeKind::Para, map(vec![
							(KEY_CHILDREN, Dat::List(vec![text("p")])),
						])),
					])),
				])),
			])),
		]));
		rejects(&tree, "rule 4", "Node 1");
	}

	#[test]
	fn test_rule_4_empty_children_list() {
		let tree = doc_with_heading(map(vec![
			("level",	Dat::U8(2)),
			(KEY_CHILDREN,	Dat::List(Vec::new())),
		]));
		rejects(&tree, "rule 4", "Node 1");
	}

	#[test]
	fn test_rule_4_children_on_a_childless_kind() {
		let tree = node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("T".to_string())),
			("lang",	Dat::Str("en".to_string())),
			(KEY_CHILDREN, Dat::List(vec![
				node(NodeKind::Image, map(vec![
					("hash",	Dat::BU8(vec![0x01])),
					("alt",	Dat::Str("a".to_string())),
					(KEY_CHILDREN,	Dat::List(vec![text("x")])),
				])),
			])),
		]));
		rejects(&tree, "rule 4", "Node 1");
	}

	#[test]
	fn test_rule_5_control_character_in_text() {
		let tree = doc_with_heading(map(vec![
			("level",	Dat::U8(2)),
			(KEY_CHILDREN,	Dat::List(vec![text("a carriage\rreturn")])),
		]));
		rejects(&tree, "rule 5", "Node 2");
	}

	#[test]
	fn test_rule_5_decomposed_text_is_not_canonical() {
		// "café" with a combining acute accent: it displays exactly as the composed form does, and
		// would hash differently, so one document would have two addresses.
		let tree = doc_with_heading(map(vec![
			("level",	Dat::U8(2)),
			(KEY_CHILDREN,	Dat::List(vec![text("cafe\u{0301}")])),
		]));
		rejects(&tree, "rule 5", "Node 2");
	}

	#[test]
	fn test_rule_5_composed_text_is_canonical() -> Outcome<()> {
		// The same word, composed. This is the one encoding the format accepts.
		let tree = doc_with_heading(map(vec![
			("level",	Dat::U8(2)),
			(KEY_CHILDREN,	Dat::List(vec![text("caf\u{00E9}")])),
		]));
		res!(check(&tree));
		Ok(())
	}

	#[test]
	fn test_rule_5_decomposed_text_in_a_field() {
		// A node with no children omits the key (rule 4), so this doc carries none.
		let tree = node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("cafe\u{0301}".to_string())),
			("lang",	Dat::Str("en".to_string())),
		]));
		rejects(&tree, "rule 5", "Node 0");
	}

	#[test]
	fn test_rule_5_control_character_in_a_field() {
		let tree = node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("a bell\u{0007}".to_string())),
			("lang",	Dat::Str("en".to_string())),
			(KEY_CHILDREN, Dat::List(vec![
				node(NodeKind::Para, map(vec![
					(KEY_CHILDREN, Dat::List(vec![text("p")])),
				])),
			])),
		]));
		rejects(&tree, "rule 5", "Node 0");
	}

	#[test]
	fn test_rule_5_tab_and_newline_are_permitted() -> Outcome<()> {
		res!(check_string("a tab\tand a newline\n"));
		Ok(())
	}

	#[test]
	fn test_rule_6_wrong_integer_width() {
		let tree = doc_with_heading(map(vec![
			("level",	Dat::U32(2)),
			(KEY_CHILDREN,	Dat::List(vec![text("h")])),
		]));
		rejects(&tree, "rules 1 and 6", "Node 1");
	}

	#[test]
	fn test_rule_6_wrong_byte_string_width() {
		let tree = node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("T".to_string())),
			("lang",	Dat::Str("en".to_string())),
			(KEY_CHILDREN, Dat::List(vec![
				node(NodeKind::Image, map(vec![
					("hash",	Dat::BU16(vec![0x01, 0x02])),
					("alt",	Dat::Str("a".to_string())),
				])),
			])),
		]));
		rejects(&tree, "rules 1 and 6", "Node 1");
	}

	#[test]
	fn test_rule_7_vek_children() {
		let vek = match Vek::try_from(vec![text("h")]) {
			Ok(vek) => vek,
			Err(_) => {
				assert!(false, "Could not build a Vek.");
				return;
			},
		};
		let tree = doc_with_heading(map(vec![
			("level",	Dat::U8(2)),
			(KEY_CHILDREN,	Dat::Vek(vek)),
		]));
		rejects(&tree, "rule 7", "Node 1");
	}

	#[test]
	fn test_hash32_field_carrying_a_bu8() {
		// A content hash is a b32 (§4.2); a variable-length bu8 of the same bytes would encode the
		// reference two ways, so canon pins the width.
		let tree = node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("T".to_string())),
			("lang",	Dat::Str("en".to_string())),
			(KEY_CHILDREN, Dat::List(vec![
				node(NodeKind::Image, map(vec![
					("hash",	Dat::BU8(vec![0x01, 0x02, 0x03])),
					("alt",	Dat::Str("a".to_string())),
				])),
			])),
		]));
		rejects(&tree, "rules 1 and 6", "Node 1");
	}

	#[test]
	fn test_i8_style_value_carrying_a_u8() {
		// A style record's size is a scale step, an i8 (§4.4); a u8 of the same value is a second
		// encoding, so canon pins the width even though both are non-negative.
		let tree = node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("T".to_string())),
			("lang",	Dat::Str("en".to_string())),
			(KEY_STYLES, map(vec![
				("lede", map(vec![
					("size",	Dat::U8(1)),
				])),
			])),
			(KEY_CHILDREN, Dat::List(vec![
				node(NodeKind::Para, map(vec![
					(KEY_CHILDREN,	Dat::List(vec![text("p")])),
				])),
			])),
		]));
		rejects(&tree, "rules 1 and 6", "Node 0");
	}

	#[test]
	fn test_styles_table_with_an_ordmap() {
		// The style table is a Dat::Map, whose order follows its keys, never a Dat::OrdMap.
		let styles = create_dat_ordmap(vec![
			(Dat::Str("lede".to_string()), map(vec![("size", Dat::I8(1))])),
		]);
		let tree = node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("T".to_string())),
			("lang",	Dat::Str("en".to_string())),
			(KEY_STYLES,	styles),
			(KEY_CHILDREN, Dat::List(vec![
				node(NodeKind::Para, map(vec![
					(KEY_CHILDREN,	Dat::List(vec![text("p")])),
				])),
			])),
		]));
		rejects(&tree, "rule 2", "Node 0");
	}

	#[test]
	fn test_address_map_with_a_non_string_name() {
		// A link address's name is a str (§4.3); a u8 there is a wrong width, named at the link node.
		let tree = node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("T".to_string())),
			("lang",	Dat::Str("en".to_string())),
			(KEY_CHILDREN, Dat::List(vec![
				node(NodeKind::Para, map(vec![
					(KEY_CHILDREN, Dat::List(vec![
						node(NodeKind::Link, map(vec![
							("to",	map(vec![("name", Dat::U8(1))])),
							(KEY_CHILDREN,	Dat::List(vec![text("x")])),
						])),
					])),
				])),
			])),
		]));
		rejects(&tree, "rules 1 and 6", "Node 2");
	}

	#[test]
	fn test_unknown_kind_is_canonicalised() -> Outcome<()> {
		// Canon does not reject an unknown kind (§4.5): it canonicalises its bytes, recurses its
		// fallback of known nodes, and leaves the legality of the kind to the validator.
		let tree = node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("T".to_string())),
			("lang",	Dat::Str("en".to_string())),
			(KEY_CHILDREN, Dat::List(vec![
				Dat::Usr(
					UsrKindId::new(20, Some("table"), None),
					Some(Box::new(map(vec![
						(KEY_FALLBACK, Dat::List(vec![
							node(NodeKind::Para, map(vec![
								(KEY_CHILDREN, Dat::List(vec![text("Q1 revenue")])),
							])),
						])),
					]))),
				),
			])),
		]));
		res!(check(&tree));
		Ok(())
	}

	#[test]
	fn test_unknown_kind_non_canonical_field_rejected() {
		// An unknown kind's other fields are not interpreted, but are still held to §3: an OrdMap in
		// one is rejected even though canon does not know the field's width.
		let field = create_dat_ordmap(vec![
			(Dat::Str("a".to_string()), Dat::U8(1)),
		]);
		let tree = node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("T".to_string())),
			("lang",	Dat::Str("en".to_string())),
			(KEY_CHILDREN, Dat::List(vec![
				Dat::Usr(
					UsrKindId::new(20, Some("table"), None),
					Some(Box::new(map(vec![
						("rows",	field),
						(KEY_FALLBACK, Dat::List(vec![
							node(NodeKind::Para, map(vec![
								(KEY_CHILDREN, Dat::List(vec![text("Q1")])),
							])),
						])),
					]))),
				),
			])),
		]));
		rejects(&tree, "rule 2", "Node 1");
	}

	#[test]
	fn test_unknown_kind_tuple_hides_control_char() {
		// The uninterpreted field is a tuple, not a map, and it hides a string with a carriage
		// return. The catch-all once let it through; check_struct must recurse the tuple (§4.5, §3
		// rule 5).
		let field = Dat::Tup2(Box::new([
			Dat::Str("bad\rstring".to_string()),
			Dat::U8(0),
		]));
		let tree = node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("T".to_string())),
			("lang",	Dat::Str("en".to_string())),
			(KEY_CHILDREN, Dat::List(vec![
				Dat::Usr(
					UsrKindId::new(20, Some("table"), None),
					Some(Box::new(map(vec![
						("rows",	field),
						(KEY_FALLBACK, Dat::List(vec![
							node(NodeKind::Para, map(vec![
								(KEY_CHILDREN, Dat::List(vec![text("Q1")])),
							])),
						])),
					]))),
				),
			])),
		]));
		rejects(&tree, "rule 5", "Node 1");
	}

	#[test]
	fn test_empty_styles_table_rejected() {
		// An empty style table renders like an absent one, so accepting it would give one document
		// two addresses.
		let tree = node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("T".to_string())),
			("lang",	Dat::Str("en".to_string())),
			("styles",	map(vec![])),
			(KEY_CHILDREN, Dat::List(vec![
				node(NodeKind::Para, map(vec![
					(KEY_CHILDREN, Dat::List(vec![text("x")])),
				])),
			])),
		]));
		rejects(&tree, "empty", "Node 0");
	}

	#[test]
	fn test_empty_style_record_rejected() {
		// A style that sets no property has no effect, the same two-address trap.
		let tree = node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("T".to_string())),
			("lang",	Dat::Str("en".to_string())),
			("styles",	map(vec![("x", map(vec![]))])),
			(KEY_CHILDREN, Dat::List(vec![
				node(NodeKind::Para, map(vec![
					(KEY_CHILDREN, Dat::List(vec![text("x")])),
				])),
			])),
		]));
		rejects(&tree, "empty", "Node 0");
	}

	#[test]
	fn test_root_that_is_not_a_node() {
		match check(&Dat::Str("not a tree".to_string())) {
			Ok(()) => assert!(false, "A tree that is not a node was accepted."),
			Err(e) => {
				let msg = fmt!("{}", e);
				assert!(msg.contains("Node 0"), "Expected the node id in the error: {}", msg);
			},
		}
	}

	#[test]
	fn test_trailing_bytes_rejected() -> Outcome<()> {
		let mut buf = res!(encode(&valid_doc()));
		buf.push(0x00);
		match decode(&buf) {
			Ok(_) => assert!(false, "Bytes trailing the tree were accepted."),
			Err(_) => (),
		}
		Ok(())
	}
}
