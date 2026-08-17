//! Writing XML: escaping, the five entities XML actually has, and a small emitter.
//!
//! This is what *creates* markup. Editing existing markup does not come through here -- it goes
//! through [`Xml::splice`](crate::xml::Xml::splice), which replaces bytes and copies the rest.
//!
//! # Only five entities
//!
//! XML predefines `&amp;`, `&lt;`, `&gt;`, `&quot;` and `&apos;`, and nothing else. A numeric
//! character reference is resolved as well, since a generator writes one for anything it is unsure
//! of. `&nbsp;` is an HTML entity and is *not* XML: [`decode`] leaves it exactly as written rather
//! than inventing a character, because a document that says `&nbsp;` without declaring it is a
//! document with a bug in it, and quietly fixing it here would make the bug arrive somewhere else.

use oxedyne_fe2o3_core::prelude::*;

/// The text with the characters that cannot stand in character data escaped.
///
/// `<` and `&` must be escaped; `>` need not be, and is, because `]]>` in character data is an error
/// and escaping every `>` is the cheap way never to write one.
pub fn escape(text: &str) -> String {
	let mut out = String::with_capacity(text.len());
	for c in text.chars() {
		match c {
			'&'	=> out.push_str("&amp;"),
			'<'	=> out.push_str("&lt;"),
			'>'	=> out.push_str("&gt;"),
			_	=> out.push(c),
		}
	}
	out
}

/// The text with the characters that cannot stand in a double-quoted attribute value escaped.
pub fn escape_attr(text: &str) -> String {
	let mut out = String::with_capacity(text.len());
	for c in text.chars() {
		match c {
			'&'	=> out.push_str("&amp;"),
			'<'	=> out.push_str("&lt;"),
			'>'	=> out.push_str("&gt;"),
			'"'	=> out.push_str("&quot;"),
			// A newline in an attribute is folded to a space by every reader, so one that is meant to
			// survive has to be written as a reference.
			'\n'	=> out.push_str("&#10;"),
			'\r'	=> out.push_str("&#13;"),
			'\t'	=> out.push_str("&#9;"),
			_	=> out.push(c),
		}
	}
	out
}

/// The text with entity and character references resolved.
///
/// A reference this does not know is left exactly as it was written. See the module's own note on
/// why that is not laxity.
pub fn decode(text: &str) -> String {
	if !text.contains('&') {
		// The overwhelmingly common case, and the one worth not allocating twice for.
		return text.to_string();
	}
	let mut out = String::with_capacity(text.len());
	let b = text.as_bytes();
	let mut i = 0;
	while i < b.len() {
		if b[i] != b'&' {
			// Step by whole characters, so a multi-byte one is copied whole.
			let c = text[i..].chars().next().unwrap_or('&');
			out.push(c);
			i += c.len_utf8();
			continue;
		}
		let end = match text[i..].find(';') {
			Some(k) if k <= 12	=> i + k,
			_			=> {
				out.push('&');
				i += 1;
				continue;
			}
		};
		let body = &text[i + 1..end];
		let c = match body {
			"amp"	=> Some('&'),
			"lt"	=> Some('<'),
			"gt"	=> Some('>'),
			"quot"	=> Some('"'),
			"apos"	=> Some('\''),
			_	=> num_ref(body),
		};
		match c {
			Some(c)	=> {
				out.push(c);
				i = end + 1;
			}
			None		=> {
				out.push('&');
				i += 1;
			}
		}
	}
	out
}

/// The character a numeric reference names, where the body of a reference is one.
fn num_ref(body: &str) -> Option<char> {
	let rest = body.strip_prefix('#')?;
	let n = match rest.strip_prefix('x').or_else(|| rest.strip_prefix('X')) {
		Some(hex)	=> u32::from_str_radix(hex, 16).ok()?,
		None		=> rest.parse::<u32>().ok()?,
	};
	char::from_u32(n)
}

/// A small emitter for building well-formed XML.
///
/// It tracks what is open, so a tag cannot be closed that was not opened and a document cannot be
/// finished with something still open. That is worth having because the alternative -- pushing
/// strings into a buffer -- produces a file Word rejects with a message naming neither the part nor
/// the element.
#[derive(Debug, Default)]
pub struct Out {
	/// What has been written.
	buf:	String,
	/// The elements left open, outermost first.
	open:	Vec<String>,
}

impl Out {

	/// A new emitter, holding nothing.
	pub fn new() -> Self {
		Self::default()
	}

	/// A new emitter that has written the XML declaration every Office part begins with.
	pub fn declared() -> Self {
		let mut out = Self::new();
		out.buf.push_str(
			"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n");
		out
	}

	/// Opens an element with the given attributes, as name and value pairs.
	pub fn open(&mut self, name: &str, attrs: &[(&str, &str)]) {
		self.tag(name, attrs, false);
		self.open.push(name.to_string());
	}

	/// Writes an element that holds nothing.
	pub fn empty(&mut self, name: &str, attrs: &[(&str, &str)]) {
		self.tag(name, attrs, true);
	}

	/// Writes an element holding one run of text.
	pub fn leaf(&mut self, name: &str, attrs: &[(&str, &str)], text: &str) {
		self.tag(name, attrs, false);
		self.buf.push_str(&escape(text));
		self.buf.push_str("</");
		self.buf.push_str(name);
		self.buf.push('>');
	}

	/// Adds a run of text, escaped.
	pub fn text(&mut self, text: &str) {
		self.buf.push_str(&escape(text));
	}

	/// Adds markup already built, exactly as it stands.
	///
	/// For a fragment that came from somewhere that has already escaped it. Nothing is checked, which
	/// is why the name says what it does.
	pub fn raw(&mut self, markup: &str) {
		self.buf.push_str(markup);
	}

	/// Closes the innermost open element, which must be the one named.
	///
	/// A refusal leaves the emitter exactly as it was, so a caller that catches one and carries on is
	/// not writing into a document whose stack this quietly unwound.
	pub fn close(&mut self, name: &str) -> Outcome<()> {
		match self.open.last() {
			Some(open) if open == name	=> {}
			Some(open)	=> return Err(err!(
				"</{}> was asked for while <{}> is the innermost element open.", name, open;
				Bug, Mismatch)),
			None		=> return Err(err!(
				"</{}> was asked for with nothing open.", name; Bug)),
		}
		self.open.pop();
		self.buf.push_str("</");
		self.buf.push_str(name);
		self.buf.push('>');
		Ok(())
	}

	/// The document, which must have nothing left open.
	pub fn finish(self) -> Outcome<String> {
		if let Some(open) = self.open.last() {
			return Err(err!(
				"The document was finished with <{}> still open.", open; Bug, Missing));
		}
		Ok(self.buf)
	}

	/// Writes a tag, open or empty.
	fn tag(&mut self, name: &str, attrs: &[(&str, &str)], empty: bool) {
		self.buf.push('<');
		self.buf.push_str(name);
		for (k, v) in attrs {
			self.buf.push(' ');
			self.buf.push_str(k);
			self.buf.push_str("=\"");
			self.buf.push_str(&escape_attr(v));
			self.buf.push('"');
		}
		match empty {
			true	=> self.buf.push_str("/>"),
			false	=> self.buf.push('>'),
		}
	}
}
