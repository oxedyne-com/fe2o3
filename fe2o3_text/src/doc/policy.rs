//! Bringing an untrusted document within what a site will publish.
//!
//! The tree a reader's comment parses to is the same tree an author's essay parses to, and the
//! renderer treats them alike. It should not: an author is trusted with a link's destination and a
//! stranger is not.
//!
//! # Why this is not a sanitiser
//!
//! A sanitiser is what a pipeline needs when author-written HTML passes through it: the markup is
//! opaque, so the dangerous parts must be named and stripped, and anything the list failed to name
//! survives. **This tree cannot carry HTML at all.** There is no `Block::Html` and no `Inline::Html`
//; every node is a named construct, and [`html::render`](crate::doc::html) writes tags itself and
//! escapes every run of text and every attribute value on the way out. A `<script>` in a comment is
//! therefore already only ever the *words* `<script>`.
//!
//! What is left is not markup injection but the handful of places where a string the author wrote
//! reaches somewhere with meaning:
//!
//! - **A link's destination and an image's source.** `escape_attr` stops a destination breaking out
//!   of its quotes, and stops nothing about what the destination *is*: `javascript:` in an `href` is
//!   a script that runs on click. This is the one real hole, and the module documentation of the
//!   renderer has named it as such since it was written.
//! - **An image's source, again, for a different reason.** A remote image is a request the reader's
//!   browser makes to a third party, carrying their address and the page they are on. A commenter who
//!   can embed one can log every reader of the thread. That is tracking, whoever is doing it.
//! - **Attributes.** An `id` or a `class` a stranger chose lands in the site's own document, where it
//!   can collide with the page's ids or borrow the site's styling to dress a comment as something it
//!   is not.
//!
//! So this is an allowlist over a *tree*, not over text: it walks a document and returns one that
//! holds only what the policy permits. It is testable on the tree -- the assertion is that no
//! `javascript:` link is present afterwards, not that some string was rewritten -- and every
//! renderer, not only the HTML one, is protected by it.
//!
//! # Depth
//!
//! The walk is recursive on the same terms as the renderer's, and a policy additionally *bounds* the
//! depth: [`Policy::max_depth`] truncates a tree deeper than a caller will accept, so a document
//! built to overflow a stack is refused rather than walked.

use crate::doc::{
	Block,
	Cell,
	Doc,
	Inline,
	Row,
};


/// The URL schemes a link may use where a caller names none of its own.
///
/// The three a person writing prose actually reaches for. `javascript:` is the attack, and `data:`
/// is one too -- a `data:text/html` destination is a document of the sender's choosing served from
/// the site's own origin.
pub const SAFE_SCHEMES: &[&str] = &["http", "https", "mailto"];

/// What a document from an untrusted author may contain.
///
/// The defaults are a comment's: prose, emphasis, links to the ordinary schemes, quotes, lists and
/// code, and nothing that reaches outside the page. Loosen deliberately, field by field, rather than
/// by starting from everything and removing.
#[derive(Clone, Debug)]
pub struct Policy {
	/// The URL schemes a link may use. A destination with no scheme is governed by
	/// [`allow_relative`](Policy::allow_relative).
	pub schemes:		Vec<String>,
	/// Whether a link may point within the site, with no scheme of its own.
	pub allow_relative:	bool,
	/// Whether images survive. Off by default: an image is a request to wherever it points, made by
	/// the reader's browser, and a commenter who can place one can count the thread's readers.
	pub allow_images:	bool,
	/// Whether headings survive, and above which level they are demoted rather than dropped. A
	/// comment inside a page has no business opening an `h1`.
	pub heading_floor:	Option<u8>,
	/// Whether tables survive.
	pub allow_tables:	bool,
	/// Whether a division or span keeps the attributes that name it. Off by default: an id or a class
	/// a stranger chose lands in the site's own document.
	pub allow_attrs:	bool,
	/// How deep the tree may nest before the walk stops and drops what is below.
	pub max_depth:		usize,
}

impl Default for Policy {

	/// The policy for a comment: prose and links, nothing that reaches out of the page.
	fn default() -> Self {
		Self {
			schemes:	SAFE_SCHEMES.iter().map(|s| s.to_string()).collect(),
			allow_relative:	true,
			allow_images:	false,
			heading_floor:	Some(3),
			allow_tables:	true,
			allow_attrs:	false,
			max_depth:	16,
		}
	}
}

impl Policy {

	/// A policy that permits everything, for prose whose author is trusted.
	///
	/// Applying this is a no-op in effect and is offered so a caller can hold one type for both cases
	/// rather than branching around whether to apply a policy at all.
	pub fn trusting() -> Self {
		Self {
			schemes:	Vec::new(),	// empty means every scheme, with `allow_relative` moot
			allow_relative:	true,
			allow_images:	true,
			heading_floor:	None,
			allow_tables:	true,
			allow_attrs:	true,
			max_depth:	usize::MAX,
		}
	}

	/// Whether this policy permits every scheme, which is what an empty allowlist means.
	fn permits_every_scheme(&self) -> bool {
		self.schemes.is_empty()
	}

	/// Whether a destination is one this policy will write.
	///
	/// The scheme is what stands before the first `:`, where that colon comes before any `/`, `?` or
	/// `#` -- so `mailto:a@b` has one, `/posts/a:b` does not, and neither does `a.html`. Compared
	/// without regard to case, because `JavaScript:` is the same scheme as `javascript:` to a browser
	/// and a different string to a comparison that forgot.
	///
	/// Leading whitespace and control characters are stripped before the test: `\tjavascript:` and
	/// `java\0script:` are destinations browsers have historically honoured, and a check that looked
	/// only at the string as given would pass them.
	pub fn permits_url(&self, url: &str) -> bool {
		if self.permits_every_scheme() {
			return true;
		}
		// Control characters and whitespace anywhere in the scheme portion are removed rather than
		// merely trimmed: a browser ignores them, so a comparison must too.
		let cleaned: String = url
			.chars()
			.filter(|c| !c.is_whitespace() && !c.is_control())
			.collect();
		let scheme_end = cleaned.find(':');
		let scheme = match scheme_end {
			Some(i) => {
				// A colon that comes after a path separator is part of the path, not a scheme.
				let before = &cleaned[..i];
				if before.contains('/') || before.contains('?') || before.contains('#') {
					return self.allow_relative;
				}
				before
			}
			// No colon at all: a relative destination.
			None => return self.allow_relative,
		};
		if scheme.is_empty() {
			// A destination beginning `:` names no scheme and is nonsense; refuse it rather than
			// guess what a browser will make of it.
			return false;
		}
		let scheme = scheme.to_ascii_lowercase();
		self.schemes.iter().any(|s| s.eq_ignore_ascii_case(&scheme))
	}
}

/// Brings a document within a policy, returning what survives.
///
/// Nothing is rejected wholesale: a document is *reduced* to what is permitted, because a comment
/// that says something worth reading and one disallowed thing should lose the one thing rather than
/// the whole comment. A link whose destination is refused keeps its words and loses its link -- the
/// reader still reads the sentence, and follows nothing.
pub fn apply(doc: &Doc, policy: &Policy) -> Doc {
	Doc { blocks: blocks(&doc.blocks, policy, 0) }
}

/// The blocks that survive, at a given depth.
fn blocks(items: &[Block], policy: &Policy, depth: usize) -> Vec<Block> {
	if depth >= policy.max_depth {
		return Vec::new();
	}
	let mut out = Vec::new();
	for item in items {
		match item {
			Block::Heading { level, content } => {
				let floor = match policy.heading_floor {
					Some(f)	=> f,
					// No floor means headings are not permitted at all: the words become a
					// paragraph, since what was said still stands.
					None	=> {
						out.push(Block::Para(inlines(content, policy, depth)));
						continue;
					}
				};
				// Demoted rather than dropped: the author meant a heading, and the only thing wrong
				// with it is how loudly it would speak inside somebody else's page.
				out.push(Block::Heading {
					level:		(*level).max(floor).min(6),
					content:	inlines(content, policy, depth),
				});
			}
			Block::Para(content) => out.push(Block::Para(inlines(content, policy, depth))),
			Block::List { ordered, items } => {
				let kept: Vec<Vec<Block>> = items.iter()
					.map(|it| blocks(it, policy, depth + 1))
					.filter(|it| !it.is_empty())
					.collect();
				if !kept.is_empty() {
					out.push(Block::List { ordered: *ordered, items: kept });
				}
			}
			// Code is text and nothing else. The renderer escapes it, so it is safe as written, and
			// the language name reaches a class attribute -- which is why it is passed through the
			// same attribute rule as everything else.
			Block::Code { lang, text } => out.push(Block::Code {
				lang:	if policy.allow_attrs { lang.clone() } else { lang.as_ref()
						.filter(|l| l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
						.cloned() },
				text:	text.clone(),
			}),
			Block::Quote(inner) => {
				let kept = blocks(inner, policy, depth + 1);
				if !kept.is_empty() {
					out.push(Block::Quote(kept));
				}
			}
			Block::Table { head, rows, cols } => {
				if !policy.allow_tables {
					// A table that is not permitted still said something; its cells become
					// paragraphs rather than vanishing.
					if let Some(h) = head {
						out.extend(row_as_paras(h, policy, depth));
					}
					for r in rows {
						out.extend(row_as_paras(r, policy, depth));
					}
					continue;
				}
				out.push(Block::Table {
					head:	head.as_ref().map(|h| row(h, policy, depth)),
					rows:	rows.iter().map(|r| row(r, policy, depth)).collect(),
					cols:	cols.clone(),
				});
			}
			Block::Rule => out.push(Block::Rule),
			Block::Div { attrs, content } => {
				let kept = blocks(content, policy, depth + 1);
				if kept.is_empty() {
					continue;
				}
				if policy.allow_attrs {
					out.push(Block::Div { attrs: attrs.clone(), content: kept });
				} else {
					// The division's content is kept and its naming is not: the words were the
					// author's to write and the site's markup was not theirs to join.
					out.extend(kept);
				}
			}
		}
	}
	out
}

/// A row, its cells brought within the policy.
fn row(r: &Row, policy: &Policy, depth: usize) -> Row {
	Row(r.0.iter().map(|c| Cell(inlines(&c.0, policy, depth))).collect())
}

/// A row's cells as paragraphs, for a table a policy will not keep.
fn row_as_paras(r: &Row, policy: &Policy, depth: usize) -> Vec<Block> {
	r.0.iter()
		.map(|c| Block::Para(inlines(&c.0, policy, depth)))
		.filter(|b| match b { Block::Para(c) => !c.is_empty(), _ => true })
		.collect()
}

/// The inlines that survive.
fn inlines(items: &[Inline], policy: &Policy, depth: usize) -> Vec<Inline> {
	if depth >= policy.max_depth {
		return Vec::new();
	}
	let mut out = Vec::new();
	for item in items {
		match item {
			Inline::Text(t)	=> out.push(Inline::Text(t.clone())),
			Inline::Code(c)	=> out.push(Inline::Code(c.clone())),
			Inline::Break	=> out.push(Inline::Break),
			Inline::Emph { strong, content } => out.push(Inline::Emph {
				strong:		*strong,
				content:	inlines(content, policy, depth + 1),
			}),
			Inline::Link { to, content } => {
				let kept = inlines(content, policy, depth + 1);
				if policy.permits_url(to) {
					out.push(Inline::Link { to: to.clone(), content: kept });
				} else {
					// The words stay and the destination goes. A reader still reads the sentence
					// and follows nothing.
					out.extend(kept);
				}
			}
			Inline::Image { src, alt } => {
				if policy.allow_images && policy.permits_url(src) {
					out.push(Inline::Image { src: src.clone(), alt: alt.clone() });
				} else if !alt.trim().is_empty() {
					// What the image stood for is what the author said it stood for.
					out.push(Inline::Text(alt.clone()));
				}
			}
			Inline::Span { attrs, content } => {
				let kept = inlines(content, policy, depth + 1);
				if policy.allow_attrs {
					out.push(Inline::Span { attrs: attrs.clone(), content: kept });
				} else {
					out.extend(kept);
				}
			}
		}
	}
	out
}


#[cfg(test)]
mod tests {
	use super::*;
	use crate::doc::{Attrs, html};
	use oxedyne_fe2o3_core::prelude::*;

	fn para(inl: Vec<Inline>) -> Doc {
		Doc { blocks: vec![Block::Para(inl)] }
	}
	fn link(to: &str) -> Doc {
		para(vec![Inline::Link { to: fmt!("{}", to), content: vec![Inline::Text(fmt!("here"))] }])
	}

	/// The whole point: a script destination does not survive, and the words around it do.
	#[test]
	fn test_a_script_destination_does_not_survive_00() -> Outcome<()> {
		let p = Policy::default();
		for bad in [
			"javascript:alert(1)",
			"JavaScript:alert(1)",
			"JAVASCRIPT:alert(1)",
			"  javascript:alert(1)",
			"java\tscript:alert(1)",
			"java\nscript:alert(1)",
			"data:text/html;base64,PHNjcmlwdD4=",
			"vbscript:msgbox(1)",
			"file:///etc/passwd",
			":nonsense",
		] {
			let out = apply(&link(bad), &p);
			let html = html::render(&out);
			assert!(!html.contains("<a "), "'{}' kept its link: {}", bad, html);
			assert!(html.contains("here"), "'{}' lost the words too: {}", bad, html);
		}
		Ok(())
	}

	/// An ordinary destination is untouched.
	#[test]
	fn test_an_ordinary_destination_survives_01() -> Outcome<()> {
		let p = Policy::default();
		for good in [
			"https://example.com/a",
			"http://example.com",
			"mailto:someone@example.com",
			"/posts/a-post",
			"a-post.html",
			"#section",
			"?tag=rust",
		] {
			let html = html::render(&apply(&link(good), &p));
			assert!(html.contains("<a href="), "'{}' lost its link: {}", good, html);
		}
		Ok(())
	}

	/// A relative destination can be refused on its own, without opening every scheme.
	#[test]
	fn test_relative_can_be_refused_02() -> Outcome<()> {
		let mut p = Policy::default();
		p.allow_relative = false;
		assert!(!p.permits_url("/posts/a"));
		assert!(!p.permits_url("a.html"));
		assert!(p.permits_url("https://example.com"));
		Ok(())
	}

	/// An image is a request to a third party, so a comment does not make one; its alt text stands.
	#[test]
	fn test_an_image_does_not_survive_by_default_03() -> Outcome<()> {
		let d = para(vec![Inline::Image {
			src: fmt!("https://tracker.example/pixel.gif"),
			alt: fmt!("a picture of a cat"),
		}]);
		let html = html::render(&apply(&d, &Policy::default()));
		assert!(!html.contains("<img"), "an image survived: {}", html);
		assert!(!html.contains("tracker.example"), "the tracker's address survived: {}", html);
		assert!(html.contains("a picture of a cat"), "the alt text was lost: {}", html);

		// A caller that wants images says so, and then gets them.
		let mut p = Policy::default();
		p.allow_images = true;
		let html = html::render(&apply(&d, &p));
		assert!(html.contains("<img"), "an allowed image was still dropped: {}", html);
		Ok(())
	}

	/// A stranger does not get to name things in the site's own document.
	#[test]
	fn test_attributes_are_dropped_but_content_is_kept_04() -> Outcome<()> {
		let mut attrs = Attrs::default();
		attrs.id = Some(fmt!("main"));
		attrs.classes = vec![fmt!("site-banner")];
		let d = Doc { blocks: vec![Block::Div {
			attrs:		attrs.clone(),
			content:	vec![Block::Para(vec![Inline::Text(fmt!("the words"))])],
		}] };
		let html = html::render(&apply(&d, &Policy::default()));
		assert!(!html.contains("main"), "an id was borrowed: {}", html);
		assert!(!html.contains("site-banner"), "a class was borrowed: {}", html);
		assert!(html.contains("the words"), "the content was lost: {}", html);
		Ok(())
	}

	/// A heading is demoted rather than dropped: it should not outrank the page it sits in.
	#[test]
	fn test_a_heading_is_demoted_05() -> Outcome<()> {
		let d = Doc { blocks: vec![Block::Heading {
			level: 1, content: vec![Inline::Text(fmt!("shouting"))],
		}] };
		let html = html::render(&apply(&d, &Policy::default()));
		assert!(!html.contains("<h1"), "an h1 survived inside a page: {}", html);
		assert!(html.contains("<h3"), "the heading was not demoted to the floor: {}", html);
		assert!(html.contains("shouting"));
		Ok(())
	}

	/// A tree deeper than the policy allows is truncated rather than walked.
	#[test]
	fn test_depth_is_bounded_06() -> Outcome<()> {
		let mut inner = vec![Block::Para(vec![Inline::Text(fmt!("bottom"))])];
		for _ in 0..64 {
			inner = vec![Block::Quote(inner)];
		}
		let mut p = Policy::default();
		p.max_depth = 8;
		let out = apply(&Doc { blocks: inner }, &p);
		let html = html::render(&out);
		assert!(!html.contains("bottom"), "the bottom of a too-deep tree survived");
		// Counting the quotes that did survive proves it stopped where it was told to.
		assert!(html.matches("<blockquote>").count() <= 8, "it went deeper than told: {}", html);
		Ok(())
	}

	/// The trusting policy leaves a document as it was, so one code path serves both cases.
	#[test]
	fn test_the_trusting_policy_changes_nothing_07() -> Outcome<()> {
		let p = Policy::trusting();
		assert!(p.permits_url("javascript:alert(1)"), "the trusting policy refused a scheme");
		let d = para(vec![Inline::Image { src: fmt!("a.png"), alt: fmt!("alt") }]);
		let html = html::render(&apply(&d, &p));
		assert!(html.contains("<img"), "the trusting policy dropped an image: {}", html);
		Ok(())
	}

	/// Text is never markup, whatever it says, and the policy does not change that.
	#[test]
	fn test_words_that_look_like_markup_stay_words_08() -> Outcome<()> {
		let d = para(vec![Inline::Text(fmt!("<script>steal()</script> & <img onerror=x>"))]);
		let html = html::render(&apply(&d, &Policy::default()));
		assert!(!html.contains("<script>"), "a script tag was written: {}", html);
		assert!(!html.contains("<img"), "an img tag was written: {}", html);
		assert!(html.contains("&lt;script&gt;"), "the words were lost: {}", html);
		Ok(())
	}
}
