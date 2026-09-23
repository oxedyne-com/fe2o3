//! The contents tree a reader's sidebar shows: the document outline nested by heading level, with each
//! branch open or closed, the rows that are showing, and which heading the reader is currently in. It
//! holds no pixels and no window, so the rules are the same whatever draws them; the native window
//! paints it, and the web reader's contents rail (`pearl-reader/index.html`) follows the same rules.

use oxedyne_fe2o3_austenite::emit::pearl::OutlineEntry;

/// A document outline arranged as a collapsible tree. Each entry nests under the nearest earlier entry
/// of a shallower level, so a level-3 heading directly after a level-1 heading is that heading's child.
/// Every branch starts open.
#[derive(Clone, Debug)]
pub struct Contents {
	entries:	Vec<OutlineEntry>,
	parent:		Vec<Option<usize>>,	// index of the enclosing entry
	depth:		Vec<usize>,			// nesting depth, 0 at the root
	kids:		Vec<bool>,			// does the entry hold at least one child?
	open:		Vec<bool>,			// is the branch expanded?
}

impl Contents {
	pub fn new(entries: Vec<OutlineEntry>) -> Self {
		let n = entries.len();
		let mut parent	= Vec::with_capacity(n);
		let mut depth	= Vec::with_capacity(n);
		let mut kids	= vec![false; n];
		// The open ancestors of the entry being placed, as (level, index).
		let mut stack: Vec<(u8, usize)> = Vec::new();
		for (i, e) in entries.iter().enumerate() {
			while let Some(&(lvl, _)) = stack.last() {
				if lvl >= e.level {
					stack.pop();
				} else {
					break;
				}
			}
			let p = stack.last().map(|&(_, idx)| idx);
			if let Some(pi) = p {
				kids[pi] = true;
			}
			parent.push(p);
			depth.push(stack.len());
			stack.push((e.level, i));
		}
		Self { entries, parent, depth, kids, open: vec![true; n] }
	}

	pub fn is_empty(&self) -> bool { self.entries.is_empty() }
	pub fn len(&self) -> usize { self.entries.len() }
	pub fn entries(&self) -> &[OutlineEntry] { &self.entries }
	pub fn depth(&self, idx: usize) -> usize { self.depth.get(idx).copied().unwrap_or(0) }

	/// Does the entry hold children, so that it can be opened and closed?
	pub fn has_children(&self, idx: usize) -> bool { self.kids.get(idx).copied().unwrap_or(false) }

	/// Is the entry's branch expanded? A leaf counts as open.
	pub fn is_open(&self, idx: usize) -> bool { self.open.get(idx).copied().unwrap_or(true) }

	/// Opens a closed branch or closes an open one. A leaf has nothing to fold, and the call reports
	/// whether anything changed.
	pub fn toggle(&mut self, idx: usize) -> bool {
		if !self.has_children(idx) {
			return false;
		}
		match self.open.get_mut(idx) {
			Some(o)	=> { *o = !*o; true },
			None	=> false,
		}
	}

	/// Is every enclosing branch of the entry open, so that its row shows?
	pub fn is_shown(&self, idx: usize) -> bool {
		let mut at = self.parent.get(idx).copied().flatten();
		while let Some(p) = at {
			if !self.is_open(p) {
				return false;
			}
			at = self.parent[p];
		}
		true
	}

	/// The entries whose rows show, in document order: everything not inside a closed branch.
	pub fn visible(&self) -> Vec<usize> {
		(0..self.entries.len()).filter(|&i| self.is_shown(i)).collect()
	}

	/// The row that stands for an entry: the entry itself when it shows, otherwise its nearest enclosing
	/// entry that does. The current section inside a closed branch is marked on that branch's row.
	pub fn shown_for(&self, idx: usize) -> usize {
		let mut row = idx;
		let mut at = self.parent.get(idx).copied().flatten();
		while let Some(p) = at {
			if !self.is_open(p) {
				row = p;
			}
			at = self.parent[p];
		}
		row
	}

	/// The heading the reader is in: the last placed entry whose position is at or above `at`, with
	/// `pos` mapping an entry's page (1-based) and y to the same scale as `at`. An entry the ledger never
	/// fixed has no position and is never current. `None` when the view is above the first heading.
	pub fn current<F>(&self, pos: F, at: f64) -> Option<usize>
		where F: Fn(&OutlineEntry) -> Option<f64>
	{
		let mut best: Option<(usize, f64)> = None;
		for (i, e) in self.entries.iter().enumerate() {
			if let Some(y) = pos(e) {
				if y <= at {
					// Ties go to the later entry: two headings on one line, the deeper one is current.
					let better = match best {
						Some((_, by))	=> y >= by,
						None			=> true,
					};
					if better {
						best = Some((i, y));
					}
				}
			}
		}
		best.map(|(i, _)| i)
	}
}
