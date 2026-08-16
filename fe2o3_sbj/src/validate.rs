//! Schema validation and the limits that are not the decoder's. See `SPEC.md` §4, §5.
//!
//! The decoder has already enforced the depth limit and the tree region size before anything gets
//! here, so what remains is the node schema, the node count, and the rules that only make sense
//! once a tree exists: the root is a `doc`, every kind code is one the schema admits, every field is
//! present and correctly typed, and every child is one its parent admits.
//!
//! The schema is a vocabulary and not a constant. A document (`oxeweb/doc/0`) admits the kinds 1 to
//! 13; the browser's chrome admits those and the `edit` node; an application's tree admits those and
//! the `surface` node as well. One walk validates all three, and which kinds it admits is
//! [`Schema::admits`] and nothing else, so "a document is never a program" is a fact about this
//! function rather than a promise made elsewhere.
//!
//! The style vocabulary is the schema's in exactly the same way ([`Schema::admits_style`]): a
//! document admits the eight properties of §4.4, and a chrome and an application admit more,
//! because the chrome is a real interface and needs to look like one. The browser's own trees are
//! held to this walk like any document, so the chrome is legal because it conforms, and not because
//! nobody checked it.
//!
//! Nodes are numbered by a depth-first, pre-order walk from 0 at the root (`SPEC.md` §4.6), and
//! every rejection names the node, its kind, and the rule it broke (`SPEC.md` §6).

use crate::{
	kinds::{
		check_address,
		check_border,
		icon_names_label,
		known_icon,
		known_style_field,
		Content,
		Field,
		FieldType,
		NodeKind,
		ReservedKind,
		Schema,
		StyleCheck,
		StyleField,
		ALIGNMENTS,
		DIRECTIONS,
		PALETTE,
		KEY_ALT,
		KEY_CHILDREN,
		KEY_FALLBACK,
		KEY_STYLE,
		KEY_STYLES,
	},
	limit,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

/// What a validated tree turned out to contain.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
	/// Number of nodes.
	pub nodes:	usize,
	/// Greatest nesting depth reached. The root alone is a depth of 1.
	pub depth:	usize,
}

/// Validates a decoded tree against the vocabulary its schema admits, naming the failing node.
///
/// The walk is iterative rather than recursive, so a tree that arrived by some route other than the
/// decoder, and therefore never met the decoder's depth limit, cannot exhaust the stack here. It
/// walks the tree by reference for the same reason: cloning a node clones every node under it, and
/// the clone a derived implementation makes recurses as deep as the tree goes, which would spend
/// the stack this walk was written to save.
pub fn validate(tree: &Dat, schema: &str) -> Outcome<Stats> {

	let schema = res!(Schema::from_name(schema));

	let mut stats = Stats::default();
	let mut id: usize = 0;

	// How many surfaces the tree has opened, against the ceiling of §5. A surface is a live
	// application instance, and a tree that may open unboundedly many is a tree that exhausts the host
	// by being opened.
	let mut surfaces: usize = 0;

	// The names the document's style table defines, filled in when the root doc is reached (§4.4). A
	// node's `style` field must name one of these, and since the root is popped before any of its
	// descendants, the table is known before any style reference is checked.
	let mut styles: Vec<String> = Vec::new();

	// The names some node actually references. A style defined but never used renders identically to
	// one that was never defined, so an unreferenced entry would give one document two addresses; it
	// is caught after the walk, once every reference is known.
	let mut referenced: Vec<String> = Vec::new();

	// A pending node: the daticle, the kind of its parent, its depth, and whether it sits inside a
	// surface's alternative. Children are pushed in reverse, so popping yields the depth-first
	// pre-order that numbers the nodes.
	//
	// The `inert` flag is what makes an alternative inert. It is set on the nodes of an alternative
	// and carried down to everything beneath them, so no schema, and no fallback, can put an `edit` or
	// a `surface` inside the content that stands in for an application. An alternative that could
	// itself hold a surface would be a hole with another hole behind it.
	let mut stack: Vec<(&Dat, Option<NodeKind>, usize, bool)> = vec![(tree, None, 1, false)];

	while let Some((node, parent, depth, inert)) = stack.pop() {

		if stats.nodes == limit::NODES {
			return Err(err!(
				"Node {}: the tree exceeds the limit of {} nodes.", id, limit::NODES;
			Invalid, Input, TooBig, LimitReached));
		}
		stats.nodes += 1;
		if depth > stats.depth {
			stats.depth = depth;
		}

		// A node is a usr daticle: a kind code, then a payload.
		let (uid, payload_opt) = match node {
			Dat::Usr(uid, payload_opt) => (uid, payload_opt),
			d => return Err(err!(
				"Node {} is a {} daticle; every node is a usr daticle carrying a kind code.",
				id, d.kind();
			Invalid, Input)),
		};

		let kind = match NodeKind::from_code(uid.code()) {
			Ok(kind) => kind,
			Err(_) => {
				// SPEC §4.2 and §4.5: a code the reader KNOWS, and the schema does not ADMIT, is
				// refused unconditionally. It is checked before the fallback rule below and never
				// falls through to it, because the fallback rule is forward compatibility for a code
				// this version has never heard of, and this is not that: the reader knows exactly what
				// it has been handed. Admitting a reserved kind on a fallback would let an author put
				// a surface in a document today, under a fallback that renders innocently, and have
				// every reader that later learned what code 15 meant begin honouring it -- a document
				// that became a program by waiting. Anyone who later "simplifies" these two paths back
				// into one reopens that.
				if let Some(reserved) = ReservedKind::from_code(uid.code()) {
					res!(check_reserved(
						id, reserved, schema, parent, payload_opt, depth, inert,
						&mut surfaces, &styles, &mut referenced, &mut stack));
					id += 1;
					continue;
				}
				// SPEC §4.5: a kind outside the vocabulary is permitted only when its payload is a
				// map carrying a non-empty `fallback` of known nodes, which stand in for it. The
				// root is never one, since it must be a doc.
				if id == 0 {
					return Err(err!(
						"Node 0 declares the unknown kind code {}; the root of an '{}' payload \
						must be a doc.", uid.code(), schema.name();
					Invalid, Input));
				}
				res!(push_fallback(id, uid.code(), parent, payload_opt, depth, inert, &mut stack));
				id += 1;
				continue;
			},
		};

		// The root of a payload is always a doc, whichever schema it declares. A chrome and an
		// application are documents that may say more, not trees of another shape.
		if id == 0 && kind != NodeKind::Doc {
			return Err(err!(
				"Node 0 is a {}; the root of an '{}' payload must be a doc.",
				kind.label(), schema.name();
			Invalid, Input));
		}

		// A child appears only where its parent admits it.
		if let Some(pkind) = parent {
			if !pkind.allows(&kind) {
				return Err(err!(
					"Node {} is a {} inside a {}, which admits {} content only.",
					id, kind.label(), pkind.label(), content_label(pkind.content());
				Invalid, Input));
			}
		}

		let payload = match payload_opt {
			Some(payload) => payload.as_ref(),
			None => return Err(err!(
				"Node {} ({}) carries no payload; a {} carries {}.",
				id, kind.label(), kind.label(), payload_label(kind);
			Invalid, Input, Missing)),
		};

		// Every payload is a map, except a text run's, which is the string itself.
		if kind.payload_is_str() {
			match payload {
				Dat::Str(_) => (),
				d => return Err(err!(
					"Node {} ({}) carries a {} payload; a {} carries a str.",
					id, kind.label(), d.kind(), kind.label();
				Invalid, Input)),
			}
		} else {
			match payload {
				Dat::Map(map) => {
					res!(check_fields(
						id, kind.label(), kind.fields(), kind.content(), kind == NodeKind::Doc, map));
					// A heading's level is the one field with a range as well as a width.
					if kind == NodeKind::Heading {
						res!(check_heading_level(id, map));
					}
					// The root doc's style table is validated once, and its names collected so
					// that every later `style` reference can be resolved against them (§4.4). The
					// schema goes with it: which properties a record may name is the schema's
					// business, exactly as which kinds the tree may carry is.
					if kind == NodeKind::Doc {
						styles = res!(check_styles_table(id, schema, map));
					}
					// A node naming a style must name one the table defines (§4.4).
					if let Some(sv) = map.get(&dat!(KEY_STYLE)) {
						res!(check_style_ref(id, kind.label(), sv, &styles));
						if let Dat::Str(s) = sv {
							referenced.push(s.clone());
						}
					}
				},
				d => return Err(err!(
					"Node {} ({}) carries a {} payload; a {} carries a map.",
					id, kind.label(), d.kind(), kind.label();
				Invalid, Input)),
			}
		}

		let kids = match children(payload) {
			Ok(kids) => kids,
			Err(e) => return Err(err!(e,
				"Node {} ({}): its children must be a list.", id, kind.label();
			Invalid, Input)),
		};
		// SPEC §4.2: a list is marked `item+` and carries at least one child. A doc and a section are
		// `flow*` and may be empty, since a stub with only a title is a legitimate thing to publish.
		if kind.requires_child() && kids.is_empty() {
			return Err(err!(
				"Node {} ({}) carries no children, but a {} must carry at least one.",
				id, kind.label(), kind.label();
			Invalid, Input, Missing));
		}
		for kid in kids.iter().rev() {
			stack.push((kid, Some(kind), depth + 1, inert));
		}

		id += 1;
	}

	// Every defined style must be used. An unreferenced entry has no rendering effect, so a document
	// carrying it and one without it are the same document at two addresses (§4.4).
	for name in &styles {
		if !referenced.contains(name) {
			return Err(err!(
				"Node 0 (doc): style '{}' is defined in the table but no node references it, which \
				would give one document two addresses; remove it (SPEC.md §4.4).", name;
			Invalid, Input));
		}
	}

	Ok(stats)
}

/// The children a node payload carries, borrowed rather than cloned.
///
/// This is [`children_of`](crate::kinds::children_of) without the clone, which matters because a
/// validator walks every node of a tree that arrived from somewhere else.
fn children(payload: &Dat) -> Outcome<&[Dat]> {
	match payload {
		Dat::Map(map) => match map.get(&dat!(KEY_CHILDREN)) {
			None => Ok(&[]),
			Some(Dat::List(v)) => Ok(&v[..]),
			Some(d) => Err(err!(
				"Node children must be a list, found {:?}.", d.kind();
			Invalid, Input)),
		},
		_ => Ok(&[]),
	}
}

/// Checks a node's payload map: no unknown keys, every field correctly typed, every required field
/// present.
///
/// The kind is passed as its label, its field table and its content class rather than as a
/// `NodeKind`, because a reserved kind (§4.2) has a payload schema of its own and is held to it by
/// exactly this routine. One check, and no second implementation to drift from the first.
fn check_fields(
	id:	usize,
	label:	&str,
	fields:	&'static [Field],
	content:	Content,
	table:	bool,
	map:	&DaticleMap,
)
	-> Outcome<()>
{
	// Every key present is either a declared field or the children list.
	for (k, v) in map.iter() {
		let key = match k {
			Dat::Str(s) => s.as_str(),
			d => return Err(err!(
				"Node {} ({}) has a map key of kind {}; every map key is a str.",
				id, label, d.kind();
			Invalid, Input)),
		};
		if key == KEY_CHILDREN {
			if content == Content::None {
				return Err(err!(
					"Node {} ({}) carries a '{}' field, but a {} admits no children.",
					id, label, KEY_CHILDREN, label;
				Invalid, Input));
			}
			continue;
		}
		// The universal `style` field (§4.4) is legal on any map payload and is resolved against the
		// document's style table elsewhere, so it is never an unknown field.
		if key == KEY_STYLE {
			continue;
		}
		// The `styles` table is the doc node's own field (§4.4), validated separately; on any other
		// kind it falls through and is rejected as unknown below.
		if key == KEY_STYLES && table {
			continue;
		}
		match fields.iter().find(|f| f.name == key) {
			Some(f) => res!(check_field(id, label, f, v)),
			None => return Err(err!(
				"Node {} ({}) carries the unknown field '{}'; a {} declares {}.",
				id, label, key, label, field_list(fields);
			Invalid, Input)),
		}
	}

	// Every required field is present.
	for f in fields {
		if !f.opt && map.get(&dat!(f.name)).is_none() {
			return Err(err!(
				"Node {} ({}) is missing the required field '{}' of type {}.",
				id, label, f.name, type_label(f.typ);
			Invalid, Input, Missing));
		}
	}

	Ok(())
}

/// Checks one field's value against the type its schema declares for it.
fn check_field(
	id:	usize,
	label:	&str,
	f:	&Field,
	v:	&Dat,
)
	-> Outcome<()>
{
	// An address is a typed sub-structure rather than a scalar, so it is checked by its own routine,
	// which tells a name from a hash and refuses a malformed target (§4.3).
	if f.typ == FieldType::Address {
		return match check_address(v) {
			Ok(_) => Ok(()),
			Err(e) => Err(err!(e,
				"Node {} ({}) field '{}' is not a valid link address.",
				id, label, f.name;
			Invalid, Input)),
		};
	}

	// A field carrying nodes is a list the walk will validate, node by node, like any other. What is
	// checked here is that it is a list at all, and that it is not empty: a `surface`'s alternative is
	// what a screen reader reads, what a search indexes, and what the reader sees when the application
	// is not running, so an empty one is a hole in the document with nothing behind it (§4.2).
	if f.typ == FieldType::Nodes {
		return match v {
			Dat::List(list) if !list.is_empty() => Ok(()),
			Dat::List(_) => Err(err!(
				"Node {} ({}) field '{}' is an empty list; it carries at least one node. An \
				alternative with nothing in it is a hole with nothing behind it (SPEC.md §4.2).",
				id, label, f.name;
			Invalid, Input, Missing)),
			d => Err(err!(
				"Node {} ({}) field '{}' is a {} daticle; the schema declares {}.",
				id, label, f.name, d.kind(), type_label(f.typ);
			Invalid, Input, Mismatch)),
		};
	}

	let typed = match (f.typ, v) {
		(FieldType::Str,	Dat::Str(_))	=> true,
		(FieldType::U8,	Dat::U8(_))	=> true,
		(FieldType::I8,	Dat::I8(_))	=> true,
		(FieldType::U32,	Dat::U32(_))	=> true,
		(FieldType::Bool,	Dat::Bool(_))	=> true,
		(FieldType::Hash32,	Dat::B32(_))	=> true,
		_	=> false,
	};
	if !typed {
		return Err(err!(
			"Node {} ({}) field '{}' is a {} daticle; the schema declares {}.",
			id, label, f.name, v.kind(), type_label(f.typ);
		Invalid, Input, Mismatch));
	}

	Ok(())
}

/// Checks a heading's level, the one field with a range as well as a width: it runs from 1 to 6.
///
/// The width is already pinned by [`check_field`], so a level that is here at all is a `u8`.
fn check_heading_level(
	id:	usize,
	map:	&DaticleMap,
)
	-> Outcome<()>
{
	if let Some(Dat::U8(level)) = map.get(&dat!("level")) {
		if *level < 1 || *level > 6 {
			return Err(err!(
				"Node {} (heading) has level {}, which is outside the range 1..=6.", id, level;
			Invalid, Input, Range));
		}
	}
	Ok(())
}

/// The schema name of a field type, as an error message spells it.
fn type_label(typ: FieldType) -> &'static str {
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

/// The name of a content class, as an error message spells it.
fn content_label(content: Content) -> &'static str {
	match content {
		Content::None	=> "no",
		Content::Flow	=> "flow",
		Content::Inline	=> "inline",
		Content::Items	=> "item",
	}
}

/// What a kind's payload is, as an error message spells it.
fn payload_label(kind: NodeKind) -> &'static str {
	if kind.payload_is_str() {
		"a str"
	} else {
		"a map"
	}
}

/// The fields a kind declares, listed for an error message.
fn field_list(fields: &'static [Field]) -> String {
	if fields.is_empty() {
		return fmt!("no fields");
	}
	let mut s = String::new();
	for (i, f) in fields.iter().enumerate() {
		if i > 0 {
			s.push_str(", ");
		}
		s.push_str(&fmt!("'{}': {}{}",
			f.name,
			type_label(f.typ),
			if f.opt { "?" } else { "" },
		));
	}
	s
}

/// Validates a node of a reserved kind (§4.2), and pushes a surface's alternative onto the walk.
///
/// The refusals here are the security boundary of the whole format, and they are ordered so that the
/// strongest one is reached first. A reserved kind inside an alternative is refused whatever the
/// schema; a reserved kind the schema does not admit is refused whatever it carries; and only then is
/// what it carries looked at all.
fn check_reserved<'a>(
	id:		usize,
	reserved:	ReservedKind,
	schema:		Schema,
	parent:		Option<NodeKind>,
	payload_opt:	&'a Option<Box<Dat>>,
	depth:		usize,
	inert:		bool,
	surfaces:	&mut usize,
	styles:		&[String],
	referenced:	&mut Vec<String>,
	stack:		&mut Vec<(&'a Dat, Option<NodeKind>, usize, bool)>,
)
	-> Outcome<()>
{
	// An alternative is inert content, always. It stands in for an application that is not running, so
	// a live thing inside it would be a hole with another hole behind it, and a surface that could
	// nest could open one instance per alternative, per instance, without end.
	if inert {
		return Err(err!(
			"Node {}: a surface's alternative may not carry {} node. The alternative is what a \
			screen reader reads and what the reader sees when the application is not running, so it \
			carries inert content only, whatever the schema '{}' admits elsewhere (SPEC.md §4.2).",
			id, reserved.with_article(), schema.name();
		Invalid, Input, Security));
	}

	// SPEC §4.2 and §4.5: a code the reader knows, and the schema does not admit, is refused
	// unconditionally, fallback or no fallback. This is not the fallback rule's business and never
	// falls through to it.
	if !schema.admits(reserved) {
		return Err(err!(
			"Node {}: the kind code {} is the reserved kind '{}', which is legal in {}. The schema \
			'{}' admits {}, so {} may not carry {} node, whether or not it carries a fallback \
			(SPEC.md §4.2, §4.5).",
			id, reserved.code(), reserved.label(), reserved.legal_in(),
			schema.name(), schema.admits_label(), schema.tree_label(), reserved.with_article();
		Invalid, Input, Security));
	}

	// Where a reserved kind may sit is its own business (§4.2): a field and a pane are blocks and go
	// where a box goes, and an icon is a glyph and goes where a text run goes. The root is never one,
	// since the root is always a doc.
	let pkind = match parent {
		Some(pkind) => pkind,
		None => return Err(err!(
			"Node 0 is {}; the root of an '{}' payload must be a doc.",
			reserved.with_article(), schema.name();
		Invalid, Input)),
	};
	if pkind.content() != reserved.sits_in() {
		return Err(err!(
			"Node {} is {} inside a {}, which admits {} content only. {} is {} content.",
			id, reserved.with_article(), pkind.label(), content_label(pkind.content()),
			reserved.with_article(), content_label(reserved.sits_in());
		Invalid, Input));
	}

	let payload = match payload_opt {
		Some(payload) => payload.as_ref(),
		None => return Err(err!(
			"Node {} ({}) carries no payload; {} carries a map.",
			id, reserved.label(), reserved.with_article();
		Invalid, Input, Missing)),
	};
	let map = match payload {
		Dat::Map(map) => map,
		d => return Err(err!(
			"Node {} ({}) carries a {} payload; {} carries a map.",
			id, reserved.label(), d.kind(), reserved.with_article();
		Invalid, Input)),
	};

	// Neither reserved kind carries children, and each has its own reason. The generic refusal in
	// `check_fields` would catch this too; it is caught here so that the refusal says which reason.
	if map.get(&dat!(KEY_CHILDREN)).is_some() {
		return Err(err!(
			"Node {} ({}) carries a '{}' key, and takes none: {} (SPEC.md §4.2).",
			id, reserved.label(), KEY_CHILDREN, reserved.no_children();
		Invalid, Input));
	}

	res!(check_fields(
		id, reserved.label(), reserved.fields(), reserved.content(), false, map));

	// The universal `style` field (§4.4) is legal on a reserved kind's map payload like any other.
	if let Some(sv) = map.get(&dat!(KEY_STYLE)) {
		res!(check_style_ref(id, reserved.label(), sv, styles));
		if let Dat::Str(s) = sv {
			referenced.push(s.clone());
		}
	}

	if reserved == ReservedKind::Icon {
		// The set is closed (§4.2). An icon names the engine's own drawing rather than carrying any,
		// so a name the engine does not draw has nothing faithful to stand in for it, and is a fault
		// in whoever built the tree rather than a gap a reader should paper over.
		let name = match map.get(&dat!("name")) {
			Some(Dat::Str(s)) => s.clone(),
			// `check_fields` has already established that the field is present and is a string, so
			// this arm is unreachable, and it is written out rather than assumed.
			_ => return Err(err!(
				"Node {} (icon): the 'name' field is not the string the schema requires, and the \
				field check did not catch it. This is a fault in the validator.", id;
			Bug, Invalid)),
		};
		if !known_icon(&name) {
			return Err(err!(
				"Node {} (icon): '{}' is not an icon this engine draws. The set is closed at '{}' \
				(SPEC.md §4.2).", id, name, icon_names_label();
			Invalid, Input, Mismatch));
		}
	}

	if reserved == ReservedKind::Surface {
		*surfaces += 1;
		if *surfaces > limit::SURFACES {
			return Err(err!(
				"Node {} (surface): the tree carries more than {} surfaces, which is the ceiling \
				(SPEC.md §5). Every surface is a live application instance the host must lay out, \
				budget and present.", id, limit::SURFACES;
			Invalid, Input, TooBig, LimitReached));
		}
		// The alternative is walked as ordinary content, under the surface's own parent, so its nodes
		// meet exactly the child rules that governed where the surface sat, and are counted and
		// numbered like any others. They are marked inert: nothing beneath them may come alive.
		let alt = match map.get(&dat!(KEY_ALT)) {
			Some(Dat::List(alt)) => alt,
			// `check_fields` has already established that the field is present and is a non-empty
			// list, so this arm is unreachable, and it is written out rather than assumed.
			_ => return Err(err!(
				"Node {} (surface): the '{}' field is not the non-empty list of nodes the schema \
				requires, and the field check did not catch it. This is a fault in the validator.",
				id, KEY_ALT;
			Bug, Invalid)),
		};
		for kid in alt.iter().rev() {
			stack.push((kid, parent, depth + 1, true));
		}
	}

	Ok(())
}

/// Validates an unknown kind's fallback and pushes its children onto the walk (§4.5).
///
/// An unknown kind is permitted only when its payload is a map carrying a non-empty `fallback` list,
/// which the reader renders and validates in the unknown node's place. The children are pushed under
/// the unknown node's own parent, so they meet exactly the child rules that governed where the
/// unknown node sat, and they are counted and numbered as ordinary nodes when the walk reaches them.
/// The unknown kind's other fields are not interpreted here.
///
/// A fallback inside an alternative stays inert, so a kind the reader has never heard of cannot smuggle
/// a live node into the content that stands in for an application.
fn push_fallback<'a>(
	id:		usize,
	code:		u16,
	parent:		Option<NodeKind>,
	payload_opt:	&'a Option<Box<Dat>>,
	depth:		usize,
	inert:		bool,
	stack:		&mut Vec<(&'a Dat, Option<NodeKind>, usize, bool)>,
)
	-> Outcome<()>
{
	let payload = match payload_opt {
		Some(payload) => payload.as_ref(),
		None => return Err(err!(
			"Node {}: the unknown kind code {} carries no payload; an unknown kind is permitted \
			only when its payload is a map carrying a non-empty '{}' (SPEC.md §4.5).",
			id, code, KEY_FALLBACK;
		Invalid, Input, Missing)),
	};
	let map = match payload {
		Dat::Map(map) => map,
		d => return Err(err!(
			"Node {}: the unknown kind code {} carries a {} payload; an unknown kind is permitted \
			only when its payload is a map carrying a non-empty '{}' (SPEC.md §4.5).",
			id, code, d.kind(), KEY_FALLBACK;
		Invalid, Input)),
	};
	let list = match map.get(&dat!(KEY_FALLBACK)) {
		Some(Dat::List(list)) if !list.is_empty() => list,
		Some(Dat::List(_)) => return Err(err!(
			"Node {}: the unknown kind code {} carries an empty '{}'; an unknown kind is permitted \
			only with a non-empty fallback of known nodes (SPEC.md §4.5).",
			id, code, KEY_FALLBACK;
		Invalid, Input)),
		Some(d) => return Err(err!(
			"Node {}: the unknown kind code {} carries a '{}' of kind {}; a fallback is a \
			non-empty list of known nodes (SPEC.md §4.5).",
			id, code, KEY_FALLBACK, d.kind();
		Invalid, Input)),
		None => return Err(err!(
			"Node {}: the unknown kind code {} carries no '{}'; an unknown kind is permitted only \
			when its payload is a map carrying a non-empty fallback of known nodes (SPEC.md §4.5).",
			id, code, KEY_FALLBACK;
		Invalid, Input, Missing)),
	};
	for child in list.iter().rev() {
		stack.push((child, parent, depth + 1, inert));
	}
	Ok(())
}

/// Validates the doc node's style table, returning the style names it defines (§4.4).
///
/// The table is a map from style name to style record. Each key must be a str, and each value a
/// record whose properties are all known and correctly valued. An absent table is simply an empty
/// set of names.
fn check_styles_table(
	id:	usize,
	schema:	Schema,
	map:	&DaticleMap,
)
	-> Outcome<Vec<String>>
{
	let table = match map.get(&dat!(KEY_STYLES)) {
		None => return Ok(Vec::new()),
		Some(Dat::Map(table)) => table,
		Some(d) => return Err(err!(
			"Node {} (doc): the '{}' style table is a map from name to style record, found a {} \
			(SPEC.md §4.4).", id, KEY_STYLES, d.kind();
		Invalid, Input)),
	};
	let mut names = Vec::with_capacity(table.len());
	for (k, v) in table.iter() {
		let name = match k {
			Dat::Str(s) => s.as_str(),
			d => return Err(err!(
				"Node {} (doc): the '{}' table has a key of kind {}; a style name is a str \
				(SPEC.md §4.4).", id, KEY_STYLES, d.kind();
			Invalid, Input)),
		};
		let record = match v {
			Dat::Map(record) => record,
			d => return Err(err!(
				"Node {} (doc): style '{}' is a {}; a style record is a map of style properties \
				(SPEC.md §4.4).", id, name, d.kind();
			Invalid, Input)),
		};
		res!(check_style_record(id, schema, name, record));
		names.push(name.to_string());
	}
	Ok(names)
}

/// Validates one style record: every property the schema admits, every value satisfying its check
/// (§4.4).
///
/// A property this schema does not admit and a property that does not exist are refused by the same
/// rule and named by different messages, because they are different mistakes. "Unknown style property
/// 'grid'" told to an author whose chrome draws a grid every day would be a lie, so the refusal says
/// which schema turned it down and where it would have been legal.
fn check_style_record(
	id:	usize,
	schema:	Schema,
	name:	&str,
	record:	&DaticleMap,
)
	-> Outcome<()>
{
	for (k, v) in record.iter() {
		let prop = match k {
			Dat::Str(s) => s.as_str(),
			d => return Err(err!(
				"Node {} (doc): style '{}' has a property key of kind {}; a style property name \
				is a str (SPEC.md §4.4).", id, name, d.kind();
			Invalid, Input)),
		};
		let field = match schema.style_field(prop) {
			Some(field) => field,
			// The property is a real one, and this schema does not admit it. A document naming an
			// interface property is refused here, and this is the whole of the reason: the v0 document
			// vocabulary is frozen at eight properties, so a document cannot dress as an interface, and
			// cannot grow into one by waiting for a reader that draws more.
			None => match known_style_field(prop) {
				Some(known) => return Err(err!(
					"Node {} (doc): style '{}' names the style property '{}', which is legal in {}. \
					The schema '{}' admits {}, so {} may not name it (SPEC.md §4.4).",
					id, name, prop, known.scope.legal_in(),
					schema.name(), schema.style_admits_label(), schema.tree_label();
				Invalid, Input)),
				None => return Err(err!(
					"Node {} (doc): style '{}' carries the unknown style property '{}' (SPEC.md \
					§4.4).", id, name, prop;
				Invalid, Input)),
			},
		};
		res!(check_style_value(id, name, field, v));
	}

	// `grid` and `pack` are two layouts, and a style naming both has asked for both. A renderer must
	// then pick one, and which it picks is a fact about that renderer rather than about the tree -- so
	// two readers could lay the same legal chrome out differently and both be right, which is the
	// thing a frozen format exists to prevent. Refusing the pair keeps the tree's meaning the tree's.
	//
	// This is the only rule here that reads a style record as a WHOLE rather than a property at a
	// time, because it is the only one about a combination.
	if record.contains_key(&dat!("grid")) && record.contains_key(&dat!("pack")) {
		return Err(err!(
			"Node {} (doc): style '{}' names both 'grid' and 'pack'. A grid shares its width out \
			among its tiles and a packed row leaves each at the width it names, so a style naming \
			both has asked for two different layouts and no reader can honour it (SPEC.md §4.4).",
			id, name;
		Invalid, Input));
	}
	Ok(())
}

/// Validates one style-record value against the enum or width its property declares (§4.4).
fn check_style_value(
	id:	usize,
	name:	&str,
	field:	&StyleField,
	v:	&Dat,
)
	-> Outcome<()>
{
	// A border is a typed sub-structure rather than a scalar, so it is checked by its own routine,
	// exactly as a link address is (§4.3), and a malformed one is refused here rather than misread by
	// the renderer.
	if field.check == StyleCheck::Border {
		return match check_border(v) {
			Ok(_) => Ok(()),
			Err(e) => Err(err!(e,
				"Node {} (doc): style '{}' property '{}' is not a valid border (SPEC.md §4.4).",
				id, name, field.name;
			Invalid, Input)),
		};
	}

	let ok = match field.check {
		StyleCheck::Palette	=> matches!(v, Dat::Str(s) if PALETTE.contains(&s.as_str())),
		StyleCheck::Direction	=> matches!(v, Dat::Str(s) if DIRECTIONS.contains(&s.as_str())),
		StyleCheck::Alignment	=> matches!(v, Dat::Str(s) if ALIGNMENTS.contains(&s.as_str())),
		StyleCheck::ScaleStep	=> matches!(v, Dat::I8(_)),
		StyleCheck::Spacing	=> matches!(v, Dat::U8(_)),
		StyleCheck::Lang	=> matches!(v, Dat::Str(_)),
		StyleCheck::Tile	=> matches!(v, Dat::U16(_)),
		StyleCheck::Share	=> matches!(v, Dat::U8(_)),
		StyleCheck::Elevation	=> matches!(v, Dat::U8(_)),
		// Checked above, and written out rather than caught by a wildcard, so that a check added later
		// and forgotten here is a compile error and not an accepted value.
		StyleCheck::Border	=> false,
	};
	if !ok {
		return Err(err!(
			"Node {} (doc): style '{}' property '{}' carries {}; the schema requires {} \
			(SPEC.md §4.4).",
			id, name, field.name, style_value_label(v), style_check_label(field.check);
		Invalid, Input));
	}
	Ok(())
}

/// A style reference names a str that resolves to an entry in the document's style table (§4.4).
fn check_style_ref(
	id:	usize,
	label:	&str,
	sv:	&Dat,
	styles:	&[String],
)
	-> Outcome<()>
{
	let name = match sv {
		Dat::Str(s) => s.as_str(),
		d => return Err(err!(
			"Node {} ({}): the '{}' field is a {}; a style reference is a str naming an entry in \
			the document's style table (SPEC.md §4.4).",
			id, label, KEY_STYLE, d.kind();
		Invalid, Input)),
	};
	if !styles.iter().any(|n| n == name) {
		return Err(err!(
			"Node {} ({}): the '{}' field names the style '{}', which the document's style table \
			does not define (SPEC.md §4.4).",
			id, label, KEY_STYLE, name;
		Invalid, Input));
	}
	Ok(())
}

/// Describes a style value for an error message, spelling out a string or scalar and naming a kind.
fn style_value_label(v: &Dat) -> String {
	match v {
		Dat::Str(s)	=> fmt!("the string \"{}\"", s),
		Dat::U8(n)	=> fmt!("the u8 {}", n),
		Dat::I8(n)	=> fmt!("the i8 {}", n),
		other	=> fmt!("a {} daticle", other.kind()),
	}
}

/// Names what a style check permits, for an error message.
fn style_check_label(check: StyleCheck) -> String {
	match check {
		StyleCheck::Palette	=> fmt!("a palette name ({})", PALETTE.join(", ")),
		StyleCheck::Direction	=> fmt!("a direction ({})", DIRECTIONS.join(", ")),
		StyleCheck::Alignment	=> fmt!("an alignment ({})", ALIGNMENTS.join(", ")),
		StyleCheck::ScaleStep	=> fmt!("a type scale step (i8)"),
		StyleCheck::Spacing	=> fmt!("a spacing index (u8)"),
		StyleCheck::Lang	=> fmt!("a language tag (str)"),
		StyleCheck::Tile	=> fmt!("a grid tile width as a percentage of a base size (u16)"),
		StyleCheck::Share	=> fmt!("a share of a packed row's leftover room (u8)"),
		StyleCheck::Elevation	=> fmt!("an elevation, in whole steps off the surface behind (u8)"),
		StyleCheck::Border	=> fmt!("a palette name and a width in pixels ([str, u8])"),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{
		SCHEMA_APP,
		SCHEMA_CHROME,
		SCHEMA_DOC,
	};

	use oxedyne_fe2o3_jdat::usr::UsrKindId;

	/// Builds a node of the given kind with the given payload.
	fn node(kind: NodeKind, payload: Dat) -> Dat {
		Dat::Usr(
			UsrKindId::new(kind.code(), Some(kind.label()), None),
			Some(Box::new(payload)),
		)
	}

	/// Builds a text run.
	fn text(s: &str) -> Dat {
		node(NodeKind::Text, dat!(s))
	}

	/// Builds a 32-byte content hash, the width of a v0 hash reference.
	fn hash32() -> Dat {
		Dat::B32(B32([0x9fu8; 32]))
	}

	/// A style may name a grid or a packed row, and not both.
	///
	/// The two are different layouts of the same children, so a style naming both leaves the reader to
	/// choose, and a legal tree whose appearance depends on which reader drew it is the thing a frozen
	/// format exists to prevent. Each ALONE is legal chrome, which is what says the pair is refused for
	/// being a pair rather than either being wrong.
	#[test]
	fn test_a_style_naming_both_a_grid_and_a_packed_row_is_refused_33() -> Outcome<()> {
		let tree = |props: Dat| -> Dat {
			node(NodeKind::Doc, mapdat!{
				"title" => dat!("A Title"),
				"lang" => dat!("en"),
				"styles" => mapdat!{ "shelf" => props },
				"children" => Dat::List(vec![
					node(NodeKind::Boxx, mapdat!{
						"style" => dat!("shelf"),
						"children" => Dat::List(vec![
							node(NodeKind::Para, mapdat!{ "children" => Dat::List(vec![text("a")]) }),
						]),
					}),
				]),
			})
		};
		// Either alone is legal chrome.
		res!(validate(&tree(mapdat!{ "grid" => dat!(1400u16) }), SCHEMA_CHROME));
		res!(validate(&tree(mapdat!{ "pack" => dat!(600u16) }), SCHEMA_CHROME));

		// Both together is not, in any schema that admits them at all.
		for schema in [SCHEMA_CHROME, SCHEMA_APP] {
			assert!(
				validate(&tree(mapdat!{ "grid" => dat!(1400u16), "pack" => dat!(600u16) }), schema).is_err(),
				"a style naming both a grid and a packed row is refused in {}", schema,
			);
		}
		// And a document may name neither, pair or no pair.
		assert!(
			validate(&tree(mapdat!{ "pack" => dat!(600u16) }), SCHEMA_DOC).is_err(),
			"a document may not name a packed row: it is an interface property",
		);
		Ok(())
	}

	/// Builds a document whose children are the given flow nodes.
	fn doc(kids: Vec<Dat>) -> Dat {
		node(NodeKind::Doc, mapdat!{
			"title" => dat!("A Title"),
			"lang" => dat!("en"),
			"children" => Dat::List(kids),
		})
	}

	/// A document using every v0 node kind at least once.
	fn every_kind() -> Dat {
		let heading = node(NodeKind::Heading, mapdat!{
			"level" => dat!(2u8),
			"children" => Dat::List(vec![text("Style without a cascade")]),
		});
		let para = node(NodeKind::Para, mapdat!{
			"children" => Dat::List(vec![
				text("A run, "),
				node(NodeKind::Emph, mapdat!{
					"strong" => dat!(true),
					"children" => Dat::List(vec![text("emphasised")]),
				}),
				node(NodeKind::Link, mapdat!{
					"to" => mapdat!{ "name" => dat!("news.cricket") },
					"children" => Dat::List(vec![text("a link")]),
				}),
			]),
		});
		let list = node(NodeKind::List, mapdat!{
			"ordered" => dat!(false),
			"children" => Dat::List(vec![
				node(NodeKind::Item, mapdat!{
					"children" => Dat::List(vec![
						node(NodeKind::Para, mapdat!{
							"children" => Dat::List(vec![text("An item")]),
						}),
					]),
				}),
			]),
		});
		let image = node(NodeKind::Image, mapdat!{
			"hash" => hash32(),
			"alt" => dat!("A diagram of the tree"),
			"w" => dat!(640u32),
			"h" => dat!(480u32),
		});
		let boxx = node(NodeKind::Boxx, mapdat!{
			"style" => dat!("note"),
			"children" => Dat::List(vec![image]),
		});
		let section = node(NodeKind::Section, mapdat!{
			"title" => dat!("A Section"),
			"children" => Dat::List(vec![heading, para, list, boxx]),
		});
		// The doc carries a style table so the box's "note" style resolves (§4.4).
		node(NodeKind::Doc, mapdat!{
			"title" => dat!("A Title"),
			"lang" => dat!("en"),
			"styles" => mapdat!{
				"note" => mapdat!{ "bg" => dat!("muted"), "pad" => dat!(3u8) },
			},
			"children" => Dat::List(vec![section]),
		})
	}

	#[test]
	fn test_valid_every_kind_00() -> Outcome<()> {
		let stats = res!(validate(&every_kind(), SCHEMA_DOC));
		// doc, section, heading, text, para, text, emph, text, link, text, list, item, para, text,
		// box, image.
		assert_eq!(stats.nodes, 16);
		// The deepest run is doc, section, list, item, para, text.
		assert_eq!(stats.depth, 6);
		Ok(())
	}

	#[test]
	fn test_wrong_schema_01() -> Outcome<()> {
		let e = match validate(&every_kind(), "oxeweb/doc/1") {
			Ok(_) => return Err(err!("A foreign schema was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("oxeweb/doc/1"), "Error must name the schema it got: {}", e);
		Ok(())
	}

	#[test]
	fn test_para_in_para_02() -> Outcome<()> {
		let inner = node(NodeKind::Para, mapdat!{
			"children" => Dat::List(vec![text("Inner")]),
		});
		let outer = node(NodeKind::Para, mapdat!{
			"children" => Dat::List(vec![inner]),
		});
		let e = match validate(&doc(vec![outer]), SCHEMA_DOC) {
			Ok(_) => return Err(err!("A para inside a para was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 2"), "Error must name node 2: {}", e);
		assert!(e.contains("para"), "Error must name the kind: {}", e);
		assert!(e.contains("inline"), "Error must name the rule broken: {}", e);
		Ok(())
	}

	#[test]
	fn test_unknown_kind_code_03() -> Outcome<()> {
		let alien = Dat::Usr(
			UsrKindId::new(99, None, None),
			Some(Box::new(mapdat!{})),
		);
		let e = match validate(&doc(vec![alien]), SCHEMA_DOC) {
			Ok(_) => return Err(err!("An unknown kind code was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 1"), "Error must name node 1: {}", e);
		assert!(e.contains("99"), "Error must name the code: {}", e);
		Ok(())
	}

	#[test]
	fn test_missing_required_field_04() -> Outcome<()> {
		// A list without its ordered field.
		let list = node(NodeKind::List, mapdat!{
			"children" => Dat::List(vec![
				node(NodeKind::Item, mapdat!{}),
			]),
		});
		let e = match validate(&doc(vec![list]), SCHEMA_DOC) {
			Ok(_) => return Err(err!("A list without 'ordered' was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 1"), "Error must name node 1: {}", e);
		assert!(e.contains("ordered"), "Error must name the field: {}", e);
		Ok(())
	}

	#[test]
	fn test_unknown_field_05() -> Outcome<()> {
		let para = node(NodeKind::Para, mapdat!{
			"align" => dat!("centre"),
			"children" => Dat::List(vec![text("Text")]),
		});
		let e = match validate(&doc(vec![para]), SCHEMA_DOC) {
			Ok(_) => return Err(err!("An unknown field was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 1"), "Error must name node 1: {}", e);
		assert!(e.contains("align"), "Error must name the field: {}", e);
		Ok(())
	}

	#[test]
	fn test_wrong_typed_field_06() -> Outcome<()> {
		// A heading level as an i32, not the declared u8: two encodings, two addresses.
		let heading = node(NodeKind::Heading, mapdat!{
			"level" => dat!(2i32),
			"children" => Dat::List(vec![text("Heading")]),
		});
		let e = match validate(&doc(vec![heading]), SCHEMA_DOC) {
			Ok(_) => return Err(err!("A wrongly typed field was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 1"), "Error must name node 1: {}", e);
		assert!(e.contains("level"), "Error must name the field: {}", e);
		assert!(e.contains("u8"), "Error must name the declared type: {}", e);
		Ok(())
	}

	#[test]
	fn test_heading_level_zero_07() -> Outcome<()> {
		let heading = node(NodeKind::Heading, mapdat!{
			"level" => dat!(0u8),
			"children" => Dat::List(vec![text("Heading")]),
		});
		let e = match validate(&doc(vec![heading]), SCHEMA_DOC) {
			Ok(_) => return Err(err!("A heading of level 0 was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 1"), "Error must name node 1: {}", e);
		assert!(e.contains("1..=6"), "Error must name the range: {}", e);
		Ok(())
	}

	#[test]
	fn test_heading_level_seven_08() -> Outcome<()> {
		let heading = node(NodeKind::Heading, mapdat!{
			"level" => dat!(7u8),
			"children" => Dat::List(vec![text("Heading")]),
		});
		let e = match validate(&doc(vec![heading]), SCHEMA_DOC) {
			Ok(_) => return Err(err!("A heading of level 7 was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 1"), "Error must name node 1: {}", e);
		assert!(e.contains("1..=6"), "Error must name the range: {}", e);
		Ok(())
	}

	#[test]
	fn test_heading_levels_one_to_six_09() -> Outcome<()> {
		for level in 1u8..=6 {
			let heading = node(NodeKind::Heading, mapdat!{
				"level" => dat!(level),
				"children" => Dat::List(vec![text("Heading")]),
			});
			let stats = res!(validate(&doc(vec![heading]), SCHEMA_DOC));
			assert_eq!(stats.nodes, 3);
		}
		Ok(())
	}

	#[test]
	fn test_missing_alt_10() -> Outcome<()> {
		let image = node(NodeKind::Image, mapdat!{
			"hash" => hash32(),
		});
		let e = match validate(&doc(vec![image]), SCHEMA_DOC) {
			Ok(_) => return Err(err!("An image without alt text was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 1"), "Error must name node 1: {}", e);
		assert!(e.contains("alt"), "Error must name the field: {}", e);
		Ok(())
	}

	#[test]
	fn test_node_count_over_limit_11() -> Outcome<()> {
		// One doc plus enough paragraphs to pass the limit.
		let mut kids = Vec::new();
		for _ in 0..limit::NODES {
			kids.push(node(NodeKind::Para, mapdat!{}));
		}
		let e = match validate(&doc(kids), SCHEMA_DOC) {
			Ok(_) => return Err(err!("A tree over the node limit was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains(&fmt!("{}", limit::NODES)), "Error must name the limit: {}", e);
		Ok(())
	}

	#[test]
	fn test_node_count_at_limit_12() -> Outcome<()> {
		let mut kids = Vec::new();
		for _ in 0..(limit::NODES - 1) {
			kids.push(node(NodeKind::Para, mapdat!{}));
		}
		let stats = res!(validate(&doc(kids), SCHEMA_DOC));
		assert_eq!(stats.nodes, limit::NODES);
		Ok(())
	}

	#[test]
	fn test_non_doc_root_13() -> Outcome<()> {
		let root = node(NodeKind::Para, mapdat!{
			"children" => Dat::List(vec![text("Not a document")]),
		});
		let e = match validate(&root, SCHEMA_DOC) {
			Ok(_) => return Err(err!("A para as root was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 0"), "Error must name node 0: {}", e);
		assert!(e.contains("doc"), "Error must name the rule broken: {}", e);
		Ok(())
	}

	#[test]
	fn test_text_payload_must_be_str_14() -> Outcome<()> {
		let para = node(NodeKind::Para, mapdat!{
			"children" => Dat::List(vec![
				node(NodeKind::Text, mapdat!{ "value" => dat!("Wrapped") }),
			]),
		});
		let e = match validate(&doc(vec![para]), SCHEMA_DOC) {
			Ok(_) => return Err(err!("A text run with a map payload was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 2"), "Error must name node 2: {}", e);
		assert!(e.contains("str"), "Error must name the payload rule: {}", e);
		Ok(())
	}

	#[test]
	fn test_non_map_payload_15() -> Outcome<()> {
		let para = Dat::Usr(
			UsrKindId::new(NodeKind::Para.code(), Some("para"), None),
			Some(Box::new(dat!("Bare string"))),
		);
		let e = match validate(&doc(vec![para]), SCHEMA_DOC) {
			Ok(_) => return Err(err!("A para with a string payload was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 1"), "Error must name node 1: {}", e);
		assert!(e.contains("map"), "Error must name the payload rule: {}", e);
		Ok(())
	}

	#[test]
	fn test_children_where_none_admitted_16() -> Outcome<()> {
		let image = node(NodeKind::Image, mapdat!{
			"hash" => Dat::BU8(vec![1]),
			"alt" => dat!("An image"),
			"children" => Dat::List(vec![text("Impossible")]),
		});
		let e = match validate(&doc(vec![image]), SCHEMA_DOC) {
			Ok(_) => return Err(err!("An image with children was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 1"), "Error must name node 1: {}", e);
		assert!(e.contains("children"), "Error must name the field: {}", e);
		Ok(())
	}

	#[test]
	fn test_node_ids_are_pre_order_17() -> Outcome<()> {
		// doc(0), section(1), para(2), text(3), para(4), text(5). The forbidden node is the second
		// para's child, so a text run misplaced there must be named 5, not 4.
		let bad = node(NodeKind::Section, mapdat!{
			"title" => dat!("S"),
			"children" => Dat::List(vec![
				node(NodeKind::Para, mapdat!{
					"children" => Dat::List(vec![text("One")]),
				}),
				node(NodeKind::Para, mapdat!{
					"children" => Dat::List(vec![
						node(NodeKind::Heading, mapdat!{
							"level" => dat!(1u8),
							"children" => Dat::List(vec![]),
						}),
					]),
				}),
			]),
		});
		let e = match validate(&doc(vec![bad]), SCHEMA_DOC) {
			Ok(_) => return Err(err!("A heading inside a para was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 5"), "Error must name node 5: {}", e);
		Ok(())
	}

	#[test]
	fn test_non_usr_node_18() -> Outcome<()> {
		let e = match validate(&doc(vec![dat!("Loose string")]), SCHEMA_DOC) {
			Ok(_) => return Err(err!("A bare string as a node was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 1"), "Error must name node 1: {}", e);
		assert!(e.contains("usr"), "Error must name the rule broken: {}", e);
		Ok(())
	}

	/// Builds a doc whose one paragraph carries a single link with the given address.
	fn doc_with_link(to: Dat) -> Dat {
		doc(vec![
			node(NodeKind::Para, mapdat!{
				"children" => Dat::List(vec![
					node(NodeKind::Link, mapdat!{
						"to" => to,
						"children" => Dat::List(vec![text("here")]),
					}),
				]),
			}),
		])
	}

	/// Builds a doc carrying a style table of one record and one box that names a style.
	fn doc_with_style(record: Dat, style_name: &str) -> Dat {
		node(NodeKind::Doc, mapdat!{
			"title" => dat!("T"),
			"lang" => dat!("en"),
			"styles" => mapdat!{ "callout" => record },
			"children" => Dat::List(vec![
				node(NodeKind::Boxx, mapdat!{
					"style" => dat!(style_name),
					"children" => Dat::List(vec![
						node(NodeKind::Para, mapdat!{
							"children" => Dat::List(vec![text("in a box")]),
						}),
					]),
				}),
			]),
		})
	}

	#[test]
	fn test_code_accepted_19() -> Outcome<()> {
		let code = node(NodeKind::Code, mapdat!{
			"lang" => dat!("rust"),
			"text" => dat!("fn main() {}"),
		});
		let stats = res!(validate(&doc(vec![code]), SCHEMA_DOC));
		// doc, code.
		assert_eq!(stats.nodes, 2);
		Ok(())
	}

	#[test]
	fn test_quote_accepted_20() -> Outcome<()> {
		let quote = node(NodeKind::Quote, mapdat!{
			"cite" => dat!("A. Author"),
			"children" => Dat::List(vec![
				node(NodeKind::Para, mapdat!{
					"children" => Dat::List(vec![text("Quoted")]),
				}),
			]),
		});
		let stats = res!(validate(&doc(vec![quote]), SCHEMA_DOC));
		// doc, quote, para, text.
		assert_eq!(stats.nodes, 4);
		Ok(())
	}

	#[test]
	fn test_link_by_name_accepted_21() -> Outcome<()> {
		let tree = doc_with_link(mapdat!{ "name" => dat!("news.cricket") });
		let stats = res!(validate(&tree, SCHEMA_DOC));
		// doc, para, link, text.
		assert_eq!(stats.nodes, 4);
		Ok(())
	}

	#[test]
	fn test_link_by_hash_accepted_22() -> Outcome<()> {
		let tree = doc_with_link(mapdat!{ "hash" => hash32() });
		let stats = res!(validate(&tree, SCHEMA_DOC));
		assert_eq!(stats.nodes, 4);
		Ok(())
	}

	#[test]
	fn test_link_address_two_entries_23() -> Outcome<()> {
		let tree = doc_with_link(mapdat!{
			"name" => dat!("news.cricket"),
			"hash" => hash32(),
		});
		let e = match validate(&tree, SCHEMA_DOC) {
			Ok(_) => return Err(err!("A two-entry link address was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 2"), "Error must name node 2: {}", e);
		assert!(e.contains("to") || e.contains("address"),
			"Error must name the address rule: {}", e);
		Ok(())
	}

	#[test]
	fn test_style_names_missing_entry_24() -> Outcome<()> {
		// The table defines "callout", but the box names "ghost".
		let tree = doc_with_style(mapdat!{ "bg" => dat!("muted") }, "ghost");
		let e = match validate(&tree, SCHEMA_DOC) {
			Ok(_) => return Err(err!("A style naming an absent entry was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 1"), "Error must name node 1, the box: {}", e);
		assert!(e.contains("ghost"), "Error must name the missing style: {}", e);
		Ok(())
	}

	#[test]
	fn test_style_defined_but_unreferenced() -> Outcome<()> {
		// The table defines "callout", but no node names it. An unused style has no effect, so a
		// document with it and one without render alike, which would be two addresses for one
		// document (§4.4).
		let tree = node(NodeKind::Doc, mapdat!{
			"title" => dat!("T"),
			"lang" => dat!("en"),
			"styles" => mapdat!{ "callout" => mapdat!{ "bg" => dat!("muted") } },
			"children" => Dat::List(vec![
				node(NodeKind::Para, mapdat!{
					"children" => Dat::List(vec![text("no style here")]),
				}),
			]),
		});
		let e = match validate(&tree, SCHEMA_DOC) {
			Ok(_) => return Err(err!("An unreferenced style entry was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("callout"), "Error must name the unused style: {}", e);
		Ok(())
	}

	#[test]
	fn test_style_unknown_property_25() -> Outcome<()> {
		let tree = doc_with_style(mapdat!{ "wibble" => dat!("x") }, "callout");
		let e = match validate(&tree, SCHEMA_DOC) {
			Ok(_) => return Err(err!("A style record with an unknown property was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		// The style table is validated at the doc, node 0.
		assert!(e.contains("Node 0"), "Error must name node 0, the doc: {}", e);
		assert!(e.contains("callout"), "Error must name the style: {}", e);
		assert!(e.contains("wibble"), "Error must name the property: {}", e);
		Ok(())
	}

	#[test]
	fn test_style_bg_out_of_palette_26() -> Outcome<()> {
		let tree = doc_with_style(mapdat!{ "bg" => dat!("purple") }, "callout");
		let e = match validate(&tree, SCHEMA_DOC) {
			Ok(_) => return Err(err!("A bg outside the palette was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 0"), "Error must name node 0, the doc: {}", e);
		assert!(e.contains("callout"), "Error must name the style: {}", e);
		assert!(e.contains("bg"), "Error must name the property: {}", e);
		Ok(())
	}

	#[test]
	fn test_style_align_out_of_enum_27() -> Outcome<()> {
		let tree = doc_with_style(mapdat!{ "align" => dat!("middle") }, "callout");
		let e = match validate(&tree, SCHEMA_DOC) {
			Ok(_) => return Err(err!("An align outside the enum was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 0"), "Error must name node 0, the doc: {}", e);
		assert!(e.contains("callout"), "Error must name the style: {}", e);
		assert!(e.contains("align"), "Error must name the property: {}", e);
		Ok(())
	}

	#[test]
	fn test_unknown_kind_with_fallback_accepted_28() -> Outcome<()> {
		// A kind 20 the reader does not know, carrying a valid fallback and an uninterpreted field.
		let alien = Dat::Usr(
			UsrKindId::new(20, None, None),
			Some(Box::new(mapdat!{
				"fallback" => Dat::List(vec![
					node(NodeKind::Para, mapdat!{
						"children" => Dat::List(vec![text("A fallback paragraph")]),
					}),
				]),
				"rows" => dat!("only a reader that knows kind 20 uses this"),
			})),
		);
		let stats = res!(validate(&doc(vec![alien]), SCHEMA_DOC));
		// doc, the unknown node, its fallback para, and that para's text all count toward the limit.
		assert_eq!(stats.nodes, 4);
		Ok(())
	}

	#[test]
	fn test_unknown_kind_without_fallback_rejected_29() -> Outcome<()> {
		let alien = Dat::Usr(
			UsrKindId::new(20, None, None),
			Some(Box::new(mapdat!{ "rows" => dat!("no fallback here") })),
		);
		let e = match validate(&doc(vec![alien]), SCHEMA_DOC) {
			Ok(_) => return Err(err!("An unknown kind with no fallback was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 1"), "Error must name node 1: {}", e);
		assert!(e.contains("fallback"), "Error must name the fallback rule: {}", e);
		Ok(())
	}

	#[test]
	fn test_unknown_kind_empty_fallback_rejected_30() -> Outcome<()> {
		let alien = Dat::Usr(
			UsrKindId::new(20, None, None),
			Some(Box::new(mapdat!{ "fallback" => Dat::List(Vec::new()) })),
		);
		let e = match validate(&doc(vec![alien]), SCHEMA_DOC) {
			Ok(_) => return Err(err!("An unknown kind with an empty fallback was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 1"), "Error must name node 1: {}", e);
		assert!(e.contains("fallback"), "Error must name the fallback rule: {}", e);
		Ok(())
	}

	/// A node of a reserved kind (§4.2), built as a document author would have to build one.
	fn reserved(kind: ReservedKind, payload: Dat) -> Dat {
		Dat::Usr(
			UsrKindId::new(kind.code(), Some(kind.label()), None),
			Some(Box::new(payload)),
		)
	}

	#[test]
	fn test_reserved_edit_in_doc_rejected_31() -> Outcome<()> {
		let edit = reserved(ReservedKind::Edit, mapdat!{
			"placeholder" => dat!("Search the oxeweb"),
		});
		let e = match validate(&doc(vec![edit]), SCHEMA_DOC) {
			Ok(_) => return Err(err!("A document carrying an edit node was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 1"), "Error must name node 1: {}", e);
		assert!(e.contains("edit"), "Error must name the kind: {}", e);
		assert!(e.contains("may not carry an edit node"),
			"Error must say a document may not carry it: {}", e);
		Ok(())
	}

	#[test]
	fn test_reserved_surface_in_doc_rejected_32() -> Outcome<()> {
		let surface = reserved(ReservedKind::Surface, mapdat!{
			"app" => dat!("app.modeller"),
		});
		let e = match validate(&doc(vec![surface]), SCHEMA_DOC) {
			Ok(_) => return Err(err!("A document carrying a surface node was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 1"), "Error must name node 1: {}", e);
		assert!(e.contains("surface"), "Error must name the kind: {}", e);
		assert!(e.contains("may not carry a surface node"),
			"Error must say a document may not carry it: {}", e);
		Ok(())
	}

	#[test]
	fn test_reserved_surface_with_fallback_still_rejected_33() -> Outcome<()> {
		// The hole §4.5 would leave open if a reserved code were treated as merely unknown. The
		// fallback is valid, non-empty, and made of known nodes, and it buys the surface nothing: a
		// reader that knows what code 15 is refuses it whether or not it is offered a stand-in.
		let surface = reserved(ReservedKind::Surface, mapdat!{
			"fallback" => Dat::List(vec![
				node(NodeKind::Para, mapdat!{
					"children" => Dat::List(vec![text("A picture of a teapot.")]),
				}),
			]),
			"app" => dat!("app.modeller"),
		});
		let e = match validate(&doc(vec![surface]), SCHEMA_DOC) {
			Ok(_) => return Err(err!(
				"A document carrying a surface node with a valid fallback was accepted. A fallback \
				admits an unknown kind, and never a reserved one."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 1"), "Error must name node 1: {}", e);
		assert!(e.contains("may not carry a surface node"),
			"Error must say a document may not carry it: {}", e);
		assert!(e.contains("whether or not it carries a fallback"),
			"Error must say the fallback does not admit it: {}", e);
		Ok(())
	}

	/// A valid border, as a chrome writes one: a palette name and a width in pixels.
	fn border() -> Dat {
		Dat::List(vec![dat!("muted"), dat!(1u8)])
	}

	#[test]
	fn test_style_interface_property_in_doc_rejected_35() -> Outcome<()> {
		// The v0 document vocabulary is closed. A document naming an interface property is refused,
		// whatever the reader can draw, so a document cannot dress as an interface.
		//
		// `radius` is NOT among these and belongs with the document's own eight: it decorates a surface
		// a document already draws with `bg` and `pad`, and it means something in prose, which the
		// layout of a bar does not. `shadow` is elevation -- a paragraph claiming to float above its
		// page IS a document dressing as an interface -- and `border` is the line round a control with
		// an edge. See [`STYLE_FIELDS`](oxedyne_fe2o3_sbj::kinds::STYLE_FIELDS) for the whole of that argument.
		for (prop, value) in [
			("grid",	dat!(1400u16)),
			("shadow",	dat!(2u8)),
			("border",	border()),
		] {
			let tree = doc_with_style(mapdat!{ prop => value }, "callout");
			let e = match validate(&tree, SCHEMA_DOC) {
				Ok(_) => return Err(err!(
					"A document naming the interface style property '{}' was accepted. The v0 \
					document vocabulary is frozen at eight.", prop; Test)),
				Err(e) => fmt!("{}", e),
			};
			assert!(e.contains("Node 0"), "Error must name node 0, the doc: {}", e);
			assert!(e.contains("callout"), "Error must name the style: {}", e);
			assert!(e.contains(prop), "Error must name the property: {}", e);
			// The refusal must not call a real property unknown: a member whose chrome draws a grid
			// every day is being misled by "unknown style property 'grid'".
			assert!(!e.contains("unknown"),
				"'{}' is a real property, and the refusal must not call it unknown: {}", prop, e);
			assert!(e.contains(SCHEMA_DOC), "Error must name the schema that refused it: {}", e);
			assert!(e.contains("a chrome or an application tree"),
				"Error must say where '{}' is legal: {}", prop, e);
		}
		Ok(())
	}

	#[test]
	fn test_style_interface_property_in_chrome_and_app_accepted_36() -> Outcome<()> {
		// The chrome is a real interface and needs to look like one: a grid of tiles, an edge, a
		// rounded corner. An application's tree is drawn by the same engine and admits the same.
		for schema in [SCHEMA_CHROME, SCHEMA_APP] {
			let tree = doc_with_style(mapdat!{
				"grid" => dat!(1400u16),
				"radius" => dat!(2u8),
				"border" => border(),
				"bg" => dat!("muted"),
				"pad" => dat!(3u8),
			}, "callout");
			let stats = res!(validate(&tree, schema));
			// doc, box, para, text.
			assert_eq!(stats.nodes, 4, "'{}' admits the interface properties", schema);
		}
		Ok(())
	}

	#[test]
	fn test_style_unknown_property_in_chrome_still_rejected_37() -> Outcome<()> {
		// A wider vocabulary is not an open one: the chrome's admitted set is closed like any other.
		let tree = doc_with_style(mapdat!{ "wibble" => dat!("x") }, "callout");
		let e = match validate(&tree, SCHEMA_CHROME) {
			Ok(_) => return Err(err!("A chrome style with an unknown property was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("wibble"), "Error must name the property: {}", e);
		assert!(e.contains("unknown"), "An invented property is unknown, and is named so: {}", e);
		Ok(())
	}

	#[test]
	fn test_style_document_properties_admitted_by_every_schema_38() -> Outcome<()> {
		// The eight are the floor, not the document's alone: a chrome is a document that may say more.
		for schema in [SCHEMA_DOC, SCHEMA_CHROME, SCHEMA_APP] {
			let tree = doc_with_style(mapdat!{
				"fill" => dat!("ink"),
				"size" => dat!(1i8),
				"lang" => dat!("en-GB"),
				"dir" => dat!("ltr"),
				"bg" => dat!("muted"),
				"pad" => dat!(3u8),
				"align" => dat!("start"),
				"radius" => dat!(2u8),
			}, "callout");
			let stats = res!(validate(&tree, schema));
			assert_eq!(stats.nodes, 4, "'{}' admits the eight document properties", schema);
		}
		Ok(())
	}

	#[test]
	fn test_a_document_may_round_the_corner_of_a_box_it_already_draws_44() -> Outcome<()> {
		// The question that moved `radius` out of the interface's list: is a corner radius a property a
		// DOCUMENT legitimately has? It decorates a surface a document already draws -- a style may name
		// `bg` and `pad`, so a tinted padded box is already a thing a document makes -- and it adds no
		// element, no geometry and no authority. A soft-cornered callout is typography, not an
		// interface.
		let callout = doc_with_style(mapdat!{
			"bg" => dat!("muted"),
			"pad" => dat!(3u8),
			"radius" => dat!(2u8),
		}, "callout");
		let stats = res!(validate(&callout, SCHEMA_DOC));
		assert_eq!(stats.nodes, 4, "a document rounds the corner of its own box");

		// And the rule it seemed to break holds where it was actually doing work: a document still
		// cannot claim to stand off its page, nor draw the edge of a control.
		for prop in ["shadow", "grow"] {
			let tree = doc_with_style(mapdat!{ prop => dat!(2u8) }, "callout");
			assert!(
				validate(&tree, SCHEMA_DOC).is_err(),
				"'{}' is the interface's and a document must not name it", prop,
			);
		}
		Ok(())
	}

	#[test]
	fn test_style_border_is_a_palette_name_and_a_width_39() -> Outcome<()> {
		// The validator and the renderer must agree about what a border is, since a border the
		// validator accepts and Kiln cannot draw is a chrome tree that passes and then fails. These
		// are the shapes Kiln refuses, and they are refused here too.
		let bad: Vec<(&str, Dat)> = vec![
			("a border of one thing",	Dat::List(vec![dat!("muted")])),
			("a border of three",	Dat::List(vec![dat!("muted"), dat!(1u8), dat!(2u8)])),
			("a border of nothing",	Dat::List(Vec::new())),
			("a border the wrong way round",	Dat::List(vec![dat!(1u8), dat!("muted")])),
			("a border that is a bare colour",	dat!("muted")),
			("a border of a colour outside the palette",	Dat::List(vec![dat!("puce"), dat!(1u8)])),
			("a border whose width is not a u8",	Dat::List(vec![dat!("muted"), dat!(1i8)])),
		];
		for (what, value) in bad {
			let tree = doc_with_style(mapdat!{ "border" => value }, "callout");
			let e = match validate(&tree, SCHEMA_CHROME) {
				Ok(_) => return Err(err!("{} was accepted, and is not a border.", what; Test)),
				Err(e) => fmt!("{}", e),
			};
			assert!(e.contains("border"), "{}: the error must name the property: {}", what, e);
		}
		// And the one shape that is a border is accepted.
		let tree = doc_with_style(mapdat!{ "border" => border() }, "callout");
		res!(validate(&tree, SCHEMA_CHROME));
		Ok(())
	}

	#[test]
	fn test_style_grid_and_radius_are_whole_scalars_40() -> Outcome<()> {
		// Kiln reads each at ONE declared width and nothing else, so a negative step or a string is
		// refused here rather than reaching a renderer that cannot read it. The widths differ -- a grid
		// tile is a percentage of a base size and a radius a step on the spacing scale -- and the error
		// must name the one it wanted, or an author cannot tell which they got wrong.
		for (prop, width) in [("grid", "u16"), ("radius", "u8")] {
			for value in [dat!("2"), dat!(-2i8), dat!(2i32), dat!(2u32)] {
				let tree = doc_with_style(mapdat!{ prop => value }, "callout");
				let e = match validate(&tree, SCHEMA_CHROME) {
					Ok(_) => return Err(err!(
						"A '{}' that is not a whole scalar was accepted.", prop; Test)),
					Err(e) => fmt!("{}", e),
				};
				assert!(e.contains(prop), "Error must name the property: {}", e);
				assert!(e.contains(width), "Error must name the declared width: {}", e);
			}
		}
		Ok(())
	}

	#[test]
	fn test_reserved_icon_in_doc_rejected_41() -> Outcome<()> {
		// An icon names the reader's own drawing, so a document carrying one would let whichever
		// reader opened it supply the document's content. Refused by the same rule as the other two.
		let icon = reserved(ReservedKind::Icon, mapdat!{ "name" => dat!("home") });
		let tree = doc(vec![node(NodeKind::Para, mapdat!{ "children" => Dat::List(vec![icon]) })]);
		let e = match validate(&tree, SCHEMA_DOC) {
			Ok(_) => return Err(err!("A document carrying an icon node was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		// doc 0, para 1, icon 2.
		assert!(e.contains("Node 2"), "Error must name the icon's node: {}", e);
		assert!(e.contains("icon"), "Error must name the kind: {}", e);
		assert!(e.contains("may not carry an icon node"),
			"Error must say a document may not carry it: {}", e);
		Ok(())
	}

	#[test]
	fn test_reserved_icon_in_chrome_and_app_accepted_42() -> Outcome<()> {
		// A chrome is a real interface and its buttons are icons. An application's tree is drawn by
		// the same engine and reaches the same set.
		for schema in [SCHEMA_CHROME, SCHEMA_APP] {
			for name in crate::kinds::ICON_NAMES {
				let icon = reserved(ReservedKind::Icon, mapdat!{ "name" => dat!(*name) });
				let tree = doc(vec![
					node(NodeKind::Para, mapdat!{ "children" => Dat::List(vec![icon]) }),
				]);
				let stats = res!(validate(&tree, schema));
				// doc, para, icon.
				assert_eq!(stats.nodes, 3, "'{}' admits the icon '{}'", schema, name);
			}
		}
		Ok(())
	}

	#[test]
	fn test_reserved_icon_of_an_unknown_name_rejected_43() -> Outcome<()> {
		// The set is closed. An icon carries no drawing of its own, so a name the engine has never
		// heard of has nothing faithful to stand in for it: there is no gap to render, only a caller
		// that asked for something that does not exist.
		for schema in [SCHEMA_CHROME, SCHEMA_APP] {
			let icon = reserved(ReservedKind::Icon, mapdat!{ "name" => dat!("wibble") });
			let tree = doc(vec![node(NodeKind::Para, mapdat!{ "children" => Dat::List(vec![icon]) })]);
			let e = match validate(&tree, schema) {
				Ok(_) => return Err(err!(
					"An icon named 'wibble' was accepted by '{}'.", schema; Test)),
				Err(e) => fmt!("{}", e),
			};
			assert!(e.contains("wibble"), "Error must name the icon asked for: {}", e);
			assert!(e.contains("back"), "Error must spell the set it would have taken: {}", e);
		}
		Ok(())
	}

	#[test]
	fn test_reserved_icon_with_fallback_still_rejected_44() -> Outcome<()> {
		// The §4.5 hole, for the icon as for the surface: a reader that knows what code 16 is refuses
		// it in a document whether or not it is offered a stand-in.
		let icon = reserved(ReservedKind::Icon, mapdat!{
			"name" => dat!("home"),
			"fallback" => Dat::List(vec![
				Dat::Usr(
					UsrKindId::new(NodeKind::Para.code(), Some("para"), None),
					Some(Box::new(mapdat!{})),
				),
			]),
		});
		let tree = doc(vec![node(NodeKind::Para, mapdat!{ "children" => Dat::List(vec![icon]) })]);
		let e = match validate(&tree, SCHEMA_DOC) {
			Ok(_) => return Err(err!("An icon with a fallback was accepted in a document."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("icon"), "Error must name the kind: {}", e);
		Ok(())
	}

	#[test]
	fn test_reserved_surface_as_root_rejected_34() -> Outcome<()> {
		let root = reserved(ReservedKind::Surface, mapdat!{
			"app" => dat!("app.modeller"),
		});
		let e = match validate(&root, SCHEMA_DOC) {
			Ok(_) => return Err(err!("A surface as the root of a document was accepted."; Test)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("Node 0"), "Error must name node 0: {}", e);
		assert!(e.contains("may not carry a surface node"),
			"Error must say a document may not carry it: {}", e);
		Ok(())
	}
}
