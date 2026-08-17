//! Reading a `.docx` into the neutral document tree, and saying what did not come with it.
//!
//! # The coverage trap, and the way round it
//!
//! Counting element names gets this wrong. Across a real corpus the thirty commonest names cover 92%
//! of the *content* and **zero of the documents**, because what breaks a reader is not the long tail
//! -- it is the structural singletons, `w:document`, `w:body`, `w:sectPr`, `w:tbl`, which appear once
//! each in every file. A reader that handled the top thirty would fail on all of them.
//!
//! So the names are not enumerated. Elements fall into four sets and the fourth is the important one:
//!
//! 1. **Handled** -- the ones that mean something to the tree: paragraphs, runs, text, tables, links.
//! 2. **Dropped** -- the ones whose content is not prose in reading order: properties, field
//!    instructions, deleted text, section breaks.
//! 3. **Counted and not drawn** -- pictures, charts, text boxes, embedded objects. These are real
//!    content and this cannot render them, so they are counted BY KIND and the count is handed back.
//! 4. **Descended through, transparently** -- everything else, whatever it is called. A content
//!    control, a smart tag, a bidirectional override, a custom XML block and every element invented
//!    since this was written contribute nothing themselves and their content is read where they
//!    stood.
//!
//! The fourth set is what makes the reader work on documents nobody had when it was written. It is
//! the same move [`oxedyne_fe2o3_text::doc::html::read`] makes with an unknown tag, for the same
//! reason.
//!
//! # It reads the document's own vocabulary rather than assuming Word's
//!
//! A paragraph is a heading because *its style resolves to a built-in heading name*, not because its
//! style id happens to be `Heading1`. A list is numbered rather than bulleted because
//! `word/numbering.xml` says its level's format is not `bullet`. A link's target comes from the
//! relationships part. A document written by LibreOffice, by Pages, or by a generator agrees with
//! Word on none of the ids and on all of the URIs and built-in names.
//!
//! # Tracked changes are read as the document stands
//!
//! An insertion's text is in the document, so it is read. A deletion's text is not, so it is not.
//! That is display, and display only -- nothing here authors `w:ins` or `w:del`, because getting
//! those subtly wrong corrupts a legal review and the person who finds out is a lawyer.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::office::opc::{
	REL_DOC,
	REL_NUMBERING,
	REL_STYLES,
};
use crate::zip::Zip;

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
	Xml,
};

use std::collections::BTreeMap;

// The most a single part is inflated to. A `word/document.xml` is XML, which compresses about ten to
// one, so a part this size came from an archive member of tens of megabytes. Well past any document
// and well short of trouble.
pub const MAX_PART: u64 = 64 * 1024 * 1024;

/// The leading bytes of an OLE compound file, which is what an encrypted Office document is.
const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// Something a reading view cannot draw.
///
/// Named by kind rather than counted as a lump, because "4 things are not drawn" tells a reader
/// nothing and "3 text boxes and 1 chart" tells them whether to go and open the file properly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Undrawable {
	Image,
	Chart,	// data plus a drawing of it, not re-rendered here
	Diagram,	// SmartArt or another diagram
	TextBox,	// prose sitting outside the flow of the document
	Object,	// a spreadsheet, a slide, another program's document
	Equation,
	Footnote,	// whose text is in another part
	Endnote,
	Comment,	// whose text is in another part
}

impl Undrawable {

	/// What to call some number of these, in English, singular or plural.
	///
	/// The number is the information. A band that said "some content is not shown" would be saying
	/// only that the reader cannot be trusted.
	pub fn say(&self, n: usize) -> String {
		let (one, many) = match self {
			Self::Image	=> ("image", "images"),
			Self::Chart	=> ("chart", "charts"),
			Self::Diagram	=> ("diagram", "diagrams"),
			Self::TextBox	=> ("text box", "text boxes"),
			Self::Object	=> ("embedded object", "embedded objects"),
			Self::Equation	=> ("equation", "equations"),
			Self::Footnote	=> ("footnote", "footnotes"),
			Self::Endnote	=> ("endnote", "endnotes"),
			Self::Comment	=> ("comment", "comments"),
		};
		match n {
			1	=> fmt!("{} {}", n, one),
			_	=> fmt!("{} {}", n, many),
		}
	}
}

/// A document read for reading: the prose, and an honest account of what is missing from it.
#[derive(Clone, Debug, Default)]
pub struct Reading {
	pub doc:	Doc,
	pub undrawn:	Vec<(Undrawable, usize)>,	// by kind and count, in a fixed order
	// Said, never run. A `.docm` is a `.docx` with `word/vbaProject.bin` in it, and a reader who is
	// not told is a reader who does not know what they have been sent.
	pub macros:	bool,
	// Insertions read as part of the text, which is what tracked changes look like when they are
	// displayed as accepted.
	pub tracked:	usize,
}

impl Reading {

	/// The whole account of what is not drawn, as one phrase, or nothing where everything is.
	///
	/// `"4 things are not drawn: 3 text boxes, 1 chart"`.
	pub fn say_undrawn(&self) -> Option<String> {
		if self.undrawn.is_empty() {
			return None;
		}
		let total: usize = self.undrawn.iter().map(|(_, n)| n).sum();
		let parts: Vec<String> = self.undrawn.iter().map(|(k, n)| k.say(*n)).collect();
		let what = match total {
			1	=> "thing is",
			_	=> "things are",
		};
		Some(fmt!("{} {} not drawn: {}", total, what, parts.join(", ")))
	}
}

pub fn read(bytes: &[u8]) -> Outcome<Reading> {
	if bytes.len() >= OLE_MAGIC.len() && bytes[..OLE_MAGIC.len()] == OLE_MAGIC {
		return Err(err!(
			"This document is encrypted. Office writes an encrypted file as an OLE compound file \
			with the real document inside it, and there is no password here to open it with. \
			Nothing is guessed at and nothing is shown."; Invalid, Input, Unimplemented));
	}
	let zip = res!(Zip::read(bytes.to_vec()));
	let mut out = Reading::default();
	out.macros = zip.names().iter().any(|n| n.ends_with("vbaProject.bin"));

	// The main part is whatever the package's own relationships point at. Every writer in practice
	// calls it `word/document.xml`, and a reader that assumed so would be right until it was not.
	let root_rels = res!(rels_of(&zip, ""));
	let main = res!(root_rels.values()
		.find(|(kind, _)| kind == REL_DOC)
		.map(|(_, target)| target.clone())
		.ok_or_else(|| err!(
			"The package names no main document part, so this is not a Word document. It holds: \
			{}.", zip.names().join(", "); Invalid, Input, Missing)));
	let dir = dir_of(&main);
	let xml = res!(Xml::parse(&res!(part_text(&zip, &main))));

	let rels = res!(rels_of(&zip, &main));
	let styles = res!(styles_of(&zip, &dir, &rels));
	let lists = res!(lists_of(&zip, &dir, &rels));

	let mut r = Read {
		xml:	&xml,
		styles:	&styles,
		lists:	&lists,
		rels:	&rels,
		undrawn:	BTreeMap::new(),
		tracked:	0,
	};
	let body = match res!(xml.root()).child("w:body") {
		Some(body)	=> body,
		None		=> return Err(err!(
			"'{}' has no <w:body>, so it is not a word-processing document.", main;
			Invalid, Input, Missing)),
	};
	out.doc = Doc { blocks: r.body(body) };
	out.undrawn = r.undrawn.into_iter().collect();
	out.tracked = r.tracked;
	Ok(out)
}

/// What one style says about the paragraphs that wear it.
#[derive(Clone, Debug, Default)]
struct Style {
	name:	String,	// the built-in name, lowered: `heading 1`, `quote`
	outline:	Option<u8>,	// the outline level the style itself sets
	// A writer that marks code with a CHARACTER STYLE rather than a font -- which is the tidier way
	// to write one, and what this crate's own writer does -- says nothing about the face on the run
	// itself. A reader that looked only at the run would read every code span as ordinary prose.
	mono:	bool,
	based_on:	Option<String>,
}

/// What one numbering definition says at each of its levels.
type Lists = BTreeMap<String, BTreeMap<usize, bool>>;

/// The state of one document being read.
struct Read<'a> {
	xml:	&'a Xml,	// the document part
	styles:	&'a BTreeMap<String, Style>,	// style id to what the style says
	lists:	&'a Lists,	// numbering id to level to whether that level is ordered
	rels:	&'a BTreeMap<String, (String, String)>,	// relationship id to its type and target
	undrawn:	BTreeMap<Undrawable, usize>,
	tracked:	usize,	// how many insertions were read
}

/// One paragraph that belongs to a list, on its way to being nested.
struct Item {
	lvl:	usize,	// how deep it sits
	ordered:	bool,	// numbered rather than bulleted
	blocks:	Vec<Block>,
}

impl<'a> Read<'a> {

	/// Reads the body into blocks, gathering runs of list paragraphs as it goes.
	fn body(&mut self, body: &Elem) -> Vec<Block> {
		let mut out = Vec::new();
		let mut items: Vec<Item> = Vec::new();
		for kid in body.elems() {
			match kid.name.qname.as_str() {
				// The section properties say what the page is, which is layout and not prose.
				"w:sectPr"	=> {}
				"w:tbl"	=> {
					flush(&mut out, &mut items);
					if let Some(table) = self.table(kid) {
						out.push(table);
					}
				}
				"w:p"		=> {
					match self.list_of(kid) {
						Some((lvl, ordered))	=> {
							let blocks = self.para(kid, true);
							if !blocks.is_empty() {
								items.push(Item { lvl, ordered, blocks });
							}
						}
						None			=> {
							flush(&mut out, &mut items);
							out.extend(self.para(kid, false));
						}
					}
				}
				// Anything else that stands at body level -- a content control, a custom XML block --
				// holds body-level content, and is read where it stood.
				_		=> {
					flush(&mut out, &mut items);
					out.extend(self.body(kid));
				}
			}
		}
		flush(&mut out, &mut items);
		out
	}

	fn list_of(&self, p: &Elem) -> Option<(usize, bool)> {
		let num = p.find(&["w:pPr", "w:numPr"])?;
		let id = num.child("w:numId")?.attr("w:val")?;
		// `w:numId` of zero means "no numbering", and is how Word turns a list item back into a
		// paragraph without taking the property off it.
		if id == "0" {
			return None;
		}
		let lvl = num.child("w:ilvl")
			.and_then(|e| e.attr("w:val"))
			.and_then(|v| v.parse::<usize>().ok())
			.unwrap_or(0);
		// A numbering the document refers to and does not define is still a list; the safe reading of
		// an unknown format is a bullet, which says less than a wrong number would.
		let ordered = self.lists.get(id)
			.and_then(|levels| levels.get(&lvl))
			.copied()
			.unwrap_or(false);
		Some((lvl, ordered))
	}

	/// A list item is never a heading and never a rule.
	fn para(&mut self, p: &Elem, in_list: bool) -> Vec<Block> {
		let style = p.find(&["w:pPr", "w:pStyle"]).and_then(|e| e.attr("w:val")).unwrap_or("");
		let mut content = Vec::new();
		self.inlines(p, &mut content, Fmt::default());
		let content = coalesce(content);
		// An empty paragraph is spacing, not prose. A document that used them for spacing -- and many
		// do -- would otherwise read as a column of blank lines.
		if content.is_empty() {
			return Vec::new();
		}
		if !in_list {
			if let Some(level) = self.heading_of(p, style) {
				return vec![Block::Heading { level, content }];
			}
			let kind = self.kind_of(style);
			match kind {
				Kind::Quote	=> return vec![Block::Quote(vec![Block::Para(content)])],
				Kind::Code	=> {
					let text = oxedyne_fe2o3_text::doc::text_of(&content);
					return vec![Block::Code { lang: None, text }];
				}
				Kind::Plain	=> {}
			}
		}
		vec![Block::Para(content)]
	}

	/// The outline level a paragraph sets itself wins, then the one its style sets, then the style's
	/// built-in name. Asking the id would be asking Word's spelling of a question the document
	/// answers for itself.
	fn heading_of(&self, p: &Elem, style: &str) -> Option<u8> {
		if let Some(lvl) = p.find(&["w:pPr", "w:outlineLvl"])
			.and_then(|e| e.attr("w:val"))
			.and_then(|v| v.parse::<u8>().ok())
		{
			// A body-level paragraph carries outline level 9, which is "not in the outline".
			if lvl < 9 {
				return Some(lvl + 1);
			}
		}
		let mut at = style;
		for _ in 0..8 {
			let s = self.styles.get(at)?;
			if let Some(lvl) = s.outline {
				if lvl < 9 {
					return Some(lvl + 1);
				}
			}
			if let Some(rest) = s.name.strip_prefix("heading ") {
				if let Ok(n) = rest.trim().parse::<u8>() {
					return Some(n.clamp(1, 6));
				}
			}
			if s.name == "title" {
				return Some(1);
			}
			if s.name == "subtitle" {
				return Some(2);
			}
			at = s.based_on.as_deref()?;
		}
		None
	}

	/// What a style makes of the paragraphs that wear it, beyond being a heading.
	fn kind_of(&self, style: &str) -> Kind {
		let mut at = style;
		for _ in 0..8 {
			let s = match self.styles.get(at) {
				Some(s)	=> s,
				None		=> {
					// A style the document does not define is still worth reading by its id, since
					// an id is what a generator that wrote no styles part will have used.
					let low = at.to_ascii_lowercase();
					return Kind::of(&low);
				}
			};
			let kind = Kind::of(&s.name);
			if kind != Kind::Plain {
				return kind;
			}
			let kind = Kind::of(&at.to_ascii_lowercase());
			if kind != Kind::Plain {
				return kind;
			}
			at = match s.based_on.as_deref() {
				Some(b)	=> b,
				None		=> return Kind::Plain,
			};
		}
		Kind::Plain
	}

	/// Reads the inline content of an element, descending through whatever it does not know.
	fn inlines(&mut self, at: &Elem, out: &mut Vec<Inline>, fmt: Fmt) {
		for kid in at.elems() {
			match kid.name.qname.as_str() {
				// Properties, not content.
				"w:pPr" | "w:rPr" | "w:tblPr" | "w:trPr" | "w:tcPr" | "w:sectPr" => {}
				// Marks that carry no words.
				"w:bookmarkStart" | "w:bookmarkEnd" | "w:proofErr" | "w:permStart"
				| "w:permEnd" | "w:lastRenderedPageBreak" | "w:commentRangeStart"
				| "w:commentRangeEnd" | "w:tblGrid" => {}
				// Removed text is not what the document says.
				"w:del" | "w:moveFrom" | "w:delText" | "w:delInstrText" => {}
				// A field's instruction is a program, not prose. Its cached result is read as the
				// surrounding runs.
				"w:instrText" => {}
				"w:r" => self.run(kid, out, fmt),
				"w:ins" | "w:moveTo" => {
					self.tracked += 1;
					self.inlines(kid, out, fmt);
				}
				"w:hyperlink" => {
					let to = self.target_of(kid);
					let mut inner = Vec::new();
					self.inlines(kid, &mut inner, fmt);
					let inner = coalesce(inner);
					match (to, inner.is_empty()) {
						(_, true)	=> {}
						(Some(to), _)	=> out.push(Inline::Link { to, content: inner }),
						(None, _)	=> out.extend(inner),
					}
				}
				// Two renderings of the same content, one for readers that understand a newer
				// vocabulary and one for those that do not. Reading both would say everything twice.
				"mc:AlternateContent" => {
					let pick = kid.child("mc:Choice").or_else(|| kid.child("mc:Fallback"));
					if let Some(pick) = pick {
						self.inlines(pick, out, fmt);
					}
				}
				// Everything else contributes nothing itself: a content control, a smart tag, a
				// bidirectional override, and every element invented since this was written.
				_ => self.inlines(kid, out, fmt),
			}
		}
	}

	fn run(&mut self, r: &Elem, out: &mut Vec<Inline>, fmt: Fmt) {
		let fmt = match r.child("w:rPr") {
			Some(pr)	=> Fmt {
				// `w:b` with `w:val="0"` or `"false"` turns bold OFF, which is how a run inside a
				// bold heading says "not this bit".
				bold:	on(pr.child("w:b"), fmt.bold),
				italic:	on(pr.child("w:i"), fmt.italic),
				code:	fmt.code
					|| mono(pr.child("w:rFonts"))
					|| self.style_mono(pr.child("w:rStyle").and_then(|e| e.attr("w:val"))),
			},
			None		=> fmt,
		};
		for kid in r.elems() {
			match kid.name.qname.as_str() {
				"w:rPr" | "w:instrText" | "w:delText" | "w:lastRenderedPageBreak"
				| "w:fldChar" | "w:footnoteRef" | "w:endnoteRef" | "w:annotationRef" => {}
				"w:t"		=> push_text(out, &self.xml.text_of(kid), fmt),
				"w:tab"	=> push_text(out, "\t", fmt),
				"w:br" | "w:cr"	=> out.push(Inline::Break),
				"w:noBreakHyphen"	=> push_text(out, "\u{2011}", fmt),
				// A soft hyphen is a place a word MAY break, and says nothing when it does not.
				"w:softHyphen"	=> {}
				"w:sym"	=> {
					// A symbol names a character by its code point in a symbol font.
					if let Some(c) = kid.attr("w:char")
						.and_then(|h| u32::from_str_radix(h, 16).ok())
						.and_then(char::from_u32)
					{
						push_text(out, &c.to_string(), fmt);
					}
				}
				"w:drawing" | "w:pict" | "w:object"	=> self.count_drawing(kid),
				"w:footnoteReference"			=> self.count(Undrawable::Footnote),
				"w:endnoteReference"			=> self.count(Undrawable::Endnote),
				"w:commentReference"			=> self.count(Undrawable::Comment),
				_					=> self.inlines(kid, out, fmt),
			}
		}
	}

	/// Whether a character style resolves to a monospaced face, following what it is based on.
	fn style_mono(&self, id: Option<&str>) -> bool {
		let mut at = match id {
			Some(id)	=> id,
			None		=> return false,
		};
		for _ in 0..8 {
			let s = match self.styles.get(at) {
				Some(s)	=> s,
				None		=> return false,
			};
			if s.mono {
				return true;
			}
			at = match s.based_on.as_deref() {
				Some(b)	=> b,
				None		=> return false,
			};
		}
		false
	}

	/// Counts a drawing as what it actually is, which its own subtree says.
	///
	/// A `w:drawing` is a picture, a chart, a diagram or a text box, and calling all four "an image"
	/// would tell a reader looking for the missing chart that there isn't one.
	fn count_drawing(&mut self, at: &Elem) {
		let kind = if !at.all("w:txbxContent").is_empty() {
			Undrawable::TextBox
		} else if holds_local(at, "chart") {
			Undrawable::Chart
		} else if holds_local(at, "relIds") || holds_local(at, "dgm") {
			Undrawable::Diagram
		} else if at.name.qname == "w:object" {
			Undrawable::Object
		} else if holds_local(at, "oMath") || holds_local(at, "oMathPara") {
			Undrawable::Equation
		} else {
			Undrawable::Image
		};
		self.count(kind);
	}

	fn count(&mut self, what: Undrawable) {
		*self.undrawn.entry(what).or_insert(0) += 1;
	}

	/// Where a link points: a relationship for a link out, an anchor for one within.
	fn target_of(&self, link: &Elem) -> Option<String> {
		if let Some(id) = link.attr("r:id") {
			if let Some((_, target)) = self.rels.get(id) {
				return Some(target.clone());
			}
		}
		link.attr("w:anchor").map(|a| fmt!("#{}", a))
	}

	fn table(&mut self, tbl: &Elem) -> Option<Block> {
		let mut rows = Vec::new();
		let mut head = None;
		for (i, tr) in tbl.children("w:tr").into_iter().enumerate() {
			let mut cells = Vec::new();
			for tc in tr.children("w:tc") {
				let mut content = Vec::new();
				for (k, p) in tc.children("w:p").into_iter().enumerate() {
					if k > 0 {
						content.push(Inline::Break);
					}
					self.inlines(p, &mut content, Fmt::default());
				}
				// A cell holds a phrase, not a document: see `Cell`'s own note on why.
				cells.push(Cell(coalesce(content)));
			}
			// A header row is one the table SAYS repeats, or a first row whose every cell is bold.
			// Both are signals a writer actually emits; guessing from anything less would put a row of
			// data where a reader expects column names.
			let is_head = i == 0 && (tr.find(&["w:trPr", "w:tblHeader"]).is_some() || all_bold(tr));
			match is_head {
				true	=> head = Some(Row(cells)),
				false	=> rows.push(Row(cells)),
			}
		}
		if head.is_none() && rows.is_empty() {
			return None;
		}
		let n = head.iter().chain(rows.iter()).map(|r| r.0.len()).max().unwrap_or(0);
		// The alignment of a column is a property of each cell in OOXML rather than of the column, so
		// the first row that says anything decides. A table whose cells disagree has no column
		// alignment to report.
		let cols = (0..n).map(|i| self.align_of(tbl, i)).collect();
		Some(Block::Table { head, rows, cols })
	}

	/// The alignment of a column, from the first cell in it that names one.
	fn align_of(&self, tbl: &Elem, col: usize) -> Align {
		for tr in tbl.children("w:tr") {
			if let Some(tc) = tr.children("w:tc").get(col) {
				if let Some(p) = tc.child("w:p") {
					if let Some(jc) = p.find(&["w:pPr", "w:jc"]).and_then(|e| e.attr("w:val")) {
						return match jc {
							"start" | "left"	=> Align::Start,
							"center" | "centre"	=> Align::Centre,
							"end" | "right"	=> Align::End,
							_			=> Align::None,
						};
					}
				}
			}
		}
		Align::None
	}
}

/// What a style makes of a paragraph, beyond a heading.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
	Plain,
	Quote,
	Code,	// a listing
}

impl Kind {

	/// What a lowered style name or id says the paragraph is.
	fn of(low: &str) -> Self {
		match low {
			// The spellings are the ones writers actually emit. `block quotation` is
			// LibreOffice's; `intense quote` is Word's; `quotations` is OpenDocument's. Reading
			// only Word's would call two of the three ordinary paragraphs.
			"quote" | "blockquote" | "block text" | "intense quote" | "quotations"
			| "blocktext" | "intensequote" | "block quotation" | "blockquotation"	=> Self::Quote,
			"source code" | "sourcecode" | "html preformatted" | "htmlpreformatted"
			| "preformatted text" | "preformattedtext" | "code" | "plain text"
			| "plaintext" | "macro text"			=> Self::Code,
			_						=> Self::Plain,
		}
	}
}

/// How a run of text is marked.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Fmt {
	bold:	bool,
	italic:	bool,
	code:	bool,	// monospaced, which the tree carries as a code span
}

/// Whether a toggle property is on.
///
/// An element that is simply there is on; one carrying `w:val="0"` or `"false"` is off, which is how a
/// run inside a bold heading says "not this bit". Absent, it inherits.
fn on(e: Option<&Elem>, inherited: bool) -> bool {
	match e {
		None		=> inherited,
		Some(e)	=> match e.attr("w:val") {
			Some("0") | Some("false") | Some("off")	=> false,
			_					=> true,
		},
	}
}

/// Whether a font specification names a monospaced face.
///
/// A short list of the faces a writer actually reaches for. It is a guess and it is a cheap one: at
/// worst a passage arrives as a code span rather than as prose, which loses nothing a reader needs.
fn mono(e: Option<&Elem>) -> bool {
	let name = match e.and_then(|e| e.attr("w:ascii")) {
		Some(n)	=> n.to_ascii_lowercase(),
		None		=> return false,
	};
	matches!(name.as_str(),
		"consolas" | "courier" | "courier new" | "monaco" | "menlo" | "liberation mono"
		| "dejavu sans mono" | "lucida console" | "andale mono" | "cascadia code"
		| "cascadia mono" | "sf mono" | "jetbrains mono" | "fira code" | "source code pro")
}

/// Whether every run in a row is bold, which is one of the two signals that says a header row.
fn all_bold(tr: &Elem) -> bool {
	let runs = tr.all("w:r");
	!runs.is_empty() && runs.iter().all(|r| {
		// A run holding no text says nothing either way, so it does not veto.
		r.all("w:t").is_empty() || r.find(&["w:rPr", "w:b"]).is_some()
	})
}

/// Whether an element or any of its descendants has that local name, whatever prefix it wears.
///
/// By local name because the prefix is the document's choice: a chart is `c:chart` in one writer's
/// output and `chart` under a default namespace in another's.
fn holds_local(at: &Elem, local: &str) -> bool {
	if at.name.local() == local {
		return true;
	}
	at.elems().any(|k| holds_local(k, local))
}

fn push_text(out: &mut Vec<Inline>, text: &str, fmt: Fmt) {
	if text.is_empty() {
		return;
	}
	let mut item = match fmt.code {
		true	=> Inline::Code(text.to_string()),
		false	=> Inline::Text(text.to_string()),
	};
	// Strong outside emphasis, so `**a *b* **` nests the way a writer would have written it.
	if fmt.italic {
		item = Inline::Emph { strong: false, content: vec![item] };
	}
	if fmt.bold {
		item = Inline::Emph { strong: true, content: vec![item] };
	}
	out.push(item);
}

/// Joins adjacent inlines that are marked alike.
///
/// A run is the unit formatting applies to in OOXML, and a writer splits one wherever it likes -- a
/// spell-check mark, a bookmark, a language change. Left alone, a bold phrase arrives as six bold
/// inlines and renders as `**a****b**`, which is not what it says.
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
	// Trailing whitespace on a paragraph is the writer's, not the author's.
	if let Some(Inline::Text(last)) = out.last_mut() {
		let trimmed = last.trim_end_matches(['\n', '\r']);
		if trimmed.len() != last.len() {
			*last = trimmed.to_string();
		}
	}
	out.retain(|i| !matches!(i, Inline::Text(t) if t.is_empty()));
	out
}

/// Turns a gathered run of list paragraphs into the nested lists they make, and adds them.
fn flush(out: &mut Vec<Block>, items: &mut Vec<Item>) {
	if items.is_empty() {
		return;
	}
	let taken = std::mem::take(items);
	out.extend(nest(&taken, 0));
}

/// Nests a run of list items by their levels.
///
/// A document says only what level each item is at; the nesting is this. An item deeper than the one
/// before it belongs inside it, which is what makes a flat run of paragraphs a tree.
fn nest(items: &[Item], depth: usize) -> Vec<Block> {
	let mut out: Vec<Block> = Vec::new();
	let mut i = 0;
	while i < items.len() {
		let ordered = items[i].ordered;
		let mut list: Vec<Vec<Block>> = Vec::new();
		// One list runs while the items stay at this level and keep the same kind.
		while i < items.len() && items[i].lvl <= depth {
			if items[i].ordered != ordered && items[i].lvl == depth {
				break;
			}
			let mut blocks = items[i].blocks.clone();
			i += 1;
			// Everything deeper that follows belongs to the item just taken.
			let from = i;
			while i < items.len() && items[i].lvl > depth {
				i += 1;
			}
			if i > from {
				blocks.extend(nest(&items[from..i], depth + 1));
			}
			list.push(blocks);
		}
		if list.is_empty() {
			// An item deeper than its depth with nothing above it: take it at this level rather than
			// spin.
			list.push(items[i].blocks.clone());
			i += 1;
		}
		out.push(Block::List { ordered, items: list });
	}
	out
}

fn part_text(zip: &Zip, name: &str) -> Outcome<String> {
	let bytes = res!(zip.content_capped(name, MAX_PART));
	Ok(res!(String::from_utf8(bytes), Decode, String))
}

/// The directory a part sits in, with its trailing slash, so a relative target resolves against it.
fn dir_of(part: &str) -> String {
	match part.rfind('/') {
		Some(k)	=> part[..k + 1].to_string(),
		None		=> String::new(),
	}
}

/// Where a relationship target actually is within the package.
fn resolve(dir: &str, target: &str) -> String {
	match target.starts_with('/') {
		true	=> target[1..].to_string(),
		false	=> fmt!("{}{}", dir, target),
	}
}

/// The relationships a part owns, by id.
///
/// A part's relationships live beside it, in a `_rels` directory, in a file named after it. The
/// package's own are in `_rels/.rels`, which is the same rule with an empty name.
fn rels_of(zip: &Zip, part: &str) -> Outcome<BTreeMap<String, (String, String)>> {
	let dir = dir_of(part);
	let name = &part[dir.len()..];
	let path = fmt!("{}_rels/{}.rels", dir, name);
	let mut out = BTreeMap::new();
	if !zip.has(&path) {
		return Ok(out);
	}
	let src = res!(part_text(zip, &path));
	let xml = res!(Xml::parse(&src));
	for rel in res!(xml.root()).children("Relationship") {
		let id = match rel.attr("Id") {
			Some(id)	=> id.to_string(),
			None		=> continue,
		};
		let kind = rel.attr("Type").unwrap_or("").to_string();
		let target = rel.attr("Target").unwrap_or("").to_string();
		// An external target is a URL and stays as written; an internal one is a path within the
		// package and is resolved against the part that names it.
		let target = match rel.attr("TargetMode") {
			Some("External")	=> target,
			_			=> resolve(&dir, &target),
		};
		out.insert(id, (kind, target));
	}
	Ok(out)
}

/// The styles the document defines, by id.
fn styles_of(
	zip:	&Zip,
	dir:	&str,
	rels:	&BTreeMap<String, (String, String)>,
)
	-> Outcome<BTreeMap<String, Style>>
{
	let mut out = BTreeMap::new();
	let part = match part_of(rels, REL_STYLES, dir, "styles.xml", zip) {
		Some(p)	=> p,
		None		=> return Ok(out),
	};
	let src = res!(part_text(zip, &part));
	let xml = res!(Xml::parse(&src));
	for s in res!(xml.root()).children("w:style") {
		let id = match s.attr("w:styleId") {
			Some(id)	=> id.to_string(),
			None		=> continue,
		};
		out.insert(id, Style {
			name:	s.child("w:name")
				.and_then(|e| e.attr("w:val"))
				.unwrap_or("")
				.to_ascii_lowercase(),
			outline:	s.find(&["w:pPr", "w:outlineLvl"])
				.and_then(|e| e.attr("w:val"))
				.and_then(|v| v.parse::<u8>().ok()),
			mono:	mono(s.find(&["w:rPr", "w:rFonts"])),
			based_on:	s.child("w:basedOn")
				.and_then(|e| e.attr("w:val"))
				.map(|v| v.to_string()),
		});
	}
	Ok(out)
}

/// What each numbering definition's levels are, by the id a paragraph names.
///
/// Two hops: a paragraph names a `w:num`, a `w:num` names an abstract definition, and the abstract
/// definition is where the levels live. A reader that skipped the indirection would read the abstract
/// id as the paragraph's, and every list in a document with more than one would be the wrong kind.
fn lists_of(
	zip:	&Zip,
	dir:	&str,
	rels:	&BTreeMap<String, (String, String)>,
)
	-> Outcome<Lists>
{
	let mut out = Lists::new();
	let part = match part_of(rels, REL_NUMBERING, dir, "numbering.xml", zip) {
		Some(p)	=> p,
		None		=> return Ok(out),
	};
	let src = res!(part_text(zip, &part));
	let xml = res!(Xml::parse(&src));
	let root = res!(xml.root());
	let mut abstracts: BTreeMap<String, BTreeMap<usize, bool>> = BTreeMap::new();
	for a in root.children("w:abstractNum") {
		let id = match a.attr("w:abstractNumId") {
			Some(id)	=> id.to_string(),
			None		=> continue,
		};
		let mut levels = BTreeMap::new();
		for lvl in a.children("w:lvl") {
			let n = lvl.attr("w:ilvl").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
			let fmt = lvl.child("w:numFmt").and_then(|e| e.attr("w:val")).unwrap_or("bullet");
			levels.insert(n, fmt != "bullet" && fmt != "none");
		}
		abstracts.insert(id, levels);
	}
	for num in root.children("w:num") {
		let id = match num.attr("w:numId") {
			Some(id)	=> id.to_string(),
			None		=> continue,
		};
		let at = num.child("w:abstractNumId").and_then(|e| e.attr("w:val")).unwrap_or("");
		if let Some(levels) = abstracts.get(at) {
			out.insert(id, levels.clone());
		}
	}
	Ok(out)
}

/// Where a supporting part is: what the relationships say, or the conventional name where they say
/// nothing.
///
/// The relationship is the authority. The fallback is for a document whose rels part is missing or
/// which never declared one, which is common in generator output and which a reader can still make
/// sense of rather than refuse.
fn part_of(
	rels:	&BTreeMap<String, (String, String)>,
	kind:	&str,
	dir:	&str,
	usual:	&str,
	zip:	&Zip,
)
	-> Option<String>
{
	if let Some((_, target)) = rels.values().find(|(k, _)| k == kind) {
		if zip.has(target) {
			return Some(target.clone());
		}
	}
	let guess = fmt!("{}{}", dir, usual);
	match zip.has(&guess) {
		true	=> Some(guess),
		false	=> None,
	}
}
