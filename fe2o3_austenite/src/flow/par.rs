// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-layout `inline/{mod,finalize}.rs`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
// U6a owns this file: a paragraph's realised children into justified lines through `linebreak.rs`, with
// Typst's `par` settings (leading, spacing, justify, first-line-indent, hanging-indent, linebreaks).
//
// The configuration and the finishing of lines follow Typst 0.15.1's `typst-layout/src/inline/{mod,
// finalize}.rs` (Apache-2.0, (c) the Typst project authors): `linebreaks: auto` is optimised when the
// paragraph justifies and simple otherwise, `hyphenate: auto` follows `justify`, a first-line indent
// applies after another paragraph (or everywhere with `all: true`, never to a first paragraph inside a
// list), and a paragraph fills its region's width unless it may shrink to its longest line.

use crate::eval::content::{
	Content,
	ElemKind,
};
use crate::eval::lib::model::list::in_list;
use crate::eval::realise::{
	realise,
	Pair,
	RealiseMode,
};
use crate::eval::styles::StyleChain;
use crate::eval::value::Value;
use crate::eval::Engine;
use crate::flow::inline::{
	commit,
	fixed_align,
	lang_of,
	prepare,
	Config,
	Frame,
	LineBox,
	Linebreaks,
	ParSituation,
};
use crate::flow::text::{
	self as ftext,
	elem_field,
	text_field,
};
use crate::flow::Region;
use crate::ir::{
	Glue,
	Node,
	Penalty,
	Sp,
};
use crate::linebreak::linebreak;

use oxedyne_fe2o3_core::prelude::*;

/// The `par` settings an inline layout starts from: a paragraph's own, or the chain's for inline content
/// that is not a paragraph.
#[derive(Clone, Debug)]
pub struct ConfigBase {
	pub justify:			bool,
	pub linebreaks:			Option<Linebreaks>,	// `auto` as `None`
	pub first_line_indent:	(f64, bool),		// amount in points, and whether it applies to all
	pub hanging_indent:		f64,
}

/// Sets a `par` element, Typst's `layout_par`: its body realised in `Inline` mode, broken into lines
/// under its own fields folded onto the chain, and the lines stacked its `leading` apart. `situation` is
/// where the flow met the paragraph, which decides the first-line indent.
///
/// Nothing is kept between calls. The lines depend only on the paragraph, its chain, `region` (the
/// width, and the base and height that relative and fractional inline boxes resolve against),
/// `situation` and the faces of `engine.fonts`; an inline box or equation inside may also read the
/// introspector, take locations and warn, through `engine`, as any layout does. That is the whole key
/// a paragraph cache needs.
pub fn layout_par(
	engine:		&mut Engine,
	par:		&Content,
	styles:		&StyleChain,
	region:		Region,
	situation:	ParSituation,
)
	-> Outcome<Vec<Node>>
{
	let body = match par.kind().and_then(|k| k.field_id("body")).and_then(|id| par.get(id)) {
		Some(Value::Content(c))	=> c.clone(),
		_						=> Content::empty(),
	};
	let children = res!(realise(engine, &body, styles, RealiseMode::Inline));
	let base = res!(base_of(styles, Some(par)));
	let lines = res!(layout_with(engine, &children, styles, region, region.expand_x, Some(situation), &base));
	let leading = match res!(par_value(styles, Some(par), "leading")) {
		Some(Value::Length(l))	=> l.resolve(styles.font_size()),
		_						=> 0.65 * styles.font_size(),
	};
	stack_with(styles, lines, leading)
}

/// Inline content that is not a paragraph -- a container's body that is inline only -- as lines with no
/// first-line or hanging indent, filling the region only when `expand`: Typst's `layout_inline`.
pub fn layout_lines(
	engine:		&mut Engine,
	children:	&[Pair],
	styles:		&StyleChain,
	region:		Region,
	expand:		bool,
)
	-> Outcome<Vec<LineBox>>
{
	let base = res!(base_of(styles, None));
	layout_with(engine, children, styles, region, expand, None, &base)
}

/// As [`layout_lines`], stacked `par.leading` apart.
pub fn layout_inline(
	engine:		&mut Engine,
	children:	&[Pair],
	styles:		&StyleChain,
	region:		Region,
	expand:		bool,
)
	-> Outcome<Vec<Node>>
{
	let lines = res!(layout_lines(engine, children, styles, region, expand));
	stack(styles, lines)
}

/// A `par` field from the element when given, else the chain.
fn par_value(styles: &StyleChain, par: Option<&Content>, name: &str) -> Outcome<Option<Value>> {
	match (par, ElemKind::Par.field_id(name)) {
		(Some(p), Some(id))	=> styles.resolve(p, id),
		(None, Some(id))	=> styles.get(ElemKind::Par, id),
		(_, None)			=> Ok(None),
	}
}

fn base_of(styles: &StyleChain, par: Option<&Content>) -> Outcome<ConfigBase> {
	let size = styles.font_size();
	let justify = matches!(res!(par_value(styles, par, "justify")), Some(Value::Bool(true)));
	let linebreaks = match res!(par_value(styles, par, "linebreaks")) {
		Some(Value::Str(s)) if s.as_str() == "simple"		=> Some(Linebreaks::Simple),
		Some(Value::Str(s)) if s.as_str() == "optimized"	=> Some(Linebreaks::Optimized),
		_													=> None,
	};
	let first_line_indent = match res!(par_value(styles, par, "first-line-indent")) {
		Some(Value::Length(l))	=> (l.resolve(size), false),
		Some(Value::Dict(d))	=> (
			match d.get("amount") {
				Some(Value::Length(l))	=> l.resolve(size),
				_						=> 0.0,
			},
			matches!(d.get("all"), Some(Value::Bool(true))),
		),
		_						=> (0.0, false),
	};
	let hanging_indent = match res!(par_value(styles, par, "hanging-indent")) {
		Some(Value::Length(l))	=> l.resolve(size),
		_						=> 0.0,
	};
	Ok(ConfigBase { justify, linebreaks, first_line_indent, hanging_indent })
}

/// The value a style property takes for every child, or `None` when the children differ.
fn shared<T: PartialEq, F: Fn(&StyleChain) -> Outcome<T>>(children: &[Pair], styles: &StyleChain, get: F)
	-> Outcome<Option<T>>
{
	let v = res!(get(styles));
	for c in children {
		if c.is_tag() {
			continue;
		}
		if res!(get(&c.styles)) != v {
			return Ok(None);
		}
	}
	Ok(Some(v))
}

fn configuration(
	base:		&ConfigBase,
	children:	&[Pair],
	styles:		&StyleChain,
	situation:	Option<ParSituation>,
)
	-> Outcome<Config>
{
	let justify = base.justify;
	let size = styles.font_size();
	let props = res!(ftext::props(styles));
	let rtl = props.rtl;
	let align = res!(fixed_align(styles, rtl));
	let (amount, all) = base.first_line_indent;
	let first_line_indent = if amount != 0.0
		&& match situation {
			// An indent under a list's marker just looks bad.
			Some(ParSituation::First)		=> all && !in_list(styles),
			Some(ParSituation::Consecutive)	=> true,
			Some(ParSituation::Other)		=> all,
			None							=> false,
		}
		&& align == crate::flow::inline::FixedAlign::Start
	{
		amount
	} else {
		0.0
	};
	let hyphenate = res!(shared(children, styles, |s| Ok(match res!(text_field(s, "hyphenate")) {
		Some(Value::Bool(b))	=> Some(b),
		_						=> None,
	}))).map(|uniform| uniform.unwrap_or(justify));
	Ok(Config {
		justify,
		linebreaks:			base.linebreaks.unwrap_or(if justify { Linebreaks::Optimized } else { Linebreaks::Simple }),
		first_line_indent,
		hanging_indent:		if situation.is_some() { base.hanging_indent } else { 0.0 },
		align,
		font_size:			size,
		rtl,
		hyphenate,
		lang:				res!(shared(children, styles, lang_of)),
		cjk_latin_spacing:	props.cjk_latin_spacing,
		costs:				props.costs,
	})
}

/// Lines under an explicit configuration base, for a caller that holds the `par` settings itself.
pub fn layout_with(
	engine:		&mut Engine,
	children:	&[Pair],
	styles:		&StyleChain,
	region:		Region,
	expand:		bool,
	situation:	Option<ParSituation>,
	base:		&ConfigBase,
)
	-> Outcome<Vec<LineBox>>
{
	let config = res!(configuration(base, children, styles, situation));
	let hang = config.hanging_indent;
	let p = res!(prepare(engine, children, config, region));
	let region_w = region.width.to_pt();
	let lines = res!(linebreak(&p, region_w - hang));

	// The width the lines fill: the region's, unless the content may shrink to its longest line and has
	// no fractional spacing.
	let finite = region.width.raw() < (1 << 30);
	let width = if !finite || (!expand && lines.iter().all(|l| l.fr() == 0.0)) {
		let longest = lines.iter().map(|l| l.width).fold(0.0f64, f64::max);
		if finite { region_w.min(hang + longest) } else { hang + longest }
	} else {
		region_w
	};
	let full = region.height.to_pt();
	let mut out = Vec::with_capacity(lines.len());
	for l in &lines {
		out.push(res!(commit(engine, &p, l, width, full)));
	}
	Ok(out)
}

/// The lines stacked `par.leading` apart, with the breaks between them a page may not take: after the
/// first line (an orphan) and before the last (a widow) while `text.costs` prices them, and none at all
/// in a paragraph of three lines guarding both. A forbidden break is an infinite penalty before the
/// leading glue, which a page breaker reads as TeX does: glue after a penalty is no breakpoint.
pub fn stack(styles: &StyleChain, lines: Vec<LineBox>) -> Outcome<Vec<Node>> {
	let size = styles.font_size();
	let leading = match res!(elem_field(styles, ElemKind::Par, "leading")) {
		Some(Value::Length(l))	=> l.resolve(size),
		_						=> 0.65 * size,
	};
	stack_with(styles, lines, leading)
}

/// As [`stack`], `leading` points apart.
fn stack_with(styles: &StyleChain, lines: Vec<LineBox>, leading: f64) -> Outcome<Vec<Node>> {
	let costs = res!(ftext::props(styles)).costs;
	let len = lines.len();
	let non_empty = |l: &LineBox| match &l.node {
		Node::HBox(b)	=> !b.list.is_empty(),
		_				=> true,
	};
	let orphans = costs.orphan > 0.0 && len >= 2 && non_empty(&lines[1]);
	let widows = costs.widow > 0.0 && len >= 2 && non_empty(&lines[len - 2]);
	let all = len == 3 && orphans && widows;
	let mut out = Vec::with_capacity(len * 3);
	for (i, l) in lines.into_iter().enumerate() {
		if i > 0 {
			// The break between line i - 1 and line i.
			let keep = all || (orphans && i == 1) || (widows && i + 1 == len);
			if keep {
				out.push(Node::Penalty(Penalty::new(Penalty::INFINITY, false)));
			}
			out.push(Node::Glue(Glue::fixed(Sp::from_pt(leading))));
		}
		out.push(l.node);
	}
	Ok(out)
}

/// Lines stacked into one frame, as an inline box holds them, its baseline its first line's.
pub fn stack_frame(styles: &StyleChain, lines: Vec<LineBox>) -> Outcome<Frame> {
	let width = lines.iter().map(|l| l.width).fold(0.0f64, f64::max);
	let first_top = lines.first().map_or(0.0, |l| l.top);
	let nodes = res!(stack(styles, lines));
	let height: f64 = nodes.iter().map(|n| n.vextent().to_pt()).sum();
	Ok(Frame::new(nodes, width, first_top, height))
}
