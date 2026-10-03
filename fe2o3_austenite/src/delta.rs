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
//! **One implementation, two feeds.** A [`Builder`] takes the pages of one compile a page at a time and keeps
//! the ids and the changed SVG only, never a page. The evaluator's fixpoint feeds it through
//! [`DeltaSink`](crate::emit::sinks::DeltaSink), which drops each page as it is taken and restarts the
//! builder when a pass is discarded, so no page of a pass that did not settle reaches `changed`; the curated
//! reader feeds it a finished list of pages through [`compute`].
//!
//! **Residency.** The prior id set is supplied by the consumer on each compile (its `known` set, the ids it
//! still holds in its own cache) and returned as the new `order`; the compiler retains none of it between
//! compiles. The rendered SVG of a page is emitted into `changed` when its id is new and then dropped, never
//! retained. So the wasm heap holds roughly one page's SVG plus one id per page during a compile, and
//! nothing between compiles -- not the whole rendered document, and not the prior set. Keeping the prior set
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
}

/// Builds a [`PageDelta`] one page at a time. Each page's id is its whole-frame content hash; a page whose
/// id is not in the consumer's `known` set is rendered and carried in `changed`, one whose id is held is left
/// for the consumer to reuse. An id is rendered at most once, so two byte-identical pages (which share an id)
/// send one SVG and appear twice in `order`, the consumer resolving both slots to the one cached page.
///
/// The builder holds ids and the SVG of the changed pages, never a [`Page`]: a page is borrowed to be hashed
/// and, if new, rendered, and is the caller's to drop.
#[derive(Debug)]
pub struct Builder {
	known:		HashSet<u64>,
	reset:		bool,
	version:	u32,				// the version this delta steps to
	order:		Vec<u64>,
	changed:	Vec<(u64, String)>,
	// The ids rendered so far, so a repeated page is sent once though `order` names it twice.
	sent:		HashSet<u64>,
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
			changed:	Vec::new(),
			sent:		HashSet::new(),
		}
	}

	/// Takes the next page in reading order.
	pub fn page(&mut self, page: &Page) -> Outcome<()> {
		let id = svg::page_id(page);
		self.order.push(id);
		if !self.known.contains(&id) && self.sent.insert(id) {
			let rendered = res!(svg::render_page(page));
			self.changed.push((id, rendered));
		}
		Ok(())
	}

	/// Forgets every page taken: the pass that gave them was discarded, so none of them is in the document.
	pub fn restart(&mut self) {
		self.order.clear();
		self.changed.clear();
		self.sent.clear();
	}

	pub fn finish(self) -> PageDelta {
		PageDelta {
			version:	self.version,
			order:		self.order,
			changed:	self.changed,
			reset:		self.reset,
		}
	}
}

/// Computes the delta between a compile's finished `pages` and the consumer's `prior` id set, as one
/// [`Builder`] run over them.
pub fn compute(pages: &[Page], prior: &[u64], prior_version: u32) -> Outcome<PageDelta> {
	let mut build = Builder::new(prior, prior_version);
	for page in pages {
		res!(build.page(page));
	}
	Ok(build.finish())
}
