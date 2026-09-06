//! Evaluating the figures the two books draw by code -- Typst CeTZ / Fletcher diagrams written inline in
//! the document rather than exported to an image.
//!
//! Three constructs are read, the subset those figures actually use: a Fletcher `diagram` of grid-placed
//! `node`s and chained/feedback `edge`s (the stochastic-model flowchart), a cetz-plot `chart.barchart`
//! (the productivity-gap bars), and a cetz-plot `plot.plot` of one or more line series (the
//! productivity-versus-wages plot). Each is parsed from the `#figure` body's source into a builder from
//! [`crate::diagram`] or [`crate::plot`], which draws it as real vector ink. Anything outside this subset
//! is left to the caller's placeholder, so an unhandled figure keeps its space and its caption.

use crate::diagram::{
	Diagram,
	DiagramStyle,
	Endpoint,
};
use crate::diagram::layout::Route;
use crate::diagram::shape::Shape;
use crate::ir::{
	Graphic,
	Sp,
};
use crate::plot::{
	nice_bar_axis,
	AxisStyle,
	BarChart,
	Plot,
	Series,
};

use super::parse::{
	call_inner,
	named_arg,
	read_group,
	split_top_args,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::set::FontSet;
use oxedyne_fe2o3_graphics::colour::Rgba;

use std::collections::HashMap;
use std::sync::Arc;

/// Points per centimetre, the unit a cetz canvas measures in by default, so a `size: (8, 4)` becomes a
/// figure eight by four centimetres.
const CM: f64 = 72.0 / 2.54;

/// The em the flowchart is set at, in points. Fletcher's `spacing` and node sizes are given in em; the
/// diagram sets its labels at this size, so one em stands for one label size throughout.
const EM: f64 = 11.0;

/// A figure the document draws by code. Each arm carries a ready builder; [`build`](CodeFigure::build)
/// turns it into the same [`Graphic`] a raster or an SVG figure produces, so it places identically.
#[derive(Clone, Debug)]
pub enum CodeFigure {
	Flowchart { diagram: Diagram, style: DiagramStyle },
	Bars(BarChart),
	Lines(Plot),
}

impl CodeFigure {
	pub fn build(&self, fonts: Arc<FontSet>) -> Outcome<Graphic> {
		match self {
			CodeFigure::Flowchart { diagram, style }	=> diagram.build(fonts, style),
			CodeFigure::Bars(chart)						=> chart.build(fonts),
			CodeFigure::Lines(plot)						=> plot.build(fonts),
		}
	}
}

/// Parses a `#figure` body's source into a [`CodeFigure`], or `None` when the body is not one of the code
/// figures this reader draws (an image, a table, or a construct outside the subset). The three kinds are
/// told apart by the call each uses: a Fletcher `diagram(...)`, a cetz-plot `barchart(...)`, or a
/// cetz-plot `plot.plot(...)`.
pub(crate) fn parse_code_figure(text: &str) -> Option<CodeFigure> {
	if text.contains("diagram(") {
		if let Some(cf) = parse_flowchart(text) {
			return Some(cf);
		}
	}
	if text.contains("barchart") {
		if let Some(cf) = parse_barchart(text) {
			return Some(cf);
		}
	}
	if text.contains("plot.add") || text.contains("plot.plot") {
		if let Some(cf) = parse_lineplot(text) {
			return Some(cf);
		}
	}
	None
}

// ---- Fletcher flowchart ----------------------------------------------------------------------------

/// One node gathered from the diagram source before it is placed.
struct NodeDef {
	row:	i64,
	label:	String,
	shape:	Shape,
	fill:	Option<Rgba>,
	w_em:	Option<f64>,
	h_em:	Option<f64>,
}

/// A resolved edge: an index pair, its optional branch label, and, for a feedback loop, how far right it
/// detours (in grid columns).
enum EdgeDef {
	Chain    { from: usize, to: usize, label: Option<String> },
	Feedback { from: usize, to: usize, label: Option<String>, cols: i64 },
}

/// Parses a Fletcher `diagram(...)` of grid-placed nodes and chained/feedback edges into a flowchart. The
/// nodes lie in one column, so they are placed top to bottom with the gap between two nodes set by the
/// difference of their grid rows; a plain `edge` chains one node to the next, and an `edge` given a
/// direction path (`"r,r,u,u,l,l"`) is a feedback loop back up to the node the path's `u` steps reach.
fn parse_flowchart(text: &str) -> Option<CodeFigure> {
	let inner	= call_inner(text, "diagram")?;
	let args	= split_top_args(&inner);

	let mut spacing_em	= 1.0f64;
	let mut node_stroke	= 1.0f32;
	let mut nodes:	Vec<NodeDef>	= Vec::new();
	let mut edges:	Vec<EdgeDef>	= Vec::new();
	// Chain edges wait for the next node to be declared, so their target resolves in one forward pass.
	let mut pending:	Vec<(usize, Option<String>)>	= Vec::new();
	let mut last_node:	Option<usize>					= None;

	for arg in &args {
		let a = arg.trim();
		if a.is_empty() {
			continue;
		}
		// A diagram-level named argument (spacing, node-stroke, debug), told apart from a node/edge call by
		// having a top-level colon.
		if let Some((key, val)) = named_arg(a) {
			match key.as_str() {
				"spacing"		=> if let Some(v) = em_value(&val) { spacing_em = v; },
				"node-stroke"	=> if let Some(v) = pt_value(&val) { node_stroke = v as f32; },
				_				=> {},
			}
			continue;
		}
		if a.starts_with("node(") {
			if let Some(nd) = parse_node(a) {
				let idx = nodes.len();
				nodes.push(nd);
				// Resolve every chain edge waiting on the next node.
				for (from, label) in pending.drain(..) {
					edges.push(EdgeDef::Chain { from, to: idx, label });
				}
				last_node = Some(idx);
			}
			continue;
		}
		if a.starts_with("edge(") {
			let from = match last_node {
				Some(i)	=> i,
				None	=> continue,	// an edge before any node has nothing to leave from
			};
			let (route, label) = parse_edge(a);
			match route {
				ParsedRoute::Chain => pending.push((from, label)),
				ParsedRoute::Feedback { u, cols } => {
					let target_row = nodes[from].row - u;
					if let Some(to) = nodes.iter().position(|n| n.row == target_row) {
						edges.push(EdgeDef::Feedback { from, to, label, cols });
					}
				},
			}
			continue;
		}
	}

	if nodes.is_empty() {
		return None;
	}

	let spacing_pt = spacing_em * EM;

	let mut d = Diagram::new();
	for (i, nd) in nodes.iter().enumerate() {
		let id = fmt!("n{}", i);
		if i == 0 {
			d.node_at(id.clone(), nd.label.clone(), Sp::ZERO, Sp::ZERO, nd.shape);
		} else {
			let delta	= (nd.row - nodes[i - 1].row).max(1);
			let gap		= Sp::from_pt(spacing_pt * delta as f64);
			let prev	= fmt!("n{}", i - 1);
			d.node_below(id.clone(), nd.label.clone(), &prev, gap, nd.shape);
		}
		if let Some(f) = nd.fill {
			d.fill(f);
		}
		match (nd.w_em, nd.h_em) {
			(Some(w), Some(h))	=> { d.size(Sp::from_pt(w * EM), Sp::from_pt(h * EM)); },
			(Some(w), None)		=> { d.size(Sp::from_pt(w * EM), Sp::ZERO); },
			(None, Some(h))		=> { d.size(Sp::ZERO, Sp::from_pt(h * EM)); },
			(None, None)		=> {},
		}
	}
	for e in &edges {
		match e {
			EdgeDef::Chain { from, to, label } => {
				d.edge_near(
					Endpoint::node(fmt!("n{}", from)),
					Endpoint::node(fmt!("n{}", to)),
					label.as_deref(),
					Route::Straight);
			},
			EdgeDef::Feedback { from, to, label, cols } => {
				let out = Sp::from_pt(spacing_pt * (*cols).max(1) as f64);
				d.edge_near(
					Endpoint::node(fmt!("n{}", from)),
					Endpoint::node(fmt!("n{}", to)),
					label.as_deref(),
					Route::Feedback { out });
			},
		}
	}

	let mut style = DiagramStyle::default();
	style.node_fill		= None;	// an unfilled node is white; every filled node carries its own wash
	style.node_stroke	= node_stroke;
	style.label_size	= Sp::from_pt(EM);
	Some(CodeFigure::Flowchart { diagram: d, style })
}

/// Parses one `node((c, r), label, fill: .., shape: .., width: .., height: ..)` call.
fn parse_node(arg: &str) -> Option<NodeDef> {
	let inner	= call_inner(arg, "node")?;
	let parts	= split_top_args(&inner);
	let mut coord:	Option<String>	= None;
	let mut label:	Option<String>	= None;
	let mut shape	= Shape::Box;
	let mut fill:	Option<Rgba>	= None;
	let mut w_em:	Option<f64>		= None;
	let mut h_em:	Option<f64>		= None;
	for p in &parts {
		let pt = p.trim();
		if pt.is_empty() {
			continue;
		}
		if let Some((key, val)) = named_arg(pt) {
			match key.as_str() {
				"fill"		=> fill = resolve_colour(&val),
				"shape"		=> shape = resolve_shape(&val),
				"width"		=> w_em = em_value(&val),
				"height"	=> h_em = em_value(&val),
				_			=> {},
			}
			continue;
		}
		if coord.is_none() {
			coord = Some(pt.to_string());
		} else if label.is_none() {
			label = Some(clean_label(pt));
		}
	}
	let coord	= coord?;
	let row		= grid_row(&coord)?;
	Some(NodeDef { row, label: label.unwrap_or_default(), shape, fill, w_em, h_em })
}

/// A parsed edge's routing: a plain chain to the next node, or a feedback loop with its `u` (rows up) and
/// column detour count read from the direction path.
enum ParsedRoute {
	Chain,
	Feedback { u: i64, cols: i64 },
}

/// Parses one `edge(...)` call into its route and optional branch label. A first positional that is a
/// direction path (`"r,r,u,u,l,l"`) marks a feedback loop; otherwise it is a marks string (`"-|>"`) and
/// the edge chains to the next node. A bracketed positional (`[N]`) is the branch label.
fn parse_edge(arg: &str) -> (ParsedRoute, Option<String>) {
	let inner = match call_inner(arg, "edge") {
		Some(i)	=> i,
		None	=> return (ParsedRoute::Chain, None),
	};
	let parts = split_top_args(&inner);
	let mut positionals: Vec<String> = Vec::new();
	for p in &parts {
		let pt = p.trim();
		if pt.is_empty() || named_arg(pt).is_some() {
			continue;	// label-pos and the rest do not change the topology
		}
		positionals.push(pt.to_string());
	}
	let mut label: Option<String> = None;
	for p in &positionals {
		if p.starts_with('[') {
			label = Some(clean_label(p));
		}
	}
	if let Some(first) = positionals.first() {
		if let Some(path) = direction_path(first) {
			let u		= path.iter().filter(|&&c| c == 'u').count() as i64;
			let cols	= path.iter().filter(|&&c| c == 'r').count() as i64;
			return (ParsedRoute::Feedback { u, cols }, label);
		}
	}
	(ParsedRoute::Chain, label)
}

/// The direction letters of a route path (`"r,r,u,u,l,l"`), or `None` when the string is a marks spec
/// (`"-|>"`) rather than a path. A path is a comma list of the single letters r/l/u/d.
fn direction_path(s: &str) -> Option<Vec<char>> {
	let inner = unquote(s);
	let mut out = Vec::new();
	for part in inner.split(',') {
		let t = part.trim();
		if t.len() != 1 {
			return None;
		}
		match t.chars().next() {
			Some(c @ ('r' | 'l' | 'u' | 'd'))	=> out.push(c),
			_									=> return None,
		}
	}
	if out.is_empty() {
		None
	} else {
		Some(out)
	}
}

/// The grid row of a `(c, r)` coordinate: the second component, rounded to an integer.
fn grid_row(coord: &str) -> Option<i64> {
	let chars: Vec<char> = coord.chars().collect();
	let (inner, _) = read_group(&chars, chars.iter().position(|&c| c == '(')?)?;
	let comps = split_top_args(&inner);
	let r = comps.get(1)?.trim();
	r.parse::<f64>().ok().map(|v| v.round() as i64)
}

/// Maps a Fletcher shape argument to a [`Shape`]: `shapes.hexagon`/`hexagon` to a hexagon, `diamond` to a
/// diamond, anything else (the default rectangle) to a box.
fn resolve_shape(val: &str) -> Shape {
	let v = val.trim();
	if v.ends_with("hexagon") {
		Shape::Hexagon
	} else if v.ends_with("diamond") {
		Shape::Diamond
	} else if v.ends_with("pill") || v.ends_with("stadium") {
		Shape::Stadium
	} else {
		Shape::Box
	}
}

// ---- cetz-plot bar chart ---------------------------------------------------------------------------

/// Parses a cetz-plot `chart.barchart(...)` inside a `cetz.canvas` block into a [`BarChart`]. The bar
/// data is a `let`-bound array of `([label], value)` tuples referenced by name in the call; the value
/// axis is sized to the data with a nice tick step.
fn parse_barchart(text: &str) -> Option<CodeFigure> {
	let block	= canvas_block(text)?;
	let lets	= let_bindings(&block);
	let inner	= call_inner(&block, "barchart")?;
	let parts	= split_top_args(&inner);

	let mut data_expr:	Option<String>	= None;
	let mut bar_frac	= 0.8f64;
	let mut x_label:	Option<String>	= None;
	let mut size:		Option<(f64, f64)>	= None;
	for p in &parts {
		let pt = p.trim();
		if pt.is_empty() {
			continue;
		}
		if let Some((key, val)) = named_arg(pt) {
			match key.as_str() {
				"bar-width"	=> if let Some(v) = plain_f64(&val) { bar_frac = v; },
				"x-label"	=> x_label = content_opt(&val),
				"size"		=> size = pair_f64(&val),
				_			=> {},
			}
			continue;
		}
		data_expr = Some(pt.to_string());	// the last positional is the data array
	}

	let data_src = match data_expr {
		Some(name) => lets.get(name.trim()).cloned().unwrap_or(name),
		None       => return None,
	};
	let bars = parse_bar_data(&data_src);
	if bars.is_empty() {
		return None;
	}

	let (w, h)			= size.unwrap_or((8.0, 4.0));
	let data_max		= bars.iter().fold(0.0f64, |m, (_, v)| m.max(*v));
	let (x_max, x_ticks)	= nice_bar_axis(data_max);

	Some(CodeFigure::Bars(BarChart {
		width:		(w * CM) as f32,
		height:		(h * CM) as f32,
		bars,
		x_max,
		x_ticks,
		x_label,
		bar_frac,
		fills:		bar_palette(),
	}))
}

/// Parses a `(([US], 60), ([UK], 25), ...)` array into label/value pairs, in order.
fn parse_bar_data(src: &str) -> Vec<(String, f64)> {
	let chars: Vec<char> = src.trim().chars().collect();
	let open = match chars.iter().position(|&c| c == '(') {
		Some(i)	=> i,
		None	=> return Vec::new(),
	};
	let inner = match read_group(&chars, open) {
		Some((s, _))	=> s,
		None			=> return Vec::new(),
	};
	let mut out = Vec::new();
	for entry in split_top_args(&inner) {
		let ec: Vec<char> = entry.trim().chars().collect();
		let eo = match ec.iter().position(|&c| c == '(') {
			Some(i)	=> i,
			None	=> continue,
		};
		let einner = match read_group(&ec, eo) {
			Some((s, _))	=> s,
			None			=> continue,
		};
		let fields = split_top_args(&einner);
		if fields.len() < 2 {
			continue;
		}
		let label = clean_label(fields[0].trim());
		if let Some(v) = plain_f64(fields[1].trim()) {
			out.push((label, v));
		}
	}
	out
}

/// A red-family palette cycled across the bars, echoing cetz-plot's default warm sequence closely enough
/// for the pattern to read; the exact hues are not load-bearing.
fn bar_palette() -> Vec<Rgba> {
	vec![
		Rgba::opaque(0xf4, 0xb8, 0xb8),
		Rgba::opaque(0xe8, 0x7d, 0x7d),
		Rgba::opaque(0xd6, 0x4a, 0x4a),
		Rgba::opaque(0xb8, 0x2a, 0x2a),
		Rgba::opaque(0x8f, 0x1d, 0x1d),
	]
}

// ---- cetz-plot line plot ---------------------------------------------------------------------------

/// Parses a cetz-plot `plot.plot(...)` inside a `cetz.canvas` block into a [`Plot`] of line series. Each
/// `plot.add` in the plot body names a `let`-bound array of `(x, y)` samples, a label and a style whose
/// dash marks the series dashed; the axis ranges, tick steps and legend come from the call's arguments.
fn parse_lineplot(text: &str) -> Option<CodeFigure> {
	let block	= canvas_block(text)?;
	let lets	= let_bindings(&block);
	let inner	= call_inner(&block, "plot")?;
	let parts	= split_top_args(&inner);

	let mut x_min = 0.0;	let mut x_max = 1.0;
	let mut y_min = 0.0;	let mut y_max = 1.0;
	let mut x_step:	Option<f64>	= None;
	let mut y_step:	Option<f64>	= None;
	let mut size:	Option<(f64, f64)>	= None;
	let mut legend	= false;
	let mut left	= false;
	let mut body:	Option<String>	= None;
	for p in &parts {
		let pt = p.trim();
		if pt.is_empty() {
			continue;
		}
		if let Some((key, val)) = named_arg(pt) {
			match key.as_str() {
				"x-min"			=> if let Some(v) = plain_f64(&val) { x_min = v; },
				"x-max"			=> if let Some(v) = plain_f64(&val) { x_max = v; },
				"y-min"			=> if let Some(v) = plain_f64(&val) { y_min = v; },
				"y-max"			=> if let Some(v) = plain_f64(&val) { y_max = v; },
				"x-tick-step"	=> x_step = plain_f64(&val),
				"y-tick-step"	=> y_step = plain_f64(&val),
				"size"			=> size = pair_f64(&val),
				"legend"		=> legend = val.trim() != "none",
				"axis-style"	=> left = unquote(&val) == "left",
				_				=> {},
			}
			continue;
		}
		if pt.starts_with('{') {
			body = Some(pt.to_string());	// the plot body block, holding the plot.add calls
		}
	}

	let body	= body?;
	let series	= parse_plot_adds(&body, &lets);
	if series.is_empty() {
		return None;
	}

	let (w, h)	= size.unwrap_or((8.0, 5.0));
	let x_ticks	= ticks(x_min, x_max, x_step);
	let y_ticks	= ticks(y_min, y_max, y_step);

	// The overall figure is the plot area plus the margins the plot builder reserves for labels.
	Some(CodeFigure::Lines(Plot {
		width:		(w * CM) as f32 + 48.0,
		height:		(h * CM) as f32 + 34.0,
		x_range:	(x_min, x_max),
		y_range:	(y_min, y_max),
		x_ticks,
		y_ticks,
		series,
		axis:		if left { AxisStyle::Left } else { AxisStyle::Framed },
		x_label:	None,
		y_label:	None,
		legend,
	}))
}

/// Parses the `plot.add(...)` calls in a plot body into line series, resolving each data reference against
/// the `let` bindings.
fn parse_plot_adds(body: &str, lets: &HashMap<String, String>) -> Vec<Series> {
	let mut out = Vec::new();
	let chars: Vec<char> = body.chars().collect();
	let mut from = 0usize;
	// Walk every `plot.add(` in order; call_inner from a moving offset would re-find the first, so scan by
	// hand for the literal and read each group.
	while let Some(pos) = find_from(&chars, "plot.add(", from) {
		let open = pos + "plot.add".chars().count();
		let (inner, after) = match read_group(&chars, open) {
			Some(t)	=> t,
			None	=> break,
		};
		from = after;
		let parts = split_top_args(&inner);
		let mut label:	Option<String>	= None;
		let mut dashed	= false;
		let mut width	= 1.4f32;
		let mut data_expr:	Option<String>	= None;
		for p in &parts {
			let pt = p.trim();
			if pt.is_empty() {
				continue;
			}
			if let Some((key, val)) = named_arg(pt) {
				match key.as_str() {
					"label"	=> label = content_opt(&val),
					"style"	=> {
						if val.contains("dash") { dashed = true; }
						if let Some(t) = thickness_pt(&val) { width = t as f32; }
					},
					_		=> {},
				}
				continue;
			}
			data_expr = Some(pt.to_string());
		}
		let data_src = match data_expr {
			Some(name) => lets.get(name.trim()).cloned().unwrap_or(name),
			None       => continue,
		};
		let points = parse_xy(&data_src);
		if points.is_empty() {
			continue;
		}
		out.push(Series { points, colour: Rgba::opaque(20, 20, 20), width, dashed, label });
	}
	out
}

/// Parses an `((x, y), (x, y), ...)` array into sample points, in order.
fn parse_xy(src: &str) -> Vec<(f64, f64)> {
	let chars: Vec<char> = src.trim().chars().collect();
	let open = match chars.iter().position(|&c| c == '(') {
		Some(i)	=> i,
		None	=> return Vec::new(),
	};
	let inner = match read_group(&chars, open) {
		Some((s, _))	=> s,
		None			=> return Vec::new(),
	};
	let mut out = Vec::new();
	for entry in split_top_args(&inner) {
		let ec: Vec<char> = entry.trim().chars().collect();
		let eo = match ec.iter().position(|&c| c == '(') {
			Some(i)	=> i,
			None	=> continue,
		};
		let einner = match read_group(&ec, eo) {
			Some((s, _))	=> s,
			None			=> continue,
		};
		let fields = split_top_args(&einner);
		if fields.len() < 2 {
			continue;
		}
		if let (Some(x), Some(y)) = (plain_f64(fields[0].trim()), plain_f64(fields[1].trim())) {
			out.push((x, y));
		}
	}
	out
}

// ---- shared helpers --------------------------------------------------------------------------------

/// The inner code of a `cetz.canvas({ ... })` block, its outer braces stripped, or `None`.
fn canvas_block(text: &str) -> Option<String> {
	let group = call_inner(text, "canvas")?;	// "{ ... }", the paren group after canvas(
	let trimmed = group.trim();
	let inner = trimmed.strip_prefix('{')?.strip_suffix('}')?;
	Some(inner.to_string())
}

/// The `let name = <group>` bindings in a code block, mapped name to the raw group source (parens kept),
/// so an array binding can be resolved where it is referenced by name.
fn let_bindings(block: &str) -> HashMap<String, String> {
	let mut out = HashMap::new();
	let chars: Vec<char> = block.chars().collect();
	let mut i = 0usize;
	while let Some(pos) = find_from(&chars, "let ", i) {
		let mut j = pos + 4;
		let start = j;
		while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == '-') {
			j += 1;
		}
		let name: String = chars[start..j].iter().collect();
		// Skip whitespace, then require '='.
		while j < chars.len() && chars[j].is_whitespace() {
			j += 1;
		}
		if chars.get(j) != Some(&'=') {
			i = pos + 4;
			continue;
		}
		j += 1;
		while j < chars.len() && chars[j].is_whitespace() {
			j += 1;
		}
		if chars.get(j) == Some(&'(') {
			if let Some((inner, after)) = read_group(&chars, j) {
				out.insert(name, fmt!("({})", inner));
				i = after;
				continue;
			}
		}
		i = pos + 4;
	}
	out
}

/// The index of the first occurrence of the literal `pat` in `chars` at or after `from`, or `None`.
fn find_from(chars: &[char], pat: &str, from: usize) -> Option<usize> {
	let p: Vec<char> = pat.chars().collect();
	if p.is_empty() || chars.len() < p.len() {
		return None;
	}
	(from..=chars.len().saturating_sub(p.len())).find(|&s| chars[s..s + p.len()] == p[..])
}

/// The tick values within `[min, max]` at multiples of `step`, the way cetz-plot marks an axis: the first
/// tick is the lowest multiple of the step not below `min`, so a range of 90 to 170 stepped by 20 marks
/// 100, 120, 140, 160 rather than 90, 110, .... With no step, just the endpoints.
fn ticks(min: f64, max: f64, step: Option<f64>) -> Vec<f64> {
	match step {
		Some(s) if s > 0.0 => {
			let mut out = Vec::new();
			let mut v = (min / s).ceil() * s;
			while v <= max + s * 0.001 {
				out.push((v * 1e6).round() / 1e6);
				v += s;
			}
			out
		},
		_ => vec![min, max],
	}
}

/// Cleans a node or cell label to plain, single- or multi-line text: `align(...)[...]` is unwrapped, the
/// outer brackets are dropped, a `\` line break becomes a newline, and a `$...$` maths span is reduced to
/// readable Unicode (`gamma` to the Greek letter, `bold`/`underline` to their argument).
fn clean_label(raw: &str) -> String {
	let mut s = raw.trim().to_string();
	// Unwrap an `align(...)[...]` wrapper to its bracketed body.
	if s.starts_with("align(") {
		if let Some(b) = bracket_body(&s) {
			s = b;
		}
	}
	// Drop the outer content brackets.
	let t = s.trim();
	if t.starts_with('[') && t.ends_with(']') {
		s = t[1..t.len() - 1].to_string();
	}
	// Line breaks: a backslash followed by whitespace or end of string.
	let s = break_lines(&s);
	// Reduce maths spans, then collapse runs of spaces on each line.
	let lines: Vec<String> = s.split('\n').map(|line| {
		let m = reduce_math(line);
		m.split_whitespace().collect::<Vec<_>>().join(" ")
	}).collect();
	lines.join("\n")
}

/// The `[...]` body at the end of an `align(...)[...]` call, or `None`.
fn bracket_body(s: &str) -> Option<String> {
	let chars: Vec<char> = s.chars().collect();
	let open = chars.iter().position(|&c| c == '[')?;
	read_group(&chars, open).map(|(inner, _)| inner)
}

/// Replaces Typst content line breaks (`\`) with newlines. A backslash that begins a maths escape inside
/// a `$...$` span is left alone, but node labels here use `\` only as a break, so a plain replace serves.
fn break_lines(s: &str) -> String {
	let mut out		= String::new();
	let chars: Vec<char> = s.chars().collect();
	let mut i = 0;
	while i < chars.len() {
		if chars[i] == '\\' {
			out.push('\n');
			i += 1;
			// Swallow the space that usually follows a break.
			while i < chars.len() && chars[i] == ' ' {
				i += 1;
			}
			continue;
		}
		out.push(chars[i]);
		i += 1;
	}
	out
}

/// Reduces the `$...$` maths spans in a line to readable Unicode, leaving the surrounding text as is.
fn reduce_math(line: &str) -> String {
	let mut out		= String::new();
	let chars: Vec<char> = line.chars().collect();
	let mut i = 0;
	while i < chars.len() {
		if chars[i] == '$' {
			let mut j = i + 1;
			let mut expr = String::new();
			while j < chars.len() && chars[j] != '$' {
				expr.push(chars[j]);
				j += 1;
			}
			out.push_str(&reduce_expr(&expr));
			i = if j < chars.len() { j + 1 } else { j };
			continue;
		}
		out.push(chars[i]);
		i += 1;
	}
	out
}

/// Reduces one maths expression to Unicode: `bold(X)`/`underline(X)`/`upright(X)` to their argument, and a
/// Greek name to its letter. Enough for the flowchart's `bold(D)` and `underline(gamma)`.
fn reduce_expr(expr: &str) -> String {
	let mut s = expr.trim().to_string();
	for f in ["bold", "underline", "upright", "italic", "arrow"] {
		while let Some(inner) = strip_call(&s, f) {
			s = inner;
		}
	}
	let s = s.trim();
	match s {
		"gamma"		=> "\u{03b3}".to_string(),
		"Gamma"		=> "\u{0393}".to_string(),
		"alpha"		=> "\u{03b1}".to_string(),
		"beta"		=> "\u{03b2}".to_string(),
		"delta"		=> "\u{03b4}".to_string(),
		"Delta"		=> "\u{0394}".to_string(),
		"phi"		=> "\u{03c6}".to_string(),
		"Phi"		=> "\u{03a6}".to_string(),
		"psi"		=> "\u{03c8}".to_string(),
		"eta"		=> "\u{03b7}".to_string(),
		"zeta"		=> "\u{03b6}".to_string(),
		other		=> other.to_string(),
	}
}

/// If `s` is exactly `name(<inner>)`, its inner; else `None`.
fn strip_call(s: &str, name: &str) -> Option<String> {
	let t = s.trim();
	let pref = fmt!("{}(", name);
	if !t.starts_with(&pref) || !t.ends_with(')') {
		return None;
	}
	Some(t[pref.len()..t.len() - 1].to_string())
}

/// The text of a `[...]` content argument, or `None` for `none`/empty.
fn content_opt(val: &str) -> Option<String> {
	let v = val.trim();
	if v == "none" || v.is_empty() {
		return None;
	}
	let cleaned = clean_label(v);
	if cleaned.is_empty() {
		None
	} else {
		Some(cleaned)
	}
}

/// Strips a pair of surrounding `"..."` quotes, returning the trimmed content or the trimmed input.
fn unquote(s: &str) -> String {
	let t = s.trim();
	if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
		t[1..t.len() - 1].to_string()
	} else {
		t.to_string()
	}
}

/// A colour expression resolved to an [`Rgba`]: a base colour (`colours.light`, `blue`, ...) optionally
/// lightened or darkened, matching the small palette these figures draw with.
fn resolve_colour(expr: &str) -> Option<Rgba> {
	let e = expr.trim();
	// Split off a single `.lighten(N%)` / `.darken(N%)` modifier.
	let (base, modifier) = if let Some(pos) = e.find(".lighten(") {
		(&e[..pos], Some(("lighten", &e[pos + ".lighten(".len()..])))
	} else if let Some(pos) = e.find(".darken(") {
		(&e[..pos], Some(("darken", &e[pos + ".darken(".len()..])))
	} else {
		(e, None)
	};
	let mut colour = base_colour(base)?;
	if let Some((kind, rest)) = modifier {
		let pct = rest.trim_end_matches(')').trim().trim_end_matches('%').trim().parse::<f64>().ok()? / 100.0;
		colour = match kind {
			"lighten"	=> lighten(colour, pct),
			"darken"	=> darken(colour, pct),
			_			=> colour,
		};
	}
	Some(colour)
}

/// The named base colours these books use: the custom `colours.*` dictionary and the Typst built-ins the
/// figures reach for directly.
fn base_colour(name: &str) -> Option<Rgba> {
	match name.trim() {
		"colours.light"		=> Some(Rgba::opaque(0xE9, 0xEC, 0xEF)),
		"colours.green"		=> Some(Rgba::opaque(0x00, 0x7c, 0x77)),
		"colours.purple"	=> Some(Rgba::opaque(0x4c, 0x1a, 0x57)),
		"colours.pink"		=> Some(Rgba::opaque(0xff, 0x3c, 0xc7)),
		"colours.yellow"	=> Some(Rgba::opaque(0xf0, 0xf6, 0x00)),
		"colours.aqua"		=> Some(Rgba::opaque(0x00, 0xe5, 0xe8)),
		"blue"				=> Some(Rgba::opaque(0x00, 0x74, 0xd9)),
		"orange"			=> Some(Rgba::opaque(0xff, 0x85, 0x1b)),
		"green"				=> Some(Rgba::opaque(0x2e, 0xcc, 0x40)),
		"red"				=> Some(Rgba::opaque(0xff, 0x41, 0x36)),
		"aqua"				=> Some(Rgba::opaque(0x7f, 0xdb, 0xff)),
		"yellow"			=> Some(Rgba::opaque(0xff, 0xdc, 0x00)),
		"purple"			=> Some(Rgba::opaque(0xb1, 0x0d, 0xc9)),
		"gray" | "grey"		=> Some(Rgba::opaque(0xaa, 0xaa, 0xaa)),
		"black"				=> Some(Rgba::BLACK),
		"white"				=> Some(Rgba::WHITE),
		_					=> None,
	}
}

/// Blends a colour toward white by `p` (0 to 1), Typst's `lighten`.
fn lighten(c: Rgba, p: f64) -> Rgba {
	let f = |v: u8| -> u8 { (v as f64 + (255.0 - v as f64) * p).round().clamp(0.0, 255.0) as u8 };
	Rgba::new(f(c.r), f(c.g), f(c.b), c.a)
}

/// Blends a colour toward black by `p` (0 to 1), Typst's `darken`.
fn darken(c: Rgba, p: f64) -> Rgba {
	let f = |v: u8| -> u8 { (v as f64 * (1.0 - p)).round().clamp(0.0, 255.0) as u8 };
	Rgba::new(f(c.r), f(c.g), f(c.b), c.a)
}

/// An `Nem` length in em, or `None`.
fn em_value(val: &str) -> Option<f64> {
	let v = val.trim();
	v.strip_suffix("em").and_then(|n| n.trim().parse::<f64>().ok())
}

/// An `Npt` length in points, or a bare number as points, or `None`.
fn pt_value(val: &str) -> Option<f64> {
	let v = val.trim();
	if let Some(n) = v.strip_suffix("pt") {
		return n.trim().parse::<f64>().ok();
	}
	v.parse::<f64>().ok()
}

/// A `thickness: Npt` inside a style value, in points.
fn thickness_pt(val: &str) -> Option<f64> {
	let idx = val.find("thickness")?;
	let rest = &val[idx + "thickness".len()..];
	let colon = rest.find(':')?;
	let after = &rest[colon + 1..];
	let num: String = after.trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
	num.parse::<f64>().ok()
}

/// A plain number, ignoring a trailing unit, or `None`.
fn plain_f64(val: &str) -> Option<f64> {
	let v = val.trim();
	let num: String = v.chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-').collect();
	num.parse::<f64>().ok()
}

/// A `(a, b)` pair of numbers, or `None`.
fn pair_f64(val: &str) -> Option<(f64, f64)> {
	let chars: Vec<char> = val.trim().chars().collect();
	let open = chars.iter().position(|&c| c == '(')?;
	let (inner, _) = read_group(&chars, open)?;
	let parts = split_top_args(&inner);
	let a = plain_f64(parts.first()?.trim())?;
	let b = plain_f64(parts.get(1)?.trim())?;
	Some((a, b))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn clean_label_reduces_maths_and_breaks() {
		assert_eq!(clean_label(r"[Initial LLM $bold(D)$ \ and scaled $underline(gamma)$]"),
			"Initial LLM D\nand scaled \u{03b3}");
		assert_eq!(clean_label(r"align(center)[Finished \ SA loop?]"), "Finished\nSA loop?");
		assert_eq!(clean_label("[Add next system]"), "Add next system");
	}

	#[test]
	fn direction_path_tells_route_from_marks() {
		assert_eq!(direction_path("\"r,r,u,u,l,l\""), Some(vec!['r', 'r', 'u', 'u', 'l', 'l']));
		assert_eq!(direction_path("\"-|>\""), None);
	}

	#[test]
	fn colours_resolve_and_lighten() -> Outcome<()> {
		assert_eq!(resolve_colour("colours.light"), Some(Rgba::opaque(0xE9, 0xEC, 0xEF)));
		// green #007c77 lightened 50% blends halfway to white.
		let g = res!(resolve_colour("colours.green.lighten(50%)").ok_or_else(|| err!("no colour"; Bug)));
		assert_eq!(g, Rgba::opaque(0x80, 0xbe, 0xbb));
		Ok(())
	}

	#[test]
	fn bar_axis_and_ticks_align_to_step() {
		let (max, bticks) = nice_bar_axis(60.0);
		assert_eq!(max, 60.0);
		assert_eq!(bticks, vec![0.0, 6.0, 12.0, 18.0, 24.0, 30.0, 36.0, 42.0, 48.0, 54.0, 60.0]);
		// A range not starting on a step multiple marks the multiples inside it, not the endpoints.
		assert_eq!(ticks(90.0, 170.0, Some(20.0)), vec![100.0, 120.0, 140.0, 160.0]);
		assert_eq!(ticks(1979.0, 2020.0, Some(10.0)), vec![1980.0, 1990.0, 2000.0, 2010.0, 2020.0]);
	}

	#[test]
	fn parses_barchart_data_in_order() {
		let src = "(([US], 60), ([UK], 25), ([NZ], 20))";
		let bars = parse_bar_data(src);
		assert_eq!(bars, vec![
			("US".to_string(), 60.0),
			("UK".to_string(), 25.0),
			("NZ".to_string(), 20.0),
		]);
	}

	#[test]
	fn parses_xy_samples() {
		let pts = parse_xy("((1979, 100), (1983, 105))");
		assert_eq!(pts, vec![(1979.0, 100.0), (1983.0, 105.0)]);
	}

	#[test]
	fn top_level_dispatch_picks_the_right_figure() {
		let bar = r#"align(center, cetz.canvas({
			import cetz.draw: *
			import "@preview/cetz-plot:0.1.1": chart
			let data = (([US], 60), ([UK], 25))
			chart.barchart(mode: "basic", size: (8, 4), label-key: 0, value-key: 1,
				bar-width: 0.6, x-label: [%], y-label: none, data)
		}))"#;
		match parse_code_figure(bar) {
			Some(CodeFigure::Bars(b)) => {
				assert_eq!(b.bars.len(), 2);
				assert_eq!(b.bars[0], ("US".to_string(), 60.0));
				assert_eq!(b.x_max, 60.0);
			},
			other => panic!("expected a bar chart, got {:?}", other.is_some()),
		}

		let line = r#"align(center, cetz.canvas({
			import "@preview/cetz-plot:0.1.1": plot
			let a = ((1979, 100), (2019, 160))
			let b = ((1979, 100), (2019, 104))
			plot.plot(axis-style: "left", x-min: 1979, x-max: 2020, y-min: 90, y-max: 170,
				size: (8, 5), x-tick-step: 10, y-tick-step: 20, legend: (1.0, 5.0),
				{
					plot.add(style: (stroke: (paint: black, thickness: 1.5pt)), label: [Productivity], a)
					plot.add(style: (stroke: (paint: black, thickness: 1.5pt, dash: "dashed")), label: [Median hourly wages], b)
				})
		}))"#;
		match parse_code_figure(line) {
			Some(CodeFigure::Lines(p)) => {
				assert_eq!(p.series.len(), 2);
				assert!(!p.series[0].dashed);
				assert!(p.series[1].dashed);
				assert_eq!(p.axis, AxisStyle::Left);
				assert!(p.legend);
				assert_eq!(p.x_ticks, vec![1980.0, 1990.0, 2000.0, 2010.0, 2020.0]);
			},
			other => panic!("expected a line plot, got {:?}", other.is_some()),
		}

		let flow = r#"align(center, [#diagram(
			spacing: 1.5em, node-stroke: 1pt,
			node((0,2), [Start], fill: colours.light, shape: shapes.hexagon),
			edge("-|>"),
			node((0,4), align(center)[Decide?], fill: blue.lighten(50%), shape: diamond, width: 6em, height: 3.25em),
			edge("r,r,u,u,l,l", "-|>", [N]),
			edge("-|>", [Y]),
			node((0,6), [End]),
		)])"#;
		assert!(matches!(parse_code_figure(flow), Some(CodeFigure::Flowchart { .. })));
	}
}
