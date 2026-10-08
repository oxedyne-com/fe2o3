// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-layout `lists.rs`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
// U6b owns this file: the layout routine of bullet and numbered lists.
//
// A port of Typst 0.15.1's `layout_list` and `layout_enum`: each item is a marker and a body laid out side
// by side, the markers measured first so they align to the widest, the bodies measured too when the list may
// not fill its region, and the items stacked by the stack layouter with the gutter between them. A marker
// sits on the first baseline of its body, or against the body's first frame when its alignment says top,
// horizon or bottom. The model's show (`eval::lib::model::list`) resolves the markers, numbers and bodies and
// hands them over in the element's `laid` field.

use crate::eval::content::{
	Content,
	ElemKind,
};
use crate::eval::locate::{
	Locator,
	Place,
};
use crate::eval::styles::StyleChain;
use crate::eval::value::{
	Direction,
	Value,
};
use crate::eval::Engine;
use crate::flow::block::{
	abs_of,
	field,
	layout_fragment_in,
	layout_frame_in,
	rel_of,
	rtl,
	Frame,
	Item,
	Regions,
	Rel,
	Spacing,
	Stacker,
};

use oxedyne_fe2o3_core::prelude::*;

/// A marker and the body it leads, each realised at a place of its own under the list's, so that the several
/// times an item is measured and laid out locate what is in it alike.
struct Entry {
	marker:			Content,
	body:			Content,
	marker_place:	Option<Place>,
	body_place:		Option<Place>,
}

/// What every item of one list shares.
struct Lister {
	indent:			f64,			// from the text start to the marker
	body_indent:	f64,			// between the marker and the body
	baseline:		bool,			// the marker sits on the body's first baseline
	rtl:			bool,
	marker_width:	f64,			// the widest marker, so they align to each other
	body_width:		Option<f64>,	// the widest body, when the list may not fill its region
}

/// Lays a shown `list` or `enum` out into `regions`: one frame for each region it spans.
pub fn layout(engine: &mut Engine, elem: &Content, styles: &StyleChain, regions: &Regions) -> Outcome<Vec<Frame>> {
	let laid = match res!(field(elem, styles, "laid")) {
		Some(Value::Dict(d))	=> d,
		_						=> return Err(err!("A {} reached layout without being shown, so it has no items.",
			elem.kind().map(|k| k.path()).unwrap_or("list"); Bug)),
	};
	let mut items = Vec::new();
	let mut places = elem.place().map(Locator::new);
	if let Some(Value::Array(rows)) = laid.get("items") {
		for row in rows.iter() {
			if let Value::Array(pair) = row {
				if let (Some(Value::Content(m)), Some(Value::Content(b))) = (pair.get(0), pair.get(1)) {
					let body_place = places.as_mut().map(|l| l.next(ElemKind::ListItem, b.fingerprint()));
					let marker_place = places.as_mut().map(|l| l.next(ElemKind::Align, m.fingerprint()));
					items.push(Entry { marker: m.clone(), body: b.clone(), marker_place, body_place });
				}
			}
		}
	}
	let gutter = match laid.get("gutter").and_then(|g| rel_of(styles, g)) {
		Some(r)	=> r,
		None	=> Rel::zero(),
	};
	let length = |name: &str| -> Outcome<f64> {
		Ok(match res!(field(elem, styles, name)) {
			Some(Value::Length(l))	=> abs_of(styles, l),
			_						=> 0.0,
		})
	};
	let mut lister = Lister {
		indent:			res!(length("indent")),
		body_indent:	res!(length("body-indent")),
		baseline:		matches!(laid.get("baseline"), Some(Value::Bool(true))),
		rtl:			res!(rtl(styles)),
		marker_width:	0.0,
		body_width:		None,
	};
	lister.marker_width = res!(measure_markers(engine, &lister, &items, styles, regions));
	if !regions.w.is_finite() || !regions.expand_x {
		lister.body_width = Some(res!(measure_bodies(engine, &lister, &items, styles, regions)));
	}
	let mut stack = Stacker::new(Direction::Ttb, regions.clone(), elem.span());
	let mut first = true;
	for entry in &items {
		if !first {
			stack.spacing(Spacing::Rel(gutter), styles);
		}
		first = false;
		res!(stack.custom(engine, styles, |engine, regions| layout_item(engine, &lister, entry, styles, regions)));
	}
	stack.finish(engine)
}

/// The widest marker, so they may align horizontally to each other.
fn measure_markers(
	engine:		&mut Engine,
	lister:		&Lister,
	items:		&[Entry],
	styles:		&StyleChain,
	regions:	&Regions,
)
	-> Outcome<f64>
{
	let avail = regions.w - lister.indent - lister.body_indent;
	let mut width = 0.0f64;
	for item in items {
		let frame = res!(layout_frame_in(engine, item.marker_place, &item.marker, styles,
			Regions::one(avail, f64::INFINITY, false, false)));
		width = width.max(frame.w);
	}
	Ok(width.min(avail))
}

/// The widest body. A list of infinite width, or one that may not expand, could not align its bodies to the
/// region's edge, so they align to each other instead. Marker and body together that exceed the width leave
/// the marker what it asked for and the body the rest.
fn measure_bodies(
	engine:		&mut Engine,
	lister:		&Lister,
	items:		&[Entry],
	styles:		&StyleChain,
	regions:	&Regions,
)
	-> Outcome<f64>
{
	let avail = regions.w - lister.indent - lister.body_indent;
	let mut width = 0.0f64;
	for item in items {
		let frame = res!(layout_frame_in(engine, item.body_place, &item.body, styles,
			Regions::one(avail, f64::INFINITY, false, false)));
		width = width.max(frame.w);
	}
	Ok(width.min(avail - lister.marker_width))
}

/// Does the first frame hold nothing but tags, with a later one that holds more? That is a forced region
/// break, which the marker ignores.
fn skip_first(frames: &[Frame]) -> bool {
	frames.len() > 1 && tags_only(&frames[0]) && frames.iter().skip(1).any(|f| !tags_only(f))
}

fn tags_only(frame: &Frame) -> bool {
	frame.items.iter().all(|(_, _, i)| matches!(i, Item::Tag(_)))
}

/// One item: the marker laid out against the body, with vertical marker alignment.
fn layout_item(
	engine:		&mut Engine,
	lister:		&Lister,
	item:		&Entry,
	styles:		&StyleChain,
	regions:	&Regions,
)
	-> Outcome<Vec<Frame>>
{
	let total = lister.indent + lister.marker_width + lister.body_indent;
	// The body is restricted to the space the marker and indents leave.
	let mut body_regions = regions.clone();
	match lister.body_width {
		Some(w)	=> {
			body_regions.w = w;
			body_regions.expand_x = true;
		},
		None	=> body_regions.w -= total,
	}
	let mut marker = res!(layout_frame_in(engine, item.marker_place, &item.marker, styles,
		Regions::one(lister.marker_width, regions.base().1, true, false)));
	let mut body = res!(layout_fragment_in(engine, item.body_place, &item.body, styles, body_regions.clone()));
	// The first frame that is not only tags.
	let mut first = if skip_first(&body) { 1 } else { 0 };
	let (mut body_y, mut marker_y) = (0.0, 0.0);
	if lister.baseline {
		// A positive difference means the marker is above and moves down; a negative one means the body is
		// above, so it is laid out again with that much less room and moved down.
		let diff = match body.get(first) {
			Some(f) if marker.has_baseline() && f.has_baseline()	=> f.baseline() - marker.baseline(),
			_														=> 0.0,
		};
		if diff >= 0.0 {
			marker_y = diff;
		} else {
			let mut shorter = body_regions.clone();
			shorter.h += diff;
			body = res!(layout_fragment_in(engine, item.body_place, &item.body, styles, shorter));
			if skip_first(&body) {
				first = 1;
			}
			body_y = -diff;
		}
	} else {
		// Explicit marker alignment: the marker is laid out again as tall as the body's first frame, so it
		// aligns itself within it.
		let height = match body.get(first) {
			Some(f)	=> f.h.max(marker.h),
			None	=> marker.h,
		};
		marker = res!(layout_frame_in(engine, item.marker_place, &item.marker, styles,
			Regions::one(lister.marker_width, height, true, true)));
	}
	// The marker joins the first frame that is not only tags, and the whole body is indented after it.
	let mut frames = Vec::with_capacity(body.len());
	for (i, body_frame) in body.into_iter().enumerate() {
		let width = total + body_frame.w;
		let mut height = body_frame.h + body_y;
		if i == first {
			height = height.max(marker.h + marker_y);
		}
		let mut frame = Frame::new(width, height);
		// In a right-to-left list the items expand to the left, so the body sits at the frame's left edge.
		let body_x = if lister.rtl { 0.0 } else { total };
		if i == first {
			let marker_x = if lister.rtl { width - (lister.marker_width + lister.indent) } else { lister.indent };
			frame.push_frame(marker_x, marker_y, marker.clone());
		}
		frame.push_frame(body_x, body_y, body_frame);
		frames.push(frame);
	}
	Ok(frames)
}
