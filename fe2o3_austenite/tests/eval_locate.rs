//! Locations across a relayout (QF3): content that lays a body out is realised at a place of its own, so laying
//! it out again, whatever was realised in between and in whatever region, locates what is in it as before.
//!
//! Each case realises a document, takes the prepared element its source made, and lays it out three times: in
//! a region, again after the document's own level has been realised once more (which takes ordinals of the
//! document's locator), and in a narrower region, which no cache of the last layout can answer. The footnote
//! the element holds must have one location in all three. A footnote placed twice is the fault: the flow dedupes
//! the notes it has composed by location, so a location that changes with the layout puts the note in again.

use oxedyne_fe2o3_austenite::eval::content::{
	Content,
	ElemKind,
};
use oxedyne_fe2o3_austenite::eval::locate::Location;
use oxedyne_fe2o3_austenite::eval::realise::{
	realise,
	Pair,
	RealiseMode,
};
use oxedyne_fe2o3_austenite::eval::styles::StyleChain;
use oxedyne_fe2o3_austenite::eval::{
	eval_source,
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::flow::block::{
	self,
	Regions,
};

use oxedyne_fe2o3_core::prelude::*;

use std::path::PathBuf;

/// A source, the kind of element it makes, and what laying that element out reaches.
const CASES: &[(&str, &str, ElemKind)] = &[
	("an unbreakable block",
		"#block(breakable: false)[A note#footnote[One.] and some words.]",						ElemKind::Block),
	("a block of full width, paragraph by paragraph",
		"#block(width: 100%)[First paragraph.\n\nSecond one#footnote[One.] with words.\n\nThird.]",	ElemKind::Block),
	("a note that a text rule makes in a block",
		"#show \"word\": it => [#it#footnote[One.]]\n#block(width: 100%)[First.\n\nSecond word here.]",	ElemKind::Block),
	("a shape",
		"#rect[A note#footnote[One.] and some words.]",											ElemKind::Rect),
	("a float",
		"#place(top, float: true)[A note#footnote[One.] and some words.]",						ElemKind::Place),
	("a stack",
		"#stack(dir: ttb, [First#footnote[One.]], [Second.])",									ElemKind::Stack),
	("a bullet list",
		"- an item#footnote[One.]\n- another",													ElemKind::List),
	("a table",
		"#table(columns: 2, [cell#footnote[One.]], [other])",									ElemKind::Table),
	("columns",
		"#columns(2)[A note#footnote[One.] and some words.]",									ElemKind::Columns),
	("a layout",
		"#layout(size => [A note#footnote[One.] and some words.])",								ElemKind::Layout),
	("a pad where blocks may not break",
		"#set block(breakable: false)\n#pad(left: 4pt)[A note#footnote[One.] and some words.]",	ElemKind::Pad),
];

fn realised(src: &str) -> Outcome<(Engine, Content, Vec<Pair>)> {
	let root = PathBuf::from("/");
	let mut world = World::new(root.clone());
	let id = res!(world.add_source(root.join("__locate.typ"), src.to_string()));
	let mut engine = Engine::new(world);
	let module = res!(eval_source(&mut engine, id));
	let pairs = res!(realise(&mut engine, &module.content, &StyleChain::root(), RealiseMode::Document));
	Ok((engine, module.content, pairs))
}

/// The locations of the footnotes in the frames the element lays out into, at a width.
fn notes(engine: &mut Engine, pair: &Pair, width: f64) -> Outcome<Vec<Location>> {
	let frames = res!(block::layout_fragment(engine, &pair.content, &pair.styles, Regions::one(width, 10_000.0, true, false)));
	let mut found = Vec::new();
	for f in &frames {
		block::find_footnotes(f, 0.0, &mut found);
	}
	Ok(found.into_iter().filter_map(|(_, c)| c.location()).collect())
}

#[test]
fn an_element_laid_out_again_locates_its_footnote_as_before() -> Outcome<()> {
	for (name, src, kind) in CASES {
		let (mut engine, content, pairs) = res!(realised(src));
		let pair = res!(pairs.iter().find(|p| p.content.is(*kind)).ok_or_else(|| err!(
			"{}: realisation made no {}", name, kind.path(); Test)));
		assert!(pair.content.place().is_some(), "{}: the element was given no place", name);
		let first = res!(notes(&mut engine, pair, 200.0));
		assert_eq!(first.len(), 1, "{}: the laid out frames hold {} footnote(s), not one", name, first.len());
		// The document's own level realised again takes ordinals of the document's locator.
		res!(realise(&mut engine, &content, &StyleChain::root(), RealiseMode::Document));
		let again = res!(notes(&mut engine, pair, 200.0));
		assert_eq!(first, again, "{}: laid out again, the footnote was located anew", name);
		let narrower = res!(notes(&mut engine, pair, 120.0));
		assert_eq!(first, narrower, "{}: laid out in a narrower region, the footnote was located anew", name);
	}
	assert_eq!(CASES.len(), 11, "a case was dropped");
	Ok(())
}

/// Two elements from one source span have places, and so footnote locations, of their own: a loop's blocks hold
/// notes that are two notes, not one.
#[test]
fn elements_from_one_span_keep_their_own_locations() -> Outcome<()> {
	let (mut engine, _, pairs) = res!(realised("#for i in range(2) [#block(breakable: false)[Word#footnote[Note.]]]"));
	let blocks: Vec<&Pair> = pairs.iter().filter(|p| p.content.is(ElemKind::Block)).collect();
	assert_eq!(blocks.len(), 2);
	let a = res!(notes(&mut engine, blocks[0], 200.0));
	let b = res!(notes(&mut engine, blocks[1], 200.0));
	assert_eq!((a.len(), b.len()), (1, 1));
	assert_ne!(a, b, "two blocks from one span located their notes alike");
	Ok(())
}
