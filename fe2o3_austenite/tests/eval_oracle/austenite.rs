// Austenite's side of every level, computed from the evaluator's own public surface: evaluate the
// fixture, run the outer fixpoint, then read the introspector (levels 1 and 2), realise the module
// once more for its structure (level 3) and read the laid-out pages (level 4).
//
// Each fixture runs on its own thread with a deadline and a panic guard, so a panic or an unbounded
// run is reported against the fixture and the run goes on; both always fail the suite. Every step
// records its own failure, so a unit whose layer works is measured even while a later layer is a
// stub.

use crate::harness::corpus::Fixture;
use crate::harness::json::J;
use crate::harness::layout::{
	APage,
	ARun,
};
use crate::harness::structure::{
	self,
	Sk,
};
use crate::harness::PosRow;

use oxedyne_fe2o3_austenite::eval::content::{
	Content,
	ElemKind,
	FieldId,
};
use oxedyne_fe2o3_austenite::eval::fixpoint::{
	self,
	Laid,
};
use oxedyne_fe2o3_austenite::eval::lib::foundations;
use oxedyne_fe2o3_austenite::eval::ops;
use oxedyne_fe2o3_austenite::eval::realise::{
	realise,
	RealiseMode,
};
use oxedyne_fe2o3_austenite::eval::select::Selector;
use oxedyne_fe2o3_austenite::eval::styles::StyleChain;
use oxedyne_fe2o3_austenite::eval::value::{
	Label,
	Module,
	Value,
};
use oxedyne_fe2o3_austenite::eval::{
	eval_source,
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::page::PlacedKind;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::mpsc;
use std::time::Duration;

pub const PROBE: &str = "probe";

/// Where level-1 values were read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeSource {
	Introspector,	// the query Typst itself answers, after layout
	Content,		// the evaluated module's content, for a fixture with no show rule or context
}

#[derive(Debug, Default)]
pub struct AusOut {
	pub eval:		Option<String>,	// the evaluation failure, if any
	pub layout:		Option<String>,	// the fixpoint failure, if any
	pub diags:		Vec<String>,
	pub first_error:	Option<(String, Option<(usize, usize)>)>,	// message, then line and column
	pub probes:		Option<std::result::Result<(Vec<J>, ProbeSource), String>>,
	pub positions:	Option<std::result::Result<Vec<PosRow>, String>>,
	pub skeleton:	Option<std::result::Result<Vec<Sk>, String>>,
	pub pages:		Option<std::result::Result<Vec<APage>, String>>,
}

pub enum RunEnd {
	Done(AusOut),
	Panic(String),
	Timeout(u64),
}

/// Which levels to compute; the harness asks only for what it will compare.
#[derive(Clone, Copy, Debug)]
pub struct Want {
	pub levels:	[bool; 4],
}

pub fn run(fx: &Fixture, want: Want, timeout: Duration) -> RunEnd {
	let (tx, rx) = mpsc::channel();
	let path = fx.path.clone();
	let root = fx.root.clone();
	let contextual = fx.contextual();
	let spawned = std::thread::Builder::new()
		.name(fmt!("eval-oracle {}", fx.id()))
		.stack_size(256 << 20)
		.spawn(move || {
			let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
				compute(&path, &root, want, contextual)
			}));
			let _ = tx.send(match r {
				Ok(out)	=> Ok(out),
				Err(p)	=> Err(panic_text(p)),
			});
		});
	if let Err(e) = spawned {
		return RunEnd::Panic(fmt!("could not start the fixture thread: {}", e));
	}
	match rx.recv_timeout(timeout) {
		Ok(Ok(out))	=> RunEnd::Done(out),
		Ok(Err(p))	=> RunEnd::Panic(p),
		Err(mpsc::RecvTimeoutError::Timeout)		=> RunEnd::Timeout(timeout.as_secs()),
		Err(mpsc::RecvTimeoutError::Disconnected)	=> RunEnd::Panic("the fixture thread ended without a result".to_string()),
	}
}

fn panic_text(p: Box<dyn std::any::Any + Send>) -> String {
	if let Some(s) = p.downcast_ref::<&str>() {
		return s.to_string();
	}
	if let Some(s) = p.downcast_ref::<String>() {
		return s.clone();
	}
	"a panic with no message".to_string()
}

fn compute(path: &std::path::Path, root: &std::path::Path, want: Want, contextual: bool) -> AusOut {
	let mut out = AusOut::default();
	let mut world = World::new(root.to_path_buf());
	let id = match world.load(path) {
		Ok(id)	=> id,
		Err(e)	=> {
			out.eval = Some(fmt!("load: {}", plain(&e)));
			return out;
		}
	};
	let mut engine = Engine::new(world);
	let module = match eval_source(&mut engine, id) {
		Ok(m)	=> m,
		Err(e)	=> {
			out.eval = Some(plain(&e));
			out.diags = render_diags(&engine);
			out.first_error = first_error(&engine);
			return out;
		}
	};
	let laid = match fixpoint::run(&mut engine, &module) {
		Ok(l)	=> Some(l),
		Err(e)	=> {
			out.layout = Some(plain(&e));
			None
		}
	};
	if want.levels[0] {
		out.probes = Some(probes(&module, laid.as_ref(), contextual));
	}
	if want.levels[1] {
		out.positions = Some(match &laid {
			Some(l)	=> positions(l),
			None	=> Err("no layout: the fixpoint did not run".to_string()),
		});
	}
	if want.levels[2] {
		out.skeleton = Some(skeleton(&mut engine, &module));
	}
	if want.levels[3] {
		out.pages = Some(match &laid {
			Some(l)	=> Ok(pages(l)),
			None	=> Err("no layout: the fixpoint did not run".to_string()),
		});
	}
	out.diags = render_diags(&engine);
	out.first_error = first_error(&engine);
	out
}

fn render_diags(engine: &Engine) -> Vec<String> {
	engine.diags.iter().take(8).map(|d| d.render(&engine.world.sources)).collect()
}

// The first error diagnostic's message and 1-based line and column, as Typst reports them.
fn first_error(engine: &Engine) -> Option<(String, Option<(usize, usize)>)> {
	engine.diags.iter().find(|d| d.is_error()).map(|d| {
		let pos = engine.world.sources.iter()
			.find(|src| src.id == d.span.file && !d.span.is_detached())
			.map(|src| src.line_col(d.span.start));
		(d.message.clone(), pos)
	})
}

// Level 1

fn probes(module: &Module, laid: Option<&Laid>, contextual: bool)
	-> std::result::Result<(Vec<J>, ProbeSource), String>
{
	let from_intro = match laid {
		Some(l)	=> match l.intro.query(&Selector::Label(Label::new(PROBE))) {
			Ok(elems)	=> {
				let mut v = Vec::new();
				for e in &elems {
					match e.field("value") {
						Some(x)	=> v.push(to_json(x)),
						None	=> return Err(fmt!("a <{}> element ({}) has no `value` field", PROBE,
							e.kind().map(|k| k.path()).unwrap_or("sequence"))),
					}
				}
				Some(Ok((v, ProbeSource::Introspector)))
			}
			Err(e)		=> Some(Err(fmt!("introspector query: {}", plain(&e)))),
		},
		None	=> None,
	};
	match from_intro {
		Some(Ok(v))	=> Ok(v),
		// Without layout, a fixture free of show rules and context has the same probes in its
		// evaluated content as Typst's query finds; one with either does not, so it waits for layout.
		other if !contextual => {
			let mut v = Vec::new();
			let mut fault = None;
			collect_probes(&module.content, &mut v, &mut fault, 0);
			match fault {
				Some(f)	=> Err(f),
				None	=> match other {
					Some(Err(e)) if v.is_empty()	=> Err(e),
					_								=> Ok((v, ProbeSource::Content)),
				},
			}
		}
		Some(Err(e))	=> Err(e),
		None			=> Err("no layout, and the fixture uses show rules or context, so its probes \
			exist only after realisation".to_string()),
	}
}

fn collect_probes(c: &Content, out: &mut Vec<J>, fault: &mut Option<String>, depth: usize) {
	if depth > 256 || fault.is_some() {
		return;
	}
	match c {
		Content::Sequence(_) | Content::Styled(_) => for k in c.children() {
			collect_probes(k, out, fault, depth + 1);
		},
		Content::Elem(e) => {
			if e.kind == ElemKind::Metadata && e.label.as_ref().map(|l| l.as_str() == PROBE).unwrap_or(false) {
				match c.field("value") {
					Some(v)	=> out.push(to_json(v)),
					None	=> *fault = Some(fmt!("a <{}> metadata element has no `value` field", PROBE)),
				}
				return;
			}
			for (_, v) in &e.fields {
				collect_value_content(v, out, fault, depth + 1);
			}
		}
	}
}

fn collect_value_content(v: &Value, out: &mut Vec<J>, fault: &mut Option<String>, depth: usize) {
	match v {
		Value::Content(c)	=> collect_probes(c, out, fault, depth),
		Value::Array(a)		=> for x in a.iter() {
			collect_value_content(x, out, fault, depth + 1);
		},
		_					=> (),
	}
}

/// A value as Typst's `typst eval` serialises it: JSON's own types as themselves, a symbol as its
/// text, content as `{"func": .., fields.., "label": ..}`, and everything else as its `repr`.
pub fn to_json(v: &Value) -> J {
	match v {
		Value::None			=> J::Null,
		Value::Bool(b)		=> J::Bool(*b),
		Value::Int(i)		=> J::Int(*i),
		Value::Float(f)		=> if f.is_finite() { J::Float(*f) } else { J::Null },
		Value::Str(s)		=> J::Str(s.to_string()),
		Value::Symbol(s)	=> J::Str(ops::symbol_text(s)),
		Value::Array(a)		=> J::Arr(a.iter().map(to_json).collect()),
		Value::Dict(d)		=> J::Obj(d.iter().map(|(k, x)| (k.to_string(), to_json(x))).collect()),
		Value::Content(c)	=> content_json(c),
		other				=> J::Str(foundations::repr(other)),
	}
}

fn content_json(c: &Content) -> J {
	let mut kv: Vec<(String, J)> = Vec::new();
	match c {
		Content::Sequence(s) => {
			kv.push(("func".to_string(), J::str("sequence")));
			kv.push(("children".to_string(), J::Arr(s.children.iter().map(content_json).collect())));
		}
		Content::Styled(s) => {
			kv.push(("func".to_string(), J::str("styled")));
			kv.push(("child".to_string(), content_json(&s.child)));
			kv.push(("styles".to_string(), J::str("styles(..)")));
		}
		Content::Elem(e) => {
			kv.push(("func".to_string(), J::str(e.kind.name())));
			let mut fields: Vec<&(FieldId, Value)> = e.fields.iter().collect();
			fields.sort_by_key(|(id, _)| id.0);
			for (id, v) in fields {
				let name = match e.kind.field_spec(*id) {
					Some(spec)	=> spec.name.to_string(),
					None		=> fmt!("<field {} of {} outside its schema>", id.0, e.kind.path()),
				};
				kv.push((name, to_json(v)));
			}
		}
	}
	if let Some(l) = c.label() {
		kv.push(("label".to_string(), J::Str(fmt!("<{}>", l.as_str()))));
	}
	J::Obj(kv)
}

// Level 2

pub fn positions(laid: &Laid) -> std::result::Result<Vec<PosRow>, String> {
	let mut rows = Vec::new();
	for e in &laid.intro.elems {
		let kind = match e.kind() {
			Some(k)	=> k,
			None	=> continue,
		};
		let probe = kind == ElemKind::Metadata && e.label().map(|l| l.as_str() == PROBE).unwrap_or(false);
		let tracked = matches!(kind, ElemKind::Heading | ElemKind::Figure | ElemKind::Equation | ElemKind::Footnote);
		if !(tracked || probe) {
			continue;
		}
		let loc = match e.location() {
			Some(l)	=> l,
			None	=> return Err(fmt!("a realised {} has no location", kind.path())),
		};
		match laid.intro.position(loc) {
			Some(p)	=> rows.push(PosRow { func: kind.name().to_string(), page: p.page, x: p.x.to_pt(), y: p.y.to_pt() }),
			None	=> return Err(fmt!("the introspector has no position for a {}", kind.path())),
		}
	}
	Ok(rows)
}

// Level 3

fn skeleton(engine: &mut Engine, module: &Module) -> std::result::Result<Vec<Sk>, String> {
	engine.locator.reset();
	let mut out = Vec::new();
	match walk(engine, &module.content, &StyleChain::root(), RealiseMode::Document, &mut out, 0) {
		Ok(())	=> Ok(structure::normalise(out)),
		Err(e)	=> Err(fmt!("realise: {}", plain(&e))),
	}
}

fn field_content(c: &Content, name: &str) -> Option<Content> {
	match c.field(name) {
		Some(Value::Content(x))	=> Some(x.clone()),
		Some(Value::Str(s))		=> Some(Content::text(s)),
		_						=> None,
	}
}

fn field_int(c: &Content, name: &str) -> Option<i64> {
	match c.field(name) {
		Some(Value::Int(i))	=> Some(*i),
		_					=> None,
	}
}

fn field_bool(c: &Content, name: &str) -> bool {
	matches!(c.field(name), Some(Value::Bool(true)))
}

fn walk(
	engine:	&mut Engine,
	content:	&Content,
	styles:		&StyleChain,
	mode:		RealiseMode,
	out:		&mut Vec<Sk>,
	depth:		usize,
)
	-> Outcome<()>
{
	if depth > 128 {
		return Err(err!("the realised tree is deeper than 128 levels"; Excessive));
	}
	let pairs = res!(realise(engine, content, styles, mode));
	for p in pairs {
		res!(element(engine, &p.content, &p.styles, mode, out, depth + 1));
	}
	Ok(())
}

// One realised element into skeleton nodes. The tag table mirrors what Typst's HTML export writes for
// each model element; layout wrappers are transparent, as `div` and `span` are on the oracle side.
fn element(
	engine:	&mut Engine,
	c:		&Content,
	styles:	&StyleChain,
	mode:	RealiseMode,
	out:	&mut Vec<Sk>,
	depth:	usize,
)
	-> Outcome<()>
{
	let kind = match c.kind() {
		Some(k)	=> k,
		None	=> return walk(engine, c, styles, mode, out, depth),
	};
	let inline = RealiseMode::Inline;
	let flow = RealiseMode::Flow;
	let tagged = |name: &str, body: Option<Content>, m: RealiseMode, engine: &mut Engine, out: &mut Vec<Sk>| -> Outcome<()> {
		let mut kids = Vec::new();
		if let Some(b) = body {
			res!(walk(engine, &b, styles, m, &mut kids, depth));
		}
		out.push(Sk::Tag(name.to_string(), kids));
		Ok(())
	};
	match kind {
		ElemKind::Text => if let Some(Value::Str(s)) = c.get(FieldId(0)) {
			out.push(Sk::Text(s.to_string()));
		},
		ElemKind::Space			=> out.push(Sk::Text(" ".to_string())),
		ElemKind::SmartQuote	=> out.push(Sk::Text(if field_bool(c, "double") { "\"" } else { "'" }.to_string())),
		ElemKind::Linebreak		=> out.push(Sk::Tag("br".to_string(), Vec::new())),
		ElemKind::Par			=> res!(tagged("p", field_content(c, "body"), inline, engine, out)),
		ElemKind::Heading		=> {
			let level = field_int(c, "level").or_else(|| field_int(c, "depth")).unwrap_or(1).clamp(1, 5);
			res!(tagged(&fmt!("h{}", level + 1), field_content(c, "body"), inline, engine, out));
		}
		ElemKind::Strong		=> res!(tagged("strong", field_content(c, "body"), inline, engine, out)),
		ElemKind::Emph			=> res!(tagged("em", field_content(c, "body"), inline, engine, out)),
		ElemKind::Super			=> res!(tagged("sup", field_content(c, "body"), inline, engine, out)),
		ElemKind::Sub			=> res!(tagged("sub", field_content(c, "body"), inline, engine, out)),
		ElemKind::Highlight		=> res!(tagged("mark", field_content(c, "body"), inline, engine, out)),
		ElemKind::Link			=> res!(tagged("a", field_content(c, "body"), inline, engine, out)),
		ElemKind::Quote			=> {
			if field_bool(c, "block") {
				res!(tagged("blockquote", field_content(c, "body"), flow, engine, out));
			} else {
				res!(walk(engine, &field_content(c, "body").unwrap_or_default(), styles, inline, out, depth));
			}
		}
		ElemKind::Raw			=> {
			let text = match c.field("text") {
				Some(Value::Str(s))	=> s.to_string(),
				_					=> String::new(),
			};
			let code = Sk::Tag("code".to_string(), vec![Sk::Text(text)]);
			if field_bool(c, "block") {
				out.push(Sk::Tag("pre".to_string(), vec![code]));
			} else {
				out.push(code);
			}
		}
		ElemKind::List | ElemKind::Enum | ElemKind::Terms => {
			let name = match kind {
				ElemKind::List	=> "ul",
				ElemKind::Enum	=> "ol",
				_				=> "dl",
			};
			let mut kids = Vec::new();
			if let Some(Value::Array(items)) = c.field("children") {
				for it in items.iter() {
					if let Value::Content(ic) = it {
						res!(element(engine, ic, styles, flow, &mut kids, depth + 1));
					}
				}
			}
			out.push(Sk::Tag(name.to_string(), kids));
		}
		ElemKind::ListItem | ElemKind::EnumItem => res!(tagged("li", field_content(c, "body"), flow, engine, out)),
		ElemKind::TermItem		=> {
			res!(tagged("dt", field_content(c, "term"), inline, engine, out));
			res!(tagged("dd", field_content(c, "description"), flow, engine, out));
		}
		ElemKind::Figure		=> {
			let mut kids = Vec::new();
			if let Some(b) = field_content(c, "body") {
				res!(walk(engine, &b, styles, flow, &mut kids, depth));
			}
			if let Some(cap) = field_content(c, "caption") {
				if cap.is(ElemKind::FigureCaption) {
					res!(element(engine, &cap, styles, flow, &mut kids, depth + 1));
				} else {
					let mut ck = Vec::new();
					res!(walk(engine, &cap, styles, inline, &mut ck, depth));
					kids.push(Sk::Tag("figcaption".to_string(), ck));
				}
			}
			out.push(Sk::Tag("figure".to_string(), kids));
		}
		ElemKind::FigureCaption	=> res!(tagged("figcaption", field_content(c, "body"), inline, engine, out)),
		ElemKind::Table			=> {
			let mut kids = Vec::new();
			if let Some(Value::Array(items)) = c.field("children") {
				for it in items.iter() {
					if let Value::Content(ic) = it {
						if ic.is(ElemKind::TableCell) {
							res!(element(engine, ic, styles, flow, &mut kids, depth + 1));
						} else {
							let mut ck = Vec::new();
							res!(walk(engine, ic, styles, flow, &mut ck, depth));
							kids.push(Sk::Tag("td".to_string(), ck));
						}
					}
				}
			}
			out.push(Sk::Tag("table".to_string(), kids));
		}
		ElemKind::TableCell		=> res!(tagged("td", field_content(c, "body"), flow, engine, out)),
		ElemKind::Equation		=> out.push(Sk::Tag("math".to_string(), Vec::new())),
		ElemKind::Image			=> out.push(Sk::Tag("img".to_string(), Vec::new())),
		// Left to levels 2 and 4 (see `structure.rs`), and invisible.
		ElemKind::Footnote | ElemKind::FootnoteEntry | ElemKind::Metadata | ElemKind::CounterUpdate
			| ElemKind::StateUpdate | ElemKind::Parbreak => (),
		// Transparent: whatever body the element carries, realised in the mode it sits in.
		_ => {
			let m = match kind {
				ElemKind::Block | ElemKind::Align | ElemKind::Pad | ElemKind::Columns => flow,
				_ => mode,
			};
			if let Some(b) = field_content(c, "body") {
				res!(walk(engine, &b, styles, m, out, depth));
			}
		}
	}
	Ok(())
}

// Level 4

pub fn pages(laid: &Laid) -> Vec<APage> {
	laid.pages.iter().map(|p| APage {
		width:	p.geom.width.to_pt(),
		height:	p.geom.height.to_pt(),
		runs:	p.frame.placed.iter().filter_map(|pl| match &pl.kind {
			PlacedKind::Text(st) => Some(ARun {
				text:	st.source().to_string(),
				x0:		pl.x.to_pt(),
				x1:		(pl.x + pl.dims.width).to_pt(),
				base:	(pl.y + pl.dims.height).to_pt(),
			}),
			_ => None,
		}).collect(),
	}).collect()
}

/// A module's content, evaluated only: for the package corpus, which has no oracle.
pub fn eval_only(path: &std::path::Path, root: &std::path::Path, timeout: Duration) -> RunEnd {
	let (tx, rx) = mpsc::channel();
	let path = path.to_path_buf();
	let root = root.to_path_buf();
	let spawned = std::thread::Builder::new()
		.stack_size(256 << 20)
		.spawn(move || {
			let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
				let mut out = AusOut::default();
				let mut world = World::new(root);
				match world.load(&path) {
					Ok(id)	=> {
						let mut engine = Engine::new(world);
						if let Err(e) = eval_source(&mut engine, id) {
							out.eval = Some(plain(&e));
						}
					}
					Err(e)	=> out.eval = Some(fmt!("load: {}", plain(&e))),
				}
				out
			}));
			let _ = tx.send(match r {
				Ok(out)	=> Ok(out),
				Err(p)	=> Err(panic_text(p)),
			});
		});
	if let Err(e) = spawned {
		return RunEnd::Panic(fmt!("could not start the eval thread: {}", e));
	}
	match rx.recv_timeout(timeout) {
		Ok(Ok(out))	=> RunEnd::Done(out),
		Ok(Err(p))	=> RunEnd::Panic(p),
		Err(mpsc::RecvTimeoutError::Timeout)		=> RunEnd::Timeout(timeout.as_secs()),
		Err(mpsc::RecvTimeoutError::Disconnected)	=> RunEnd::Panic("the eval thread ended without a result".to_string()),
	}
}

/// An error's text without the terminal colour codes `Error`'s display adds.
pub fn plain<E: std::fmt::Display>(e: &E) -> String {
	let s = fmt!("{}", e);
	let mut out = String::with_capacity(s.len());
	let mut chars = s.chars();
	while let Some(c) = chars.next() {
		if c == '\u{1b}' {
			for d in chars.by_ref() {
				if d.is_ascii_alphabetic() {
					break;
				}
			}
			continue;
		}
		out.push(c);
	}
	out
}
