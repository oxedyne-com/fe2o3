// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-layout `inline/shaping.rs` and typst-library `text/mod.rs`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
// U6a owns this file: text shaping under resolved `text` styles, and the font store `Engine::fonts`
// holds -- family lookup by name, weight, style and stretch over the embedded faces and every font file
// in the vfs, with Typst's fallback list and a warning on a missing family.
//
// Shaping is a port of Typst 0.15.1's `typst-layout/src/inline/shaping.rs` (Apache-2.0, (c) the Typst
// project authors): a run is shaped with the first family that has a face for the variant, and every
// stretch that face cannot draw is shaped again with the next family, down the list and then Typst's
// fallback list; tracking and spacing are applied per glyph; every glyph carries the stretch and shrink
// justification may give it (`par.justification-limits`). Advances are kept in ems, as Typst keeps them,
// so the line breaker weighs the same widths the oracle weighs.

use crate::eval::content::ElemKind;
use crate::eval::styles::StyleChain;
use crate::eval::value::{
	Dict,
	Length,
	Paint,
	Relative,
	Value,
};
use crate::eval::Engine;
use crate::font::ShapedText;
use crate::fonts::{
	is_default_ignorable,
	stretch_of_ratio,
	BookFace,
	FaceStyle,
	FaceVariant,
	FontBook,
};
use crate::ir::{
	Dims,
	Graphic,
	DrawOp,
	Leaf,
	LinkTarget,
	Node,
	Sp,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::shape::{
	Dir as ShapeDir,
	Feature,
	Glyph as RunGlyph,
	ShapeSpec,
};
use oxedyne_fe2o3_graphics::colour::Rgba;
use oxedyne_fe2o3_graphics::path::{
	Bounds,
	Path,
	PathBuilder,
	Pt,
};
use oxedyne_fe2o3_graphics::transform::Transform;
use oxedyne_fe2o3_text::regex::Regex;
use oxedyne_fe2o3_text::unicode::lookup::Partitioned;
use oxedyne_fe2o3_text::unicode::prop::Script;

use std::path::PathBuf;
use std::sync::Arc;

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ FONT STORE                                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// The faces available to one compilation: Typst's embedded faces the crate carries, then every font
/// file under the directories and byte buffers the caller adds. The book is built on first use and
/// rebuilt after an addition.
#[derive(Clone, Debug, Default)]
pub struct FontStore {
	pub families:	Vec<String>,			// family names known, for diagnostics
	book:			Option<Arc<FontBook>>,
	base:			Option<Arc<FontBook>>,	// the embedded faces, parsed once by the host, else parsed here
	dirs:			Vec<PathBuf>,
	files:			Vec<Arc<Vec<u8>>>,
	shape_budget:	Option<usize>,			// bytes the shaped-run cache may hold, else the default
}

impl FontStore {
	/// A store over a book of the embedded faces the host parsed once. A compile that adds no font of its own
	/// shares that book; one that does forks it and adds to the fork, so the base never changes and no
	/// project's fonts reach another's.
	pub fn with_base(base: Arc<FontBook>) -> Self {
		Self { base: Some(base), ..Self::default() }
	}

	/// Adds every font file beneath `dir` in the vfs, after the embedded faces.
	pub fn add_dir(&mut self, dir: PathBuf) {
		self.dirs.push(dir);
		self.book = None;
	}

	/// The font directories added, and every font file found in them once the book is built: what a watch
	/// polls so that a font added, mended or removed is seen.
	pub fn scanned(&self) -> Vec<PathBuf> {
		let mut out = self.dirs.clone();
		if let Some(book) = &self.book {
			out.extend(book.scanned().iter().cloned());
		}
		out
	}

	/// Adds one font file's bytes, after the embedded faces and any directory.
	pub fn add_bytes(&mut self, bytes: Vec<u8>) {
		self.files.push(Arc::new(bytes));
		self.book = None;
	}

	/// Sets the bytes the shaped-run cache may hold, in the book now and in every book built later.
	pub fn set_shape_budget(&mut self, bytes: usize) -> Outcome<()> {
		self.shape_budget = Some(bytes);
		if let Some(book) = &self.book {
			res!(book.set_shape_budget(bytes));
		}
		Ok(())
	}

	/// The book, built on first use.
	pub fn book(&mut self) -> Outcome<Arc<FontBook>> {
		if let Some(b) = &self.book {
			return Ok(b.clone());
		}
		let mut book = match &self.base {
			Some(base) if self.dirs.is_empty() && self.files.is_empty()	=> {
				self.families = base.family_names();
				if let Some(bytes) = self.shape_budget {
					res!(base.set_shape_budget(bytes));
				}
				self.book = Some(base.clone());
				return Ok(base.clone());
			},
			Some(base)	=> base.fork(),
			None		=> res!(FontBook::embedded()),
		};
		for dir in &self.dirs {
			book.add_dir(dir);
		}
		for bytes in &self.files {
			let _ = book.add(bytes.as_ref().clone());
		}
		self.families = book.family_names();
		if let Some(bytes) = self.shape_budget {
			res!(book.set_shape_budget(bytes));
		}
		let book = Arc::new(book);
		self.book = Some(book.clone());
		Ok(book)
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ RESOLVED TEXT PROPERTIES                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// One family of a `font` list: the lower-cased name and the characters it is restricted to.
#[derive(Clone, Debug)]
pub struct FamilySpec {
	pub name:	String,
	pub covers:	Option<Arc<Regex>>,
}

/// A top or bottom text edge: a font metric, the glyph bounds, or a fixed length in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edge {
	Ascender,
	CapHeight,
	XHeight,
	Baseline,
	Descender,
	Bounds,
	Length(f64),
}

impl Edge {
	fn of(v: &Value, size: f64, top: bool) -> Self {
		match v {
			Value::Str(s) => match s.as_str() {
				"ascender"		=> Edge::Ascender,
				"cap-height"	=> Edge::CapHeight,
				"x-height"		=> Edge::XHeight,
				"baseline"		=> Edge::Baseline,
				"descender"		=> Edge::Descender,
				"bounds"		=> Edge::Bounds,
				_				=> if top { Edge::CapHeight } else { Edge::Baseline },
			},
			Value::Length(l)	=> Edge::Length(l.resolve(size)),
			_					=> if top { Edge::CapHeight } else { Edge::Baseline },
		}
	}
}

/// Typst's `text.costs`, each a ratio of the default cost.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Costs {
	pub hyphenation:	f64,
	pub runt:			f64,
	pub widow:			f64,
	pub orphan:			f64,
}

impl Default for Costs {
	fn default() -> Self { Self { hyphenation: 1.0, runt: 1.0, widow: 1.0, orphan: 1.0 } }
}

/// Typst's `par.justification-limits`: how far a space may shrink and stretch (as a ratio of its width
/// plus a length in points) and how far tracking may go between other glyphs, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
	pub spacing_min:	(f64, f64),
	pub spacing_max:	(f64, f64),
	pub tracking_min:	f64,
	pub tracking_max:	f64,
}

impl Default for Limits {
	fn default() -> Self {
		Self { spacing_min: (2.0 / 3.0, 0.0), spacing_max: (1.5, 0.0), tracking_min: 0.0, tracking_max: 0.0 }
	}
}

/// A sub- or superscript request (`text`'s internal `shift-settings`): whether to prefer the font's own
/// forms, the shift and size in ems of the outer size when given, and which kind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShiftSettings {
	pub typographic:	bool,
	pub shift:			Option<f64>,	// ems, positive upwards
	pub size:			Option<f64>,	// ems
	pub superscript:	bool,
}

/// What a decoration draws.
#[derive(Clone, Debug)]
pub enum DecoLine {
	Underline { stroke: Option<DecoStroke>, offset: Option<f64>, evade: bool, background: bool },
	Overline { stroke: Option<DecoStroke>, offset: Option<f64>, evade: bool, background: bool },
	Strikethrough { stroke: Option<DecoStroke>, offset: Option<f64>, background: bool },
	Highlight { fill: Option<Rgba>, top: Edge, bottom: Edge },
}

/// A decoration's stroke, resolved: its colour (the text fill when unset) and thickness in points (the
/// font's recommended thickness when unset).
#[derive(Clone, Copy, Debug)]
pub struct DecoStroke {
	pub paint:		Option<Rgba>,
	pub thickness:	Option<f64>,
}

/// One decoration and how far it extends past the text on each side, in points.
#[derive(Clone, Debug)]
pub struct Deco {
	pub line:	DecoLine,
	pub extent:	f64,
}

/// The `text` properties shaping and inline layout read, resolved once for a style chain.
#[derive(Clone, Debug)]
pub struct TextProps {
	pub size:				f64,
	pub families:			Vec<FamilySpec>,	// the `font` list, then the fallback list when `fallback` is on
	pub fallback:			bool,
	pub variant:			FaceVariant,
	pub features:			Vec<Feature>,
	pub lang:				String,
	pub region:				Option<String>,
	pub script:				Option<[u8; 4]>,
	pub rtl:				bool,
	pub tracking:			f64,
	pub spacing:			(f64, f64),			// ratio of the space's width, plus points
	pub fill:				Rgba,
	pub baseline:			f64,
	pub overhang:			bool,
	pub top_edge:			Edge,
	pub bottom_edge:		Edge,
	pub hyphenate:			Option<bool>,
	pub costs:				Costs,
	pub decos:				Vec<Deco>,
	pub shift:				Option<ShiftSettings>,
	pub case:				Option<bool>,		// Some(true) upper, Some(false) lower
	pub hidden:				bool,
	pub cjk_latin_spacing:	bool,
	pub limits:				Limits,
	pub link:				Option<LinkTarget>,	// where the text links (`link.current`)
	pub link_outset:		f64,				// half the leading, which a link area adds above and below
}

/// Typst's fallback families, tried after a `font` list when `fallback` is on.
const FALLBACKS: &[&str] = &[
	"libertinus serif",
	"twitter color emoji",
	"noto color emoji",
	"apple color emoji",
	"segoe ui emoji",
];

/// A field of `text` in force on the chain, schema default included.
pub fn text_field(styles: &StyleChain, name: &str) -> Outcome<Option<Value>> {
	match ElemKind::Text.field_id(name) {
		Some(id)	=> styles.get(ElemKind::Text, id),
		None		=> Ok(None),
	}
}

/// Every value set for a field of `text`, outermost first.
fn text_values(styles: &StyleChain, name: &str) -> Vec<Value> {
	match ElemKind::Text.field_id(name) {
		Some(id)	=> styles.values(ElemKind::Text, id).into_iter().rev().cloned().collect(),
		None		=> Vec::new(),
	}
}

/// A field of another element in force on the chain, when that element's schema has it.
pub fn elem_field(styles: &StyleChain, kind: ElemKind, name: &str) -> Outcome<Option<Value>> {
	match kind.field_id(name) {
		Some(id)	=> styles.get(kind, id),
		None		=> Ok(None),
	}
}

/// A paint as the one colour the back end draws with: a colour as it is, a gradient by its first stop.
pub fn paint_rgba(v: &Value) -> Outcome<Option<Rgba>> {
	match v {
		Value::Color(c)		=> Ok(Some(res!(c.to_rgba()))),
		Value::Gradient(g)	=> match g.stops.first() {
			Some((c, _))	=> Ok(Some(res!(c.to_rgba()))),
			None			=> Ok(None),
		},
		Value::Stroke(s)	=> match &s.paint {
			Some(Paint::Color(c))		=> Ok(Some(res!(c.to_rgba()))),
			Some(Paint::Gradient(g))	=> match g.stops.first() {
				Some((c, _))	=> Ok(Some(res!(c.to_rgba()))),
				None			=> Ok(None),
			},
			_							=> Ok(None),
		},
		_					=> Ok(None),
	}
}

fn length_pt(v: &Value, size: f64) -> Option<f64> {
	match v {
		Value::Length(l)	=> Some(l.resolve(size)),
		_					=> None,
	}
}

fn relative_of(v: &Value, size: f64) -> Option<(f64, f64)> {
	match v {
		Value::Ratio(r)		=> Some((r.0, 0.0)),
		Value::Length(l)	=> Some((0.0, l.resolve(size))),
		Value::Relative(r)	=> Some((r.rel.0, r.abs.resolve(size))),
		_					=> None,
	}
}

/// The weight a `weight` value names: a number, or one of Typst's names.
pub fn weight_of(v: &Value) -> Option<u16> {
	match v {
		Value::Int(i)	=> Some((*i).clamp(0, u16::MAX as i64) as u16),
		Value::Str(s)	=> match s.as_str() {
			"thin"			=> Some(100),
			"extralight"	=> Some(200),
			"light"			=> Some(300),
			"regular"		=> Some(400),
			"medium"		=> Some(500),
			"semibold"		=> Some(600),
			"bold"			=> Some(700),
			"extrabold"		=> Some(800),
			"black"			=> Some(900),
			_				=> None,
		},
		_				=> None,
	}
}

/// The font variant the chain asks for: `style`, `weight` and `stretch`, thickened by `strong`'s delta
/// and slanted or straightened by `emph`'s toggle.
pub fn variant(styles: &StyleChain) -> Outcome<FaceVariant> {
	let style = match res!(text_field(styles, "style")) {
		Some(Value::Str(s)) => match s.as_str() {
			"italic"	=> FaceStyle::Italic,
			"oblique"	=> FaceStyle::Oblique,
			_			=> FaceStyle::Normal,
		},
		_ => FaceStyle::Normal,
	};
	let weight = res!(text_field(styles, "weight")).as_ref().and_then(weight_of).unwrap_or(400).clamp(100, 900);
	let stretch = match res!(text_field(styles, "stretch")) {
		Some(Value::Ratio(r))	=> stretch_of_ratio(r.0),
		_						=> 1000,
	};
	let mut v = FaceVariant { style, weight, stretch };
	let delta: i64 = text_values(styles, "delta").iter().map(|d| match d {
		Value::Int(i)	=> *i,
		_				=> 0,
	}).sum();
	v = v.thicken(delta);
	let emph = text_values(styles, "emph").iter().fold(false, |acc, e| acc ^ matches!(e, Value::Bool(true)));
	if emph {
		v.style = match v.style {
			FaceStyle::Normal	=> FaceStyle::Italic,
			FaceStyle::Italic	=> FaceStyle::Normal,
			FaceStyle::Oblique	=> FaceStyle::Normal,
		};
	}
	Ok(v)
}

/// The families the chain's `font` names, as Typst resolves them, with the fallback list after when
/// `fallback` is on.
pub fn families(styles: &StyleChain) -> Outcome<(Vec<FamilySpec>, bool)> {
	let mut out = Vec::new();
	match res!(text_field(styles, "font")) {
		Some(v) => res!(push_families(&v, &mut out)),
		None	=> out.push(FamilySpec { name: "libertinus serif".to_string(), covers: None }),
	}
	let fallback = !matches!(res!(text_field(styles, "fallback")), Some(Value::Bool(false)));
	if fallback {
		for f in FALLBACKS {
			out.push(FamilySpec { name: f.to_string(), covers: None });
		}
	}
	Ok((out, fallback))
}

fn push_families(v: &Value, out: &mut Vec<FamilySpec>) -> Outcome<()> {
	match v {
		Value::Str(s)	=> out.push(FamilySpec { name: s.to_lowercase(), covers: None }),
		Value::Dict(d)	=> {
			let name = match d.get("name") {
				Some(Value::Str(s))	=> s.to_lowercase(),
				_					=> return Err(err!("a font family dictionary needs a `name`"; Input, Invalid)),
			};
			let covers = match d.get("covers") {
				Some(Value::Str(s)) if s.as_str() == "latin-in-cjk" => Some(Arc::new(res!(Regex::new(
					"[^\u{00B7}\u{2013}\u{2014}\u{2018}\u{2019}\u{201C}\u{201D}\u{2025}-\u{2027}\u{2E3A}]")))),
				Some(Value::Regex(r))	=> Some(Arc::new(r.re.clone())),
				_						=> None,
			};
			out.push(FamilySpec { name, covers });
		}
		Value::Array(a)	=> {
			if a.is_empty() {
				return Err(err!("font fallback list must not be empty"; Input, Invalid));
			}
			for item in a.iter() {
				res!(push_families(item, out));
			}
		}
		_				=> (),
	}
	Ok(())
}

/// The OpenType features the chain asks for, in Typst's order: the switches that are on by default only
/// when turned off, the others only when turned on, then `features` as given.
pub fn features(styles: &StyleChain) -> Outcome<Vec<Feature>> {
	let mut tags = Vec::new();
	let field = |name: &str| text_field(styles, name);
	if matches!(res!(field("kerning")), Some(Value::Bool(false))) {
		tags.push(Feature::set(b"kern", 0));
	}
	if let Some(Value::Str(s)) = res!(field("smallcaps")) {
		tags.push(Feature::set(b"smcp", 1));
		if s.as_str() == "all" {
			tags.push(Feature::set(b"c2sc", 1));
		}
	}
	match res!(field("alternates")) {
		Some(Value::Bool(true))		=> tags.push(Feature::set(b"salt", 1)),
		Some(Value::Int(i)) if i > 0	=> tags.push(Feature::set(b"salt", i as u32)),
		_							=> (),
	}
	let sets: Vec<i64> = match res!(field("stylistic-set")) {
		Some(Value::Int(i))		=> vec![i],
		Some(Value::Array(a))	=> a.iter().filter_map(|v| match v { Value::Int(i) => Some(*i), _ => None }).collect(),
		_						=> Vec::new(),
	};
	let mut sets: Vec<i64> = sets.into_iter().filter(|i| (1..=20).contains(i)).collect();
	sets.sort();
	sets.dedup();
	for set in sets {
		let t = [b's', b's', b'0' + (set / 10) as u8, b'0' + (set % 10) as u8];
		tags.push(Feature::set(&t, 1));
	}
	if matches!(res!(field("ligatures")), Some(Value::Bool(false))) {
		tags.push(Feature::set(b"liga", 0));
		tags.push(Feature::set(b"clig", 0));
	}
	if matches!(res!(field("discretionary-ligatures")), Some(Value::Bool(true))) {
		tags.push(Feature::set(b"dlig", 1));
	}
	if matches!(res!(field("historical-ligatures")), Some(Value::Bool(true))) {
		tags.push(Feature::set(b"hlig", 1));
	}
	match res!(field("number-type")) {
		Some(Value::Str(s)) if s.as_str() == "lining"		=> tags.push(Feature::set(b"lnum", 1)),
		Some(Value::Str(s)) if s.as_str() == "old-style"	=> tags.push(Feature::set(b"onum", 1)),
		_													=> (),
	}
	match res!(field("number-width")) {
		Some(Value::Str(s)) if s.as_str() == "proportional"	=> tags.push(Feature::set(b"pnum", 1)),
		Some(Value::Str(s)) if s.as_str() == "tabular"		=> tags.push(Feature::set(b"tnum", 1)),
		_													=> (),
	}
	if matches!(res!(field("slashed-zero")), Some(Value::Bool(true))) {
		tags.push(Feature::set(b"zero", 1));
	}
	if matches!(res!(field("fractions")), Some(Value::Bool(true))) {
		tags.push(Feature::set(b"frac", 1));
	}
	for v in text_values(styles, "features") {
		match v {
			Value::Array(a) => for t in a.iter() {
				if let Value::Str(s) = t {
					if let Some(tag) = tag_of(s) {
						tags.push(Feature::set(&tag, 1));
					}
				}
			},
			Value::Dict(d) => for (k, val) in d.iter() {
				if let (Some(tag), Value::Int(n)) = (tag_of(k), val) {
					tags.push(Feature::set(&tag, (*n).clamp(0, u32::MAX as i64) as u32));
				}
			},
			_ => (),
		}
	}
	Ok(tags)
}

/// A four-byte OpenType tag from a string of one to four printable characters, space padded.
fn tag_of(s: &str) -> Option<[u8; 4]> {
	let b = s.as_bytes();
	if b.is_empty() || b.len() > 4 || !b.iter().all(|c| (0x21..=0x7E).contains(c)) {
		return None;
	}
	let mut t = [b' '; 4];
	t[..b.len()].copy_from_slice(b);
	Some(t)
}

/// Is `lang` written right to left?
pub fn lang_is_rtl(lang: &str) -> bool {
	matches!(lang, "ar" | "dv" | "fa" | "he" | "ks" | "pa" | "ps" | "sd" | "ug" | "ur" | "yi")
}

/// The justification limits `par` puts in force, Typst's defaults for whatever is unset.
pub fn limits(styles: &StyleChain, size: f64) -> Outcome<Limits> {
	let mut l = Limits::default();
	if let Some(Value::Dict(d)) = res!(elem_field(styles, ElemKind::Par, "justification-limits")) {
		if let Some(Value::Dict(s)) = d.get("spacing") {
			if let Some(v) = s.get("min").and_then(|v| relative_of(v, size)) { l.spacing_min = v; }
			if let Some(v) = s.get("max").and_then(|v| relative_of(v, size)) { l.spacing_max = v; }
		}
		if let Some(Value::Dict(t)) = d.get("tracking") {
			if let Some(v) = t.get("min").and_then(|v| length_pt(v, size)) { l.tracking_min = v; }
			if let Some(v) = t.get("max").and_then(|v| length_pt(v, size)) { l.tracking_max = v; }
		}
	}
	Ok(l)
}

fn costs(styles: &StyleChain) -> Costs {
	let mut c = Costs::default();
	for v in text_values(styles, "costs") {
		if let Value::Dict(d) = v {
			let r = |key: &str| match d.get(key) {
				Some(Value::Ratio(r))	=> Some(r.0),
				_						=> None,
			};
			if let Some(x) = r("hyphenation") { c.hyphenation = x; }
			if let Some(x) = r("runt") { c.runt = x; }
			if let Some(x) = r("widow") { c.widow = x; }
			if let Some(x) = r("orphan") { c.orphan = x; }
		}
	}
	c
}

fn deco_stroke(v: Option<&Value>, size: f64) -> Outcome<Option<DecoStroke>> {
	let v = match v {
		None | Some(Value::Auto) | Some(Value::None)	=> return Ok(None),
		Some(v)											=> v,
	};
	let s = match crate::eval::styles::to_stroke(v) {
		Some(s)	=> s,
		None	=> return Ok(None),
	};
	let paint = match &s.paint {
		Some(Paint::Color(c))		=> Some(res!(c.to_rgba())),
		Some(Paint::Gradient(g))	=> match g.stops.first() {
			Some((c, _))	=> Some(res!(c.to_rgba())),
			None			=> None,
		},
		_							=> None,
	};
	Ok(Some(DecoStroke { paint, thickness: s.thickness.map(|t| t.resolve(size)) }))
}

fn decos(styles: &StyleChain, size: f64) -> Outcome<Vec<Deco>> {
	let mut out = Vec::new();
	for v in text_values(styles, "deco") {
		let items: Vec<Value> = match v {
			Value::Array(a)	=> (*a).clone(),
			other			=> vec![other],
		};
		for item in items {
			let d = match item {
				Value::Dict(d)	=> d,
				_				=> continue,
			};
			let flag = |k: &str, dflt: bool| match d.get(k) {
				Some(Value::Bool(b))	=> *b,
				_						=> dflt,
			};
			let offset = d.get("offset").and_then(|v| length_pt(v, size));
			let extent = d.get("extent").and_then(|v| length_pt(v, size)).unwrap_or(0.0);
			let line = match d.get("line") {
				Some(Value::Str(s)) => match s.as_str() {
					"underline"		=> DecoLine::Underline {
						stroke:		res!(deco_stroke(d.get("stroke"), size)),
						offset,
						evade:		flag("evade", true),
						background:	flag("background", false),
					},
					"overline"		=> DecoLine::Overline {
						stroke:		res!(deco_stroke(d.get("stroke"), size)),
						offset,
						evade:		flag("evade", true),
						background:	flag("background", false),
					},
					"strike"		=> DecoLine::Strikethrough {
						stroke:		res!(deco_stroke(d.get("stroke"), size)),
						offset,
						background:	flag("background", false),
					},
					"highlight"		=> DecoLine::Highlight {
						fill:	match d.get("fill") {
							Some(v)	=> res!(paint_rgba(v)),
							None	=> None,
						},
						top:	d.get("top-edge").map(|v| Edge::of(v, size, true)).unwrap_or(Edge::Ascender),
						bottom:	d.get("bottom-edge").map(|v| Edge::of(v, size, false)).unwrap_or(Edge::Descender),
					},
					_				=> continue,
				},
				_ => continue,
			};
			out.push(Deco { line, extent });
		}
	}
	Ok(out)
}

fn shift_settings(styles: &StyleChain) -> Outcome<Option<ShiftSettings>> {
	let d: Dict = match res!(text_field(styles, "shift-settings")) {
		Some(Value::Dict(d))	=> (*d).clone(),
		_						=> return Ok(None),
	};
	let em = |k: &str| match d.get(k) {
		Some(Value::Float(f))	=> Some(*f),
		Some(Value::Int(i))		=> Some(*i as f64),
		_						=> None,
	};
	Ok(Some(ShiftSettings {
		typographic:	!matches!(d.get("typographic"), Some(Value::Bool(false))),
		shift:			em("shift"),
		size:			em("size"),
		superscript:	matches!(d.get("kind"), Some(Value::Str(s)) if s.as_str() == "super"),
	}))
}

/// The language and the region the chain sets text in, `en` and none where nothing says otherwise. A
/// document's language is this of the chain its first page is set under, so a `set text(lang: ..)` at the
/// top of the document reaches it.
pub fn locale(styles: &StyleChain) -> Outcome<(String, Option<String>)> {
	let lang = match res!(text_field(styles, "lang")) {
		Some(Value::Str(s))	=> s.to_lowercase(),
		_					=> "en".to_string(),
	};
	let region = match res!(text_field(styles, "region")) {
		Some(Value::Str(s))	=> Some(s.to_uppercase()),
		_					=> None,
	};
	Ok((lang, region))
}

/// Resolves the chain's text properties once, for shaping and inline layout.
pub fn props(styles: &StyleChain) -> Outcome<TextProps> {
	let size = styles.font_size();
	let (families, fallback) = res!(families(styles));
	let (lang, region) = res!(locale(styles));
	let script = match res!(text_field(styles, "script")) {
		Some(Value::Str(s))	=> tag_of(&title_case(&s)),
		_					=> None,
	};
	let rtl = match res!(text_field(styles, "dir")) {
		Some(Value::Direction(d))	=> matches!(d, crate::eval::value::Direction::Rtl),
		_							=> lang_is_rtl(&lang),
	};
	let fill = match res!(text_field(styles, "fill")) {
		Some(v)	=> res!(paint_rgba(&v)).unwrap_or(Rgba::BLACK),
		None	=> Rgba::BLACK,
	};
	let top_edge = res!(text_field(styles, "top-edge")).map(|v| Edge::of(&v, size, true)).unwrap_or(Edge::CapHeight);
	let bottom_edge = res!(text_field(styles, "bottom-edge")).map(|v| Edge::of(&v, size, false)).unwrap_or(Edge::Baseline);
	let hyphenate = match res!(text_field(styles, "hyphenate")) {
		Some(Value::Bool(b))	=> Some(b),
		_						=> None,
	};
	let case = match res!(text_field(styles, "case")) {
		Some(Value::Str(s)) if s.as_str() == "upper"	=> Some(true),
		Some(Value::Str(s)) if s.as_str() == "lower"	=> Some(false),
		_												=> None,
	};
	let link = res!(elem_field(styles, ElemKind::Link, "current")).as_ref().and_then(link_target);
	let link_outset = match (&link, res!(elem_field(styles, ElemKind::Par, "leading"))) {
		(None, _)						=> 0.0,
		(Some(_), Some(Value::Length(l)))	=> 0.5 * l.resolve(size),
		(Some(_), _)					=> 0.5 * 0.65 * size,
	};
	Ok(TextProps {
		size,
		families,
		fallback,
		variant:			res!(variant(styles)),
		features:			res!(features(styles)),
		lang,
		region,
		script,
		rtl,
		tracking:			res!(text_field(styles, "tracking")).and_then(|v| length_pt(&v, size)).unwrap_or(0.0),
		spacing:			res!(text_field(styles, "spacing")).and_then(|v| relative_of(&v, size)).unwrap_or((1.0, 0.0)),
		fill,
		baseline:			res!(text_field(styles, "baseline")).and_then(|v| length_pt(&v, size)).unwrap_or(0.0),
		overhang:			!matches!(res!(text_field(styles, "overhang")), Some(Value::Bool(false))),
		top_edge,
		bottom_edge,
		hyphenate,
		costs:				costs(styles),
		decos:				res!(decos(styles, size)),
		shift:				res!(shift_settings(styles)),
		case,
		hidden:				res!(crate::eval::lib::visual::is_hidden(styles)),
		cjk_latin_spacing:	!matches!(res!(text_field(styles, "cjk-latin-spacing")), Some(Value::None)),
		limits:				res!(limits(styles, size)),
		link,
		link_outset,
	})
}

/// Where `link.current` points, as the IR carries a link: a URL, or a location through its anchor. A page
/// position has no IR form yet, so it gives none; inline layout warns of it.
pub fn link_target(v: &Value) -> Option<LinkTarget> {
	match v {
		Value::Str(url)			=> Some(LinkTarget::Uri(url.to_string())),
		Value::Location(loc)	=> Some(LinkTarget::Anchor(loc.anchor())),
		_						=> None,
	}
}

fn title_case(s: &str) -> String {
	let mut out = String::new();
	for (i, c) in s.chars().enumerate() {
		if i == 0 {
			out.extend(c.to_uppercase());
		} else {
			out.extend(c.to_lowercase());
		}
	}
	out
}

impl TextProps {
	/// The BCP 47 language the shaper is told, the region appended when set.
	pub fn bcp47(&self) -> String {
		match &self.region {
			Some(r)	=> fmt!("{}-{}", self.lang, r),
			None	=> self.lang.clone(),
		}
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ SHAPED TEXT                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// How a CJK punctuation mark is set, by the standard of the text's region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CjkPunctStyle {
	Gb,		// mainland China
	Cns,	// Taiwan and Hong Kong
	Jis,	// Japan
}

pub fn cjk_punct_style(lang: &str, region: Option<&str>) -> CjkPunctStyle {
	match (lang, region) {
		("zh", Some("TW")) | ("zh", Some("HK"))	=> CjkPunctStyle::Cns,
		("ja", _)								=> CjkPunctStyle::Jis,
		_										=> CjkPunctStyle::Gb,
	}
}

/// One shaped glyph, its metrics in ems of `size`, as Typst's `ShapedGlyph`.
#[derive(Clone, Debug)]
pub struct SGlyph {
	pub face:			usize,			// the font book's face id
	pub id:				u32,
	pub x_advance:		f64,
	pub x_offset:		f64,
	pub y_offset:		f64,
	pub size:			f64,			// points
	pub stretch:		(f64, f64),		// left and right, ems
	pub shrink:			(f64, f64),
	pub range:			(usize, usize),	// the cluster's bytes in the paragraph's text
	pub c:				char,			// the cluster's first character
	pub justifiable:	bool,
	pub script:			Script,
}

impl SGlyph {
	pub fn is_space(&self) -> bool { is_space(self.c) }

	pub fn is_cj_script(&self) -> bool { is_cj_script(self.c, self.script) }

	pub fn is_cjk_left_aligned_punctuation(&self, style: CjkPunctStyle) -> bool {
		is_cjk_left_aligned_punctuation(self.c, self.x_advance, self.stretch, style)
	}

	pub fn is_cjk_right_aligned_punctuation(&self) -> bool {
		is_cjk_right_aligned_punctuation(self.c, self.x_advance, self.stretch)
	}

	pub fn is_cjk_center_aligned_punctuation(&self, style: CjkPunctStyle) -> bool {
		is_cjk_center_aligned_punctuation(self.c, style)
	}

	pub fn is_cjk_punctuation(&self) -> bool {
		self.is_cjk_left_aligned_punctuation(CjkPunctStyle::Gb)
			|| self.is_cjk_right_aligned_punctuation()
			|| self.is_cjk_center_aligned_punctuation(CjkPunctStyle::Gb)
	}

	/// Is it a western letter or number, for CJK-Latin spacing?
	pub fn is_letter_or_number(&self) -> bool {
		matches!(Script::of(self.c), Script::Latin | Script::Greek | Script::Cyrillic)
			|| matches!(self.c, '#' | '$' | '%' | '&')
			|| self.c.is_ascii_digit()
	}

	pub fn shrink_left(&mut self, amount: f64) {
		self.x_offset -= amount;
		self.x_advance -= amount;
		self.shrink.0 -= amount;
	}

	pub fn shrink_right(&mut self, amount: f64) {
		self.x_advance -= amount;
		self.shrink.1 -= amount;
	}

	/// The stretch and shrink justification may give the glyph (Typst's `base_adjustability`): a space
	/// takes the spacing and tracking limits, CJK punctuation may give up half or a quarter of itself,
	/// and any other glyph that ends its cluster the tracking limits.
	fn base_adjustability(&self, style: CjkPunctStyle, limits: &Limits, font_size: f64, stretchable: bool)
		-> ((f64, f64), (f64, f64))
	{
		let width = self.x_advance;
		let limited = |v: f64| v.min(width * 0.75);
		let em = |pt: f64| if font_size > 0.0 { pt / font_size } else { 0.0 };
		if self.is_space() {
			let max = (limits.spacing_max.0, limits.spacing_max.1 + limits.tracking_max);
			let min = (limits.spacing_min.0, limits.spacing_min.1 + limits.tracking_min);
			let stretch = ((max.0 - 1.0) * width + em(max.1)).max(0.0);
			let shrink = limited((1.0 - min.0) * width + em(-min.1));
			((0.0, stretch), (0.0, shrink))
		} else if self.is_cjk_left_aligned_punctuation(style) {
			((0.0, 0.0), (0.0, width / 2.0))
		} else if self.is_cjk_right_aligned_punctuation() {
			((0.0, 0.0), (width / 2.0, 0.0))
		} else if self.is_cjk_center_aligned_punctuation(style) {
			((0.0, 0.0), (width / 4.0, width / 4.0))
		} else if stretchable {
			((0.0, em(limits.tracking_max).max(0.0)), (0.0, limited(em(-limits.tracking_min))))
		} else {
			((0.0, 0.0), (0.0, 0.0))
		}
	}
}

/// A run of shaped text with one style and direction: Typst's `ShapedText`. Glyphs outside `kept` are
/// end-of-line white space trimmed from layout but still drawn, with no advance.
#[derive(Clone, Debug)]
pub struct ShapedRun {
	pub base:		usize,				// where the text starts in the paragraph's text
	pub text:		Arc<str>,
	pub rtl:		bool,
	pub props:		Arc<TextProps>,
	pub glyphs:		Arc<Vec<SGlyph>>,
	pub kept:		(usize, usize),
}

impl ShapedRun {
	/// The glyphs layout sees.
	pub fn kept(&self) -> &[SGlyph] {
		self.glyphs.get(self.kept.0..self.kept.1).unwrap_or(&[])
	}

	pub fn kept_mut(&mut self) -> &mut [SGlyph] {
		let (a, b) = self.kept;
		match Arc::make_mut(&mut self.glyphs).get_mut(a..b) {
			Some(s)	=> s,
			None	=> &mut [],
		}
	}

	pub fn end(&self) -> usize { self.base + self.text.len() }

	/// The natural width, in points.
	pub fn width(&self) -> f64 {
		self.kept().iter().map(|g| g.x_advance * g.size).sum()
	}

	pub fn stretchability(&self) -> f64 {
		self.kept().iter().map(|g| (g.stretch.0 + g.stretch.1) * g.size).sum()
	}

	pub fn shrinkability(&self) -> f64 {
		self.kept().iter().map(|g| (g.shrink.0 + g.shrink.1) * g.size).sum()
	}

	pub fn justifiables(&self) -> usize {
		self.kept().iter().filter(|g| g.justifiable).count()
	}

	/// Is the last glyph CJK, which justification leaves alone at a line's end?
	pub fn cjk_justifiable_at_last(&self) -> bool {
		self.kept().last().map_or(false, |g| g.is_cj_script() || g.is_cjk_punctuation())
	}

	/// Trims glyphs matching `f` from both ends of the kept range.
	pub fn trim<F: Fn(&SGlyph) -> bool>(&mut self, f: F) {
		let all = self.glyphs.as_slice();
		let mut a = 0;
		while a < all.len() && f(&all[a]) {
			a += 1;
		}
		let mut b = all.len();
		while b > a && f(&all[b - 1]) {
			b -= 1;
		}
		self.kept = (a, b);
	}

	/// The same properties with no text.
	pub fn empty(&self) -> Self {
		Self { text: Arc::from(""), glyphs: Arc::new(Vec::new()), kept: (0, 0), ..self.clone() }
	}

	/// The run's top and bottom extent: the chain's edges measured over each kept glyph's face, or over
	/// the first face the families give when the run has no glyphs at all.
	pub fn measure(&self, book: &FontBook) -> (f64, f64) {
		let size = self.props.size;
		let mut top = 0.0f64;
		let mut bottom = 0.0f64;
		if self.glyphs.is_empty() {
			let face = self.props.families.iter().find_map(|f| book.select(&f.name, self.props.variant));
			if let Some(f) = face.and_then(|i| book.face(i)) {
				let (t, b) = edges(f, self.props.top_edge, self.props.bottom_edge, size, None);
				top = top.max(t);
				bottom = bottom.max(b);
			}
		} else {
			for g in self.kept() {
				if let Some(f) = book.face(g.face) {
					let (t, b) = edges(f, self.props.top_edge, self.props.bottom_edge, size, Some(g.id));
					top = top.max(t);
					bottom = bottom.max(b);
				}
			}
		}
		(top, bottom)
	}
}

/// A face's top and bottom edges at a size, in points, positive away from the baseline.
pub fn edges(face: &BookFace, top: Edge, bottom: Edge, size: f64, glyph: Option<u32>) -> (f64, f64) {
	let m = &face.metrics;
	let em = |u: f32| face.to_em(u) * size;
	let bbox = || glyph.and_then(|g| glyph_bounds(face, g));
	let t = match top {
		Edge::Ascender	=> em(m.ascender),
		Edge::CapHeight	=> em(m.cap_height.unwrap_or(m.ascender)),
		Edge::XHeight	=> em(m.x_height.unwrap_or(m.ascender)),
		Edge::Baseline	=> 0.0,
		Edge::Descender	=> em(m.descender),
		Edge::Bounds	=> bbox().map(|b| em(b.y1)).unwrap_or(0.0),
		Edge::Length(l)	=> l,
	};
	let b = match bottom {
		Edge::Ascender	=> -em(m.ascender),
		Edge::CapHeight	=> -em(m.cap_height.unwrap_or(m.ascender)),
		Edge::XHeight	=> -em(m.x_height.unwrap_or(m.ascender)),
		Edge::Baseline	=> 0.0,
		Edge::Descender	=> -em(m.descender),
		Edge::Bounds	=> bbox().map(|b| -em(b.y0)).unwrap_or(0.0),
		Edge::Length(l)	=> -l,
	};
	(t, b)
}

/// A glyph's outline bounds in font units, y up.
pub fn glyph_bounds(face: &BookFace, id: u32) -> Option<Bounds> {
	let upem = face.metrics.units_per_em;
	let path = face.font.outline(0, id, upem).ok()?;
	path.bounds(&Transform::IDENTITY)
}

/// Is it a space for justification: an ordinary, a no-break or an ideographic space?
pub fn is_space(c: char) -> bool {
	matches!(c, ' ' | '\u{00A0}' | '\u{3000}')
}

/// Is the character Chinese or Japanese script (not Korean), which sets without spaces?
pub fn is_cj_script(c: char, script: Script) -> bool {
	matches!(script, Script::Hiragana | Script::Katakana | Script::Han) || c == '\u{30FC}'
}

pub fn is_of_cj_script(c: char) -> bool {
	is_cj_script(c, Script::of(c))
}

fn is_cjk_left_aligned_punctuation(c: char, x_advance: f64, stretch: (f64, f64), style: CjkPunctStyle) -> bool {
	if matches!(c, '\u{201D}' | '\u{2019}') && x_advance + stretch.1 == 1.0 {
		return true;
	}
	if matches!(style, CjkPunctStyle::Gb | CjkPunctStyle::Jis)
		&& matches!(c, '\u{FF0C}' | '\u{3002}' | '\u{FF0E}' | '\u{3001}' | '\u{FF1A}' | '\u{FF1B}')
	{
		return true;
	}
	if matches!(style, CjkPunctStyle::Gb) && matches!(c, '\u{FF1F}' | '\u{FF01}') {
		return true;
	}
	matches!(c, '\u{300B}' | '\u{FF09}' | '\u{300F}' | '\u{300D}' | '\u{3011}' | '\u{3017}' | '\u{3015}'
		| '\u{3009}' | '\u{FF3D}' | '\u{FF5D}')
}

fn is_cjk_right_aligned_punctuation(c: char, x_advance: f64, stretch: (f64, f64)) -> bool {
	if matches!(c, '\u{201C}' | '\u{2018}') && x_advance + stretch.0 == 1.0 {
		return true;
	}
	matches!(c, '\u{300A}' | '\u{FF08}' | '\u{300E}' | '\u{300C}' | '\u{3010}' | '\u{3016}' | '\u{3014}'
		| '\u{3008}' | '\u{FF3B}' | '\u{FF5B}')
}

fn is_cjk_center_aligned_punctuation(c: char, style: CjkPunctStyle) -> bool {
	if matches!(style, CjkPunctStyle::Cns)
		&& matches!(c, '\u{FF0C}' | '\u{3002}' | '\u{FF0E}' | '\u{3001}' | '\u{FF1A}' | '\u{FF1B}')
	{
		return true;
	}
	matches!(c, '\u{30FB}' | '\u{00B7}')
}

fn is_justifiable(c: char, script: Script, x_advance: f64, stretch: (f64, f64)) -> bool {
	let style = CjkPunctStyle::Gb;
	is_space(c)
		|| is_cj_script(c, script)
		|| is_cjk_left_aligned_punctuation(c, x_advance, stretch, style)
		|| is_cjk_right_aligned_punctuation(c, x_advance, stretch)
		|| is_cjk_center_aligned_punctuation(c, style)
}

/// The CJK punctuation a line may begin with.
pub const BEGIN_PUNCT: &[char] = &['\u{201C}', '\u{2018}', '\u{300A}', '\u{3008}', '\u{FF08}', '\u{300E}',
	'\u{300C}', '\u{3010}', '\u{3016}', '\u{3014}', '\u{FF3B}', '\u{FF5B}'];

/// The CJK punctuation a line may end with.
pub const END_PUNCT: &[char] = &['\u{201D}', '\u{2019}', '\u{FF0C}', '\u{FF0E}', '\u{3002}', '\u{3001}',
	'\u{FF1A}', '\u{FF1B}', '\u{300B}', '\u{3009}', '\u{FF09}', '\u{300F}', '\u{300D}', '\u{3011}',
	'\u{3017}', '\u{3015}', '\u{FF3D}', '\u{FF5D}', '\u{FF1F}', '\u{FF01}'];

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ SHAPING                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

struct Ctx<'a> {
	book:		&'a FontBook,
	props:		&'a TextProps,
	glyphs:		Vec<SGlyph>,
	used:		Vec<usize>,	// faces exhausted, which fallback may not choose again
	rtl:		bool,
	lang:		String,
}

/// Shapes `text`, which starts at byte `base` of the paragraph's text, under `props`.
pub fn shape_run(book: &FontBook, base: usize, text: &str, props: &Arc<TextProps>, rtl: bool) -> Outcome<ShapedRun> {
	let mut ctx = Ctx {
		book,
		props:	props.as_ref(),
		glyphs:	Vec::new(),
		used:	Vec::new(),
		rtl,
		lang:	props.bcp47(),
	};
	if !text.is_empty() {
		res!(shape_segment(&mut ctx, base, text, 0));
	}
	track_and_space(&mut ctx);
	calculate_adjustability(&mut ctx);
	let n = ctx.glyphs.len();
	Ok(ShapedRun {
		base,
		text:	Arc::from(text),
		rtl,
		props:	props.clone(),
		glyphs:	Arc::new(ctx.glyphs),
		kept:	(0, n),
	})
}

/// The first family from `from` with a face for the variant that is not yet exhausted, then Typst's
/// fallback search; the face, its coverage and the index of the family after it.
fn font_and_covers(ctx: &mut Ctx, text: &str, from: usize)
	-> (Option<(usize, Option<Arc<Regex>>)>, usize)
{
	let mut next = from;
	let mut selection = None;
	while next < ctx.props.families.len() {
		let fam = &ctx.props.families[next];
		next += 1;
		if let Some(id) = ctx.book.select(&fam.name, ctx.props.variant) {
			if !ctx.used.contains(&id) {
				selection = Some((id, fam.covers.clone()));
				break;
			}
		}
	}
	if selection.is_none() && ctx.props.fallback {
		let first = ctx.used.first().copied();
		if let Some(id) = ctx.book.select_fallback(first, ctx.props.variant, text) {
			if !ctx.used.contains(&id) {
				selection = Some((id, None));
			}
		}
	}
	(selection, next)
}

/// Shapes a segment with the next family that has a face, re-shaping whatever that face cannot draw
/// with the families after it; with no face left, the first face used draws not-defined glyphs.
fn shape_segment(ctx: &mut Ctx, base: usize, text: &str, from: usize) -> Outcome<()> {
	if text.chars().all(|c| c == '\n' || c == '\t' || is_default_ignorable(c)) {
		return Ok(());
	}
	let (sel, next) = font_and_covers(ctx, text, from);
	let (fid, covers) = match sel {
		Some(s)	=> s,
		None	=> {
			if let Some(&first) = ctx.used.first() {
				res!(shape_tofus(ctx, base, text, first));
			}
			return Ok(());
		},
	};
	if covers.is_none() {
		ctx.used.push(fid);
	}
	let bf = match ctx.book.face(fid) {
		Some(f)	=> f,
		None	=> return Err(err!("Face {} is not in the font book.", fid; Bug)),
	};
	let upem = bf.metrics.units_per_em;

	// Scripts: the font's own sub- and superscript forms when asked for and present, else a synthesised
	// shift, compensation and scale from the face's metrics.
	let mut features = ctx.props.features.clone();
	let (script_shift, script_comp, scale) = match ctx.props.shift {
		Some(s) => {
			let (shift, comp, scale, feat) = res!(determine_shift(bf, text, s, &ctx.props.features, ctx.rtl));
			if let Some(f) = feat {
				features.push(f);
			}
			(shift, comp, scale)
		},
		None => (0.0, 0.0, 1.0),
	};

	let spec = ShapeSpec {
		features:			&features,
		language:			Some(&ctx.lang),
		script:				ctx.props.script,
		remove_ignorables:	true,
	};
	let dir = if ctx.rtl { ShapeDir::Rtl } else { ShapeDir::Ltr };
	// Shaped at one pixel per font unit, so advances and offsets come back in font units exactly, and
	// from the book's cache when the same call has been made before.
	let shaped = res!(ctx.book.shape(fid, text, dir, &spec));
	let glyphs = shaped.as_slice();
	let to_em = |u: f32| u as f64 / upem as f64;

	let is_covered = |offset: usize| -> bool {
		match &covers {
			None		=> true,
			Some(re)	=> {
				let end = text.get(offset..).and_then(|t| t.char_indices().nth(1).map(|(i, _)| offset + i))
					.unwrap_or(text.len());
				text.get(offset..end).map_or(false, |s| matches!(re.is_match(s), Ok(true)))
			},
		}
	};

	let mut pens = Vec::with_capacity(glyphs.len());
	let mut pen = 0.0f32;
	for g in glyphs.iter() {
		pens.push(pen);
		pen += g.adv;
	}

	let mut i = 0;
	while i < glyphs.len() {
		let g: RunGlyph = glyphs[i];
		let cluster = g.cluster;
		if g.id != 0 && is_covered(cluster) {
			let start = base + cluster;
			// The cluster's end: the next glyph in logical order with another cluster, else the text's end.
			let mut k = i;
			let end = loop {
				let nk = if ctx.rtl { k.checked_sub(1) } else { Some(k + 1) };
				match nk.and_then(|n| glyphs.get(n).map(|gi| (n, gi))) {
					None				=> break base + text.len(),
					Some((n, gi))		=> {
						if gi.cluster != cluster {
							break base + gi.cluster;
						}
						k = n;
					}
				}
			};
			let c = text.get(cluster..).and_then(|t| t.chars().next()).unwrap_or(' ');
			let script = Script::of(c);
			let x_advance = to_em(g.adv);
			ctx.glyphs.push(SGlyph {
				face:			fid,
				id:				g.id,
				x_advance,
				x_offset:		to_em(g.x - pens[i]) + script_comp,
				y_offset:		to_em(g.y) + script_shift,
				size:			scale * ctx.props.size,
				stretch:		(0.0, 0.0),
				shrink:			(0.0, 0.0),
				range:			(start, end),
				c,
				justifiable:	is_justifiable(c, script, x_advance, (0.0, 0.0)),
				script,
			});
		} else {
			// The end of the not-drawn stretch, then its text range.
			let k = i;
			while glyphs.get(i + 1).map_or(false, |gi| gi.id == 0 || !is_covered(gi.cluster)) {
				i += 1;
			}
			let start = glyphs[if ctx.rtl { i } else { k }].cluster;
			let end_idx = if ctx.rtl { k.checked_sub(1) } else { Some(i + 1) };
			let end = end_idx.and_then(|e| glyphs.get(e)).map_or(text.len(), |gi| gi.cluster);
			let (lo, hi) = (base + start, base + end);
			while ctx.glyphs.last().map_or(false, |gl| gl.range.0 >= lo && gl.range.0 < hi) {
				ctx.glyphs.pop();
			}
			if let Some(sub) = text.get(start..end) {
				res!(shape_segment(ctx, base + start, sub, next));
			}
		}
		i += 1;
	}
	ctx.used.pop();
	Ok(())
}

/// The shift, compensation and scale (ems) that make a sub- or superscript, and the feature that gives
/// the font's own forms when it has them for every character.
fn determine_shift(face: &BookFace, text: &str, s: ShiftSettings, base: &[Feature], rtl: bool)
	-> Outcome<(f64, f64, f64, Option<Feature>)>
{
	let tag = if s.superscript { *b"sups" } else { *b"subs" };
	if s.typographic {
		// The font's forms serve when the feature substitutes every character: shaped with and without
		// it, each glyph changes and none is missing.
		let f = res!(face.face());
		let upem = face.metrics.units_per_em;
		let dir = if rtl { ShapeDir::Rtl } else { ShapeDir::Ltr };
		let plain = res!(f.shape_spec(text, upem, dir, 0, 0, &ShapeSpec { features: base, ..ShapeSpec::default() }));
		let mut with: Vec<Feature> = base.to_vec();
		with.push(Feature::set(&tag, 1));
		let feat = res!(f.shape_spec(text, upem, dir, 0, 0, &ShapeSpec { features: &with, ..ShapeSpec::default() }));
		let all = !text.is_empty()
			&& text.chars().all(|c| f.glyph_index(c).is_some())
			&& plain.glyphs.len() == feat.glyphs.len()
			&& plain.glyphs.iter().zip(feat.glyphs.iter()).all(|(a, b)| a.id != b.id);
		if all {
			return Ok((0.0, 0.0, 1.0, Some(Feature::set(&tag, 1))));
		}
	}
	let (height, hoff, voff) = script_metrics(Some(face), s.superscript);
	Ok((s.shift.unwrap_or(voff), hoff, s.size.unwrap_or(height), None))
}

/// A face's advice for a synthesised script, in ems: its height, and its offsets right and up; Typst's
/// defaults where the face gives none.
fn script_metrics(face: Option<&BookFace>, superscript: bool) -> (f64, f64, f64) {
	let m = face.map(|f| (f, if superscript { f.metrics.superscript } else { f.metrics.subscript }));
	match (superscript, m) {
		(true, Some((f, Some(sm))))		=> (f.to_em(sm.y_size), f.to_em(sm.x_offset), f.to_em(sm.y_offset)),
		(false, Some((f, Some(sm))))	=> (f.to_em(sm.y_size), f.to_em(sm.x_offset), -f.to_em(sm.y_offset)),
		(true, _)						=> (0.6, 0.0, 0.5),
		(false, _)						=> (0.6, 0.0, -0.2),
	}
}

/// How far an inline frame's contents move under the text's baseline shift, right and down, in points:
/// Typst's `apply_shift`, `text.baseline` and a script's offset from the first face of the families.
pub fn frame_shift(book: &FontBook, props: &TextProps) -> (f64, f64) {
	let mut right = 0.0;
	let mut down = props.baseline;
	if let Some(s) = props.shift {
		let face = props.families.iter().find_map(|f| book.select(&f.name, props.variant)).and_then(|i| book.face(i));
		let (_, hoff, voff) = script_metrics(face, s.superscript);
		down -= s.shift.unwrap_or(voff) * props.size;
		right += hoff * props.size;
	}
	(right, down)
}

/// Not-defined glyphs from face `fid`, one per character.
fn shape_tofus(ctx: &mut Ctx, base: usize, text: &str, fid: usize) -> Outcome<()> {
	let bf = match ctx.book.face(fid) {
		Some(f)	=> f,
		None	=> return Ok(()),
	};
	let adv = res!(bf.face()).advance_units(0).map(|u| bf.to_em(u)).unwrap_or(0.0);
	let mut push = |cluster: usize, c: char| {
		let script = Script::of(c);
		ctx.glyphs.push(SGlyph {
			face:			fid,
			id:				0,
			x_advance:		adv,
			x_offset:		0.0,
			y_offset:		0.0,
			size:			ctx.props.size,
			stretch:		(0.0, 0.0),
			shrink:			(0.0, 0.0),
			range:			(base + cluster, base + cluster + c.len_utf8()),
			c,
			justifiable:	is_justifiable(c, script, adv, (0.0, 0.0)),
			script,
		});
	};
	if ctx.rtl {
		for (i, c) in text.char_indices().rev() {
			push(i, c);
		}
	} else {
		for (i, c) in text.char_indices() {
			push(i, c);
		}
	}
	Ok(())
}

/// Applies `tracking` between clusters and `spacing` to spaces; a no-break space takes the width of an
/// ordinary one.
fn track_and_space(ctx: &mut Ctx) {
	let size = ctx.props.size;
	let tracking = if size > 0.0 { ctx.props.tracking / size } else { 0.0 };
	let (ratio, abs) = ctx.props.spacing;
	let abs_em = if size > 0.0 { abs / size } else { 0.0 };
	let n = ctx.glyphs.len();
	for i in 0..n {
		if ctx.glyphs[i].c == '\u{00A0}' {
			let delta = nbsp_delta(ctx.book, ctx.glyphs[i].face).unwrap_or(0.0);
			ctx.glyphs[i].x_advance -= delta;
		}
		if ctx.glyphs[i].is_space() {
			ctx.glyphs[i].x_advance = ratio * ctx.glyphs[i].x_advance + abs_em;
		}
		if i + 1 < n && ctx.glyphs[i].range.0 != ctx.glyphs[i + 1].range.0 {
			ctx.glyphs[i].x_advance += tracking;
		}
	}
}

/// A face's no-break space advance less its space advance, in ems.
fn nbsp_delta(book: &FontBook, fid: usize) -> Option<f64> {
	let bf = book.face(fid)?;
	let f = bf.face().ok()?;
	let space = f.glyph_index(' ')?;
	let nbsp = f.glyph_index('\u{00A0}')?;
	Some(bf.to_em(f.advance_units(nbsp)?) - bf.to_em(f.advance_units(space)?))
}

/// Gives each glyph its stretch and shrink, then compresses consecutive CJK punctuation.
fn calculate_adjustability(ctx: &mut Ctx) {
	let style = cjk_punct_style(&ctx.props.lang, ctx.props.region.as_deref());
	let limits = ctx.props.limits;
	let size = ctx.props.size;
	let n = ctx.glyphs.len();
	for i in 0..n {
		let stretchable = i + 1 >= n || ctx.glyphs[i].range.0 != ctx.glyphs[i + 1].range.0;
		let (st, sh) = ctx.glyphs[i].base_adjustability(style, &limits, size, stretchable);
		ctx.glyphs[i].stretch = st;
		ctx.glyphs[i].shrink = sh;
	}
	for i in 0..n {
		if ctx.glyphs[i].is_cjk_punctuation() && style == CjkPunctStyle::Cns {
			continue;
		}
		if i + 1 >= n {
			continue;
		}
		let width = ctx.glyphs[i].x_advance;
		let delta = width / 2.0;
		if ctx.glyphs[i].is_cjk_punctuation()
			&& ctx.glyphs[i + 1].is_cjk_punctuation()
			&& ctx.glyphs[i].shrink.1 + ctx.glyphs[i + 1].shrink.0 >= delta
		{
			let left = ctx.glyphs[i].shrink.1.min(delta);
			ctx.glyphs[i].shrink_right(left);
			ctx.glyphs[i + 1].shrink_left(delta - left);
		}
	}
}

impl ShapedRun {
	/// The slice `start..end` of the run, shaped afresh: a line boundary cutting a run is where kerning
	/// and ligatures may change, and shaping the slice gives exactly what Typst's safe-to-break reuse
	/// or reshape gives.
	pub fn reshape(&self, book: &FontBook, start: usize, end: usize) -> Outcome<ShapedRun> {
		let text = match self.text.get(start - self.base..end - self.base) {
			Some(t)	=> t,
			None	=> return Err(err!("A line cuts a text run off a character boundary at {}..{}.", start, end; Bug)),
		};
		shape_run(book, start, text, &self.props, self.rtl)
	}

	/// A hyphen in the run's style, from the first family whose face has one: a soft hyphen's glyph when
	/// `soft` (it copies as the soft hyphen it stands for), else an ordinary hyphen.
	pub fn hyphen(&self, book: &FontBook, pos: usize, soft: bool) -> Option<ShapedRun> {
		let props = &self.props;
		let mut ids: Vec<usize> = props.families.iter()
			.filter(|f| f.covers.as_ref().map_or(true, |c| matches!(c.is_match("-"), Ok(true))))
			.filter_map(|f| book.select(&f.name, props.variant))
			.collect();
		if props.fallback {
			if let Some(id) = book.select_fallback(None, props.variant, "-") {
				ids.push(id);
			}
		}
		for id in ids {
			let bf = book.face(id)?;
			let f = bf.face().ok()?;
			let gid = match f.glyph_index('-') {
				Some(g)	=> g,
				None	=> continue,
			};
			let adv = match f.advance_units(gid) {
				Some(a)	=> bf.to_em(a),
				None	=> continue,
			};
			let text: &str = if soft { "\u{00AD}" } else { "-" };
			let g = SGlyph {
				face:			id,
				id:				gid,
				x_advance:		adv,
				x_offset:		0.0,
				y_offset:		0.0,
				size:			props.size,
				stretch:		(0.0, 0.0),
				shrink:			(0.0, 0.0),
				range:			(pos, pos + text.len()),
				c:				if soft { '\u{00AD}' } else { '-' },
				justifiable:	false,
				script:			Script::Common,
			};
			return Some(ShapedRun {
				base:	pos,
				text:	Arc::from(text),
				rtl:	self.rtl,
				props:	props.clone(),
				glyphs:	Arc::new(vec![g]),
				kept:	(0, 1),
			});
		}
		None
	}

	/// The run as drawn nodes for a line, each seated on its baseline: a text leaf per group of glyphs
	/// sharing a face, offset and size, justified by `ratio` (of each glyph's stretch or shrink) and
	/// `extra` points on each justifiable glyph, decorated as the style asks. Returns the nodes and the
	/// advance they take, in points.
	pub fn build(&self, book: &FontBook, ratio: f64, extra: f64) -> Outcome<(Vec<Node>, f64)> {
		let props = &self.props;
		let size = props.size;
		let (top, bottom) = self.measure(book);
		let mut out: Vec<Node> = Vec::new();
		let mut offset = 0.0f64;
		let all = self.glyphs.as_slice();
		let mut i = 0;
		while i < all.len() {
			let key = (all[i].face, all[i].y_offset, all[i].size);
			let mut j = i;
			while j < all.len() && (all[j].face, all[j].y_offset, all[j].size) == key {
				j += 1;
			}
			let group = &all[i..j];
			let lo = group.iter().map(|g| g.range.0).min().unwrap_or(self.base);
			let hi = group.iter().map(|g| g.range.1).max().unwrap_or(self.base);
			let gsize = key.2;
			let mut run_glyphs: Vec<RunGlyph> = Vec::with_capacity(group.len());
			let mut pen = 0.0f64;
			for (gi, g) in group.iter().enumerate() {
				let kept = (i + gi) >= self.kept.0 && (i + gi) < self.kept.1;
				let (adv, xoff) = if kept {
					let left = if ratio < 0.0 { g.shrink.0 } else { g.stretch.0 };
					let right = if ratio < 0.0 { g.shrink.1 } else { g.stretch.1 };
					let jl = left * ratio;
					let mut jr = right * ratio;
					if g.justifiable && gsize > 0.0 {
						jr += extra / gsize;
					}
					(g.x_advance + jl + jr, g.x_offset + jl)
				} else {
					(0.0, 0.0)
				};
				run_glyphs.push(RunGlyph {
					id:			g.id,
					face:		0,
					x:			((pen + xoff) * gsize) as f32,
					y:			0.0,
					adv:		(adv * gsize) as f32,
					cluster:	g.range.0.saturating_sub(lo),
				});
				pen += adv;
			}
			let width = pen * gsize;
			let bf = match book.face(key.0) {
				Some(f)	=> f,
				None	=> return Err(err!("Face {} is not in the font book.", key.0; Bug)),
			};
			let text = self.text.get(lo.saturating_sub(self.base)..hi.saturating_sub(self.base)).unwrap_or("");
			// The group's baseline: `baseline` lowers (a positive `text.baseline` moves text down), the
			// glyph offset raises.
			let shift = props.baseline - key.1 * size;
			if props.hidden {
				out.push(Node::Glue(crate::ir::Glue::fixed(Sp::from_pt(width))));
			} else {
				let shaped = ShapedText::from_glyphs(
					bf.font.clone(), gsize as f32, run_glyphs, text.to_string(), props.fill,
					Dims::new(Sp::from_pt(width), Sp::from_pt(top), Sp::from_pt(bottom)),
				);
				// Decorations are zero-size graphics at the group's start on the baseline, those under the
				// text first.
				let mut front: Vec<Node> = Vec::new();
				for d in &props.decos {
					let (nodes, background) = res!(decorate(d, bf, group, props, width, shift));
					if background { out.extend(nodes); } else { front.extend(nodes); }
				}
				out.extend(front);
				let dims = shaped.dims();
				out.push(Node::Leaf(Leaf::text_dims(shaped, dims).with_shift(Sp::from_pt(shift))));
			}
			offset += width;
			i = j;
		}
		Ok((out, offset))
	}
}

/// A decoration for one group of glyphs `width` points wide whose baseline is lowered by `shift`:
/// zero-size graphics at the group's start whose origin is the line's baseline, drawn under the text
/// when `background`.
fn decorate(
	deco:	&Deco,
	face:	&BookFace,
	group:	&[SGlyph],
	props:	&TextProps,
	width:	f64,
	shift:	f64,
)
	-> Outcome<(Vec<Node>, bool)>
{
	let size = group.first().map_or(props.size, |g| g.size);
	let m = &face.metrics;
	let em = |u: f32| face.to_em(u) * size;
	let mut ops: Vec<DrawOp> = Vec::new();
	let x0 = -deco.extent;
	let background;
	match &deco.line {
		DecoLine::Highlight { fill, top: te, bottom: be } => {
			let mut t = 0.0f64;
			let mut b = 0.0f64;
			for g in group {
				let (tt, bb) = edges(face, *te, *be, size, Some(g.id));
				t = t.max(tt);
				b = b.max(bb);
			}
			if let Some(fill) = fill {
				let r = Bounds::new(x0 as f32, (shift - t) as f32, (width + deco.extent) as f32, (shift + b) as f32);
				ops.push(DrawOp::Fill { path: res!(Path::rect(r)), colour: *fill });
			}
			background = true;
		}
		line => {
			let (stroke, pos_thick, off, evade, bg) = match line {
				DecoLine::Strikethrough { stroke, offset, background } => (
					stroke, m.strikeout.map(|(p, t)| (em(p), em(t))).unwrap_or((0.25 * size, 0.06 * size)), offset, false, *background),
				DecoLine::Overline { stroke, offset, evade, background } => {
					let und = m.underline.or(m.strikeout).map(|(_, t)| em(t)).unwrap_or(0.06 * size);
					(stroke, (em(m.cap_height.unwrap_or(m.ascender)) + 0.1 * size, und), offset, *evade, *background)
				}
				DecoLine::Underline { stroke, offset, evade, background } => (
					stroke, m.underline.map(|(p, t)| (em(p), em(t))).unwrap_or((-0.2 * size, 0.06 * size)), offset, *evade, *background),
				DecoLine::Highlight { .. } => return Ok((Vec::new(), false)),
			};
			background = bg;
			let y = off.unwrap_or(-pos_thick.0) + shift;
			let colour = stroke.and_then(|s| s.paint).unwrap_or(props.fill);
			let thick = stroke.and_then(|s| s.thickness).unwrap_or(pos_thick.1);
			let gap = 0.08 * size;
			let min_width = 0.162 * size;
			let start = x0;
			let end = width + deco.extent;
			let mut cuts: Vec<f64> = Vec::new();
			if evade {
				let mut x = 0.0f64;
				for g in group {
					let dx = g.x_offset * size + x;
					x += g.x_advance * size;
					if let Ok(path) = face.font.outline(0, g.id, size as f32) {
						let band = y - shift;	// the line's height against the glyph's baseline, down
						if let Some(bb) = path.bounds(&Transform::IDENTITY) {
							if !(-band >= bb.y0 as f64 && -band <= bb.y1 as f64) {
								continue;
							}
						}
						for poly in path.flatten(&Transform::IDENTITY, 0.01) {
							for w in poly.windows(2) {
								let (p, q) = (w[0], w[1]);
								let (py, qy) = (-(p.y as f64), -(q.y as f64));
								if (py - band) * (qy - band) <= 0.0 && py != qy {
									let t = (band - py) / (qy - py);
									cuts.push(dx + p.x as f64 + t * (q.x as f64 - p.x as f64));
								}
							}
						}
					}
				}
			}
			let mut segs: Vec<(f64, f64)> = Vec::new();
			if !evade {
				segs.push((start, end));
			} else {
				cuts.push(start - gap);
				cuts.push(end + gap);
				cuts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
				for w in cuts.windows(2) {
					let (l, r) = (w[0], w[1]);
					if r - l < gap {
						continue;
					}
					let (a, b) = (l + gap, r - gap);
					if b - a >= min_width {
						segs.push((a, b));
					}
				}
			}
			for (a, b) in segs {
				let mut pb = PathBuilder::new();
				pb.move_to(Pt::new(a as f32, y as f32));
				pb.line_to(Pt::new(b as f32, y as f32));
				let path = res!(pb.finish());
				ops.push(DrawOp::Stroke { path, colour, width: thick as f32 });
			}
		}
	}
	if ops.is_empty() {
		return Ok((Vec::new(), background));
	}
	let graphic = Graphic::new(ops, Dims::new(Sp::ZERO, Sp::ZERO, Sp::ZERO));
	Ok((vec![Node::Leaf(Leaf::graphic(graphic))], background))
}

/// Shapes one run of text under the chain's `text` properties, as one drawable run: the contract entry
/// for callers outside inline layout (an SVG's text, a label). Faces are chosen with Typst's fallback;
/// the run's glyphs may come from several faces.
pub fn shape(engine: &mut Engine, text: &str, styles: &StyleChain) -> Outcome<ShapedText> {
	let book = res!(engine.fonts.book());
	let props = Arc::new(res!(props(styles)));
	let run = res!(shape_run(&book, 0, text, &props, props.rtl));
	let size = props.size;
	let mut fonts: Vec<Arc<oxedyne_fe2o3_font::font::Font>> = Vec::new();
	let mut glyphs = Vec::with_capacity(run.glyphs.len());
	let mut pen = 0.0f64;	// points
	for g in run.glyphs.iter() {
		let bf = match book.face(g.face) {
			Some(f)	=> f,
			None	=> continue,
		};
		let fi = match fonts.iter().position(|f| Arc::ptr_eq(f, &bf.font)) {
			Some(i)	=> i,
			None	=> {
				fonts.push(bf.font.clone());
				fonts.len() - 1
			}
		};
		glyphs.push(RunGlyph {
			id:			g.id,
			face:		fi as u8,
			x:			(pen + g.x_offset * g.size) as f32,
			y:			(g.y_offset * size) as f32,
			adv:		(g.x_advance * g.size) as f32,
			cluster:	g.range.0,
		});
		pen += g.x_advance * g.size;
	}
	let (top, bottom) = run.measure(&book);
	let width = run.width();
	ShapedText::from_faces(
		fonts, size as f32, glyphs, text.to_string(), props.fill,
		Dims::new(Sp::from_pt(width), Sp::from_pt(top), Sp::from_pt(bottom)),
	)
}

/// The absolute length of a relative one at a whole of `whole` points under the chain's font size.
pub fn resolve_rel(r: &Relative, whole: f64, styles: &StyleChain) -> f64 {
	r.rel.0 * whole + styles.resolve_length(r.abs)
}

/// A length in points under the chain's font size.
pub fn resolve_len(l: &Length, styles: &StyleChain) -> f64 {
	styles.resolve_length(*l)
}
