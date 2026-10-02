// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-layout `pages/{run,finalize}.rs`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
// U9 owns this file: page furniture. The header, footer, background and foreground are evaluated once per
// page per pass, laid out beside the body, with `here()` at the page; they sit outside the body flow, so
// they never feed back into pagination.
//
// A port of Typst 0.15.1's page-run marginals. Each is laid out with the page's own styles, expanding to
// its area and aligned in it: the header to the bottom of the strip above the body (the top margin less
// `header-ascent`), the footer to the top of the strip below it (the bottom margin less `footer-descent`),
// and the background and foreground to the centre of the whole page. An `auto` header or footer holds the
// page number when `numbering` is set, a `counter(page).display(numbering)` aligned by `number-align`,
// whose `y` picks the header or the footer; `none` clears the slot. The frames are placed by
// [`crate::driver::place_page`] in Typst's order (background, header, body, footer, foreground), so the
// located elements of the furniture are recorded in that order and the introspector reads them as it does.
//
// Bleed is not drawn: a page with a bleed warns, and its marginals are laid to the trimmed page.

use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	ElemKind,
};
use crate::eval::func::{
	Func,
	NativeFunc,
};
use crate::eval::intro::Counter;
use crate::eval::lib::intro::IntroFn;
use crate::eval::lib::numbering::Pattern;
use crate::eval::styles::{
	Property,
	Style,
	StyleChain,
	Styles,
};
use crate::eval::value::{
	Alignment,
	HAlign,
	VAlign,
	Value,
};
use crate::eval::Engine;
use crate::flow::block::{
	self,
	Frame,
	Regions,
};
use crate::flow::page::{
	page_field,
	rel,
};
use crate::flow::{
	RunSetup,
	Slot,
};
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

// Where Typst has the default `number-align` (`center + bottom`) and the marginals' own alignments.
const NUMBER_ALIGN:	Alignment = Alignment { x: Some(HAlign::Center), y: Some(VAlign::Bottom) };
const HEADER_ALIGN:	Alignment = Alignment { x: None, y: Some(VAlign::Bottom) };
const FOOTER_ALIGN:	Alignment = Alignment { x: None, y: Some(VAlign::Top) };
const MIDDLE:		Alignment = Alignment { x: Some(HAlign::Center), y: Some(VAlign::Horizon) };

/// A page's laid-out furniture. Each frame is as large as its area, so a footer stands at the page's foot
/// by `foot_h`, its area's height, whatever it holds.
#[derive(Debug, Default)]
pub struct Marginals {
	pub background:	Option<Frame>,
	pub header:		Option<Frame>,
	pub footer:		Option<Frame>,
	pub foreground:	Option<Frame>,
	pub foot_h:		f64,	// the footer area's height: the bottom margin less `footer-descent`
}

impl Marginals {
	/// Does the page have no furniture?
	pub fn is_empty(&self) -> bool {
		self.background.is_none() && self.header.is_none() && self.footer.is_none() && self.foreground.is_none()
	}
}

/// Lays out a page's furniture. `inner` is the size of the page's body frame, inside the margins, which the
/// header and footer span and the background and foreground exceed by the margins. The page number, where
/// there is one, is read in a context at the footer's own location, from the introspector the previous pass
/// built, and the reads are recorded for the fixpoint like any other.
pub fn decorate_page(engine: &mut Engine, setup: &RunSetup, inner: (f64, f64)) -> Outcome<Marginals> {
	let styles	= &setup.styles;
	let geom	= &setup.geom;
	let (top, bottom, left, right) = (geom.top.to_pt(), geom.bottom.to_pt(), geom.inside.to_pt(), geom.outside.to_pt());
	let ascent	= res!(strip(styles, "header-ascent", top));
	let descent	= res!(strip(styles, "footer-descent", bottom));
	let align	= match res!(page_field(styles, "number-align")) {
		Some(Value::Alignment(a))	=> a,
		_							=> NUMBER_ALIGN,
	};
	let numbered = res!(numbering_marginal(&setup.numbering, align));
	// `number-align`'s vertical part picks the header or the footer to carry the number.
	let (header, footer) = match align.y {
		Some(VAlign::Top)	=> (pick(&setup.header, numbered), pick(&setup.footer, None)),
		_					=> (pick(&setup.header, None), pick(&setup.footer, numbered)),
	};
	let head_h	= (top - ascent).max(0.0);
	let foot_h	= (bottom - descent).max(0.0);
	let (full_w, full_h) = (inner.0 + left + right, inner.1 + top + bottom);
	let mut out = Marginals { foot_h, ..Marginals::default() };
	if let Some(c) = &header {
		out.header = Some(res!(marginal(engine, c, HEADER_ALIGN, styles, inner.0, head_h)));
	}
	if let Some(c) = &footer {
		out.footer = Some(res!(marginal(engine, c, FOOTER_ALIGN, styles, inner.0, foot_h)));
	}
	if let Some(c) = &setup.background {
		out.background = Some(res!(marginal(engine, c, MIDDLE, styles, full_w, full_h)));
	}
	if let Some(c) = &setup.foreground {
		out.foreground = Some(res!(marginal(engine, c, MIDDLE, styles, full_w, full_h)));
	}
	Ok(out)
}

/// A margin strip's depth: the `header-ascent` or `footer-descent` taken from its margin.
fn strip(styles: &StyleChain, field: &str, margin: f64) -> Outcome<f64> {
	Ok(match res!(page_field(styles, field)) {
		Some(v)	=> rel(styles, &v).map(|r| r.relative_to(margin)).unwrap_or(0.3 * margin),
		None	=> 0.3 * margin,
	})
}

/// The content a slot holds: its own, none, or the page number's when it is left `auto`.
fn pick(slot: &Slot, auto: Option<Content>) -> Option<Content> {
	match slot {
		Slot::Auto		=> auto,
		Slot::Off		=> None,
		Slot::On(c)		=> Some(c.clone()),
	}
}

/// The page number as Typst builds it: `align(x)[#context counter(page).display(numbering, both:)]`,
/// `both` when the pattern has two counting symbols (or the numbering is a function), so `"1 / 1"` shows
/// the page and the total. A numbering that is not a pattern or a function has no marginal.
fn numbering_marginal(numbering: &Value, align: Alignment) -> Outcome<Option<Content>> {
	let both = match numbering {
		Value::Str(s)	=> Pattern::parse(s).map(|p| p.pieces.len() >= 2).unwrap_or(false),
		Value::Func(_)	=> true,
		_				=> return Ok(None),
	};
	let span = Span::detached();
	let mut args = Args::new(span);
	args.push(span, Value::Counter(Arc::new(Counter::page())));
	args.push(span, numbering.clone());
	args.push_named(span, "both", Value::Bool(both));
	let show = Func::Native(NativeFunc::Intro(IntroFn::CounterDisplay)).with(args);
	let func = res!(ElemKind::Context.field_id("func")
		.ok_or_else(|| err!("The context element has no func field."; Bug)));
	let context = Content::new(ElemKind::Context, vec![(func, Value::Func(show))], span);
	let body = res!(ElemKind::Align.field_id("body")
		.ok_or_else(|| err!("The align element has no body field."; Bug)));
	let at = res!(ElemKind::Align.field_id("alignment")
		.ok_or_else(|| err!("The align element has no alignment field."; Bug)));
	// Only the horizontal part of `number-align` aligns the number; the vertical part chose the slot.
	Ok(Some(match align.x {
		Some(x)	=> Content::new(ElemKind::Align, vec![
			(body, Value::Content(context)),
			(at, Value::Alignment(Alignment { x: Some(x), y: None })),
		], span),
		None	=> context,
	}))
}

/// One marginal laid out as Typst's `layout_marginal`: its content aligned by a property set on it, into a
/// region of `w` by `h` that it expands to fill.
fn marginal(
	engine:		&mut Engine,
	content:	&Content,
	align:		Alignment,
	styles:		&StyleChain,
	w:			f64,
	h:			f64,
)
	-> Outcome<Frame>
{
	let at = res!(ElemKind::Align.field_id("alignment")
		.ok_or_else(|| err!("The align element has no alignment field."; Bug)));
	let set = Style::Property(Property::new(ElemKind::Align, at, Value::Alignment(align), content.span()));
	let aligned = content.clone().styled(Styles::from_style(set));
	block::layout_frame(engine, &aligned, styles, Regions::one(w, h, true, true))
}
