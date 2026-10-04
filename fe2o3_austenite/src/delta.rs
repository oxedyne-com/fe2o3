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
//! each page to its id, keeps the page itself only when the consumer does not hold that id, and renders
//! nothing until [`Builder::finish`]. The evaluator's fixpoint feeds it through
//! [`DeltaSink`](crate::emit::sinks::DeltaSink), which restarts the builder when a pass is discarded; the
//! fixpoint decides a pass has settled only after its last page, so a page cannot be known final when it is
//! taken, and a page rendered then would be rendered for nothing whenever another pass follows. Holding the
//! page and drawing it once, after the pass that stands, costs one frame a page and saves every discarded
//! pass's SVG. A finished list of pages is fed through [`compute`].
//!
//! **Residency.** The prior id set is supplied by the consumer on each compile (its `known` set, the ids it
//! still holds in its own cache) and returned as the new `order`; the compiler retains none of it between
//! compiles. During a compile the builder holds one id for each page and one frame for each distinct page
//! the consumer lacks, those of the pass in hand only; a recompile in which the consumer holds the rest
//! holds almost no frame. At `finish` each frame is drawn, its SVG handed to the [`Changed`] consumer, and
//! both dropped before the next is drawn, so at most one page's SVG lives in the compiler, and a consumer
//! that moves each one on, as the browser door does into a JavaScript array, holds none at all. Nothing is
//! retained between compiles -- not the rendered document, and not the prior set. Keeping the prior set
//! in the compiler would also be wrong, not merely heavier: a consumer that cleared its cache while the one
//! singleton compiler lived on would be told nothing changed against a cache holding nothing.

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

/// Builds a delta one page at a time. Each page's id is its whole-frame content hash; a page whose id is
/// not in the consumer's `known` set is held to be drawn at [`finish`](Self::finish), one whose id is held
/// by the consumer is dropped as it is taken. An id is drawn at most once, so two byte-identical pages
/// (which share an id) send one SVG and appear twice in `order`, the consumer resolving both slots to the
/// one cached page.
///
/// The builder holds the frames of the pages the consumer lacks, those of the pass in hand, and no SVG:
/// see the module note.
#[derive(Debug)]
pub struct Builder {
	known:		HashSet<u64>,
	reset:		bool,
	version:	u32,				// the version this delta steps to
	order:		Vec<u64>,
	held:		Vec<(u64, Page)>,	// the new pages, in reading order, each id once
	queued:		HashSet<u64>,		// the ids in `held`, so a repeated page is held once though `order` names it twice
	rendered:	u32,				// the SVG documents drawn, counted where each is
}

impl Builder {
	/// Starts a delta against the consumer's `known` ids (its last `order`, empty on its first compile).
	/// `prior_version` is the version the caller last returned; the delta steps it by one.
	pub fn new(known: &[u64], prior_version: u32) -> Self {
		Self {
			known:		known.iter().copied().collect(),
			reset:		known.is_empty(),
			version:	prior_version.wrapping_add(1),
			order:		Vec::new(),
			held:		Vec::new(),
			queued:		HashSet::new(),
			rendered:	0,
		}
	}

	/// Takes the next page in reading order, and drops it unless the consumer lacks it.
	pub fn page(&mut self, page: Page) {
		let id = svg::page_id(&page);
		self.order.push(id);
		if !self.known.contains(&id) && self.queued.insert(id) {
			self.held.push((id, page));
		}
	}

	/// Forgets every page taken: the pass that gave them was discarded, so none of them is in the document.
	pub fn restart(&mut self) {
		self.order.clear();
		self.held.clear();
		self.queued.clear();
	}

	/// Draws each page held, in reading order, and hands its SVG to `out` before drawing the next.
	pub fn finish<C: Changed>(mut self, out: &mut C) -> Outcome<Head> {
		for (id, page) in std::mem::take(&mut self.held) {
			let drawn = res!(svg::render_page(&page));
			drop(page);
			self.rendered += 1;
			res!(out.take(id, drawn));
		}
		Ok(Head { version: self.version, order: self.order, reset: self.reset, rendered: self.rendered })
	}
}

/// Computes the delta between a compile's finished `pages` and the consumer's `prior` id set, as one
/// [`Builder`] run over them. The pages are borrowed, so each one the consumer lacks is cloned to be held.
pub fn compute<'a, I>(pages: I, prior: &[u64], prior_version: u32) -> Outcome<PageDelta>
where
	I: IntoIterator<Item = &'a Page>,
{
	let mut build = Builder::new(prior, prior_version);
	for page in pages {
		build.page(page.clone());
	}
	let mut changed = Vec::new();
	let head = res!(build.finish(&mut changed));
	Ok(PageDelta::of(head, changed))
}
