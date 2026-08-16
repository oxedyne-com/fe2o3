//! Importing Markdown: ordinary prose, mapped to the node vocabulary of `SPEC.md` §4.
//!
//! A document's source is its JDAT text form, and nobody writes prose that way. Most prose that
//! exists is Markdown, so this reads the neutral document tree that
//! [`markdown`](oxedyne_fe2o3_text::doc::markdown) produces and maps it to a `doc` tree. What comes out
//! is a tree like any other: it is validated, canonically encoded, hashed and signed by the same
//! code that does it for a document written by hand, because by then it is the same document.
//!
//! # Where the two vocabularies do not meet
//!
//! The v0 vocabulary is closed, and Markdown says four things it has no room for. None of them is
//! a reason to grow the vocabulary, and each is handled by saying less rather than by saying it
//! wrongly:
//!
//! - A **thematic break** is dropped. v0 has no kind for one, and there is nothing it can degrade to
//!   that means what it meant.
//! - A **table** becomes a box of paragraphs, one to a row. v0 has no kind for a grid, and a box is
//!   what says "these things are one thing", which is as much of a table as v0 says.
//! - An **image** becomes its alt text. SBJ addresses an image by the content hash of the blob
//!   (§4.2), and Markdown gives a path; nothing here can resolve one into the other, and an image
//!   node with an invented hash would address a blob that does not exist.
//! - An **inline code span** becomes its characters. SBJ's `code` is flow content and a paragraph
//!   admits inline content only, so a code span cannot sit where Markdown puts it.
//!
//! Each of these loses something, and each loses it visibly: the words survive, and only the mark-up
//! around them goes. A mapping that cannot be exact should be lossy in the direction of the prose.

use crate::{
	kinds::{
		NodeKind,
		ADDR_NAME,
		KEY_CHILDREN,
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
		Block,
		Inline,
		Row,
	},
	unicode::norm,
};

use oxedyne_fe2o3_core::prelude::*;

/// The language a document declares when the caller names none.
pub const DEFAULT_LANG: &'static str = "en";

/// The name a document is titled by when it has no title and the caller supplies no source name.
pub const DEFAULT_STEM: &'static str = "document";

/// What a degraded table's cells are divided by, once the grid that divided them is gone.
///
/// A table's rows become paragraphs (see [`block`]), so a cell boundary either survives in the prose
/// or does not survive at all. A pipe is what a reader of plain text reads a column boundary as, and
/// it survives being laid out: a tab or a run of spaces is whitespace, and whatever renders the
/// paragraph is free to collapse it, which would run two columns into one word.
const CELL_SEP: &'static str = " | ";

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

/// Maps a parsed Markdown document to a document tree.
///
/// The mapping never fails. Markdown has no syntax errors, only text that means less than the author
/// hoped, and the three things v0 has no room for are dropped or degraded rather than refused. What
/// comes back is a tree the validator accepts, and whether it does is [`validate`](crate::validate)'s
/// business rather than a promise made here.
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

/// Maps a run of blocks to the flow nodes they become, leaving out the ones that become nothing.
fn blocks(blocks: &[Block]) -> Vec<Dat> {
	let mut out = Vec::with_capacity(blocks.len());
	for b in blocks {
		if let Some(node) = block(b) {
			out.push(node);
		}
	}
	out
}

/// Maps one block, or `None` where the vocabulary has no room for it.
fn block(b: &Block) -> Option<Dat> {
	match b {
		Block::Heading { level, content } => Some(node(
			NodeKind::Heading,
			vec![("level", Dat::U8(heading_level(*level)))],
			inlines(content),
		)),
		Block::Para(content) => {
			// Whitespace is not a paragraph. A blank one carries no words and renders as nothing, so
			// carrying it would give one document two addresses for the same prose.
			if doc::text_of(content).trim().is_empty() {
				return None;
			}
			Some(node(NodeKind::Para, Vec::new(), inlines(content)))
		},
		Block::List { ordered, items } => {
			let kids: Vec<Dat> = items.iter()
				.map(|item| node(NodeKind::Item, Vec::new(), blocks(item)))
				.collect();
			// SPEC §4.2 marks a list `item+`, so a list of no items is refused by the validator. A
			// list that lost every item is a list of nothing, which is what dropping it says.
			if kids.is_empty() {
				return None;
			}
			Some(node(NodeKind::List, vec![("ordered", Dat::Bool(*ordered))], kids))
		},
		Block::Code { lang, text } => {
			let mut fields = Vec::with_capacity(2);
			// The language is optional, and a fence that named none, or named nothing but
			// whitespace, has none to declare.
			if let Some(lang) = lang {
				if !lang.trim().is_empty() {
					fields.push(("lang", Dat::Str(clean(lang))));
				}
			}
			fields.push(("text", Dat::Str(clean(text))));
			Some(node(NodeKind::Code, fields, Vec::new()))
		},
		// A quote carries no `cite`, because Markdown gives none. Attribution is a thing the author
		// writes inside the quotation, and inventing one from it would be a guess.
		Block::Quote(quoted) => Some(node(NodeKind::Quote, Vec::new(), self::blocks(quoted))),
		// A table degrades to a box of paragraphs, one to a row, each row's cells divided by
		// CELL_SEP. v0 has no kind for a grid (§4.2) and the vocabulary is frozen, so no mapping
		// keeps the grid; what any mapping can keep is every word, in the order it was written, and
		// the rows it was written in. A box is flow content that holds flow content, so a run of
		// paragraphs that belong together is the thing it is for: it says "these paragraphs are one
		// thing", which is the most of a table v0 says.
		//
		// The header row is not marked out from the rest. It comes first, as it did, and that is all.
		// Emphasising it would say the author wrote emphasis, and they wrote a header; the bold a
		// browser puts on a header cell is a rendering convention and not something the prose said.
		// This is the reason a quote invents no `cite`, applied a second time.
		//
		// The column alignments go with the grid. An alignment is a column's, there are no columns
		// left to carry one, and §4.4's `align` style belongs to a whole node rather than to a slice
		// down the middle of several: a row is not a column, so there is nowhere true to put it.
		Block::Table { head, rows, .. } => {
			let mut kids = Vec::with_capacity(rows.len() + 1);
			if let Some(head) = head {
				if let Some(para) = table_row(head) {
					kids.push(para);
				}
			}
			for row in rows {
				if let Some(para) = table_row(row) {
					kids.push(para);
				}
			}
			// A table that said nothing renders as nothing, as a blank paragraph does.
			if kids.is_empty() {
				return None;
			}
			Some(node(NodeKind::Boxx, Vec::new(), kids))
		},
		// A thematic break is dropped: v0 has no kind for one (§4.2), and the vocabulary is frozen. A
		// rule is a division between passages and nothing else, so there is no node it degrades to
		// that keeps what it meant -- an empty paragraph or a box would each say something the author
		// did not.
		Block::Rule => None,
		// A division degrades to a box of its blocks. A box is flow content holding flow content, which
		// is what a division is, and it says "these blocks are one thing" without keeping the id and
		// classes v0 has no vocabulary for (§4.4 names styles locally, not by class). An empty division
		// says nothing and is dropped, as a blank paragraph is.
		Block::Div { content, .. } => {
			let kids = self::blocks(content);
			if kids.is_empty() {
				return None;
			}
			Some(node(NodeKind::Boxx, Vec::new(), kids))
		},
	}
}

/// One row of a table, as the paragraph it degrades to, or `None` where the row says nothing.
///
/// A cell holds inline content and a paragraph admits inline content, so a cell's inlines are carried
/// across whole: the emphasis, the links and the words are all the row's, and only the cell walls go.
fn table_row(row: &Row) -> Option<Dat> {
	// A row of empty cells carries no words, and a paragraph of nothing but separators would say
	// something the author did not. It is dropped, as a blank paragraph is.
	if row.0.iter().all(|cell| cell.text_of().trim().is_empty()) {
		return None;
	}
	let mut out = Vec::new();
	for (n, cell) in row.0.iter().enumerate() {
		if n > 0 {
			push_text(&mut out, CELL_SEP);
		}
		out.extend(inlines(&cell.0));
	}
	Some(node(NodeKind::Para, Vec::new(), out))
}

/// A heading's level, held to the 1 to 6 the schema admits.
///
/// Markdown has no other level, so this only ever matters for a tree built by hand. A level outside
/// the range is clamped rather than refused, since an import that dies on a heading has told an
/// author nothing they can act on.
fn heading_level(n: u8) -> u8 {
	n.clamp(1, 6)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ INLINES                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// Maps a run of inlines to the inline nodes they become.
fn inlines(content: &[Inline]) -> Vec<Dat> {
	let mut out = Vec::with_capacity(content.len());
	for item in content {
		match item {
			Inline::Text(s) => push_text(&mut out, s),
			Inline::Emph { strong, content } => out.push(node(
				NodeKind::Emph,
				vec![("strong", Dat::Bool(*strong))],
				inlines(content),
			)),
			Inline::Link { to, content } => out.push(node(
				NodeKind::Link,
				vec![(KEY_TO, address(to))],
				inlines(content),
			)),
			// An image degrades to its alt text. An `image` node addresses its blob by content hash
			// (§4.2) and Markdown gives a path, so nothing here can build one: resolving a path to a
			// blob, hashing it and publishing it is the business of whatever puts blobs on the
			// oxeweb, and a hash invented to fill the field would address nothing.
			Inline::Image { alt, .. } => push_text(&mut out, alt),
			// A code span degrades to its characters. SBJ's `code` is flow content and a paragraph
			// admits inline content only, so a code node cannot sit where Markdown puts this. The
			// characters are what the span says; the monospace is how it looked.
			Inline::Code(s) => push_text(&mut out, s),
			// A hard break is a newline in a text run. §3 rule 5 permits one in a string, which is
			// what the `every_kind` fixture carries.
			Inline::Break => out.push(text("\n".to_string())),
			// A span degrades to its content, flattened into the line. v0 has no generic inline grouping
			// kind (§4.2), and the id and classes it carries have no vocabulary here, so what it keeps is
			// every word and every emphasis inside it, in order, exactly as a code span keeps its
			// characters.
			Inline::Span { content, .. } => out.extend(inlines(content)),
		}
	}
	out
}

/// Pushes a text run, unless it would say nothing.
fn push_text(
	out:	&mut Vec<Dat>,
	s:	&str,
) {
	let s = clean(s);
	if !s.is_empty() {
		out.push(text(s));
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
	fields:	Vec<(&str, Dat)>,
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
	};

	use oxedyne_fe2o3_text::doc::{
		Align,
		Cell,
	};

	/// Builds a document of the given blocks, and holds it to the validator.
	///
	/// Every test goes through here, because a mapping that produces a tree the format refuses is a
	/// mapping that is wrong, whatever else the test then checks about it.
	fn imported(blocks: Vec<Block>) -> Outcome<Dat> {
		let md = doc::Doc { blocks };
		let tree = from_doc(&md, &Options::default());
		res!(validate::validate(&tree, SCHEMA_DOC));
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
		Cell(vec![t(s)])
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
	fn test_an_empty_paragraph_is_skipped_04() -> Outcome<()> {
		// A paragraph of nothing but whitespace renders as nothing, so it is nothing.
		let tree = res!(imported(vec![p("   \t "), p("Real prose."), Block::Para(Vec::new())]));
		assert_eq!(res!(kids(&tree)).len(), 1);
		assert_eq!(res!(text_of(&res!(kid(&res!(kid(&tree, 0)), 0)))), "Real prose.");
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
	fn test_a_list_of_no_items_is_dropped_06() -> Outcome<()> {
		// SPEC §4.2 marks a list `item+`, so an empty one would be refused by the validator, which is
		// what running this through it proves.
		let tree = res!(imported(vec![
			Block::List { ordered: false, items: Vec::new() },
			p("After."),
		]));
		assert_eq!(res!(kids(&tree)).len(), 1);
		assert_eq!(res!(kind_of(&res!(kid(&tree, 0)))), NodeKind::Para);
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
		// absence, so a fence that named no language carries no key.
		for n in 0..2 {
			let code = res!(kid(&tree, n));
			assert!(field(&code, "lang").is_err(), "A code block invented a language.");
			assert_eq!(res!(field(&code, "text")), dat!("plain"));
		}
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
	fn test_a_rule_is_dropped_13() -> Outcome<()> {
		// v0 has no thematic break, and the vocabulary is frozen, so the rule goes and the prose stays.
		let tree = res!(imported(vec![p("Before."), Block::Rule, p("After.")]));
		let kids = res!(kids(&tree));
		assert_eq!(kids.len(), 2, "A rule left something behind: {:?}", kids);
		assert_eq!(res!(text_of(&res!(kid(&kids[0], 0)))), "Before.");
		assert_eq!(res!(text_of(&res!(kid(&kids[1], 0)))), "After.");
		Ok(())
	}

	#[test]
	fn test_an_image_degrades_to_its_alt_text_14() -> Outcome<()> {
		// An `image` node addresses a blob by content hash, and Markdown gives a path, so the words
		// survive and the picture does not.
		let tree = res!(imported(vec![
			Block::Para(vec![
				t("Look: "),
				Inline::Image {
					src:	"tree.png".to_string(),
					alt:	"A diagram of a tree".to_string(),
				},
			]),
		]));
		let para = res!(kid(&tree, 0));
		let alt = res!(kid(&para, 1));
		assert_eq!(res!(kind_of(&alt)), NodeKind::Text);
		assert_eq!(res!(text_of(&alt)), "A diagram of a tree");
		Ok(())
	}

	#[test]
	fn test_an_image_with_no_alt_text_says_nothing_15() -> Outcome<()> {
		let tree = res!(imported(vec![
			Block::Para(vec![
				t("Words."),
				Inline::Image { src: "tree.png".to_string(), alt: String::new() },
			]),
		]));
		assert_eq!(res!(kids(&res!(kid(&tree, 0)))).len(), 1);
		Ok(())
	}

	#[test]
	fn test_an_inline_code_span_degrades_to_text_16() -> Outcome<()> {
		// SBJ's `code` is flow content and a para admits inline content only, so a code node cannot
		// sit here at all. That the tree validates is the whole point of the degrading.
		let tree = res!(imported(vec![
			Block::Para(vec![t("Run "), Inline::Code("cargo test".to_string()), t(" first.")]),
		]));
		let para = res!(kid(&tree, 0));
		let span = res!(kid(&para, 1));
		assert_eq!(res!(kind_of(&span)), NodeKind::Text);
		assert_eq!(res!(text_of(&span)), "cargo test");
		Ok(())
	}

	#[test]
	fn test_a_hard_break_is_a_newline_17() -> Outcome<()> {
		let tree = res!(imported(vec![
			Block::Para(vec![t("One line"), Inline::Break, t("and the next")]),
		]));
		let para = res!(kid(&tree, 0));
		let brk = res!(kid(&para, 1));
		assert_eq!(res!(kind_of(&brk)), NodeKind::Text);
		assert_eq!(res!(text_of(&brk)), "\n");
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
		res!(validate::validate(&tree, SCHEMA_DOC));
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
		res!(validate::validate(&tree, SCHEMA_DOC));
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
		res!(validate::validate(&tree, SCHEMA_DOC));
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
		res!(validate::validate(&tree, SCHEMA_DOC));
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
				Inline::Image { src: "x.png".to_string(), alt: "a picture".to_string() },
			]),
			Block::Rule,
			Block::Table {
				head:	Some(Row(vec![c("Name"), c("Age")])),
				rows:	vec![Row(vec![c("Alice"), c("30")])],
				cols:	vec![Align::Start, Align::None],
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
		let stats = res!(validate::validate(&tree, SCHEMA_DOC));
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
		let buf = res!(crate::doc::write(&tree, SCHEMA_DOC, &signer, 0));
		let read = res!(crate::doc::read(&buf));
		assert_eq!(read.tree(), &tree, "An imported document did not survive its own signing path.");
		Ok(())
	}

	#[test]
	fn test_a_table_degrades_to_a_box_of_rows_26() -> Outcome<()> {
		// v0 has no kind for a grid and the vocabulary is frozen, so the grid goes and every word
		// stays. That the box validates is the whole of the claim: a box is flow content that holds
		// flow content, so a paragraph to a row is a thing a document may say.
		let tree = res!(imported(vec![
			Block::Table {
				head:	Some(Row(vec![c("Name"), c("Age")])),
				rows:	vec![
					Row(vec![c("Alice"), c("30")]),
					Row(vec![c("Bob"), c("25")]),
				],
				cols:	vec![Align::Start, Align::End],
			},
		]));
		let boxx = res!(kid(&tree, 0));
		assert_eq!(res!(kind_of(&boxx)), NodeKind::Boxx);
		// A paragraph to a row, the header's among them, and nothing lost but the walls between the
		// cells.
		assert_eq!(res!(kids(&boxx)).len(), 3);
		for (n, expected) in ["Name | Age", "Alice | 30", "Bob | 25"].iter().enumerate() {
			let para = res!(kid(&boxx, n));
			assert_eq!(res!(kind_of(&para)), NodeKind::Para);
			assert_eq!(&res!(said(&para)), expected);
		}
		res!(crate::canon::check(&tree));
		Ok(())
	}

	#[test]
	fn test_a_table_needs_no_header_and_keeps_its_markup_27() -> Outcome<()> {
		// A cell's inlines are a paragraph's inlines, so what a cell holds sits where it sat.
		let tree = res!(imported(vec![
			Block::Table {
				head:	None,
				rows:	vec![Row(vec![
					Cell(vec![Inline::Emph { strong: true, content: vec![t("loud")] }]),
					Cell(vec![Inline::Link {
						to:		"news.cricket".to_string(),
						content:	vec![t("a link")],
					}]),
				])],
				cols:	vec![Align::None, Align::None],
			},
		]));
		let boxx = res!(kid(&tree, 0));
		assert_eq!(res!(kind_of(&boxx)), NodeKind::Boxx);
		// A table with no header row is a table of its body, and the box holds the one row it has.
		assert_eq!(res!(kids(&boxx)).len(), 1);
		let para = res!(kid(&boxx, 0));
		assert_eq!(res!(kind_of(&para)), NodeKind::Para);
		assert_eq!(res!(kind_of(&res!(kid(&para, 0)))), NodeKind::Emph);
		assert_eq!(res!(text_of(&res!(kid(&para, 1)))), CELL_SEP);
		assert_eq!(res!(kind_of(&res!(kid(&para, 2)))), NodeKind::Link);
		assert_eq!(res!(said(&para)), "loud | a link");
		Ok(())
	}

	#[test]
	fn test_a_table_that_says_nothing_is_dropped_28() -> Outcome<()> {
		// A row of empty cells is a blank paragraph by another name, and a box of no paragraphs
		// renders as nothing at all: the same rule an empty paragraph is held to.
		let tree = res!(imported(vec![
			Block::Table {
				head:	Some(Row(vec![Cell(Vec::new()), Cell(Vec::new())])),
				rows:	vec![Row(vec![c("   "), Cell(Vec::new())])],
				cols:	vec![Align::None, Align::None],
			},
			p("After."),
		]));
		let kids = res!(kids(&tree));
		assert_eq!(kids.len(), 1, "An empty table left something behind: {:?}", kids);
		assert_eq!(res!(kind_of(&kids[0])), NodeKind::Para);
		Ok(())
	}
}
