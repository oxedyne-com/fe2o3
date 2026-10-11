//! U9: the paragraph memo. A paragraph whose children are text-like keeps the break that ends each of its lines
//! under a key of the caches' configuration, its `Config`, the region's width and each child's fingerprint; a
//! hit prepares the paragraph again, builds its lines from the kept breaks and commits the same boxes a miss
//! would.
//!
//! - A paragraph set again is a hit and comes out the same; a taller region, which no kept paragraph reads, is
//!   a hit too.
//! - Each field of the key, changed alone, is a miss that comes out as a cold layout does: the text, the
//!   style chain, the `Config` (through a `par` that no child's chain names), the width and the situation.
//! - A paragraph with an inline box or an equation is never kept; one that warns is, and warns on a hit too.
//! - A hit puts the current source's tag spans in the lines.
//! - A hit is a miss to the bit across the kinds of break: hyphenated, mandatory, right to left, first fit.
//! - The budget: nothing is kept at none, and a small one refuses and changes nothing the compile makes.
//! - In a session: an unchanged recompile hits every paragraph, an edit misses the edited one alone, and warm
//!   bytes equal cold bytes with the memo on and off.

use oxedyne_fe2o3_austenite::caches::{
	Budgets,
	Caches,
	Counters,
};
use oxedyne_fe2o3_austenite::compile::Session;
use oxedyne_fe2o3_austenite::emit::sinks::PdfSink;
use oxedyne_fe2o3_austenite::eval::content::ElemKind;
use oxedyne_fe2o3_austenite::eval::realise::{
	realise,
	Pair,
	RealiseMode,
	Tag,
};
use oxedyne_fe2o3_austenite::eval::styles::StyleChain;
use oxedyne_fe2o3_austenite::eval::{
	eval_source,
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::flow::inline::ParSituation;
use oxedyne_fe2o3_austenite::flow::par::layout_par;
use oxedyne_fe2o3_austenite::flow::text::FontStore;
use oxedyne_fe2o3_austenite::flow::Region;
use oxedyne_fe2o3_austenite::ir::{
	LeafKind,
	Node,
	Sp,
};
use oxedyne_fe2o3_hash::fingerprint::Fingerprint;

use oxedyne_fe2o3_core::prelude::*;

use std::path::{
	Path,
	PathBuf,
};

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ENGINE LEVEL                                                               │
// └───────────────────────────────────────────────────────────────────────────┘

fn work(name: &str) -> Outcome<PathBuf> {
	let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("parmemo").join(name);
	res!(std::fs::create_dir_all(&dir).map_err(|e| err!(e, "Could not make {:?}.", dir; IO)));
	Ok(dir)
}

// The paragraphs a source realises into, with the engine.
fn paragraphs(src: &str) -> Outcome<(Engine, Vec<Pair>)> {
	let root = res!(work("pars"));
	let mut world = World::new(root.clone());
	let id = res!(world.add_source(root.join("p.typ"), src.to_string()));
	let mut engine = Engine::new(world);
	let module = res!(eval_source(&mut engine, id));
	let pairs = res!(realise(&mut engine, &module.content, &StyleChain::root(), RealiseMode::Document));
	Ok((engine, pairs.into_iter().filter(|p| p.content.is(ElemKind::Par)).collect()))
}

fn region(width: f64, height: f64) -> Region {
	Region::new(Sp::from_pt(width), Sp::from_pt(height))
}

fn set(engine: &mut Engine, p: &Pair, region: Region, situation: ParSituation) -> Outcome<Vec<Node>> {
	layout_par(engine, &p.content, &p.styles, region, situation)
}

fn counts(engine: &Engine) -> (u64, u64) {
	(engine.caches.counters.par_hits, engine.caches.counters.par_misses)
}

// Everything layout decides about nodes, to the bit, with each tag's span: two equal signatures are two
// identical layouts.
fn signature(nodes: &[Node], out: &mut String) {
	for n in nodes {
		match n {
			Node::HBox(b) | Node::VBox(b) => {
				out.push_str(&fmt!("[{} {} {}|", b.dims.width.raw(), b.dims.height.raw(), b.dims.depth.raw()));
				signature(&b.list, out);
				out.push(']');
			}
			Node::Leaf(l) => match &l.kind {
				LeafKind::Text(t) => {
					out.push_str(&fmt!("(t {:?} {} {}", t.source(), l.dims.width.raw(), l.shift.raw()));
					for g in &t.run().glyphs {
						out.push_str(&fmt!(" {}:{}:{}:{}", g.id, g.x.to_bits(), g.y.to_bits(), g.adv.to_bits()));
					}
					out.push(')');
				}
				other => out.push_str(&fmt!("(leaf {:?} {})", std::mem::discriminant(other), l.dims.width.raw())),
			},
			Node::Glue(g)		=> out.push_str(&fmt!("(g {} {} {})", g.natural.raw(), g.stretch.raw(), g.shrink.raw())),
			Node::Penalty(p)	=> out.push_str(&fmt!("(p {:?})", p)),
			Node::Tag(Tag::Start(c))	=> out.push_str(&fmt!("(tag {}-{})", c.span().start, c.span().end)),
			Node::Tag(Tag::End(l))		=> out.push_str(&fmt!("(end {:?})", l)),
			other				=> out.push_str(&fmt!("({:?})", std::mem::discriminant(other))),
		}
	}
}

fn sig(nodes: &[Node]) -> String {
	let mut s = String::new();
	signature(nodes, &mut s);
	s
}

fn tags(nodes: &[Node]) -> Vec<(u32, u32)> {
	fn walk(n: &Node, out: &mut Vec<(u32, u32)>) {
		match n {
			Node::HBox(b) | Node::VBox(b)	=> for c in &b.list { walk(c, out); },
			Node::Tag(Tag::Start(c))		=> out.push((c.span().start, c.span().end)),
			_								=> (),
		}
	}
	let mut out = Vec::new();
	for n in nodes {
		walk(n, &mut out);
	}
	out
}

const PROSE: &str = "A paragraph with enough ordinary words in it to run on to a second line of the measure and a third \
	besides, set so that the breaks fall in the middle of it and not at its ends";

// The layout of the first paragraph of `src` on a fresh engine: what a miss makes.
fn cold(src: &str, w: f64, situation: ParSituation) -> Outcome<String> {
	let (mut engine, pars) = res!(paragraphs(src));
	Ok(sig(&res!(set(&mut engine, &pars[0], region(w, 10_000.0), situation))))
}

/// A paragraph set again is a hit and comes out as it did, and what no kept paragraph reads is not in the key:
/// the region's height.
#[test]
fn a_paragraph_set_again_is_a_hit_and_comes_out_the_same() -> Outcome<()> {
	let (mut engine, pars) = res!(paragraphs(&fmt!("{} #strong[and a strong word].\n", PROSE)));
	let first = res!(set(&mut engine, &pars[0], region(150.0, 10_000.0), ParSituation::First));
	assert_eq!(counts(&engine), (0, 1), "the first setting is a miss");
	assert_eq!(engine.caches.pars.len(), 1, "and is kept");
	let again = res!(set(&mut engine, &pars[0], region(150.0, 10_000.0), ParSituation::First));
	assert_eq!(counts(&engine), (1, 1), "the same paragraph, set again, is a hit");
	assert!(sig(&first) == sig(&again), "a hit came out differently from the miss");
	let taller = res!(set(&mut engine, &pars[0], region(150.0, 40.0), ParSituation::First));
	assert_eq!(counts(&engine), (2, 1), "the region's height is not in the key");
	assert!(sig(&first) == sig(&taller), "a shorter region changed the lines");
	assert!(first.len() >= 4, "the paragraph sets in {} lines, too few to test with", first.len());
	Ok(())
}

/// Each field of the key, changed alone, is a miss, and the miss comes out as a cold layout does.
#[test]
fn each_field_of_the_key_changed_is_a_miss_that_comes_out_cold() -> Outcome<()> {
	let plain = fmt!("{}.\n", PROSE);
	let indented = fmt!("#set par(first-line-indent: 1em)\n{}.\n", PROSE);
	let first = ParSituation::First;
	// (name, the paragraph set first and its situation, the paragraph set next, its width and situation)
	let cases: Vec<(&str, &str, ParSituation, String, f64, ParSituation)> = vec![
		("text",			&plain, first, fmt!("{} again.\n", PROSE),								150.0, first),
		("style",			&plain, first, fmt!("#set text(size: 11pt)\n{}.\n", PROSE),			150.0, first),
		("justify",			&plain, first, fmt!("#set par(justify: true)\n{}.\n", PROSE),			150.0, first),
		("hanging indent",	&plain, first, fmt!("#set par(hanging-indent: 1em)\n{}.\n", PROSE),	150.0, first),
		("width",			&plain, first, plain.clone(),											151.0, first),
		("situation",		&indented, first, indented.clone(),										150.0, ParSituation::Consecutive),
		("smart quote",		&plain, first, fmt!("{}.\n", PROSE.replace("ordinary", "\"ordinary\"")), 150.0, first),
	];
	for (name, before, before_at, src, w, situation) in cases {
		let (mut engine, pars) = res!(paragraphs(before));
		let _ = res!(set(&mut engine, &pars[0], region(150.0, 10_000.0), before_at));
		// The next paragraph is set on an engine that holds the memo the first one made.
		let (mut next, next_pars) = res!(paragraphs(&src));
		next.caches = std::mem::take(&mut engine.caches);
		let made = res!(set(&mut next, &next_pars[0], region(w, 10_000.0), situation));
		assert_eq!(counts(&next), (0, 2), "{}: a paragraph that differs in this field alone was a hit", name);
		let want = res!(cold(&src, w, situation));
		assert!(sig(&made) == want, "{}: the miss came out differently from a cold layout", name);
	}
	Ok(())
}

/// The `Config` a `par` element sets is in the key though no child's chain names it: the same children under
/// `par(justify: true)` are a different paragraph.
#[test]
fn a_par_setting_no_child_names_is_in_the_key() -> Outcome<()> {
	let src = fmt!("{}.\n\n#par(justify: true)[{}.]\n", PROSE, PROSE);
	let (mut engine, pars) = res!(paragraphs(&src));
	assert_eq!(pars.len(), 2);
	let plain = res!(set(&mut engine, &pars[0], region(150.0, 10_000.0), ParSituation::First));
	let justified = res!(set(&mut engine, &pars[1], region(150.0, 10_000.0), ParSituation::First));
	assert_eq!(counts(&engine), (0, 2), "the justified paragraph took the plain one's lines");
	assert!(sig(&plain) != sig(&justified), "justifying changed nothing, so the case tests nothing");
	let (mut fresh, fresh_pars) = res!(paragraphs(&src));
	let want = res!(set(&mut fresh, &fresh_pars[1], region(150.0, 10_000.0), ParSituation::First));
	assert!(sig(&justified) == sig(&want), "the justified paragraph came out differently from a cold layout");
	Ok(())
}

/// A paragraph with an inline box or an equation is never kept. One that warns is kept, and a hit warns as the
/// miss did, because a hit prepares the paragraph again.
#[test]
fn a_paragraph_with_a_box_or_an_equation_is_never_kept_and_a_hit_warns_again() -> Outcome<()> {
	let sources = [
		("box",			fmt!("{} #box(width: 20pt, height: 8pt, fill: gray) and on.\n", PROSE)),
		("equation",	fmt!("{} $x^2 + y$ and on.\n", PROSE)),
	];
	for (name, src) in sources {
		let (mut engine, pars) = res!(paragraphs(&src));
		assert_eq!(pars.len(), 1, "{}: one paragraph", name);
		let one = res!(set(&mut engine, &pars[0], region(150.0, 10_000.0), ParSituation::First));
		let two = res!(set(&mut engine, &pars[0], region(150.0, 10_000.0), ParSituation::First));
		assert_eq!(counts(&engine), (0, 2), "{}: a paragraph that cannot be kept was a hit", name);
		assert_eq!(engine.caches.pars.len(), 0, "{}: and was kept", name);
		assert!(sig(&one) == sig(&two), "{}: laid out twice it came out differently", name);
	}
	// The warning is the engine's, so a hit on a fresh engine that holds the memo shows it raised again.
	let src = fmt!("{} #link((page: 1, x: 0pt, y: 0pt))[and a link to a position] and on.\n", PROSE);
	let (mut a, pars_a) = res!(paragraphs(&src));
	let miss = res!(set(&mut a, &pars_a[0], region(150.0, 10_000.0), ParSituation::First));
	assert_eq!(a.diags.len(), 1, "the miss warns");
	let (mut b, pars_b) = res!(paragraphs(&src));
	b.caches = std::mem::take(&mut a.caches);
	let hit = res!(set(&mut b, &pars_b[0], region(150.0, 10_000.0), ParSituation::First));
	assert_eq!(counts(&b), (1, 1), "the warning paragraph was kept and hit");
	assert_eq!(b.diags.len(), 1, "and the hit warned as the miss did");
	assert!(sig(&hit) == sig(&miss), "the hit came out differently from the miss");
	Ok(())
}

/// A hit puts the tags of the current source in the lines: a tag holds the span of the element it starts,
/// which the fingerprint leaves out.
#[test]
fn a_hit_takes_its_tags_from_the_current_source() -> Outcome<()> {
	let body = fmt!("{} #strong[the strong word] and #emph[a slanted one] to end.\n", PROSE);
	let moved = fmt!("\n\n\n\n// A comment above the paragraph moves every span in it.\n\n{}", body);
	let (mut a, pars_a) = res!(paragraphs(&body));
	let first = res!(set(&mut a, &pars_a[0], region(150.0, 10_000.0), ParSituation::First));
	let (mut b, pars_b) = res!(paragraphs(&moved));
	b.caches = std::mem::take(&mut a.caches);
	let hit = res!(set(&mut b, &pars_b[0], region(150.0, 10_000.0), ParSituation::First));
	assert_eq!(counts(&b), (1, 1), "the edited-above paragraph is a hit");
	let want = res!(cold(&moved, 150.0, ParSituation::First));
	assert_eq!(tags(&hit).len(), 2, "strong and emph each start a tag");
	assert_ne!(tags(&first), tags(&hit), "the spans did not move, so the case tests nothing");
	assert!(sig(&hit) == want, "the hit's lines differ from a cold layout of the same source");
	Ok(())
}

/// A hit builds its lines from the kept breaks, and they are the miss's to the bit whatever ends them: a
/// hyphen, a mandatory break, a right-to-left line, and the first-fit breaker's choices as the optimised one's.
#[test]
fn a_hit_is_a_miss_to_the_bit_across_the_kinds_of_break() -> Outcome<()> {
	let long = "Incomprehensibilities notwithstanding, extraordinarily uncharacteristically \
		disproportionate counterrevolutionaries institutionalised internationalisation, \
		electroencephalographically misunderstanding indistinguishable phenomenological \
		interdisciplinary considerations";
	// (name, source, width, the source the case must differ from to test anything)
	let cases: Vec<(&str, String, f64, Option<String>)> = vec![
		("hyphen",
			fmt!("#set text(hyphenate: true, lang: \"en\")\n#set par(justify: true)\n{}.\n", long), 110.0,
			Some(fmt!("#set text(hyphenate: false, lang: \"en\")\n#set par(justify: true)\n{}.\n", long))),
		("mandatory",
			fmt!("#set par(justify: true)\nA first short part \\\nthen a second \\\n{}.\n", PROSE), 150.0,
			Some(fmt!("#set par(justify: true)\nA first short part then a second {}.\n", PROSE))),
		("right to left",
			fmt!("#set par(justify: true, first-line-indent: (amount: 12pt, all: true))\n#set text(dir: rtl)\n{}.\n", PROSE),
			150.0, Some(fmt!("#set par(justify: true, first-line-indent: (amount: 12pt, all: true))\n{}.\n", PROSE))),
		("first fit",
			fmt!("#set par(linebreaks: \"simple\", justify: true)\n#set text(hyphenate: true, lang: \"en\")\n{}.\n", long),
			110.0, None),
	];
	for (name, src, w, witness) in cases {
		let (mut engine, pars) = res!(paragraphs(&src));
		assert_eq!(pars.len(), 1, "{}: one paragraph", name);
		let miss = res!(set(&mut engine, &pars[0], region(w, 10_000.0), ParSituation::First));
		let hit = res!(set(&mut engine, &pars[0], region(w, 10_000.0), ParSituation::First));
		assert_eq!(counts(&engine), (1, 1), "{}: the second setting was not a hit", name);
		assert!(sig(&hit) == sig(&miss), "{}: the hit came out differently from the miss", name);
		assert!(sig(&hit) == res!(cold(&src, w, ParSituation::First)), "{}: the hit differs from a cold layout", name);
		assert!(miss.len() >= 3, "{}: the paragraph sets in {} lines, too few to test with", name, miss.len());
		if let Some(other) = witness {
			assert!(sig(&miss) != res!(cold(&other, w, ParSituation::First)),
				"{}: the case sets as it would without its kind of break, so it tests nothing", name);
		}
	}
	Ok(())
}

/// A budget of none keeps nothing, and a small one refuses what does not fit; neither changes the lines.
#[test]
fn a_small_budget_refuses_and_changes_nothing() -> Outcome<()> {
	let src = fmt!("{}.\n\n{} again.\n", PROSE, PROSE);
	let (mut engine, pars) = res!(paragraphs(&src));
	let (mut free, free_pars) = res!(paragraphs(&src));
	free.caches.begin(Fingerprint::default());
	let a = res!(set(&mut free, &free_pars[0], region(150.0, 10_000.0), ParSituation::First));
	let held = free.caches.pars.bytes();
	assert!(held > 0, "a paragraph's lines are worth some bytes");

	engine.caches = Caches::new(Budgets { pars: 0, ..Budgets::default() });
	engine.caches.begin(Fingerprint::default());
	let none = res!(set(&mut engine, &pars[0], region(150.0, 10_000.0), ParSituation::First));
	let none_again = res!(set(&mut engine, &pars[0], region(150.0, 10_000.0), ParSituation::First));
	assert_eq!((engine.caches.pars.len(), counts(&engine)), (0, (0, 2)), "a budget of none kept a paragraph");
	assert_eq!(engine.caches.pars.refused(), 0, "a budget of none computes no key, so offers nothing to refuse");
	assert!(sig(&none) == sig(&a) && sig(&none_again) == sig(&a), "a budget changed the lines");

	// Room for one paragraph and not two.
	let (mut small, small_pars) = res!(paragraphs(&src));
	small.caches = Caches::new(Budgets { pars: held + held / 2, ..Budgets::default() });
	small.caches.begin(Fingerprint::default());
	let _ = res!(set(&mut small, &small_pars[0], region(150.0, 10_000.0), ParSituation::First));
	let second = res!(set(&mut small, &small_pars[1], region(150.0, 10_000.0), ParSituation::First));
	assert_eq!(small.caches.pars.len(), 1, "the second paragraph did not fit and was kept");
	assert_eq!(small.caches.pars.refused(), 1);
	let (mut fresh, fresh_pars) = res!(paragraphs(&src));
	let want = res!(set(&mut fresh, &fresh_pars[1], region(150.0, 10_000.0), ParSituation::First));
	assert!(sig(&second) == sig(&want), "a refused paragraph came out differently from a cold layout");
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ SESSION LEVEL                                                              │
// └───────────────────────────────────────────────────────────────────────────┘

const HEAD: &str = "#set page(width: 220pt, height: 130pt, margin: 16pt)\n#set text(size: 9pt)\n";

const TAILS: [&str; 8] = ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel"];

// A document of eight paragraphs, each its own, after `extra` settings.
fn doc(extra: &str, edit: Option<usize>) -> String {
	let mut s = String::from(HEAD);
	s.push_str(extra);
	for (i, t) in TAILS.iter().enumerate() {
		let more = if edit == Some(i) { " and an edited clause" } else { "" };
		s.push_str(&format!("Paragraph {} has enough ordinary words in it to run on to a second line of the page, \
			and then {}{}.\n\n", i, t, more));
	}
	s
}

fn scratch(name: &str) -> PathBuf {
	let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("parmemo_session").join(name);
	let _ = std::fs::remove_dir_all(&d);
	std::fs::create_dir_all(&d).expect("the case's directory");
	std::fs::canonicalize(&d).expect("the case's canonical path")
}

fn session(budgets: Budgets) -> Session {
	let mut s = Session::new(FontStore::default());
	s.set_budgets(budgets);
	s
}

// The PDF and the counters of a compile of `text` under `session`.
fn build(session: &mut Session, dir: &Path, text: &str) -> (Vec<u8>, Counters) {
	let main = dir.join("main.typ");
	std::fs::write(&main, text).expect("a project file");
	let mut sink = PdfSink::new().expect("a PDF sink");
	let done = session.compile(&main, dir, &mut sink, None, None, false).expect("the main is readable");
	if let Err(e) = &done.laid {
		panic!("the compile failed: {}", e.plain());
	}
	(sink.output().expect("a finished PDF").to_vec(), done.counters)
}

/// An unchanged recompile takes every paragraph from the memo, and makes the same bytes.
#[test]
fn an_unchanged_recompile_hits_every_paragraph() {
	let dir = scratch("again");
	let text = doc("", None);
	let mut s = session(Budgets::default());
	let (first, c1) = build(&mut s, &dir, &text);
	assert!(c1.par_misses >= 8, "a first compile lays the eight paragraphs, {:?}", c1);
	let (second, c2) = build(&mut s, &dir, &text);
	assert_eq!(c2.par_misses, 0, "a recompile of the same source laid a paragraph afresh: {:?}", c2);
	assert!(c2.par_hits >= 8, "and hit {} times for eight paragraphs", c2.par_hits);
	assert!(first == second, "the recompile made different bytes");
	assert_eq!(s.caches().pars.len(), 8, "the memo holds the eight paragraphs");
}

/// An edit misses the edited paragraph alone, and the bytes are a cold compile's, with the memo on and off.
#[test]
fn an_edit_misses_the_edited_paragraph_alone_and_warm_equals_cold() {
	let dir = scratch("edit");
	let mut s = session(Budgets::default());
	let _ = build(&mut s, &dir, &doc("", None));
	let edited = doc("", Some(3));
	let (warm, c) = build(&mut s, &dir, &edited);
	assert_eq!(c.par_misses, 1, "the edit laid more than the edited paragraph: {:?}", c);
	assert!(c.par_hits >= 7, "the other seven paragraphs were not all hits: {:?}", c);
	let (cold_on, _) = build(&mut session(Budgets::default()), &scratch("edit_cold"), &edited);
	let off = Budgets { pars: 0, ..Budgets::default() };
	let (cold_off, c_off) = build(&mut session(off), &scratch("edit_off"), &edited);
	assert_eq!((c_off.par_hits, c_off.par_misses > 0), (0, true), "a budget of none hit: {:?}", c_off);
	assert!(warm == cold_on, "warm bytes differ from cold bytes with the memo on");
	assert!(warm == cold_off, "warm bytes differ from cold bytes with the memo off");
}

/// Each field of the key moved by a document-wide setting misses every paragraph the earlier compile kept:
/// the counters are those of a session that kept nothing, and the bytes are a cold compile's.
#[test]
fn a_changed_key_field_leaves_the_earlier_entries_unread() {
	let settings = [
		("style",			"#set text(size: 10pt)\n"),
		("justify",			"#set par(justify: true)\n"),
		("hanging indent",	"#set par(hanging-indent: 1em)\n"),
		("first-line indent", "#set par(first-line-indent: 1em)\n"),
		("width",			"#set page(columns: 2)\n"),
		("margin",			"#set page(margin: 24pt)\n"),
	];
	for (name, extra) in settings {
		let dir = scratch(&format!("key_{}", name.replace(' ', "_")));
		let mut s = session(Budgets::default());
		let _ = build(&mut s, &dir, &doc("", None));
		let changed = doc(extra, None);
		let (warm, cw) = build(&mut s, &dir, &changed);
		let (cold, cc) = build(&mut session(Budgets::default()), &scratch(&format!("key_cold_{}", name.replace(' ', "_"))), &changed);
		assert_eq!((cw.par_hits, cw.par_misses), (cc.par_hits, cc.par_misses),
			"{}: the earlier compile's paragraphs were read under a changed key", name);
		assert!(warm == cold, "{}: warm bytes differ from cold bytes", name);
	}
}

/// A paragraph with an inline box or an equation is laid afresh every compile, and the document is still the
/// cold bytes.
#[test]
fn paragraphs_with_boxes_and_equations_are_laid_afresh() {
	let dir = scratch("gate");
	let mut text = doc("", None);
	text.push_str("A paragraph with an inline #box(width: 14pt, height: 7pt, fill: gray) box in it, run on a while.\n\n");
	text.push_str("A paragraph with an equation $x^2 + y$ in it, run on a while longer to wrap the line.\n\n");
	let mut s = session(Budgets::default());
	let _ = build(&mut s, &dir, &text);
	let (second, c) = build(&mut s, &dir, &text);
	assert!(c.par_misses >= 2, "the box and equation paragraphs were kept: {:?}", c);
	assert!(c.par_hits >= 8, "the eight plain ones were not hits: {:?}", c);
	assert_eq!(s.caches().pars.len(), 8, "only the eight plain paragraphs are kept");
	let (cold, _) = build(&mut session(Budgets::default()), &scratch("gate_cold"), &text);
	assert!(second == cold, "warm bytes differ from cold bytes");
}

/// A small budget refuses entries and changes nothing the compile makes; a budget of none keeps none.
#[test]
fn the_budget_bounds_the_memo_and_never_the_bytes() {
	let text = doc("", None);
	let mut whole = session(Budgets::default());
	let (full, _) = build(&mut whole, &scratch("budget_full"), &text);
	let each = whole.caches().pars.bytes() / whole.caches().pars.len().max(1);	// a paragraph's bytes, on average
	let none = Budgets { pars: 0, ..Budgets::default() };
	let mut s = session(none);
	let (a, c) = build(&mut s, &scratch("budget_none"), &text);
	let (b, c2) = build(&mut s, &scratch("budget_none"), &text);
	assert_eq!((s.caches().pars.len(), c.par_hits, c2.par_hits), (0, 0, 0), "a budget of none kept something");
	assert!(a == full && b == full, "a budget of none changed the bytes");
	let small = Budgets { pars: each * 3, ..Budgets::default() };	// about three of the eight
	let mut s = session(small);
	let (a, _) = build(&mut s, &scratch("budget_small"), &text);
	let (b, c2) = build(&mut s, &scratch("budget_small"), &text);
	assert!(s.caches().pars.len() > 0 && s.caches().pars.len() < 8, "a small budget holds {} of eight paragraphs", s.caches().pars.len());
	assert!(s.caches().pars.refused() > 0, "a small budget refused nothing");
	assert!(c2.par_hits > 0 && c2.par_misses > 0, "and the recompile mixed hits and misses: {:?}", c2);
	assert!(a == full && b == full, "a small budget changed the bytes");
}

/// A paragraph whose text a `context` reads is realised before it is laid, so what was read is in its
/// children and in the key: moved to another page by an edit above, it misses and shows the new page.
#[test]
fn a_paragraph_that_shows_what_a_context_read_misses_when_the_read_changes() {
	let dir = scratch("context");
	let tail = "This paragraph is on page #context here().page() of the document, and runs on a little.\n\n";
	let base = format!("{}{}", doc("", None), tail);
	let mut s = session(Budgets::default());
	let _ = build(&mut s, &dir, &base);
	let edited = format!("{}{}{}", doc("", None), doc("", None).trim_start_matches(HEAD), tail);
	let (warm, _) = build(&mut s, &dir, &edited);
	let (cold, _) = build(&mut session(Budgets::default()), &scratch("context_cold"), &edited);
	assert!(warm == cold, "warm bytes differ from cold bytes after the read changed");
}

/// The same paragraphs in a narrower block are the same children under the same chain, so only the region's
/// width tells them apart: the warm compile misses them and makes the cold bytes.
#[test]
fn a_narrower_block_misses_on_the_width_alone() {
	let body = doc("", None);
	let body = body.trim_start_matches(HEAD);
	let wide = format!("{}#block(width: 100%)[\n{}]\n", HEAD, body);
	let narrow = format!("{}#block(width: 80%)[\n{}]\n", HEAD, body);
	let dir = scratch("block_width");
	let mut s = session(Budgets::default());
	let _ = build(&mut s, &dir, &wide);
	let (warm, c) = build(&mut s, &dir, &narrow);
	let (cold, _) = build(&mut session(Budgets::default()), &scratch("block_width_cold"), &narrow);
	assert!(c.par_misses >= 8, "the narrower paragraphs took the wider ones' lines: {:?}", c);
	assert!(warm == cold, "warm bytes differ from cold bytes");
}
