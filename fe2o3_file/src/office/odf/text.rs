//! `.odt`: an OpenDocument text document, written from and read back into the neutral document tree.
//!
//! # A heading says its own level
//!
//! `<text:h text:outline-level="2">` is a level two heading, full stop. There is no style to resolve
//! and no built-in name to recognise, which is the whole of what made the WordprocessingML reader
//! need `styles.xml` before it could tell a heading from a paragraph. This is the easier direction and
//! it is worth saying why: OpenDocument put the meaning in the element and Microsoft put it in a
//! style, and every consequence follows from that one choice.

use crate::office::edit::{
	Find,
	Piece,
	Tally,
	apply,
};
use crate::office::odf::{
	NS_FO,
	NS_OFFICE,
	NS_STYLE,
	NS_TABLE,
	NS_TEXT,
	NS_XLINK,
	pkg,
};
use crate::zip::{
	Method,
	Zip,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::doc::{
	Align,
	Block,
	Cell,
	Doc,
	Inline,
	Row,
};
use oxedyne_fe2o3_text::xml::{
	Elem,
	Node,
	Xml,
};
use oxedyne_fe2o3_text::xml::write::{
	Out,
	escape,
};

use std::collections::BTreeMap;

/// The media type an `.odt` declares in its first member.
pub const MEDIA: &str = "application/vnd.oasis.opendocument.text";

/// The most a single part is inflated to. An `.odt` is one `content.xml`, so this is the whole
/// document rather than a piece of it.
pub const MAX_PART: u64 = 64 * 1024 * 1024;

/// What a created document could not carry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Left {
	/// The images whose bytes could not be reached, by the source each was written with.
	pub images:	Vec<String>,
}

impl Left {

	/// Whether everything in the tree reached the document.
	pub fn is_empty(&self) -> bool {
		self.images.is_empty()
	}
}

/// Writes a document tree as the bytes of an `.odt`.
pub fn write(doc: &Doc) -> Outcome<(Vec<u8>, Left)> {
	let mut left = Left::default();
	let mut out = Out::declared();
	out.open("office:document-content", &[
		("xmlns:office", NS_OFFICE),
		("xmlns:text", NS_TEXT),
		("xmlns:table", NS_TABLE),
		("xmlns:style", NS_STYLE),
		("xmlns:fo", NS_FO),
		("xmlns:xlink", NS_XLINK),
		("office:version", pkg::VERSION),
	]);
	// The list style every list refers to. One definition serves both kinds: whether a level is
	// numbered is a property of the LEVEL here, so a bulleted and a numbered list cannot share one --
	// hence two.
	out.open("office:automatic-styles", &[]);
	for (name, ordered) in [("LB", false), ("LN", true)] {
		out.open("text:list-style", &[("style:name", name)]);
		for lvl in 1..=9 {
			let n = fmt!("{}", lvl);
			match ordered {
				true	=> out.open("text:list-level-style-number", &[
					("text:level", &n), ("style:num-suffix", "."), ("style:num-format", "1"),
				]),
				false	=> out.open("text:list-level-style-bullet", &[
					("text:level", &n), ("text:bullet-char", "\u{2022}"),
				]),
			}
			res!(out.close(match ordered {
				true	=> "text:list-level-style-number",
				false	=> "text:list-level-style-bullet",
			}));
		}
		res!(out.close("text:list-style"));
	}
	res!(out.close("office:automatic-styles"));
	out.open("office:body", &[]);
	out.open("office:text", &[]);
	res!(blocks(&mut out, &doc.blocks, &mut left));
	res!(out.close("office:text"));
	res!(out.close("office:body"));
	res!(out.close("office:document-content"));

	let mut zip = pkg::start(MEDIA);
	zip.set("content.xml", res!(out.finish()).into_bytes(), Method::Deflate);
	zip.set("styles.xml", res!(pkg::styles_for(MEDIA)).into_bytes(), Method::Deflate);
	zip.set("meta.xml", res!(pkg::meta(MEDIA)).into_bytes(), Method::Deflate);
	res!(pkg::finish(&mut zip, MEDIA));
	Ok((res!(zip.write()), left))
}

/// Writes a run of blocks.
///
/// The parameter is `run` and not `blocks`: a parameter of the same name as the function shadows it,
/// and the recursive calls below then resolve to a slice rather than to this.
fn blocks(out: &mut Out, run: &[Block], left: &mut Left) -> Outcome<()> {
	for block in run {
		match block {
			Block::Heading { level, content }	=> {
				let lvl = fmt!("{}", (*level).clamp(1, 10));
				out.open("text:h", &[("text:outline-level", &lvl)]);
				res!(inlines(out, content, left));
				res!(out.close("text:h"));
			}
			Block::Para(content)		=> {
				out.open("text:p", &[]);
				res!(inlines(out, content, left));
				res!(out.close("text:p"));
			}
			Block::Quote(inner)		=> {
				// A quotation is a run of paragraphs in the quotation style, because OpenDocument has
				// no quotation element. That is what every writer does with one.
				for b in inner {
					match b {
						Block::Para(content)	=> {
							out.open("text:p", &[("text:style-name", "Quotations")]);
							res!(inlines(out, content, left));
							res!(out.close("text:p"));
						}
						other			=> res!(blocks(out, &[other.clone()], left)),
					}
				}
			}
			Block::Code { text, .. }		=> {
				for line in text.lines() {
					out.open("text:p", &[("text:style-name", "Preformatted_20_Text")]);
					res!(inlines(out, &[Inline::Text(line.to_string())], left));
					res!(out.close("text:p"));
				}
			}
			Block::List { ordered, items }	=> res!(list(out, *ordered, items, left, true)),
			Block::Rule			=> {
				// OpenDocument has no thematic break element either; a paragraph in a style with a
				// bottom border is what a reader recognises as one.
				out.open("text:p", &[("text:style-name", "Horizontal_20_Line")]);
				res!(out.close("text:p"));
			}
			Block::Table { head, rows, cols }	=> res!(table(out, head, rows, cols, left)),
			Block::Div { content, .. }		=> res!(blocks(out, content, left)),
		}
	}
	Ok(())
}

/// Writes a list, nesting by putting a list inside an item.
fn list(
	out:	&mut Out,
	ordered:	bool,
	items:	&[Vec<Block>],
	left:	&mut Left,
	top:	bool,
)
	-> Outcome<()>
{
	let style = match ordered {
		true	=> "LN",
		false	=> "LB",
	};
	match top {
		true	=> out.open("text:list", &[("text:style-name", style)]),
		// A nested list carries no style name: it inherits the level from where it sits, which is
		// how OpenDocument nests. Naming the style again restarts the numbering.
		false	=> out.open("text:list", &[]),
	}
	for item in items {
		out.open("text:list-item", &[]);
		for b in item {
			match b {
				Block::List { ordered, items }	=> res!(list(out, *ordered, items, left, false)),
				other				=> res!(blocks(out, &[other.clone()], left)),
			}
		}
		res!(out.close("text:list-item"));
	}
	res!(out.close("text:list"));
	Ok(())
}

/// Writes a table.
fn table(
	out:	&mut Out,
	head:	&Option<Row>,
	rows:	&[Row],
	cols:	&[Align],
	left:	&mut Left,
)
	-> Outcome<()>
{
	let n = head.iter().chain(rows).map(|r| r.0.len()).max().unwrap_or(0).max(cols.len());
	if n == 0 {
		return Ok(());
	}
	out.open("table:table", &[("table:name", "Table1")]);
	out.empty("table:table-column", &[("table:number-columns-repeated", &fmt!("{}", n))]);
	if let Some(head) = head {
		out.open("table:table-header-rows", &[]);
		res!(row_of(out, head, n, left));
		res!(out.close("table:table-header-rows"));
	}
	for r in rows {
		res!(row_of(out, r, n, left));
	}
	res!(out.close("table:table"));
	Ok(())
}

/// One row of a table, padded to the width of the widest.
fn row_of(out: &mut Out, r: &Row, n: usize, left: &mut Left) -> Outcome<()> {
	let empty = Cell::default();
	out.open("table:table-row", &[]);
	for i in 0..n {
		out.open("table:table-cell", &[("office:value-type", "string")]);
		out.open("text:p", &[]);
		res!(inlines(out, &r.0.get(i).unwrap_or(&empty).0, left));
		res!(out.close("text:p"));
		res!(out.close("table:table-cell"));
	}
	res!(out.close("table:table-row"));
	Ok(())
}

/// Writes a run of inline content.
fn inlines(out: &mut Out, content: &[Inline], left: &mut Left) -> Outcome<()> {
	for item in content {
		match item {
			Inline::Text(t)			=> res!(text_run(out, t)),
			Inline::Code(t)			=> {
				out.open("text:span", &[("text:style-name", "Source_20_Text")]);
				res!(text_run(out, t));
				res!(out.close("text:span"));
			}
			Inline::Emph { strong, content }	=> {
				let style = match strong {
					true	=> "Strong_20_Emphasis",
					false	=> "Emphasis",
				};
				out.open("text:span", &[("text:style-name", style)]);
				res!(inlines(out, content, left));
				res!(out.close("text:span"));
			}
			Inline::Link { to, content }		=> {
				// No relationship part, no id: the destination is on the element. This is the whole
				// of the difference the module note is about.
				out.open("text:a", &[("xlink:href", to), ("xlink:type", "simple")]);
				res!(inlines(out, content, left));
				res!(out.close("text:a"));
			}
			Inline::Image { src, alt }		=> {
				left.images.push(src.clone());
				out.open("text:span", &[("text:style-name", "Emphasis")]);
				res!(text_run(out, alt));
				res!(out.close("text:span"));
			}
			Inline::Span { content, .. }		=> res!(inlines(out, content, left)),
			Inline::Break				=> out.empty("text:line-break", &[]),
		}
	}
	Ok(())
}

/// Text, with runs of spaces and tabs written as the elements OpenDocument has for them.
///
/// A reader collapses whitespace in `text:p` exactly as HTML does, so two consecutive spaces arrive
/// as one unless the second is a `<text:s/>`. A listing indented with spaces loses its indentation
/// entirely without this, which is the whole point of writing a listing out.
fn text_run(out: &mut Out, text: &str) -> Outcome<()> {
	let mut run = String::new();
	let mut chars = text.chars().peekable();
	while let Some(c) = chars.next() {
		match c {
			'\t'	=> {
				if !run.is_empty() {
					out.text(&run);
					run.clear();
				}
				out.empty("text:tab", &[]);
			}
			' '	=> {
				let mut n = 1;
				while chars.peek() == Some(&' ') {
					chars.next();
					n += 1;
				}
				// A LEADING space is dropped by every reader, so ALL of a leading run goes out as
				// `<text:s/>`. Keeping the first as a literal is what lost one space off the front of
				// every indented line of every listing -- four became three, and only an external
				// reader showed it.
				let leading = run.is_empty();
				if !leading {
					run.push(' ');
					n -= 1;
					out.text(&run);
					run.clear();
				}
				match n {
					0	=> {}
					1	=> out.empty("text:s", &[]),
					_	=> out.empty("text:s", &[("text:c", &fmt!("{}", n))]),
				}
			}
			_	=> run.push(c),
		}
	}
	if !run.is_empty() {
		out.text(&run);
	}
	Ok(())
}

/// A document read for reading.
#[derive(Clone, Debug, Default)]
pub struct Reading {
	/// The prose.
	pub doc:	Doc,
	/// How many pictures the document holds, which this does not draw.
	pub images:	usize,
	/// Whether the file carries a macro project. Said, never run.
	pub macros:	bool,
}

/// What one style says about what wears it.
///
/// **A foreign document names almost nothing the way this crate's writer does.** LibreOffice writes
/// `T1`, `P2`, `L3` -- automatic styles, generated per document -- and puts the meaning in their
/// PROPERTIES. A reader matching on the name finds `Strong_20_Emphasis` in its own output and
/// nothing at all in anybody else's, so every bold word in a real document arrives as plain text.
/// The foreign fixture is what found that, and it is the same mistake the WordprocessingML reader
/// would have made had it trusted style ids.
#[derive(Clone, Debug, Default)]
struct Style {
	/// Bold, where the style says so itself.
	bold:	bool,
	/// Italic.
	italic:	bool,
	/// A monospaced face.
	mono:	bool,
	/// The style this one is based on, followed for the properties it does not set itself.
	parent:	Option<String>,
}

/// Every style a document defines, by name, and which list styles are numbered.
#[derive(Clone, Debug, Default)]
struct Styles {
	/// Style name to what it says.
	by_name:	BTreeMap<String, Style>,
	/// List style name to whether its first level is numbered.
	lists:	BTreeMap<String, bool>,
}

impl Styles {

	/// Whether a style resolves to bold, italic or monospaced, following what it is based on.
	fn of(&self, name: &str) -> Style {
		let mut out = Style::default();
		let mut at = name.to_string();
		for _ in 0..8 {
			let s = match self.by_name.get(&at) {
				Some(s)	=> s.clone(),
				None		=> break,
			};
			out.bold |= s.bold;
			out.italic |= s.italic;
			out.mono |= s.mono;
			match s.parent {
				Some(p)	=> at = p,
				None		=> break,
			}
		}
		// A style this document does not define may still be one of the well-known names, which is
		// what this crate's own writer emits.
		let low = name.to_ascii_lowercase();
		out.bold |= low.contains("strong") || low.contains("bold");
		out.italic |= low.contains("emphasis") && !low.contains("strong");
		out.mono |= low.contains("source") || low.contains("teletype");
		out
	}

	/// Whether the paragraph style resolves to a quotation or a listing, by name anywhere up the
	/// chain it is based on.
	fn para_kind(&self, name: &str) -> (bool, bool) {
		let mut at = name.to_string();
		for _ in 0..8 {
			let low = at.to_ascii_lowercase();
			if low.starts_with("quotation") || low.contains("block_20_quotation") {
				return (true, false);
			}
			if low.starts_with("preformatted") || low.starts_with("source_20_text")
				|| low.contains("plain_20_text")
			{
				return (false, true);
			}
			match self.by_name.get(&at).and_then(|s| s.parent.clone()) {
				Some(p)	=> at = p,
				None		=> break,
			}
		}
		(false, false)
	}
}

/// The styles a part defines, added to what is already known.
fn gather_styles(xml: &Xml, into: &mut Styles) {
	for s in xml.all("style:style") {
		let name = match s.attr("style:name") {
			Some(n)	=> n.to_string(),
			None		=> continue,
		};
		let props = s.child("style:text-properties");
		into.by_name.insert(name, Style {
			bold:	props.and_then(|p| p.attr("fo:font-weight"))
				.map(|v| v == "bold" || v == "600" || v == "700" || v == "800" || v == "900")
				.unwrap_or(false),
			italic:	props.and_then(|p| p.attr("fo:font-style"))
				.map(|v| v == "italic" || v == "oblique")
				.unwrap_or(false),
			mono:	props.and_then(|p| p.attr("style:font-name").or(p.attr("fo:font-family")))
				.map(|f| {
					let f = f.to_ascii_lowercase();
					f.contains("mono") || f.contains("courier") || f.contains("consol")
				})
				.unwrap_or(false),
			parent:	s.attr("style:parent-style-name").map(|v| v.to_string()),
		});
	}
	for l in xml.all("text:list-style") {
		let name = match l.attr("style:name") {
			Some(n)	=> n.to_string(),
			None		=> continue,
		};
		// Whether the list is numbered is a property of its FIRST LEVEL, and asking whether ANY level
		// is numbered is wrong: LibreOffice writes ten levels for every list style and makes level
		// TEN a number even in a pure bullet list. That one line turned every bulleted list in a
		// foreign document into a numbered one, and only the foreign fixture could have shown it.
		let ordered = l.elems()
			.find(|e| e.attr("text:level") == Some("1"))
			.map(|e| e.name.qname == "text:list-level-style-number")
			.unwrap_or(false);
		into.lists.insert(name, ordered);
	}
}

/// Reads an `.odt` into the document tree.
pub fn read(bytes: &[u8]) -> Outcome<Reading> {
	let zip = res!(Zip::read(bytes.to_vec()));
	let mut out = Reading::default();
	// An OpenDocument macro lives in `Basic/`, not in a `vbaProject.bin`.
	out.macros = zip.names().iter().any(|n| n.starts_with("Basic/"));
	let src = res!(String::from_utf8(res!(zip.content_capped("content.xml", MAX_PART))),
		Decode, String);
	let xml = res!(Xml::parse(&src));
	let body = res!(res!(xml.root()).find(&["office:body", "office:text"]).ok_or_else(|| err!(
		"This package has no <office:text>, so it is not a text document."; Invalid, Input, Missing)));
	// Both parts, because a document splits its styles between them: the named ones a person applies
	// live in `styles.xml` and the generated ones a writer makes live beside the content.
	let mut styles = Styles::default();
	if zip.has("styles.xml") {
		if let Ok(b) = zip.content_capped("styles.xml", MAX_PART) {
			if let Ok(t) = String::from_utf8(b) {
				if let Ok(x) = Xml::parse(&t) {
					gather_styles(&x, &mut styles);
				}
			}
		}
	}
	gather_styles(&xml, &mut styles);
	let mut blocks = Vec::new();
	read_blocks(&xml, body, &mut blocks, &mut out.images, &styles);
	// A listing arrives as one paragraph per line, so consecutive ones are one block. Left apart,
	// every line of a program becomes its own fenced block.
	out.doc = Doc { blocks: merge_code(blocks) };
	Ok(out)
}

/// Reads a run of blocks out of an element.
fn read_blocks(xml: &Xml, at: &Elem, out: &mut Vec<Block>, images: &mut usize, st: &Styles) {
	for kid in at.elems() {
		match kid.name.qname.as_str() {
			"text:h"	=> {
				// The level is ON the element. Nothing has to be resolved.
				let level = kid.attr("text:outline-level")
					.and_then(|v| v.parse::<u8>().ok())
					.unwrap_or(1)
					.clamp(1, 6);
				let content = read_inlines(xml, kid, images, st);
				if !content.is_empty() {
					out.push(Block::Heading { level, content });
				}
			}
			"text:p"	=> {
				let content = read_inlines(xml, kid, images, st);
				if content.is_empty() {
					continue;
				}
				let style = kid.attr("text:style-name").unwrap_or("");
				let (quote, code) = st.para_kind(style);
				if quote {
					out.push(Block::Quote(vec![Block::Para(content)]));
				} else if code {
					out.push(Block::Code {
						lang:	None,
						text:	oxedyne_fe2o3_text::doc::text_of(&content),
					});
				} else {
					out.push(Block::Para(content));
				}
			}
			"text:list"	=> {
				if let Some(list) = read_list(xml, kid, images, st) {
					out.push(list);
				}
			}
			"table:table"	=> {
				if let Some(t) = read_table(xml, kid, images, st) {
					out.push(t);
				}
			}
			// A section, a frame, a change region: contributes nothing itself, and what it holds is
			// read where it stood. The same rule as everywhere else in this crate.
			_	=> read_blocks(xml, kid, out, images, st),
		}
	}
}

/// Reads a list.
///
/// Whether it is numbered is a property of its STYLE, which is one level of indirection rather than
/// the two a `.xlsx` needs. A style this cannot find is read as a bullet, which says less than a
/// wrong number would.
fn read_list(xml: &Xml, at: &Elem, images: &mut usize, st: &Styles) -> Option<Block> {
	// From the STYLE DEFINITION, not from the name: a foreign document calls its list style `L2`
	// and puts `text:list-level-style-number` inside it, so a reader matching on the name reads
	// every numbered list in the world as a bulleted one.
	let style = at.attr("text:style-name").unwrap_or("");
	let ordered = match st.lists.get(style) {
		Some(o)	=> *o,
		None		=> style.contains("LN") || style.to_ascii_lowercase().contains("number"),
	};
	let mut items = Vec::new();
	for item in at.children("text:list-item") {
		let mut blocks = Vec::new();
		read_blocks(xml, item, &mut blocks, images, st);
		if !blocks.is_empty() {
			items.push(blocks);
		}
	}
	match items.is_empty() {
		true	=> None,
		false	=> Some(Block::List { ordered, items }),
	}
}

/// Reads a table.
fn read_table(xml: &Xml, at: &Elem, images: &mut usize, st: &Styles) -> Option<Block> {
	let mut head = None;
	let mut rows = Vec::new();
	if let Some(hr) = at.child("table:table-header-rows") {
		if let Some(first) = hr.children("table:table-row").first() {
			head = Some(read_row(xml, first, images, st));
		}
	}
	for tr in at.children("table:table-row") {
		rows.push(read_row(xml, tr, images, st));
	}
	if head.is_none() && rows.is_empty() {
		return None;
	}
	let n = head.iter().chain(rows.iter()).map(|r| r.0.len()).max().unwrap_or(0);
	Some(Block::Table { head, rows, cols: vec![Align::None; n] })
}

/// One row of a table.
fn read_row(xml: &Xml, tr: &Elem, images: &mut usize, st: &Styles) -> Row {
	let mut cells = Vec::new();
	for tc in tr.elems() {
		match tc.name.qname.as_str() {
			"table:table-cell"	=> {
				let mut content = Vec::new();
				for (i, p) in tc.children("text:p").into_iter().enumerate() {
					if i > 0 {
						content.push(Inline::Break);
					}
					content.extend(read_inlines(xml, p, images, st));
				}
				// A cell may say it repeats, which is how a run of empty cells is written.
				let n = tc.attr("table:number-columns-repeated")
					.and_then(|v| v.parse::<usize>().ok())
					.unwrap_or(1)
					.min(1024);
				for _ in 0..n {
					cells.push(Cell(content.clone()));
				}
			}
			"table:covered-table-cell"	=> cells.push(Cell::default()),
			_				=> {}
		}
	}
	Row(cells)
}

/// The inline content of a paragraph or a heading.
fn read_inlines(xml: &Xml, at: &Elem, images: &mut usize, st: &Styles) -> Vec<Inline> {
	let mut out = Vec::new();
	for node in &at.kids {
		match node {
			oxedyne_fe2o3_text::xml::Node::Text(span)	=> {
				let t = xml.text(span);
				if !t.is_empty() {
					out.push(Inline::Text(t));
				}
			}
			oxedyne_fe2o3_text::xml::Node::Elem(e)	=> match e.name.qname.as_str() {
				"text:s"	=> {
					let n = e.attr("text:c").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1);
					out.push(Inline::Text(" ".repeat(n.min(256))));
				}
				"text:tab"		=> out.push(Inline::Text("\t".to_string())),
				"text:line-break"	=> out.push(Inline::Break),
				"draw:frame" | "draw:image"	=> {
					*images += 1;
					// A frame may hold a caption, which is prose and is kept.
					out.extend(read_inlines(xml, e, images, st));
				}
				"text:a"	=> {
					let to = e.attr("xlink:href").unwrap_or("").to_string();
					let content = read_inlines(xml, e, images, st);
					match content.is_empty() {
						true	=> {}
						false	=> out.push(Inline::Link { to, content }),
					}
				}
				"text:span"	=> {
					let style = st.of(e.attr("text:style-name").unwrap_or(""));
					let content = read_inlines(xml, e, images, st);
					if content.is_empty() {
						continue;
					}
					let mut content = match style.mono {
						true	=> vec![Inline::Code(oxedyne_fe2o3_text::doc::text_of(&content))],
						false	=> content,
					};
					if style.italic {
						content = vec![Inline::Emph { strong: false, content }];
					}
					if style.bold {
						content = vec![Inline::Emph { strong: true, content }];
					}
					out.extend(content);
				}
				// A bookmark, a note anchor, a soft page break: no words of their own.
				"text:bookmark" | "text:bookmark-start" | "text:bookmark-end"
				| "text:soft-page-break" | "text:tracked-changes"	=> {}
				_	=> out.extend(read_inlines(xml, e, images, st)),
			},
			_	=> {}
		}
	}
	coalesce(out)
}

/// Joins consecutive code blocks into one.
///
/// A listing is one paragraph per line in this format, so a program arrives as a run of one-line
/// blocks. Left apart, every line of it renders as its own fenced block.
fn merge_code(blocks: Vec<Block>) -> Vec<Block> {
	let mut out: Vec<Block> = Vec::with_capacity(blocks.len());
	for b in blocks {
		match (out.last_mut(), b) {
			(Some(Block::Code { text: a, .. }), Block::Code { text: b, .. })	=> {
				a.push('\n');
				a.push_str(&b);
			}
			(_, b)								=> out.push(b),
		}
	}
	out
}

/// Joins adjacent inlines that are marked alike.
fn coalesce(items: Vec<Inline>) -> Vec<Inline> {
	let mut out: Vec<Inline> = Vec::with_capacity(items.len());
	for item in items {
		match (out.last_mut(), item) {
			(Some(Inline::Text(a)), Inline::Text(b))	=> a.push_str(&b),
			(Some(Inline::Code(a)), Inline::Code(b))	=> a.push_str(&b),
			(
				Some(Inline::Emph { strong: sa, content: ca }),
				Inline::Emph { strong: sb, content: cb },
			) if *sa == sb				=> {
				let mut joined = std::mem::take(ca);
				joined.extend(cb);
				*ca = coalesce(joined);
			}
			(_, item)					=> out.push(item),
		}
	}
	out.retain(|i| !matches!(i, Inline::Text(t) if t.is_empty()));
	out
}

// ---------------------------------------------------------------------------
// Editing an `.odt` in place
// ---------------------------------------------------------------------------

/// What an edit of an `.odt` produced.
#[derive(Clone, Debug, Default)]
pub struct Edited {
	pub bytes:	Vec<u8>,
	pub tallies:	Vec<Tally>,	// one per edit asked for, in order
	pub runs:	usize,	// runs of text rewritten
}

/// Replaces text in an `.odt`, leaving every other byte of the package as it arrived.
///
/// The counterpart of [`crate::office::docx::edit::edit`] and the same design: the text of a paragraph
/// is the concatenation of its runs, a match is found there and pushed back down onto the runs it
/// covered, and [`crate::zip`] copies every member nobody touched. `styles.xml`, `meta.xml`, the
/// manifest, the macros and the pictures are never opened.
///
/// Only `content.xml` is searched, so a phrase in a header or a footer -- which live in `styles.xml` --
/// reports as absent rather than being changed in one of two places.
pub fn edit(bytes: &[u8], edits: &[Find]) -> Outcome<Edited> {
	if edits.is_empty() {
		return Err(err!("An edit of a document was asked for with no edits in it."; Invalid, Input));
	}
	let mut zip = res!(Zip::read(bytes.to_vec()));
	let src = res!(String::from_utf8(res!(zip.content_capped("content.xml", MAX_PART))),
		Decode, String);
	let mut xml = res!(Xml::parse(&src));
	let body = res!(res!(xml.root()).find(&["office:body", "office:text"]).ok_or_else(|| err!(
		"This package has no <office:text>, so it is not a text document."; Invalid, Input, Missing)));
	let mut groups = Vec::new();
	edit_walk(&xml, body, &mut groups);
	let (changes, tallies) = res!(apply(&groups, edits));
	let runs = changes.len();
	for c in &changes {
		res!(xml.splice(c.piece.span.clone(), content_markup(&c.text)));
	}
	zip.set("content.xml", xml.render().into_bytes(), Method::Deflate);
	Ok(Edited { bytes: res!(zip.write()), tallies, runs })
}

/// Every paragraph and heading at or below an element, as a group of text pieces each.
///
/// A nested paragraph -- a frame's caption inside a paragraph -- gets its own group and its text is not
/// also in the enclosing one, for the reason `docx::edit` gives: two splices over one span is a
/// refusal, and rightly.
fn edit_walk(xml: &Xml, at: &Elem, out: &mut Vec<Vec<Piece>>) {
	match at.name.qname.as_str() {
		"text:p" | "text:h"	=> {
			let slot = out.len();
			out.push(Vec::new());
			let mut group = Vec::new();
			edit_gather(xml, at, &mut group, out);
			out[slot] = group;
		}
		_	=> {
			for kid in at.elems() {
				edit_walk(xml, kid, out);
			}
		}
	}
}

/// One paragraph's own text, run by run.
///
/// Character data, and the three elements that ARE text: `<text:s>` is a run of spaces, because
/// OpenDocument collapses literal ones; `<text:tab>` is a tab; `<text:line-break>` is a newline. A
/// reader that skipped them would match `Q1 2026` against a document holding `Q1<text:s/>2026` and
/// report the phrase absent -- and the phrase is there, it is what a person typed.
fn edit_gather(xml: &Xml, at: &Elem, group: &mut Vec<Piece>, out: &mut Vec<Vec<Piece>>) {
	for kid in &at.kids {
		match kid {
			Node::Text(span)	=> group.push(Piece::new(span.clone(), xml.text(span))),
			Node::Elem(e)	=> match e.name.qname.as_str() {
				"text:p" | "text:h"	=> edit_walk(xml, e, out),
				"text:s"	=> {
					let n = e.attr("text:c").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1);
					group.push(Piece::new(e.span.clone(), " ".repeat(n.min(4096))));
				}
				"text:tab"	=> group.push(Piece::new(e.span.clone(), "\t")),
				"text:line-break"	=> group.push(Piece::new(e.span.clone(), "\n")),
				// A footnote's body, a comment's body and an index mark hold text that is not the
				// paragraph's own, and replacing in them would edit two places for one phrase.
				"text:note" | "office:annotation"	=> {}
				_	=> edit_gather(xml, e, group, out),
			},
			_	=> {}
		}
	}
}

/// Text as OpenDocument paragraph content: the markup that means exactly these characters.
///
/// Three characters cannot be written literally and survive. A run of two or more spaces is collapsed
/// to one by every reader, so it becomes `<text:s text:c="n"/>`; a space at either end of the run is
/// collapsed for the same reason and becomes `<text:s/>`; a tab and a newline have elements of their
/// own. Writing them literally produces a file that opens and says something slightly different from
/// what the edit asked for, which is the worst of the available outcomes.
pub fn content_markup(text: &str) -> String {
	let mut out = String::with_capacity(text.len() + 16);
	let chars: Vec<char> = text.chars().collect();
	let mut i = 0;
	while i < chars.len() {
		match chars[i] {
			' '	=> {
				let mut n = 0;
				while i + n < chars.len() && chars[i + n] == ' ' {
					n += 1;
				}
				// A single space with a character either side of it is safe as itself, and leaving it
				// alone keeps the markup readable. Anywhere else it is spelled out.
				let interior = i > 0 && i + n < chars.len();
				match n == 1 && interior {
					true	=> out.push(' '),
					false	=> match n {
						1	=> out.push_str("<text:s/>"),
						_	=> out.push_str(&fmt!("<text:s text:c=\"{}\"/>", n)),
					},
				}
				i += n;
			}
			'\t'	=> {
				out.push_str("<text:tab/>");
				i += 1;
			}
			'\n' | '\r'	=> {
				out.push_str("<text:line-break/>");
				i += 1;
			}
			c	=> {
				out.push_str(&escape(&c.to_string()));
				i += 1;
			}
		}
	}
	out
}

/// The text of the body, paragraph by paragraph, as an edit sees it.
///
/// The strings a `find` is matched against, which is not the same as the document as prose: this is
/// where a run split by a style, or a space written as `<text:s/>`, shows up.
pub fn body_text(bytes: &[u8]) -> Outcome<Vec<String>> {
	let zip = res!(Zip::read(bytes.to_vec()));
	let src = res!(String::from_utf8(res!(zip.content_capped("content.xml", MAX_PART))),
		Decode, String);
	let xml = res!(Xml::parse(&src));
	let body = res!(res!(xml.root()).find(&["office:body", "office:text"]).ok_or_else(|| err!(
		"This package has no <office:text>, so it is not a text document."; Invalid, Input, Missing)));
	let mut groups = Vec::new();
	edit_walk(&xml, body, &mut groups);
	Ok(groups.iter()
		.map(|g| g.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().concat())
		.collect())
}
