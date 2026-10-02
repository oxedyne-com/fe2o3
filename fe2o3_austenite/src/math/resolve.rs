// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-library `math/ir/resolve.rs`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
//! Realised maths content into IR items: a port of Typst 0.15's `math/ir/resolve.rs`. Each element's
//! body is realised in maths mode and resolved in turn, so show rules reach every level; the styles an
//! element sets for its parts (script size, cramped style, the alphabet) are chained as it descends.

use crate::diag::DiagnosticKind;
use crate::eval::content::{
	Content,
	ElemKind,
	FieldId,
};
use crate::eval::realise::{
	realise,
	Pair,
	RealiseMode,
};
use crate::eval::content::display;
use crate::eval::styles::{
	to_stroke,
	StyleChain,
};
use crate::eval::value::{
	Length,
	Ratio,
	Relative,
	Value,
};
use crate::eval::Engine;
use crate::math::class::{
	default_math_class,
	MathClass,
};
use crate::math::item::{
	Alternator,
	Augment,
	Cancel,
	Fixed,
	FencedBody,
	Glyph,
	Item,
	Kind,
	Limits,
	Position,
	Props,
	Raw,
	Rel,
	Row,
	Scripts,
	SharedSizing,
	Stretch,
	StretchInfo,
	Table,
};
use crate::math::process::{
	expand_multiline_fence,
	process_group,
	process_table_cell,
	Group,
};
use crate::math::props;
use crate::math::style::{
	styled,
	MathSize,
};

use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::unicode::property::Binary;
use oxedyne_fe2o3_text::unicode::segment::graphemes;

use std::cell::Cell;

pub const THIN:				f64 = 1.0 / 6.0;
pub const MEDIUM:			f64 = 2.0 / 9.0;
pub const THICK:			f64 = 5.0 / 18.0;
pub const DELIM_SHORT_FALL:	f64 = 0.1;
pub const ACCENT_SHORT_FALL:	f64 = 0.5;
pub const FRAC_PADDING:		f64 = 0.1;
const ROW_GAP:				f64 = 0.2;	// ems, vec, mat and cases
const COL_GAP:				f64 = 0.5;	// ems, mat

/// Builds IR from realised maths content.
pub struct Resolver<'e> {
	pub engine:	&'e mut Engine,
	items:		Vec<Raw>,
}

impl<'e> Resolver<'e> {
	pub fn new(engine: &'e mut Engine) -> Self { Self { engine, items: Vec::new() } }

	fn push(&mut self, item: Item) { self.items.push(Raw::Item(item)); }

	/// Resolves content, returning where its items start in the buffer.
	fn resolve_into_items(&mut self, content: &Content, styles: &StyleChain) -> Outcome<usize> {
		let start = self.items.len();
		let pairs = res!(realise(self.engine, content, styles, RealiseMode::Math));
		for pair in pairs {
			res!(self.realised(pair));
		}
		Ok(start)
	}

	/// Resolves content into one item: the item itself, a group, or a multiline item.
	pub fn resolve_into_item(&mut self, content: &Content, styles: &StyleChain) -> Outcome<Item> {
		let start = res!(self.resolve_into_items(content, styles));
		let len = self.items.len() - start;
		if len == 1 && matches!(self.items.last(), Some(Raw::Item(_))) {
			if let Some(Raw::Item(i)) = self.items.pop() {
				return Ok(i);
			}
		}
		let drained: Vec<Raw> = self.items.drain(start..).collect();
		Ok(match process_group(drained, styles, false, true, false) {
			Group::Multiline(rows)	=> multiline(rows, styles),
			Group::Flat(items)		=> Item::wrap(items, styles),
		})
	}

	fn realised(&mut self, pair: Pair) -> Outcome<()> {
		if let Some(tag) = pair.tag {
			self.push(Item::Tag(tag));
			return Ok(());
		}
		let elem = &pair.content;
		let styles = &pair.styles;
		let kind = match elem.kind() {
			Some(k)	=> k,
			None	=> return Ok(()),
		};
		match kind {
			ElemKind::Text				=> self.text(elem, styles, false),
			ElemKind::Symbol			=> self.text(elem, styles, true),
			ElemKind::Space				=> { self.push(Item::Space); Ok(()) }
			ElemKind::MathAttach		=> self.attach(elem, styles),
			ElemKind::MathLr			=> self.lr(elem, styles),
			ElemKind::MathOp			=> self.op(elem, styles),
			ElemKind::H					=> self.h(elem, styles),
			ElemKind::MathOverline		=> self.overline(elem, styles),
			ElemKind::MathAlignPoint	=> { self.items.push(Raw::Align); Ok(()) }
			ElemKind::MathPrimes		=> self.primes(elem, styles),
			ElemKind::MathClass			=> self.class(elem, styles),
			ElemKind::Linebreak			=> { self.items.push(Raw::Linebreak); Ok(()) }
			ElemKind::MathFrac			=> self.frac(elem, styles),
			ElemKind::MathAccent		=> self.accent(elem, styles),
			ElemKind::MathLimits		=> self.limits(elem, styles),
			ElemKind::MathStretch		=> self.stretch(elem, styles),
			ElemKind::MathRoot			=> self.root(elem, styles),
			ElemKind::MathMat			=> self.mat(elem, styles),
			ElemKind::MathMid			=> self.mid(elem, styles),
			ElemKind::MathUnderline		=> self.underline(elem, styles),
			ElemKind::MathCases			=> self.cases(elem, styles),
			ElemKind::MathScripts		=> self.scripts(elem, styles),
			ElemKind::MathCancel		=> self.cancel(elem, styles),
			ElemKind::MathVec			=> self.vec(elem, styles),
			ElemKind::MathBinom			=> self.binom(elem, styles),
			ElemKind::MathUnderbrace	=> self.spreader(elem, styles, '\u{23df}', Position::Below),
			ElemKind::MathOverbrace		=> self.spreader(elem, styles, '\u{23de}', Position::Above),
			ElemKind::MathUnderbracket	=> self.spreader(elem, styles, '\u{23b5}', Position::Below),
			ElemKind::MathOverbracket	=> self.spreader(elem, styles, '\u{23b4}', Position::Above),
			ElemKind::MathUnderparen	=> self.spreader(elem, styles, '\u{23dd}', Position::Below),
			ElemKind::MathOverparen		=> self.spreader(elem, styles, '\u{23dc}', Position::Above),
			ElemKind::MathUndershell	=> self.spreader(elem, styles, '\u{23e1}', Position::Below),
			ElemKind::MathOvershell		=> self.spreader(elem, styles, '\u{23e0}', Position::Above),
			ElemKind::Box				=> {
				let props = Props { spaced: true, ..Props::new(styles, None, elem.span()) };
				self.push(Item::comp(Kind::External(elem.clone()), props, styles.clone()));
				Ok(())
			}
			_ => {
				let props = Props {
					spaced:		true,
					ignorant:	kind == ElemKind::Place,
					..Props::new(styles, None, elem.span())
				};
				self.push(Item::comp(Kind::External(elem.clone()), props, styles.clone()));
				Ok(())
			}
		}
	}

	// Text and symbols

	fn text(&mut self, elem: &Content, styles: &StyleChain, symbol: bool) -> Outcome<()> {
		let text = match elem.get(FieldId(0)) {
			Some(Value::Str(s))	=> s.to_string(),
			_					=> return Ok(()),
		};
		if symbol {
			self.symbol(&text, elem.span(), styles)
		} else {
			self.text_elem(&text, elem.span(), styles)
		}
	}

	fn text_elem(&mut self, text: &str, span: Span, styles: &StyleChain) -> Outcome<()> {
		let variant	= props::variant(styles);
		let bold	= props::bold(styles);
		let italic	= props::italic(styles).or(Some(false));
		let item_of = |t: &str| -> Item {
			let s = styled(t, variant, bold, italic);
			if is_number(t) {
				Item::comp(Kind::Number(s), Props::new(styles, None, span), styles.clone())
			} else {
				let props = Props { spaced: true, ..Props::new(styles, Some(MathClass::Alphabetic), span) };
				Item::comp(Kind::Text(s), props, styles.clone())
			}
		};
		let mut lines: Vec<&str> = split_newlines(text);
		if lines.last().map(|l| l.is_empty()).unwrap_or(false) {
			lines.pop();
		}
		let item = if lines.len() == 1 {
			item_of(lines[0])
		} else {
			let rows = lines.into_iter().map(|l| Row(vec![item_of(l)])).collect();
			multiline(rows, styles).with_multiline_centering()
		};
		self.push(item);
		Ok(())
	}

	fn symbol(&mut self, text: &str, span: Span, styles: &StyleChain) -> Outcome<()> {
		let variant	= props::variant(styles);
		let bold	= props::bold(styles);
		let italic	= props::italic(styles);
		let ignorable = Binary::find("Default_Ignorable_Code_Point");
		for cluster in graphemes(text) {
			if let Some(ig) = ignorable {
				if cluster.chars().all(|c| ig.contains(c)) {
					continue;
				}
			}
			let item = glyph(styled(cluster, variant, bold, italic), styles, span);
			if item.class() == MathClass::Large && item.size() == Some(MathSize::Display) {
				item.replace_stretch(Stretch::new().with_y(StretchInfo::default()));
			}
			self.push(item);
		}
		Ok(())
	}

	/// A single character resolved as a symbol under `styles`.
	fn symbol_item(&mut self, c: char, span: Span, styles: &StyleChain) -> Outcome<Item> {
		let content = Content::symbol(&c.to_string()).with_span(span);
		self.resolve_into_item(&content, styles)
	}

	// Spacing

	fn h(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<()> {
		let amount = res!(self.f(elem, styles, "amount"));
		let weak = matches!(res!(self.f(elem, styles, "weak")), Some(Value::Bool(true)));
		let len = match amount {
			Some(Value::Length(l))								=> Some(l),
			Some(Value::Relative(r)) if r.rel.0 == 0.0			=> Some(r.abs),
			Some(Value::Ratio(r)) if r.0 == 0.0				=> Some(Length::zero()),
			_													=> None,
		};
		if let Some(l) = len {
			self.push(Item::Spacing(props::resolve(styles, l), weak));
		}
		Ok(())
	}

	// Attachments

	fn attach(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<()> {
		let mut outer: [Vec<Option<Content>>; 6] = Default::default();
		let item = res!(self.inner_attach(elem, &mut outer, false, styles));
		self.push(item);
		Ok(())
	}

	// Typst merges nested attachments inward (`attach(attach(a, t: 1), b: 2)` is `attach(a, t: 1, b: 2)`)
	// through a linked list of each position's outer attachments; here each position keeps a stack,
	// outermost first, and an inner level takes the nearest free one.
	fn inner_attach(
		&mut self,
		elem:		&Content,
		outer:		&mut [Vec<Option<Content>>; 6],
		outer_t_inside_tr:	bool,
		styles:		&StyleChain,
	)
		-> Outcome<Item>
	{
		let sup = props::chain_with(styles, vec![props::for_superscript(styles)]);
		let sub = props::chain_with(styles, props::for_subscript(styles));
		let own_t	= res!(self.script(elem, "t", &sup));
		let own_tr	= res!(self.script(elem, "tr", &sup));
		let no_inner_tr = own_tr.is_none();
		let t_inside_tr = no_inner_tr && (own_t.is_some() || outer_t_inside_tr);
		// Positions: t, b, tl, tr, bl, br.
		let own = [
			own_t,
			res!(self.script(elem, "b", &sub)),
			res!(self.script(elem, "tl", &sup)),
			own_tr,
			res!(self.script(elem, "bl", &sub)),
			res!(self.script(elem, "br", &sub)),
		];
		for (i, o) in own.into_iter().enumerate() {
			let merged = match o {
				Some(c)	=> Some(c),
				None	=> take_outer(&mut outer[i]),
			};
			outer[i].push(merged);
		}
		let mut base = res!(self.c(elem, styles, "base"));
		while base.is(ElemKind::Equation) {
			match base.field("body") {
				Some(Value::Content(b))	=> base = b.clone(),
				_						=> break,
			}
		}
		let base_item = if base.is(ElemKind::MathAttach) {
			res!(self.inner_attach(&base, outer, t_inside_tr, styles))
		} else {
			res!(self.resolve_into_item(&base, styles))
		};
		let mut mine: Vec<Option<Content>> = Vec::with_capacity(6);
		for o in outer.iter_mut() {
			mine.push(o.pop().flatten());
		}
		let (t, b, tl, tr, bl, br) = (mine[0].take(), mine[1].take(), mine[2].take(), mine[3].take(),
			mine[4].take(), mine[5].take());
		if t.is_none() && b.is_none() && tl.is_none() && tr.is_none() && bl.is_none() && br.is_none() {
			return Ok(base_item);
		}
		let limits = base_item.limits().active(styles);
		let (t, tr) = match (t, tr) {
			(Some(t), Some(tr)) if !limits => {
				let primed = tr.is(ElemKind::MathPrimes);
				if primed && no_inner_tr && t_inside_tr {
					// A primed tr brought inward goes back out, so `tr + t` does not invert the order.
					unmerge(&mut outer[3], tr);
					(None, Some(t))
				} else if primed {
					(None, Some(Content::sequence(vec![tr, t])))
				} else {
					(Some(t), Some(tr))
				}
			}
			(Some(t), None) if !limits	=> (None, Some(t)),
			(t, tr)						=> (t, tr),
		};
		let (b, br) = if limits || br.is_some() { (b, br) } else { (None, b) };
		let lay = |c: Option<Content>, chain: &StyleChain, me: &mut Self| -> Outcome<Option<Item>> {
			match c {
				Some(c)	=> Ok(Some(res!(me.resolve_into_item(&c, chain)))),
				None	=> Ok(None),
			}
		};
		let top		= res!(lay(t, &sup, self));
		let bottom	= res!(lay(b, &sub, self));
		let tl		= res!(lay(tl, &sup, self));
		let bl		= res!(lay(bl, &sub, self));
		let tr		= res!(lay(tr, &sup, self));
		let br		= res!(lay(br, &sub, self));
		Ok(scripts(base_item, top, bottom, tl, bl, tr, br, styles))
	}

	fn primes(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<()> {
		let count = match res!(self.f(elem, styles, "count")) {
			Some(Value::Int(n))	=> n.max(0) as usize,
			_					=> 1,
		};
		let c = match count {
			1	=> Some('\u{2032}'),
			2	=> Some('\u{2033}'),
			3	=> Some('\u{2034}'),
			4	=> Some('\u{2057}'),
			_	=> None,
		};
		match c {
			Some(c) => {
				let item = res!(self.symbol_item(c, elem.span(), styles));
				self.push(item);
			}
			None => self.push(Item::comp(Kind::Primes(count), Props::new(styles, None, Span::detached()), styles.clone())),
		}
		Ok(())
	}

	fn scripts(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<()> {
		let mut item = { let tmp = res!(self.body(elem, styles)); res!(self.resolve_into_item(&tmp, styles)) };
		item.set_limits(Limits::Never);
		self.push(item);
		Ok(())
	}

	fn limits(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<()> {
		let mut item = { let tmp = res!(self.body(elem, styles)); res!(self.resolve_into_item(&tmp, styles)) };
		let inline = !matches!(res!(self.f(elem, styles, "inline")), Some(Value::Bool(false)));
		item.set_limits(if inline { Limits::Always } else { Limits::Display });
		self.push(item);
		Ok(())
	}

	fn stretch(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<()> {
		let item = { let tmp = res!(self.body(elem, styles)); res!(self.resolve_into_item(&tmp, styles)) };
		let size = relative(res!(self.f(elem, styles, "size"))).unwrap_or(ONE);
		item.update_stretch(StretchInfo::from_size(&size, 0.0, props::font_size(styles)));
		self.push(item);
		Ok(())
	}

	fn cancel(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<()> {
		let base = { let tmp = res!(self.body(elem, styles)); res!(self.resolve_into_item(&tmp, styles)) };
		let fs = props::font_size(styles);
		let length = match relative(res!(self.f(elem, styles, "length"))) {
			Some(r)	=> Rel::of(&r, fs),
			None	=> Rel::new(1.0, 0.3 * fs),
		};
		let mut stroke = match res!(self.f(elem, styles, "stroke")) {
			Some(v)	=> to_stroke(&v).unwrap_or_default(),
			None	=> crate::eval::value::Stroke::default(),
		};
		if stroke.thickness.is_none() {
			stroke.thickness = Some(Length::em(0.05));
		}
		let pen = crate::math::frame::pen(&stroke, fs, res!(props::fill(styles)));
		let invert	= matches!(res!(self.f(elem, styles, "inverted")), Some(Value::Bool(true)));
		let cross	= matches!(res!(self.f(elem, styles, "cross")), Some(Value::Bool(true)));
		let angle	= res!(self.f(elem, styles, "angle")).unwrap_or(Value::Auto);
		let class = base.raw_class();
		let cancel = Cancel { base, length, stroke: pen, cross, invert_first_line: !cross && invert, angle };
		self.push(Item::comp(Kind::Cancel(Box::new(cancel)), Props::new(styles, class, elem.span()), styles.clone()));
		Ok(())
	}

	// Fractions

	fn frac(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<()> {
		let num		= res!(self.c(elem, styles, "num"));
		let denom	= res!(self.c(elem, styles, "denom"));
		let style = match res!(self.f(elem, styles, "style")) {
			Some(Value::Str(s))	=> s.to_string(),
			_					=> "vertical".to_string(),
		};
		match style.as_str() {
			"skewed"		=> self.skewed_frac(&num, &denom, elem.span(), styles),
			"horizontal"	=> {
				let np = matches!(res!(self.f(elem, styles, "num-deparenthesized")), Some(Value::Bool(true)));
				let dp = matches!(res!(self.f(elem, styles, "denom-deparenthesized")), Some(Value::Bool(true)));
				self.horizontal_frac(num, denom, np, dp, elem.span(), styles)
			}
			_ => self.vertical_frac(&num, &[denom], false, elem.span(), styles),
		}
	}

	fn binom(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<()> {
		let upper = res!(self.c(elem, styles, "upper"));
		let lower = res!(self.children(elem, styles, "lower"));
		self.vertical_frac(&upper, &lower, true, elem.span(), styles)
	}

	fn vertical_frac(
		&mut self,
		num:	&Content,
		denom:	&[Content],
		binom:	bool,
		span:	Span,
		styles:	&StyleChain,
	)
		-> Outcome<()>
	{
		let num_styles = props::chain_with(styles, vec![props::for_numerator(styles)]);
		let den_styles = props::chain_with(styles, props::for_denominator(styles));
		let numerator = res!(self.resolve_into_item(num, &num_styles));
		let mut parts = Vec::new();
		for (i, d) in denom.iter().enumerate() {
			if i > 0 {
				parts.push(Content::symbol(",").with_span(span));
			}
			parts.push(d.clone());
		}
		let denominator = res!(self.resolve_into_item(&Content::sequence(parts), &den_styles));
		let frac = Item::comp(Kind::Fraction { num: numerator, denom: denominator, line: !binom, padding: FRAC_PADDING },
			Props::new(styles, None, span), styles.clone());
		if binom {
			let stretch = Stretch::new().with_y(StretchInfo::new(Rel::one(), DELIM_SHORT_FALL));
			let open = res!(self.symbol_item('(', span, styles));
			open.set_stretch(stretch);
			let close = res!(self.symbol_item(')', span, styles));
			close.set_stretch(stretch);
			self.push(fenced(Some(open), Some(close), FencedBody::Owned(frac), false, styles, span));
		} else {
			self.push(frac);
		}
		Ok(())
	}

	fn horizontal_frac(
		&mut self,
		num:	Content,
		denom:	Content,
		num_deparen:	bool,
		denom_deparen:	bool,
		span:	Span,
		styles:	&StyleChain,
	)
		-> Outcome<()>
	{
		let paren = |c: Content| -> Content {
			let body = Content::sequence(vec![Content::symbol("("), c, Content::symbol(")")]);
			match ElemKind::MathLr.field_id("body") {
				Some(id)	=> Content::new(ElemKind::MathLr, vec![(id, Value::Content(body))], span),
				None		=> body,
			}
		};
		let num = if num_deparen { paren(num) } else { num };
		let n = res!(self.resolve_into_item(&num, styles));
		self.push(n);
		let mut slash = res!(self.symbol_item('/', span, styles));
		slash.set_class(MathClass::Binary);
		slash.set_lspace(Some(0.0));
		slash.set_rspace(Some(0.0));
		self.push(slash);
		let denom = if denom_deparen { paren(denom) } else { denom };
		let d = res!(self.resolve_into_item(&denom, styles));
		self.push(d);
		Ok(())
	}

	fn skewed_frac(&mut self, num: &Content, denom: &Content, span: Span, styles: &StyleChain) -> Outcome<()> {
		let num_styles = props::chain_with(styles, vec![props::for_numerator(styles)]);
		let den_styles = props::chain_with(styles, props::for_denominator(styles));
		let numerator	= res!(self.resolve_into_item(num, &num_styles));
		let denominator	= res!(self.resolve_into_item(denom, &den_styles));
		let slash = res!(self.symbol_item('\u{2044}', span, styles));
		slash.set_stretch(Stretch::new().with_y(StretchInfo::new(Rel::one(), DELIM_SHORT_FALL)));
		self.push(Item::comp(Kind::Skewed { num: numerator, denom: denominator, slash },
			Props::new(styles, None, span), styles.clone()));
		Ok(())
	}

	// Delimiters

	fn lr(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<()> {
		let mut body = res!(self.c(elem, styles, "body"));
		if body.is(ElemKind::Equation) {
			if let Some(Value::Content(b)) = body.field("body") {
				body = b.clone();
			}
		}
		if body.is(ElemKind::MathLr) {
			let inner_one = relative(res!(self.f(&body, styles, "size"))).map(|r| crate::math::item::is_one(&r)).unwrap_or(true);
			if inner_one {
				body = res!(self.c(&body, styles, "body"));
			}
		}
		let start = res!(self.resolve_into_items(&body, styles));
		// Leading and trailing ignorant items stay outside the fence.
		let slice = &self.items[start..];
		let lead = slice.iter().take_while(|r| r.is_ignorant()).count();
		let trail = slice.iter().rev().take_while(|r| r.is_ignorant()).count();
		let (s_idx, e_idx) = if lead == slice.len() { (lead, lead) } else { (lead, slice.len() - trail) };
		let lo = start + s_idx;
		let hi = start + e_idx;
		let size = relative(res!(self.f(elem, styles, "size"))).unwrap_or(ONE);
		let fs = props::font_size(styles);
		let stretch = Stretch::new().with_y(StretchInfo::from_size(&size, DELIM_SHORT_FALL, fs));
		let n = hi - lo;
		if n == 1 {
			if let Some(Raw::Item(one)) = self.items.get(lo) {
				let mut done = false;
				if let Item::Comp(c) = one {
					if let Kind::Fenced { open, close, body: fbody, .. } = &c.kind {
						if let Some(o) = open { o.set_stretch(stretch); }
						if let Some(cl) = close { cl.set_stretch(stretch); }
						for it in fbody.item().as_slice() {
							if it.mid_stretched() == Some(true) {
								it.set_stretch(stretch);
							}
						}
						done = true;
					}
				}
				if !done {
					let mut info = StretchInfo::new(Rel::new(size.rel.0, size.abs.resolve(fs)), DELIM_SHORT_FALL);
					if !crate::math::item::is_one(&size) {
						info.requested_target = Some(size);
					}
					one.set_y_stretch(info);
				}
			}
			return Ok(());
		}
		if n >= 2 {
			let is_delim = |i: &Item| matches!(i.class(), MathClass::Opening | MathClass::Closing | MathClass::Fence);
			if let Some(Raw::Item(first)) = self.items.get_mut(lo) {
				if is_delim(first) {
					first.set_stretch(stretch);
					first.set_class(MathClass::Opening);
				}
			}
			if let Some(Raw::Item(last)) = self.items.get_mut(hi - 1) {
				if is_delim(last) {
					last.set_stretch(stretch);
					last.set_class(MathClass::Closing);
				}
			}
		}
		for r in self.items[lo..hi].iter() {
			if let Raw::Item(i) = r {
				if i.mid_stretched() == Some(false) {
					i.set_mid_stretched(Some(true));
					i.set_stretch(stretch);
				}
			}
		}
		let mut inner: Vec<Raw> = self.items.drain(lo..hi).collect();
		let len = inner.len();
		let opening = matches!(inner.first(), Some(Raw::Item(i)) if i.class() == MathClass::Opening);
		let closing = matches!(inner.last(), Some(Raw::Item(i)) if i.class() == MathClass::Closing);
		let mut index = 0;
		inner.retain(|r| {
			let discard = ((index == 1 && opening) || (index + 2 == len && closing))
				&& matches!(r, Raw::Item(Item::Spacing(_, true)));
			index += 1;
			!discard
		});
		let open = if opening && !inner.is_empty() { inner.remove(0).into_item() } else { None };
		let close = if closing { inner.pop().and_then(Raw::into_item) } else { None };
		match process_group(inner, styles, close.is_some(), false, true) {
			Group::Multiline(rows) => {
				let items = expand_multiline_fence(rows, open, close, styles, elem.span());
				let tail = self.items.split_off(lo);
				self.items.extend(items);
				self.items.extend(tail);
			}
			Group::Flat(items) => {
				let body = Item::comp(Kind::Group(items), Props::new(styles, None, Span::detached()), styles.clone());
				let item = fenced(open, close, FencedBody::Owned(body), true, styles, elem.span());
				self.items.insert(lo, Raw::Item(item));
			}
		}
		Ok(())
	}

	fn mid(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<()> {
		let body = res!(self.body(elem, styles));
		let start = res!(self.resolve_into_items(&body, styles));
		for r in self.items[start..].iter_mut() {
			if let Raw::Item(i) = r {
				i.set_mid_stretched(Some(false));
				i.set_class(MathClass::Relation);
			}
		}
		Ok(())
	}

	// Tables

	fn vec(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<()> {
		let rows: Vec<Vec<Content>> = res!(self.children(elem, styles, "children")).into_iter().map(|c| vec![c]).collect();
		let fs = props::font_size(styles);
		let gap = relative(res!(self.f(elem, styles, "gap"))).map(|r| Rel::of(&r, fs)).unwrap_or(Rel::new(0.0, ROW_GAP * fs));
		let align = align_of(res!(self.f(elem, styles, "align")));
		let cells = res!(self.cells(rows, elem.span(), align, Alternator::Right, None, (Rel::default(), gap),
			"elements", styles));
		let (open, close) = delims(res!(self.f(elem, styles, "delim")), ('(', ')'));
		self.delimiters(cells, open, close, elem.span(), styles)
	}

	fn mat(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<()> {
		let raw_rows: Vec<Value> = match res!(self.f(elem, styles, "rows")) {
			Some(Value::Array(rows))	=> rows.iter().cloned().collect(),
			_							=> Vec::new(),
		};
		let mut rows: Vec<Vec<Content>> = Vec::with_capacity(raw_rows.len());
		for r in raw_rows {
			let cells: Vec<Value> = match r {
				Value::Array(cells)	=> cells.iter().cloned().collect(),
				other				=> vec![other],
			};
			let mut row = Vec::with_capacity(cells.len());
			for c in cells {
				row.push(res!(self.disp(c, elem.span())));
			}
			rows.push(row);
		}
		let nrows = rows.len();
		let ncols = rows.first().map(|r| r.len()).unwrap_or(0);
		let fs = props::font_size(styles);
		let aug = res!(self.f(elem, styles, "augment"));
		let augment = res!(self.augment(aug, elem.span(), nrows, ncols));
		let rgap = relative(res!(self.f(elem, styles, "row-gap"))).map(|r| Rel::of(&r, fs)).unwrap_or(Rel::new(0.0, ROW_GAP * fs));
		let cgap = relative(res!(self.f(elem, styles, "column-gap"))).map(|r| Rel::of(&r, fs)).unwrap_or(Rel::new(0.0, COL_GAP * fs));
		let align = align_of(res!(self.f(elem, styles, "align")));
		let cells = res!(self.cells(rows, elem.span(), align, Alternator::Right, augment, (cgap, rgap), "cells", styles));
		let (open, close) = delims(res!(self.f(elem, styles, "delim")), ('(', ')'));
		self.delimiters(cells, open, close, elem.span(), styles)
	}

	fn augment(&mut self, v: Option<Value>, span: Span, nrows: usize, ncols: usize) -> Outcome<Option<Augment>> {
		let offsets = |v: Option<&Value>| -> Vec<i64> {
			match v {
				Some(Value::Int(i))		=> vec![*i],
				Some(Value::Array(a))	=> a.iter().filter_map(|x| match x {
					Value::Int(i)	=> Some(*i),
					_				=> None,
				}).collect(),
				_						=> Vec::new(),
			}
		};
		let aug = match v {
			None | Some(Value::None)	=> return Ok(None),
			Some(Value::Int(i))			=> Augment { hline: Vec::new(), vline: vec![i], stroke: None },
			Some(Value::Dict(d))		=> Augment {
				hline:	offsets(d.get("hline")),
				vline:	offsets(d.get("vline")),
				stroke:	d.get("stroke").and_then(to_stroke),
			},
			Some(_)						=> return Ok(None),
		};
		for &o in &aug.hline {
			if o > nrows as i64 || o.unsigned_abs() as usize > nrows {
				return Err(self.engine.error(DiagnosticKind::Type, span, fmt!(
					"cannot draw a horizontal line at offset {} in a matrix with {} rows", o, nrows)));
			}
		}
		for &o in &aug.vline {
			if o > ncols as i64 || o.unsigned_abs() as usize > ncols {
				return Err(self.engine.error(DiagnosticKind::Type, span, fmt!(
					"cannot draw a vertical line at offset {} in a matrix with {} columns", o, ncols)));
			}
		}
		Ok(Some(aug))
	}

	fn cases(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<()> {
		let rows: Vec<Vec<Content>> = res!(self.children(elem, styles, "children")).into_iter().map(|c| vec![c]).collect();
		let fs = props::font_size(styles);
		let gap = relative(res!(self.f(elem, styles, "gap"))).map(|r| Rel::of(&r, fs)).unwrap_or(Rel::new(0.0, ROW_GAP * fs));
		let cells = res!(self.cells(rows, elem.span(), Fixed::Start, Alternator::None, None,
			(Rel::default(), gap), "branches", styles));
		let (open, close) = delims(res!(self.f(elem, styles, "delim")), ('{', '}'));
		let reverse = matches!(res!(self.f(elem, styles, "reverse")), Some(Value::Bool(true)));
		let (open, close) = if reverse { (None, close) } else { (open, None) };
		self.delimiters(cells, open, close, elem.span(), styles)
	}

	#[allow(clippy::too_many_arguments)]
	fn cells(
		&mut self,
		rows:		Vec<Vec<Content>>,
		span:		Span,
		align:		Fixed,
		alternator:	Alternator,
		augment:	Option<Augment>,
		gap:		(Rel, Rel),
		what:		&str,
		styles:		&StyleChain,
	)
		-> Outcome<Item>
	{
		let cell_styles = props::chain_with(styles, props::for_denominator(styles));
		let mut cells: Vec<Vec<Row>> = Vec::with_capacity(rows.len());
		for row in rows {
			let mut out = Vec::with_capacity(row.len());
			for cell in row {
				let start = res!(self.resolve_into_items(&cell, &cell_styles));
				let drained: Vec<Raw> = self.items.drain(start..).collect();
				let (sub, had_breaks) = process_table_cell(drained, &cell_styles);
				if had_breaks {
					let d = crate::diag::Diagnostic::warning(DiagnosticKind::Internal, cell.span(), fmt!("linebreaks are ignored in {}", what))
						.with_hint("use commas instead to separate each line");
					self.engine.diags.push(d);
				}
				out.push(sub);
			}
			cells.push(out);
		}
		let ncols = cells.first().map(|r| r.len()).unwrap_or(0);
		for c in 0..ncols {
			let max = cells.iter().map(|r| r.get(c).map(|x| x.len()).unwrap_or(0)).max().unwrap_or(0);
			for row in cells.iter_mut() {
				if let Some(x) = row.get_mut(c) {
					x.pad_to(max, &cell_styles);
				}
			}
		}
		let table = Table { cells, gap, augment, align, alternator };
		Ok(Item::comp(Kind::Table(Box::new(table)), Props::new(styles, None, span), styles.clone()))
	}

	fn delimiters(&mut self, cells: Item, open: Option<char>, close: Option<char>, span: Span, styles: &StyleChain) -> Outcome<()> {
		let stretch = Stretch::new().with_y(StretchInfo::new(Rel::new(1.1, 0.0), DELIM_SHORT_FALL));
		let open = match open {
			Some(c)	=> {
				let i = res!(self.symbol_item(c, span, styles));
				i.set_stretch(stretch);
				Some(i)
			}
			None	=> None,
		};
		let close = match close {
			Some(c)	=> {
				let i = res!(self.symbol_item(c, span, styles));
				i.set_stretch(stretch);
				Some(i)
			}
			None	=> None,
		};
		self.push(fenced(open, close, FencedBody::Owned(cells), false, styles, span));
		Ok(())
	}

	// Classes and operators

	fn class(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<()> {
		let class = match res!(self.f(elem, styles, "class")) {
			Some(Value::Str(s))	=> match MathClass::from_name(&s) {
				Some(c)	=> c,
				None	=> return Err(self.engine.error(DiagnosticKind::Type, elem.span(), fmt!("unknown math class: {}", s))),
			},
			_ => MathClass::Normal,
		};
		let mut item = { let tmp = res!(self.body(elem, styles)); res!(self.resolve_into_item(&tmp, styles)) };
		item.set_explicit_class(class);
		item.set_limits(Limits::for_class(class));
		self.push(item);
		Ok(())
	}

	fn op(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<()> {
		let text = res!(self.c(elem, styles, "text"));
		let mut item = res!(self.resolve_into_item(&text, styles));
		item.set_class(MathClass::Large);
		let limits = matches!(res!(self.f(elem, styles, "limits")), Some(Value::Bool(true)));
		item.set_limits(if limits { Limits::Display } else { Limits::Never });
		self.push(item);
		Ok(())
	}

	// Radicals, accents and lines

	fn root(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<()> {
		let cramped = props::chain_with(styles, vec![props::set_cramped(true)]);
		let radicand = { let tmp = res!(self.c(elem, styles, "radicand")); res!(self.resolve_into_item(&tmp, &cramped)) }
			.with_multiline_centering();
		let index = match res!(self.f(elem, styles, "index")) {
			None | Some(Value::None)	=> None,
			Some(v)						=> {
				let ss = props::chain_with(&cramped, vec![props::set_size(MathSize::ScriptScript)]);
				let c = res!(self.disp(v, elem.span()));
				Some(res!(self.resolve_into_item(&c, &ss)))
			}
		};
		let sqrt = res!(self.symbol_item('\u{221a}', elem.span(), styles));
		sqrt.set_stretch(Stretch::new().with_y(StretchInfo::new(Rel::one(), 0.0)));
		self.push(Item::comp(Kind::Radical { radicand, index, sqrt }, Props::new(styles, None, elem.span()), styles.clone()));
		Ok(())
	}

	fn accent(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<()> {
		let accent = match res!(self.f(elem, styles, "accent")).and_then(|v| accent_char(&v)) {
			Some(c)	=> c,
			None	=> return Err(self.engine.error(DiagnosticKind::Type, elem.span(), "expected a single-codepoint symbol")),
		};
		let position = if accent_is_bottom(accent) { Position::Below } else { Position::Above };
		let dotless = !matches!(res!(self.f(elem, styles, "dotless")), Some(Value::Bool(false)));
		let base_styles = if position == Position::Above {
			props::chain_with(styles, vec![props::set_cramped(true)])
		} else {
			styles.clone()
		};
		// The base asks for dotless forms (the `dtls` feature) so an accent over `i` loses the dot.
		let base_styles = if position == Position::Above && dotless {
			props::chain_with(&base_styles, vec![props::property(ElemKind::Equation, "dtls", Value::Bool(true))])
		} else {
			base_styles
		};
		let base = { let tmp = res!(self.c(elem, styles, "base")); res!(self.resolve_into_item(&tmp, &base_styles)) };
		let mut acc = res!(self.symbol_item(accent, elem.span(), styles));
		acc.set_class(MathClass::Diacritic);
		let fs = props::font_size(styles);
		let size = relative(res!(self.f(elem, styles, "size"))).map(|r| Rel::of(&r, fs)).unwrap_or(Rel::one());
		acc.set_stretch(Stretch::new().with_x(StretchInfo::new(size, ACCENT_SHORT_FALL)));
		let class = base.raw_class();
		self.push(Item::comp(Kind::Accent { base, accent: acc, position, dotless, exact: false },
			Props::new(styles, class, Span::detached()), styles.clone()));
		Ok(())
	}

	fn underline(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<()> {
		let base = { let tmp = res!(self.body(elem, styles)); res!(self.resolve_into_item(&tmp, styles)) };
		let class = base.raw_class();
		self.push(Item::comp(Kind::Line { base, position: Position::Below }, Props::new(styles, class, elem.span()), styles.clone()));
		Ok(())
	}

	fn overline(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<()> {
		let cramped = props::chain_with(styles, vec![props::set_cramped(true)]);
		let base = { let tmp = res!(self.body(elem, styles)); res!(self.resolve_into_item(&tmp, &cramped)) };
		let class = base.raw_class();
		self.push(Item::comp(Kind::Line { base, position: Position::Above }, Props::new(styles, class, elem.span()), styles.clone()));
		Ok(())
	}

	fn spreader(&mut self, elem: &Content, styles: &StyleChain, c: char, position: Position) -> Outcome<()> {
		let base = { let tmp = res!(self.body(elem, styles)); res!(self.resolve_into_item(&tmp, styles)) };
		let mut acc = res!(self.symbol_item(c, elem.span(), styles));
		acc.set_class(MathClass::Diacritic);
		acc.set_stretch(Stretch::new().with_x(StretchInfo::new(Rel::one(), 0.0)));
		let class = base.raw_class();
		let base = Item::comp(Kind::Accent { base, accent: acc, position, dotless: false, exact: true },
			Props::new(styles, class, Span::detached()), styles.clone());
		let annotation = match res!(self.f(elem, styles, "annotation")) {
			None | Some(Value::None)	=> None,
			Some(v)						=> Some(res!(self.disp(v, elem.span()))),
		};
		let annotation = match annotation {
			Some(a)	=> a,
			None	=> {
				self.push(base);
				return Ok(());
			}
		};
		let item = match position {
			Position::Below => {
				let under = props::chain_with(styles, props::for_subscript(styles));
				let a = res!(self.resolve_into_item(&annotation, &under));
				scripts(base, None, Some(a), None, None, None, None, styles)
			}
			Position::Above => {
				let over = props::chain_with(styles, vec![props::for_superscript(styles)]);
				let a = res!(self.resolve_into_item(&annotation, &over));
				scripts(base, Some(a), None, None, None, None, None, styles)
			}
		};
		self.push(item);
		Ok(())
	}
	// Field access

	/// A field's value: the element's own, else the style chain's.
	pub fn f(&mut self, elem: &Content, styles: &StyleChain, name: &str) -> Outcome<Option<Value>> {
		let kind = match elem.kind() {
			Some(k)	=> k,
			None	=> return Ok(None),
		};
		match kind.field_id(name) {
			Some(id)	=> styles.resolve(elem, id),
			None		=> Ok(None),
		}
	}

	fn disp(&mut self, v: Value, span: Span) -> Outcome<Content> { display(self.engine, v, span) }

	fn c(&mut self, elem: &Content, styles: &StyleChain, name: &str) -> Outcome<Content> {
		match res!(self.f(elem, styles, name)) {
			Some(v)	=> self.disp(v, elem.span()),
			None	=> Ok(Content::empty()),
		}
	}

	fn body(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<Content> { self.c(elem, styles, "body") }

	// An attachment under the chain its position sets, `None` when unset.
	fn script(&mut self, elem: &Content, name: &str, chain: &StyleChain) -> Outcome<Option<Content>> {
		match res!(self.f(elem, chain, name)) {
			None | Some(Value::None)	=> Ok(None),
			Some(v)						=> Ok(Some(res!(self.disp(v, elem.span())))),
		}
	}

	fn children(&mut self, elem: &Content, styles: &StyleChain, name: &str) -> Outcome<Vec<Content>> {
		let items = match res!(self.f(elem, styles, name)) {
			Some(Value::Array(a))	=> a.iter().cloned().collect(),
			Some(v)					=> vec![v],
			None					=> Vec::new(),
		};
		let mut out = Vec::with_capacity(items.len());
		for v in items {
			out.push(res!(self.disp(v, elem.span())));
		}
		Ok(out)
	}
}

// Construction helpers

const ONE: Relative = Relative { rel: Ratio(1.0), abs: Length { abs: 0.0, em: 0.0 } };

/// A symbol's glyph item: its class and default limits from its first character.
pub fn glyph(text: String, styles: &StyleChain, span: Span) -> Item {
	let c = text.chars().next().unwrap_or(' ');
	let class = default_math_class(c);
	let limits = Limits::for_char_with_class(c, class);
	let g = Glyph {
		text,
		class:			class.unwrap_or(MathClass::Normal),
		stretch:		Cell::new(Stretch::new()),
		mid_stretched:	Cell::new(None),
		flac:			Cell::new(false),
	};
	let mut props = Props::new(styles, class, span);
	props.limits = limits;
	Item::comp(Kind::Glyph(g), props, styles.clone())
}

pub fn multiline(rows: Vec<Row>, styles: &StyleChain) -> Item {
	Item::comp(Kind::Multiline { rows, centered: false }, Props::new(styles, None, Span::detached()), styles.clone())
}

pub fn fenced(open: Option<Item>, close: Option<Item>, body: FencedBody, balanced: bool, styles: &StyleChain, span: Span) -> Item {
	Item::comp(Kind::Fenced { open, close, body, balanced }, Props::new(styles, None, span), styles.clone())
}

#[allow(clippy::too_many_arguments)]
fn scripts(
	base:	Item,
	t:		Option<Item>,
	b:		Option<Item>,
	tl:		Option<Item>,
	bl:		Option<Item>,
	tr:		Option<Item>,
	br:		Option<Item>,
	styles:	&StyleChain,
)
	-> Item
{
	let class = base.raw_class();
	let s = Scripts { base, t, b, tl, bl, tr, br };
	Item::comp(Kind::Scripts(Box::new(s)), Props::new(styles, class, Span::detached()), styles.clone())
}

pub fn shared_sizing(items: Vec<Item>, styles: &StyleChain) -> std::rc::Rc<SharedSizing> {
	SharedSizing::new(items, styles.clone())
}

// Typst's `merge_inward` over one position's outer attachments (outermost first): the innermost outer
// value comes in, and every value further out moves one level inward.
fn take_outer(stack: &mut [Option<Content>]) -> Option<Content> {
	let n = stack.len();
	if n == 0 {
		return None;
	}
	let out = stack[n - 1].take();
	for j in (1..n).rev() {
		stack[j] = stack[j - 1].take();
	}
	out
}

// Typst's `unmerge`: gives a value back to the innermost outer level, pushing what is there outward
// until an empty level takes it.
fn unmerge(stack: &mut [Option<Content>], content: Content) {
	let mut cur = Some(content);
	for j in (0..stack.len()).rev() {
		if cur.is_none() {
			break;
		}
		cur = std::mem::replace(&mut stack[j], cur);
	}
}

fn relative(v: Option<Value>) -> Option<Relative> {
	match v {
		Some(Value::Relative(r))	=> Some(r),
		Some(Value::Length(l))		=> Some(Relative { rel: Ratio(0.0), abs: l }),
		Some(Value::Ratio(r))		=> Some(Relative { rel: r, abs: Length::zero() }),
		_							=> None,
	}
}

fn align_of(v: Option<Value>) -> Fixed {
	use crate::eval::value::HAlign;
	match v {
		Some(Value::Alignment(a)) => match a.x {
			Some(HAlign::Start) | Some(HAlign::Left)	=> Fixed::Start,
			Some(HAlign::End) | Some(HAlign::Right)		=> Fixed::End,
			_											=> Fixed::Center,
		},
		_ => Fixed::Center,
	}
}

/// The single character of a delimiter value, `None` for `none`.
pub fn delim_char(v: &Value) -> Option<Option<char>> {
	let one = |s: &str| -> Option<Option<char>> {
		let mut it = s.chars();
		match (it.next(), it.next()) {
			(Some(c), None)	=> Some(Some(c)),
			_				=> None,
		}
	};
	match v {
		Value::None			=> Some(None),
		Value::Str(s)		=> one(s),
		Value::Symbol(s)	=> one(&crate::eval::ops::symbol_text(s)),
		Value::Content(c)	=> one(&c.plain_text()),
		_					=> None,
	}
}

/// The delimiter matching an opening one: brackets and braces by name, other openers and closers by
/// their neighbouring code point, fences by themselves.
pub fn matching(c: char) -> Option<char> {
	match c {
		'['	=> Some(']'),
		']'	=> Some('['),
		'{'	=> Some('}'),
		'}'	=> Some('{'),
		c	=> match default_math_class(c) {
			Some(MathClass::Opening)	=> char::from_u32(c as u32 + 1),
			Some(MathClass::Closing)	=> char::from_u32(c as u32 - 1),
			_							=> Some(c),
		},
	}
}

// A `delim` value as an opening and closing pair: one delimiter and its match, or an array of two.
fn delims(v: Option<Value>, default: (char, char)) -> (Option<char>, Option<char>) {
	match v {
		None => (Some(default.0), Some(default.1)),
		Some(Value::Array(a)) if a.len() == 2 => {
			let o = a.first().and_then(delim_char).flatten();
			let c = a.get(1).and_then(delim_char).flatten();
			(o, c)
		}
		Some(other) => match delim_char(&other) {
			Some(Some(c))	=> (Some(c), matching(c)),
			_				=> (None, None),
		},
	}
}

/// The combining accents Typst knows, each with the spellings that name it.
pub const ACCENTS: &[(char, &[&str])] = &[
	('\u{0300}', &["`"]),
	('\u{0301}', &["\u{b4}"]),
	('\u{0302}', &["^", "\u{2c6}"]),
	('\u{0303}', &["~", "\u{223c}", "\u{2dc}"]),
	('\u{0304}', &["\u{af}"]),
	('\u{0305}', &["-", "\u{2013}", "\u{203e}", "\u{2212}"]),
	('\u{0306}', &["\u{2d8}"]),
	('\u{0307}', &[".", "\u{2d9}", "\u{22c5}"]),
	('\u{0308}', &["\u{a8}"]),
	('\u{20db}', &[]),
	('\u{20dc}', &[]),
	('\u{030a}', &["\u{2218}", "\u{25cb}"]),
	('\u{030b}', &["\u{2dd}"]),
	('\u{030c}', &["\u{2c7}"]),
	('\u{20d6}', &["\u{2190}"]),
	('\u{20d7}', &["\u{2192}", "\u{27f6}"]),
	('\u{20e1}', &["\u{2194}", "\u{2194}\u{fe0e}", "\u{27f7}"]),
	('\u{20d0}', &["\u{21bc}"]),
	('\u{20d1}', &["\u{21c0}"]),
];

/// The combining accent a spelling names, if it names one.
pub fn accent_combining(s: &str) -> Option<char> {
	let mut it = s.chars();
	let single = match (it.next(), it.next()) {
		(Some(c), None)	=> Some(c),
		_				=> None,
	};
	ACCENTS.iter().find(|(a, names)| Some(*a) == single || names.contains(&s)).map(|(a, _)| *a)
}

/// Typst's accent cast: a known spelling to its combining form, else any single character as it is.
pub fn accent_char(v: &Value) -> Option<char> {
	let s = match v {
		Value::Str(s)		=> s.to_string(),
		Value::Symbol(s)	=> crate::eval::ops::symbol_text(s),
		Value::Content(c)	=> c.plain_text(),
		_					=> return None,
	};
	if let Some(c) = accent_combining(&s) {
		return Some(c);
	}
	let mut it = s.chars();
	match (it.next(), it.next()) {
		(Some(c), None)	=> Some(c),
		_				=> None,
	}
}

// Bottom accents: the under-spreaders and every combining mark of canonical class 220 (below).
fn accent_is_bottom(c: char) -> bool {
	if matches!(c, '\u{23df}' | '\u{23b5}' | '\u{23dd}' | '\u{23e1}') {
		return true;
	}
	oxedyne_fe2o3_text::unicode::norm::combining_class(c) == 220
}

/// Is the text a number as maths sets it: digits with at most one decimal point?
pub fn is_number(text: &str) -> bool {
	let mut dots = 0;
	let digits = text.chars().all(|c| {
		if c == '.' {
			dots += 1;
		}
		c.is_ascii_digit() || c == '.'
	});
	digits && dots != text.chars().count() && dots <= 1
}

// Typst's `split_newlines`: `\n`, `\r\n`, `\r` and the other Unicode line terminators.
fn split_newlines(text: &str) -> Vec<&str> {
	let mut out = Vec::new();
	let mut start = 0;
	let bytes: Vec<(usize, char)> = text.char_indices().collect();
	let mut i = 0;
	while i < bytes.len() {
		let (at, c) = bytes[i];
		let nl = matches!(c, '\n' | '\x0b' | '\x0c' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}');
		if nl {
			out.push(&text[start..at]);
			let mut next = at + c.len_utf8();
			if c == '\r' {
				if let Some(&(_, '\n')) = bytes.get(i + 1) {
					next += 1;
					i += 1;
				}
			}
			start = next;
		}
		i += 1;
	}
	out.push(&text[start..]);
	out
}
