//! The incremental content-hash memo, for a live edit-and-re-render loop (Daimond's browser view).
//!
//! A straight recompile re-authors and re-emits the whole document on every keystroke, which is the
//! ~15x live-view regression that blocks swapping Austenite in for the Typst-wasm compiler. This memo
//! caches the two costly, content-addressed stages so an edit recomputes only what actually changed:
//!
//! * The **block authoring memo** ([`Memo::blocks`]) keys each top-level document block on its content
//!   and the counter state it enters under (heading, footnote, figure numbers; the glossary first-use
//!   set), so an unedited block splices its previously authored nodes back rather than re-shaping and
//!   re-breaking its paragraphs. It is ledger-independent: a forward reference is a `Reserved` leaf
//!   that resolves later in the pass loop, so the authored nodes never embed a page number.
//! * The **page emit memo** ([`Memo::pages`]) keys each page on the content of its body frame -- the
//!   placed glyph runs, rules and graphics -- so an unedited page reuses its rendered SVG body and only
//!   the furniture (the running head and folio, which differ page to page) is drawn fresh.
//!
//! Both keys are 64-bit FNV-1a fingerprints of the content that determines the output. A fingerprint
//! can in principle collide; at 64 bits over a single document the probability is negligible, which is
//! the same footing every incremental compiler (Typst.ts included) rests a content cache on.
//!
//! **Residency and the memo's contract.** The memo holds authored nodes and rendered SVG, so it costs
//! roughly the memory of one document copy. A two-generation sweep ([`Memo::sweep`]) drops any entry not
//! touched in the last two compiles, bounding that to ~2x rather than growing without limit. The memo is
//! valid only while the document's fonts, faces, page geometry and theme are unchanged -- those are
//! folded into a single global fingerprint the caller supplies once per compile ([`Memo::begin`]); a
//! change to any of them belongs in a fresh [`Memo`], not this one.

use crate::doc::{
	Heading,
	Segment,
};
use crate::ir::Node;
use crate::ledger::AnchorId;

use std::collections::HashMap;

/// A 64-bit FNV-1a accumulator. The whole memo keys on this: it is fast, allocation-free, and stable
/// run to run, which is all a content fingerprint needs. It is not a cryptographic hash and makes no
/// claim to be one.
#[derive(Clone, Copy)]
pub struct Fnv {
	state:	u64,
}

impl Fnv {
	const OFFSET:	u64	= 0xcbf29ce484222325;
	const PRIME:	u64	= 0x00000100000001b3;

	pub fn new() -> Self {
		Self { state: Self::OFFSET }
	}

	pub fn write(&mut self, bytes: &[u8]) {
		for b in bytes {
			self.state ^= *b as u64;
			self.state = self.state.wrapping_mul(Self::PRIME);
		}
	}

	pub fn write_u8(&mut self, v: u8)	{ self.write(&[v]); }
	pub fn write_u32(&mut self, v: u32)	{ self.write(&v.to_le_bytes()); }
	pub fn write_u64(&mut self, v: u64)	{ self.write(&v.to_le_bytes()); }
	pub fn write_i32(&mut self, v: i32)	{ self.write(&v.to_le_bytes()); }
	pub fn write_usize(&mut self, v: usize)	{ self.write(&(v as u64).to_le_bytes()); }
	pub fn write_bool(&mut self, v: bool)	{ self.write_u8(v as u8); }

	/// Hashes an `f32` by its bit pattern, so a coordinate hashes exactly and reproducibly. Positions
	/// and advances are never `NaN`, so the several bit patterns of `NaN` are not a concern here.
	pub fn write_f32(&mut self, v: f32)	{ self.write(&v.to_bits().to_le_bytes()); }

	/// Hashes a string length-prefixed, so "ab"+"c" and "a"+"bc" do not collide.
	pub fn write_str(&mut self, s: &str) {
		self.write_u64(s.len() as u64);
		self.write(s.as_bytes());
	}

	pub fn finish(self) -> u64 { self.state }
}

impl Default for Fnv {
	fn default() -> Self { Self::new() }
}

/// The scalar authoring counters at a block boundary: the state a block enters under (its key) and the
/// state it leaves (its value, restored on a cache hit). These are exactly the counters a block bakes
/// into its output -- a heading's dotted number, a footnote's mark, a figure's caption number, the
/// anchor identity keyed off `heads_len` -- so two compiles that enter a block with the same state and
/// the same content produce byte-identical nodes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockState {
	pub first:			bool,
	pub prev_para:		bool,
	pub pending_banner:	bool,
	pub sec:			[u32; 6],
	pub part_no:		u32,
	pub foot_no:		u32,
	pub ref_no:			u32,
	pub margin_no:		u32,
	pub eq_no:			u32,
	pub fig_no:			u32,
	pub heads_len:		u32,
	pub index_no:		u32,
}

impl BlockState {
	/// Folds the entering state into a key hasher. `heads_len` and `index_no` are in here because a
	/// heading's and an index marker's anchor identity is keyed off them; a block whose position among
	/// the headings has shifted must not hit.
	pub fn hash_into(&self, h: &mut Fnv) {
		h.write_bool(self.first);
		h.write_bool(self.prev_para);
		h.write_bool(self.pending_banner);
		for s in &self.sec { h.write_u32(*s); }
		h.write_u32(self.part_no);
		h.write_u32(self.foot_no);
		h.write_u32(self.ref_no);
		h.write_u32(self.margin_no);
		h.write_u32(self.eq_no);
		h.write_u32(self.fig_no);
		h.write_u32(self.heads_len);
		h.write_u32(self.index_no);
	}
}

/// One cached block-authoring result: the nodes the block appended, the heading and index/claim
/// occurrences it recorded, how many source blocks it consumed (a chapter heading swallows the first
/// line of the paragraph it introduces, so two), and the counter state it left. The glossary terms it
/// marked first-seen and the supplement counters it stepped are stored as deltas, applied on a hit so
/// the shared accumulators advance exactly as a fresh authoring would advance them.
#[derive(Clone)]
pub struct BlockEntry {
	pub consume:		usize,
	pub nodes:			Vec<Node>,
	pub heads:			Vec<Heading>,
	pub index_occ:		Vec<(String, Option<String>, Vec<Segment>, AnchorId, bool)>,
	pub claim_occ:		Vec<(String, AnchorId)>,
	pub seen_add:		Vec<String>,
	pub counters_set:	Vec<(String, u32)>,
	pub exit:			BlockState,
	last_gen:			u64,	// the generation this entry was last touched, for the two-generation sweep
}

impl BlockEntry {
	#[allow(clippy::too_many_arguments)]
	pub fn new(
		consume:		usize,
		nodes:			Vec<Node>,
		heads:			Vec<Heading>,
		index_occ:		Vec<(String, Option<String>, Vec<Segment>, AnchorId, bool)>,
		claim_occ:		Vec<(String, AnchorId)>,
		seen_add:		Vec<String>,
		counters_set:	Vec<(String, u32)>,
		exit:			BlockState,
	)
		-> Self
	{
		Self { consume, nodes, heads, index_occ, claim_occ, seen_add, counters_set, exit, last_gen: 0 }
	}
}

/// One cached page-emit result: the SVG of the page's body frame, split into the visible glyph and
/// graphic ink and the invisible selectable text layer's tspans, plus whether the body left any text
/// behind (so the furniture's tspans take their leading interword space exactly as a single-pass render
/// would). The furniture -- running head and folio -- is never cached, since its folio differs page to
/// page; it is drawn fresh and concatenated onto the cached body, byte for byte as one pass produces.
#[derive(Clone)]
pub struct PageEntry {
	pub body_ink:		String,
	pub body_tspans:	String,
	pub seen_text:		bool,
	last_gen:			u64,
}

/// The document's incremental memo: the block-authoring and page-emit caches, the global fingerprint
/// that scopes both to one (fonts, faces, geometry, theme) configuration, the generation counter the
/// two-generation sweep reads, and the hit/miss tallies the gate measures.
#[derive(Default)]
pub struct Memo {
	blocks:		HashMap<u64, BlockEntry>,
	pages:		HashMap<u64, PageEntry>,
	global_fp:	u64,	// fonts+faces+geometry+theme+refs+bib fingerprint; folded into every key
	gen:		u64,
	pub block_hits:		u64,
	pub block_misses:	u64,
	pub page_hits:		u64,
	pub page_misses:	u64,
}

impl Memo {
	pub fn new() -> Self { Self::default() }

	/// Opens a compile generation: steps the generation counter (so this compile's touches are
	/// distinguishable from the last), resets the hit/miss tallies, and installs the configuration
	/// fingerprint. A fingerprint that differs from the one the cached entries were built under clears
	/// both caches, since every key was scoped to the old configuration.
	pub fn begin(&mut self, global_fp: u64) {
		if self.global_fp != global_fp && (!self.blocks.is_empty() || !self.pages.is_empty()) {
			self.blocks.clear();
			self.pages.clear();
		}
		self.global_fp	= global_fp;
		self.gen		= self.gen.wrapping_add(1);
		self.block_hits		= 0;
		self.block_misses	= 0;
		self.page_hits		= 0;
		self.page_misses	= 0;
	}

	pub fn global_fp(&self) -> u64 { self.global_fp }

	/// Drops every entry not touched in the current or the immediately preceding generation, so the two
	/// caches hold at most the working sets of the last two compiles -- roughly twice the live document,
	/// never an unbounded accumulation of stale edits.
	pub fn sweep(&mut self) {
		let gen = self.gen;
		self.blocks.retain(|_, e| gen.wrapping_sub(e.last_gen) < 2);
		self.pages.retain(|_, e| gen.wrapping_sub(e.last_gen) < 2);
	}

	// --- block authoring cache ---------------------------------------------------------------------

	/// Looks a block up, returning a clone of its cached result and marking it touched this generation.
	/// The clone releases the borrow so the caller can splice the nodes into the authoring state; a
	/// paragraph's node clone is far cheaper than re-shaping and re-breaking it.
	pub fn block_lookup(&mut self, key: u64) -> Option<BlockEntry> {
		let gen = self.gen;
		match self.blocks.get_mut(&key) {
			Some(e)	=> { e.last_gen = gen; self.block_hits += 1; Some(e.clone()) },
			None	=> { self.block_misses += 1; None },
		}
	}

	pub fn block_store(&mut self, key: u64, mut entry: BlockEntry) {
		entry.last_gen = self.gen;
		self.blocks.insert(key, entry);
	}

	// --- page emit cache ---------------------------------------------------------------------------

	pub fn page_lookup(&mut self, key: u64) -> Option<PageEntry> {
		let gen = self.gen;
		match self.pages.get_mut(&key) {
			Some(e)	=> { e.last_gen = gen; self.page_hits += 1; Some(e.clone()) },
			None	=> { self.page_misses += 1; None },
		}
	}

	pub fn page_store(&mut self, key: u64, body_ink: String, body_tspans: String, seen_text: bool) {
		self.pages.insert(key, PageEntry { body_ink, body_tspans, seen_text, last_gen: self.gen });
	}
}
