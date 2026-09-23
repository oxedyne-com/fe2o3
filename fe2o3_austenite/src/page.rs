//! The page and frame model.
//!
//! A page is a view onto a frame: the frame holds boxes placed at absolute positions, and the page
//! adds its geometry and its folio. The flat-memory property lives here. Pass A builds one frame,
//! hands the page to a writer, and drops it -- so the engine holds one window of frames plus the
//! ledger, never the document.

use crate::font::ShapedText;
use crate::ir::{
	Dims,
	Graphic,
	Sp,
};

use std::sync::Arc;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_geom::rect::AbsSize;

/// A page's physical geometry: its trim size and four margins. A book binds along one edge, so the
/// inside (binding) and outside (fore-edge) margins differ, and the two alternate between recto and
/// verso -- a mirror. The driver lays every page at the recto split (`content_left` = inside); a verso
/// page is the same frame shifted by [`mirror_shift`](Self::mirror_shift), which is why the geometry
/// keeps both margins rather than one left edge.
#[derive(Clone, Copy, Debug)]
pub struct PageGeometry {
	pub width:	Sp,
	pub height:	Sp,
	pub inside:	Sp,	// the binding-edge margin: the left on a recto, the right on a verso
	pub outside:	Sp,	// the fore-edge margin, opposite the binding
	pub top:	Sp,
	pub bottom:	Sp,
}

impl PageGeometry {
	/// A uniform margin on all four sides -- the demos' geometry, and single-file `ingot`.
	pub fn new(width: Sp, height: Sp, margin: Sp) -> Self {
		Self { width, height, inside: margin, outside: margin, top: margin, bottom: margin }
	}

	/// A book geometry with mirror margins: `inside` binds, `outside` is the fore-edge.
	pub fn with_margins(width: Sp, height: Sp, inside: Sp, outside: Sp, top: Sp, bottom: Sp) -> Self {
		Self { width, height, inside, outside, top, bottom }
	}

	/// A4 portrait, 595.276 by 841.890 points, with a two-centimetre margin (56.9 points).
	pub fn a4() -> Self {
		Self::new(Sp::from_pt(595.276), Sp::from_pt(841.890), Sp::from_pt(56.9))
	}

	pub fn content_left(&self) -> Sp { self.inside }

	pub fn content_top(&self) -> Sp { self.top }

	/// The width available to a line of text: the trim less both side margins.
	pub fn content_width(&self) -> Sp { self.width - self.inside - self.outside }

	/// The height available to a column of vertical material before the page is full.
	pub fn content_height(&self) -> Sp { self.height - self.top - self.bottom }

	/// The geometry of the `i`-th of `n` equal columns within this page's content block, adjacent columns
	/// parted by `gutter`. Its [`content_left`](Self::content_left) and [`content_width`](Self::content_width)
	/// are that column's; every other measurement -- the trim, the vertical margins, and so the mirror shift
	/// -- is the page's own unchanged, so a caller sets a column's material with the ordinary placement
	/// helpers at a recto x, and the single verso mirror still applies once, to the whole frame, afterwards.
	/// The column width is the content width less the `n - 1` gutters, divided `n` ways in the integer
	/// domain; any one-scaled-point remainder from that division falls to the fore-edge margin, so every
	/// column is the same width and the split stays byte-identical run to run.
	pub fn column_slice(&self, i: usize, n: usize, gutter: Sp) -> PageGeometry {
		let n		= n.max(1);
		let i		= i.min(n - 1);
		let inner	= self.content_width() - gutter * (n as i32 - 1);	// width left for the columns themselves
		let col_w	= Sp(inner.raw() / n as i32);
		let col_left	= self.content_left() + (col_w + gutter) * i as i32;
		Self {
			width:		self.width,
			height:		self.height,
			inside:		col_left,
			outside:	self.width - col_left - col_w,
			top:		self.top,
			bottom:		self.bottom,
		}
	}

	/// The horizontal shift that turns the recto frame the driver laid into a verso one: the content
	/// block moves from `inside` to `outside` on the left, so the binding margin stays at the spine.
	/// Zero when the margins are uniform, so a non-book page never moves.
	pub fn mirror_shift(&self) -> Sp { self.outside - self.inside }

	/// The page extent in whole device points, for an SVG viewport. A viewport extent is non-negative
	/// device-space, which is what `fe2o3_geom`'s unsigned `Dim` models; rounding to whole points is
	/// harmless here.
	pub fn media_box(&self) -> AbsSize {
		let w = self.width.to_pt().round() as usize;
		let h = self.height.to_pt().round() as usize;
		AbsSize::from((w, h))
	}
}

/// What a placed box draws. A `Reserved` is a forward reference's held-open space, outlined faintly so
/// a proof shows where a value will land.
#[derive(Clone, Debug)]
pub enum PlacedKind {
	Rule,
	Reserved,
	Text(ShapedText),
	Graphic(Arc<Graphic>),	// a figure's baked paths, drawn at this box's position
}

/// Which of a page's three vertical regions a placed item belongs to. A float insertion reflows one region
/// by moving the items that belong to it, so membership -- not a y-coordinate window -- decides what moves.
/// A body line whose glyphs were raised above the body band's top edge by cap-height seating (see
/// `linebreak::raise_leaves`) still belongs to the body, and moves down with it when a top float is inserted
/// above; keying the shift on the raised glyph y instead would leave that line drawn under the float.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Region {
	Top,	// a top float's band, stacked from the page top down
	#[default]
	Body,	// the flowing column between the two float bands
	Foot,	// a foot float's band (footnotes are laid here too, once the page closes and nothing more shifts)
}

/// A box set at an absolute position on a page. The position is the top-left of the box; the
/// baseline sits `dims.height` below it.
#[derive(Clone, Debug)]
pub struct Placed {
	pub x:		Sp,
	pub y:		Sp,
	pub dims:	Dims,
	pub kind:	PlacedKind,
	pub region:	Region,	// which page region it flows in; decides float-relayout membership
}

impl Placed {
	/// A placed item defaults to the body region. A float's own material is placed through the same
	/// helpers and then reclaimed for its band with [`Frame::stamp_region`].
	pub fn new(x: Sp, y: Sp, dims: Dims, kind: PlacedKind) -> Self {
		Self { x, y, dims, kind, region: Region::Body }
	}
}

/// The placed material of one page. Built in Pass A, written, then dropped.
#[derive(Clone, Debug, Default)]
pub struct Frame {
	pub placed:	Vec<Placed>,
}

impl Frame {
	pub fn new() -> Self {
		Self { placed: Vec::new() }
	}

	pub fn push(&mut self, item: Placed) {
		self.placed.push(item);
	}

	pub fn is_empty(&self) -> bool {
		self.placed.is_empty()
	}

	pub fn len(&self) -> usize {
		self.placed.len()
	}

	/// Translates every placed item belonging to `region` by `by` (down for a positive `by`, up for a
	/// negative one). This is how a float inserted into a part-filled page makes room without disturbing the
	/// other regions: a top float shifts the body region down, a foot float shifts the existing foot region
	/// up, and the bands not being reflowed stay put. It mirrors Typst's relayout, which re-flows the whole
	/// region when a float is inserted. Membership, not a y window, is the test, so a body line raised above
	/// the band edge by cap-height seating still moves with its body (see [`Region`]).
	pub fn shift_region(&mut self, by: Sp, region: Region) {
		for item in &mut self.placed {
			if item.region == region {
				item.y = item.y + by;
			}
		}
	}

	/// As [`Frame::shift_region`], for the items from index `from` on only: a column float shifts the material
	/// of its own column -- everything placed since the column opened -- and not the columns beside it.
	pub fn shift_region_from(&mut self, from: usize, by: Sp, region: Region) {
		for item in self.placed.iter_mut().skip(from) {
			if item.region == region {
				item.y = item.y + by;
			}
		}
	}

	/// Reclaims every item from index `from` to the end for `region`. A float's material is placed through
	/// the ordinary helpers, which stamp it `Body`; the caller records the frame length before placing the
	/// float and calls this after, so exactly the float's own items join its band and the body items already
	/// on the page keep their membership.
	pub fn stamp_region(&mut self, from: usize, region: Region) {
		for item in &mut self.placed[from..] {
			item.region = region;
		}
	}
}

/// A page: its one-based folio, its geometry, and its frame.
#[derive(Clone, Debug)]
pub struct Page {
	pub number:	u32,
	pub geom:	PageGeometry,
	pub frame:	Frame,
	// The count of placed items that are body, recorded before `doc::decorate` appends the running head
	// and folio. The page-emit memo keys on `placed[..body_len]` and draws the furniture beyond it fresh,
	// so an unedited page reuses its body SVG while its folio still renders per page. `usize::MAX` means
	// the whole frame is body (nothing decorated it), which is what every non-memo path leaves it at.
	body_len:	usize,
}

impl Page {
	pub fn new(number: u32, geom: PageGeometry, frame: Frame) -> Self {
		Self { number, geom, frame, body_len: usize::MAX }
	}

	/// Records the body/furniture split point: the placed count at the moment before decoration. Called
	/// once, by the memo-threading compile path, just before `doc::decorate` stamps the furniture on.
	pub fn set_body_len(&mut self, n: usize) { self.body_len = n; }

	/// The split point clamped to the current frame, so `placed[..body_len()]` is always in bounds even
	/// after the verso mirror shift has moved items around (it never adds or removes any).
	pub fn body_len(&self) -> usize { self.body_len.min(self.frame.placed.len()) }
}
