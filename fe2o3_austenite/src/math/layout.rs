// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-layout `math/` (mod, run, text, scripts, fraction, radical, fenced, accent, line, cancel, table), version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
//! Maths IR into fragments and frames: a port of Typst 0.15's `typst-layout/src/math` (mod, run, text,
//! scripts, fraction, radical, fenced, accent, line, cancel, table). Lengths are points, y down.

use crate::diag::DiagnosticKind;
use crate::eval::content::Content;
use crate::eval::func::Func;
use crate::eval::styles::StyleChain;
use crate::eval::value::{
	Angle,
	HAlign,
	LineCap,
	Value,
};
use crate::eval::Engine;
use crate::flow::visual::P2;
use crate::flow::Region;
use crate::math::class::MathClass;
use crate::math::font::{
	resolve as resolve_font,
	Corner,
	MathFont,
};
use crate::math::fragment::{
	Frag,
	FrameFrag,
	GlyphFrag,
};
use crate::math::frame::{
	line_pen,
	pen,
	FItem,
	GlyphRun,
	MFrame,
	Pen,
};
use crate::math::item::{
	Alternator,
	Cancel,
	FencedBody,
	Fixed,
	Item,
	Kind,
	Position,
	Props,
	Row,
	Scripts,
	Table,
};
use crate::math::props;
use crate::math::style::MathSize;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::shape::Feature;

use std::sync::Arc;

const TIGHT_LEADING:			f64 = 0.25;	// ems, between rows in script sizes
const DEFAULT_STROKE_THICKNESS:	f64 = 0.05;	// ems, matrix augmentation lines

pub type Run = Vec<Frag>;

/// The state of one equation's layout.
pub struct Ctx<'e> {
	pub engine:	&'e mut Engine,
	pub region:	(f64, f64),
	fonts:		Vec<Arc<MathFont>>,
	frags:		Vec<Frag>,
}

impl<'e> Ctx<'e> {
	pub fn new(engine: &'e mut Engine, region: (f64, f64), font: Arc<MathFont>) -> Self {
		Self { engine, region, fonts: vec![font], frags: Vec::new() }
	}

	pub fn font(&self) -> Arc<MathFont> {
		match self.fonts.last() {
			Some(f)	=> f.clone(),
			None	=> Arc::clone(&self.fonts[0]),
		}
	}

	fn push(&mut self, f: Frag) { self.frags.push(f); }

	pub fn layout_into_fragments(&mut self, item: &Item, styles: &StyleChain) -> Outcome<Run> {
		let start = self.frags.len();
		res!(self.layout_into_self(item, styles));
		Ok(self.frags.drain(start..).collect())
	}

	pub fn layout_into_fragment(&mut self, item: &Item, styles: &StyleChain) -> Outcome<Frag> {
		let mut frags = res!(self.layout_into_fragments(item, styles));
		if frags.len() == 1 {
			if let Some(f) = frags.pop() {
				return Ok(f);
			}
		}
		let text_like = frags.iter().filter(|f| f.math_size().is_some()).all(|f| f.is_text_like());
		let st = item.styles().cloned().unwrap_or_else(|| styles.clone());
		let props = Props::new(&st, None, Span::detached());
		let frame = into_frame(frags);
		let mut ff = FrameFrag::new(&props, &st, frame);
		ff.text_like = text_like;
		Ok(Frag::Frame(ff))
	}

	fn layout_into_self(&mut self, item: &Item, styles: &StyleChain) -> Outcome<()> {
		let outer = item.styles().cloned().unwrap_or_else(|| styles.clone());
		let outer_families = res!(props::families(&outer));
		for it in item.as_slice() {
			let st = it.styles().cloned().unwrap_or_else(|| outer.clone());
			if !st.ptr_eq(&outer) && res!(props::families(&st)) != outer_families {
				let f = res!(resolve_font(&res!(props::families(&st)), res!(props::fallback(&st)), res!(props::weight(&st))));
				let scaled = props::chain_with(&st, vec![props::set_script_scale(
					f.consts.script_percent_scale_down, f.consts.script_script_percent_scale_down)]);
				self.fonts.push(f);
				let r = self.layout_realised(it, &scaled);
				self.fonts.pop();
				res!(r);
			} else {
				res!(self.layout_realised(it, &st));
			}
		}
		Ok(())
	}

	fn layout_realised(&mut self, item: &Item, styles: &StyleChain) -> Outcome<()> {
		let comp = match item {
			Item::Comp(c)			=> c,
			Item::Spacing(w, _)		=> { self.push(Frag::Space(*w)); return Ok(()); }
			Item::Space				=> {
				let w = self.font().consts.space_width * props::font_size(styles);
				self.push(Frag::Space(w));
				return Ok(());
			}
			Item::Tag(t)			=> { self.push(Frag::Tag(t.clone())); return Ok(()); }
		};
		let p = &comp.props;
		if let Some(l) = p.lspace {
			if !p.align_form_infix && l != 0.0 {
				self.push(Frag::Space(l * props::font_size(styles)));
			}
		}
		match &comp.kind {
			Kind::External(c)									=> res!(self.external(c, styles, p)),
			Kind::Glyph(g)										=> res!(self.glyph(g, styles, p)),
			Kind::Cancel(c)										=> res!(self.cancel(c, styles, p)),
			Kind::Radical { radicand, index, sqrt }				=> res!(self.radical(radicand, index.as_ref(), sqrt, styles, p)),
			Kind::Line { base, position }						=> res!(self.line(base, *position, styles, p)),
			Kind::Accent { base, accent, position, exact, .. }	=> res!(self.accent(base, accent, *position, *exact, styles, p)),
			Kind::Scripts(s)									=> res!(self.scripts(s, styles, p)),
			Kind::Primes(n)										=> res!(self.primes(*n, styles, p)),
			Kind::Table(t)										=> res!(self.table(t, styles, p)),
			Kind::Fraction { num, denom, line, padding }		=> res!(self.fraction(num, denom, *line, *padding, styles, p)),
			Kind::Skewed { num, denom, slash }					=> res!(self.skewed(num, denom, slash, styles, p)),
			Kind::Text(t)										=> res!(self.text(t, styles, p)),
			Kind::Number(t)										=> res!(self.number(t, styles, p)),
			Kind::Fenced { open, close, body, balanced }		=> res!(self.fenced(open.as_ref(), close.as_ref(), body, *balanced, styles)),
			Kind::Multiline { rows, centered }					=> {
				let mut frame = res!(self.multiline(rows, styles)).build(false);
				if *centered {
					let axis = self.font().consts.axis_height * props::font_size(styles);
					frame.set_baseline(frame.h / 2.0 + axis);
				}
				self.push(Frag::Frame(FrameFrag::new(p, styles, frame)));
			}
			Kind::Group(_) => {
				let f = res!(self.layout_into_fragment(item, styles));
				let italics = f.italics_correction();
				let attach = f.accent_attach();
				let mut ff = FrameFrag::new(p, styles, f.into_frame());
				ff.italics = italics;
				ff.accent_attach = attach;
				self.push(Frag::Frame(ff));
			}
		}
		if let Some(r) = p.rspace {
			if r != 0.0 {
				self.push(Frag::Space(r * props::font_size(styles)));
			}
		}
		Ok(())
	}

	// Leaves

	fn external(&mut self, content: &Content, styles: &StyleChain, p: &Props) -> Outcome<()> {
		let w = self.region.0;
		let h = self.region.1;
		let region = Region {
			width:		sp(w),
			height:		sp(h),
			base:		(sp(w), sp(h)),
			expand_x:	false,
			expand_y:	false,
		};
		let nodes = res!(crate::flow::layout_block(self.engine, content, styles, region));
		let dims = crate::flow::visual::vlist_dims(&nodes);
		let fw = dims.width.to_pt();
		let fh = dims.vextent().to_pt();
		let mut frame = MFrame::new(fw, fh);
		match crate::flow::visual::first_baseline(&nodes) {
			Some(b)	=> frame.set_baseline(b.to_pt()),
			None	=> {
				let axis = self.font().consts.axis_height * props::font_size(styles);
				frame.set_baseline(fh / 2.0 + axis);
			}
		}
		frame.push(P2::new(0.0, 0.0), FItem::Node { nodes, w: fw, h: fh });
		self.push(Frag::Frame(FrameFrag::new(p, styles, frame)));
		Ok(())
	}

	fn glyph(&mut self, g: &crate::math::item::Glyph, styles: &StyleChain, p: &Props) -> Outcome<()> {
		let mut features = res!(props::features(styles));
		if props::dtls(styles) {
			features.push(Feature { tag: *b"dtls", value: 1 });
		}
		if g.flac.get() {
			features.push(Feature { tag: *b"flac", value: 1 });
		}
		// A dotless letter is shaped from its dotted form with the `dtls` feature where the font has it.
		let font = self.font();
		let mut text = g.text.clone();
		if text.chars().any(|c| dotted(c).is_some()) && font.has_feature(b"dtls") {
			features.push(Feature { tag: *b"dtls", value: 1 });
			let (v, b, i) = (props::variant(styles), props::bold(styles), props::italic(styles));
			let mut s = String::new();
			for c in text.chars() {
				let c = dotted(c).unwrap_or(c);
				crate::math::style::push_styled(&mut s, c, crate::math::style::MathStyle::select(c, v, b, i));
			}
			text = s;
		}
		let glyph = res!(GlyphFrag::new(self.engine, &text, &g.stretch.get(), styles, features, p, g.class));
		if let Some(mut glyph) = glyph {
			if glyph.class == MathClass::Large {
				glyph.center_on_axis();
			}
			glyph.class = p.class();
			self.push(Frag::Glyph(glyph));
		}
		Ok(())
	}

	fn text(&mut self, text: &str, styles: &StyleChain, p: &Props) -> Outcome<()> {
		// A run of text in maths sets as a line of its own, its box the ink's bounds.
		let fonts = res!(crate::math::font::chain(&res!(props::families(styles)), res!(props::fallback(styles)), res!(props::weight(styles))));
		let mut feats: Vec<Feature> = res!(props::features(styles)).into_iter().filter(|f| &f.tag != b"ssty").collect();
		if props::dtls(styles) {
			feats.push(Feature { tag: *b"dtls", value: 1 });
		}
		let font = match fonts.iter().find(|f| f.covers(text)).or_else(|| fonts.first()) {
			Some(f)	=> f.clone(),
			None	=> return Ok(()),
		};
		let glyphs = res!(font.shape_text(text, &feats));
		let size = props::font_size(styles);
		let mut asc = 0.0f64;
		let mut desc = 0.0f64;
		let mut pen = 0.0f64;
		let mut any = false;
		for g in &glyphs {
			if let Some(b) = res!(font.bbox(g.id)) {
				let dy = g.y_offset;
				asc = if any { asc.max(b.y1 + dy) } else { b.y1 + dy };
				desc = if any { desc.max(-(b.y0 + dy)) } else { -(b.y0 + dy) };
				any = true;
			}
			pen += g.x_advance;
		}
		let (asc, desc) = (asc * size, desc * size);
		let mut frame = MFrame::new(pen * size, asc + desc);
		frame.set_baseline(asc);
		frame.push(P2::new(0.0, asc + res!(props::baseline(styles))), FItem::Glyphs(GlyphRun {
			font,
			size,
			fill:	res!(props::fill(styles)),
			text:	text.to_string(),
			glyphs,
		}));
		let mut ff = FrameFrag::new(p, styles, frame);
		ff.text_like = true;
		self.push(Frag::Frame(ff));
		Ok(())
	}

	fn number(&mut self, text: &str, styles: &StyleChain, p: &Props) -> Outcome<()> {
		let mut frags = Vec::new();
		for c in text.chars() {
			if let Some(g) = res!(GlyphFrag::synthetic(styles, c, p.span)) {
				frags.push(Frag::Glyph(g));
			}
		}
		let mut ff = FrameFrag::new(p, styles, into_frame(frags));
		ff.text_like = true;
		self.push(Frag::Frame(ff));
		Ok(())
	}

	fn primes(&mut self, count: usize, styles: &StyleChain, p: &Props) -> Outcome<()> {
		let prime = match res!(GlyphFrag::synthetic(styles, '\u{2032}', p.span)) {
			Some(g)	=> Frag::Glyph(g).into_frame(),
			None	=> return Ok(()),
		};
		let w = prime.w * (count + 1) as f64 / 2.0;
		let mut frame = MFrame::new(w, prime.h);
		frame.set_baseline(prime.ascent());
		for i in 0..count {
			frame.push_frame(P2::new(prime.w * (i as f64 / 2.0), 0.0), prime.clone());
		}
		let mut ff = FrameFrag::new(p, styles, frame);
		ff.text_like = true;
		self.push(Frag::Frame(ff));
		Ok(())
	}

	// Scripts

	fn scripts(&mut self, s: &Scripts, styles: &StyleChain, p: &Props) -> Outcome<()> {
		let t = match &s.t { Some(i) => Some(res!(self.layout_into_fragment(i, styles))), None => None };
		let b = match &s.b { Some(i) => Some(res!(self.layout_into_fragment(i, styles))), None => None };
		let rel = t.as_ref().map(|f| f.width()).unwrap_or(0.0).max(b.as_ref().map(|f| f.width()).unwrap_or(0.0));
		s.base.set_stretch_relative_to(rel, false);
		let base = res!(self.layout_into_fragment(&s.base, styles));
		let tl = match &s.tl { Some(i) => Some(res!(self.layout_into_fragment(i, styles))), None => None };
		let tr = match &s.tr { Some(i) => Some(res!(self.layout_into_fragment(i, styles))), None => None };
		let bl = match &s.bl { Some(i) => Some(res!(self.layout_into_fragment(i, styles))), None => None };
		let br = match &s.br { Some(i) => Some(res!(self.layout_into_fragment(i, styles))), None => None };
		let bstyles = s.base.styles().cloned().unwrap_or_else(|| styles.clone());
		self.attachments(p, &bstyles, base, [tl, t, tr, bl, b, br])
	}

	fn attachments(&mut self, p: &Props, styles: &StyleChain, base: Frag, parts: [Option<Frag>; 6]) -> Outcome<()> {
		let [tl, t, tr, bl, b, br] = parts;
		let (font, size) = base.font(&self.font(), styles);
		let c = font.consts;
		let cramped = props::cramped(styles);
		let asc = |f: &Option<Frag>| f.as_ref().map(|f| f.ascent()).unwrap_or(0.0);
		let desc = |f: &Option<Frag>| f.as_ref().map(|f| f.descent()).unwrap_or(0.0);
		let wid = |f: &Option<Frag>| f.as_ref().map(|f| f.width()).unwrap_or(0.0);

		// Shifts of the side scripts' baselines from the base's.
		let (tx_shift, bx_shift) = if tl.is_none() && tr.is_none() && bl.is_none() && br.is_none() {
			(0.0, 0.0)
		} else {
			let sup_shift_up = (if cramped { c.superscript_shift_up_cramped } else { c.superscript_shift_up }) * size;
			let sup_bottom_min = c.superscript_bottom_min * size;
			let sup_bottom_max_with_sub = c.superscript_bottom_max_with_subscript * size;
			let sup_drop_max = c.superscript_baseline_drop_max * size;
			let gap_min = c.sub_superscript_gap_min * size;
			let sub_shift_down = c.subscript_shift_down * size;
			let sub_top_max = c.subscript_top_max * size;
			let sub_drop_min = c.subscript_baseline_drop_min * size;
			let text_like = base.is_text_like();
			let mut up = 0.0f64;
			let mut down = 0.0f64;
			if tl.is_some() || tr.is_some() {
				let a = base.base_ascent();
				up = up.max(sup_shift_up)
					.max(if text_like { 0.0 } else { a - sup_drop_max })
					.max(sup_bottom_min + desc(&tl))
					.max(sup_bottom_min + desc(&tr));
			}
			if bl.is_some() || br.is_some() {
				let d = base.base_descent();
				down = down.max(sub_shift_down)
					.max(if text_like { 0.0 } else { d + sub_drop_min })
					.max(asc(&bl) - sub_top_max)
					.max(asc(&br) - sub_top_max);
			}
			for (sup, sub) in [(&tl, &bl), (&tr, &br)] {
				if let (Some(sup), Some(sub)) = (sup, sub) {
					let sup_bottom = up - sup.descent();
					let sub_top = sub.ascent() - down;
					let gap = sup_bottom - sub_top;
					if gap >= gap_min {
						continue;
					}
					let increase = gap_min - gap;
					let sup_only = (sup_bottom_max_with_sub - sup_bottom).clamp(0.0, increase);
					let rest = (increase - sup_only) / 2.0;
					up += sup_only + rest;
					down += rest;
				}
			}
			(up, down)
		};
		// Limits' baselines.
		let t_shift = match &t {
			Some(t)	=> base.ascent() + (c.upper_limit_baseline_rise_min * size).max(c.upper_limit_gap_min * size + t.descent()),
			None	=> 0.0,
		};
		let b_shift = match &b {
			Some(b)	=> base.descent() + (c.lower_limit_baseline_drop_min * size).max(c.lower_limit_gap_min * size + b.ascent()),
			None	=> 0.0,
		};
		let ascent = base.ascent().max(tx_shift + asc(&tr)).max(tx_shift + asc(&tl)).max(t_shift + asc(&t));
		let descent = base.descent().max(bx_shift + desc(&br)).max(bx_shift + desc(&bl)).max(b_shift + desc(&b));
		let height = ascent + descent;
		let delta = base.italics_correction() / 2.0;
		let (t_pre, t_post) = match &t {
			Some(t)	=> { let h = (t.width() - base.width()) / 2.0; (h - delta, h + delta) }
			None	=> (0.0, 0.0),
		};
		let (b_pre, b_post) = match &b {
			Some(b)	=> { let h = (b.width() - base.width()) / 2.0; (h + delta, h - delta) }
			None	=> (0.0, 0.0),
		};
		let space_after = c.space_after_script * size;
		let tl_pre = match &tl {
			Some(f)	=> space_after + f.width() + math_kern(&base, f, tx_shift, Corner::TopLeft),
			None	=> 0.0,
		};
		let bl_pre = match &bl {
			Some(f)	=> space_after + f.width() + math_kern(&base, f, bx_shift, Corner::BottomLeft),
			None	=> 0.0,
		};
		let (tr_post, tr_kern) = match &tr {
			Some(f)	=> { let k = math_kern(&base, f, tx_shift, Corner::TopRight); (space_after + f.width() + k, k) }
			None	=> (0.0, 0.0),
		};
		let (br_post, br_kern) = match &br {
			Some(f)	=> {
				let k = math_kern(&base, f, bx_shift, Corner::BottomRight) - base.italics_correction();
				(space_after + f.width() + k, k)
			}
			None	=> (0.0, 0.0),
		};
		let pre = t_pre.max(b_pre).max(tl_pre).max(bl_pre);
		let base_w = base.width();
		let post = t_post.max(b_post).max(tr_post).max(br_post);
		let width = pre + base_w + post;
		let base_y = ascent - base.ascent();
		let mut frame = MFrame::new(width, height);
		frame.set_baseline(ascent);
		frame.push_frame(P2::new(pre, base_y), base.into_frame());
		let place = |f: Option<Frag>, x: f64, y_of: &dyn Fn(&Frag) -> f64, frame: &mut MFrame| {
			if let Some(f) = f {
				let y = y_of(&f);
				frame.push_frame(P2::new(x, y), f.into_frame());
			}
		};
		let tx_y = |f: &Frag| ascent - tx_shift - f.ascent();
		let bx_y = |f: &Frag| ascent + bx_shift - f.ascent();
		let t_y = |f: &Frag| ascent - t_shift - f.ascent();
		let b_y = |f: &Frag| ascent + b_shift - f.ascent();
		let _ = wid;
		place(tl, pre - tl_pre + space_after, &tx_y, &mut frame);
		place(bl, pre - bl_pre + space_after, &bx_y, &mut frame);
		place(tr, pre + base_w + tr_kern, &tx_y, &mut frame);
		place(br, pre + base_w + br_kern, &bx_y, &mut frame);
		place(t, pre - t_pre, &t_y, &mut frame);
		place(b, pre - b_pre, &b_y, &mut frame);
		self.push(Frag::Frame(FrameFrag::new(p, styles, frame)));
		Ok(())
	}

	// Fractions

	fn fraction(&mut self, num: &Item, denom: &Item, line: bool, padding: f64, styles: &StyleChain, p: &Props) -> Outcome<()> {
		let num = res!(self.layout_into_fragment(num, styles)).into_frame();
		let denom = res!(self.layout_into_fragment(denom, styles)).into_frame();
		let c = self.font().consts;
		let size = props::font_size(styles);
		let display = p.size == MathSize::Display;
		let pad = padding * size;
		let frame = if line {
			let axis = c.axis_height * size;
			let thickness = c.fraction_rule_thickness * size;
			let shift_up = (if display { c.fraction_numerator_display_style_shift_up } else { c.fraction_numerator_shift_up }) * size;
			let shift_down = (if display { c.fraction_denominator_display_style_shift_down } else { c.fraction_denominator_shift_down }) * size;
			let num_min = (if display { c.fraction_num_display_style_gap_min } else { c.fraction_numerator_gap_min }) * size;
			let denom_min = (if display { c.fraction_denom_display_style_gap_min } else { c.fraction_denominator_gap_min }) * size;
			let num_gap = (shift_up - (axis + thickness / 2.0) - num.descent()).max(num_min);
			let denom_gap = (shift_down + (axis - thickness / 2.0) - denom.ascent()).max(denom_min);
			let line_w = num.w.max(denom.w);
			let width = line_w + 2.0 * pad;
			let height = num.h + num_gap + thickness + denom_gap + denom.h;
			let line_pos = P2::new((width - line_w) / 2.0, num.h + num_gap + thickness / 2.0);
			let mut frame = MFrame::new(width, height);
			frame.set_baseline(line_pos.y + axis);
			let (nw, dw, dh) = (num.w, denom.w, denom.h);
			frame.push_frame(P2::new((width - nw) / 2.0, 0.0), num);
			frame.push_frame(P2::new((width - dw) / 2.0, height - dh), denom);
			let fill = res!(props::fill(styles));
			match res!(props::stroke(styles)) {
				Some(s) => frame.push(P2::new(line_pos.x, line_pos.y - thickness / 2.0),
					FItem::Rect { w: line_w, h: thickness, fill, pen: Some(pen(&s, size, res!(props::fill(styles)))) }),
				None => frame.push(line_pos, FItem::Line { to: P2::new(line_w, 0.0), pen: line_pen(fill, thickness) }),
			}
			frame
		} else {
			let shift_up = (if display { c.stack_top_display_style_shift_up } else { c.stack_top_shift_up }) * size;
			let shift_down = (if display { c.stack_bottom_display_style_shift_down } else { c.stack_bottom_shift_down }) * size;
			let gap_min = (if display { c.stack_display_style_gap_min } else { c.stack_gap_min }) * size;
			let gap = (shift_up - num.descent()) + (shift_down - denom.ascent());
			let width = num.w.max(denom.w) + 2.0 * pad;
			let height = num.h + gap.max(gap_min) + denom.h;
			let baseline = num.ascent() + shift_up + (gap_min - gap).max(0.0) / 2.0;
			let mut frame = MFrame::new(width, height);
			frame.set_baseline(baseline);
			let (nw, dw, dh) = (num.w, denom.w, denom.h);
			frame.push_frame(P2::new((width - nw) / 2.0, 0.0), num);
			frame.push_frame(P2::new((width - dw) / 2.0, height - dh), denom);
			frame
		};
		self.push(Frag::Frame(FrameFrag::new(p, styles, frame)));
		Ok(())
	}

	fn skewed(&mut self, num: &Item, denom: &Item, slash: &Item, styles: &StyleChain, p: &Props) -> Outcome<()> {
		let c = self.font().consts;
		let size = props::font_size(styles);
		let vgap = c.skewed_fraction_vertical_gap * size;
		let hgap = c.skewed_fraction_horizontal_gap * size;
		let axis = c.axis_height * size;
		let num = res!(self.layout_into_fragment(num, styles)).into_frame();
		let denom = res!(self.layout_into_fragment(denom, styles)).into_frame();
		let mut height = num.h + denom.h + vgap;
		slash.set_stretch_relative_to(height, true);
		let slash = res!(self.layout_into_fragment(slash, styles)).into_frame();
		let voff = (slash.h - height).max(0.0) / 2.0;
		height = height.max(slash.h);
		let mut slash_pos = P2::new(num.w + hgap / 2.0 - slash.w / 2.0, height / 2.0 - slash.h / 2.0);
		let mut num_pos = P2::new(0.0, voff);
		let mut denom_pos = P2::new(num_pos.x + num.w + hgap, num_pos.y + num.h + vgap);
		let width = (denom_pos.x + denom.w).max(slash_pos.x + slash.w) + (-slash_pos.x).max(0.0);
		let hoff = (-slash_pos.x).max(0.0);
		slash_pos.x += hoff;
		num_pos.x += hoff;
		denom_pos.x += hoff;
		let mut frame = MFrame::new(width, height);
		frame.set_baseline(height / 2.0 + axis);
		frame.push_frame(num_pos, num);
		frame.push_frame(denom_pos, denom);
		frame.push_frame(slash_pos, slash);
		self.push(Frag::Frame(FrameFrag::new(p, styles, frame)));
		Ok(())
	}

	// Radicals

	fn radical(&mut self, radicand: &Item, index: Option<&Item>, sqrt: &Item, styles: &StyleChain, p: &Props) -> Outcome<()> {
		let radicand = res!(self.layout_into_fragment(radicand, styles)).into_frame();
		let sq_styles = sqrt.styles().cloned().unwrap_or_else(|| styles.clone());
		let target = {
			let s = res!(self.layout_into_fragment(sqrt, styles));
			let (font, size) = s.font(&self.font(), &sq_styles);
			let thickness = font.consts.radical_rule_thickness * size;
			let gap = (if props::size(&sq_styles) == MathSize::Display {
				font.consts.radical_display_style_vertical_gap
			} else {
				font.consts.radical_vertical_gap
			}) * size;
			radicand.h + thickness + gap
		};
		sqrt.set_stretch_relative_to(target, true);
		let s = res!(self.layout_into_fragment(sqrt, styles));
		let (font, size) = s.font(&self.font(), &sq_styles);
		let c = font.consts;
		let thickness = c.radical_rule_thickness * size;
		let extra = c.radical_extra_ascender * size;
		let kern_before = c.radical_kern_before_degree * size;
		let kern_after = c.radical_kern_after_degree * size;
		let raise = c.radical_degree_bottom_raise_percent;
		let gap0 = (if props::size(&sq_styles) == MathSize::Display { c.radical_display_style_vertical_gap } else { c.radical_vertical_gap }) * size;
		let fill = match s.fill() {
			Some(f)	=> f,
			None	=> res!(props::fill(&sq_styles)),
		};
		let line_w = radicand.w;
		let sqrt_f = s.into_frame();
		let index = match index {
			Some(i)	=> Some(res!(self.layout_into_fragment(i, styles)).into_frame()),
			None	=> None,
		};
		let gap = gap0.max((sqrt_f.h - thickness - radicand.h + gap0) / 2.0);
		let sqrt_ascent = radicand.ascent() + gap + thickness;
		let descent = sqrt_f.h - sqrt_ascent;
		let inner_ascent = sqrt_ascent + extra;
		let mut offset = 0.0;
		let mut shift_up = 0.0;
		let mut ascent = inner_ascent;
		if let Some(i) = &index {
			offset = kern_before + i.w + kern_after;
			shift_up = raise * (inner_ascent - descent) + i.descent();
			ascent = ascent.max(shift_up + i.ascent());
		}
		let sqrt_x = offset.max(0.0);
		let radicand_x = sqrt_x + sqrt_f.w;
		let radicand_y = ascent - radicand.ascent();
		let mut frame = MFrame::new(radicand_x + line_w, ascent + descent);
		frame.set_baseline(ascent);
		if let Some(i) = index {
			let ix = -offset.min(0.0) + kern_before;
			let iy = ascent - i.ascent() - shift_up;
			frame.push_frame(P2::new(ix, iy), i);
		}
		frame.push_frame(P2::new(sqrt_x, radicand_y - gap - thickness), sqrt_f);
		let line_pos = P2::new(radicand_x, radicand_y - gap - thickness / 2.0);
		frame.push(line_pos, FItem::Line { to: P2::new(line_w, 0.0), pen: line_pen(fill, thickness) });
		frame.push_frame(P2::new(radicand_x, radicand_y), radicand);
		self.push(Frag::Frame(FrameFrag::new(p, styles, frame)));
		Ok(())
	}

	// Accents and lines

	fn accent(&mut self, base: &Item, accent: &Item, position: Position, exact: bool, styles: &StyleChain, p: &Props) -> Outcome<()> {
		let top = position == Position::Above;
		let bstyles = base.styles().cloned().unwrap_or_else(|| styles.clone());
		let base = res!(self.layout_into_fragment(base, styles));
		let (font, size) = base.font(&self.font(), &bstyles);
		let base_attach = base.accent_attach();
		if top && base.ascent() > font.consts.flattened_accent_base_height * size {
			accent.set_flac();
		}
		accent.set_stretch_relative_to(base.width(), false);
		accent.set_stretch_font_size(size, false);
		let acc = res!(self.layout_into_fragment(accent, styles));
		let acc_attach = acc.accent_attach().0;
		let acc = acc.into_frame();
		let battach = if top { base_attach.0 } else { base_attach.1 };
		let (width, base_x, acc_x) = if !exact {
			(base.width(), 0.0, battach - acc_attach)
		} else {
			let pre = acc_attach - battach;
			let post = (acc.w - acc_attach) - (base.width() - battach);
			let width = pre.max(0.0) + base.width() + post.max(0.0);
			if pre < 0.0 { (width, 0.0, -pre) } else { (width, pre, 0.0) }
		};
		let (gap, acc_pos, base_pos) = if top {
			let abh = font.consts.accent_base_height * size;
			let gap = -acc.descent() - base.ascent().min(abh);
			(gap, P2::new(acc_x, 0.0), P2::new(base_x, acc.h + gap))
		} else {
			let gap = -acc.ascent();
			(gap, P2::new(acc_x, base.height() + gap), P2::new(base_x, 0.0))
		};
		let height = acc.h + gap + base.height();
		let baseline = base_pos.y + base.ascent();
		let text_like = !exact && base.is_text_like();
		let italics = base.italics_correction();
		let (ba, bd) = (base.base_ascent(), base.base_descent());
		let mut frame = MFrame::new(width, height);
		frame.set_baseline(baseline);
		frame.push_frame(base_pos, base.into_frame());
		frame.push_frame(acc_pos, acc);
		let mut ff = FrameFrag::new(p, styles, frame);
		ff.base_ascent = ba;
		ff.base_descent = bd;
		ff.italics = italics;
		ff.text_like = text_like;
		ff.accent_attach = base_attach;
		self.push(Frag::Frame(ff));
		Ok(())
	}

	fn line(&mut self, base: &Item, position: Position, styles: &StyleChain, p: &Props) -> Outcome<()> {
		let bstyles = base.styles().cloned().unwrap_or_else(|| styles.clone());
		let content = res!(self.layout_into_fragment(base, styles));
		let (font, size) = content.font(&self.font(), &bstyles);
		let c = font.consts;
		let (extra, line_y, content_y, baseline, thickness, adjust) = match position {
			Position::Below => {
				let sep = c.underbar_extra_descender * size;
				let t = c.underbar_rule_thickness * size;
				let gap = c.underbar_vertical_gap * size;
				(sep + t + gap, content.height() + gap + t / 2.0, 0.0, content.ascent(), t, -content.italics_correction())
			}
			Position::Above => {
				let sep = c.overbar_extra_ascender * size;
				let t = c.overbar_rule_thickness * size;
				let gap = c.overbar_vertical_gap * size;
				let extra = sep + t + gap;
				(extra, sep + t / 2.0, extra, content.ascent() + extra, t, 0.0)
			}
		};
		let width = content.width();
		let height = content.height() + extra;
		let line_w = width + adjust;
		let text_like = content.is_text_like();
		let italics = content.italics_correction();
		let mut frame = MFrame::new(width, height);
		frame.set_baseline(baseline);
		frame.push_frame(P2::new(0.0, content_y), content.into_frame());
		let fill = res!(props::fill(styles));
		match res!(props::stroke(styles)) {
			Some(s) => frame.push(P2::new(0.0, line_y - thickness / 2.0),
				FItem::Rect { w: line_w, h: thickness, fill, pen: Some(pen(&s, size, res!(props::fill(styles)))) }),
			None => frame.push(P2::new(0.0, line_y), FItem::Line { to: P2::new(line_w, 0.0), pen: line_pen(fill, thickness) }),
		}
		let mut ff = FrameFrag::new(p, styles, frame);
		ff.italics = italics;
		ff.text_like = text_like;
		self.push(Frag::Frame(ff));
		Ok(())
	}

	fn cancel(&mut self, c: &Cancel, styles: &StyleChain, p: &Props) -> Outcome<()> {
		let body = res!(self.layout_into_fragment(&c.base, styles));
		let text_like = body.is_text_like();
		let italics = body.italics_correction();
		let attach = body.accent_attach();
		let mut frame = body.into_frame();
		let (w, h) = (frame.w, frame.h);
		let center = P2::new(w / 2.0, h / 2.0);
		res!(self.cancel_line(&mut frame, c, c.invert_first_line, center, styles, p.span));
		if c.cross {
			res!(self.cancel_line(&mut frame, c, true, center, styles, p.span));
		}
		let mut ff = FrameFrag::new(p, styles, frame);
		ff.italics = italics;
		ff.text_like = text_like;
		ff.accent_attach = attach;
		self.push(Frag::Frame(ff));
		Ok(())
	}

	fn cancel_line(&mut self, frame: &mut MFrame, c: &Cancel, invert: bool, center: P2, styles: &StyleChain, span: Span) -> Outcome<()> {
		let (w, h) = (frame.w, frame.h);
		let default = (w / h).atan();
		let mut angle = match &c.angle {
			Value::Angle(a)		=> a.0,
			Value::Func(f)		=> {
				let f: Func = f.clone();
				let mut args = crate::eval::args::Args::new(span);
				args.push(span, Value::Angle(Angle(default)));
				let saved = std::mem::replace(&mut self.engine.context.styles, Some(styles.clone()));
				let out = self.engine.call_func(&f, args);
				self.engine.context.styles = saved;
				match res!(out) {
					Value::Angle(a)	=> a.0,
					other			=> return Err(self.engine.error(DiagnosticKind::Type, span, fmt!(
						"expected angle, found {}", other.ty().long_name()))),
				}
			}
			_					=> default,
		};
		if invert {
			angle = -angle;
		}
		let length = c.length.relative_to(w.hypot(h));
		// A vertical segment of `length` centred on the origin, rotated clockwise from the y axis.
		let (s, co) = angle.sin_cos();
		let rot = |x: f64, y: f64| P2::new(x * co - y * s, x * s + y * co);
		let a = rot(0.0, length / 2.0);
		let b = rot(0.0, -length / 2.0);
		frame.push(P2::new(center.x + a.x, center.y + a.y), FItem::Line { to: P2::new(b.x - a.x, b.y - a.y), pen: c.stroke.clone() });
		Ok(())
	}

	// Fences

	fn fenced(&mut self, open: Option<&Item>, close: Option<&Item>, body: &FencedBody, balanced: bool, styles: &StyleChain) -> Outcome<()> {
		let (relative_to, initial) = match body {
			FencedBody::Shared { sizing, .. } => {
				let r = match sizing.relative_to.get() {
					Some(r)	=> r,
					None	=> {
						let mut m = 0.0f64;
						for it in &sizing.items {
							let frags = res!(self.layout_into_fragments(it, &sizing.styles));
							let st = it.styles().cloned().unwrap_or_else(|| sizing.styles.clone());
							m = m.max(self.relative_to(&frags, &st, balanced));
						}
						sizing.relative_to.set(Some(m));
						m
					}
				};
				(r, None)
			}
			FencedBody::Owned(item) => {
				let frags = res!(self.layout_into_fragments(item, styles));
				let st = item.styles().cloned().unwrap_or_else(|| styles.clone());
				let r = self.relative_to(&frags, &st, balanced);
				(r, Some(frags))
			}
		};
		let mut mid = false;
		for it in body.item().as_slice() {
			if it.mid_stretched() == Some(true) {
				mid = true;
				it.set_stretch_relative_to(relative_to, true);
			}
		}
		if let Some(o) = open {
			o.set_stretch_relative_to(relative_to, true);
			let f = res!(self.layout_into_fragment(o, styles));
			self.push(f);
		}
		let frags = match initial {
			Some(f) if !mid	=> f,
			_				=> res!(self.layout_into_fragments(body.item(), styles)),
		};
		self.frags.extend(frags);
		if let Some(c) = close {
			c.set_stretch_relative_to(relative_to, true);
			let f = res!(self.layout_into_fragment(c, styles));
			self.push(f);
		}
		Ok(())
	}

	fn relative_to(&self, frags: &[Frag], styles: &StyleChain, balanced: bool) -> f64 {
		let cur = self.font();
		let mut m = 0.0f64;
		for f in frags {
			let v = if balanced {
				let (font, size) = f.font(&cur, styles);
				let axis = font.consts.axis_height * size;
				2.0 * (f.ascent() - axis).max(f.descent() + axis)
			} else {
				f.height()
			};
			m = m.max(v);
		}
		m
	}

	// Rows and tables

	pub fn aligned_row(&mut self, row: &Row, styles: &StyleChain) -> Outcome<Vec<Run>> {
		let mut cells = Vec::with_capacity(row.len());
		for (c, item) in row.0.iter().enumerate() {
			let mut frags = res!(self.layout_into_fragments(item, styles));
			if c % 2 == 0 {
				if let Some(next) = row.0.get(c + 1) {
					if let Some(sp) = alignment_lspace(next) {
						frags.push(Frag::Space(sp));
					}
				}
			}
			cells.push(frags);
		}
		Ok(cells)
	}

	pub fn multiline(&mut self, rows: &[Row], styles: &StyleChain) -> Outcome<Builder> {
		let nrows = rows.len();
		let ncols = rows.first().map(|r| r.len()).unwrap_or(0);
		if nrows == 0 || ncols == 0 {
			return Ok(Builder::default());
		}
		let mut widths = vec![0.0f64; ncols];
		let mut laid = Vec::with_capacity(nrows);
		for row in rows {
			let cells = res!(self.aligned_row(row, styles));
			for (c, cell) in cells.iter().enumerate() {
				if let Some(w) = widths.get_mut(c) {
					*w = w.max(cell.iter().map(|f| f.width()).sum());
				}
			}
			laid.push(cells);
		}
		let leading = if props::size(styles) >= MathSize::Text {
			res!(props::leading(styles))
		} else {
			TIGHT_LEADING * props::font_size(styles)
		};
		let align = match res!(props::align_x(styles)) {
			Some(HAlign::Center)						=> Fixed::Center,
			Some(HAlign::Right) | Some(HAlign::End)		=> Fixed::End,
			_											=> Fixed::Start,
		};
		let layouts = laid.into_iter().map(|cells| {
			let h = measure_row(&cells);
			RowLayout { cells, frame_height: h, row_height: None }
		}).collect();
		Ok(stack_rows(layouts, &widths, Alternator::Right, align, leading, 0.0))
	}

	fn table(&mut self, t: &Table, styles: &StyleChain, p: &Props) -> Outcome<()> {
		let nrows = t.cells.len();
		let ncols = t.cells.first().map(|r| r.len()).unwrap_or(0);
		if nrows == 0 || ncols == 0 {
			self.push(Frag::Frame(FrameFrag::new(p, styles, MFrame::new(0.0, 0.0))));
			return Ok(());
		}
		let gap_x = t.gap.0.relative_to(self.region.0);
		let gap_y = t.gap.1.relative_to(self.region.1);
		let size = props::font_size(styles);
		let fill = res!(props::fill(styles));
		let mut default_pen = line_pen(fill.clone(), DEFAULT_STROKE_THICKNESS * size);
		default_pen.cap = LineCap::Square;
		let (mut hline, mut vline, stroke) = match &t.augment {
			Some(a) => {
				let s = match &a.stroke {
					Some(s) => {
						let mut base = crate::eval::value::Stroke::default();
						base.thickness = Some(crate::eval::value::Length::pt(DEFAULT_STROKE_THICKNESS * size));
						base.cap = Some(LineCap::Square);
						let merged = crate::eval::value::Stroke {
							paint:			s.paint.clone().or(base.paint),
							thickness:		s.thickness.or(base.thickness),
							cap:			s.cap.or(base.cap),
							join:			s.join.or(base.join),
							dash:			s.dash.clone().or(base.dash),
							miter_limit:	s.miter_limit.or(base.miter_limit),
						};
						pen(&merged, size, fill.clone())
					}
					None => default_pen.clone(),
				};
				(a.hline.clone(), a.vline.clone(), s)
			}
			None => (Vec::new(), Vec::new(), default_pen.clone()),
		};
		let den = props::chain_with(styles, props::for_denominator(styles));
		let (pa, pd) = match res!(GlyphFrag::synthetic(&den, '(', Span::detached())) {
			Some(g)	=> (g.ascent(), g.descent()),
			None	=> (0.0, 0.0),
		};
		let mut cols: Vec<Vec<(Vec<Run>, (f64, f64))>> = (0..ncols).map(|_| Vec::with_capacity(nrows)).collect();
		let mut heights = vec![(0.0f64, 0.0f64); nrows];
		for (r, row) in t.cells.iter().enumerate() {
			for (c, cell) in row.iter().enumerate() {
				let subs = res!(self.aligned_row(cell, styles));
				let h = measure_row(&subs);
				heights[r].0 = heights[r].0.max(h.0.max(pa));
				heights[r].1 = heights[r].1.max(h.1.max(pd));
				if let Some(col) = cols.get_mut(c) {
					col.push((subs, h));
				}
			}
		}
		for l in hline.iter_mut() {
			if *l < 0 {
				*l += nrows as i64;
			}
		}
		for l in vline.iter_mut() {
			if *l < 0 {
				*l += ncols as i64;
			}
		}
		let mut total_h: f64 = heights.iter().map(|(a, b)| a + b).sum::<f64>() + gap_y * (nrows as f64 - 1.0);
		if hline.contains(&0) {
			total_h += gap_y;
		}
		if hline.contains(&(nrows as i64)) {
			total_h += gap_y;
		}
		let mut frame = MFrame::new(0.0, total_h);
		let mut x = 0.0;
		let vert = |len: f64, pen: &Pen| FItem::Line { to: P2::new(0.0, len), pen: pen.clone() };
		if vline.contains(&0) {
			frame.push(P2::new(x + gap_x / 2.0, 0.0), vert(total_h, &stroke));
			x += gap_x;
		}
		for (index, col) in cols.into_iter().enumerate() {
			let widths = sub_column_widths(&col);
			let layouts = col.into_iter().enumerate().map(|(r, (subs, h))| RowLayout {
				cells:			subs,
				frame_height:	h,
				row_height:		Some(heights[r]),
			}).collect();
			let start = if hline.contains(&0) { gap_y } else { 0.0 };
			let b = stack_rows(layouts, &widths, t.alternator, t.align, gap_y, start);
			for (cell, mut pos) in b.frames {
				pos.x += x;
				frame.push_frame(pos, cell);
			}
			x += b.w;
			if vline.contains(&(index as i64 + 1)) {
				frame.push(P2::new(x + gap_x / 2.0, 0.0), vert(total_h, &stroke));
			}
			x += gap_x;
		}
		let total_w = if !vline.contains(&(ncols as i64)) { x - gap_x } else { x };
		for l in hline {
			let off = if l == 0 {
				gap_y
			} else {
				heights[0..(l as usize).min(nrows)].iter().map(|(a, b)| a + b).sum::<f64>() + gap_y * (l as f64 - 1.0) + gap_y / 2.0
			};
			frame.push(P2::new(0.0, off), FItem::Line { to: P2::new(total_w, 0.0), pen: stroke.clone() });
		}
		frame.w = total_w;
		let axis = self.font().consts.axis_height * size;
		let h = frame.h;
		frame.set_baseline(h / 2.0 + axis);
		self.push(Frag::Frame(FrameFrag::new(p, styles, frame)));
		Ok(())
	}
}

fn sp(pt: f64) -> crate::ir::Sp {
	if !pt.is_finite() || pt >= (1u64 << 30) as f64 / 65536.0 { crate::ir::Sp(i32::MAX) } else { crate::ir::Sp::from_pt(pt) }
}

// Cut-in kerning of a script against its base (OpenType MATH, MathKernInfo), the larger of the two
// correction heights' sums.
fn math_kern(base: &Frag, script: &Frag, shift: f64, corner: Corner) -> f64 {
	let (top, bot) = match corner {
		Corner::TopLeft | Corner::TopRight		=> (base.ascent() - shift, shift - script.descent()),
		Corner::BottomLeft | Corner::BottomRight	=> (script.ascent() - shift, shift - base.descent()),
	};
	let sum = |h: f64| base.kern_at_height(corner, h) + script.kern_at_height(corner.inv(), h);
	sum(top).max(sum(bot))
}

fn alignment_lspace(cell: &Item) -> Option<f64> {
	cell.as_slice().iter().find(|i| !matches!(i, Item::Tag(_))).and_then(|i| match i {
		Item::Comp(c) if c.props.align_form_infix => c.props.lspace.map(|l| l * props::font_size(&c.styles)),
		_ => None,
	})
}

fn sub_column_widths(col: &[(Vec<Run>, (f64, f64))]) -> Vec<f64> {
	let n = col.iter().map(|(s, _)| s.len()).max().unwrap_or(1);
	let mut w = vec![0.0f64; n];
	for (subs, _) in col {
		for (i, s) in subs.iter().enumerate() {
			if let Some(x) = w.get_mut(i) {
				*x = x.max(s.iter().map(|f| f.width()).sum());
			}
		}
	}
	w
}

/// A row to stack: its cells and the ascent and descent its frame and its slot take.
pub struct RowLayout {
	pub cells:			Vec<Run>,
	pub frame_height:	(f64, f64),
	pub row_height:		Option<(f64, f64)>,
}

/// Rows stacked into one frame's worth of positioned line frames.
#[derive(Default)]
pub struct Builder {
	pub w:		f64,
	pub h:		f64,
	pub frames:	Vec<(MFrame, P2)>,
}

impl Builder {
	/// The frame, its baseline the first row's when `aligned`.
	pub fn build(self, aligned: bool) -> MFrame {
		let mut f = MFrame::new(self.w, self.h);
		let mut set = aligned;
		for (sub, pos) in self.frames {
			if set && sub.has_baseline() {
				f.set_baseline(sub.baseline());
			}
			f.push_frame(pos, sub);
			set = false;
		}
		f
	}
}

pub fn measure_row(cells: &[Run]) -> (f64, f64) {
	let mut out: Option<(f64, f64)> = None;
	for f in cells.iter().flat_map(|c| c.iter()) {
		if matches!(f, Frag::Tag(_)) {
			continue;
		}
		let (a, d) = (f.ascent(), f.descent());
		out = Some(match out {
			None			=> (a, d),
			Some((ma, md))	=> (ma.max(a), md.max(d)),
		});
	}
	out.unwrap_or((0.0, 0.0))
}

pub fn stack_rows(rows: Vec<RowLayout>, widths: &[f64], alternator: Alternator, align: Fixed, leading: f64, start_y: f64) -> Builder {
	let (points, total) = cumulative(widths);
	let has_alignment = !points.is_empty();
	let mut frames = Vec::new();
	let mut w = 0.0f64;
	let mut h = start_y;
	for (i, row) in rows.into_iter().enumerate() {
		let rh = row.row_height.unwrap_or(row.frame_height);
		let sub = line_frame(row.cells, &points, alternator, Some(row.frame_height));
		if i > 0 {
			h += leading;
		}
		let mut pos = P2::new(0.0, h + rh.0 - sub.ascent());
		if !has_alignment {
			pos.x = align.position(total - sub.w);
		}
		w = w.max(sub.w);
		h += rh.0 + rh.1;
		frames.push((sub, pos));
	}
	Builder { w, h, frames }
}

fn cumulative(widths: &[f64]) -> (Vec<f64>, f64) {
	if widths.len() <= 1 {
		return (Vec::new(), widths.first().copied().unwrap_or(0.0));
	}
	let mut points = Vec::with_capacity(widths.len());
	let mut acc = 0.0;
	for w in widths {
		acc += w;
		points.push(acc);
	}
	(points, acc)
}

/// Cells placed at alignment points on one baseline.
pub fn line_frame(cells: Vec<Run>, points: &[f64], mut alternator: Alternator, height: Option<(f64, f64)>) -> MFrame {
	let (ascent, descent) = height.unwrap_or_else(|| measure_row(&cells));
	let mut frame = MFrame::new(0.0, ascent + descent);
	frame.set_baseline(ascent);
	let mut prev = 0.0;
	let mut pts = points.iter().copied();
	let mut x_end = 0.0;
	for cell in cells {
		let width: f64 = cell.iter().map(|f| f.width()).sum();
		let cell_x = match pts.next() {
			Some(pt) => {
				let x = match alternator.next() {
					Alternator::Right	=> pt - width,
					_					=> prev,
				};
				prev = pt;
				x
			}
			None => prev,
		};
		let mut x = cell_x;
		for f in cell {
			let y = ascent - f.ascent();
			let w = f.width();
			frame.push_frame(P2::new(x, y), f.into_frame());
			x += w;
		}
		x_end = x;
	}
	frame.w = x_end;
	frame
}

/// A run of fragments as one line frame.
pub fn into_frame(run: Run) -> MFrame {
	line_frame(vec![run], &[], Alternator::Right, None)
}

// The dotted letter behind a dotless one, shaped with `dtls` where the font offers it.
fn dotted(c: char) -> Option<char> {
	match c {
		'\u{131}' | '\u{1d6a4}'	=> Some('i'),
		'\u{237}' | '\u{1d6a5}'	=> Some('j'),
		_						=> None,
	}
}
