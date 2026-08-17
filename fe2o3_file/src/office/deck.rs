//! A neutral deck: what a presentation *is*, free of the format it will be stored in.
//!
//! The third of the neutral models, beside [`oxedyne_fe2o3_text::doc`] for prose and
//! [`crate::office::sheet`] for a grid. It is the smallest of the three, because a presentation
//! carries the least: a sequence of slides, each a title and some bullets.
//!
//! # Deliberately small, and it is not an oversight
//!
//! There is no position, no size, no colour, no picture, no transition and no animation here. A deck
//! that carried those would be a deck this had to lay out, and laying out a slide is the job the
//! reader does when it opens the file -- the same argument that keeps a layout engine out of the
//! document side. What a generator has to decide is what goes on which slide; where it sits on the
//! slide is the template's business.
//!
//! # A deck is made from prose, and the shape of the prose decides the slides
//!
//! [`Deck::from_doc`] splits a document at its headings: each heading starts a slide and is its
//! title, and everything until the next heading becomes the bullets. That is the convention every
//! Markdown-to-slides tool uses, and it is a convention rather than a rule because it is the one
//! authors already write to.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_text::doc::{
	Block,
	Doc,
	Inline,
	text_of,
};

// How deep a bullet may be indented. Beyond this a reader stops distinguishing levels, and a deck
// nested deeper than this has stopped being a deck.
pub const MAX_LEVEL: usize = 4;

/// One line of a slide's body.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bullet {
	pub level:	usize,	// zero being the outermost
	pub content:	Vec<Inline>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Slide {
	pub title:	Option<Vec<Inline>>,
	pub bullets:	Vec<Bullet>,	// the body
	pub notes:	Option<String>,	// the speaker's, which are not on the slide and are not lost either
}

impl Slide {

	pub fn titled(title: &str) -> Self {
		Self {
			title:	Some(vec![Inline::Text(title.to_string())]),
			bullets:	Vec::new(),
			notes:	None,
		}
	}

	/// The slide's words, title and bullets alike.
	pub fn text_of(&self) -> String {
		let mut out = String::new();
		if let Some(t) = &self.title {
			out.push_str(&text_of(t));
		}
		for b in &self.bullets {
			if !out.is_empty() {
				out.push('\n');
			}
			out.push_str(&text_of(&b.content));
		}
		out
	}

	/// A slide carrying only speaker's notes is empty, because nothing is on it.
	pub fn is_empty(&self) -> bool {
		self.title.is_none() && self.bullets.is_empty()
	}
}

/// A presentation: the slides it holds, in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Deck {
	pub slides:	Vec<Slide>,
}

impl Deck {

	pub fn new() -> Self {
		Self::default()
	}

	/// The deck a document makes, split at its headings.
	///
	/// **Every heading starts a slide**, whatever its level, and is that slide's title. Not the
	/// shallowest level, which is what this did first and which was wrong: a document written
	/// `# Title` and then `## Section` twice is one slide holding everything, and the author who
	/// wrote three headings expected three slides.
	///
	/// The rule that borrowed the shallowest level was
	/// [`Doc::top_heading`](oxedyne_fe2o3_text::doc::Doc::top_heading)'s, and it is right there and
	/// wrong here. That one asks "which heading is the document's TITLE", where the level says where
	/// the prose came from rather than what it means. This asks "where does a slide end", and the
	/// answer an author intends is: at the next heading. Predictable beats clever, and a deck of
	/// slightly too many slides is a deck somebody merges in a minute.
	pub fn from_doc(doc: &Doc) -> Self {
		let mut deck = Self::new();
		let mut slide = Slide::default();
		for block in &doc.blocks {
			match block {
				Block::Heading { content, .. }	=> {
					if !slide.is_empty() {
						deck.slides.push(std::mem::take(&mut slide));
					}
					slide.title = Some(content.clone());
				}
				other	=> gather(other, 0, &mut slide.bullets),
			}
		}
		if !slide.is_empty() {
			deck.slides.push(slide);
		}
		deck
	}
}

/// Adds a block's lines to a slide's bullets, at a depth.
fn gather(block: &Block, level: usize, out: &mut Vec<Bullet>) {
	let level = level.min(MAX_LEVEL);
	match block {
		Block::Para(content)	=> out.push(Bullet { level, content: content.clone() }),
		// A heading nested inside a list or a quotation is a line on the slide. One at the top of
		// the document is a new slide, and `from_doc` takes those before they reach here.
		Block::Heading { content, .. }	=> {
			out.push(Bullet { level, content: content.clone() })
		}
		Block::List { items, .. }	=> {
			for item in items {
				for (i, b) in item.iter().enumerate() {
					// The first block of an item is the item; anything after it is nested under it.
					gather(b, level + usize::from(i > 0), out);
				}
			}
		}
		Block::Quote(inner)	=> {
			for b in inner {
				gather(b, level, out);
			}
		}
		Block::Div { content, .. }	=> {
			for b in content {
				gather(b, level, out);
			}
		}
		// A listing goes on a slide as its lines. It is not prose and it is not nothing, and a deck
		// generated from a technical document is mostly this.
		Block::Code { text, .. }	=> {
			for line in text.lines() {
				out.push(Bullet {
					level,
					content: vec![Inline::Code(line.to_string())],
				});
			}
		}
		// A table on a slide would need a table on a slide, which is layout. Its rows go on as lines,
		// which says what it says and is honest about not being a table.
		Block::Table { head, rows, .. }	=> {
			for row in head.iter().chain(rows) {
				out.push(Bullet {
					level,
					content: vec![Inline::Text(row.text_of())],
				});
			}
		}
		Block::Rule	=> {}
	}
}
