// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-layout `math/fragment`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
//! Laid-out maths pieces: glyphs (with their stretching to a size through variants and assemblies),
//! frames, spaces and tags. A port of Typst 0.15's `typst-layout/src/math/fragment`.

use crate::diag::DiagnosticKind;
use crate::eval::realise::Tag;
use crate::eval::styles::StyleChain;
use crate::eval::value::Paint;
use crate::eval::Engine;
use crate::flow::visual::P2;
use crate::math::class::{
	default_math_class,
	MathClass,
};
use crate::math::font::{
	chain,
	Corner,
	MathFont,
	ShapedGlyph,
};
use crate::math::frame::{
	FItem,
	GlyphRun,
	MFrame,
};
use crate::math::item::{
	Props,
	Stretch,
};
use crate::math::props;
use crate::math::style::MathSize;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::shape::Feature;

use std::sync::Arc;

const MAX_REPEATS: usize = 1024;	// most repetitions of an assembly's extenders

#[derive(Clone)]
pub struct GlyphFrag {
	pub font:				Arc<MathFont>,
	pub size:				f64,		// font size, points
	pub fill:				Paint,
	pub text:				String,
	pub glyphs:				Vec<ShapedGlyph>,
	pub w:					f64,
	pub h:					f64,
	baseline:				Option<f64>,
	pub italics:			f64,
	pub accent_attach:		(f64, f64),
	pub math_size:			MathSize,
	pub class:				MathClass,
	pub extended:			bool,
	shift:					f64,
	align:					f64,
	pub span:				Span,
}

impl GlyphFrag {
	/// A glyph for one character in the maths font chain the styles name.
	pub fn synthetic(engine: &mut Engine, styles: &StyleChain, c: char, span: Span) -> Outcome<Option<Self>> {
		let class = default_math_class(c).unwrap_or(MathClass::Normal);
		let mut s = String::new();
		s.push(c);
		let g = res!(Self::base(engine, styles, &res!(props::features(styles)), &s, class, props::size(styles)));
		Ok(g.map(|mut g| { g.span = span; g }))
	}

	/// A glyph for `text` (one grapheme), stretched as `stretch` asks.
	pub fn new(
		engine:		&mut Engine,
		text:		&str,
		stretch:	&Stretch,
		styles:		&StyleChain,
		features:	Vec<Feature>,
		props:		&Props,
		class:		MathClass,
	)
		-> Outcome<Option<Self>>
	{
		let mut glyph = match res!(Self::base(engine, styles, &features, text, class, props.size)) {
			Some(g)	=> g,
			None	=> return Ok(None),
		};
		glyph.span = props.span;
		let mut action = res!(decide(&glyph, stretch));
		if matches!(action, Action::Fallback) {
			// Retry without the flattened-accent and script-style forms until one can stretch or fits.
			let had_flac = features.iter().rev().find(|f| &f.tag == b"flac").map(|f| f.value != 0).unwrap_or(false);
			let prev_ssty = features.iter().rev().find(|f| &f.tag == b"ssty").map(|f| f.value).unwrap_or(0);
			let base: Vec<Feature> = features.iter().copied().filter(|f| &f.tag != b"flac" && &f.tag != b"ssty").collect();
			const OPTIONS: [(bool, u32); 6] = [(true, 2), (true, 1), (true, 0), (false, 2), (false, 1), (false, 0)];
			for (flac, ssty) in OPTIONS {
				if (flac && !had_flac) || ssty > prev_ssty || (flac, ssty) == (had_flac, prev_ssty) {
					continue;
				}
				let mut f = base.clone();
				if flac {
					f.push(Feature { tag: *b"flac", value: 1 });
				}
				if ssty > 0 {
					f.push(Feature { tag: *b"ssty", value: ssty });
				}
				if let Some(mut g) = res!(Self::base(engine, styles, &f, text, class, props.size)) {
					g.span = props.span;
					let a = res!(decide(&g, stretch));
					if !matches!(a, Action::Fallback) {
						glyph = g;
						action = a;
						break;
					}
				}
			}
		}
		match action {
			Action::Stretch { vertical, target, short_fall } => {
				res!(glyph.stretch(engine, target, short_fall, vertical));
				if vertical {
					glyph.center_on_axis();
				}
			}
			Action::WarnBothAxes => {
				let d = crate::diag::Diagnostic::warning(DiagnosticKind::Lint, props.span, "glyph has both vertical and horizontal constructions")
					.with_hint("this is probably a font bug");
				engine.diags.push(d);
			}
			Action::Keep | Action::Fallback => (),
		}
		Ok(Some(glyph))
	}

	fn base(engine: &mut Engine, styles: &StyleChain, features: &[Feature], text: &str, class: MathClass, math_size: MathSize) -> Outcome<Option<Self>> {
		let book = res!(engine.fonts.book());
		let fonts = res!(chain(&book, &res!(props::families(styles)), res!(props::fallback(styles)), res!(props::face_variant(styles))));
		let mut chosen: Option<(Arc<MathFont>, Vec<ShapedGlyph>)> = None;
		for f in &fonts {
			let glyphs = res!(f.shape(text, features));
			let complete = !glyphs.is_empty() && glyphs.iter().all(|g| g.id != 0) && f.covers(text);
			if complete {
				chosen = Some((f.clone(), glyphs));
				break;
			}
			if chosen.is_none() && !glyphs.is_empty() {
				chosen = Some((f.clone(), glyphs));
			}
		}
		let (font, glyphs) = match chosen {
			Some(c)	=> c,
			None	=> return Ok(None),
		};
		let mut g = Self {
			font,
			size:			props::font_size(styles),
			fill:			res!(props::fill(styles)),
			text:			text.to_string(),
			glyphs,
			w:				0.0,
			h:				0.0,
			baseline:		None,
			italics:		0.0,
			accent_attach:	(0.0, 0.0),
			math_size,
			class,
			extended:		false,
			shift:			res!(props::baseline(styles)),
			align:			0.0,
			span:			Span::detached(),
		};
		res!(g.update_glyph());
		Ok(Some(g))
	}

	fn run_width(&self) -> f64 { self.glyphs.iter().map(|g| g.x_advance).sum::<f64>() * self.size }

	fn first(&self) -> u16 { self.glyphs.first().map(|g| g.id).unwrap_or(0) }

	// Typst's `update_glyph`: metrics from the first glyph, its italics correction added to its advance
	// unless it is an extended shape.
	fn update_glyph(&mut self) -> Outcome<()> {
		let id = self.first();
		let extended = self.font.is_extended_shape(id);
		let italics_em = self.font.italics_correction(id).unwrap_or(0.0);
		let width = self.run_width();
		if !extended {
			if let Some(g) = self.glyphs.first_mut() {
				g.x_advance += italics_em;
			}
		}
		let italics = italics_em * self.size;
		let (asc, desc) = res!(self.font.ascent_descent(id)).unwrap_or((0.0, 0.0));
		let top = self.font.top_accent_attachment(id).map(|a| a * self.size).unwrap_or((width + italics) / 2.0);
		let bottom = (width - italics) / 2.0;
		self.baseline		= Some(asc * self.size);
		self.w				= self.run_width();
		self.h				= (asc + desc) * self.size;
		self.italics		= italics;
		self.accent_attach	= (top, bottom);
		self.extended		= extended;
		Ok(())
	}

	pub fn ascent(&self) -> f64 { self.baseline.unwrap_or(self.h) }
	pub fn descent(&self) -> f64 { self.h - self.ascent() }

	pub fn into_frame(self) -> MFrame {
		let mut f = MFrame::new(self.w, self.h);
		let asc = self.ascent();
		f.set_baseline(asc);
		let y = asc + self.shift + self.align;
		f.push(P2::new(0.0, y), FItem::Glyphs(GlyphRun {
			font:	self.font,
			size:	self.size,
			fill:	self.fill,
			text:	self.text,
			glyphs:	self.glyphs,
		}));
		f
	}

	fn stretch_advance(&self, vertical: bool) -> f64 {
		if vertical {
			self.h
		} else if !self.extended {
			self.w - self.italics
		} else {
			self.w
		}
	}

	/// Grows the glyph toward `target` along an axis: the smallest pre-drawn variant that reaches it,
	/// else an assembly from parts.
	fn stretch(&mut self, engine: &mut Engine, target: f64, short_fall: f64, vertical: bool) -> Outcome<()> {
		let advance = self.stretch_advance(vertical);
		let short_target = target - short_fall;
		if short_target <= advance {
			return Ok(());
		}
		let id = self.first();
		let font = self.font.clone();
		let construction = match font.construction(id, vertical) {
			Some(c)	=> c.clone(),
			None	=> return Ok(()),
		};
		let mut best_id = id;
		let mut best_adv = advance;
		for &(gid, adv) in &construction.variants {
			best_id = gid;
			best_adv = font.em(adv as f64) * self.size;
			if short_target <= best_adv {
				break;
			}
		}
		if short_target <= best_adv || construction.assembly.is_none() {
			self.glyphs = vec![ShapedGlyph {
				id:			best_id,
				x_advance:	font.advance(best_id),
				x_offset:	0.0,
				y_advance:	0.0,
				y_offset:	0.0,
			}];
			return self.update_glyph();
		}
		let assembly = match construction.assembly {
			Some(a)	=> a,
			None	=> return Ok(()),
		};
		let min_overlap = font.min_connector_overlap().unwrap_or(0.0) * self.size;
		self.assemble(engine, &assembly, min_overlap, target, vertical)
	}

	fn assemble(
		&mut self,
		engine:			&mut Engine,
		assembly:		&crate::mathtable::Assembly,
		min_overlap:	f64,
		target:			f64,
		vertical:		bool,
	)
		-> Outcome<()>
	{
		let font = self.font.clone();
		let size = self.size;
		let parts = |repeat: usize| -> Vec<crate::mathtable::Part> {
			let mut v = Vec::new();
			for p in &assembly.parts {
				let n = if p.extender { repeat } else { 1 };
				for _ in 0..n {
					v.push(*p);
				}
			}
			v
		};
		let mut full;
		let mut ratio;
		let mut repeat = 0;
		let mut warned = false;
		loop {
			full = 0.0;
			ratio = 0.0;
			let ps = parts(repeat);
			let mut growable = 0.0;
			for (i, part) in ps.iter().enumerate() {
				let mut advance = font.em(part.full_advance as f64) * size;
				if let Some(next) = ps.get(i + 1) {
					let max_overlap = font.em(part.end_connector.min(next.start_connector) as f64) * size;
					if max_overlap < min_overlap && !warned {
						warned = true;
						let d = crate::diag::Diagnostic::warning(DiagnosticKind::Lint, self.span,
							"glyph has assembly parts with overlap less than minConnectorOverlap")
							.with_hint("its rendering may appear broken - this is probably a font bug");
						engine.diags.push(d);
					}
					advance -= max_overlap;
					growable += (max_overlap - min_overlap).max(0.0);
				}
				full += advance;
			}
			if full < target {
				let delta = target - full;
				ratio = if growable > 0.0 { (delta / growable).min(1.0) } else { 0.0 };
				full += ratio * growable;
			}
			if target <= full || repeat >= MAX_REPEATS {
				break;
			}
			repeat += 1;
		}
		let ps = parts(repeat);
		let mut glyphs = Vec::with_capacity(ps.len());
		for (i, part) in ps.iter().enumerate() {
			let mut advance = font.em(part.full_advance as f64) * size;
			if let Some(next) = ps.get(i + 1) {
				let max_overlap = font.em(part.end_connector.min(next.start_connector) as f64) * size;
				advance -= max_overlap;
				advance += ratio * (max_overlap - min_overlap);
			}
			let g = if vertical {
				let desc = res!(font.ascent_descent(part.glyph)).map(|x| x.1).unwrap_or(0.0);
				ShapedGlyph { id: part.glyph, x_advance: 0.0, x_offset: 0.0, y_advance: advance / size, y_offset: desc }
			} else {
				ShapedGlyph { id: part.glyph, x_advance: advance / size, x_offset: 0.0, y_advance: 0.0, y_offset: 0.0 }
			};
			glyphs.push(g);
		}
		if vertical {
			self.baseline = None;
			self.h = full;
			let mut w = 0.0f64;
			for g in &glyphs {
				w = w.max(font.advance(g.id));
			}
			self.w = w * size;
		} else {
			self.w = full;
			// The parts' extremes, not clamped to the baseline: a rule above it has a negative depth.
			let mut ext: Option<(f64, f64)> = None;
			for g in &glyphs {
				if let Some((a, d)) = res!(font.ascent_descent(g.id)) {
					ext = Some(match ext {
						None			=> (a, d),
						Some((ma, md))	=> (ma.max(a), md.max(d)),
					});
				}
			}
			let (asc, desc) = ext.unwrap_or((0.0, 0.0));
			self.baseline = Some(asc * size);
			self.h = (asc + desc) * size;
		}
		self.glyphs = glyphs;
		self.italics = font.em(assembly.italics_correction as f64) * size;
		if !vertical {
			self.accent_attach = (full / 2.0, full / 2.0);
		}
		self.extended = true;
		Ok(())
	}

	/// Centres the glyph vertically on the maths axis.
	pub fn center_on_axis(&mut self) {
		let axis = self.font.consts.axis_height * self.size;
		self.align += self.ascent();
		self.baseline = Some((self.h + 2.0 * axis) / 2.0);
		self.align -= self.ascent();
	}

	pub fn kern_at_height(&self, corner: Corner, height: f64) -> f64 {
		let vertical = self.glyphs.iter().all(|g| g.y_advance != 0.0);
		let last = self.glyphs.len().saturating_sub(1);
		let i = match (vertical, corner) {
			(true, Corner::TopLeft | Corner::TopRight)			=> last,
			(false, Corner::TopRight | Corner::BottomRight)		=> last,
			_													=> 0,
		};
		let id = self.glyphs.get(i).map(|g| g.id).unwrap_or(0);
		self.font.kern_at_height(id, corner, height / self.size).unwrap_or(0.0) * self.size
	}
}

#[derive(Clone, Copy)]
enum Action {
	Keep,
	Stretch { vertical: bool, target: f64, short_fall: f64 },
	WarnBothAxes,
	Fallback,
}

enum AxisStatus {
	Sufficient,
	Stretchable(f64, f64),
	Fallback,
}

fn decide(glyph: &GlyphFrag, stretch: &Stretch) -> Outcome<Action> {
	let id = glyph.first();
	let font = &glyph.font;
	let assess = |vertical: bool| -> Outcome<AxisStatus> {
		let (target, short_fall) = match resolve_stretch(glyph, stretch, vertical) {
			Some(t)	=> t,
			None	=> return Ok(AxisStatus::Sufficient),
		};
		if font.construction(id, vertical).is_some() {
			return Ok(AxisStatus::Stretchable(target, short_fall));
		}
		let mut advance = glyph.stretch_advance(vertical);
		// A combining mark has no advance; its ink box says how wide it is.
		if let Some(b) = res!(font.bbox(id)) {
			let ext = if vertical { b.y1 - b.y0 } else { b.x1 - b.x0 };
			advance = advance.max(ext * glyph.size);
		}
		Ok(if target - short_fall <= advance { AxisStatus::Sufficient } else { AxisStatus::Fallback })
	};
	Ok(match (res!(assess(false)), res!(assess(true))) {
		(AxisStatus::Stretchable(..), AxisStatus::Stretchable(..))	=> Action::WarnBothAxes,
		(AxisStatus::Stretchable(t, s), _)	=> Action::Stretch { vertical: false, target: t, short_fall: s },
		(_, AxisStatus::Stretchable(t, s))	=> Action::Stretch { vertical: true, target: t, short_fall: s },
		(AxisStatus::Sufficient, AxisStatus::Sufficient)	=> Action::Keep,
		_													=> Action::Fallback,
	})
}

fn resolve_stretch(glyph: &GlyphFrag, stretch: &Stretch, vertical: bool) -> Option<(f64, f64)> {
	let info = stretch.resolve(vertical)?;
	let rel = info.relative_to.unwrap_or_else(|| {
		if vertical && glyph.class == MathClass::Large && glyph.math_size == MathSize::Display {
			glyph.font.consts.display_operator_min_height * glyph.size
		} else if vertical {
			glyph.h
		} else {
			glyph.w
		}
	});
	let target = info.target.relative_to(rel);
	let short_fall = info.short_fall * info.font_size.unwrap_or(glyph.size);
	Some((target, short_fall))
}

/// A laid-out part with the metadata maths placement reads.
#[derive(Clone)]
pub struct FrameFrag {
	pub frame:			MFrame,
	pub font_size:		f64,
	pub class:			MathClass,
	pub math_size:		MathSize,
	pub base_ascent:	f64,
	pub base_descent:	f64,
	pub italics:		f64,
	pub accent_attach:	(f64, f64),
	pub text_like:		bool,
}

impl FrameFrag {
	pub fn new(props: &Props, styles: &StyleChain, frame: MFrame) -> Self {
		let half = frame.w / 2.0;
		Self {
			font_size:		props::font_size(styles),
			class:			props.class(),
			math_size:		props.size,
			base_ascent:	frame.ascent(),
			base_descent:	frame.descent(),
			italics:		0.0,
			accent_attach:	(half, half),
			text_like:		false,
			frame,
		}
	}
}

#[derive(Clone)]
pub enum Frag {
	Glyph(GlyphFrag),
	Frame(FrameFrag),
	Space(f64),
	Tag(Tag),
}

impl Frag {
	pub fn width(&self) -> f64 {
		match self {
			Frag::Glyph(g)	=> g.w,
			Frag::Frame(f)	=> f.frame.w,
			Frag::Space(w)	=> *w,
			Frag::Tag(_)	=> 0.0,
		}
	}

	pub fn height(&self) -> f64 {
		match self {
			Frag::Glyph(g)	=> g.h,
			Frag::Frame(f)	=> f.frame.h,
			_				=> 0.0,
		}
	}

	pub fn ascent(&self) -> f64 {
		match self {
			Frag::Glyph(g)	=> g.ascent(),
			Frag::Frame(f)	=> f.frame.ascent(),
			_				=> 0.0,
		}
	}

	pub fn descent(&self) -> f64 {
		match self {
			Frag::Glyph(g)	=> g.descent(),
			Frag::Frame(f)	=> f.frame.descent(),
			_				=> 0.0,
		}
	}

	pub fn base_ascent(&self) -> f64 {
		match self {
			Frag::Frame(f)	=> f.base_ascent,
			_				=> self.ascent(),
		}
	}

	pub fn base_descent(&self) -> f64 {
		match self {
			Frag::Frame(f)	=> f.base_descent,
			_				=> self.descent(),
		}
	}

	pub fn class(&self) -> MathClass {
		match self {
			Frag::Glyph(g)	=> g.class,
			Frag::Frame(f)	=> f.class,
			Frag::Space(_)	=> MathClass::Space,
			Frag::Tag(_)	=> MathClass::Special,
		}
	}

	pub fn math_size(&self) -> Option<MathSize> {
		match self {
			Frag::Glyph(g)	=> Some(g.math_size),
			Frag::Frame(f)	=> Some(f.math_size),
			_				=> None,
		}
	}

	/// The font a fragment was set in and its size: a glyph's own, else the context's.
	pub fn font(&self, current: &Arc<MathFont>, styles: &StyleChain) -> (Arc<MathFont>, f64) {
		match self {
			Frag::Glyph(g)	=> (g.font.clone(), g.size),
			Frag::Frame(f)	=> (current.clone(), f.font_size),
			_				=> (current.clone(), props::font_size(styles)),
		}
	}

	pub fn is_text_like(&self) -> bool {
		match self {
			Frag::Glyph(g)	=> !g.extended,
			Frag::Frame(f)	=> f.text_like,
			_				=> false,
		}
	}

	pub fn italics_correction(&self) -> f64 {
		match self {
			Frag::Glyph(g)	=> g.italics,
			Frag::Frame(f)	=> f.italics,
			_				=> 0.0,
		}
	}

	pub fn accent_attach(&self) -> (f64, f64) {
		match self {
			Frag::Glyph(g)	=> g.accent_attach,
			Frag::Frame(f)	=> f.accent_attach,
			_				=> (self.width() / 2.0, self.width() / 2.0),
		}
	}

	pub fn fill(&self) -> Option<Paint> {
		match self {
			Frag::Glyph(g)	=> Some(g.fill.clone()),
			_				=> None,
		}
	}

	pub fn kern_at_height(&self, corner: Corner, height: f64) -> f64 {
		match self {
			Frag::Glyph(g)	=> g.kern_at_height(corner, height),
			_				=> 0.0,
		}
	}

	pub fn into_frame(self) -> MFrame {
		match self {
			Frag::Glyph(g)	=> g.into_frame(),
			Frag::Frame(f)	=> f.frame,
			Frag::Tag(t)	=> {
				let mut f = MFrame::new(0.0, 0.0);
				f.push(P2::new(0.0, 0.0), FItem::Tag(t));
				f
			}
			Frag::Space(w)	=> MFrame::new(w, 0.0),
		}
	}
}
