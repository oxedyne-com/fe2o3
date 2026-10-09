//! A page drawn through the evaluator reaches the PDF writer with each colour in the space it was written
//! in. In [`ColourOut::Native`] the file says so: `g` and `G` for a `luma`, `k` and `K` for a `cmyk`, `rg`
//! and `RG` for the rest, in the text, a fill and a stroke alike. In [`ColourOut::Rgb`], the mode every
//! writer has by default and the bytes a file carried before an ink knew its space, there is no operator but
//! `rg` and `RG`.

use oxedyne_fe2o3_austenite::compile;
use oxedyne_fe2o3_austenite::emit::pdf::render_page;
use oxedyne_fe2o3_austenite::eval::fixpoint::PageSink;
use oxedyne_fe2o3_austenite::eval::intro::Introspector;
use oxedyne_fe2o3_austenite::eval::Engine;
use oxedyne_fe2o3_austenite::flow::text::FontStore;
use oxedyne_fe2o3_austenite::page::Page;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::pdf::{
	ColourOut,
	PdfPage,
	PdfWriter,
};

use std::path::{
	Path,
	PathBuf,
};

// Every surface has its own value in each space, so a colour found in the file is known by where it came from.
const SRC: &str = "\
#set page(width: 240pt, height: 240pt, margin: 12pt)
Plain default text.

#text(fill: luma(25%))[grey text] #text(fill: cmyk(100%, 0%, 0%, 0%))[cmyk text] #text(fill: rgb(\"#00ff00\"))[rgb text]

#rect(width: 40pt, height: 12pt, fill: luma(50%))
#rect(width: 40pt, height: 12pt, fill: cmyk(0%, 100%, 100%, 0%))
#rect(width: 40pt, height: 12pt, fill: rgb(\"#ff0000\"))

#grid(columns: (30pt,), stroke: 1pt + luma(75%), [grey ruled])
#grid(columns: (30pt,), stroke: 1pt + cmyk(0%, 0%, 100%, 0%), [cmyk ruled])
#grid(columns: (30pt,), stroke: 1pt + rgb(\"#0000ff\"), [rgb ruled])
";

/// Collects the pages and writes them with the colour mode under test, uncompressed.
struct ModeSink {
	pages:	Vec<PdfPage>,
	mode:	ColourOut,
	out:	Option<Vec<u8>>,
}

impl PageSink for ModeSink {
	fn page(&mut self, _engine: &mut Engine, page: Page) -> Outcome<()> {
		self.pages.push(res!(render_page(&page)));
		Ok(())
	}

	fn discard_pass(&mut self) -> Outcome<()> {
		self.pages.clear();
		self.out = None;
		Ok(())
	}

	fn finish(&mut self, _engine: &mut Engine, _intro: &Introspector) -> Outcome<()> {
		let mut w = PdfWriter::new().with_colour_out(self.mode.clone());
		for p in self.pages.drain(..) {
			w.add_page(p);
		}
		self.out = Some(res!(w.to_bytes()));
		Ok(())
	}
}

/// The content streams of a file written uncompressed, joined: every stream that is text, which leaves out
/// the binary font programmes.
fn content_of(pdf: &[u8]) -> String {
	let (open, close) = (&b"stream\n"[..], &b"\nendstream"[..]);
	let mut text = String::new();
	let mut at = 0;
	while let Some(i) = pdf[at..].windows(open.len()).position(|w| w == open) {
		let from = at + i + open.len();
		let len = match pdf[from..].windows(close.len()).position(|w| w == close) {
			Some(n)	=> n,
			None	=> break,
		};
		if let Ok(s) = std::str::from_utf8(&pdf[from..from + len]) {
			text.push_str(s);
			text.push('\n');
		}
		at = from + len + close.len();
	}
	text
}

fn written(name: &str, mode: ColourOut) -> Outcome<String> {
	let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("eval-ink-native").join(name);
	res!(std::fs::create_dir_all(&dir));
	res!(std::fs::write(dir.join("main.typ"), SRC));
	let main: PathBuf = dir.join("main.typ");
	let mut sink = ModeSink { pages: Vec::new(), mode, out: None };
	let done = res!(compile::assemble_eval(&main, &dir, FontStore::default(), &mut sink));
	res!(done.laid.as_ref().map_err(|e| err!("{} did not lay out: {}", name, e.plain(); Test)));
	let pdf = res!(sink.out.ok_or_else(|| err!("{}: no PDF", name; Test)));
	Ok(content_of(&pdf))
}

fn expect(content: &str, want: &[&str], absent: &[&str], mode: &str) -> Outcome<()> {
	for w in want {
		if !content.contains(w) {
			return Err(err!("{} mode: the file lacks {:?}:\n{}", mode, w, content; Test));
		}
	}
	for a in absent {
		if content.contains(a) {
			return Err(err!("{} mode: the file holds {:?}", mode, a; Test));
		}
	}
	Ok(())
}

#[test]
fn native_mode_writes_each_surface_in_its_own_space() -> Outcome<()> {
	let c = res!(written("native", ColourOut::Native));
	expect(&c, &[
		// Default text is Typst's black, a grey.
		"0 g\n",
		// Text colour, one space each.
		"0.25 g\n", "1 0 0 0 k\n", "0 1 0 rg\n",
		// Fills.
		"0.5 g\n", "0 1 1 0 k\n", "1 0 0 rg\n",
		// Strokes.
		"0.75 G\n", "0 0 1 0 K\n", "0 0 1 RG\n",
	], &[], "native")
}

#[test]
fn rgb_mode_lowers_every_surface_to_rg() -> Outcome<()> {
	let c = res!(written("rgb", ColourOut::Rgb));
	expect(&c, &[
		"0 0 0 rg\n", "0 1 0 rg\n", "1 0 0 rg\n", "0 0 1 RG\n",
	], &[" g\n", " k\n", " G\n", " K\n"], "rgb")
}

/// The two modes draw one page: they differ in the colour operators and in nothing else.
#[test]
fn the_two_modes_differ_only_in_their_colour_operators() -> Outcome<()> {
	let native	= res!(written("native-twin", ColourOut::Native));
	let rgb		= res!(written("rgb-twin", ColourOut::Rgb));
	let strip = |s: &str| -> Vec<String> {
		s.lines().filter(|l| {
			let t = l.trim_end();
			!(t.ends_with(" g") || t.ends_with(" k") || t.ends_with(" G") || t.ends_with(" K")
				|| t.ends_with(" rg") || t.ends_with(" RG"))
		}).map(|l| l.to_string()).collect()
	};
	if strip(&native) != strip(&rgb) {
		return Err(err!("the modes differ beyond their colour operators"; Test));
	}
	Ok(())
}
