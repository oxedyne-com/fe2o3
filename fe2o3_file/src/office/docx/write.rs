//! Creating a `.docx` from the neutral document tree.
//!
//! The direction that is genuinely easy: every byte is written here, so there is nothing to preserve
//! and nothing to guess. What arrives is [`Doc`], which knows what a passage *is* -- a heading, a
//! quotation, a list item -- and what leaves is a document in which those are Word's own heading,
//! quotation and list, named rather than drawn.
//!
//! # What is not carried, and is said rather than dropped quietly
//!
//! [`write`] hands back the parts it could not carry along with the bytes, so a caller can
//! tell the user. Today that is one thing: an image, whose bytes this cannot reach -- the tree holds
//! the *source* an image was written with, a path or a URL, and this crate has no filesystem and no
//! network. The alt text is written in its place and the image is counted. A caller that can resolve
//! the source is where images will be added.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_text::doc::{
	Align,
	Block,
	Cell,
	Doc,
	Inline,
	Row,
};
use crate::office::docx::{
	MARGIN,
	NS_W,
	NUM_BULLET,
	NUM_ORDERED,
	PAGE_H,
	PAGE_W,
	TEXT_W,
	parts,
};
use crate::office::opc::{
	CT_DOCUMENT,
	CT_NUMBERING,
	CT_STYLES,
	NS_R,
	REL_DOC,
	REL_HYPERLINK,
	REL_NUMBERING,
	REL_STYLES,
	Rels,
	Types,
};
use oxedyne_fe2o3_text::xml::write::Out;

use oxedyne_fe2o3_core::prelude::*;
use crate::zip::{
	Method,
	Zip,
};

// The deepest a list may nest before its items stop being indented further. `parts::numbering`
// defines nine levels, so a tenth would name a level the document does not define and lose its
// bullet. Deeper items sit at the ninth rather than vanish.
const MAX_LEVEL: usize = 8;

/// What a created document could not carry.
///
/// Counted and named rather than dropped in silence. A reader who is told "one image is not drawn"
/// knows what they are looking at; a reader who is told nothing thinks the document is complete.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Left {
	pub images:	Vec<String>,	// by the source each was written with
}

impl Left {

	/// Whether everything in the tree reached the document.
	pub fn is_empty(&self) -> bool {
		self.images.is_empty()
	}
}

/// Writes a document tree as the bytes of a `.docx`, and says what did not fit.
pub fn write(doc: &Doc) -> Outcome<(Vec<u8>, Left)> {
	let mut b = Build {
		out:	Out::declared(),
		rels:	Rels::new(),
		left:	Left::default(),
	};
	// The two parts the document always refers to. They are added before the body is written so a
	// hyperlink's id, handed out during the body, never collides with them.
	let _ = b.rels.add(REL_STYLES, "styles.xml");
	let _ = b.rels.add(REL_NUMBERING, "numbering.xml");

	b.out.open("w:document", &[("xmlns:w", NS_W), ("xmlns:r", NS_R)]);
	b.out.open("w:body", &[]);
	res!(b.blocks(&doc.blocks, Ctx::default()));
	res!(b.section());
	res!(b.out.close("w:body"));
	res!(b.out.close("w:document"));
	let document = res!(b.out.finish());

	let mut types = Types::new();
	types.over("/word/document.xml", CT_DOCUMENT);
	types.over("/word/styles.xml", CT_STYLES);
	types.over("/word/numbering.xml", CT_NUMBERING);

	let mut root = Rels::new();
	let _ = root.add(REL_DOC, "word/document.xml");

	let mut zip = Zip::new();
	// The order is the order Word writes them in. Nothing depends on it, and matching it means a
	// document made here and one made there differ in their content rather than in their shape.
	zip.set("[Content_Types].xml", res!(types.write()).into_bytes(), Method::Deflate);
	zip.set("_rels/.rels", res!(root.write()).into_bytes(), Method::Deflate);
	zip.set("word/document.xml", document.into_bytes(), Method::Deflate);
	zip.set("word/_rels/document.xml.rels", res!(b.rels.write()).into_bytes(), Method::Deflate);
	zip.set("word/styles.xml", res!(parts::styles()).into_bytes(), Method::Deflate);
	zip.set("word/numbering.xml", res!(parts::numbering()).into_bytes(), Method::Deflate);
	Ok((res!(zip.write()), b.left))
}

/// What a block is being written inside, which decides what its paragraphs look like.
///
/// Carried down rather than looked up, because the same paragraph is a quotation inside a quotation
/// and a list item inside a list, and nothing about the paragraph itself says which.
#[derive(Clone, Copy, Debug, Default)]
struct Ctx {
	style:	Option<&'static str>,	// where the block does not name one of its own
	num:	Option<(&'static str, usize)>,	// the `w:numId`, and how deep it sits
}

/// How a run of text is marked.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Fmt {
	bold:	bool,
	italic:	bool,
	code:	bool,	// a span of code within a line
	link:	bool,	// part of a link, which is what the `Hyperlink` character style is for
}

/// The state of one document being written.
struct Build {
	out:	Out,	// the document part, under construction
	rels:	Rels,	// what the document part refers to
	left:	Left,	// what could not be carried
}

impl Build {

	fn blocks(&mut self, blocks: &[Block], ctx: Ctx) -> Outcome<()> {
		for block in blocks {
			res!(self.block(block, ctx));
		}
		Ok(())
	}

	fn block(&mut self, block: &Block, ctx: Ctx) -> Outcome<()> {
		match block {
			Block::Heading { level, content }	=> {
				// A heading is Word's OWN heading, by name, so the navigation pane and a generated
				// contents page both find it. Six levels, and anything past six is a level six.
				let style = match level.clamp(&1, &6) {
					1	=> "Heading1",
					2	=> "Heading2",
					3	=> "Heading3",
					4	=> "Heading4",
					5	=> "Heading5",
					_	=> "Heading6",
				};
				res!(self.para(content, Ctx { style: Some(style), num: None }))
			}
			Block::Para(content)			=> res!(self.para(content, ctx)),
			Block::Quote(inner)			=> {
				res!(self.blocks(inner, Ctx { style: Some("Quote"), ..ctx }))
			}
			Block::List { ordered, items }	=> {
				let num = match ordered {
					true	=> NUM_ORDERED,
					false	=> NUM_BULLET,
				};
				let lvl = match ctx.num {
					Some((_, l))	=> (l + 1).min(MAX_LEVEL),
					None		=> 0,
				};
				let inner = Ctx { style: Some("ListParagraph"), num: Some((num, lvl)) };
				for item in items {
					res!(self.blocks(item, inner));
				}
			}
			Block::Code { text, .. }		=> {
				// One paragraph to a line: a listing's line structure is what it says, and a single
				// paragraph holding newlines would be reflowed by Word into one long line.
				for line in text.lines() {
					let content = [Inline::Text(line.to_string())];
					res!(self.para(&content, Ctx { style: Some("SourceCode"), num: None }));
				}
			}
			Block::Rule				=> res!(self.rule()),
			Block::Table { head, rows, cols }	=> res!(self.table(head, rows, cols)),
			// A division names a region and says nothing about how it looks, which is the tree's whole
			// design. There is nothing to draw, so its content is written where it stands.
			Block::Div { content, .. }		=> res!(self.blocks(content, ctx)),
		}
		Ok(())
	}

	fn para(&mut self, content: &[Inline], ctx: Ctx) -> Outcome<()> {
		self.out.open("w:p", &[]);
		if ctx.style.is_some() || ctx.num.is_some() {
			self.out.open("w:pPr", &[]);
			if let Some(style) = ctx.style {
				self.out.empty("w:pStyle", &[("w:val", style)]);
			}
			if let Some((num, lvl)) = ctx.num {
				let ilvl = fmt!("{}", lvl);
				self.out.open("w:numPr", &[]);
				self.out.empty("w:ilvl", &[("w:val", &ilvl)]);
				self.out.empty("w:numId", &[("w:val", num)]);
				res!(self.out.close("w:numPr"));
			}
			res!(self.out.close("w:pPr"));
		}
		res!(self.inlines(content, Fmt::default()));
		res!(self.out.close("w:p"));
		Ok(())
	}

	fn inlines(&mut self, content: &[Inline], fmt: Fmt) -> Outcome<()> {
		for item in content {
			match item {
				Inline::Text(text)			=> res!(self.run(text, fmt)),
				Inline::Code(text)			=> {
					res!(self.run(text, Fmt { code: true, ..fmt }))
				}
				Inline::Emph { strong, content }	=> {
					let fmt = match strong {
						true	=> Fmt { bold: true, ..fmt },
						false	=> Fmt { italic: true, ..fmt },
					};
					res!(self.inlines(content, fmt))
				}
				Inline::Link { to, content }		=> {
					// A link out of the document is a relationship, not an attribute: the URL lives
					// in the rels part and the body names it by id.
					let id = self.rels.add_external(REL_HYPERLINK, to);
					self.out.open("w:hyperlink", &[("r:id", &id)]);
					res!(self.inlines(content, Fmt { link: true, ..fmt }));
					res!(self.out.close("w:hyperlink"));
				}
				Inline::Image { src, alt }		=> {
					// The tree holds where an image is, not what it holds, and nothing here can
					// fetch it. The alt text stands in its place and the omission is counted, so the
					// caller can say so rather than the reader having to notice.
					self.left.images.push(src.clone());
					res!(self.run(alt, Fmt { italic: true, ..fmt }));
				}
				// A span names a region and says nothing about how it looks. Its content stands.
				Inline::Span { content, .. }		=> res!(self.inlines(content, fmt)),
				Inline::Break				=> {
					self.out.open("w:r", &[]);
					self.out.empty("w:br", &[]);
					res!(self.out.close("w:r"));
				}
			}
		}
		Ok(())
	}

	/// Writes one run of text, split at any newline it holds.
	///
	/// A newline inside a `w:t` is whitespace to Word, and would join two lines into one. It is a
	/// break, so it is written as one.
	fn run(&mut self, text: &str, fmt: Fmt) -> Outcome<()> {
		for (i, line) in text.split('\n').enumerate() {
			if i > 0 {
				self.out.open("w:r", &[]);
				self.out.empty("w:br", &[]);
				res!(self.out.close("w:r"));
			}
			if line.is_empty() {
				continue;
			}
			self.out.open("w:r", &[]);
			if fmt != Fmt::default() {
				self.out.open("w:rPr", &[]);
				// Only one character style may apply, and a link that happens to be in code is a link
				// first: what the reader does with it is follow it.
				match (fmt.link, fmt.code) {
					(true, _)	=> self.out.empty("w:rStyle", &[("w:val", "Hyperlink")]),
					(false, true)	=> self.out.empty("w:rStyle", &[("w:val", "InlineCode")]),
					(false, false)	=> {}
				}
				if fmt.bold {
					self.out.empty("w:b", &[]);
				}
				if fmt.italic {
					self.out.empty("w:i", &[]);
				}
				res!(self.out.close("w:rPr"));
			}
			// `xml:space="preserve"` or the leading and trailing spaces of a run go, and a sentence
			// built from three runs loses the spaces between them.
			self.out.leaf("w:t", &[("xml:space", "preserve")], line);
			res!(self.out.close("w:r"));
		}
		Ok(())
	}

	/// Writes a thematic break, which Word has no element for.
	///
	/// It is an empty paragraph with a rule under it. That is what Word itself writes when a person
	/// types three hyphens, so it is what a person opening the document will recognise.
	fn rule(&mut self) -> Outcome<()> {
		self.out.open("w:p", &[]);
		self.out.open("w:pPr", &[]);
		self.out.open("w:pBdr", &[]);
		self.out.empty("w:bottom", &[
			("w:val", "single"),
			("w:sz", "6"),
			("w:space", "1"),
			("w:color", "auto"),
		]);
		res!(self.out.close("w:pBdr"));
		res!(self.out.close("w:pPr"));
		res!(self.out.close("w:p"));
		Ok(())
	}

	fn table(&mut self, head: &Option<Row>, rows: &[Row], cols: &[Align]) -> Outcome<()> {
		// The widest row decides the grid, because a row with fewer cells than the header is a table
		// somebody wrote by hand and Word still has to lay it out.
		let n = head.iter().chain(rows).map(|r| r.0.len()).max().unwrap_or(0).max(cols.len());
		if n == 0 {
			return Ok(());
		}
		let width = fmt!("{}", TEXT_W as usize / n);
		self.out.open("w:tbl", &[]);
		self.out.open("w:tblPr", &[]);
		self.out.empty("w:tblW", &[("w:w", "0"), ("w:type", "auto")]);
		// The borders are written on the table rather than taken from a style, so the table has lines
		// in a document that carries no theme and no table styles.
		self.out.open("w:tblBorders", &[]);
		for side in ["top", "left", "bottom", "right", "insideH", "insideV"] {
			self.out.empty(&fmt!("w:{}", side), &[
				("w:val", "single"),
				("w:sz", "4"),
				("w:space", "0"),
				("w:color", "auto"),
			]);
		}
		res!(self.out.close("w:tblBorders"));
		res!(self.out.close("w:tblPr"));
		self.out.open("w:tblGrid", &[]);
		for _ in 0..n {
			self.out.empty("w:gridCol", &[("w:w", &width)]);
		}
		res!(self.out.close("w:tblGrid"));
		if let Some(head) = head {
			res!(self.row(head, cols, n, true));
		}
		for row in rows {
			res!(self.row(row, cols, n, false));
		}
		res!(self.out.close("w:tbl"));
		// A table may not be the last thing in a body, and two tables in a row would run together. An
		// empty paragraph after one is what Word writes and what keeps both true.
		self.out.open("w:p", &[]);
		res!(self.out.close("w:p"));
		Ok(())
	}

	/// Writes one row of a table, padded to the width of the grid.
	fn row(&mut self, row: &Row, cols: &[Align], n: usize, head: bool) -> Outcome<()> {
		self.out.open("w:tr", &[]);
		if head {
			// A header row repeats at the top of each page it runs onto, which is what makes a long
			// table readable and what a reader will notice the absence of.
			self.out.open("w:trPr", &[]);
			self.out.empty("w:tblHeader", &[]);
			res!(self.out.close("w:trPr"));
		}
		let empty = Cell::default();
		for i in 0..n {
			let cell = row.0.get(i).unwrap_or(&empty);
			let align = cols.get(i).copied().unwrap_or(Align::None);
			self.out.open("w:tc", &[]);
			self.out.open("w:tcPr", &[]);
			self.out.empty("w:tcW", &[("w:w", "0"), ("w:type", "auto")]);
			res!(self.out.close("w:tcPr"));
			self.out.open("w:p", &[]);
			// The tree names the sides `Start` and `End` because it does not know which way its text
			// runs, and OOXML has the same two words for the same reason. They line up exactly, so
			// nothing here has to decide what "left" means.
			let jc = match align {
				Align::None	=> None,
				Align::Start	=> Some("start"),
				Align::Centre	=> Some("center"),
				Align::End	=> Some("end"),
			};
			match (jc, head) {
				(None, false)	=> {}
				(jc, head)	=> {
					self.out.open("w:pPr", &[]);
					if let Some(jc) = jc {
						self.out.empty("w:jc", &[("w:val", jc)]);
					}
					if head {
						self.out.open("w:rPr", &[]);
						self.out.empty("w:b", &[]);
						res!(self.out.close("w:rPr"));
					}
					res!(self.out.close("w:pPr"));
				}
			}
			res!(self.inlines(&cell.0, Fmt { bold: head, ..Fmt::default() }));
			res!(self.out.close("w:p"));
			res!(self.out.close("w:tc"));
		}
		res!(self.out.close("w:tr"));
		Ok(())
	}

	/// Writes the section properties, which say what the page is. Last thing in the body, as the
	/// schema requires.
	fn section(&mut self) -> Outcome<()> {
		let (w, h) = (fmt!("{}", PAGE_W), fmt!("{}", PAGE_H));
		let m = fmt!("{}", MARGIN);
		self.out.open("w:sectPr", &[]);
		self.out.empty("w:pgSz", &[("w:w", &w), ("w:h", &h)]);
		self.out.empty("w:pgMar", &[
			("w:top", &m),
			("w:right", &m),
			("w:bottom", &m),
			("w:left", &m),
			("w:header", "720"),
			("w:footer", "720"),
			("w:gutter", "0"),
		]);
		self.out.empty("w:cols", &[("w:space", "720")]);
		res!(self.out.close("w:sectPr"));
		Ok(())
	}
}
