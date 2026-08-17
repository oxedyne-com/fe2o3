//! XML that remembers where it came from.
//!
//! A namespace-aware reader whose every node retains the byte span of the source it was read from,
//! and a writer that edits a document by *splicing* into those spans rather than by serialising a
//! model back out.
//!
//! # Why the spans are the whole point
//!
//! The ordinary shape of an XML editor is: parse into a structure, change the structure, write the
//! structure out. Everything the structure has no field for is then silently gone. In a `.docx` that
//! is the comments, the bookmarks, the tracked changes, the content controls, the custom XML, the
//! theme and the tab stops -- and the person who notices is not the user, it is the colleague they
//! sent the file to.
//!
//! So nothing here is ever written out of the tree. An element with no handler is still a node, and
//! it serialises by emitting the bytes it was read from. An edit is a [`Splice`]: a byte range and
//! what replaces it. [`Xml::render`] walks the source, drops in the splices, and copies everything
//! else. A document nobody edited renders as the bytes it was parsed from, not because that was
//! tested but because there is no code path that could do otherwise.
//!
//! That leaves one thing worth testing, and it is tested: **the spans tile the source exactly**. Every
//! node's span, concatenated in order, is the source with nothing missing and nothing counted twice.
//! A lexer that lost a byte would show up there and nowhere else.
//!
//! # This is not [`crate::doc`], and the difference matters
//!
//! [`crate::doc`] is a neutral document tree that deliberately cannot carry markup it has no node for
//! -- see [`crate::doc::policy`] on why. That makes it right for *reading* a document into prose and
//! right for *creating* one from prose, and wrong for editing a file somebody else wrote, because
//! everything it cannot represent is everything an edit would destroy. Reach for `doc` to read or to
//! create. Reach for this to edit.
//!
//! # It is not only for Office
//!
//! `fe2o3_net`'s UPnP client hand-rolls `element_body()` and `first_element()` over strings today.
//! Anything that reads XML by looking for angle brackets is a candidate for this.
//!
//! # Usage
//!
//! ```ignore
//! use oxedyne_fe2o3_text::xml::Xml;
//!
//! let mut xml = res!(Xml::parse(&src));
//! let body = res!(xml.root()).find(&["w:body"]);
//! // ... locate a paragraph, then replace exactly its bytes.
//! res!(xml.splice(para.span.clone(), fresh));
//! let out = xml.render();
//! ```

pub mod read;
pub mod write;

use oxedyne_fe2o3_core::prelude::*;

use std::ops::Range;

/// A byte range in the source a document was read from.
pub type Span = Range<usize>;

/// How deep a document may nest its elements before the reader refuses it.
///
/// A `.docx` nests a dozen deep at its worst -- a table in a cell in a table, inside a text box. A
/// thousand is a document built to exhaust the stack of whatever reads it.
pub const DEPTH_LIMIT: usize = 256;

/// A replacement of one byte range of the source by fresh text.
///
/// The only way this module changes a document. What is outside every splice is copied, so what was
/// never understood is never touched.
#[derive(Clone, Debug, PartialEq)]
pub struct Splice {
	/// The bytes being replaced.
	pub span:	Span,
	/// What replaces them.
	pub text:	String,
}

/// A qualified name, and the namespace it resolved to.
#[derive(Clone, Debug, PartialEq)]
pub struct Name {
	/// The name as it was written, prefix and all: `w:pStyle`.
	pub qname:	String,
	/// Where the name sits in the source.
	pub span:	Span,
	/// The namespace URI it resolved to, as an index into the document's table. `None` where the name
	/// carries no prefix and no default namespace is in scope.
	pub ns:	Option<usize>,
}

impl Name {

	/// The local part: what follows the colon, or the whole name where there is none.
	pub fn local(&self) -> &str {
		match self.qname.find(':') {
			Some(i)	=> &self.qname[i + 1..],
			None		=> &self.qname,
		}
	}

	/// The prefix, empty where the name carries none.
	pub fn prefix(&self) -> &str {
		match self.qname.find(':') {
			Some(i)	=> &self.qname[..i],
			None		=> "",
		}
	}
}

/// One attribute of an element.
#[derive(Clone, Debug, PartialEq)]
pub struct Attr {
	/// The attribute's name.
	pub name:	Name,
	/// Its value, with entity references resolved.
	pub value:	String,
	/// The whole `name="value"`, in the source.
	pub span:	Span,
	/// The value alone, between its quotes.
	pub val_span:	Span,
}

/// An element, its attributes and what it holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Elem {
	/// The element's name.
	pub name:	Name,
	/// Its attributes, in the order written. Namespace declarations are among them: they are
	/// attributes, and dropping them would make the element unwritable.
	pub attrs:	Vec<Attr>,
	/// What it holds, in order.
	pub kids:	Vec<Node>,
	/// The whole element in the source, from the `<` of its open tag to the `>` of its close.
	pub span:	Span,
	/// Its open tag alone.
	pub open:	Span,
	/// What lies between the tags. `None` where the element was written `<a/>`.
	pub inner:	Option<Span>,
}

impl Elem {

	/// The value of an attribute, by the name as written.
	pub fn attr(&self, qname: &str) -> Option<&str> {
		self.attrs.iter().find(|a| a.name.qname == qname).map(|a| a.value.as_str())
	}

	/// The value of an attribute, by its namespace and local name.
	///
	/// What to ask where the document's choice of prefix is not yours to assume. A `.docx` written by
	/// Word and one written by LibreOffice agree on the URIs and need not agree on the prefixes.
	pub fn attr_ns(&self, uri: Option<usize>, local: &str) -> Option<&str> {
		self.attrs.iter()
			.find(|a| a.name.ns == uri && a.name.local() == local)
			.map(|a| a.value.as_str())
	}

	/// The child elements, in order.
	pub fn elems(&self) -> impl Iterator<Item = &Elem> {
		self.kids.iter().filter_map(|k| match k {
			Node::Elem(e)	=> Some(e),
			_		=> None,
		})
	}

	/// The first child element of that name, as written.
	pub fn child(&self, qname: &str) -> Option<&Elem> {
		self.elems().find(|e| e.name.qname == qname)
	}

	/// Every child element of that name, as written.
	pub fn children(&self, qname: &str) -> Vec<&Elem> {
		self.elems().filter(|e| e.name.qname == qname).collect()
	}

	/// The element at the end of a path of child names, where each step is the first match.
	pub fn find(&self, path: &[&str]) -> Option<&Elem> {
		let mut at = self;
		for step in path {
			at = at.child(step)?;
		}
		Some(at)
	}

	/// Every descendant of that name, in document order, the element itself included where it matches.
	pub fn all(&self, qname: &str) -> Vec<&Elem> {
		let mut out = Vec::new();
		self.gather(qname, &mut out);
		out
	}

	/// Adds this element and its descendants of that name to a list, in document order.
	fn gather<'a>(&'a self, qname: &str, out: &mut Vec<&'a Elem>) {
		if self.name.qname == qname {
			out.push(self);
		}
		for kid in self.elems() {
			kid.gather(qname, out);
		}
	}

	/// Whether the element carries a descendant of any of those names.
	///
	/// What an edit asks before it touches a span: a paragraph holding a bookmark, a comment anchor or
	/// a footnote reference is one whose deletion would leave a dangling reference in another part of
	/// the document, and no check on the bytes of *this* part would catch it.
	pub fn holds_any(&self, qnames: &[&str]) -> bool {
		if qnames.iter().any(|n| self.name.qname == *n) {
			return true;
		}
		self.elems().any(|k| k.holds_any(qnames))
	}
}

/// A node of the document: an element, or one of the things that are not elements and are still
/// bytes somebody wrote.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
	/// An element.
	Elem(Elem),
	/// Character data, as its span. Undecoded, because most of it is never looked at.
	Text(Span),
	/// A comment, whole, `<!--` to `-->`.
	Comment(Span),
	/// A processing instruction, the XML declaration among them.
	Pi(Span),
	/// A `<![CDATA[ ... ]]>` section, whole.
	CData(Span),
	/// A document type declaration, whole.
	DocType(Span),
}

impl Node {

	/// The node's span in the source, whatever kind it is.
	pub fn span(&self) -> Span {
		match self {
			Self::Elem(e)		=> e.span.clone(),
			Self::Text(s)
			| Self::Comment(s)
			| Self::Pi(s)
			| Self::CData(s)
			| Self::DocType(s)	=> s.clone(),
		}
	}
}

/// A parsed XML document, holding the source it was read from.
#[derive(Clone, Debug, Default)]
pub struct Xml {
	/// The source. Every span addresses into it and every unedited byte is written back out of it.
	src:		String,
	/// The nodes at the top of the document, in order.
	pub nodes:	Vec<Node>,
	/// Every namespace URI the document declared, once each. A [`Name`] refers to one by index.
	uris:		Vec<String>,
	/// The edits, kept in order of where they fall and never overlapping.
	edits:		Vec<Splice>,
}

impl Xml {

	/// The source the document was read from.
	pub fn source(&self) -> &str {
		&self.src
	}

	/// The raw source of a span, exactly as written.
	pub fn raw(&self, span: &Span) -> &str {
		self.src.get(span.clone()).unwrap_or("")
	}

	/// The text of a span with entity references resolved.
	pub fn text(&self, span: &Span) -> String {
		write::decode(self.raw(span))
	}

	/// The namespace URIs the document declared.
	pub fn uris(&self) -> &[String] {
		&self.uris
	}

	/// Where a namespace URI sits in the document's table, if it declared one.
	pub fn uri_index(&self, uri: &str) -> Option<usize> {
		self.uris.iter().position(|u| u == uri)
	}

	/// The document's root element.
	pub fn root(&self) -> Outcome<&Elem> {
		for node in &self.nodes {
			if let Node::Elem(e) = node {
				return Ok(e);
			}
		}
		Err(err!("The document has no root element."; Invalid, Input, Missing))
	}

	/// Whether nothing has been spliced, so rendering gives the source back.
	pub fn is_pristine(&self) -> bool {
		self.edits.is_empty()
	}

	/// The splices waiting to be rendered.
	pub fn edits(&self) -> &[Splice] {
		&self.edits
	}

	/// Replaces a byte range of the source with fresh text.
	///
	/// The range must lie within the source and must not overlap a splice already made, both of which
	/// are refused rather than resolved: two edits that overlap have no defined result, and guessing
	/// one would be a corruption nobody asked for.
	pub fn splice(&mut self, span: Span, text: String) -> Outcome<()> {
		if span.start > span.end || span.end > self.src.len() {
			return Err(err!(
				"An edit was asked for over bytes {}..{} of a document of {} bytes.",
				span.start, span.end, self.src.len(); Invalid, Input, Range));
		}
		if !self.src.is_char_boundary(span.start) || !self.src.is_char_boundary(span.end) {
			return Err(err!(
				"An edit was asked for over bytes {}..{}, which cut a character in half.",
				span.start, span.end; Invalid, Input, Range));
		}
		let at = self.edits.partition_point(|e| e.span.end <= span.start);
		if let Some(next) = self.edits.get(at) {
			if next.span.start < span.end {
				return Err(err!(
					"An edit over bytes {}..{} overlaps one already made over {}..{}.",
					span.start, span.end, next.span.start, next.span.end; Invalid, Input));
			}
		}
		self.edits.insert(at, Splice { span, text });
		Ok(())
	}

	/// Undoes every splice, so the document renders as its source again.
	pub fn revert(&mut self) {
		self.edits.clear();
	}

	/// The document as it now stands: the source, with the splices dropped in.
	///
	/// Everything outside a splice is copied, so a construct this never understood is written back
	/// exactly as it arrived.
	pub fn render(&self) -> String {
		let mut out = String::with_capacity(self.src.len());
		let mut i = 0;
		for e in &self.edits {
			out.push_str(&self.src[i..e.span.start]);
			out.push_str(&e.text);
			i = e.span.end;
		}
		out.push_str(&self.src[i..]);
		out
	}

	/// The plain text an element holds, its descendants included, with entities resolved.
	///
	/// Elements contribute nothing of themselves, so `<w:t>a</w:t><w:t>b</w:t>` gives `ab`. A caller
	/// that wants a space between runs puts one there; this does not invent characters the document
	/// does not hold.
	pub fn text_of(&self, elem: &Elem) -> String {
		let mut out = String::new();
		self.gather_text(elem, &mut out);
		out
	}

	/// Adds an element's character data to a string, descending as it goes.
	fn gather_text(&self, elem: &Elem, out: &mut String) {
		for kid in &elem.kids {
			match kid {
				Node::Text(s)	=> out.push_str(&self.text(s)),
				Node::CData(s)	=> {
					// The content of a section, without its `<![CDATA[` and `]]>`, and undecoded --
					// that is what a CDATA section is for.
					let raw = self.raw(s);
					let body = raw.strip_prefix("<![CDATA[")
						.and_then(|r| r.strip_suffix("]]>"))
						.unwrap_or(raw);
					out.push_str(body);
				}
				Node::Elem(e)	=> self.gather_text(e, out),
				_		=> {}
			}
		}
	}

	/// Every element of that name anywhere in the document, in document order.
	pub fn all(&self, qname: &str) -> Vec<&Elem> {
		let mut out = Vec::new();
		for node in &self.nodes {
			if let Node::Elem(e) = node {
				e.gather(qname, &mut out);
			}
		}
		out
	}
}
