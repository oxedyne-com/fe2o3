use oxedyne_fe2o3_pearlite::contents::Contents;

use oxedyne_fe2o3_austenite::emit::pearl::{
	OutlineEntry,
	PearlDoc,
};
use oxedyne_fe2o3_austenite::ir::Sp;

fn entry(level: u8, title: &str, page: Option<u32>, y_pt: f64) -> OutlineEntry {
	OutlineEntry {
		level,
		number:	String::new(),
		title:	title.to_string(),
		page,
		y:		page.map(|_| Sp::from_pt(y_pt)),
	}
}

// 0 Intro(1)  1 Scope(2)  2 Detail(3)  3 Aside(2)  4 Method(1)  5 Lost(2, unplaced)  6 Deep(3, straight
// under a level 1).
fn sample() -> Contents {
	Contents::new(vec![
		entry(1, "Intro",	Some(1), 100.0),
		entry(2, "Scope",	Some(1), 300.0),
		entry(3, "Detail",	Some(2), 50.0),
		entry(2, "Aside",	Some(2), 400.0),
		entry(1, "Method",	Some(3), 80.0),
		entry(2, "Lost",	None,    0.0),
		entry(3, "Deep",	Some(3), 500.0),
	])
}

#[test]
fn test_contents_nests_by_level() {
	let c = sample();
	let depths: Vec<usize> = (0..c.len()).map(|i| c.depth(i)).collect();
	assert_eq!(depths, vec![0, 1, 2, 1, 0, 1, 2]);
	let kids: Vec<bool> = (0..c.len()).map(|i| c.has_children(i)).collect();
	assert_eq!(kids, vec![true, true, false, false, true, true, false]);
}

#[test]
fn test_contents_folding_hides_descendants_and_marks_the_branch() {
	let mut c = sample();
	assert_eq!(c.visible(), vec![0, 1, 2, 3, 4, 5, 6]);
	assert!(c.toggle(0));
	assert_eq!(c.visible(), vec![0, 4, 5, 6]);
	// The current heading inside a folded branch is shown on the branch's row.
	assert_eq!(c.shown_for(2), 0);
	assert_eq!(c.shown_for(5), 5);
	// A leaf does not fold.
	assert!(!c.toggle(2));
	assert!(c.toggle(0));
	assert_eq!(c.visible().len(), 7);
}

#[test]
fn test_contents_current_is_the_last_heading_above_the_view() {
	let c = sample();
	// A simple stack: 1000 pixels a page, one pixel a point.
	let pos = |e: &OutlineEntry| e.page.map(|p| (p as f64 - 1.0) * 1000.0 + e.y.map(|y| y.to_pt()).unwrap_or(0.0));
	assert_eq!(c.current(pos, 0.0), None);
	assert_eq!(c.current(pos, 100.0), Some(0));
	assert_eq!(c.current(pos, 1100.0), Some(2));
	assert_eq!(c.current(pos, 1450.0), Some(3));
	// The unplaced heading is never current.
	assert_eq!(c.current(pos, 2090.0), Some(4));
	assert_eq!(c.current(pos, 9000.0), Some(6));
}

#[test]
fn test_contents_from_the_hierarchy_sample() {
	let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fe2o3_austenite/web/pearl-reader/samples/hierarchy.prl");
	let doc = match PearlDoc::read_file(path) {
		Ok(d)	=> d,
		Err(e)	=> panic!("{}", e),
	};
	let outline = match doc.outline() {
		Ok(o)	=> o,
		Err(e)	=> panic!("{}", e),
	};
	let c = Contents::new(outline);
	assert!(c.len() > 3, "the hierarchy sample carries an outline");
	assert!((0..c.len()).any(|i| c.depth(i) > 0), "and it nests");
	assert!(c.entries().iter().all(|e| e.page.is_some()), "every heading is placed");
}
