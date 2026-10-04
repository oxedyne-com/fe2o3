//! The evaluator's page sinks: where the fixpoint's pages go, one at a time.
//!
//! A pass hands each finished page to a sink and drops it, so a sink keeps only what it writes. [`PdfSink`]
//! serialises each page's objects as they come, to chunks of bytes held until the document is done (a pass
//! that has not settled is discarded and begun again, so nothing may reach a file before the last), and at
//! `finish` writes the page tree, the outline from the final introspector, the Info dictionary from the
//! document's metadata, the subset fonts with their `/ToUnicode` maps and the cross-reference table.
//! [`VectorSink`] renders each page to an SVG document of its own, for the live view. [`CountSink`] counts
//! and drops, for the heap probe and for tests.

use crate::emit::svg;
use crate::eval::content::ElemKind;
use crate::eval::fixpoint::PageSink;
use crate::eval::intro::Introspector;
use crate::eval::select::Selector;
use crate::eval::value::Value;
use crate::eval::Engine;
use crate::page::Page;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::pdf::{
	OutlineItem,
	PdfStream,
};

use std::io::Write;

const CHUNK: usize = 256 * 1024;	// the size a chunk of output closes at

/// Output held as a list of chunks, never one buffer that doubles as it grows and for a moment holds two
/// copies of the file.
#[derive(Debug, Default)]
pub struct Chunks {
	done:	Vec<Vec<u8>>,
	cur:	Vec<u8>,
	len:	usize,
}

impl Chunks {
	/// The bytes written so far.
	pub fn len(&self) -> usize { self.len }

	pub fn is_empty(&self) -> bool { self.len == 0 }

	/// Writes every chunk, in order, to `out`.
	pub fn write_to<W: Write>(&self, out: &mut W) -> Outcome<()> {
		for c in &self.done {
			res!(out.write_all(c));
		}
		res!(out.write_all(&self.cur));
		Ok(())
	}

	/// The whole file as one buffer. For a test or a small document: it holds a second copy of the output.
	pub fn to_vec(&self) -> Vec<u8> {
		let mut v = Vec::with_capacity(self.len);
		for c in &self.done {
			v.extend_from_slice(c);
		}
		v.extend_from_slice(&self.cur);
		v
	}

	/// The chunks themselves, for a host that copies them out one by one.
	pub fn chunks(&self) -> impl Iterator<Item = &[u8]> + '_ {
		self.done.iter().map(|c| c.as_slice()).chain(std::iter::once(self.cur.as_slice()))
	}
}

impl Write for Chunks {
	fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
		self.len += buf.len();
		self.cur.extend_from_slice(buf);
		if self.cur.len() >= CHUNK {
			let full = std::mem::replace(&mut self.cur, Vec::new());
			self.done.push(full);
		}
		Ok(buf.len())
	}

	fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

/// Streams the document's pages into a PDF.
pub struct PdfSink {
	stream:	Option<PdfStream<Chunks>>,
	out:	Option<Chunks>,
}

impl std::fmt::Debug for PdfSink {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		write!(f, "PdfSink {{ open: {}, finished: {} }}", self.stream.is_some(), self.out.is_some())
	}
}

impl PdfSink {
	pub fn new() -> Outcome<Self> {
		Ok(Self { stream: Some(res!(PdfStream::open(Chunks::default(), true))), out: None })
	}

	/// The finished file, once the fixpoint has run `finish`.
	pub fn output(&self) -> Option<&Chunks> { self.out.as_ref() }

	/// Takes the finished file.
	pub fn into_output(self) -> Option<Chunks> { self.out }
}

impl PageSink for PdfSink {
	fn page(&mut self, _engine: &mut Engine, page: Page) -> Outcome<()> {
		let built = res!(crate::emit::pdf::render_page(&page));
		match self.stream.as_mut() {
			Some(s)	=> s.page(&built),
			None	=> Err(err!("The PDF sink was handed a page after it finished."; Bug)),
		}
	}

	fn discard_pass(&mut self) -> Outcome<()> {
		self.stream	= Some(res!(PdfStream::open(Chunks::default(), true)));
		self.out	= None;
		Ok(())
	}

	fn finish(&mut self, _engine: &mut Engine, intro: &Introspector) -> Outcome<()> {
		let stream = match self.stream.take() {
			Some(s)	=> s,
			None	=> return Err(err!("The PDF sink finished twice."; Bug)),
		};
		let outline	= res!(outline(intro));
		let info	= crate::emit::pdf::pdf_info(intro.info());
		self.out	= Some(res!(stream.close(outline, Some(info))));
		Ok(())
	}
}

/// The document outline, from the final introspector: each outlined heading in document order, titled
/// with its plain text, at its level and the page its start tag landed on.
pub fn outline(intro: &Introspector) -> Outcome<Vec<OutlineItem>> {
	let mut items = Vec::new();
	for h in res!(intro.query(&Selector::Elem(ElemKind::Heading, None))) {
		let outlined = !matches!(h.field("outlined"), Some(Value::Bool(false)));
		let marked = match h.field("bookmarked") {
			Some(Value::Bool(b))	=> *b,
			_						=> outlined,
		};
		if !marked {
			continue;
		}
		let level = match h.field("level") {
			Some(Value::Int(l)) if *l >= 1	=> (*l - 1) as u8,
			_								=> 0,
		};
		let title = match h.field("body") {
			Some(Value::Content(c))	=> c.plain_text(),
			_						=> String::new(),
		};
		let page = match h.location().and_then(|l| intro.page(l)) {
			Some(p) if p >= 1	=> (p - 1) as usize,
			_					=> continue,
		};
		items.push(OutlineItem { title, page, level });
	}
	Ok(items)
}

/// Renders each page of the pass to an SVG document, in order. A pass that has not settled is discarded
/// whole, so the pages held are those of the pass that did.
#[derive(Debug, Default)]
pub struct VectorSink {
	pages:		Vec<String>,
	finished:	bool,
}

impl VectorSink {
	/// The pages, once the fixpoint has run `finish`.
	pub fn into_pages(self) -> Option<Vec<String>> {
		if self.finished { Some(self.pages) } else { None }
	}
}

impl PageSink for VectorSink {
	fn page(&mut self, _engine: &mut Engine, page: Page) -> Outcome<()> {
		self.pages.push(res!(svg::render_page(&page)));
		Ok(())
	}

	fn discard_pass(&mut self) -> Outcome<()> {
		self.pages.clear();
		self.finished = false;
		Ok(())
	}

	fn finish(&mut self, _engine: &mut Engine, _intro: &Introspector) -> Outcome<()> {
		self.finished = true;
		Ok(())
	}
}

/// Counts pages and drops them.
#[derive(Debug, Default)]
pub struct CountSink {
	pub pages:		u32,
	pub finished:	bool,
}

impl PageSink for CountSink {
	fn page(&mut self, _engine: &mut Engine, _page: Page) -> Outcome<()> {
		self.pages += 1;
		Ok(())
	}

	fn discard_pass(&mut self) -> Outcome<()> {
		self.pages = 0;
		Ok(())
	}

	fn finish(&mut self, _engine: &mut Engine, _intro: &Introspector) -> Outcome<()> {
		self.finished = true;
		Ok(())
	}
}
