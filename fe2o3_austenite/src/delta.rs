//! The changed-only page delta, for the browser live view.
//!
//! Daimond's live view rerenders on every keystroke. Returning the full set of per-page SVG strings each
//! compile costs the consumer a fresh parse of every page and costs the wasm heap the memory of the whole
//! rendered document -- the very memory the Austenite swap exists to save. This layer returns instead only
//! the pages whose rendered SVG has changed since the consumer's last-known set, keyed by a stable page
//! identity so the consumer caches the rest:
//!
//! ```text
//! { version, order: [id, ...], changed: [{ id, svg }, ...], reset }
//! ```
//!
//! * `order` is the full per-compile sequence of page ids. The consumer renders in this order, keeping a
//!   cache keyed by id and dropping any id absent from `order`.
//! * `changed` carries the SVG of only those pages whose id is new this compile (not in the consumer's
//!   prior set), so a page the consumer already holds is never resent.
//! * `version` is a monotonic counter, stepped each compile, for the consumer to order deltas by.
//! * `reset` is true on the first compile and any compile made against an empty prior set: `changed` then
//!   carries every page in `order`, since the consumer holds nothing to reuse.
//!
//! **The identity is a content hash of the WHOLE page frame** -- body and furniture (running head, folio)
//! alike -- so two compiles yield the same id for a page exactly when it renders to the same SVG bytes.
//! This is deliberately keyed on *content*, not on the page's ordinal position: inserting a page near the
//! front shifts every later page's position but leaves each later page's content-hash unchanged, so only
//! the inserted page is resent. A position-keyed cache would resend every page from the insertion point on,
//! because each would fall at a slot that last held a different page. It is a superset of the emit memo's
//! [`page_key`](crate::emit::svg), which hashes the body alone because the memo always redraws the
//! furniture fresh; the delta needs the folio in the key, since the consumer reuses the whole SVG by id and
//! a page whose only change is its printed folio genuinely renders differently.
//!
//! **One implementation, two feeds.** A [`Builder`] takes the pages of one compile a page at a time. It hashes
//! each page to its id, keeps the page, and renders nothing until [`Builder::finish`]. The evaluator's fixpoint
//! feeds it through [`DeltaSink`](crate::emit::sinks::DeltaSink), which restarts the builder when a pass is
//! discarded; the fixpoint decides a pass has settled only after its last page, so a page cannot be known
//! final when it is taken, and a page rendered then would be rendered for nothing whenever another pass
//! follows. A finished list of pages is fed through [`compute`].
//!
//! **The window and the kept pages (12 Oct 2026).** A browser that held the SVG of every page took a
//! 300-page book to 2.75-3.7 GB and was killed, because a page it dropped could not be had back. So a
//! [`Window`] names the span of `order` the consumer shows, and `finish` draws the changed pages inside it
//! alone; and `finish` hands back every page of the pass as a [`Kept`], which the door holds until the next
//! delta and from which [`Kept::render`] draws any page, byte for byte as the delta would have, without a
//! recompile. With no window, every changed page is sent, as before.
//!
//! **Residency.** The prior id set is supplied by the consumer on each compile (its `known` set, the ids it
//! still holds in its own cache) and returned as the new `order`; the compiler retains none of it between
//! compiles. Keeping it in the compiler would be wrong, not merely heavier: a consumer that cleared its cache
//! while the one singleton compiler lived on would be told nothing changed against a cache holding nothing.
//! What the compiler does keep is one compile's laid-out frames, which are a small fraction of their SVG
//! ([`Kept::heap_bytes`] counts them); the next delta replaces them, so they never accumulate. During a
//! compile the builder holds the frames of the pass in hand, and while it runs the door still holds the last
//! delta's, so the peak is two compiles' frames. At `finish` each page sent is drawn, its SVG handed to the
//! [`Changed`] consumer and dropped before the next is drawn, so at most one page's SVG lives in the
//! compiler, and a consumer that moves each one on, as the browser door does into a JavaScript array, holds
//! none at all.

use crate::emit::svg;
use crate::page::Page;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashSet;

/// A changed-only compile result: the per-compile page-id sequence, the SVG of only the pages whose id is
/// new since the prior set, the monotonic version and whether this was a full (reset) send.
#[derive(Debug)]
pub struct PageDelta {
	pub version:	u32,
	pub order:		Vec<u64>,			// every page's content id, in reading order
	pub changed:	Vec<(u64, String)>,	// (id, svg) for each id new this compile, rendered once
	pub reset:		bool,				// true when the prior set was empty, so `changed` carries all of `order`
	pub rendered:	u32,				// the SVG documents this compile drew, which is `changed.len()`
}

impl PageDelta {
	/// The whole delta from its head and the changed pages a [`Changed`] collected.
	pub fn of(head: Head, changed: Vec<(u64, String)>) -> Self {
		Self { version: head.version, order: head.order, changed, reset: head.reset, rendered: head.rendered }
	}
}

/// What a delta holds besides its changed pages, which went to a [`Changed`] as they were drawn.
#[derive(Debug)]
pub struct Head {
	pub version:	u32,
	pub order:		Vec<u64>,
	pub reset:		bool,
	pub rendered:	u32,
}

/// Where a changed page's SVG goes the moment it is drawn, so the compiler never holds the set. A `Vec`
/// collects them for a caller that wants the whole delta; the browser door pushes each into a JavaScript
/// array and keeps nothing.
pub trait Changed {
	fn take(&mut self, id: u64, svg: String) -> Outcome<()>;
}

impl Changed for Vec<(u64, String)> {
	fn take(&mut self, id: u64, svg: String) -> Outcome<()> {
		self.push((id, svg));
		Ok(())
	}
}

/// The span of `order` a consumer shows, as `order[from..to]`: a delta sends the SVG of changed pages inside
/// it only. A span reaching past the last page stops there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
	pub from:	usize,
	pub to:		usize,	// exclusive
}

impl Window {
	/// Does the window hold the page at `index` in `order`?
	pub fn holds(&self, index: usize) -> bool { index >= self.from && index < self.to }
}

/// Builds a delta one page at a time. Each page's id is its whole-frame content hash; a page whose id is
/// not in the consumer's `known` set, and which falls inside the window when there is one, is drawn at
/// [`finish`](Self::finish). An id is drawn at most once, so two byte-identical pages (which share an id)
/// send one SVG and appear twice in `order`, the consumer resolving both slots to the one cached page.
///
/// The builder holds every page of the pass in hand and no SVG, and hands the pages on at `finish` as a
/// [`Kept`], from which any page is drawn later on request: see the module note.
#[derive(Debug)]
pub struct Builder {
	known:		HashSet<u64>,
	reset:		bool,
	version:	u32,				// the version this delta steps to
	want:		Option<Window>,		// the pages whose SVG may be sent, all when none
	order:		Vec<u64>,
	pages:		Vec<Page>,			// every page of the pass, in reading order
	rendered:	u32,				// the SVG documents drawn, counted where each is
}

impl Builder {
	/// Starts a delta against the consumer's `known` ids (its last `order`, empty on its first compile).
	/// `prior_version` is the version the caller last returned; the delta steps it by one.
	pub fn new(known: &[u64], prior_version: u32) -> Self {
		Self::windowed(known, prior_version, None)
	}

	/// As [`new`](Self::new), sending the SVG of changed pages inside `want` only.
	pub fn windowed(known: &[u64], prior_version: u32, want: Option<Window>) -> Self {
		Self {
			known:		known.iter().copied().collect(),
			reset:		known.is_empty(),
			version:	prior_version.wrapping_add(1),
			want,
			order:		Vec::new(),
			pages:		Vec::new(),
			rendered:	0,
		}
	}

	/// Takes the next page in reading order.
	pub fn page(&mut self, page: Page) {
		self.order.push(svg::page_id(&page));
		self.pages.push(page);
	}

	/// Forgets every page taken: the pass that gave them was discarded, so none of them is in the document.
	pub fn restart(&mut self) {
		self.order.clear();
		self.pages.clear();
	}

	/// Draws each page the consumer lacks inside the window, in reading order, and hands its SVG to `out`
	/// before drawing the next; then hands back every page as a [`Kept`].
	pub fn finish<C: Changed>(mut self, out: &mut C) -> Outcome<(Head, Kept)> {
		let mut sent: HashSet<u64> = HashSet::new();
		for (i, (id, page)) in self.order.iter().zip(self.pages.iter()).enumerate() {
			let inside = match self.want {
				Some(w)	=> w.holds(i),
				None	=> true,
			};
			if !inside || self.known.contains(id) || !sent.insert(*id) {
				continue;
			}
			let drawn = res!(svg::render_page(page));
			self.rendered += 1;
			res!(out.take(*id, drawn));
		}
		let kept = Kept { version: self.version, order: self.order.clone(), pages: std::mem::take(&mut self.pages) };
		Ok((Head { version: self.version, order: self.order, reset: self.reset, rendered: self.rendered }, kept))
	}
}

/// Which page a request names: its content id, or its index in `order`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageAt {
	Id(u64),
	Index(usize),
}

/// Why a requested page could not be drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageMiss {
	Uncompiled,	// no delta has been compiled
	Gone,		// no page of the current compile has that id
	Range,		// the index is past the last page
}

impl PageMiss {
	pub fn code(self) -> &'static str {
		match self {
			Self::Uncompiled	=> "uncompiled",
			Self::Gone			=> "gone",
			Self::Range			=> "range",
		}
	}
}

/// The laid-out pages of the compile a delta came from, kept so that any one of them is drawn on request,
/// byte for byte as the delta would have drawn it, without compiling again. The next delta's pages replace
/// them, so one compile's frames are held at a time.
#[derive(Debug, Default)]
pub struct Kept {
	version:	u32,
	order:		Vec<u64>,
	pages:		Vec<Page>,
}

impl Kept {
	/// The version of the delta these pages came from.
	pub fn version(&self) -> u32 { self.version }

	pub fn order(&self) -> &[u64] { &self.order }

	pub fn len(&self) -> usize { self.pages.len() }

	pub fn is_empty(&self) -> bool { self.pages.is_empty() }

	/// The index in `order` of the page `at` names, its first when two pages share an id.
	pub fn index(&self, at: PageAt) -> Result<usize, PageMiss> {
		match at {
			PageAt::Index(i) if i < self.order.len()	=> Ok(i),
			PageAt::Index(_)							=> Err(PageMiss::Range),
			PageAt::Id(id)								=> match self.order.iter().position(|o| *o == id) {
				Some(i)	=> Ok(i),
				None	=> Err(PageMiss::Gone),
			},
		}
	}

	/// The id, index and SVG of the page `at` names.
	pub fn render(&self, at: PageAt) -> Outcome<Result<(u64, usize, String), PageMiss>> {
		let i = match self.index(at) {
			Ok(i)	=> i,
			Err(m)	=> return Ok(Err(m)),
		};
		match (self.order.get(i), self.pages.get(i)) {
			(Some(id), Some(page))	=> Ok(Ok((*id, i, res!(svg::render_page(page))))),
			_						=> Err(err!("Kept page {} has no frame among {}.", i, self.pages.len(); Bug)),
		}
	}

	/// The bytes the kept pages hold of their own; see [`Page::heap_bytes`].
	pub fn heap_bytes(&self) -> usize {
		self.order.capacity() * std::mem::size_of::<u64>()
			+ self.pages.capacity().saturating_sub(self.pages.len()) * std::mem::size_of::<Page>()
			+ self.pages.iter().map(|p| p.heap_bytes()).sum::<usize>()
	}
}

/// Computes the delta between a compile's finished `pages` and the consumer's `prior` id set, as one
/// [`Builder`] run over them. The pages are borrowed, so each is cloned to be held.
pub fn compute<'a, I>(pages: I, prior: &[u64], prior_version: u32) -> Outcome<PageDelta>
where
	I: IntoIterator<Item = &'a Page>,
{
	let mut build = Builder::new(prior, prior_version);
	for page in pages {
		build.page(page.clone());
	}
	let mut changed = Vec::new();
	let (head, _) = res!(build.finish(&mut changed));
	Ok(PageDelta::of(head, changed))
}
