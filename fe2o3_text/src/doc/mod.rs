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
//! # Front-ends
//!
//! - [`markdown`] -- reads Markdown, the form most existing prose is written in.
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

pub mod html;
pub mod markdown;

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
			Inline::Break			=> s.push(' '),
		}
	}
	s
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
}
