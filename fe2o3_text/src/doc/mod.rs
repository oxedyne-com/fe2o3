//! A neutral document tree: what a piece of prose *is*, free of the syntax it was written in and the
//! form it will be rendered to.
//!
//! A document is a sequence of [`Block`]s, and a line of prose a sequence of [`Inline`]s. The
//! vocabulary is small and closed: a heading, a paragraph, a list, a quotation, a table, a run of
//! text, a link. That is the whole of it.
//!
//! # The tree names nothing at either end
//!
//! It names no input syntax -- there is no `Block::Asterisks`, only [`Inline::Emph`] -- so a second
//! front-end produces the same tree and every consumer of it keeps working. And it names no output
//! format, so a caller walks it and makes of it whatever it likes: HTML, a signed document, a
//! terminal rendering, an index.
//!
//! Both halves are load-bearing. A tree that admitted one syntax's spelling would make every consumer
//! learn that syntax; a tree that admitted one format's constructs -- a raw HTML node, say -- would
//! make every front-end learn that format. The tree is the narrow waist between the two, and it stays
//! narrow by carrying meaning rather than markup.
//!
//! # Attributes are names, not meanings
//!
//! A [`Block::Div`] and an [`Inline::Span`] carry [`Attrs`] -- an id, classes, key-value pairs. The
//! tree carries them and interprets none of them. `{.warning}` says a region is in the class
//! `warning`; it does not say what `warning` looks like. That is the whole of how the tree can carry a
//! named box or a styled span and still name no format: the name travels, and the meaning is supplied
//! where the tree is rendered -- a stylesheet for HTML, a style table for a signed document -- never
//! here. A tree that resolved `warning` to a colour would be an HTML tree, or an SBJ tree, and no
//! longer the narrow waist between them.
//!
//! # Front-ends
//!
//! - [`markdown`] -- reads Markdown, the form most existing prose is written in.
//! - [`djot`] -- reads Djot, which a prose author reaches for to name a box or a style the syntax of
//!   Markdown cannot.
//! - [`html`] -- reads HTML, the form a typesetter exports prose to once it has resolved the author's
//!   own macros.
//!
//! # Outputs
//!
//! - [`html`] -- writes the tree out as HTML, for a browser to read.
//!
//! # Usage
//!
//! ```ignore
//! use oxedyne_fe2o3_text::doc::markdown;
//!
//! let tree = res!(markdown::parse("# A heading\n\nA paragraph with *emphasis*.\n"));
//! for block in &tree.blocks {
//!     // Walk the tree.
//! }
//! ```

pub mod djot;
pub mod html;
pub mod markdown;
pub mod policy;

/// The attributes a [`Block::Div`] or an [`Inline::Span`] carries: an id, classes, and key-value
/// pairs.
///
/// Opaque, on purpose. The tree holds the names and interprets none of them, which is what lets it
/// carry a named box or a styled span while still naming no output format. See the module's own
/// "Attributes are names, not meanings". `{#intro .warning .boxed k=v}` parses to an id `intro`, the
/// classes `warning` and `boxed`, and the pair `(k, v)`; what any of those *mean* is the business of
/// whatever renders the tree.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attrs {
	/// The id, where one was given. At most one; a second `{#...}` replaces the first, as Djot's does.
	pub id:		Option<String>,
	/// The classes, in the order written.
	pub classes:	Vec<String>,
	/// Key-value pairs, in the order written.
	pub pairs:	Vec<(String, String)>,
}

impl Attrs {

	/// Whether these attributes name nothing at all -- no id, no class, no pair.
	///
	/// A span or a div that carries empty attributes is one the syntax marked but named nothing on;
	/// a consumer may treat it as the bare content, since there is nothing to render from an empty
	/// set.
	pub fn is_empty(&self) -> bool {
		self.id.is_none() && self.classes.is_empty() && self.pairs.is_empty()
	}
}

/// A document: the blocks it is made of, in the order they were written.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Doc {
	/// The document's blocks, in reading order.
	pub blocks: Vec<Block>,
}

impl Doc {

	/// An empty document.
	pub fn new() -> Self {
		Self { blocks: Vec::new() }
	}

	/// The text of the document's first heading of the given level, if it has one.
	///
	/// A convenience for the common case of a document whose title is its opening heading. Returns the
	/// heading's text with every inline flattened, so emphasis inside a title does not lose its words.
	pub fn first_heading(&self, level: u8) -> Option<String> {
		for block in &self.blocks {
			if let Block::Heading { level: l, content } = block {
				if *l == level {
					return Some(text_of(content));
				}
			}
		}
		None
	}

	/// The text of the document's most prominent heading: the first at the shallowest level it has.
	///
	/// What a caller after the document's own idea of its title wants. Naming a level would ask the
	/// wrong question, because which level a piece is headed by says where the prose came from rather
	/// than what it says: an author writing Markdown heads a chapter with a level 1, and the same
	/// chapter exported from Typst arrives headed by a level 2, since the exporter keeps level 1 for
	/// the document it thinks it is making. A caller asking for level 1 finds no title at all in the
	/// second, and titles the chapter after its file.
	///
	/// Taking the first heading of any level would be wrong the other way, since a piece may carry a
	/// lesser heading above its title. The shallowest level present is the prominent one, whatever
	/// number it happens to wear, and among equals the first wins.
	pub fn top_heading(&self) -> Option<String> {
		let mut top: Option<(u8, &Vec<Inline>)> = None;
		for block in &self.blocks {
			if let Block::Heading { level, content } = block {
				match top {
					// Strictly shallower, so a later heading of an equal level never displaces an
					// earlier one.
					Some((best, _)) if *level >= best	=> {},
					_					=> top = Some((*level, content)),
				}
			}
		}
		top.map(|(_, content)| text_of(content))
	}

	/// The number of words in the document's prose.
	///
	/// Counts what a reader reads: headings, paragraphs, lists, quotations, divisions and the cells of
	/// a table. A code block is left out, because a listing is scanned rather than read, and counting
	/// one at prose speed puts minutes on a piece that no reader spends.
	pub fn word_count(&self) -> usize {
		count_blocks(&self.blocks)
	}
}

/// A block-level element: the things a document is a sequence of.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
	/// A heading, of a level from 1 to 6.
	Heading {
		/// The heading's level, 1 being the most prominent.
		level:		u8,
		/// The heading's inline content.
		content:	Vec<Inline>,
	},
	/// A paragraph of inline content.
	Para(Vec<Inline>),
	/// An ordered or unordered list.
	List {
		/// Whether the list is numbered.
		ordered:	bool,
		/// The items, each a sequence of blocks, so an item may hold a paragraph, a nested list, or more.
		items:		Vec<Vec<Block>>,
	},
	/// A run of source code, preserved exactly as written.
	Code {
		/// The language the fence named, if it named one.
		lang:		Option<String>,
		/// The code itself, its line structure intact.
		text:		String,
	},
	/// A block quotation, itself a sequence of blocks.
	Quote(Vec<Block>),
	/// A table: a header row where there is one, the rows of the body, and the columns they are laid
	/// out in.
	///
	/// The table's words reach a summary or an index through its cells, each of which flattens with
	/// [`Cell::text_of`] as any other run of inlines does.
	Table {
		/// The header row, where the table names its columns. A table need not: a grid of figures is
		/// a table whether or not anything stands at the head of it.
		head:	Option<Row>,
		/// The rows of the body, in reading order.
		rows:	Vec<Row>,
		/// The columns, one entry to each, so a row's nth cell is aligned by the nth entry.
		cols:	Vec<Align>,
	},
	/// A thematic break: a division between passages.
	Rule,
	/// A named or attributed division: a box the prose itself asked for.
	///
	/// The construct a prose front-end reaches for to name a region -- an aside, a warning, a figure
	/// -- without saying, or knowing, what that region looks like. Markdown cannot write one; Djot's
	/// `:::` can. The [`Attrs`] name it and the content is a document in its own right, so a division
	/// may hold paragraphs, lists, or further divisions.
	Div {
		/// What names the division: its id, classes and pairs.
		attrs:		Attrs,
		/// The blocks the division holds.
		content:	Vec<Block>,
	},
}

/// One row of a [`Block::Table`]: the cells it holds, in reading order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row(pub Vec<Cell>);

impl Row {

	/// The row's plain text: every cell's words, a space between each.
	pub fn text_of(&self) -> String {
		let mut s = String::new();
		for (i, cell) in self.0.iter().enumerate() {
			if i > 0 {
				s.push(' ');
			}
			s.push_str(&cell.text_of());
		}
		s
	}
}

/// One cell of a [`Row`]: the inline content it holds.
///
/// A cell holds inlines and not blocks. A cell is a phrase -- a name, a figure, a link -- and a tree
/// that admitted a list or a quotation here would promise every consumer a cell it must lay out as a
/// document of its own. That is a promise no front-end this tree has can keep, and a tree should not
/// make one on their behalf.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cell(pub Vec<Inline>);

impl Cell {

	/// The cell's plain text, its every inline flattened. See [`text_of`].
	pub fn text_of(&self) -> String {
		text_of(&self.0)
	}
}

/// How a column's cells sit within the width they are given.
///
/// The sides are named `Start` and `End`, and are never named `Left` and `Right`. The tree does not
/// know left from right, because it does not know which way its text runs: this crate ships
/// [`bidi`](crate::unicode::bidi) precisely because the prose it carries may run right to left, and a
/// column aligned to the start of the line is then on the *right* of the page. `Start` is the side the
/// text begins on, whichever side that is, and the consumer -- which knows the direction it is laying
/// out in, and is the only thing that does -- is where the two meet.
///
/// A tree that said `Left` would be wrong for half the world's prose, and would be wrong silently: the
/// table would lay out, and lay out backwards. This is worth leaving alone.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Align {
	/// No alignment given, which is most columns: the consumer's own default stands.
	#[default]
	None,
	/// Aligned to the side the text begins on.
	Start,
	/// Centred within the column.
	Centre,
	/// Aligned to the side the text ends on.
	End,
}

/// An inline element: the things a line of prose is a sequence of.
#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
	/// A run of literal text.
	Text(String),
	/// Emphasised content.
	Emph {
		/// Whether the emphasis is strong (bold) rather than ordinary (italic).
		strong:		bool,
		/// The emphasised content.
		content:	Vec<Inline>,
	},
	/// A link to a destination.
	Link {
		/// Where the link points, exactly as written: a URL, a path, or any other name.
		to:		String,
		/// The link's own content, which is what a reader sees.
		content:	Vec<Inline>,
	},
	/// An image, by its source and the text that stands for it.
	Image {
		/// Where the image is, exactly as written.
		src:		String,
		/// The text that stands in for the image.
		alt:		String,
	},
	/// A span of code within a line.
	Code(String),
	/// A run of inline content the prose named or attributed.
	///
	/// The inline counterpart to [`Block::Div`]: `[text]{.highlight}` marks a span the way `:::` marks
	/// a division. The [`Attrs`] name it and interpret nothing.
	Span {
		/// What names the span.
		attrs:		Attrs,
		/// The span's content.
		content:	Vec<Inline>,
	},
	/// A break the author asked for within a paragraph.
	///
	/// Only ever a *hard* break. Where an author's editor wrapped a line is not a break the author
	/// asked for, so a front-end resolves such a wrap to a space in the surrounding [`Inline::Text`]
	/// and never emits this. A consumer may therefore honour this as a break unconditionally, and
	/// needs no rule of its own about whitespace.
	Break,
}

/// The plain text of a run of inlines, with every element flattened to its words.
///
/// Emphasis and links contribute their content, an image its alt text, a code span its code, and a
/// hard break a single space. What is left is what the passage says, with nothing of how it is marked
/// up -- which is what a title, a summary or an index wants.
pub fn text_of(content: &[Inline]) -> String {
	let mut s = String::new();
	for item in content {
		match item {
			Inline::Text(t)			=> s.push_str(t),
			Inline::Emph { content, .. }	=> s.push_str(&text_of(content)),
			Inline::Link { content, .. }	=> s.push_str(&text_of(content)),
			Inline::Image { alt, .. }	=> s.push_str(alt),
			Inline::Code(c)			=> s.push_str(c),
			Inline::Span { content, .. }	=> s.push_str(&text_of(content)),
			Inline::Break			=> s.push(' '),
		}
	}
	s
}

/// The number of words in a run of blocks, descending into those that hold blocks of their own.
fn count_blocks(blocks: &[Block]) -> usize {
	let mut n = 0;
	for block in blocks {
		n += match block {
			Block::Heading { content, .. }	=> count_inlines(content),
			Block::Para(content)		=> count_inlines(content),
			Block::List { items, .. }	=> items.iter().map(|item| count_blocks(item)).sum(),
			Block::Quote(content)		=> count_blocks(content),
			Block::Div { content, .. }	=> count_blocks(content),
			Block::Table { head, rows, .. }	=> head.iter().chain(rows)
				.map(|row| row.0.iter().map(|cell| count_inlines(&cell.0)).sum::<usize>())
				.sum(),
			// A listing is scanned rather than read, and a rule holds no words. See `Doc::word_count`.
			Block::Code { .. } | Block::Rule	=> 0,
		};
	}
	n
}

/// The number of words in a run of inlines, flattened.
///
/// A word is a segment holding at least one alphanumeric character, which is what separates the words
/// from the runs of space and punctuation that [`words`](crate::unicode::segment::words) returns
/// between them.
fn count_inlines(content: &[Inline]) -> usize {
	crate::unicode::segment::words(&text_of(content))
		.iter()
		.filter(|w| w.chars().any(char::is_alphanumeric))
		.count()
}

#[cfg(test)]
mod tests {
	use super::*;

	use oxedyne_fe2o3_core::prelude::*;

	#[test]
	fn test_the_plain_text_of_a_run_flattens_every_inline_00() -> Outcome<()> {
		// Every inline contributes its words and none of its markup, so a title reads as it was written.
		let content = vec![
			Inline::Text("A ".to_string()),
			Inline::Emph {
				strong:		true,
				content:	vec![Inline::Text("loud".to_string())],
			},
			Inline::Text(" ".to_string()),
			Inline::Link {
				to:		"somewhere".to_string(),
				content:	vec![Inline::Text("link".to_string())],
			},
		];
		assert_eq!(text_of(&content), "A loud link");
		Ok(())
	}

	/// A table says what its cells say, so a summary or an index that walks the tree finds a table's
	/// words where it finds every other block's.
	#[test]
	fn test_a_table_contributes_the_words_of_its_cells_01() -> Outcome<()> {
		let head = Row(vec![
			Cell(vec![Inline::Text("Name".to_string())]),
			Cell(vec![Inline::Text("Age".to_string())]),
		]);
		let row = Row(vec![
			Cell(vec![
				Inline::Emph {
					strong:		true,
					content:	vec![Inline::Text("Alice".to_string())],
				},
			]),
			Cell(vec![Inline::Text("30".to_string())]),
		]);
		// A cell flattens as any other run of inlines does, and a row is its cells.
		assert_eq!(head.0[0].text_of(), "Name");
		assert_eq!(head.text_of(), "Name Age");
		assert_eq!(row.text_of(), "Alice 30");
		let table = Block::Table {
			head:	Some(head),
			rows:	vec![row],
			cols:	vec![Align::Start, Align::End],
		};
		match &table {
			Block::Table { head, rows, cols }	=> {
				match head {
					Some(head)	=> assert_eq!(head.text_of(), "Name Age"),
					None		=> panic!("the table lost its header row"),
				}
				assert_eq!(rows[0].text_of(), "Alice 30");
				assert_eq!(cols.len(), 2);
			}
			other					=> panic!("expected a table, got {:?}", other),
		}
		Ok(())
	}

	/// A column nobody aligned is aligned by nothing, which is what a consumer's own default is for.
	#[test]
	fn test_an_alignment_defaults_to_none_02() -> Outcome<()> {
		assert_eq!(Align::default(), Align::None);
		Ok(())
	}

	/// The count is of words and not of the punctuation and spaces between them, and it reaches the
	/// words a nested block holds.
	#[test]
	fn test_a_document_counts_the_words_a_reader_reads_03() -> Outcome<()> {
		let text = |s: &str| vec![Inline::Text(s.to_string())];
		let doc = Doc {
			blocks: vec![
				Block::Heading { level: 1, content: text("A short title") },	// 3
				Block::Para(text("Four words, one comma.")),			// 4
				Block::Quote(vec![Block::Para(text("Two words"))]),		// 2
				Block::List {
					ordered:	false,
					items:		vec![
						vec![Block::Para(text("One"))],			// 1
						vec![Block::Para(text("Another two"))],		// 2
					],
				},
				Block::Rule,
			],
		};
		assert_eq!(doc.word_count(), 12);
		Ok(())
	}

	/// A listing is scanned rather than read, so it is not counted: a post whose bulk is code would
	/// otherwise be given minutes no reader spends on it.
	#[test]
	fn test_a_code_block_is_not_counted_04() -> Outcome<()> {
		let doc = Doc {
			blocks: vec![
				Block::Para(vec![Inline::Text("Three words here".to_string())]),
				Block::Code {
					lang:	Some("rust".to_string()),
					text:	"let a = 1; let b = 2; let c = 3;".to_string(),
				},
			],
		};
		assert_eq!(doc.word_count(), 3);
		Ok(())
	}
}
