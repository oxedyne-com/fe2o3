//! Exporting a document tree back to the neutral tree of [`fe2o3_text::doc`](oxedyne_fe2o3_text::doc).
//!
//! The inverse of [`import::from_doc`](crate::import::from_doc): `to_doc(from_doc(d)) == d`. A node
//! that `from_doc` never writes has no block or inline to come back as, and is refused by name rather
//! than guessed at: a `section`, a `box`, a block `image`, a quote's `cite`, a `style`, and an address
//! or an inline image given by content hash, which a caller resolves to a source first.

use crate::{
	canon,
	kinds::{
		NodeKind,
		ADDR_HASH,
		ADDR_NAME,
		KEY_CHILDREN,
		KEY_CLASSES,
		KEY_COLS,
		KEY_HEAD,
		KEY_HEADER,
		KEY_ID,
		KEY_PAIRS,
		KEY_SRC,
		KEY_STYLE,
		KEY_STYLES,
		KEY_TO,
	},
	limit,
	validate,
	SCHEMA_DOC_1,
};

use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_text::doc::{
	Align,
	Attrs,
	Block,
	Cell,
	Doc,
	Inline,
	Row,
	Term,
};

use oxedyne_fe2o3_core::prelude::*;

/// Maps an `oxeweb/doc/1` tree to the document it holds.
///
/// The tree is held to canon and to the validator first, so what is mapped is a tree the format
/// accepts; the walk then refuses any node deeper than [`limit::DEPTH`] whatever route the tree
/// arrived by, since it recurses.
pub fn to_doc(tree: &Dat) -> Outcome<Doc> {
	res!(canon::check(tree));
	res!(validate::validate(tree, SCHEMA_DOC_1));
	let (kind, map) = res!(node_map(tree));
	if kind != NodeKind::Doc {
		return Err(err!("The root is a {}, not a doc.", kind.label(); Invalid, Input));
	}
	if map.get(&dat!(KEY_STYLES)).is_some() {
		return Err(err!(
			"The doc carries a '{}' table, which a document of fe2o3_text::doc has no place for.",
			KEY_STYLES;
		Invalid, Input, Unimplemented));
	}
	Ok(Doc { blocks: res!(blocks(res!(kids(map)), 1)) })
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ BLOCKS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

fn blocks(
	nodes:	&[Dat],
	depth:	usize,
)
	-> Outcome<Vec<Block>>
{
	let mut out = Vec::with_capacity(nodes.len());
	for n in nodes {
		out.push(res!(block(n, depth + 1)));
	}
	Ok(out)
}

fn block(
	n:	&Dat,
	depth:	usize,
)
	-> Outcome<Block>
{
	res!(check_depth(depth));
	let (kind, map) = res!(node_map(n));
	let kids = res!(kids(map));
	Ok(match kind {
		NodeKind::Heading => Block::Heading {
			level:		match map.get(&dat!("level")) {
				Some(Dat::U8(level)) => *level,
				_ => return Err(err!("A heading carries no u8 level."; Invalid, Input, Missing)),
			},
			content:	res!(inlines(kids, depth)),
		},
		NodeKind::Para => Block::Para(res!(inlines(kids, depth))),
		NodeKind::List => {
			let mut items = Vec::with_capacity(kids.len());
			for item in kids {
				let (_, imap) = res!(node_map(item));
				items.push(res!(blocks(res!(self::kids(imap)), depth + 1)));
			}
			Block::List { ordered: res!(flag(map, "ordered")), items }
		},
		NodeKind::Code => Block::Code {
			lang:	res!(opt_str(map, "lang")),
			text:	res!(req_str(map, "text")),
		},
		NodeKind::Quote => {
			if map.get(&dat!("cite")).is_some() {
				return Err(err!(
					"A quote carries a 'cite', which a document of fe2o3_text::doc has no place \
					for."; Invalid, Input, Unimplemented));
			}
			Block::Quote(res!(blocks(kids, depth)))
		},
		NodeKind::Rule => Block::Rule,
		NodeKind::Table => {
			let mut cols = Vec::new();
			if let Some(Dat::List(list)) = map.get(&dat!(KEY_COLS)) {
				for c in list {
					cols.push(match c {
						Dat::Str(s) if s == "none"	=> Align::None,
						Dat::Str(s) if s == "start"	=> Align::Start,
						Dat::Str(s) if s == "center"	=> Align::Centre,
						Dat::Str(s) if s == "end"	=> Align::End,
						d => return Err(err!(
							"A table column has the alignment {:?}.", d; Invalid, Input, Range)),
					});
				}
			}
			let mut caption = Vec::new();
			let mut head = None;
			let mut rows = Vec::with_capacity(kids.len());
			for kid in kids {
				let (k, kmap) = res!(node_map(kid));
				match k {
					NodeKind::Caption => caption = res!(inlines(res!(self::kids(kmap)), depth + 1)),
					NodeKind::Row => {
						let row = res!(table_row(res!(self::kids(kmap)), depth + 1));
						if res!(flag(kmap, KEY_HEAD)) {
							head = Some(row);
						} else {
							rows.push(row);
						}
					},
					_ => return Err(err!(
						"A table holds a {}, which is neither a caption nor a row.", k.label();
					Invalid, Input)),
				}
			}
			Block::Table { head, rows, cols, caption }
		},
		NodeKind::Div => Block::Div {
			attrs:		res!(attrs(map)),
			content:	res!(blocks(kids, depth)),
		},
		NodeKind::Figure => {
			let mut content = Vec::with_capacity(kids.len());
			let mut caption = Vec::new();
			for kid in kids {
				let (k, kmap) = res!(node_map(kid));
				if k == NodeKind::Caption {
					caption = res!(inlines(res!(self::kids(kmap)), depth + 1));
				} else {
					content.push(res!(block(kid, depth + 1)));
				}
			}
			Block::Figure { attrs: res!(attrs(map)), content, caption }
		},
		NodeKind::Terms => {
			let mut items = Vec::with_capacity(kids.len());
			for kid in kids {
				items.push(res!(term(kid, depth + 1)));
			}
			Block::Terms { attrs: res!(attrs(map)), items }
		},
		k => return Err(err!(
			"A {} has no block of fe2o3_text::doc to become; only a tree import::from_doc could \
			have written maps back.", k.label();
		Invalid, Input, Unimplemented)),
	})
}

fn table_row(
	cells:	&[Dat],
	depth:	usize,
)
	-> Outcome<Row>
{
	res!(check_depth(depth));
	let mut out = Vec::with_capacity(cells.len());
	for c in cells {
		res!(check_depth(depth + 1));
		let (_, cmap) = res!(node_map(c));
		out.push(Cell {
			content:	res!(inlines(res!(kids(cmap)), depth + 1)),
			header:		res!(flag(cmap, KEY_HEADER)),
		});
	}
	Ok(Row(out))
}

fn term(
	n:	&Dat,
	depth:	usize,
)
	-> Outcome<Term>
{
	res!(check_depth(depth));
	let (_, map) = res!(node_map(n));
	let mut t = Term::default();
	for kid in res!(kids(map)) {
		let (k, kmap) = res!(node_map(kid));
		let parts = res!(kids(kmap));
		match k {
			NodeKind::Label => t.term = res!(inlines(parts, depth + 1)),
			_ => t.defs.push(res!(blocks(parts, depth + 1))),
		}
	}
	Ok(t)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ INLINES                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

fn inlines(
	nodes:	&[Dat],
	depth:	usize,
)
	-> Outcome<Vec<Inline>>
{
	let mut out = Vec::with_capacity(nodes.len());
	for n in nodes {
		out.push(res!(inline(n, depth + 1)));
	}
	Ok(out)
}

fn inline(
	n:	&Dat,
	depth:	usize,
)
	-> Outcome<Inline>
{
	res!(check_depth(depth));
	// A text run's payload is the string itself.
	if let Dat::Usr(uid, Some(payload)) = n {
		if uid.code() == NodeKind::Text.code() {
			return match payload.as_ref() {
				Dat::Str(s) => Ok(Inline::Text(s.clone())),
				d => Err(err!("A text run carries a {:?}.", d.kind(); Invalid, Input)),
			};
		}
	}
	let (kind, map) = res!(node_map(n));
	let kids = res!(kids(map));
	Ok(match kind {
		NodeKind::Emph => Inline::Emph {
			strong:		res!(flag(map, "strong")),
			content:	res!(inlines(kids, depth)),
		},
		NodeKind::Link => {
			let to = match map.get(&dat!(KEY_TO)) {
				Some(Dat::Map(addr)) => match addr.get(&dat!(ADDR_NAME)) {
					Some(Dat::Str(s)) => s.clone(),
					_ => return Err(err!(
						"A link is addressed by content hash, which has no source string to \
						become; resolve it to a name first."; Invalid, Input, Unimplemented)),
				},
				_ => return Err(err!("A link carries no address."; Invalid, Input, Missing)),
			};
			Inline::Link { to, content: res!(inlines(kids, depth)) }
		},
		NodeKind::Img => {
			if map.get(&dat!(ADDR_HASH)).is_some() {
				return Err(err!(
					"An inline image is addressed by content hash, which has no source string to \
					become; resolve it to a source first."; Invalid, Input, Unimplemented));
			}
			Inline::Image {
				src:	res!(req_str(map, KEY_SRC)),
				alt:	res!(req_str(map, "alt")),
				attrs:	res!(attrs(map)),
			}
		},
		NodeKind::Tt => Inline::Code(res!(req_str(map, "text"))),
		NodeKind::Span => Inline::Span {
			attrs:		res!(attrs(map)),
			content:	res!(inlines(kids, depth)),
		},
		NodeKind::Break => Inline::Break,
		NodeKind::Sup => Inline::Sup(res!(inlines(kids, depth))),
		NodeKind::Sub => Inline::Sub(res!(inlines(kids, depth))),
		k => return Err(err!(
			"A {} has no inline of fe2o3_text::doc to become.", k.label();
		Invalid, Input, Unimplemented)),
	})
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ NODES AND FIELDS                                                           │
// └───────────────────────────────────────────────────────────────────────────┘

fn check_depth(depth: usize) -> Outcome<()> {
	if depth > limit::DEPTH {
		return Err(err!(
			"The tree is deeper than the limit of {} nodes.", limit::DEPTH;
		Invalid, Input, TooBig, LimitReached));
	}
	Ok(())
}

/// A node's kind and payload map, refusing a `style`, which no document of `fe2o3_text::doc`
/// carries.
fn node_map(n: &Dat) -> Outcome<(NodeKind, &DaticleMap)> {
	match n {
		Dat::Usr(uid, Some(payload)) => {
			let kind = res!(NodeKind::from_code(uid.code()));
			match payload.as_ref() {
				Dat::Map(map) => {
					if map.get(&dat!(KEY_STYLE)).is_some() {
						return Err(err!(
							"A {} carries a 'style', which a document of fe2o3_text::doc has no \
							place for.", kind.label();
						Invalid, Input, Unimplemented));
					}
					Ok((kind, map))
				},
				d => Err(err!("A {} carries a {:?} payload.", kind.label(), d.kind(); Invalid, Input)),
			}
		},
		d => Err(err!("Expected a node, found a {:?}.", d.kind(); Invalid, Input)),
	}
}

fn kids(map: &DaticleMap) -> Outcome<&[Dat]> {
	match map.get(&dat!(KEY_CHILDREN)) {
		None => Ok(&[]),
		Some(Dat::List(v)) => Ok(&v[..]),
		Some(d) => Err(err!("Node children are a {:?}, not a list.", d.kind(); Invalid, Input)),
	}
}

/// A flag field, absent meaning false.
fn flag(
	map:	&DaticleMap,
	key:	&str,
)
	-> Outcome<bool>
{
	match map.get(&dat!(key)) {
		None => Ok(false),
		Some(Dat::Bool(b)) => Ok(*b),
		Some(d) => Err(err!("Field '{}' is a {:?}, not a bool.", key, d.kind(); Invalid, Input)),
	}
}

fn opt_str(
	map:	&DaticleMap,
	key:	&str,
)
	-> Outcome<Option<String>>
{
	match map.get(&dat!(key)) {
		None => Ok(None),
		Some(Dat::Str(s)) => Ok(Some(s.clone())),
		Some(d) => Err(err!("Field '{}' is a {:?}, not a str.", key, d.kind(); Invalid, Input)),
	}
}

fn req_str(
	map:	&DaticleMap,
	key:	&str,
)
	-> Outcome<String>
{
	match res!(opt_str(map, key)) {
		Some(s) => Ok(s),
		None => Err(err!("Field '{}' is missing.", key; Invalid, Input, Missing)),
	}
}

fn strs(
	map:	&DaticleMap,
	key:	&str,
)
	-> Outcome<Vec<String>>
{
	match map.get(&dat!(key)) {
		None => Ok(Vec::new()),
		Some(Dat::List(list)) => {
			let mut out = Vec::with_capacity(list.len());
			for d in list {
				match d {
					Dat::Str(s) => out.push(s.clone()),
					d => return Err(err!(
						"Field '{}' holds a {:?}, not a str.", key, d.kind(); Invalid, Input)),
				}
			}
			Ok(out)
		},
		Some(d) => Err(err!("Field '{}' is a {:?}, not a list.", key, d.kind(); Invalid, Input)),
	}
}

fn attrs(map: &DaticleMap) -> Outcome<Attrs> {
	let flat = res!(strs(map, KEY_PAIRS));
	let mut pairs = Vec::with_capacity(flat.len() / 2);
	for kv in flat.chunks(2) {
		match kv {
			[k, v] => pairs.push((k.clone(), v.clone())),
			_ => return Err(err!(
				"Field '{}' holds an odd number of strings.", KEY_PAIRS; Invalid, Input)),
		}
	}
	Ok(Attrs {
		id:		res!(opt_str(map, KEY_ID)),
		classes:	res!(strs(map, KEY_CLASSES)),
		pairs,
	})
}
