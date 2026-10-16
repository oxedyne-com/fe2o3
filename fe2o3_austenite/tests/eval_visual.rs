//! U6d: visual elements (shapes, curves, images, transforms, hide, repeat) laid out by `flow::visual`,
//! checked against the `typst` 0.15 oracle. Each fixture under `tests/fixtures/eval/visual/` is compiled by
//! `typst` to SVG at test time; the SVG's drawn paths, stroke widths, colours and images are compared, op by
//! op, with the frame Austenite lays out for the same element, and the page size with the frame's size.
//! The element is built here through `content::construct`, as the evaluator will build it from the call.
//! A missing `typst` fails the suite unless `EVAL_ORACLE_SKIP=1` is set.

use oxedyne_fe2o3_austenite::diag::DiagnosticKind;
use oxedyne_fe2o3_austenite::eval::args::Args;
use oxedyne_fe2o3_austenite::eval::content::{
	construct,
	Content,
	ElemKind,
};
use oxedyne_fe2o3_austenite::eval::eval::{
	eval_string,
	EvalMode,
};
use oxedyne_fe2o3_austenite::eval::lib::visual;
use oxedyne_fe2o3_austenite::eval::scope::Scope;
use oxedyne_fe2o3_austenite::eval::styles::StyleChain;
use oxedyne_fe2o3_austenite::eval::value::{
	Alignment,
	Angle,
	Color,
	ColorSpace,
	Dict,
	Fraction,
	HAlign,
	Length,
	Paint,
	Ratio,
	VAlign,
	Value,
};
use oxedyne_fe2o3_austenite::eval::{
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::flow::visual::{
	frame_to_node,
	layout_frame_with,
	Frame,
	Item,
	Pod,
};
use oxedyne_fe2o3_austenite::flow::Region;
use oxedyne_fe2o3_austenite::ir::{
	Sp,
	Transform,
};
use oxedyne_fe2o3_austenite::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::path::{
	Bounds,
	Path,
};
use oxedyne_fe2o3_graphics::svg_doc::{
	self,
	SvgOp,
};
use oxedyne_fe2o3_graphics::transform::Transform as GTransform;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

const TOL: f32 = 0.02;	// points

// Values

fn pt(x: f64) -> Value { Value::Length(Length::pt(x)) }
fn pct(x: f64) -> Value { Value::Ratio(Ratio(x / 100.0)) }
fn deg(x: f64) -> Value { Value::Angle(Angle(x.to_radians())) }
fn pair(a: Value, b: Value) -> Value { Value::array(vec![a, b]) }
fn pts(a: f64, b: f64) -> Value { pair(pt(a), pt(b)) }
fn s(x: &str) -> Value { Value::str(x) }
fn rgb(r: u8, g: u8, b: u8) -> Value {
	Value::Color(Color { space: ColorSpace::Rgb, c: [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 0.0], alpha: 1.0 })
}
fn dict(kv: Vec<(&str, Value)>) -> Value {
	let mut d = Dict::new();
	for (k, v) in kv {
		d.insert(k, v);
	}
	Value::dict(d)
}
fn stroke_of(thickness: f64, colour: Value) -> Value { dict(vec![("thickness", pt(thickness)), ("paint", colour)]) }
fn align(x: Option<HAlign>, y: Option<VAlign>) -> Value { Value::Alignment(Alignment { x, y }) }

fn engine() -> Engine { Engine::new(World::new(PathBuf::from("/"))) }

fn el(e: &mut Engine, kind: ElemKind, pos: Vec<Value>, named: Vec<(&str, Value)>) -> Outcome<Content> {
	let mut args = Args::new(Span::detached());
	for v in pos {
		args.push(Span::detached(), v);
	}
	for (k, v) in named {
		args.push_named(Span::detached(), k, v);
	}
	construct(e, kind, &mut args)
}

fn c(v: Content) -> Value { Value::Content(v) }

// The PNG the image fixtures name, kept beside them so the fixtures stand alone: 20 by 10 pixels,
// each distinct, so a crop that is off shows.
fn png_bytes() -> Vec<u8> {
	let p = fixture_dir().join("px.png");
	match std::fs::read(&p) {
		Ok(b)	=> b,
		Err(e)	=> panic!("{}: {}", p.display(), e),
	}
}

const SVG_SRC: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"40\" height=\"20\" viewBox=\"0 0 40 20\"><rect x=\"0\" y=\"0\" width=\"40\" height=\"20\" fill=\"#0000ff\"/></svg>";

/// The element each fixture draws, built as the evaluator builds it.
fn build(e: &mut Engine, name: &str) -> Outcome<Content> {
	use ElemKind as K;
	let blue	= rgb(0, 0, 255);
	let red		= rgb(255, 0, 0);
	let green	= rgb(0, 128, 0);
	Ok(match name {
		"rect_default"		=> res!(el(e, K::Rect, vec![], vec![])),
		"rect_fill"			=> res!(el(e, K::Rect, vec![], vec![("width", pt(20.0)), ("height", pt(10.0)), ("fill", blue)])),
		"rect_fill_stroke"	=> res!(el(e, K::Rect, vec![], vec![("width", pt(20.0)), ("height", pt(10.0)),
			("fill", blue), ("stroke", stroke_of(2.0, red))])),
		"rect_radius"		=> res!(el(e, K::Rect, vec![], vec![("width", pt(30.0)), ("height", pt(20.0)), ("radius", pt(5.0))])),
		"rect_sides"		=> res!(el(e, K::Rect, vec![], vec![("width", pt(20.0)), ("height", pt(10.0)), ("radius", pt(2.0)),
			("stroke", dict(vec![("left", stroke_of(2.0, red)), ("rest", pt(1.0))])), ("fill", blue)])),
		"rect_corners"		=> res!(el(e, K::Rect, vec![], vec![("width", pt(40.0)), ("height", pt(24.0)),
			("radius", dict(vec![("top-left", pt(8.0)), ("bottom-right", pt(3.0))])),
			("stroke", dict(vec![("top", pt(3.0)), ("bottom", stroke_of(1.0, green))]))])),
		"rect_outset"		=> res!(el(e, K::Rect, vec![], vec![("width", pt(20.0)), ("height", pt(10.0)),
			("outset", dict(vec![("x", pt(3.0)), ("y", pt(1.0))])), ("fill", blue)])),
		"rect_dashed"		=> res!(el(e, K::Rect, vec![], vec![("width", pt(20.0)), ("height", pt(10.0)),
			("stroke", dict(vec![("thickness", pt(2.0)), ("dash", s("dashed"))]))])),
		"rect_cap_round"	=> res!(el(e, K::Rect, vec![], vec![("width", pt(40.0)), ("height", pt(24.0)),
			("radius", dict(vec![("top-left", pt(10.0)), ("top-right", pt(10.0))])),
			("stroke", dict(vec![("top", dict(vec![("thickness", pt(3.0)), ("cap", s("round"))]))]))])),
		"rect_cap_square"	=> res!(el(e, K::Rect, vec![], vec![("width", pt(40.0)), ("height", pt(24.0)), ("radius", pt(10.0)),
			("stroke", dict(vec![("top", dict(vec![("thickness", pt(3.0)), ("cap", s("square"))])), ("right", pt(1.0))]))])),
		"rect_dash_sides"	=> res!(el(e, K::Rect, vec![], vec![("width", pt(40.0)), ("height", pt(24.0)), ("radius", pt(6.0)),
			("stroke", dict(vec![("left", dict(vec![("thickness", pt(2.0)), ("dash", s("dashed"))])), ("rest", pt(1.0))]))])),
		"rect_rel_radius"	=> res!(el(e, K::Rect, vec![], vec![("width", pt(30.0)), ("height", pt(20.0)), ("radius", pct(50.0)), ("fill", blue)])),
		"square_default"	=> res!(el(e, K::Square, vec![], vec![])),
		"square_size"		=> res!(el(e, K::Square, vec![], vec![("size", pt(12.0))])),
		"square_min"		=> res!(el(e, K::Square, vec![], vec![("width", pt(10.0)), ("height", pt(20.0))])),
		"circle_default"	=> res!(el(e, K::Circle, vec![], vec![])),
		"circle_radius"		=> res!(el(e, K::Circle, vec![], vec![("radius", pt(7.0)), ("fill", red), ("stroke", stroke_of(1.0, blue))])),
		"ellipse_sized"		=> res!(el(e, K::Ellipse, vec![], vec![("width", pt(40.0)), ("height", pt(10.0))])),
		"line_default"		=> res!(el(e, K::Line, vec![], vec![])),
		"line_end"			=> res!(el(e, K::Line, vec![], vec![("end", pts(20.0, 10.0))])),
		"line_angle"		=> res!(el(e, K::Line, vec![], vec![("start", pts(5.0, 5.0)), ("length", pt(20.0)),
			("angle", deg(30.0)), ("stroke", pt(3.0))])),
		"line_up"			=> res!(el(e, K::Line, vec![], vec![("end", pts(10.0, -5.0))])),
		"line_round"		=> res!(el(e, K::Line, vec![], vec![("length", pt(20.0)),
			("stroke", dict(vec![("thickness", pt(4.0)), ("cap", s("round"))]))])),
		"line_relative"		=> res!(el(e, K::Line, vec![], vec![("length", pct(50.0)), ("angle", deg(90.0))])),
		"polygon_tri"		=> res!(el(e, K::Polygon, vec![pts(0.0, 0.0), pts(20.0, 5.0), pts(5.0, 15.0)],
			vec![("fill", blue), ("stroke", stroke_of(2.0, red))])),
		"polygon_regular" | "polygon_em" => {
			let mut args = Args::new(Span::detached());
			if name == "polygon_regular" {
				args.push_named(Span::detached(), "size", pt(20.0));
				args.push_named(Span::detached(), "vertices", Value::Int(5));
				args.push_named(Span::detached(), "fill", green);
			} else {
				args.push_named(Span::detached(), "vertices", Value::Int(6));
			}
			match res!(visual::call(visual::VisualFn::PolygonRegular, e, args)) {
				Value::Content(c)	=> c,
				_					=> return Err(err!("polygon.regular did not give content"; Bug)),
			}
		},
		"curve_mixed" => {
			let m	= res!(el(e, K::CurveMove, vec![pts(0.0, 10.0)], vec![]));
			let l	= res!(el(e, K::CurveLine, vec![pts(10.0, 0.0)], vec![]));
			let q	= res!(el(e, K::CurveQuad, vec![Value::Auto, pts(20.0, 10.0)], vec![]));
			let cu	= res!(el(e, K::CurveCubic, vec![Value::None, pts(25.0, 30.0), pts(10.0, 20.0)], vec![]));
			let cl	= res!(el(e, K::CurveClose, vec![], vec![]));
			res!(el(e, K::Curve, vec![c(m), c(l), c(q), c(cu), c(cl)], vec![("fill", blue)]))
		},
		"curve_relative" => {
			let m	= res!(el(e, K::CurveMove, vec![pts(5.0, 5.0)], vec![]));
			let l	= res!(el(e, K::CurveLine, vec![pts(10.0, 0.0)], vec![("relative", Value::Bool(true))]));
			let q	= res!(el(e, K::CurveQuad, vec![pts(5.0, 10.0), pts(0.0, 10.0)], vec![("relative", Value::Bool(true))]));
			let cl	= res!(el(e, K::CurveClose, vec![], vec![("mode", s("straight"))]));
			res!(el(e, K::Curve, vec![c(m), c(l), c(q), c(cl)], vec![]))
		},
		"curve_open" => {
			let m	= res!(el(e, K::CurveMove, vec![pts(0.0, 0.0)], vec![]));
			let c1	= res!(el(e, K::CurveCubic, vec![pts(10.0, 20.0), pts(20.0, -10.0), pts(30.0, 10.0)], vec![]));
			let c2	= res!(el(e, K::CurveCubic, vec![Value::Auto, pts(40.0, 0.0), pts(50.0, 5.0)], vec![]));
			res!(el(e, K::Curve, vec![c(m), c(c1), c(c2)], vec![("stroke", pt(2.0))]))
		},
		"rect_body" => {
			let inner = res!(el(e, K::Rect, vec![], vec![("width", pt(10.0)), ("height", pt(10.0)), ("fill", blue)]));
			res!(el(e, K::Rect, vec![c(inner)], vec![("inset", pt(4.0))]))
		},
		"rect_body_sized" => {
			let inner = res!(el(e, K::Rect, vec![], vec![("width", pt(10.0)), ("height", pt(10.0)), ("fill", blue)]));
			res!(el(e, K::Rect, vec![c(inner)], vec![("width", pt(40.0)),
				("inset", dict(vec![("x", pt(2.0)), ("top", pt(6.0))]))]))
		},
		"circle_body" => {
			let inner = res!(el(e, K::Rect, vec![], vec![("width", pt(10.0)), ("height", pt(5.0)), ("fill", blue)]));
			res!(el(e, K::Circle, vec![c(inner)], vec![]))
		},
		"square_body" => {
			let inner = res!(el(e, K::Rect, vec![], vec![("width", pt(10.0)), ("height", pt(4.0)), ("fill", blue)]));
			res!(el(e, K::Square, vec![c(inner)], vec![]))
		},
		"ellipse_body" => {
			let inner = res!(el(e, K::Rect, vec![], vec![("width", pt(20.0)), ("height", pt(6.0))]));
			res!(el(e, K::Ellipse, vec![c(inner)], vec![("inset", pt(0.0))]))
		},
		"rotate_plain" | "rotate_reflow" => {
			let inner = res!(el(e, K::Rect, vec![], vec![("width", pt(20.0)), ("height", pt(10.0)), ("fill", blue)]));
			let reflow = name == "rotate_reflow";
			res!(el(e, K::Rotate, vec![deg(30.0), c(inner)], vec![("reflow", Value::Bool(reflow))]))
		},
		"rotate_origin" => {
			let inner = res!(el(e, K::Rect, vec![], vec![("width", pt(20.0)), ("height", pt(10.0))]));
			res!(el(e, K::Rotate, vec![deg(-45.0), c(inner)], vec![
				("origin", align(Some(HAlign::Left), Some(VAlign::Top))), ("reflow", Value::Bool(true))]))
		},
		"scale_half" => {
			let inner = res!(el(e, K::Rect, vec![], vec![]));
			res!(el(e, K::Scale, vec![pct(50.0), c(inner)], vec![]))
		},
		"scale_reflow" => {
			let inner = res!(el(e, K::Rect, vec![], vec![("width", pt(20.0)), ("height", pt(10.0)), ("fill", blue)]));
			res!(el(e, K::Scale, vec![c(inner)], vec![("x", pct(200.0)), ("y", pct(50.0)), ("reflow", Value::Bool(true))]))
		},
		"scale_length" => {
			let inner = res!(el(e, K::Rect, vec![], vec![("width", pt(20.0)), ("height", pt(10.0))]));
			res!(el(e, K::Scale, vec![c(inner)], vec![("x", pt(40.0)), ("y", Value::Auto), ("reflow", Value::Bool(true))]))
		},
		"skew_plain" => {
			let inner = res!(el(e, K::Rect, vec![], vec![("width", pt(20.0)), ("height", pt(10.0))]));
			res!(el(e, K::Skew, vec![c(inner)], vec![("ax", deg(20.0))]))
		},
		"skew_reflow" => {
			let inner = res!(el(e, K::Rect, vec![], vec![("width", pt(20.0)), ("height", pt(10.0)), ("fill", blue)]));
			res!(el(e, K::Skew, vec![c(inner)], vec![("ay", deg(10.0)), ("reflow", Value::Bool(true))]))
		},
		"move_plain" => {
			let inner = res!(el(e, K::Rect, vec![], vec![("width", pt(20.0)), ("height", pt(10.0))]));
			res!(el(e, K::Move, vec![c(inner)], vec![("dx", pt(5.0)), ("dy", pt(3.0))]))
		},
		"rotate_nested" => {
			let inner = res!(el(e, K::Rect, vec![], vec![("width", pt(20.0)), ("height", pt(10.0))]));
			let sc = res!(el(e, K::Scale, vec![c(inner)], vec![("x", pct(50.0)), ("y", pct(100.0)), ("reflow", Value::Bool(true))]));
			res!(el(e, K::Rotate, vec![deg(90.0), c(sc)], vec![("reflow", Value::Bool(true))]))
		},
		"hide_rect" => {
			let inner = res!(el(e, K::Rect, vec![], vec![]));
			res!(el(e, K::Hide, vec![c(inner)], vec![]))
		},
		"repeat_gap" | "repeat_nojustify" => {
			let inner = res!(el(e, K::Rect, vec![], vec![("width", pt(10.0)), ("height", pt(5.0))]));
			let mut named = vec![("gap", pt(2.0))];
			if name == "repeat_nojustify" {
				named.push(("justify", Value::Bool(false)));
			}
			res!(el(e, K::Repeat, vec![c(inner)], named))
		},
		"rect_relative"		=> res!(el(e, K::Rect, vec![], vec![("width", pct(50.0)), ("height", pt(10.0)), ("fill", blue)])),
		"image_natural"		=> res!(el(e, K::Image, vec![Value::Bytes(Arc::new(png_bytes()))], vec![])),
		"image_width"		=> res!(el(e, K::Image, vec![Value::Bytes(Arc::new(png_bytes()))], vec![("width", pt(30.0))])),
		"image_cover"		=> res!(el(e, K::Image, vec![Value::Bytes(Arc::new(png_bytes()))], vec![("width", pt(30.0)), ("height", pt(30.0))])),
		"image_contain"		=> res!(el(e, K::Image, vec![Value::Bytes(Arc::new(png_bytes()))], vec![("width", pt(30.0)), ("height", pt(30.0)), ("fit", s("contain"))])),
		"image_stretch"		=> res!(el(e, K::Image, vec![Value::Bytes(Arc::new(png_bytes()))], vec![("width", pt(30.0)), ("height", pt(30.0)), ("fit", s("stretch"))])),
		"image_svg"			=> res!(el(e, K::Image, vec![Value::Bytes(Arc::new(SVG_SRC.as_bytes().to_vec()))], vec![])),
		other				=> return Err(err!("no builder for fixture {}", other; Bug, Missing)),
	})
}

// The page width a fixture sets, when not `auto`.
fn page_width(src: &str) -> Option<f64> {
	let i = src.find("width: ")?;
	let rest = &src[i + 7..];
	let end = rest.find("pt")?;
	rest[..end].trim().parse::<f64>().ok()
}

// Lays a body out as the test's flow: a visual element through `layout_frame_with`, recursively; the empty
// body as an empty frame. Anything else needs U6a/U6b's flow and is not a fixture here.
fn body_fn(e: &mut Engine, body: &Content, styles: &StyleChain, pod: Pod) -> Outcome<Frame> {
	if body.is_empty() {
		return Ok(Frame::new(if pod.expand_x { pod.w } else { 0.0 }, if pod.expand_y { pod.h } else { 0.0 }));
	}
	let mut f = res!(layout_frame_with(e, body, styles, pod.region(), &mut body_fn));
	if pod.expand_x && pod.w.is_finite() {
		f.width = pod.w;
	}
	if pod.expand_y && pod.h.is_finite() {
		f.height = pod.h;
	}
	Ok(f)
}

// Comparison

#[derive(Clone, Debug)]
enum Op {
	Fill { b: Bounds, colour: Option<String> },
	Stroke { b: Bounds, width: f32, colour: Option<String> },
	Image { b: Bounds },
}

fn hex(v: &Paint) -> Option<String> {
	match v {
		Paint::Color(c) => {
			let to = |x: f32| (x * 255.0).round() as u8;
			match c.space {
				ColorSpace::Rgb		=> Some(fmt!("#{:02x}{:02x}{:02x}", to(c.c[0]), to(c.c[1]), to(c.c[2]))),
				ColorSpace::Luma	=> Some(fmt!("#{:02x}{:02x}{:02x}", to(c.c[0]), to(c.c[0]), to(c.c[0]))),
				_					=> None,
			}
		},
		_ => None,
	}
}

fn gt(t: &Transform) -> GTransform {
	GTransform { a: t.a as f32, b: t.b as f32, c: t.c as f32, d: t.d as f32, e: t.e as f32, f: t.f as f32 }
}

// The bounds of what a path draws. Typst's SVG opens every path with `M 0 0` and then a relative move,
// so a move followed by another move draws nothing and is left out.
fn bounds(p: &Path) -> Outcome<Bounds> {
	use oxedyne_fe2o3_graphics::path::Seg;
	let segs = p.segs();
	let mut b: Option<Bounds> = None;
	let mut add = |x: f32, y: f32| {
		let pb = Bounds::new(x, y, x, y);
		b = Some(match b {
			Some(cur)	=> Bounds::new(cur.x0.min(x), cur.y0.min(y), cur.x1.max(x), cur.y1.max(y)),
			None		=> pb,
		});
	};
	for (i, s) in segs.iter().enumerate() {
		match s {
			Seg::MoveTo(pt) => {
				if !matches!(segs.get(i + 1), Some(Seg::MoveTo(_)) | None) {
					add(pt.x, pt.y);
				}
			},
			Seg::LineTo(pt)			=> add(pt.x, pt.y),
			Seg::QuadTo(_, pt)		=> add(pt.x, pt.y),
			Seg::CubicTo(_, _, pt)	=> add(pt.x, pt.y),
			Seg::Close				=> (),
		}
	}
	// Curves bulge past their end points; the flattened bounds catch that, begun from the drawn points.
	if let Some(fb) = p.bounds(&GTransform::IDENTITY) {
		if let Some(cur) = b {
			let dangling = segs.first().map(|s| matches!(s, Seg::MoveTo(_))).unwrap_or(false)
				&& matches!(segs.get(1), Some(Seg::MoveTo(_)));
			if !dangling {
				b = Some(Bounds::new(cur.x0.min(fb.x0), cur.y0.min(fb.y0), cur.x1.max(fb.x1), cur.y1.max(fb.y1)));
			} else {
				let mut rest = oxedyne_fe2o3_graphics::path::PathBuilder::new();
				for s in &segs[1..] {
					match *s {
						Seg::MoveTo(q)			=> rest.move_to(q),
						Seg::LineTo(q)			=> rest.line_to(q),
						Seg::QuadTo(c, q)		=> rest.quad_to(c, q),
						Seg::CubicTo(a, c, q)	=> rest.cubic_to(a, c, q),
						Seg::Close				=> rest.close(),
					}
				}
				if let Ok(rp) = rest.finish() {
					if let Some(rb) = rp.bounds(&GTransform::IDENTITY) {
						b = Some(Bounds::new(cur.x0.min(rb.x0), cur.y0.min(rb.y0), cur.x1.max(rb.x1), cur.y1.max(rb.y1)));
					}
				}
			}
		}
	}
	b.ok_or_else(|| err!("empty path"; Bug))
}

// Every op the frame draws, in page coordinates and paint order.
fn frame_ops(f: &Frame, t: GTransform, out: &mut Vec<Op>) -> Outcome<()> {
	for (p, item) in &f.items {
		let it = GTransform::translate(p.x as f32, p.y as f32).then(&t);
		match item {
			Item::Shape(sh) => {
				if std::env::var("VISUAL_DEBUG").is_ok() {
					eprintln!("shape {:?}", sh.curve);
				}
				let path = res!(res!(sh.curve.to_path(Default::default())).transform(&it));
				if let Some(fill) = &sh.fill {
					out.push(Op::Fill { b: res!(bounds(&path)), colour: hex(fill) });
				}
				if let Some(st) = &sh.stroke {
					out.push(Op::Stroke { b: res!(bounds(&path)), width: st.thickness as f32 * it.scale_factor(), colour: hex(&st.paint) });
				}
			},
			Item::Raster { w, h, .. } => {
				let a = it.apply(oxedyne_fe2o3_graphics::path::Pt::new(0.0, 0.0));
				let b = it.apply(oxedyne_fe2o3_graphics::path::Pt::new(*w as f32, *h as f32));
				out.push(Op::Image { b: Bounds::new(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y)) });
			},
			Item::Ops(ops) => for op in ops {
				match op {
					oxedyne_fe2o3_austenite::ir::DrawOp::Fill { path, colour } => out.push(Op::Fill {
						b: res!(bounds(&res!(path.transform(&it)))), colour: Some(colour.to_rgba().to_hex()[..7].to_lowercase()) }),
					_ => return Err(err!("unexpected op kind in an image"; Bug)),
				}
			},
			Item::Nodes { .. }	=> return Err(err!("a fixture produced laid-out nodes"; Bug)),
			Item::Group { frame, transform, .. } => res!(frame_ops(frame, gt(transform).then(&it), out)),
		}
	}
	Ok(())
}

// The oracle's ops, an image clipped to the page as Typst clips a covering image.
fn svg_ops(pic: &svg_doc::SvgPicture) -> Outcome<Vec<Op>> {
	let mut out = Vec::new();
	for op in &pic.ops {
		match op {
			SvgOp::Fill { path, colour }			=> out.push(Op::Fill { b: res!(bounds(path)), colour: Some(colour.to_hex()[..7].to_lowercase()) }),
			SvgOp::Stroke { path, colour, stroke }	=> out.push(Op::Stroke { b: res!(bounds(path)), width: stroke.width, colour: Some(colour.to_hex()[..7].to_lowercase()) }),
			SvgOp::Image { x, y, w, h, .. } => {
				let b = Bounds::new(*x, *y, x + w, y + h).intersect(Bounds::new(0.0, 0.0, pic.width, pic.height));
				out.push(Op::Image { b });
			},
			SvgOp::Text { .. } => return Err(err!("the oracle drew text"; Bug)),
		}
	}
	Ok(out)
}

fn close(a: &Bounds, b: &Bounds) -> bool {
	(a.x0 - b.x0).abs() <= TOL && (a.y0 - b.y0).abs() <= TOL && (a.x1 - b.x1).abs() <= TOL && (a.y1 - b.y1).abs() <= TOL
}

fn compare(name: &str, mine: &[Op], oracle: &[Op]) -> Vec<String> {
	let mut errs = Vec::new();
	if mine.len() != oracle.len() {
		errs.push(fmt!("{}: {} ops, oracle {}\n  mine   {:?}\n  oracle {:?}", name, mine.len(), oracle.len(), mine, oracle));
		return errs;
	}
	for (i, (m, o)) in mine.iter().zip(oracle).enumerate() {
		let ok = match (m, o) {
			(Op::Fill { b: a, colour: ca }, Op::Fill { b, colour: cb }) =>
				close(a, b) && (ca.is_none() || cb.is_none() || ca == cb),
			(Op::Stroke { b: a, width: wa, colour: ca }, Op::Stroke { b, width: wb, colour: cb }) =>
				close(a, b) && (wa - wb).abs() < 1e-3 && (ca.is_none() || cb.is_none() || ca == cb),
			(Op::Image { b: a }, Op::Image { b }) => close(a, b),
			_ => false,
		};
		if !ok {
			errs.push(fmt!("{} op {}: mine {:?}, oracle {:?}", name, i, m, o));
		}
	}
	errs
}

fn fixture_dir() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval/visual")
}

fn work_dir() -> Outcome<PathBuf> {
	let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("eval_visual");
	res!(std::fs::create_dir_all(&d).map_err(|e| err!(e, "work dir"; IO)));
	Ok(d)
}

fn typst_available() -> bool {
	Command::new("typst").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

// Compiles a fixture with `typst` and reads its SVG.
fn oracle(name: &str, src: &str) -> Outcome<svg_doc::SvgPicture> {
	let dir = res!(work_dir());
	let typ = dir.join(fmt!("{}.typ", name));
	let svg = dir.join(fmt!("{}.svg", name));
	res!(std::fs::write(&typ, src).map_err(|e| err!(e, "write fixture"; IO)));
	res!(std::fs::copy(fixture_dir().join("px.png"), dir.join("px.png")).map_err(|e| err!(e, "copy png"; IO)));
	let out = res!(Command::new("typst").arg("compile").arg(&typ).arg(&svg).output()
		.map_err(|e| err!(e, "run typst"; IO)));
	if !out.status.success() {
		return Err(err!("typst failed on {}: {}", name, String::from_utf8_lossy(&out.stderr); Input, Invalid));
	}
	let text = res!(std::fs::read_to_string(&svg).map_err(|e| err!(e, "read svg"; IO)));
	svg_doc::read_document(&text)
}

#[test]
fn visual_fixtures_match_the_typst_oracle() -> Outcome<()> {
	if !typst_available() {
		if std::env::var("EVAL_ORACLE_SKIP").as_deref() == Ok("1") {
			return Ok(());
		}
		return Err(err!("typst is not on PATH; set EVAL_ORACLE_SKIP=1 to skip the oracle deliberately"; Missing));
	}
	let mut names: Vec<String> = Vec::new();
	for entry in res!(std::fs::read_dir(fixture_dir()).map_err(|e| err!(e, "fixture dir"; IO))) {
		let p = res!(entry.map_err(|e| err!(e, "fixture entry"; IO))).path();
		if p.extension().and_then(|x| x.to_str()) == Some("typ") {
			if let Some(stem) = p.file_stem().and_then(|x| x.to_str()) {
				names.push(stem.to_string());
			}
		}
	}
	names.sort();
	assert!(names.len() >= 50, "expected the whole visual fixture set, found {}", names.len());
	let mut errs = Vec::new();
	for name in &names {
		let src = res!(std::fs::read_to_string(fixture_dir().join(fmt!("{}.typ", name))).map_err(|e| err!(e, "read"; IO)));
		let pic = res!(oracle(name, &src));
		let mut e = engine();
		let elem = res!(build(&mut e, name));
		let w = match page_width(src.lines().next().unwrap_or("")) {
			Some(w)	=> Sp::from_pt(w),
			None	=> Sp(i32::MAX),
		};
		let region	= Region { width: w, height: Sp(i32::MAX), base: (w, Sp(i32::MAX)), expand_x: false, expand_y: false };
		let frame	= match layout_frame_with(&mut e, &elem, &StyleChain::root(), region, &mut body_fn) {
			Ok(f)	=> f,
			Err(err) => {
				errs.push(fmt!("{}: layout failed: {} {:?}", name, err, e.diags));
				continue;
			},
		};
		// The page is the frame, except where the fixture fixes the page width; Typst's page is never
		// less than 1pt in either direction.
		if (frame.height.max(1.0) as f32 - pic.height).abs() > TOL
			|| (w.0 == i32::MAX && (frame.width as f32 - pic.width).abs() > TOL)
		{
			errs.push(fmt!("{}: size {}x{}, oracle {}x{}", name, frame.width, frame.height, pic.width, pic.height));
		}
		let mut mine = Vec::new();
		if let Err(err) = frame_ops(&frame, GTransform::IDENTITY, &mut mine) {
			errs.push(fmt!("{}: drawing failed: {}", name, err));
			continue;
		}
		// Typst embeds an SVG image as an SVG data URI the reader does not open; its size is the check.
		let svg_text = res!(std::fs::read_to_string(res!(work_dir()).join(fmt!("{}.svg", name))).map_err(|e| err!(e, "read"; IO)));
		if svg_text.contains("data:image/svg+xml") {
			continue;
		}
		errs.extend(compare(name, &mine, &res!(svg_ops(&pic))));
	}
	assert!(errs.is_empty(), "{} mismatches against typst:\n{}", errs.len(), errs.join("\n"));
	Ok(())
}

// Values: the casts that normalise sides, corners, sizes and scale factors, against `typst eval`.
#[test]
fn visual_field_casts_match_the_typst_oracle() -> Outcome<()> {
	if !typst_available() {
		if std::env::var("EVAL_ORACLE_SKIP").as_deref() == Ok("1") {
			return Ok(());
		}
		return Err(err!("typst is not on PATH; set EVAL_ORACLE_SKIP=1 to skip the oracle deliberately"; Missing));
	}
	// Each probe: the Typst expression, and the same element built here.
	let probe = |expr: &str| -> Outcome<String> {
		let out = res!(Command::new("typst").arg("eval").arg(expr).output().map_err(|e| err!(e, "run typst"; IO)));
		if !out.status.success() {
			return Err(err!("typst eval failed: {}", String::from_utf8_lossy(&out.stderr); Invalid));
		}
		Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
	};
	let mut e = engine();
	let keys = |v: Option<&Value>| -> String {
		match v {
			Some(Value::Dict(d)) => fmt!("[{}]", d.keys().map(|k| fmt!("\"{}\"", k)).collect::<Vec<_>>().join(",")),
			Some(other) => fmt!("\"{}\"", other.ty().name()),
			None => "null".to_string(),
		}
	};
	let compact = |s: String| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
	let mut errs = Vec::new();
	let cases: Vec<(&str, Content, &str)> = vec![
		("rect(stroke: (left: 2pt, x: red)).stroke.keys()",
			res!(el(&mut e, ElemKind::Rect, vec![], vec![("stroke", dict(vec![("left", pt(2.0)), ("x", rgb(255, 0, 0))]))])), "stroke"),
		("rect(radius: (top-left: 2pt, rest: 1pt)).radius.keys()",
			res!(el(&mut e, ElemKind::Rect, vec![], vec![("radius", dict(vec![("top-left", pt(2.0)), ("rest", pt(1.0))]))])), "radius"),
		("rect(radius: (left: 2pt, top: 1pt)).radius.keys()",
			res!(el(&mut e, ElemKind::Rect, vec![], vec![("radius", dict(vec![("left", pt(2.0)), ("top", pt(1.0))]))])), "radius"),
		("rect(inset: (x: 2pt)).inset.keys()",
			res!(el(&mut e, ElemKind::Rect, vec![], vec![("inset", dict(vec![("x", pt(2.0))]))])), "inset"),
		("str(type(rect(inset: (rest: 2pt)).inset))",
			res!(el(&mut e, ElemKind::Rect, vec![], vec![("inset", dict(vec![("rest", pt(2.0))]))])), "inset"),
		("str(type(rect(width: 1pt).width))",
			res!(el(&mut e, ElemKind::Rect, vec![], vec![("width", pt(1.0))])), "width"),
		("str(type(rect(height: 1fr).height))",
			res!(el(&mut e, ElemKind::Rect, vec![], vec![("height", Value::Fraction(Fraction(1.0)))])), "height"),
		("str(type(circle(radius: 2pt).height))",
			res!(el(&mut e, ElemKind::Circle, vec![], vec![("radius", pt(2.0))])), "height"),
		("str(type(scale(50%)[a].y))",
			res!(el(&mut e, ElemKind::Scale, vec![pct(50.0), Value::Content(Content::text("a"))], vec![])), "y"),
		("str(type(line(stroke: 2pt).stroke))",
			res!(el(&mut e, ElemKind::Line, vec![], vec![("stroke", pt(2.0))])), "stroke"),
	];
	for (expr, content, field) in cases {
		let want = compact(res!(probe(expr)));
		// Typst names the relative type "relative length"; U0's `Type::name` says "relative".
		let got = match expr.starts_with("str(type(") {
			true	=> fmt!("\"{}\"", content.field(field).map(|v| match v.ty() {
				oxedyne_fe2o3_austenite::eval::value::Type::Relative	=> "relativelength",
				t														=> t.name(),
			}).unwrap_or("none")),
			false	=> keys(content.field(field)),
		};
		if want != got {
			errs.push(fmt!("{}: typst {}, austenite {}", expr, want, got));
		}
	}
	// The sides of `(left: 2pt, x: red)`: left keeps its own 2pt, right takes x's red.
	let r = res!(el(&mut e, ElemKind::Rect, vec![], vec![("stroke", dict(vec![("left", pt(2.0)), ("x", rgb(255, 0, 0))]))]));
	if let Some(v) = r.field("stroke") {
		let sides = visual::sides_of(v);
		let left_ok = matches!(&sides[0], Some(Value::Stroke(s)) if s.thickness == Some(Length::pt(2.0)) && s.paint.is_none());
		let right_ok = matches!(&sides[2], Some(Value::Stroke(s)) if s.thickness.is_none() && s.paint.is_some());
		if !left_ok || !right_ok {
			errs.push(fmt!("stroke sides of (left: 2pt, x: red) are wrong: {:?}", sides));
		}
	}
	assert!(errs.is_empty(), "{}", errs.join("\n"));
	Ok(())
}

// `path` is the file-path type in Typst 0.15; the old drawing element is refused, as Typst refuses it.
#[test]
fn path_element_is_refused_as_in_typst() -> Outcome<()> {
	let mut e = engine();
	let r = el(&mut e, ElemKind::Path, vec![pts(0.0, 0.0), pts(1.0, 1.0)], vec![]);
	assert!(r.is_err(), "the removed path element was constructed");
	Ok(())
}

// Lowering to IR: a hidden shape is an ink-less graphic leaf of the shape's size (45pt by 30pt, as the
// oracle fixture `hide_rect` measures), its baseline at the bottom; laid-out content is overlaid in a
// vertical box of the frame's size.
#[test]
fn frames_lower_to_one_box_of_the_frame_size() -> Outcome<()> {
	use oxedyne_fe2o3_austenite::flow::visual::{
		layout_visual,
		P2,
	};
	use oxedyne_fe2o3_austenite::ir::{
		Dims,
		LeafKind,
		Node,
	};
	let mut e = engine();
	let r = res!(el(&mut e, ElemKind::Rect, vec![], vec![]));
	let h = res!(el(&mut e, ElemKind::Hide, vec![c(r)], vec![]));
	let region = Region { width: Sp(i32::MAX), height: Sp(i32::MAX), base: (Sp(i32::MAX), Sp(i32::MAX)), expand_x: false, expand_y: false };
	// `hide` shows as its body under the `hidden` style, and flow draws nothing under it.
	let shown = match res!(visual::show(&mut e, &h, &StyleChain::root())) {
		Some(s)	=> s,
		None	=> return Err(err!("hide has no show"; Bug)),
	};
	let (inner, styles) = match &shown {
		Content::Styled(st)	=> (st.child.clone(), StyleChain::root().chain(&st.styles)),
		_					=> return Err(err!("hide did not style its body"; Bug)),
	};
	assert!(res!(visual::is_hidden(&styles)));
	let node = res!(layout_visual(&mut e, &inner, &styles, region));
	match node {
		Node::Leaf(l) => {
			assert_eq!(l.dims, Dims::new(Sp::from_pt(45.0), Sp::from_pt(30.0), Sp::ZERO));
			match l.kind {
				LeafKind::Graphic(g)	=> assert!(g.ops.is_empty(), "a hidden rect drew ink"),
				_						=> panic!("not a graphic"),
			}
		},
		other => panic!("expected a leaf, got {:?}", other),
	}
	// Laid-out content keeps its nodes, placed at their offset inside a box of the frame's size.
	let mut f = Frame::new(20.0, 10.0);
	f.push(P2::new(3.0, 2.0), Item::Nodes { nodes: vec![], dims: Dims::new(Sp::from_pt(5.0), Sp::from_pt(4.0), Sp::ZERO) });
	match res!(frame_to_node(&mut e, &f, Span::detached())) {
		Node::VBox(b)	=> assert_eq!(b.dims, Dims::new(Sp::from_pt(20.0), Sp::from_pt(10.0), Sp::ZERO)),
		other			=> panic!("expected a vertical box, got {:?}", other),
	}
	Ok(())
}

// The drawing layer carries flat colours only, so a gradient fill is drawn in one colour and a tiling not
// at all. Each says so with a warning of kind `unsupported`, the kind a strict compile refuses: the
// document was not set as written.
#[test]
fn a_paint_the_drawing_layer_cannot_draw_warns_as_unsupported() -> Outcome<()> {
	let mut e = engine();
	let region = Region { width: Sp(i32::MAX), height: Sp(i32::MAX), base: (Sp(i32::MAX), Sp(i32::MAX)), expand_x: false, expand_y: false };
	let mut raised = 0;
	for paint in ["gradient.linear(red, blue)", "tiling(size: (4pt, 4pt))[x]"] {
		let fill = res!(eval_string(&mut e, paint, EvalMode::Code, Scope::new(), Span::detached()));
		let rect = res!(el(&mut e, ElemKind::Rect, vec![], vec![("width", pt(20.0)), ("height", pt(10.0)), ("fill", fill)]));
		let before = e.diags.len();
		let frame = res!(layout_frame_with(&mut e, &rect, &StyleChain::root(), region, &mut body_fn));
		// The paint is turned into ink only when the frame lowers to a node, so the warning is raised there.
		res!(frame_to_node(&mut e, &frame, Span::detached()));
		let warned: Vec<_> = e.diags[before..].iter().filter(|d| !d.is_error()).collect();
		assert!(!warned.is_empty(), "{} fill drew without a warning", paint);
		for d in &warned {
			assert_eq!(d.kind, DiagnosticKind::Unsupported, "{}: {}", paint, d.message);
			assert!(d.kind.refuses_strict(), "{}: a strict compile must refuse it", paint);
		}
		raised += warned.len();
	}
	assert!(raised >= 2, "only {} warning(s) for the two paints", raised);
	Ok(())
}
