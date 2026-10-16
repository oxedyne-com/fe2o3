// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-library `math/ir/process.rs` and `math/ir/multiline.rs`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
//! Spacing between items by their classes, and linebreaks and alignment points into rows and columns: a
//! port of Typst 0.15's `math/ir/process.rs` and `multiline.rs`.

use crate::eval::styles::StyleChain;
use crate::math::class::MathClass;
use crate::math::item::{
	FencedBody,
	Item,
	Raw,
	Row,
};
use crate::math::resolve::{
	fenced,
	shared_sizing,
	MEDIUM,
	THICK,
	THIN,
};
use crate::math::style::MathSize;
use crate::syntax::Span;

/// Items with their spacing decided: flat, or split into rows at linebreaks.
pub enum Group {
	Flat(Vec<Item>),
	Multiline(Vec<Row>),
}

/// Spaces and splits a run of raw items. `closing`: a closing delimiter follows; `pad`: rows are padded
/// to a common column count; `split`: alignment points split columns even without a linebreak.
pub fn process_group(items: Vec<Raw>, styles: &StyleChain, closing: bool, pad: bool, split: bool) -> Group {
	let pre = preprocess(items, closing, false);
	if pre.linebreaks > 0 || (split && pre.has_align) {
		let mut rows = Vec::new();
		let mut row = Vec::new();
		for raw in pre.items.into_iter().chain(std::iter::once(Raw::Linebreak)) {
			match raw {
				Raw::Linebreak	=> rows.push(split_at_align(std::mem::take(&mut row), styles)),
				other			=> row.push(other),
			}
		}
		if pad {
			let n = rows.iter().map(|r: &Row| r.len()).max().unwrap_or(0);
			for r in rows.iter_mut() {
				r.pad_to(n, styles);
			}
		}
		Group::Multiline(rows)
	} else {
		Group::Flat(pre.items.into_iter().filter_map(Raw::into_item).collect())
	}
}

/// A table cell's items: linebreaks stripped (and reported), split at alignment points.
pub fn process_table_cell(items: Vec<Raw>, styles: &StyleChain) -> (Row, bool) {
	let pre = preprocess(items, false, true);
	let row = if pre.has_align {
		split_at_align(pre.items, styles)
	} else {
		Row(vec![Item::wrap(pre.items.into_iter().filter_map(Raw::into_item).collect(), styles)])
	};
	(row, pre.had_linebreaks)
}

struct Pre {
	items:			Vec<Raw>,
	had_linebreaks:	bool,
	has_align:		bool,
	linebreaks:		u32,
}

fn last_index(v: &[Raw]) -> Option<usize> { v.iter().rposition(|i| !i.is_ignorant()) }

fn preprocess(items: Vec<Raw>, closing: bool, strip_linebreaks: bool) -> Pre {
	let mut out: Vec<Raw> = Vec::with_capacity(items.len());
	let mut last: Option<usize> = None;
	let mut space: Option<Item> = None;
	let mut had_linebreaks = false;
	let mut has_align = false;
	let mut linebreaks = 0u32;
	for raw in items {
		let mut item = match raw {
			Raw::Item(Item::Tag(t)) => {
				out.push(Raw::Item(Item::Tag(t)));
				continue;
			}
			Raw::Item(Item::Space) => {
				if last.is_some() {
					space = Some(Item::Space);
				}
				continue;
			}
			Raw::Item(Item::Spacing(w, weak)) => {
				last = None;
				space = None;
				if weak {
					let li = last_index(&out);
					match li.and_then(|i| out.get_mut(i)) {
						None => continue,
						Some(Raw::Item(Item::Spacing(pw, true))) => {
							if *pw < w {
								*pw = w;
							}
							continue;
						}
						Some(_) => (),
					}
				}
				out.push(Raw::Item(Item::Spacing(w, weak)));
				continue;
			}
			Raw::Align => {
				has_align = true;
				out.push(Raw::Align);
				continue;
			}
			Raw::Linebreak => {
				had_linebreaks = true;
				if strip_linebreaks {
					continue;
				}
				linebreaks += 1;
				out.push(Raw::Linebreak);
				space = None;
				last = None;
				continue;
			}
			Raw::Item(i) => i,
		};
		// A varying operator becomes binary after an operand.
		if item.class() == MathClass::Vary {
			if let Some(Raw::Item(prev)) = last.and_then(|i| out.get(i)) {
				if matches!(prev.class(), MathClass::Normal | MathClass::Alphabetic | MathClass::Closing | MathClass::Fence) {
					item.set_class(MathClass::Binary);
				}
			}
		}
		if !item.is_ignorant() {
			if let Some(i) = last {
				let s = match out.get_mut(i) {
					Some(Raw::Item(prev))	=> spacing(prev, space.take(), &mut item),
					_						=> None,
				};
				if let Some(s) = s {
					out.insert(i + 1, Raw::Item(s));
				}
			}
			last = Some(out.len());
		}
		out.push(Raw::Item(item));
	}
	// Closing punctuation takes thin space; a trailing weak space goes.
	let mut applied = false;
	if closing {
		if let Some(i) = last_index(&out) {
			if let Some(Raw::Item(item)) = out.get_mut(i) {
				if item.rclass() == MathClass::Punctuation && item.size().map(|s| s > MathSize::Script).unwrap_or(true) {
					item.set_rspace(Some(THIN));
					applied = true;
				}
			}
		}
	}
	if !applied {
		if let Some(i) = last_index(&out) {
			if matches!(out.get(i), Some(Raw::Item(Item::Spacing(_, true)))) {
				out.remove(i);
			}
		}
	}
	if !closing {
		if let Some(i) = last_index(&out) {
			if matches!(out.get(i), Some(Raw::Linebreak)) {
				out.remove(i);
				linebreaks = linebreaks.saturating_sub(1);
			}
		}
	}
	Pre { items: out, had_linebreaks, has_align, linebreaks }
}

// Typst's class-pair spacing; `space` is a source space, kept only between spaced items.
fn spacing(l: &mut Item, space: Option<Item>, r: &mut Item) -> Option<Item> {
	use MathClass as C;
	let script = |f: &Item| f.size().map(|s| s <= MathSize::Script).unwrap_or(false);
	match (l.rclass(), r.lclass()) {
		(_, C::Punctuation)							=> None,
		(C::Punctuation, _) if !script(l)			=> { l.set_rspace(Some(THIN)); None }
		(C::Opening, _) | (_, C::Closing)			=> None,
		(C::Relation, C::Relation)					=> None,
		(C::Relation, _) if !script(l)				=> { l.set_rspace(Some(THICK)); None }
		(_, C::Relation) if !script(r)				=> { r.set_lspace(Some(THICK)); None }
		(C::Binary, _) if !script(l)				=> { l.set_rspace(Some(MEDIUM)); None }
		(_, C::Binary) if !script(r)				=> { r.set_lspace(Some(MEDIUM)); None }
		(C::Large, C::Opening | C::Fence)			=> None,
		(C::Large, _)								=> { l.set_rspace(Some(THIN)); None }
		(_, C::Large)								=> { r.set_lspace(Some(THIN)); None }
		_											=> spaced(l, r, space),
	}
}

fn spaced(l: &Item, r: &Item, space: Option<Item>) -> Option<Item> {
	if l.is_spaced() || r.is_spaced() { space } else { None }
}

/// Splits items at alignment points into columns, marking an infix operator that opens a left-aligned
/// column so its left space moves to the right-aligned column before it.
pub fn split_at_align(items: Vec<Raw>, styles: &StyleChain) -> Row {
	let mut cols: Vec<Vec<Item>> = vec![Vec::new()];
	let mut at_boundary = false;
	for raw in items {
		match raw {
			Raw::Align => {
				cols.push(Vec::new());
				at_boundary = true;
			}
			Raw::Linebreak => (),
			Raw::Item(mut item) => {
				if at_boundary && !item.is_ignorant() {
					if cols.len() % 2 == 0 && matches!(item.class(), MathClass::Relation | MathClass::Binary) {
						if let Item::Comp(c) = &mut item {
							c.props.align_form_infix = true;
						}
					}
					at_boundary = false;
				}
				if let Some(col) = cols.last_mut() {
					col.push(item);
				}
			}
		}
	}
	Row(cols.into_iter().map(|c| Item::wrap(c, styles)).collect())
}

/// A fence whose body breaks over rows: each row's columns become fenced segments sharing one size,
/// the opening delimiter on the first and the closing on the last.
pub fn expand_multiline_fence(
	rows:	Vec<Row>,
	open:	Option<Item>,
	close:	Option<Item>,
	styles:	&StyleChain,
	span:	Span,
)
	-> Vec<Raw>
{
	let nrows = rows.len();
	let mut bodies = Vec::new();
	let mut lengths = Vec::with_capacity(nrows);
	for r in rows {
		lengths.push(r.len());
		bodies.extend(r.0);
	}
	let sizing = shared_sizing(bodies, styles);
	let mut open = open;
	let mut close = close;
	let mut out = Vec::new();
	let mut index = 0;
	for (ri, &ncols) in lengths.iter().enumerate() {
		if ri > 0 {
			out.push(Raw::Linebreak);
		}
		for ci in 0..ncols {
			if ci > 0 {
				out.push(Raw::Align);
			}
			let first	= ri == 0 && ci == 0;
			let last	= ri + 1 == nrows && ci + 1 == ncols;
			let o = if first { open.take() } else { None };
			let c = if last { close.take() } else { None };
			out.push(Raw::Item(fenced(o, c, FencedBody::Shared { index, sizing: sizing.clone() }, true, styles, span)));
			index += 1;
		}
	}
	out
}
