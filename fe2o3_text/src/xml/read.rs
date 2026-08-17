//! Reading XML into a tree that remembers its bytes.
//!
//! Stricter than [`crate::doc::html::read`], which is written for a browser's forgiveness. XML is not
//! forgiving and neither is this: a close tag that names the wrong element, an element left open at
//! the end, a quotation mark that never closes, are all refused by name rather than recovered from.
//! The documents this reads are generator output, and a generator that emits mismatched tags has a
//! bug the caller should hear about rather than have papered over.
//!
//! The one thing it is deliberately lenient about is *content it has no opinion on*. A processing
//! instruction, a comment, a CDATA section and a doctype are each read whole, as one node holding
//! their bytes, and are never looked into. They travel through an edit untouched because there is no
//! code here that could touch them.
//!
//! # Where the `>` is
//!
//! A tag's end is found by parsing its attributes rather than by searching for `>`, because `>` is a
//! legal character inside an attribute value and appears in real documents. Searching for it is the
//! bug this is written to not have.

use crate::xml::{
	Attr,
	DEPTH_LIMIT,
	Elem,
	Name,
	Node,
	Span,
	Xml,
	write::decode,
};

use oxedyne_fe2o3_core::prelude::*;

/// The namespace URI of the `xml` prefix, which is bound everywhere and declared nowhere.
pub const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

impl Xml {

	/// Reads a document.
	///
	/// The source is kept, because every byte nobody edits is written back out of it.
	pub fn parse(src: &str) -> Outcome<Self> {
		let mut p = Parse {
			src,
			b:	src.as_bytes(),
			i:	0,
			uris:	Vec::new(),
			scope:	Vec::new(),
			stack:	Vec::new(),
			marks:	Vec::new(),
			top:	Vec::new(),
		};
		res!(p.run());
		Ok(Self {
			src:	src.to_string(),
			nodes:	p.top,
			uris:	p.uris,
			edits:	Vec::new(),
		})
	}
}

/// The state of one pass over a document.
struct Parse<'a> {
	/// The source.
	src:	&'a str,
	/// The source as bytes, which is what the scanning is over.
	b:	&'a [u8],
	/// Where the next token begins.
	i:	usize,
	/// The namespace URIs seen so far, once each.
	uris:	Vec<String>,
	/// The namespace declarations in scope: a prefix, empty for the default, and what it is bound to.
	scope:	Vec<(String, Option<usize>)>,
	/// The elements left open, outermost first.
	stack:	Vec<Elem>,
	/// How much of `scope` was in force when each open element began.
	marks:	Vec<usize>,
	/// The nodes at the top of the document.
	top:	Vec<Node>,
}

impl<'a> Parse<'a> {

	/// Reads the whole document.
	fn run(&mut self) -> Outcome<()> {
		while self.i < self.b.len() {
			match self.b[self.i] {
				b'<'	=> res!(self.markup()),
				_	=> res!(self.chars()),
			}
		}
		if let Some(e) = self.stack.last() {
			return Err(err!(
				"<{}> is still open at the end of the document, which began at byte {}.",
				e.name.qname, e.span.start; Invalid, Input, Missing));
		}
		Ok(())
	}

	/// Reads a run of character data, up to the next `<`.
	fn chars(&mut self) -> Outcome<()> {
		let from = self.i;
		let to = match self.src[from..].find('<') {
			Some(k)	=> from + k,
			None		=> self.b.len(),
		};
		self.i = to;
		self.push(Node::Text(from..to));
		Ok(())
	}

	/// Reads whatever begins with a `<`.
	fn markup(&mut self) -> Outcome<()> {
		let from = self.i;
		let rest = &self.src[from..];
		if rest.starts_with("<!--") {
			let to = res!(self.until(from + 4, "-->", "a comment"));
			self.i = to;
			self.push(Node::Comment(from..to));
			return Ok(());
		}
		if rest.starts_with("<![CDATA[") {
			let to = res!(self.until(from + 9, "]]>", "a CDATA section"));
			self.i = to;
			self.push(Node::CData(from..to));
			return Ok(());
		}
		if rest.starts_with("<?") {
			let to = res!(self.until(from + 2, "?>", "a processing instruction"));
			self.i = to;
			self.push(Node::Pi(from..to));
			return Ok(());
		}
		if rest.starts_with("<!") {
			// A doctype, which may carry an internal subset in square brackets. The subset can hold a
			// `>`, so the end is the bracket's where there is one.
			let to = match rest.find('[') {
				Some(k) if k < rest.find('>').unwrap_or(usize::MAX)
					=> res!(self.until(from + k + 1, "]>", "a document type declaration")),
				_	=> res!(self.until(from + 2, ">", "a document type declaration")),
			};
			self.i = to;
			self.push(Node::DocType(from..to));
			return Ok(());
		}
		if rest.starts_with("</") {
			return self.close(from);
		}
		self.open(from)
	}

	/// Where a run ends, just past the marker that ends it.
	fn until(&self, at: usize, marker: &str, what: &str) -> Outcome<usize> {
		match self.src[at..].find(marker) {
			Some(k)	=> Ok(at + k + marker.len()),
			None		=> Err(err!(
				"{} opened at byte {} and never closed with `{}`.", what, at, marker;
				Invalid, Input, Missing)),
		}
	}

	/// Reads an open tag, and the element it opens.
	fn open(&mut self, from: usize) -> Outcome<()> {
		let ns = from + 1;
		let ne = name_end(self.b, ns);
		if ne == ns {
			return Err(err!(
				"A tag at byte {} opens with no element name.", from; Invalid, Input));
		}
		let (raw, empty, end) = res!(self.attrs(ne));
		let mark = self.scope.len();
		// The declarations an element carries are in force for the element itself, so they are read
		// before either its own name or its attributes are resolved.
		for a in &raw {
			let name = &self.src[a.name.clone()];
			let prefix = match name {
				"xmlns"					=> Some(String::new()),
				n if n.starts_with("xmlns:")	=> Some(n[6..].to_string()),
				_					=> None,
			};
			if let Some(prefix) = prefix {
				let uri = decode(&self.src[a.value.clone()]);
				// An empty URI undeclares the prefix, which is legal and means what it says.
				let at = match uri.is_empty() {
					true	=> None,
					false	=> Some(self.intern(uri)),
				};
				self.scope.push((prefix, at));
			}
		}
		let name = res!(self.name(ns..ne, true));
		let mut attrs = Vec::with_capacity(raw.len());
		for a in raw {
			attrs.push(Attr {
				name:	res!(self.name(a.name.clone(), false)),
				value:	decode(&self.src[a.value.clone()]),
				span:	a.span,
				val_span:	a.value,
			});
		}
		self.i = end;
		if empty {
			let elem = Elem { name, attrs, kids: Vec::new(), span: from..end, open: from..end, inner: None };
			self.scope.truncate(mark);
			self.push(Node::Elem(elem));
			return Ok(());
		}
		if self.stack.len() >= DEPTH_LIMIT {
			return Err(err!(
				"<{}> at byte {} nests deeper than the limit of {}. A document this deep was built \
				to exhaust the stack of whatever reads it.", name.qname, from, DEPTH_LIMIT;
				Excessive, Input));
		}
		self.stack.push(Elem { name, attrs, kids: Vec::new(), span: from..end, open: from..end, inner: None });
		self.marks.push(mark);
		Ok(())
	}

	/// Reads a close tag, and closes the element it names.
	fn close(&mut self, from: usize) -> Outcome<()> {
		let ns = from + 2;
		let ne = name_end(self.b, ns);
		let qname = &self.src[ns..ne];
		let end = res!(self.until(ne, ">", "a closing tag"));
		let mut elem = match self.stack.pop() {
			Some(e)	=> e,
			None		=> return Err(err!(
				"</{}> at byte {} closes an element that was never opened.", qname, from;
				Invalid, Input)),
		};
		if elem.name.qname != qname {
			return Err(err!(
				"</{}> at byte {} closes <{}>, which was opened at byte {}.",
				qname, from, elem.name.qname, elem.span.start; Invalid, Input, Mismatch));
		}
		let mark = match self.marks.pop() {
			Some(m)	=> m,
			None		=> return Err(err!(
				"The namespace scopes and the open elements went out of step at byte {}.", from;
				Bug)),
		};
		self.scope.truncate(mark);
		elem.inner = Some(elem.open.end..from);
		elem.span = elem.span.start..end;
		self.i = end;
		self.push(Node::Elem(elem));
		Ok(())
	}

	/// Adds a node to whatever is open, or to the top of the document.
	fn push(&mut self, node: Node) {
		match self.stack.last_mut() {
			Some(e)	=> e.kids.push(node),
			None		=> self.top.push(node),
		}
	}

	/// Reads the attributes of a tag, saying whether it closed itself and where it ended.
	///
	/// Attributes are parsed rather than skipped over, so a `>` inside a value ends nothing.
	fn attrs(&self, from: usize) -> Outcome<(Vec<Raw>, bool, usize)> {
		let mut out = Vec::new();
		let mut i = from;
		loop {
			while i < self.b.len() && self.b[i].is_ascii_whitespace() {
				i += 1;
			}
			match self.b.get(i) {
				None		=> return Err(err!(
					"A tag opened at byte {} and never closed.", from; Invalid, Input, Missing)),
				Some(b'>')	=> return Ok((out, false, i + 1)),
				Some(b'/')	=> {
					match self.b.get(i + 1) {
						Some(b'>')	=> return Ok((out, true, i + 2)),
						_		=> return Err(err!(
							"A `/` at byte {} is not followed by the `>` that would close the \
							tag.", i; Invalid, Input)),
					}
				}
				Some(_)	=> {}
			}
			let ns = i;
			let ne = name_end(self.b, ns);
			if ne == ns {
				return Err(err!(
					"Byte {} of the tag opened at byte {} is neither an attribute name nor the end \
					of the tag.", i, from; Invalid, Input));
			}
			i = ne;
			while i < self.b.len() && self.b[i].is_ascii_whitespace() {
				i += 1;
			}
			if self.b.get(i) != Some(&b'=') {
				return Err(err!(
					"The attribute `{}` at byte {} has no value. XML has no bare attribute.",
					&self.src[ns..ne], ns; Invalid, Input, Missing));
			}
			i += 1;
			while i < self.b.len() && self.b[i].is_ascii_whitespace() {
				i += 1;
			}
			let quote = match self.b.get(i) {
				Some(q) if *q == b'"' || *q == b'\''	=> *q,
				_					=> return Err(err!(
					"The value of `{}` at byte {} is not quoted.", &self.src[ns..ne], i;
					Invalid, Input)),
			};
			let vs = i + 1;
			let ve = match self.b[vs..].iter().position(|c| *c == quote) {
				Some(k)	=> vs + k,
				None		=> return Err(err!(
					"The value of `{}`, which opens at byte {}, is never closed.",
					&self.src[ns..ne], i; Invalid, Input, Missing)),
			};
			i = ve + 1;
			out.push(Raw { name: ns..ne, value: vs..ve, span: ns..i });
		}
	}

	/// Resolves a qualified name against the declarations in scope.
	///
	/// An element with no prefix takes the default namespace; an attribute with no prefix takes none,
	/// which is what the specification says and what a reader that treated them alike would get wrong
	/// for every unprefixed attribute in a namespaced document.
	fn name(&mut self, span: Span, is_elem: bool) -> Outcome<Name> {
		let qname = self.src[span.clone()].to_string();
		let prefix = match qname.find(':') {
			Some(k)	=> &qname[..k],
			None		=> "",
		};
		let ns = match (prefix, is_elem) {
			("xmlns", _)	=> None,
			("xml", _)	=> Some(self.intern(XML_NS.to_string())),
			("", false)	=> None,
			(p, _)		=> {
				match self.scope.iter().rev().find(|(q, _)| q == p) {
					Some((_, at))	=> *at,
					None			=> match p.is_empty() {
						true	=> None,
						false	=> return Err(err!(
							"The prefix `{}` at byte {} is not bound to a namespace.",
							p, span.start; Invalid, Input, Missing)),
					},
				}
			}
		};
		Ok(Name { qname, span, ns })
	}

	/// Where a URI sits in the document's table, adding it if it is new.
	fn intern(&mut self, uri: String) -> usize {
		match self.uris.iter().position(|u| *u == uri) {
			Some(k)	=> k,
			None		=> {
				self.uris.push(uri);
				self.uris.len() - 1
			}
		}
	}
}

/// One attribute as it was found, before its name is resolved.
struct Raw {
	/// The name.
	name:	Span,
	/// The value, between its quotes.
	value:	Span,
	/// The whole of it.
	span:	Span,
}

/// Where a name ends: at whitespace, or at any of the characters that end or divide a tag.
fn name_end(b: &[u8], from: usize) -> usize {
	let mut i = from;
	while i < b.len() {
		match b[i] {
			c if c.is_ascii_whitespace()	=> break,
			b'>' | b'/' | b'='		=> break,
			_				=> i += 1,
		}
	}
	i
}
