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
//! **Residency.** The only state a caller keeps between compiles is the last `order` -- a `Vec` of 64-bit
//! ids. The rendered SVG of a page is emitted into `changed` when its id is new and then dropped; it is
//! never retained. So the wasm heap holds roughly one page's SVG plus one id per page, not the whole
//! rendered document.

use crate::emit::svg;
use crate::page::Page;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashSet;

/// A changed-only compile result: the per-compile page-id sequence, the SVG of only the pages whose id is
/// new since the prior set, the monotonic version and whether this was a full (reset) send.
pub struct PageDelta {
	pub version:	u32,
	pub order:		Vec<u64>,			// every page's content id, in reading order
	pub changed:	Vec<(u64, String)>,	// (id, svg) for each id new this compile, rendered once
	pub reset:		bool,				// true when the prior set was empty, so `changed` carries all of `order`
}

/// Computes the delta between this compile's `pages` and the consumer's `prior` id set (its last `order`,
/// empty on the first compile). Each page's id is its whole-frame content hash; a page whose id is not in
/// `prior` is rendered and carried in `changed`, a page whose id is already known is left for the consumer
/// to reuse. An id is rendered at most once per compile, so two byte-identical pages (which share an id)
/// send one SVG and appear twice in `order` -- the consumer resolves both slots to the one cached page.
///
/// `prior_version` is the version the caller last returned; the result steps it by one.
pub fn compute(pages: &[Page], prior: &[u64], prior_version: u32) -> Outcome<PageDelta> {
	let reset		= prior.is_empty();
	let known:	HashSet<u64>	= prior.iter().copied().collect();
	let mut order	= Vec::with_capacity(pages.len());
	let mut changed	= Vec::new();
	// The ids rendered so far this compile, so a repeated page is emitted once though `order` names it twice.
	let mut sent:	HashSet<u64>	= HashSet::new();
	for page in pages {
		let id = svg::page_id(page);
		order.push(id);
		if !known.contains(&id) && sent.insert(id) {
			let rendered = res!(svg::render_page(page));
			changed.push((id, rendered));
		}
	}
	Ok(PageDelta {
		version:	prior_version.wrapping_add(1),
		order,
		changed,
		reset,
	})
}
