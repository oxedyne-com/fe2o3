//! Streaming pagination holds one page at a time (U6b-S1, addendum section 1).
//!
//! A document of more than `PAGES` pages, paragraphs and blocks, is paginated through a counting sink: each
//! page's body is placed and dropped before the next is asked for. The process's resident memory is read at
//! page `AT` and at page `PAGES`, and the growth per page between them must stay under `BOUND`, set from the
//! measured slope of the correct pipeline. A pipeline that kept its pages (a `Vec` of bodies, frames or placed pages) grows by a
//! page's size per page and fails the bound. It is the only test in this target, so nothing else moves the
//! process's memory while it reads it.

use oxedyne_fe2o3_austenite::driver::Nowhere;
use oxedyne_fe2o3_austenite::eval::styles::StyleChain;
use oxedyne_fe2o3_austenite::eval::{
	eval_source,
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::flow;
use oxedyne_fe2o3_austenite::page::Page;
use oxedyne_fe2o3_sys::proc_self::ProcSelf;

use oxedyne_fe2o3_core::prelude::*;

use std::path::Path;

const PAGES:	u32		= 1000;
const AT:		u32		= 100;
const BOUND:	f64		= 4096.0;	// bytes a page may add between page AT and the last

/// A document longer than `PAGES` pages: a paragraph of text, then a filled and stroked block, over and over,
/// so that pages fill with lines, split blocks and spacing alike.
fn source() -> String {
	fmt!(
		"#set page(width: 200pt, height: 200pt, margin: 20pt)\n\
		#set text(size: 10pt)\n\
		#for i in range({}) {{ [#lorem(40)#parbreak()]; block(width: 100%, height: 16pt, fill: luma(235), stroke: 0.5pt) }}\n",
		PAGES * 2)
}

/// Takes each page and drops it, as the pipeline's counting sink does.
struct CountSink {
	pages:	u32,
}

impl CountSink {
	fn page(&mut self, page: Page) {
		self.pages += 1;
		drop(page);
	}
}

fn resident() -> Outcome<f64> { Ok(res!(ProcSelf::sample()).rss_bytes as f64) }

#[test]
fn pagination_holds_one_page_at_a_time() -> Outcome<()> {
	let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("eval_stream");
	res!(std::fs::create_dir_all(&dir).map_err(|e| err!(e, "Could not make {}.", dir.display(); IO, File)));
	let file = dir.join("long.typ");
	res!(std::fs::write(&file, source()).map_err(|e| err!(e, "Could not write {}.", file.display(); IO, File)));
	let mut world = World::new(dir.clone());
	let id = res!(world.load(&file));
	let mut engine = Engine::new(world);
	let module = res!(eval_source(&mut engine, id));
	let mut paginator = flow::paginate(&module.content, &StyleChain::root());
	let mut sink = CountSink { pages: 0 };
	let (mut at, mut last) = (0.0, 0.0);
	while let Some((page, _setup)) = res!(paginator.next_placed(&mut engine, &mut Nowhere)) {
		sink.page(page);
		if sink.pages == AT {
			at = res!(resident());
		}
		if sink.pages == PAGES {
			last = res!(resident());
			break;
		}
	}
	assert_eq!(sink.pages, PAGES, "the document should lay out to at least {} pages", PAGES);
	let slope = (last - at) / (PAGES - AT) as f64;
	println!("resident {:.0} bytes at page {}, {:.0} at page {}: {:.1} bytes a page", at, AT, last, PAGES, slope);
	assert!(slope < BOUND, "resident memory grew {:.1} bytes a page between pages {} and {}, over {}",
		slope, AT, PAGES, BOUND);
	Ok(())
}
