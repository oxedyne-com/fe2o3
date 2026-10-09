//! A page drawn through the evaluator and written in a converting mode holds every colour in the output
//! space and nothing in RGB. In `Cmyk` a fill, a stroke and a run of text are `k`, `K` and `g`, `G`, a raster
//! is `/DeviceCMYK` and each page names a CMYK transparency group; in `Grey` all of them are `g`, `G`
//! and `/DeviceGray`. A soft mask is `/DeviceGray` in every mode. A gradient reaches the writer as one flat ink,
//! so it too is converted. A raster seen twice through one mode is converted once.
//!
//! The settings that build these modes, and their refusals by key, are held in `tests/watch.rs`.

use oxedyne_fe2o3_austenite::compile;
use oxedyne_fe2o3_austenite::emit::pdf::render_page;
use oxedyne_fe2o3_austenite::eval::fixpoint::PageSink;
use oxedyne_fe2o3_austenite::eval::intro::Introspector;
use oxedyne_fe2o3_austenite::eval::Engine;
use oxedyne_fe2o3_austenite::flow::text::FontStore;
use oxedyne_fe2o3_austenite::page::Page;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::colour::{
	Ink,
	Rgba,
};
use oxedyne_fe2o3_graphics::icc;
use oxedyne_fe2o3_graphics::icc::transform::{
	Dev,
	Intent,
	Transform,
};
use oxedyne_fe2o3_graphics::pdf::{
	Black,
	ColourOut,
	PdfPage,
	PdfWriter,
};
use oxedyne_fe2o3_graphics::pixmap::Pixmap;

use std::path::{
	Path,
	PathBuf,
};
use std::sync::Arc;

// Every surface has a value in a space of its own, so a colour found in the file is known by where it came from.
const SRC: &str = "\
#set page(width: 240pt, height: 300pt, margin: 12pt)
Plain default text.

#text(fill: luma(25%))[grey text] #text(fill: cmyk(100%, 0%, 0%, 0%))[cmyk text] #text(fill: rgb(\"#00ff00\"))[rgb text]

#rect(width: 40pt, height: 12pt, fill: luma(50%))
#rect(width: 40pt, height: 12pt, fill: cmyk(0%, 100%, 100%, 0%))
#rect(width: 40pt, height: 12pt, fill: rgb(\"#ff0000\"))
#rect(width: 40pt, height: 12pt, fill: gradient.linear(rgb(\"#ff0000\"), rgb(\"#0000ff\")))

#grid(columns: (30pt,), stroke: 1pt + luma(75%), [grey ruled])
#grid(columns: (30pt,), stroke: 1pt + cmyk(0%, 0%, 100%, 0%), [cmyk ruled])
#grid(columns: (30pt,), stroke: 1pt + rgb(\"#0000ff\"), [rgb ruled])

#image(\"pic.png\", width: 24pt)
#image(\"soft.png\", width: 24pt)
";

// A page of only the two pictures, for the cache.
const PICS: &str = "\
#set page(width: 120pt, height: 120pt, margin: 8pt)
#image(\"pic.png\", width: 24pt)
#image(\"soft.png\", width: 24pt)
";

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

// What a compile wrote: the text of the content streams, and the whole file read as text, for its dictionaries.
struct Written {
	content:	String,
	whole:		String,
}

// The content streams of a file written uncompressed, joined; the binary streams are left out.
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

// A project directory holding the pictures: an opaque one, and one whose left half is clear.
fn project(name: &str, src: &str) -> Outcome<PathBuf> {
	let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("colour-out").join(name);
	res!(std::fs::create_dir_all(&dir));
	res!(std::fs::write(dir.join("main.typ"), src));
	let mut pic = res!(Pixmap::filled(8, 8, Rgba::opaque(200, 40, 40)));
	for y in 0..8 {
		for x in 0..8 {
			pic.set_pixel(x, y, Rgba::opaque(25 * x as u8, 30 * y as u8, 128));
		}
	}
	res!(std::fs::write(dir.join("pic.png"), res!(pic.to_png())));
	let mut soft = res!(Pixmap::filled(8, 8, Rgba::new(10, 160, 60, 255)));
	for y in 0..8 {
		for x in 0..4 {
			soft.set_pixel(x, y, Rgba::new(10, 160, 60, 0));
		}
	}
	res!(std::fs::write(dir.join("soft.png"), res!(soft.to_png())));
	Ok(dir)
}

fn written(name: &str, src: &str, mode: ColourOut) -> Outcome<Written> {
	let dir = res!(project(name, src));
	let main = dir.join("main.typ");
	let mut sink = ModeSink { pages: Vec::new(), mode, out: None };
	let done = res!(compile::assemble_eval(&main, &dir, FontStore::default(), &mut sink));
	res!(done.laid.as_ref().map_err(|e| err!("{} did not lay out: {}", name, e.plain(); Test)));
	let pdf = res!(sink.out.ok_or_else(|| err!("{}: no PDF", name; Test)));
	Ok(Written { content: content_of(&pdf), whole: String::from_utf8_lossy(&pdf).to_string() })
}

fn cmyk_mode(black: Black) -> Outcome<ColourOut> {
	let rgb		= res!(Dev::srgb());
	let cmyk	= res!(Dev::from_profile(&res!(icc::fogra39l())));
	ColourOut::cmyk(Arc::new(res!(Transform::new(&rgb, &cmyk, Intent::Perceptual, true))), black)
}

fn grey_mode() -> Outcome<ColourOut> {
	let rgb		= res!(Dev::srgb());
	let grey	= Dev::sgray();
	let cmyk	= res!(Dev::from_profile(&res!(icc::fogra39l())));
	let xf		= Arc::new(res!(Transform::new(&rgb, &grey, Intent::Perceptual, true)));
	let cx		= Arc::new(res!(Transform::new(&cmyk, &grey, Intent::Perceptual, true)));
	ColourOut::grey(xf, cx)
}

fn absent(w: &Written, bad: &[&str], mode: &str) -> Outcome<()> {
	for b in bad {
		if w.content.contains(b) {
			return Err(err!("{} mode: the content holds {:?}:\n{}", mode, b, w.content; Test));
		}
	}
	Ok(())
}

fn present(w: &Written, want: &[&str], mode: &str) -> Outcome<()> {
	for s in want {
		if !w.content.contains(s) && !w.whole.contains(s) {
			return Err(err!("{} mode: the file lacks {:?}:\n{}", mode, s, w.content; Test));
		}
	}
	Ok(())
}

// Does the content hold an operator `op` after exactly `n` numbers, on a line of its own?
fn has_op(content: &str, op: &str, n: usize) -> bool {
	content.lines().any(|l| {
		let parts: Vec<&str> = l.split_whitespace().collect();
		parts.len() == n + 1 && parts[n] == op && parts[..n].iter().all(|p| p.parse::<f64>().is_ok())
	})
}

// A line of the content whose operator is `op` and whose numbers are CMYK: the cyan, magenta and yellow of the
// line, summed.
fn cmy_of(content: &str, op: &str) -> Vec<f64> {
	content.lines().filter_map(|l| {
		let parts: Vec<&str> = l.split_whitespace().collect();
		if parts.len() == 5 && parts[4] == op {
			let v: Vec<f64> = parts[..3].iter().filter_map(|p| p.parse::<f64>().ok()).collect();
			if v.len() == 3 { Some(v.iter().sum()) } else { None }
		} else { None }
	}).collect()
}

#[test]
fn cmyk_mode_writes_every_surface_as_cmyk_or_k_only_grey() -> Outcome<()> {
	let w = res!(written("cmyk", SRC, res!(cmyk_mode(Black::KOnly))));
	// Under K only, a grey and the default black stay on the black ink, and a CMYK ink is kept.
	present(&w, &["0 g\n", "0.25 g\n", "0.5 g\n", "0.75 G\n", "1 0 0 0 k\n", "0 1 1 0 k\n", "0 0 1 0 K\n"], "cmyk")?;
	absent(&w, &[" rg\n", " RG\n"], "cmyk")?;
	// The green and blue are converted: they are four-number `k` and `K`, with ink on a channel besides black.
	if cmy_of(&w.content, "k").iter().filter(|s| **s > 0.05).count() < 3 {
		return Err(err!("cmyk mode: the converted fills are missing:\n{}", w.content; Test));
	}
	if cmy_of(&w.content, "K").iter().filter(|s| **s > 0.05).count() < 2 {
		return Err(err!("cmyk mode: the converted strokes are missing:\n{}", w.content; Test));
	}
	// Images: the opaque one and the masked one are both CMYK, the mask is grey, and there is no RGB image.
	present(&w, &["/ColorSpace /DeviceCMYK", "/ColorSpace /DeviceGray", "/SMask"], "cmyk")?;
	if w.whole.contains("/DeviceRGB") {
		return Err(err!("cmyk mode: the file names /DeviceRGB"; Test));
	}
	// Each page says what it composites in.
	present(&w, &["/Group << /S /Transparency /CS /DeviceCMYK /I true >>"], "cmyk")
}

#[test]
fn cmyk_mode_with_a_rich_black_sends_a_neutral_through_the_profile() -> Outcome<()> {
	let w = res!(written("cmyk-rich", SRC, res!(cmyk_mode(Black::Rich))));
	absent(&w, &[" g\n", " G\n", " rg\n", " RG\n"], "cmyk rich")?;
	// The default black is the profile's own: four numbers, with more than black alone.
	if !cmy_of(&w.content, "k").iter().any(|s| *s > 0.3) {
		return Err(err!("cmyk rich mode: no neutral carries cyan, magenta and yellow:\n{}", w.content; Test));
	}
	Ok(())
}

#[test]
fn grey_mode_writes_every_surface_as_grey() -> Outcome<()> {
	let w = res!(written("grey", SRC, res!(grey_mode())));
	absent(&w, &[" rg\n", " RG\n", " k\n", " K\n"], "grey")?;
	// Fill, stroke and text are `g` and `G`; the CMYK ink goes through its own profile to a grey.
	if !has_op(&w.content, "g", 1) || !has_op(&w.content, "G", 1) {
		return Err(err!("grey mode: no `g` or no `G`:\n{}", w.content; Test));
	}
	present(&w, &["/ColorSpace /DeviceGray", "/SMask", "/Group << /S /Transparency /CS /DeviceGray /I true >>"], "grey")?;
	if w.whole.contains("/DeviceRGB") || w.whole.contains("/DeviceCMYK") {
		return Err(err!("grey mode: the file names a colour space besides /DeviceGray"; Test));
	}
	Ok(())
}

#[test]
fn native_and_rgb_modes_add_no_transparency_group_and_keep_rgb_images() -> Outcome<()> {
	for (name, mode) in [("native", ColourOut::Native), ("rgb", ColourOut::Rgb)] {
		let w = res!(written(&format!("plain-{}", name), SRC, mode));
		if w.whole.contains("/Group") {
			return Err(err!("{} mode: a page carries a /Group", name; Test));
		}
		present(&w, &["/ColorSpace /DeviceRGB"], name)?;
	}
	Ok(())
}

#[test]
fn a_flat_ink_is_taken_by_the_mode_and_a_neutral_is_exactly_neutral() -> Outcome<()> {
	let k = res!(cmyk_mode(Black::KOnly));
	let rich = res!(cmyk_mode(Black::Rich));
	let mid = Ink::Rgb(Rgba::opaque(128, 128, 128));
	match res!(k.ink(mid)) {
		Ink::Grey { v, .. }	=> assert!((v - 128.0 / 255.0).abs() < 1e-6, "{}", v),
		other				=> return Err(err!("K only sent a neutral to {:?}", other; Test)),
	}
	// A near-neutral is not neutral, so it takes the transform, and so does everything under a rich black.
	match res!(k.ink(Ink::Rgb(Rgba::opaque(128, 128, 129)))) {
		Ink::Cmyk { .. }	=> (),
		other				=> return Err(err!("K only sent a near-neutral to {:?}", other; Test)),
	}
	match res!(rich.ink(mid)) {
		Ink::Cmyk { c, m, y, .. }	=> assert!(c + m + y > 0.0, "a rich neutral holds no cyan, magenta or yellow"),
		other						=> return Err(err!("rich sent a neutral to {:?}", other; Test)),
	}
	// Under grey every ink is a grey, and a CMYK ink is read in its own profile.
	let g = res!(grey_mode());
	for ink in [mid, Ink::Cmyk { c: 1.0, m: 0.0, y: 0.0, k: 0.0, a: 255 }, Ink::Rgb(Rgba::opaque(255, 0, 0))] {
		match res!(g.ink(ink)) {
			Ink::Grey { .. }	=> (),
			other				=> return Err(err!("grey sent {:?} to {:?}", ink, other; Test)),
		}
	}
	// Native and rgb convert nothing.
	assert!(res!(ColourOut::Native.image(&[0; 12], 2, 2)).is_none());
	assert!(res!(ColourOut::Rgb.image(&[0; 12], 2, 2)).is_none());
	Ok(())
}

#[test]
fn a_raster_comes_out_with_one_channel_in_grey_and_four_in_cmyk() -> Outcome<()> {
	// A white and a black pixel, a red, and a mid grey.
	let px = [255u8, 255, 255, 0, 0, 0, 255, 0, 0, 128, 128, 128];
	let (n, c) = res!(res!(res!(cmyk_mode(Black::KOnly)).image(&px, 2, 2)).ok_or_else(|| err!("no CMYK raster"; Test)));
	assert_eq!((n, c.len()), (4, 16));
	assert!(c[..4].iter().all(|v| *v < 12), "white carries ink: {:?}", &c[..4]);
	assert!(c[4..8].iter().any(|v| *v > 200), "black carries none: {:?}", &c[4..8]);
	let (n, c) = res!(res!(res!(grey_mode()).image(&px, 2, 2)).ok_or_else(|| err!("no grey raster"; Test)));
	assert_eq!((n, c.len()), (1, 4));
	assert!(c[0] > 245 && c[1] < 10, "white and black: {:?}", c);
	assert!(c[3] > 100 && c[3] < 160, "the mid grey: {}", c[3]);
	// A raster whose length is not its size is refused.
	assert!(cmyk_mode(Black::KOnly).and_then(|m| m.image(&px[..11], 2, 2)).is_err());
	Ok(())
}

#[test]
fn a_raster_seen_again_is_read_from_the_cache() -> Outcome<()> {
	let mode = res!(cmyk_mode(Black::KOnly));
	let cache = res!(mode.images().ok_or_else(|| err!("cmyk has no cache"; Test))).clone();
	let first = res!(written("cache-a", PICS, mode.clone()));
	let a = res!(cache.stats());
	if a.misses == 0 || a.held == 0 || a.hits != 0 {
		return Err(err!("after one compile of two pictures: {:?}", a; Test));
	}
	let second = res!(written("cache-b", PICS, mode.clone()));
	let b = res!(cache.stats());
	if b.misses != a.misses || b.hits < a.misses || b.held != a.held {
		return Err(err!("a second compile converted again: {:?} then {:?}", a, b; Test));
	}
	if first.whole != second.whole {
		return Err(err!("a cached raster wrote other bytes"; Test));
	}
	// A mode made afresh has a cache of its own, which holds nothing.
	let other = res!(cmyk_mode(Black::KOnly));
	assert_eq!(res!(res!(other.images().ok_or_else(|| err!("no cache"; Test))).stats()).held, 0);
	Ok(())
}
