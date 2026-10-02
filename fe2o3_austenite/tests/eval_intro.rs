//! Introspection and the layout fixpoint (U8) against the host's `typst` 0.15.x, on the synthetic
//! fixtures in `tests/fixtures/eval/intro/`.
//!
//! Until the document layouter can place pages, fixtures run through [`Flat`], a layouter that realises
//! the document level by level and places every element on the page its page breaks give it, streaming
//! each page to the sink as it ends, which is all a query, a counter or a state can see. A line
//! `// intro: needs <parts>` names the units a fixture waits on (`layout`, `boxheight`,
//! `numbering`, `heading`, `figure`, `array`, `par`, `selector`), each probed at run time, so a fixture is checked the moment its
//! parts exist and is reported, not silently passed, until then. A fixture marked `oracle: rejects` must
//! fail with Typst's first error, message and line:column.
//!
//!     EVAL_ORACLE_TYPST=~/bin/typst cargo test -p oxedyne_fe2o3_austenite --test eval_intro -- --nocapture

#[allow(dead_code)]
#[path = "eval_oracle/mod.rs"]
mod harness;

use harness::austenite::to_json;
use harness::corpus::{
	self,
	Expect,
	Filter,
	Fixture,
};
use harness::json::{
	self,
	J,
};
use harness::oracle::Oracle;

use oxedyne_fe2o3_austenite::eval::content::{
	Content,
	Elem,
	ElemKind,
	FieldId,
};
use oxedyne_fe2o3_austenite::eval::fixpoint::{
	self,
	Laid,
	Layouter,
	PageSink,
	PageSource,
};
use oxedyne_fe2o3_austenite::eval::intro::{
	self,
	Builder,
	Introspector,
};
use oxedyne_fe2o3_austenite::eval::lib;
use oxedyne_fe2o3_austenite::eval::locate::Location;
use oxedyne_fe2o3_austenite::eval::realise::{
	realise,
	RealiseMode,
	Tag,
};
use oxedyne_fe2o3_austenite::eval::select::Selector;
use oxedyne_fe2o3_austenite::eval::styles::StyleChain;
use oxedyne_fe2o3_austenite::eval::value::{
	Label,
	Value,
};
use oxedyne_fe2o3_austenite::eval::eval::{
	eval_string,
	EvalMode,
};
use oxedyne_fe2o3_austenite::eval::{
	eval_source,
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::ir::Sp;
use oxedyne_fe2o3_austenite::ledger::Position;
use oxedyne_fe2o3_austenite::page::{
	Frame,
	Page,
	PageGeometry,
};
use oxedyne_fe2o3_austenite::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::path::PathBuf;
use std::sync::Arc;

/// Realises level by level and records each element's start tag on the page the page breaks before it
/// give it, handing each page to the sink when the next begins. It lays nothing out, so it answers every
/// question but where on the page.
struct Flat;

/// What [`Flat`] carries down the walk: the page it is on, whether anything but tags has landed there,
/// the page's numbering, read from the first styles the page meets, and the records made so far.
struct Walk {
	page:		u32,
	filled:		bool,
	numbering:	Option<Value>,
	records:	i32,
}

impl Layouter for Flat {
	fn lay<S: PageSink>(
		&mut self,
		engine:		&mut Engine,
		content:	&Content,
		styles:		&StyleChain,
		builder:	&mut Builder,
		sink:		&mut S,
	)
		-> Outcome<u32>
	{
		let mut w = Walk { page: 1, filled: false, numbering: None, records: 0 };
		res!(walk(engine, content, styles, RealiseMode::Document, builder, sink, &mut w, 0));
		res!(end_page(engine, builder, sink, &mut w));
		Ok(w.page)
	}
}

fn end_page<S: PageSink>(engine: &mut Engine, builder: &mut Builder, sink: &mut S, w: &mut Walk) -> Outcome<()> {
	let geom = PageGeometry::new(Sp::from_pt(595.0), Sp::from_pt(842.0), Sp::from_pt(72.0));
	builder.page(w.page, &w.numbering.clone().unwrap_or(Value::None));
	sink.page(engine, Page::new(w.page, geom, Frame::new()))
}

/// The page numbering a chain sets, `none` where the page element has no such field yet.
fn numbering(styles: &StyleChain) -> Outcome<Value> {
	match ElemKind::Page.field_id("numbering") {
		Some(id)	=> Ok(res!(styles.get(ElemKind::Page, id)).unwrap_or(Value::None)),
		None		=> Ok(Value::None),
	}
}

#[allow(clippy::too_many_arguments)]
fn walk<S: PageSink>(
	engine:		&mut Engine,
	content:	&Content,
	styles:		&StyleChain,
	mode:		RealiseMode,
	builder:	&mut Builder,
	sink:		&mut S,
	w:			&mut Walk,
	depth:		usize,
)
	-> Outcome<()>
{
	if depth > 64 {
		return Err(err!("Flat layout nested past 64 levels."; Excessive));
	}
	let pairs = res!(realise(engine, content, styles, mode));
	for p in pairs {
		if w.numbering.is_none() {
			w.numbering = Some(res!(numbering(&p.styles)));
		}
		match &p.tag {
			Some(Tag::Start(c)) => {
				builder.record(c, Position::new(w.page, Sp::ZERO, Sp(w.records)), None);
				w.records += 1;
			}
			Some(Tag::End(_)) => (),
			None => {
				let e = match p.content.elem() {
					Some(e)	=> e,
					None	=> continue,
				};
				match e.kind {
					ElemKind::Pagebreak => {
						// A weak break on a page that holds nothing yet collapses, as Typst's does.
						let weak = matches!(p.content.field("weak"), Some(Value::Bool(true)));
						if weak && !w.filled {
							continue;
						}
						res!(end_page(engine, builder, sink, w));
						w.page += 1;
						w.filled = false;
						w.numbering = None;
						continue;
					}
					// Values that are not shown, and maths, which only an equation's layout realises.
					ElemKind::Metadata | ElemKind::CounterUpdate | ElemKind::StateUpdate
						| ElemKind::Equation => continue,
					_ => w.filled = true,
				}
				for (_, v) in &e.fields {
					res!(walk_value(engine, v, &p.styles, builder, sink, w, depth + 1));
				}
			}
		}
	}
	Ok(())
}

#[allow(clippy::too_many_arguments)]
fn walk_value<S: PageSink>(
	engine:		&mut Engine,
	v:			&Value,
	styles:		&StyleChain,
	builder:	&mut Builder,
	sink:		&mut S,
	w:			&mut Walk,
	depth:		usize,
)
	-> Outcome<()>
{
	match v {
		Value::Content(c)	=> walk(engine, c, styles, RealiseMode::Flow, builder, sink, w, depth),
		Value::Array(a)		=> {
			for x in a.iter() {
				res!(walk_value(engine, x, styles, builder, sink, w, depth));
			}
			Ok(())
		}
		_					=> Ok(()),
	}
}

/// Counts what the fixpoint hands it and keeps no page: the pages of the pass in progress, the
/// discarded passes, the finishes, and what the finishing pass held.
#[derive(Debug, Default)]
struct Counting {
	pages:			u32,
	discards:		u32,
	finishes:		u32,
	final_pages:	u32,
	final_records:	usize,
}

impl PageSink for Counting {
	fn page(&mut self, _engine: &mut Engine, _page: Page) -> Outcome<()> {
		self.pages += 1;
		Ok(())
	}

	fn discard_pass(&mut self) -> Outcome<()> {
		self.discards += 1;
		self.pages = 0;
		Ok(())
	}

	fn finish(&mut self, _engine: &mut Engine, intro: &Introspector) -> Outcome<()> {
		self.finishes += 1;
		self.final_pages = self.pages;
		self.final_records = intro.len();
		Ok(())
	}
}

// Capabilities other units supply, probed so a waiting fixture turns on by itself.

fn has_numbering() -> bool {
	let mut engine = Engine::new(World::new(PathBuf::from("/")));
	lib::numbering::apply(&mut engine, &Value::str("1.a"), &[1, 2]).is_ok()
}

fn has_layout(root: &std::path::Path) -> bool {
	let mut world = World::new(root.to_path_buf());
	let id = match world.add_source(root.join("__probe.typ"), "x".to_string()) {
		Ok(id)	=> id,
		Err(_)	=> return false,
	};
	let mut engine = Engine::new(world);
	match eval_source(&mut engine, id) {
		Ok(m)	=> fixpoint::run(&mut engine, &m, &mut Counting::default()).is_ok(),
		Err(_)	=> false,
	}
}

fn has_array() -> bool {
	let mut engine = Engine::new(World::new(PathBuf::from("/")));
	let v = eval_string(&mut engine, "(1, 2).map(x => x).len() + if type(1) == int { 3 } else { 0 }", EvalMode::Code,
		lib::library(), oxedyne_fe2o3_austenite::syntax::Span::detached());
	matches!(v, Ok(Value::Int(5)))
}

fn has_selector() -> bool {
	let mut engine = Engine::new(World::new(PathBuf::from("/")));
	let v = eval_string(&mut engine, "selector(<a>)", EvalMode::Code, lib::library(),
		oxedyne_fe2o3_austenite::syntax::Span::detached());
	matches!(v, Ok(Value::Selector(_)))
}

/// Does a document of `src` realise, with the schemas its elements' default shows need? A heading's show
/// reads `text.size` and a figure's `block.breakable`, so the element existing is not enough.
fn realises(src: &str) -> bool {
	let root = PathBuf::from("/");
	let mut world = World::new(root.clone());
	let id = match world.add_source(root.join("__probe_realise.typ"), src.to_string()) {
		Ok(id)	=> id,
		Err(_)	=> return false,
	};
	let mut engine = Engine::new(world);
	match eval_source(&mut engine, id) {
		Ok(m)	=> realise(&mut engine, &m.content, &StyleChain::root(), RealiseMode::Document).is_ok(),
		Err(_)	=> false,
	}
}

/// Runs a document of `src` through the fixpoint and returns what its `<probe>`s hold, or `None` when it
/// does not lay out.
fn laid_probes(src: &str) -> Option<Vec<J>> {
	let root = PathBuf::from("/");
	let mut world = World::new(root.clone());
	let id = world.add_source(root.join("__probe_laid.typ"), src.to_string()).ok()?;
	let mut engine = Engine::new(world);
	let m = eval_source(&mut engine, id).ok()?;
	let laid = fixpoint::run(&mut engine, &m, &mut Counting::default()).ok()?;
	probes(&laid.intro).ok()
}

/// Does an inline `box(height:)` take its height, as `measure` reports it?
fn has_box_height() -> bool {
	match laid_probes("#context [#metadata(measure(box(width: 3pt, height: 4pt))) <probe>]") {
		Some(v)	=> v.iter().any(|j| j.render().contains("4pt")),
		None	=> false,
	}
}

fn has_elem(kind: ElemKind) -> bool {
	if kind.fields().is_empty() || kind.field_id("numbering").is_none() {
		return false;
	}
	match kind {
		ElemKind::Heading	=> realises("= A"),
		ElemKind::Figure	=> realises("#figure(rect(), caption: [c])"),
		_					=> true,
	}
}

/// The parts a fixture needs that are not yet here.
fn missing(fx: &Fixture, layout: bool, numbering: bool) -> Vec<&'static str> {
	let line = fx.text.lines().find(|l| l.trim_start().starts_with("// intro:")).unwrap_or("");
	let mut out = Vec::new();
	for (word, have, name) in [
		("layout", layout, "layout (U6a/U6b)"),
		("boxheight", has_box_height(), "inline `box(height:)` (U6b-S2)"),
		("numbering", numbering, "numbering (U3)"),
		("heading", has_elem(ElemKind::Heading), "heading (U5)"),
		("figure", has_elem(ElemKind::Figure), "figure (U5)"),
		("array", has_array(), "array methods (U3)"),
		("par", ElemKind::Par.field_id("body").is_some(), "par (U5)"),
		("selector", has_selector(), "the `selector` global (U0 `library()` calling `select::define`)"),
	] {
		if line.contains("needs") && line.contains(word) && !have {
			out.push(name);
		}
	}
	out
}

struct Run {
	laid:		std::result::Result<Laid, String>,
	first:		Option<(String, Option<(usize, usize)>)>,
	warnings:	Vec<String>,
	sink:		Counting,
}

fn run(fx: &Fixture, flat: bool) -> Run {
	let mut world = World::new(fx.root.clone());
	let mut out = Run {
		laid:		Err("not run".to_string()),
		first:		None,
		warnings:	Vec::new(),
		sink:		Counting::default(),
	};
	let id = match world.load(&fx.path) {
		Ok(id)	=> id,
		Err(e)	=> {
			out.laid = Err(fmt!("load: {}", e.plain()));
			return out;
		}
	};
	let mut engine = Engine::new(world);
	out.laid = match eval_source(&mut engine, id) {
		Ok(m) => {
			let r = if flat {
				fixpoint::run_with(&mut engine, &m, &mut Flat, &mut out.sink)
			} else {
				fixpoint::run(&mut engine, &m, &mut out.sink)
			};
			r.map_err(|e| e.plain())
		}
		Err(e) => Err(fmt!("eval: {}", e.plain())),
	};
	out.first = engine.diags.iter().find(|d| d.is_error()).map(|d| {
		let pos = engine.world.sources.iter()
			.find(|s| s.id == d.span.file && !d.span.is_detached())
			.map(|s| s.line_col(d.span.start));
		(d.message.clone(), pos)
	});
	out.warnings = engine.diags.iter().filter(|d| !d.is_error()).map(|d| d.message.clone()).collect();
	out
}

fn probes(intro: &Introspector) -> std::result::Result<Vec<J>, String> {
	let found = match intro.query(&Selector::Label(Label::new("probe"))) {
		Ok(f)	=> f,
		Err(e)	=> return Err(e.plain()),
	};
	found.iter().map(|e| match e.field("value") {
		Some(v)	=> Ok(to_json(v)),
		None	=> Err("a <probe> has no `value` field".to_string()),
	}).collect()
}

#[test]
fn intro_fixtures_match_the_typst_oracle() {
	let oracle = match Oracle::find() {
		Ok(Some(o))	=> o,
		Ok(None)	=> {
			println!("EVAL_ORACLE_SKIP=1: the introspection oracle was skipped explicitly");
			return;
		}
		Err(e)		=> panic!("{}", e.plain()),
	};
	let fixtures = match corpus::discover(&Filter::only("intro")) {
		Ok(f)	=> f,
		Err(e)	=> panic!("{}", e.plain()),
	};
	assert!(fixtures.len() >= 15, "the intro corpus shrank to {} fixtures", fixtures.len());
	let root = corpus::fixtures_dir().join("intro");
	let layout = has_layout(&root);
	let numbering = has_numbering();
	let (mut passed, mut failed, mut waiting) = (Vec::new(), Vec::new(), Vec::new());
	for fx in &fixtures {
		let lacks = missing(fx, layout, numbering);
		if !lacks.is_empty() {
			waiting.push(fmt!("{}: waits on {}", fx.id(), lacks.join(", ")));
			continue;
		}
		let flat = !layout;
		let got = run(fx, flat);
		let verdict: std::result::Result<(), Vec<String>> = match fx.expect {
			Expect::Rejects => {
				let want = match oracle.compiles(fx) {
					Ok(Err(msg))	=> msg,
					Ok(Ok(()))		=> panic!("{}: typst accepts a fixture marked `oracle: rejects`", fx.id()),
					Err(e)			=> panic!("{}: {}", fx.id(), e.plain()),
				};
				let (msg, pos) = harness::split_first_error(&want);
				match harness::compare_first_error(&msg, pos, got.first.as_ref()) {
					harness::Verdict::Pass	=> Ok(()),
					harness::Verdict::Fail(d)	=> Err(d),
					other					=> Err(vec![fmt!("{:?}", other)]),
				}
			}
			Expect::Accepts => {
				let want = match oracle.probes(fx) {
					Ok(Ok(v))	=> v,
					Ok(Err(e))	=> panic!("{}: typst: {}", fx.id(), e),
					Err(e)		=> panic!("{}: {}", fx.id(), e.plain()),
				};
				match &got.laid {
					Err(e)		=> Err(vec![fmt!("austenite: {} (first error: {:?})", e, got.first)]),
					Ok(laid)	=> match probes(&laid.intro) {
						Err(e)	=> Err(vec![e]),
						Ok(v)	=> {
							let mut d = Vec::new();
							json::diff("probes", &J::Arr(want), &J::Arr(v), &mut d);
							// Typst gives up after five passes; so must Austenite, and say so.
							let warned = got.warnings.iter().any(|w| w.contains("did not converge"));
							if fx.name.starts_with("nonconverge") && (laid.converged || laid.passes != 5 || !warned) {
								d.push(fmt!("expected five passes and a warning, got {} pass(es), converged {}",
									laid.passes, laid.converged));
							}
							if !fx.name.starts_with("nonconverge") && !laid.converged {
								d.push(fmt!("did not converge in {} passes", laid.passes));
							}
							// Every pass but the last is discarded, and the last is finished once.
							let s = &got.sink;
							if s.discards + 1 != laid.passes || s.finishes != 1 || s.final_pages != laid.pages
								|| s.final_records != laid.intro.len()
							{
								d.push(fmt!("the sink saw {} discard(s), {} finish(es) and {} final page(s) \
									for {} pass(es) of {} page(s)", s.discards, s.finishes, s.final_pages,
									laid.passes, laid.pages));
							}
							if d.is_empty() { Ok(()) } else { Err(d) }
						}
					},
				}
			}
		};
		match verdict {
			Ok(())	=> passed.push(fx.id()),
			Err(d)	=> failed.push(fmt!("{}:\n    {}", fx.id(), d.join("\n    "))),
		}
	}
	println!("intro: {} passed, {} failed, {} waiting (layout {}, numbering {})",
		passed.len(), failed.len(), waiting.len(), layout, numbering);
	for w in &waiting {
		println!("  waiting  {}", w);
	}
	for f in &failed {
		println!("  FAILED   {}", f);
	}
	assert!(failed.is_empty(), "{} introspection fixture(s) disagree with typst", failed.len());
	assert!(passed.len() >= 12, "only {} fixtures were checked; the rest are waiting", passed.len());
}

/// A metadata element with a value, a label and a location, as realisation would prepare it.
fn located(v: i64, label: &str, loc: u64) -> Content {
	Content::Elem(Arc::new(Elem {
		kind:		ElemKind::Metadata,
		fields:		vec![(FieldId(0), Value::Int(v))],
		label:		Some(Label::new(label)),
		location:	Some(Location(loc)),
		span:		Span::detached(),
		guards:		Vec::new(),
		prepared:	true,
	}))
}

fn at(page: u32, y: i32) -> Position { Position::new(page, Sp::ZERO, Sp(y)) }

fn values<'a, I: Iterator<Item = &'a Content>>(elems: I) -> Vec<i64> {
	elems.filter_map(|e| match e.field("value") { Some(Value::Int(i)) => Some(*i), _ => None }).collect()
}

fn queried(intro: &Introspector, sel: &Selector) -> Vec<i64> { values(intro.query(sel).unwrap_or_default().iter()) }

/// Before and after bound by the first match of their bound, inclusively unless told otherwise, and a
/// location the pages never placed counts everything as before it.
#[test]
fn introspector_bounds_and_counts_by_document_order() {
	let mut b = Builder::new();
	for (v, l, n) in [(1, "a", 10), (2, "b", 11), (3, "a", 12), (4, "b", 13)] {
		b.record(&located(v, l, n), at(1, n as i32), None);
	}
	let intro = b.finish();
	let a = Selector::Label(Label::new("a"));
	let bl = Arc::new(Selector::Label(Label::new("b")));
	let before = Selector::Before { selector: Arc::new(a.clone()), end: bl.clone(), inclusive: true };
	let after = Selector::After { selector: Arc::new(a.clone()), start: bl, inclusive: false };
	assert_eq!(queried(&intro, &before), vec![1]);
	assert_eq!(queried(&intro, &after), vec![3]);
	assert_eq!(intro.count_before(&a, Location(11)).ok(), Some(1));
	assert_eq!(intro.count_before(&a, Location(12)).ok(), Some(2));
	assert_eq!(intro.count_before(&a, Location(99)).ok(), Some(2));
	assert_eq!(queried(&intro, &Selector::Elem(ElemKind::Metadata, None)), vec![1, 2, 3, 4]);
	// A second placement of the same location keeps the first.
	let mut twice = Builder::new();
	twice.record(&located(1, "a", 10), at(1, 0), None);
	twice.record(&located(9, "a", 10), at(2, 0), None);
	let twice = twice.finish();
	assert_eq!(twice.len(), 1);
	assert_eq!(twice.page(Location(10)), Some(1));
}

/// Out-of-flow material is read where its parent stands: a footnote's entry just after the footnote, a
/// float's body at its mark, and material under a parent nothing marks where it was placed. U6b's
/// `Order` read it this way; the builder now does.
#[test]
fn builder_reads_out_of_flow_material_where_its_parent_stands() {
	let mut b = Builder::new();
	b.record(&located(1, "a", 1), at(1, 0), None);			// a footnote
	b.mark(100);											// where a float was written
	b.record(&located(2, "b", 2), at(1, 10), None);
	b.record(&located(3, "c", 3), at(2, 0), Some(100));		// the float's body, placed a page later
	b.record(&located(4, "d", 4), at(1, 90), Some(1));		// the footnote's entry, at the page foot
	b.record(&located(5, "e", 5), at(2, 5), Some(999));		// under a parent nothing marks
	b.record(&located(6, "f", 6), at(2, 9), Some(4));		// nested: under the entry
	let intro = b.finish();
	assert_eq!(values(intro.elems()), vec![1, 4, 6, 3, 2, 5]);
	assert_eq!(intro.order(Location(3)), Some(3));
	assert_eq!(intro.position(Location(3)), Some(at(2, 0)), "a record keeps where it was placed");
}

/// Page numbering is held once per run of equal numberings; a page given twice counts once.
#[test]
fn builder_holds_page_numbering_as_runs() {
	let mut b = Builder::new();
	b.record(&located(1, "a", 1), at(3, 0), None);
	b.page(1, &Value::str("1"));
	b.page(2, &Value::str("1"));
	b.page(3, &Value::str("i"));
	b.page(3, &Value::str("x"));
	b.page(4, &Value::str("i"));
	let intro = b.finish();
	assert_eq!(intro.pages(), 4);
	let shown = |p: u32| match intro.numbering_of_page(p) {
		Value::Str(s)	=> s.to_string(),
		Value::None		=> "none".to_string(),
		other			=> fmt!("{:?}", other),
	};
	assert_eq!([shown(1), shown(2), shown(3), shown(4), shown(5)], ["1", "1", "i", "i", "none"]);
	assert!(matches!(intro.page_numbering(Location(1)), Value::Str(s) if s.as_str() == "i"));
	assert_eq!(intro.numbering_runs(), 2, "two runs, not four pages' worth");
}

/// The `.prl` ledger comes from the introspector: one `Location` anchor per located element.
#[test]
fn ledger_is_derived_from_the_introspector() {
	let mut b = Builder::new();
	b.record(&located(1, "a", 0xbeef), Position::new(2, Sp::from_pt(10.0), Sp::from_pt(20.0)), None);
	b.page(1, &Value::None);
	b.page(2, &Value::None);
	let ledger = b.finish().ledger();
	assert_eq!(ledger.total_pages, 2);
	assert_eq!(ledger.get(&Location(0xbeef).anchor()).map(|a| a.pos),
		Some(Position::new(2, Sp::from_pt(10.0), Sp::from_pt(20.0))));
}

/// A page source that makes pages up to `last`, recording one element on each, as `driver::place_page`
/// records a page's elements when it places the page.
struct Numbered {
	next:	u32,
	last:	u32,
}

impl PageSource for Numbered {
	fn next_page(&mut self, _engine: &mut Engine, builder: &mut Builder) -> Outcome<Option<(Page, Value)>> {
		if self.next > self.last {
			return Ok(None);
		}
		let n = self.next;
		self.next += 1;
		builder.record(&located(n as i64, "p", 0x100 + n as u64), at(n, 0), None);
		let geom = PageGeometry::new(Sp::from_pt(100.0), Sp::from_pt(100.0), Sp::ZERO);
		let numbering = if n < 3 { Value::str("1") } else { Value::str("i") };
		Ok(Some((Page::new(n, geom, Frame::new()), numbering)))
	}
}

/// `place` streams: every page reaches the sink as it is made, its numbering reaches the builder, and
/// the count comes back.
#[test]
fn place_streams_each_page_to_the_sink() -> Outcome<()> {
	let mut engine = Engine::new(World::new(PathBuf::from("/")));
	let mut builder = Builder::new();
	let mut sink = Counting::default();
	let n = res!(fixpoint::place(&mut engine, &mut Numbered { next: 1, last: 4 }, &mut builder, &mut sink));
	assert_eq!(n, 4);
	assert_eq!(sink.pages, 4);
	let intro = builder.finish();
	assert_eq!(intro.pages(), 4);
	assert_eq!(intro.len(), 4);
	assert_eq!(intro.page(Location(0x103)), Some(3));
	assert!(matches!(intro.numbering_of_page(2), Value::Str(s) if s.as_str() == "1"));
	assert!(matches!(intro.numbering_of_page(4), Value::Str(s) if s.as_str() == "i"));
	Ok(())
}

/// `measure` lays out under a fresh root locator and puts the document's back: measured content takes no
/// ordinal, so the next element of the document is located as if nothing had been measured, and the
/// fresh root hands a measured element the location the document's first such element has, as Typst's
/// `Locator::root()` does.
#[test]
fn measure_takes_none_of_the_documents_ordinals() -> Outcome<()> {
	let span = Span::detached();
	let mut e = Engine::new(World::new(PathBuf::from("/")));
	let first = e.locator.locate(ElemKind::Heading, span);
	let inside = res!(intro::detached(&mut e, |e| {
		let a = e.locator.locate(ElemKind::Heading, span);
		let b = e.locator.locate(ElemKind::Heading, span);
		Ok((a, b))
	}));
	let second = e.locator.locate(ElemKind::Heading, span);
	// What a document that measured nothing locates its second heading at.
	let mut quiet = Engine::new(World::new(PathBuf::from("/")));
	assert_eq!(quiet.locator.locate(ElemKind::Heading, span), first);
	let want = quiet.locator.locate(ElemKind::Heading, span);
	assert_eq!(second, want, "the second heading of the document took an ordinal the measured layout used");
	assert_eq!(inside.0, first, "a measured element is located under a fresh root");
	assert_ne!(inside.0, inside.1, "ordinals still count inside the measured layout");
	assert_ne!(second, first);
	// The locator comes back when the layout fails.
	let failed: Outcome<()> = intro::detached(&mut e, |e| {
		e.locator.locate(ElemKind::Heading, span);
		Err(err!("a measured layout failed"; Test))
	});
	assert!(failed.is_err());
	let third = e.locator.locate(ElemKind::Heading, span);
	assert_eq!(third, quiet.locator.locate(ElemKind::Heading, span), "an error left the measured ordinals behind");
	Ok(())
}

/// Typst's `CounterState::step`, on the cases `counter_str.typ` checks against the oracle end to end,
/// spelt out so a regression names the rule rather than a fixture.
#[test]
fn counter_step_pads_with_zero_and_truncates() {
	let mut s = vec![2];
	intro::step(&mut s, 3, 1);
	assert_eq!(s, vec![2, 0, 1]);
	let mut s = vec![2, 5, 9];
	intro::step(&mut s, 2, 1);
	assert_eq!(s, vec![2, 6]);
	let mut s = vec![1];
	intro::step(&mut s, 1, 3);
	assert_eq!(s, vec![4]);
}

/// The introspector's own cost per located element, on a document of 4,000 labelled metadata elements
/// laid out by [`Flat`]: the record and the location, kind and label indices, by capacity. The elements'
/// own fields are counted apart, since realisation made them and the module shares their bodies.
#[test]
fn introspector_cost_per_element() -> Outcome<()> {
	let mut src = String::new();
	for i in 0..4000 {
		src.push_str(&fmt!("#metadata({}) <m>\n", i));
	}
	let root = PathBuf::from("/");
	let mut world = World::new(root.clone());
	let id = res!(world.add_source(root.join("__cost.typ"), src));
	let mut engine = Engine::new(world);
	let module = res!(eval_source(&mut engine, id));
	let laid = res!(fixpoint::run_with(&mut engine, &module, &mut Flat, &mut Counting::default()));
	let n = laid.intro.len();
	assert_eq!(n, 4000);
	let own = laid.intro.footprint() as f64 / n as f64;
	let elem = laid.intro.elems().map(|e| match e.elem() {
		Some(x)	=> std::mem::size_of::<Elem>() + 2 * std::mem::size_of::<usize>()
			+ x.fields.capacity() * std::mem::size_of::<(FieldId, Value)>(),
		None	=> 0,
	}).sum::<usize>() as f64 / n as f64;
	println!("introspector: {:.1} bytes per element of its own (a record is {} bytes); each prepared \
		metadata element holds {:.1} bytes more", own, std::mem::size_of::<intro::Record>(), elem);
	assert!(own <= 96.0, "the introspector costs {:.1} bytes per element", own);
	Ok(())
}
