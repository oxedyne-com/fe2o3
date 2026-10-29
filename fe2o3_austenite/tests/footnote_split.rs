//! A footnote whose entry splits across a page break is set once: its first part at the foot of the page
//! its marker was offered on, the rest at the top of the next page's foot area, as Typst sets it.
//!
//! The marker sits in a nested list item, a breakable block. The note's reservation pushes the marker's
//! line to the next page, so the block's next fragment carries the marker again; the flow must still know
//! the note as placed there, or it sets the whole entry a second time below its continuation (the
//! OxedynePreSeed fault of 2026-10-29, where page 5 repeated footnote 2 in full).

use oxedyne_fe2o3_austenite::driver::Recorder;
use oxedyne_fe2o3_austenite::eval::content::{
	Content,
	ElemKind,
};
use oxedyne_fe2o3_austenite::eval::styles::StyleChain;
use oxedyne_fe2o3_austenite::eval::{
	eval_source,
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::flow;
use oxedyne_fe2o3_austenite::ledger::Position;

use oxedyne_fe2o3_core::prelude::*;

use std::path::PathBuf;

const SOURCE: &str = "\
#set page(width: 200pt, height: 200pt, margin: 20pt)
#set text(size: 10pt)
#lorem(9)

- *Outer* item:
  - #lorem(3) they pay#footnote[Zyx #lorem(80)] and #lorem(20)
  - #lorem(10)
- Second outer item.
";

/// The pages each footnote entry's start was placed on.
#[derive(Default)]
struct Entries {
	pages: Vec<u32>,
}

impl Recorder for Entries {
	fn record(&mut self, elem: &Content, pos: Position, _parent: Option<u64>) {
		if elem.is(ElemKind::FootnoteEntry) {
			self.pages.push(pos.page);
		}
	}

	fn mark(&mut self, _id: u64) {}
}

#[test]
fn a_footnote_split_across_pages_is_set_once() -> Outcome<()> {
	let root	= PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
	let mut world = World::new(root.clone());
	let id		= res!(world.add_source(root.join("footnote_split.typ"), SOURCE.to_string()));
	let mut engine = Engine::new(world);
	let module	= res!(eval_source(&mut engine, id));
	let mut paginator = flow::paginate(&module.content, &StyleChain::root());
	let mut entries = Entries::default();
	let mut pages = 0;
	while res!(paginator.next_placed(&mut engine, &mut entries)).is_some() {
		pages += 1;
	}
	if pages < 2 {
		return Err(err!("The fixture must break its footnote across pages, but it laid {} page(s).", pages; Test));
	}
	if entries.pages != vec![1] {
		return Err(err!(
			"The split footnote's entry was started on pages {:?}; it must start once, on page 1, its \
			continuation alone following on page 2.", entries.pages; Test));
	}
	Ok(())
}
