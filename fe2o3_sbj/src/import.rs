//! Importing Markdown: ordinary prose, mapped to the node vocabulary of `SPEC.md` §4.
//!
//! A document's source is its JDAT text form, and nobody writes prose that way. Most prose that
//! exists is Markdown, so this reads the neutral document tree that
//! [`markdown`](oxedyne_fe2o3_text::doc::markdown) produces and maps it to a `doc` tree. What comes out
//! is a tree like any other: it is validated, canonically encoded, hashed and signed by the same
//! code that does it for a document written by hand, because by then it is the same document.
//!
//! # One block, one node
//!
//! The tree is written under `oxeweb/doc/1` ([`SCHEMA_DOC_1`](crate::SCHEMA_DOC_1)), whose page kinds
//! give every construct of [`doc`](oxedyne_fe2o3_text::doc) a node of its own: a table keeps its grid,
//! a rule its place, a span its names, an inline image its source. Nothing is dropped or degraded, so
//! [`export::to_doc`](crate::export::to_doc) gives back the tree this was handed: `to_doc(from_doc(d))
//! == d` for every `d` whose strings already keep §3 rule 5 and whose headings run from 1 to 6. The
//! two corrections made at the door are the only exceptions: a string is normalised to NFC with its
//! stray control characters removed, and a heading level is clamped.

use crate::{
	kinds::{
		NodeKind,
		ADDR_NAME,
		KEY_CHILDREN,
		KEY_CLASSES,
		KEY_COLS,
		KEY_HEAD,
		KEY_HEADER,
		KEY_ID,
		KEY_PAIRS,
		KEY_SRC,
		KEY_TO,
	},
	text::ukid,
};

use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_text::{
	doc::{
		self,
		html,
		markdown,
		Align,
		Attrs,
		Block,
		Inline,
		Row,
		Term,
	},
	unicode::norm,
};

use oxedyne_fe2o3_core::prelude::*;

/// The language a document declares when the caller names none.
pub const DEFAULT_LANG: &'static str = "en";

/// The name a document is titled by when it has no title and the caller supplies no source name.
pub const DEFAULT_STEM: &'static str = "document";

///

/// What the caller knows that the Markdown does not.
///
/// A `doc` node requires a title and a language (§4.2) and Markdown carries neither, so both are
/// settled here.
#[derive(Clone, Debug)]
pub struct Options {
	/// The document's title, when the caller names one. Otherwise the first level 1 heading, and
	/// failing that, the [`stem`](Options::stem).
	pub title:	Option<String>,
	/// The document's language tag, BCP-47.
	pub lang:	String,
	/// The name the source is known by, usually its file stem: the title of last resort.
	pub stem:	String,
}

impl Default for Options {

	fn default() -> Self {
		Self {
			title:	None,
			lang:	DEFAULT_LANG.to_string(),
			stem:	DEFAULT_STEM.to_string(),
		}
	}
}

/// Reads Markdown text and maps it to a document tree.
///
/// The outcome is an error only when the Markdown will not parse, which is when it breaks a limit
/// the parser holds against a hostile document. The mapping itself does not fail: see [`from_doc`].
pub fn from_markdown(
	src:	&str,
	opts:	&Options,
)
	-> Outcome<Dat>
{
	let md = res!(markdown::parse(src));
	Ok(from_doc(&md, opts))
}

/// Reads HTML and maps it to a document tree.
///
/// The same mapping as [`from_markdown`], reached by a second road. Prose written in a form no
/// reader here understands can often be got at through HTML, because the thing that understands it
/// will export HTML -- which is how prose written in Typst reaches a document, its author's own
/// macros already resolved by the tool that defined them.
pub fn from_html(
	src:	&str,
	opts:	&Options,
)
	-> Outcome<Dat>
{
	let doc = res!(html::parse(src));
	Ok(from_doc(&doc, opts))
}

/// Maps a parsed document to an `oxeweb/doc/1` tree, one node to each block and inline.
///
/// The mapping never fails and loses nothing: see the module's "One block, one node". What comes
/// back is a tree the validator accepts under [`SCHEMA_DOC_1`](crate::SCHEMA_DOC_1), and whether it
/// does is [`validate`](crate::validate)'s business rather than a promise made here.
pub fn from_doc(
	md:	&doc::Doc,
	opts:	&Options,
)
	-> Dat
{
	node(
		NodeKind::Doc,
		vec![
			("title",	Dat::Str(title(md, opts))),
			("lang",	Dat::Str(clean(&opts.lang))),
		],
		blocks(&md.blocks),
	)
}

/// The document's title: the caller's, else the first level 1 heading, else the source's name.
///
/// A title of nothing but whitespace is no title, whichever of the three it came from, since a `doc`
/// carries the field whether or not the author thought about it.
fn title(
	md:	&doc::Doc,
	opts:	&Options,
)
	-> String
{
	if let Some(t) = &opts.title {
		if !t.trim().is_empty() {
			return clean(t);
		}
	}
	// The most prominent heading, and not the first level 1. What level a piece is headed
	// by says where the prose came from rather than what it says: an author writing Markdown heads a
	// chapter with a level 1, and the same chapter exported from Typst arrives headed by a level 2,
	// because the exporter keeps level 1 for the document it thinks it is making. Asking for level 1
	// found no title in a whole shelf of books, and titled every one of them after its file.
	if let Some(t) = md.top_heading() {
		if !t.trim().is_empty() {
			return clean(&t);
		}
	}
	clean(&opts.stem)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ BLOCKS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

/// Maps a run of blocks to the flow nodes they become, one to each.
fn blocks(blocks: &[Block]) -> Vec<Dat> {
	blocks.iter().map(block).collect()
}

/// Maps one block to its node. Every block has one, and nothing is dropped, so that
/// [`export::to_doc`](crate::export::to_doc) can give back the block it came from.
fn block(b: &Block) -> Dat {
	match b {
		Block::Heading { level, content } => node(
			NodeKind::Heading,
			vec![("level", Dat::U8(heading_level(*level)))],
			inlines(content),
		),
		Block::Para(content) => node(NodeKind::Para, Vec::new(), inlines(content)),
		Block::List { ordered, items } => node(
			NodeKind::List,
			vec![("ordered", Dat::Bool(*ordered))],
			items.iter().map(|item| node(NodeKind::Item, Vec::new(), blocks(item))).collect(),
		),
		Block::Code { lang, text } => {
			let mut fields = Vec::with_capacity(2);
			if let Some(lang) = lang {
				fields.push(("lang", Dat::Str(clean(lang))));
			}
			fields.push(("text", Dat::Str(clean(text))));
			node(NodeKind::Code, fields, Vec::new())
		},
		// A quote carries no `cite`, because the prose gives none. Attribution is a thing the author
		// writes inside the quotation, and inventing one from it would be a guess.
		Block::Quote(quoted) => node(NodeKind::Quote, Vec::new(), blocks(quoted)),
		Block::Table { head, rows, cols, caption } => {
			let mut fields = Vec::with_capacity(1);
			if !cols.is_empty() {
				fields.push((KEY_COLS, Dat::List(cols.iter().map(|a| dat!(col_align(*a))).collect())));
			}
			let mut kids = Vec::with_capacity(rows.len() + 2);
			if !caption.is_empty() {
				kids.push(node(NodeKind::Caption, Vec::new(), inlines(caption)));
			}
			if let Some(head) = head {
				kids.push(table_row(head, true));
			}
			for row in rows {
				kids.push(table_row(row, false));
			}
			node(NodeKind::Table, fields, kids)
		},
		Block::Rule => node(NodeKind::Rule, Vec::new(), Vec::new()),
		Block::Div { attrs, content } => node(NodeKind::Div, attr_fields(attrs), blocks(content)),
		Block::Figure { attrs, content, caption } => {
			let mut kids = blocks(content);
			if !caption.is_empty() {
				kids.push(node(NodeKind::Caption, Vec::new(), inlines(caption)));
			}
			node(NodeKind::Figure, attr_fields(attrs), kids)
		},
		Block::Terms { attrs, items } => node(
			NodeKind::Terms,
			attr_fields(attrs),
			items.iter().map(term).collect(),
		),
	}
}

/// One row of a table, its head row marked as one.
fn table_row(
	row:	&Row,
	head:	bool,
)
	-> Dat
{
	let fields = if head { vec![(KEY_HEAD, Dat::Bool(true))] } else { Vec::new() };
	let cells = row.0.iter().map(|cell| {
		let fields = if cell.header { vec![(KEY_HEADER, Dat::Bool(true))] } else { Vec::new() };
		node(NodeKind::Cell, fields, inlines(&cell.content))
	}).collect();
	node(NodeKind::Row, fields, cells)
}

/// One entry of a definition list: its label where the term has words, then each definition.
fn term(t: &Term) -> Dat {
	let mut kids = Vec::with_capacity(t.defs.len() + 1);
	if !t.term.is_empty() {
		kids.push(node(NodeKind::Label, Vec::new(), inlines(&t.term)));
	}
	for def in &t.defs {
		kids.push(node(NodeKind::Def, Vec::new(), blocks(def)));
	}
	node(NodeKind::Term, Vec::new(), kids)
}

/// The wire name of a column alignment, one of [`COL_ALIGNMENTS`](crate::kinds::COL_ALIGNMENTS).
pub fn col_align(a: Align) -> &'static str {
	match a {
		Align::None	=> "none",
		Align::Start	=> "start",
		Align::Centre	=> "center",
		Align::End	=> "end",
	}
}

/// The attribute fields of a figure, a definition list, a division, a span or an inline image, each
/// left out when it names nothing.
fn attr_fields(attrs: &Attrs) -> Vec<(&'static str, Dat)> {
	let mut fields = Vec::with_capacity(3);
	if !attrs.classes.is_empty() {
		fields.push((KEY_CLASSES, Dat::List(attrs.classes.iter().map(|c| Dat::Str(clean(c))).collect())));
	}
	if let Some(id) = &attrs.id {
		fields.push((KEY_ID, Dat::Str(clean(id))));
	}
	if !attrs.pairs.is_empty() {
		let mut flat = Vec::with_capacity(attrs.pairs.len() * 2);
		for (k, v) in &attrs.pairs {
			flat.push(Dat::Str(clean(k)));
			flat.push(Dat::Str(clean(v)));
		}
		fields.push((KEY_PAIRS, Dat::List(flat)));
	}
	fields
}

/// A heading's level, held to the 1 to 6 the schema admits.
///
/// No reader has any other level, so this only ever matters for a tree built by hand. A level outside
/// the range is clamped rather than refused, since an import that dies on a heading has told an
/// author nothing they can act on.
fn heading_level(n: u8) -> u8 {
	n.clamp(1, 6)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ INLINES                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// Maps a run of inlines to the inline nodes they become, one to each.
fn inlines(content: &[Inline]) -> Vec<Dat> {
	content.iter().map(inline).collect()
}

/// Maps one inline to its node.
fn inline(item: &Inline) -> Dat {
	match item {
		Inline::Text(s) => text(clean(s)),
		Inline::Emph { strong, content } => node(
			NodeKind::Emph,
			vec![("strong", Dat::Bool(*strong))],
			inlines(content),
		),
		Inline::Link { to, content } => node(
			NodeKind::Link,
			vec![(KEY_TO, address(to))],
			inlines(content),
		),
		// An inline image keeps the source it was written with. SBJ's block `image` addresses a blob
		// by content hash (§4.2), which a path is not; whatever publishes the blob rewrites `src` to
		// `hash` then, and until it does the source is what the prose said.
		Inline::Image { src, alt, attrs } => {
			let mut fields = vec![("alt", Dat::Str(clean(alt))), (KEY_SRC, Dat::Str(clean(src)))];
			fields.extend(attr_fields(attrs));
			node(NodeKind::Img, fields, Vec::new())
		}
		Inline::Code(s) => node(NodeKind::Tt, vec![("text", Dat::Str(clean(s)))], Vec::new()),
		Inline::Span { attrs, content } => node(NodeKind::Span, attr_fields(attrs), inlines(content)),
		Inline::Break => node(NodeKind::Break, Vec::new(), Vec::new()),
		Inline::Sup(content) => node(NodeKind::Sup, Vec::new(), inlines(content)),
		Inline::Sub(content) => node(NodeKind::Sub, Vec::new(), inlines(content)),
	}
}

/// A link's typed address (§4.3).
///
/// Markdown gives a destination exactly as it was written, which is a name and not a content hash: a
/// hash is 32 bytes an author never types, and a URL, a path or a NAMES name are all names as far as
/// the format is concerned.
fn address(to: &str) -> Dat {
	create_dat_map(vec![
		(Dat::Str(ADDR_NAME.to_string()),	Dat::Str(clean(to))),
	])
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ NODES AND STRINGS                                                          │
// └───────────────────────────────────────────────────────────────────────────┘

/// Builds a node from its fields and its children.
///
/// An empty children list is left out entirely rather than written empty, since §3 rule 4 admits one
/// encoding of a node with no children and that is the one without the key.
fn node(
	kind:	NodeKind,
	fields:	Vec<(&'static str, Dat)>,
	kids:	Vec<Dat>,
)
	-> Dat
{
	let mut kv: Vec<(Dat, Dat)> = fields.into_iter()
		.map(|(k, v)| (Dat::Str(k.to_string()), v))
		.collect();
	if !kids.is_empty() {
		kv.push((Dat::Str(KEY_CHILDREN.to_string()), Dat::List(kids)));
	}
	Dat::Usr(ukid(kind), Some(Box::new(create_dat_map(kv))))
}

/// Builds a text run, the one node whose payload is the string itself.
fn text(s: String) -> Dat {
	Dat::Usr(ukid(NodeKind::Text), Some(Box::new(Dat::Str(s))))
}

/// A string as a document may carry it: Unicode NFC, and no control character but tab and newline.
///
/// This is §3 rule 5 applied at the door. Prose in the wild carries a stray carriage return or a
/// vertical tab often enough, and text typed on one machine and pasted from another is decomposed
/// often enough, that an importer which passed either through would refuse its own output at the
/// moment of signing, naming a rule the author never heard of. Both are corrected here instead, and
/// neither changes what the prose says: the composed and decomposed forms of a letter display
/// identically, and a control character displays as nothing at all.
fn clean(s: &str) -> String {
	let stripped: String = s.chars()
		.filter(|c| !c.is_control() || *c == '\t' || *c == '\n')
		.collect();
	norm::nfc(&stripped)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{
		kinds::children_of,
		validate,
		SCHEMA_DOC,
		SCHEMA_DOC_1,
	};

	use oxedyne_fe2o3_text::doc::Cell;

	/// Builds a document of the given blocks, and holds it to the validator.
	///
	/// Every test goes through here, because a mapping that produces a tree the format refuses is a
	/// mapping that is wrong, whatever else the test then checks about it.
	fn imported(blocks: Vec<Block>) -> Outcome<Dat> {
		let md = doc::Doc { blocks };
		let tree = from_doc(&md, &Options::default());
		res!(validate::validate(&tree, SCHEMA_DOC_1));
		Ok(tree)
	}

	/// The kind code and payload of a node.
	fn parts(d: &Dat) -> Outcome<(u16, &Dat)> {
		match d {
			Dat::Usr(uid, Some(payload)) => Ok((uid.code(), payload.as_ref())),
			d => Err(err!("Expected a node, found a {:?}.", d.kind(); Test, Invalid)),
		}
	}

	/// The kind of a node, which every test asserts before it looks inside one.
	fn kind_of(d: &Dat) -> Outcome<NodeKind> {
		let (code, _) = res!(parts(d));
		NodeKind::from_code(code)
	}

	/// The children of a node.
	fn kids(d: &Dat) -> Outcome<Vec<Dat>> {
		let (_, payload) = res!(parts(d));
		children_of(payload)
	}

	/// The `n`th child of a node.
	fn kid(
		d:	&Dat,
		n:	usize,
	)
		-> Outcome<Dat>
	{
		let kids = res!(kids(d));
		match kids.get(n) {
			Some(kid) => Ok(kid.clone()),
			None => Err(err!(
				"The node carries {} children, and child {} was asked for.", kids.len(), n;
			Test, Missing)),
		}
	}

	/// One field of a node's payload map.
	fn field(
		d:	&Dat,
		name:	&str,
	)
		-> Outcome<Dat>
	{
		let (_, payload) = res!(parts(d));
		match payload {
			Dat::Map(map) => match map.get(&dat!(name)) {
				Some(v) => Ok(v.clone()),
				None => Err(err!("The node carries no '{}' field.", name; Test, Missing)),
			},
			d => Err(err!("Expected a map payload, found a {:?}.", d.kind(); Test, Invalid)),
		}
	}

	/// The string a text run carries.
	fn text_of(d: &Dat) -> Outcome<String> {
		match res!(parts(d)) {
			(9, Dat::Str(s)) => Ok(s.clone()),
			(code, payload) => Err(err!(
				"Expected a text run, found kind {} carrying a {:?}.", code, payload.kind();
			Test, Invalid)),
		}
	}

	/// The words a node holds, every text run within it flattened wherever it sits: what a degraded
	/// row says, all told, and no more of how it says it than [`doc::text_of`] keeps.
	fn said(d: &Dat) -> Outcome<String> {
		if let (9, Dat::Str(s)) = res!(parts(d)) {
			return Ok(s.clone());
		}
		let mut s = String::new();
		for kid in res!(kids(d)) {
			s.push_str(&res!(said(&kid)));
		}
		Ok(s)
	}

	/// A run of literal text.
	fn t(s: &str) -> Inline {
		Inline::Text(s.to_string())
	}

	/// A paragraph of one run of literal text.
	fn p(s: &str) -> Block {
		Block::Para(vec![t(s)])
	}

	/// A cell of one run of literal text.
	fn c(s: &str) -> Cell {
		Cell::new(vec![t(s)])
	}

	#[test]
	fn test_a_doc_carries_its_title_and_language_00() -> Outcome<()> {
		let tree = res!(imported(vec![p("A paragraph.")]));
		assert_eq!(res!(kind_of(&tree)), NodeKind::Doc);
		// Both fields are required (§4.2), so both are always there, whatever the Markdown said.
		assert_eq!(res!(field(&tree, "title")), dat!("document"));
		assert_eq!(res!(field(&tree, "lang")), dat!("en"));
		Ok(())
	}

	#[test]
	fn test_a_heading_carries_its_level_and_its_words_01() -> Outcome<()> {
		let tree = res!(imported(vec![
			Block::Heading { level: 3, content: vec![t("A heading")] },
		]));
		let heading = res!(kid(&tree, 0));
		assert_eq!(res!(kind_of(&heading)), NodeKind::Heading);
		assert_eq!(res!(field(&heading, "level")), Dat::U8(3));
		assert_eq!(res!(text_of(&res!(kid(&heading, 0)))), "A heading");
		Ok(())
	}

	#[test]
	fn test_a_heading_level_is_held_to_the_schemas_range_02() -> Outcome<()> {
		// The parser gives 1 to 6 and nothing else, so this is only ever a hand-built tree. It is
		// clamped rather than refused, and the validator is what says the clamp worked.
		let tree = res!(imported(vec![
			Block::Heading { level: 0, content: vec![t("Too high")] },
			Block::Heading { level: 9, content: vec![t("Too low")] },
		]));
		assert_eq!(res!(field(&res!(kid(&tree, 0)), "level")), Dat::U8(1));
		assert_eq!(res!(field(&res!(kid(&tree, 1)), "level")), Dat::U8(6));
		Ok(())
	}

	#[test]
	fn test_a_paragraph_carries_its_inlines_03() -> Outcome<()> {
		let tree = res!(imported(vec![p("A paragraph.")]));
		let para = res!(kid(&tree, 0));
		assert_eq!(res!(kind_of(&para)), NodeKind::Para);
		assert_eq!(res!(text_of(&res!(kid(&para, 0)))), "A paragraph.");
		Ok(())
	}

	#[test]
	fn test_an_empty_paragraph_is_kept_04() -> Outcome<()> {
		// Nothing is dropped, or the export could not give back what the import was handed.
		let tree = res!(imported(vec![p("   \t "), p("Real prose."), Block::Para(Vec::new())]));
		assert_eq!(res!(kids(&tree)).len(), 3);
		assert_eq!(res!(text_of(&res!(kid(&res!(kid(&tree, 1)), 0)))), "Real prose.");
		assert_eq!(res!(kids(&res!(kid(&tree, 2)))).len(), 0);
		Ok(())
	}

	#[test]
	fn test_a_list_carries_its_items_05() -> Outcome<()> {
		let tree = res!(imported(vec![
			Block::List {
				ordered:	true,
				items:		vec![vec![p("One")], vec![p("Two")]],
			},
		]));
		let list = res!(kid(&tree, 0));
		assert_eq!(res!(kind_of(&list)), NodeKind::List);
		assert_eq!(res!(field(&list, "ordered")), Dat::Bool(true));
		assert_eq!(res!(kids(&list)).len(), 2);
		// A list holds items, and an item holds flow content: the paragraph is the item's child.
		let item = res!(kid(&list, 0));
		assert_eq!(res!(kind_of(&item)), NodeKind::Item);
		let para = res!(kid(&item, 0));
		assert_eq!(res!(kind_of(&para)), NodeKind::Para);
		assert_eq!(res!(text_of(&res!(kid(&para, 0)))), "One");
		Ok(())
	}

	#[test]
	fn test_a_list_of_no_items_is_kept_under_doc_1_06() -> Outcome<()> {
		// `oxeweb/doc/1` carries an empty list; `oxeweb/doc/0` still marks a list `item+`.
		let tree = res!(imported(vec![
			Block::List { ordered: false, items: Vec::new() },
			p("After."),
		]));
		assert_eq!(res!(kids(&tree)).len(), 2);
		assert_eq!(res!(kind_of(&res!(kid(&tree, 0)))), NodeKind::List);
		assert!(validate::validate(&tree, SCHEMA_DOC).is_err(), "doc/0 admitted an empty list.");
		Ok(())
	}

	#[test]
	fn test_a_nested_list_nests_07() -> Outcome<()> {
		let tree = res!(imported(vec![
			Block::List {
				ordered:	false,
				items:		vec![vec![
					p("Outer"),
					Block::List {
						ordered:	false,
						items:		vec![vec![p("Inner")]],
					},
				]],
			},
		]));
		// doc > list > item > [para, list] > item > para > text.
		let outer_item = res!(kid(&res!(kid(&tree, 0)), 0));
		assert_eq!(res!(kids(&outer_item)).len(), 2);
		let inner_list = res!(kid(&outer_item, 1));
		assert_eq!(res!(kind_of(&inner_list)), NodeKind::List);
		let inner_para = res!(kid(&res!(kid(&inner_list, 0)), 0));
		assert_eq!(res!(text_of(&res!(kid(&inner_para, 0)))), "Inner");
		Ok(())
	}

	#[test]
	fn test_a_code_block_carries_its_language_and_its_text_08() -> Outcome<()> {
		let tree = res!(imported(vec![
			Block::Code {
				lang:	Some("rust".to_string()),
				text:	"fn main() {}\n".to_string(),
			},
		]));
		let code = res!(kid(&tree, 0));
		assert_eq!(res!(kind_of(&code)), NodeKind::Code);
		assert_eq!(res!(field(&code, "lang")), dat!("rust"));
		// The line structure is preserved: a listing is one string, not a run of spans.
		assert_eq!(res!(field(&code, "text")), dat!("fn main() {}\n"));
		Ok(())
	}

	#[test]
	fn test_a_code_block_naming_no_language_carries_none_09() -> Outcome<()> {
		let tree = res!(imported(vec![
			Block::Code { lang: None, text: "plain".to_string() },
			Block::Code { lang: Some("  ".to_string()), text: "plain".to_string() },
		]));
		// The field is optional (§4.2), and the canonical encoding of an absent optional is its
		// absence, so a fence that named no language carries no key. A blank name is still a name,
		// and the lossless import keeps it as written.
		let code = res!(kid(&tree, 0));
		assert!(field(&code, "lang").is_err(), "A code block invented a language.");
		assert_eq!(res!(field(&code, "text")), dat!("plain"));
		let code = res!(kid(&tree, 1));
		assert_eq!(res!(field(&code, "lang")), dat!("  "));
		assert_eq!(res!(field(&code, "text")), dat!("plain"));
		Ok(())
	}

	#[test]
	fn test_a_quote_carries_its_blocks_and_no_cite_10() -> Outcome<()> {
		let tree = res!(imported(vec![Block::Quote(vec![p("A quoted line.")])]));
		let quote = res!(kid(&tree, 0));
		assert_eq!(res!(kind_of(&quote)), NodeKind::Quote);
		// Markdown gives no attribution, so none is invented.
		assert!(field(&quote, "cite").is_err(), "A quote invented an attribution.");
		let para = res!(kid(&quote, 0));
		assert_eq!(res!(kind_of(&para)), NodeKind::Para);
		assert_eq!(res!(text_of(&res!(kid(&para, 0)))), "A quoted line.");
		Ok(())
	}

	#[test]
	fn test_emphasis_carries_its_strength_11() -> Outcome<()> {
		let tree = res!(imported(vec![
			Block::Para(vec![
				Inline::Emph { strong: true, content: vec![t("loud")] },
				Inline::Emph { strong: false, content: vec![t("quiet")] },
			]),
		]));
		let para = res!(kid(&tree, 0));
		let strong = res!(kid(&para, 0));
		assert_eq!(res!(kind_of(&strong)), NodeKind::Emph);
		assert_eq!(res!(field(&strong, "strong")), Dat::Bool(true));
		assert_eq!(res!(text_of(&res!(kid(&strong, 0)))), "loud");
		assert_eq!(res!(field(&res!(kid(&para, 1)), "strong")), Dat::Bool(false));
		Ok(())
	}

	#[test]
	fn test_a_link_carries_a_named_address_12() -> Outcome<()> {
		let tree = res!(imported(vec![
			Block::Para(vec![
				Inline::Link {
					to:		"news.cricket".to_string(),
					content:	vec![t("a link")],
				},
			]),
		]));
		let link = res!(kid(&res!(kid(&tree, 0)), 0));
		assert_eq!(res!(kind_of(&link)), NodeKind::Link);
		// A destination as written is a name, never a content hash (§4.3).
		assert_eq!(res!(field(&link, "to")), mapdat!{ "name" => dat!("news.cricket") });
		assert_eq!(res!(text_of(&res!(kid(&link, 0)))), "a link");
		Ok(())
	}

	#[test]
	fn test_a_rule_is_a_rule_13() -> Outcome<()> {
		let tree = res!(imported(vec![p("Before."), Block::Rule, p("After.")]));
		assert_eq!(res!(kids(&tree)).len(), 3);
		assert_eq!(res!(kind_of(&res!(kid(&tree, 1)))), NodeKind::Rule);
		Ok(())
	}

	#[test]
	fn test_an_image_keeps_its_source_and_alt_text_14() -> Outcome<()> {
		let tree = res!(imported(vec![
			Block::Para(vec![
				t("Look: "),
				Inline::Image {
					src:	"tree.png".to_string(),
					alt:	"A diagram of a tree".to_string(),
					attrs:	Attrs::default(),
				},
			]),
		]));
		let img = res!(kid(&res!(kid(&tree, 0)), 1));
		assert_eq!(res!(kind_of(&img)), NodeKind::Img);
		assert_eq!(res!(field(&img, KEY_SRC)), dat!("tree.png"));
		assert_eq!(res!(field(&img, "alt")), dat!("A diagram of a tree"));
		assert!(field(&img, "hash").is_err(), "An unresolved image carried a hash.");
		Ok(())
	}

	#[test]
	fn test_an_image_with_no_alt_text_is_still_an_image_15() -> Outcome<()> {
		let tree = res!(imported(vec![
			Block::Para(vec![
				t("Words."),
				Inline::Image { src: "tree.png".to_string(), alt: String::new(), attrs: Attrs::default() },
			]),
		]));
		let img = res!(kid(&res!(kid(&tree, 0)), 1));
		assert_eq!(res!(field(&img, "alt")), dat!(""));
		Ok(())
	}

	#[test]
	fn test_an_inline_code_span_is_a_tt_16() -> Outcome<()> {
		let tree = res!(imported(vec![
			Block::Para(vec![t("Run "), Inline::Code("cargo test".to_string()), t(" first.")]),
		]));
		let span = res!(kid(&res!(kid(&tree, 0)), 1));
		assert_eq!(res!(kind_of(&span)), NodeKind::Tt);
		assert_eq!(res!(field(&span, "text")), dat!("cargo test"));
		Ok(())
	}

	#[test]
	fn test_a_hard_break_is_a_break_17() -> Outcome<()> {
		let tree = res!(imported(vec![
			Block::Para(vec![t("One line"), Inline::Break, t("and the next")]),
		]));
		let brk = res!(kid(&res!(kid(&tree, 0)), 1));
		assert_eq!(res!(kind_of(&brk)), NodeKind::Break);
		Ok(())
	}

	#[test]
	fn test_the_title_is_the_callers_18() -> Outcome<()> {
		let md = doc::Doc {
			blocks: vec![Block::Heading { level: 1, content: vec![t("The heading")] }],
		};
		let opts = Options {
			title:	Some("The caller's title".to_string()),
			..Options::default()
		};
		let tree = from_doc(&md, &opts);
		res!(validate::validate(&tree, SCHEMA_DOC_1));
		assert_eq!(res!(field(&tree, "title")), dat!("The caller's title"));
		Ok(())
	}

	#[test]
	fn test_the_title_falls_back_to_the_first_heading_19() -> Outcome<()> {
		let md = doc::Doc {
			blocks: vec![
				Block::Heading { level: 2, content: vec![t("Not the title")] },
				Block::Heading {
					level:		1,
					content:	vec![
						t("A "),
						Inline::Emph { strong: true, content: vec![t("loud")] },
						t(" title"),
					],
				},
				Block::Heading { level: 1, content: vec![t("The second one")] },
			],
		};
		let tree = from_doc(&md, &Options::default());
		res!(validate::validate(&tree, SCHEMA_DOC_1));
		// The first level 1 heading, flattened: emphasis inside a title keeps its words.
		assert_eq!(res!(field(&tree, "title")), dat!("A loud title"));
		Ok(())
	}

	#[test]
	fn test_the_title_falls_back_to_the_stem_20() -> Outcome<()> {
		let md = doc::Doc { blocks: vec![p("No heading here.")] };
		let opts = Options {
			// A title of whitespace is no title, and neither is a level 2 heading.
			title:	Some("  ".to_string()),
			stem:	"the_file_name".to_string(),
			..Options::default()
		};
		let tree = from_doc(&md, &opts);
		res!(validate::validate(&tree, SCHEMA_DOC_1));
		assert_eq!(res!(field(&tree, "title")), dat!("the_file_name"));
		Ok(())
	}

	#[test]
	fn test_the_language_is_the_callers_21() -> Outcome<()> {
		let md = doc::Doc { blocks: vec![p("Une phrase.")] };
		let opts = Options {
			lang:	"fr".to_string(),
			..Options::default()
		};
		let tree = from_doc(&md, &opts);
		res!(validate::validate(&tree, SCHEMA_DOC_1));
		assert_eq!(res!(field(&tree, "lang")), dat!("fr"));
		Ok(())
	}

	#[test]
	fn test_a_string_is_normalised_and_stripped_22() -> Outcome<()> {
		// §3 rule 5: the composed and decomposed forms of a letter display identically and hash
		// differently, and a control character displays as nothing. Text that reaches the signer
		// carrying either would be refused there, naming a rule the author never heard of.
		let tree = res!(imported(vec![
			Block::Para(vec![t("Cafe\u{0301}\r, said the \u{000B}sign.")]),
		]));
		let s = res!(text_of(&res!(kid(&res!(kid(&tree, 0)), 0))));
		assert_eq!(s, "Café, said the sign.");
		res!(crate::canon::check(&tree));
		Ok(())
	}

	#[test]
	fn test_an_empty_document_validates_23() -> Outcome<()> {
		// A doc is `flow*`, so nothing at all is a document, and an empty children list is not the
		// canonical way to say so (§3 rule 4).
		let tree = res!(imported(Vec::new()));
		assert!(kids(&tree).is_ok());
		assert_eq!(res!(kids(&tree)).len(), 0);
		assert!(field(&tree, KEY_CHILDREN).is_err(), "An empty children list was written.");
		res!(crate::canon::check(&tree));
		Ok(())
	}

	#[test]
	fn test_every_block_kind_at_once_validates_24() -> Outcome<()> {
		// The whole vocabulary the importer reaches, in one document, held to the validator and to
		// the canonical encoding rules that the signing path would hold it to.
		let tree = res!(imported(vec![
			Block::Heading { level: 1, content: vec![t("A title")] },
			Block::Para(vec![
				t("Prose with "),
				Inline::Emph { strong: true, content: vec![t("emphasis")] },
				t(", a "),
				Inline::Link { to: "somewhere".to_string(), content: vec![t("link")] },
				t(", a "),
				Inline::Code("span".to_string()),
				Inline::Break,
				Inline::Image { src: "x.png".to_string(), alt: "a picture".to_string(), attrs: Attrs::default() },
			]),
			Block::Rule,
			Block::Table {
				head:	Some(Row(vec![c("Name"), c("Age")])),
				rows:	vec![Row(vec![c("Alice"), c("30")])],
				cols:	vec![Align::Start, Align::None],
				caption:	Vec::new(),
			},
			Block::Quote(vec![
				p("Quoted."),
				Block::List { ordered: false, items: vec![vec![p("In a quote")]] },
			]),
			Block::List {
				ordered:	true,
				items:		vec![
					vec![p("One"), Block::Code { lang: None, text: "x".to_string() }],
					vec![p("Two")],
				],
			},
			Block::Code { lang: Some("rust".to_string()), text: "fn main() {}".to_string() },
		]));
		let stats = res!(validate::validate(&tree, SCHEMA_DOC_1));
		assert!(stats.nodes > 15, "The document lost most of itself: {:?}", stats);
		res!(crate::canon::check(&tree));
		Ok(())
	}

	#[test]
	fn test_an_imported_tree_signs_and_reads_back_25() -> Outcome<()> {
		// The claim the importer makes is that what comes out is a document, and this is that claim
		// rather than an assertion about it: the tree goes through the whole path a compile puts one
		// through -- validated, canonically encoded, hashed, signed -- and is then read back the way
		// a reader reads it, which verifies every one of those in turn.
		let pair = res!(crate::key::KeyPair::generate());
		let signer = res!(pair.signer());
		let tree = res!(imported(vec![
			Block::Heading { level: 1, content: vec![t("A title")] },
			Block::Para(vec![
				t("Prose, "),
				Inline::Emph { strong: false, content: vec![t("emphasised")] },
				t(", and a "),
				Inline::Link { to: "news.cricket".to_string(), content: vec![t("link")] },
				t("."),
			]),
			Block::Rule,
			Block::List { ordered: false, items: vec![vec![p("An item")]] },
			Block::Quote(vec![p("A quoted line.")]),
			Block::Code { lang: Some("rust".to_string()), text: "fn main() {}".to_string() },
		]));
		let buf = res!(crate::doc::write(&tree, SCHEMA_DOC_1, &signer, 0));
		let read = res!(crate::doc::read(&buf));
		assert_eq!(read.tree(), &tree, "An imported document did not survive its own signing path.");
		Ok(())
	}

	#[test]
	fn test_the_page_constructs_keep_their_parts_in_order_26a() -> Outcome<()> {
		let tree = res!(imported(vec![
			Block::Figure {
				attrs:		Attrs { id: Some("f1".to_string()), ..Default::default() },
				content:	vec![p("Content.")],
				caption:	vec![t("Caption.")],
			},
			Block::Terms {
				attrs:	Attrs { classes: vec!["gloss".to_string()], ..Default::default() },
				items:	vec![Term {
					term:	vec![t("Term")],
					defs:	vec![vec![Block::Para(vec![t("m"), Inline::Sup(vec![t("2")])])]],
				}],
			},
		]));
		let fig = res!(kid(&tree, 0));
		assert_eq!(res!(kind_of(&fig)), NodeKind::Figure);
		assert_eq!(res!(field(&fig, KEY_ID)), dat!("f1"));
		assert_eq!(res!(kind_of(&res!(kid(&fig, 0)))), NodeKind::Para);
		assert_eq!(res!(kind_of(&res!(kid(&fig, 1)))), NodeKind::Caption);
		let terms = res!(kid(&tree, 1));
		assert_eq!(res!(field(&terms, KEY_CLASSES)), Dat::List(vec![dat!("gloss")]));
		let term = res!(kid(&terms, 0));
		assert_eq!(res!(kind_of(&res!(kid(&term, 0)))), NodeKind::Label);
		let def = res!(kid(&term, 1));
		assert_eq!(res!(kind_of(&def)), NodeKind::Def);
		let sup = res!(kid(&res!(kid(&def, 0)), 1));
		assert_eq!(res!(kind_of(&sup)), NodeKind::Sup);
		res!(crate::canon::check(&tree));
		Ok(())
	}

	#[test]
	fn test_a_table_keeps_its_grid_26() -> Outcome<()> {
		let tree = res!(imported(vec![
			Block::Table {
				head:	Some(Row(vec![c("Name"), c("Age")])),
				rows:	vec![
					Row(vec![c("Alice"), c("30")]),
					Row(vec![c("Bob"), c("25")]),
				],
				cols:	vec![Align::Start, Align::End],
				caption:	vec![t("People")],
			},
		]));
		let table = res!(kid(&tree, 0));
		assert_eq!(res!(kind_of(&table)), NodeKind::Table);
		assert_eq!(res!(field(&table, KEY_COLS)), Dat::List(vec![dat!("start"), dat!("end")]));
		assert_eq!(res!(kids(&table)).len(), 4);
		assert_eq!(res!(kind_of(&res!(kid(&table, 0)))), NodeKind::Caption);
		let head = res!(kid(&table, 1));
		assert_eq!(res!(field(&head, KEY_HEAD)), Dat::Bool(true));
		assert!(field(&res!(kid(&table, 2)), KEY_HEAD).is_err(), "A body row carried 'head'.");
		let cell = res!(kid(&res!(kid(&table, 3)), 1));
		assert_eq!(res!(kind_of(&cell)), NodeKind::Cell);
		assert_eq!(&res!(said(&cell)), "25");
		res!(crate::canon::check(&tree));
		Ok(())
	}

	#[test]
	fn test_a_header_cell_in_a_body_row_is_marked_27() -> Outcome<()> {
		let tree = res!(imported(vec![
			Block::Table {
				head:	None,
				rows:	vec![Row(vec![
					Cell { content: vec![t("Alice")], header: true },
					Cell::new(vec![Inline::Emph { strong: true, content: vec![t("30")] }]),
				])],
				cols:	Vec::new(),
				caption:	Vec::new(),
			},
		]));
		let table = res!(kid(&tree, 0));
		assert!(field(&table, KEY_COLS).is_err(), "No columns were written as an empty list.");
		let row = res!(kid(&table, 0));
		assert_eq!(res!(field(&res!(kid(&row, 0)), KEY_HEADER)), Dat::Bool(true));
		assert!(field(&res!(kid(&row, 1)), KEY_HEADER).is_err(), "A plain cell said 'header'.");
		Ok(())
	}

	#[test]
	fn test_a_page_kind_is_refused_under_doc_0_28() -> Outcome<()> {
		// A table is a kind this reader knows, so under `oxeweb/doc/0` it is refused by name and
		// never read through a fallback.
		let tree = res!(imported(vec![Block::Rule]));
		let e = match validate::validate(&tree, SCHEMA_DOC) {
			Ok(_) => return Err(err!("doc/0 admitted a rule."; Test, Unexpected)),
			Err(e) => fmt!("{}", e),
		};
		assert!(e.contains("rule") && e.contains(SCHEMA_DOC), "Unhelpful refusal: {}", e);
		Ok(())
	}

}
