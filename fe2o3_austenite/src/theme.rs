//! `Theme`: a document's styling held as data rather than scattered constants.
//!
//! Austenite began with a flat `Copy` `Style` struct filled by string-scraping a template's config.
//! This module replaces it with a grouped, serialisable `Theme`: the same values today's renderer
//! reads, arranged by the element they style (text, paragraphs, headings, lists, tables, furniture,
//! colours) with reserved groups (figure, code, callout, equation, per-part page geometry) that a
//! later unit fills from the document's own `#set`/`#show` declarations.
//!
//! **Canonical serialisation.** [`Theme::to_dat`] emits an ordered map whose key order is fixed and
//! documented here, and every sub-struct does the same in field-declaration order. The order is a
//! stable contract: a later unit hashes a theme's `to_dat` bytes as one input to a block address, so
//! the ordering must not change without changing that address deliberately. The canonical group order
//! is: `text, par, heading, list, enumeration, table, figure, code, callout, equation, page,
//! furniture, colours, calibration`. This unit computes no address; it only fixes the ordering.
//!
//! **Defaults reproduce the old `Style` exactly**, so a corpus rendered with a default (or
//! config-built) `Theme` is byte-identical to the pre-`Theme` build. The gate on the unit that landed
//! this file is exactly that identity.

use crate::doc::HeadingStyle;
use crate::ir::Sp;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_graphics::colour::Rgba;

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE THEME                                                                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// A document's styling as data. Grouped by the element each value styles; see the module header for
/// the canonical serialisation order the block-address hash later depends on.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
	pub text:			ThemeText,
	pub par:			ThemePar,
	pub heading:		ThemeHeading,
	pub list:			ThemeList,
	pub enumeration:	ThemeEnum,
	pub table:			ThemeTable,
	pub figure:			ThemeFigure,
	pub code:			ThemeCode,
	pub callout:		ThemeCallout,
	pub equation:		ThemeEquation,
	pub page:			ThemePage,
	pub furniture:		ThemeFurniture,
	pub colours:		ThemeColours,
	pub calibration:	ThemeCalibration,
}

impl Default for Theme {
	fn default() -> Self {
		Self {
			text:			ThemeText::default(),
			par:			ThemePar::default(),
			heading:		ThemeHeading::default(),
			list:			ThemeList::default(),
			enumeration:	ThemeEnum::default(),
			table:			ThemeTable::default(),
			figure:			ThemeFigure::default(),
			code:			ThemeCode::default(),
			callout:		ThemeCallout::default(),
			equation:		ThemeEquation::default(),
			page:			ThemePage::default(),
			furniture:		ThemeFurniture::default(),
			colours:		ThemeColours::default(),
			calibration:	ThemeCalibration::default(),
		}
	}
}

impl Theme {
	/// The type size a heading of this level is set at. Level 0 (a part divider) takes the chapter-title
	/// size, and any level past 4 takes the level-4 size.
	pub fn heading_size(&self, level: u8) -> Sp {
		match level {
			0 | 1	=> self.heading.levels[0].size,
			2		=> self.heading.levels[1].size,
			3		=> self.heading.levels[2].size,
			_		=> self.heading.levels[3].size,
		}
	}

	/// The space set above a heading of this level, always greater than the space below it, so a
	/// heading binds visually to the text it introduces rather than to the text it follows. Levels
	/// past 2 (and a level-0 part divider) share the level-3 spacing.
	pub fn space_above(&self, level: u8) -> Sp {
		match level {
			1	=> self.heading.levels[0].space_above,
			2	=> self.heading.levels[1].space_above,
			_	=> self.heading.levels[2].space_above,
		}
	}

	/// The space set below a heading of this level. See [`Theme::space_above`] for the level mapping.
	pub fn space_below(&self, level: u8) -> Sp {
		match level {
			1	=> self.heading.levels[0].space_below,
			2	=> self.heading.levels[1].space_below,
			_	=> self.heading.levels[2].space_below,
		}
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ GROUPS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

/// Body text: the size and baseline the flow sets, and reserved run-level switches a later unit lowers
/// from `set text(...)`.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeText {
	pub body_size:	Sp,
	pub leading:	Sp,		// baseline-to-baseline distance, not a gap; see the calibration group
	pub tracking:	Sp,		// reserved: extra letter spacing, unread until a later unit lowers it
	pub ligatures:	bool,	// reserved
	pub hyphenate:	bool,	// reserved
	pub justify:	bool,	// reserved
	pub faces:		FaceSet,	// reserved: role -> family name, resolved to a loaded face at render
}

impl Default for ThemeText {
	fn default() -> Self {
		Self {
			body_size:	Sp::from_pt(11.0),
			leading:	Sp::from_pt(13.2),	// 1.2x the body
			tracking:	Sp::ZERO,
			ligatures:	true,
			hyphenate:	true,
			justify:	true,
			faces:		FaceSet::default(),
		}
	}
}

/// Reserved: the family name each text role is set in, or `None` to take the loaded default. A later
/// unit lowers `set text(font: ...)` and `show <role>: set text(...)` into these; the renderer does
/// not read them yet.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FaceSet {
	pub body:		Option<String>,
	pub emphasis:	Option<String>,
	pub heading:	Option<String>,
	pub mono:		Option<String>,
}

/// Paragraph shape: the space between one paragraph and the next, and the first-line indent a
/// paragraph following another takes.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemePar {
	pub skip:	Sp,	// extra space between one paragraph and the next
	pub indent:	Sp,	// first-line indent of a paragraph following another paragraph
}

impl Default for ThemePar {
	fn default() -> Self {
		Self {
			skip:	Sp::from_pt(6.0),
			indent:	Sp::ZERO,	// no first-line indent unless a config sets one
		}
	}
}

/// One heading level's styling. Only `size`, `space_above` and `space_below` are read today; the
/// face, weight, italic, smallcaps and numbering fields are reserved for the unit that lowers a
/// document's `set heading(numbering: ...)` and per-level `show` rules.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeHeadingLevel {
	pub size:			Sp,
	pub face:			Option<String>,	// reserved
	pub weight:			Option<u16>,	// reserved: font weight, 100-900
	pub italic:			bool,			// reserved
	pub smallcaps:		bool,			// reserved
	pub numbering:		Option<String>,	// reserved: a Typst numbering pattern, e.g. "1.1"
	pub space_above:	Sp,
	pub space_below:	Sp,
}

impl ThemeHeadingLevel {
	fn new(size_pt: f64, above_pt: f64, below_pt: f64) -> Self {
		Self {
			size:			Sp::from_pt(size_pt),
			face:			None,
			weight:			None,
			italic:			false,
			smallcaps:		false,
			numbering:		None,
			space_above:	Sp::from_pt(above_pt),
			space_below:	Sp::from_pt(below_pt),
		}
	}
}

/// Headings: which opener and numbering the top level takes, the per-level styling indexed 0..4 for
/// levels 1..4, and the chapter opener's giant number size and grid.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeHeading {
	pub kind:			HeadingStyle,			// which top-level opener and numbering the headings take
	pub levels:			[ThemeHeadingLevel; 4],	// index 0 is level 1, index 3 is level 4
	pub chap_num_size:	Sp,						// the giant chapter number on a chapter-opening page
	pub chap_grid:		[Sp; 4],				// opener grid rows: number band, gap, title band, gap-to-body
}

impl Default for ThemeHeading {
	fn default() -> Self {
		Self {
			kind:	HeadingStyle::BookOpener,
			levels:	[
				ThemeHeadingLevel::new(16.0, 20.0, 8.0),	// level 1, the chapter title
				ThemeHeadingLevel::new(13.0, 15.0, 6.0),	// level 2
				ThemeHeadingLevel::new(12.0, 12.0, 5.0),	// level 3
				ThemeHeadingLevel::new(11.0, 12.0, 5.0),	// level 4
			],
			chap_num_size:	Sp::from_pt(54.0),
			chap_grid:		[Sp::from_pt(72.0), Sp::from_pt(8.0), Sp::from_pt(36.0), Sp::from_pt(20.0)],
		}
	}
}

/// Bulleted lists: the gap after a marker and the space between one item and the next.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeList {
	pub marker_gap:	Sp,	// space between a list marker and the item text it introduces
	pub item_skip:	Sp,	// vertical space set between one list item and the next
}

impl Default for ThemeList {
	fn default() -> Self {
		Self {
			marker_gap:	Sp::from_pt(6.0),
			item_skip:	Sp::from_pt(3.0),
		}
	}
}

/// Numbered lists. Reserved: today the enumeration is set with the same metrics as a bulleted list;
/// a later unit lowers `set enum(...)` into its own marker pattern here.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeEnum {
	pub marker_gap:	Sp,
	pub item_skip:	Sp,
	pub numbering:	Option<String>,	// reserved: a Typst numbering pattern, e.g. "1."
}

impl Default for ThemeEnum {
	fn default() -> Self {
		Self {
			marker_gap:	Sp::from_pt(6.0),
			item_skip:	Sp::from_pt(3.0),
			numbering:	None,
		}
	}
}

/// Tables: the space around a table, the cell padding, the inter-line leading within a cell, and the
/// two rule weights.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeTable {
	pub skip:		Sp,	// space set above and below a table
	pub cell_pad_x:	Sp,	// horizontal padding between a cell's text and its column rules
	pub cell_pad_y:	Sp,	// vertical padding above and below a cell's lines
	pub line_gap:	Sp,	// leading between the wrapped lines within one cell
	pub rule_thin:	Sp,	// an interior grid rule (also the maths fraction bar)
	pub rule_thick:	Sp,	// the frame and the rule beneath a header
}

impl Default for ThemeTable {
	fn default() -> Self {
		Self {
			skip:		Sp::from_pt(10.0),
			cell_pad_x:	Sp::from_pt(5.0),
			cell_pad_y:	Sp::from_pt(3.0),
			line_gap:	Sp::from_pt(3.0),
			rule_thin:	Sp::from_pt(0.4),
			rule_thick:	Sp::from_pt(0.8),
		}
	}
}

/// Figures and their captions. Reserved: the renderer sizes a caption inline today; a later unit
/// lowers `show figure.caption: set text(...)` into `caption_size`.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeFigure {
	pub caption_size:	Sp,	// reserved
	pub skip:			Sp,	// reserved: space around a figure
}

impl Default for ThemeFigure {
	fn default() -> Self {
		Self {
			caption_size:	Sp::from_pt(9.9),	// reserved placeholder, 0.9em of an 11pt body
			skip:			Sp::from_pt(10.0),
		}
	}
}

/// Code blocks. Reserved for a later unit that lowers a document's raw-block styling.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeCode {
	pub size:		Sp,		// reserved
	pub background:	Rgba,	// reserved
}

impl Default for ThemeCode {
	fn default() -> Self {
		Self {
			size:		Sp::from_pt(9.9),
			background:	Rgba::opaque(245, 245, 245),
		}
	}
}

/// Callout boxes. Reserved: the block layer washes a callout with a fixed lilac today; a later unit
/// lowers a document's own callout styling into `fill`.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeCallout {
	pub fill:	Rgba,	// reserved
}

impl Default for ThemeCallout {
	fn default() -> Self {
		Self { fill: Rgba::opaque(245, 230, 255) }
	}
}

/// Equations. Reserved for the unit that lowers `set math.equation(numbering: ...)`.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeEquation {
	pub numbering:	Option<String>,	// reserved
}

impl Default for ThemeEquation {
	fn default() -> Self {
		Self { numbering: None }
	}
}

/// A per-part page-geometry override. Every field is `None` today, meaning the part takes the
/// document-level geometry passed to the driver; a later unit lowers a document's sequential top-level
/// `set page(...)` calls into per-part overrides here.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ThemePagePart {
	pub width:			Option<Sp>,
	pub height:			Option<Sp>,
	pub margin_inside:	Option<Sp>,
	pub margin_outside:	Option<Sp>,
	pub margin_top:		Option<Sp>,
	pub margin_bottom:	Option<Sp>,
}

/// Page geometry per part of the book. Reserved; see [`ThemePagePart`].
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ThemePage {
	pub front:	ThemePagePart,
	pub body:	ThemePagePart,
	pub back:	ThemePagePart,
}

/// Page furniture: the running head, the folio, and the footnote text metrics.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeFurniture {
	pub header_size:	Sp,	// the running head's size
	pub folio_size:		Sp,
	pub foot_size:		Sp,	// the footnote text's size, a touch below the body
	pub foot_leading:	Sp,	// leading between the wrapped lines of one footnote
}

impl Default for ThemeFurniture {
	fn default() -> Self {
		Self {
			header_size:	Sp::from_pt(9.5),
			folio_size:		Sp::from_pt(10.0),
			foot_size:		Sp::from_pt(9.0),
			foot_leading:	Sp::from_pt(10.8),	// 1.2x the footnote size
		}
	}
}

/// The theme's named colours.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeColours {
	pub chap_num_grey:	Rgba,	// the fill of the giant chapter number, a light grey
	pub header_fill:	Rgba,	// the wash behind a table header row
}

impl Default for ThemeColours {
	fn default() -> Self {
		Self {
			chap_num_grey:	Rgba::opaque(200, 200, 200),	// Typst's luma(200)
			header_fill:	Rgba::opaque(235, 238, 241),	// #E9ECEF lightened 10%, the template's header1
		}
	}
}

/// Calibration constants the flow is measured against. `line_box_em` is the Libertinus line box Typst
/// sets, as a fraction of the em: Typst's config leading is the gap added between line boxes, and the
/// baseline-to-baseline skip is that gap plus the box. The box is not the face's nominal ascender +
/// descender (fe2o3_font reports ~1.14 em for Libertinus, ~30% too loose); Typst's rendered Libertinus
/// line box measures ~0.66 em. For an 11pt body at 0.78em leading that gives (0.66 + 0.78) x 11 = 15.84
/// pt baseline-to-baseline, matching the Lucronics oracle measured at 300 DPI (66 px). An earlier 0.682
/// set the pitch 0.24 pt too loose, losing ~1 line per page and driving a whole-book pagination drift.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeCalibration {
	pub line_box_em:	f64,
}

impl Default for ThemeCalibration {
	fn default() -> Self {
		Self { line_box_em: 0.660 }
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THEME PATCH                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// A sparse overlay on a [`Theme`]: the same fourteen groups in the same order, every leaf an `Option`
/// of the theme's own field type, so `None` leaves a field as it stands and `Some(v)` sets it to `v`.
/// Lowering a document's `#set`/`#show` declarations ([`crate::lang::set`]) builds one of these rather
/// than mutating a theme in place, and [`Theme::apply`] folds it onto a theme at the scope it governs --
/// the document, an included chapter, or a `#styled-box` body. A leaf whose theme type is itself optional
/// (a face name, a numbering pattern) takes an `Option<Option<..>>`, so a patch can set it, clear it, or
/// leave it, keeping the overlay fully general for the rule engine that reuses this machinery to apply a
/// patch to a selected subtree.
///
/// The field declaration order mirrors [`Theme`]'s canonical serialisation order (see the module header),
/// so a future patch serialisation lines up with a theme's without a second ordering contract to keep.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemePatch {
	pub text:			ThemeTextPatch,
	pub par:			ThemeParPatch,
	pub heading:		ThemeHeadingPatch,
	pub list:			ThemeListPatch,
	pub enumeration:	ThemeEnumPatch,
	pub table:			ThemeTablePatch,
	pub figure:			ThemeFigurePatch,
	pub code:			ThemeCodePatch,
	pub callout:		ThemeCalloutPatch,
	pub equation:		ThemeEquationPatch,
	pub page:			ThemePagePatch,
	pub furniture:		ThemeFurniturePatch,
	pub colours:		ThemeColoursPatch,
	pub calibration:	ThemeCalibrationPatch,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaceSetPatch {
	pub body:		Option<Option<String>>,
	pub emphasis:	Option<Option<String>>,
	pub heading:	Option<Option<String>>,
	pub mono:		Option<Option<String>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeTextPatch {
	pub body_size:	Option<Sp>,
	pub leading:	Option<Sp>,
	pub tracking:	Option<Sp>,
	pub ligatures:	Option<bool>,
	pub hyphenate:	Option<bool>,
	pub justify:	Option<bool>,
	pub faces:		FaceSetPatch,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeParPatch {
	pub skip:	Option<Sp>,
	pub indent:	Option<Sp>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeHeadingLevelPatch {
	pub size:			Option<Sp>,
	pub face:			Option<Option<String>>,
	pub weight:			Option<Option<u16>>,
	pub italic:			Option<bool>,
	pub smallcaps:		Option<bool>,
	pub numbering:		Option<Option<String>>,
	pub space_above:	Option<Sp>,
	pub space_below:	Option<Sp>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeHeadingPatch {
	pub kind:			Option<HeadingStyle>,
	pub levels:			[ThemeHeadingLevelPatch; 4],
	pub chap_num_size:	Option<Sp>,
	pub chap_grid:		Option<[Sp; 4]>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeListPatch {
	pub marker_gap:	Option<Sp>,
	pub item_skip:	Option<Sp>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeEnumPatch {
	pub marker_gap:	Option<Sp>,
	pub item_skip:	Option<Sp>,
	pub numbering:	Option<Option<String>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeTablePatch {
	pub skip:		Option<Sp>,
	pub cell_pad_x:	Option<Sp>,
	pub cell_pad_y:	Option<Sp>,
	pub line_gap:	Option<Sp>,
	pub rule_thin:	Option<Sp>,
	pub rule_thick:	Option<Sp>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeFigurePatch {
	pub caption_size:	Option<Sp>,
	pub skip:			Option<Sp>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeCodePatch {
	pub size:		Option<Sp>,
	pub background:	Option<Rgba>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeCalloutPatch {
	pub fill:	Option<Rgba>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeEquationPatch {
	pub numbering:	Option<Option<String>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemePagePartPatch {
	pub width:			Option<Option<Sp>>,
	pub height:			Option<Option<Sp>>,
	pub margin_inside:	Option<Option<Sp>>,
	pub margin_outside:	Option<Option<Sp>>,
	pub margin_top:		Option<Option<Sp>>,
	pub margin_bottom:	Option<Option<Sp>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemePagePatch {
	pub front:	ThemePagePartPatch,
	pub body:	ThemePagePartPatch,
	pub back:	ThemePagePartPatch,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeFurniturePatch {
	pub header_size:	Option<Sp>,
	pub folio_size:		Option<Sp>,
	pub foot_size:		Option<Sp>,
	pub foot_leading:	Option<Sp>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeColoursPatch {
	pub chap_num_grey:	Option<Rgba>,
	pub header_fill:	Option<Rgba>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeCalibrationPatch {
	pub line_box_em:	Option<f64>,
}

// Folds one patch leaf onto its theme field: a `Some` overwrites, a `None` leaves the field as it stands.
// The clone covers the non-`Copy` leaves (a face name, a numbering pattern); a `Copy` leaf clones for free.
macro_rules! patch_merge {
	($from:expr, $to:expr) => {
		if let Some(v) = &$from {
			$to = v.clone();
		}
	};
}

impl Theme {
	/// Folds `patch` onto this theme in place: every field the patch names is overwritten, every field it
	/// leaves `None` stands. Applying an empty patch is a no-op, so a scope that declares nothing changes
	/// nothing -- which is what keeps a document with no `#set` byte-identical.
	pub fn apply(&mut self, patch: &ThemePatch) {
		patch.text.apply(&mut self.text);
		patch.par.apply(&mut self.par);
		patch.heading.apply(&mut self.heading);
		patch.list.apply(&mut self.list);
		patch.enumeration.apply(&mut self.enumeration);
		patch.table.apply(&mut self.table);
		patch.figure.apply(&mut self.figure);
		patch.code.apply(&mut self.code);
		patch.callout.apply(&mut self.callout);
		patch.equation.apply(&mut self.equation);
		patch.page.apply(&mut self.page);
		patch.furniture.apply(&mut self.furniture);
		patch.colours.apply(&mut self.colours);
		patch.calibration.apply(&mut self.calibration);
	}
}

impl FaceSetPatch {
	fn apply(&self, t: &mut FaceSet) {
		patch_merge!(self.body, t.body);
		patch_merge!(self.emphasis, t.emphasis);
		patch_merge!(self.heading, t.heading);
		patch_merge!(self.mono, t.mono);
	}
}

impl ThemeTextPatch {
	fn apply(&self, t: &mut ThemeText) {
		patch_merge!(self.body_size, t.body_size);
		patch_merge!(self.leading, t.leading);
		patch_merge!(self.tracking, t.tracking);
		patch_merge!(self.ligatures, t.ligatures);
		patch_merge!(self.hyphenate, t.hyphenate);
		patch_merge!(self.justify, t.justify);
		self.faces.apply(&mut t.faces);
	}
}

impl ThemeParPatch {
	fn apply(&self, t: &mut ThemePar) {
		patch_merge!(self.skip, t.skip);
		patch_merge!(self.indent, t.indent);
	}
}

impl ThemeHeadingLevelPatch {
	fn apply(&self, l: &mut ThemeHeadingLevel) {
		patch_merge!(self.size, l.size);
		patch_merge!(self.face, l.face);
		patch_merge!(self.weight, l.weight);
		patch_merge!(self.italic, l.italic);
		patch_merge!(self.smallcaps, l.smallcaps);
		patch_merge!(self.numbering, l.numbering);
		patch_merge!(self.space_above, l.space_above);
		patch_merge!(self.space_below, l.space_below);
	}
}

impl ThemeHeadingPatch {
	fn apply(&self, h: &mut ThemeHeading) {
		patch_merge!(self.kind, h.kind);
		for (p, l) in self.levels.iter().zip(h.levels.iter_mut()) {
			p.apply(l);
		}
		patch_merge!(self.chap_num_size, h.chap_num_size);
		patch_merge!(self.chap_grid, h.chap_grid);
	}
}

impl ThemeListPatch {
	fn apply(&self, t: &mut ThemeList) {
		patch_merge!(self.marker_gap, t.marker_gap);
		patch_merge!(self.item_skip, t.item_skip);
	}
}

impl ThemeEnumPatch {
	fn apply(&self, t: &mut ThemeEnum) {
		patch_merge!(self.marker_gap, t.marker_gap);
		patch_merge!(self.item_skip, t.item_skip);
		patch_merge!(self.numbering, t.numbering);
	}
}

impl ThemeTablePatch {
	fn apply(&self, t: &mut ThemeTable) {
		patch_merge!(self.skip, t.skip);
		patch_merge!(self.cell_pad_x, t.cell_pad_x);
		patch_merge!(self.cell_pad_y, t.cell_pad_y);
		patch_merge!(self.line_gap, t.line_gap);
		patch_merge!(self.rule_thin, t.rule_thin);
		patch_merge!(self.rule_thick, t.rule_thick);
	}
}

impl ThemeFigurePatch {
	fn apply(&self, t: &mut ThemeFigure) {
		patch_merge!(self.caption_size, t.caption_size);
		patch_merge!(self.skip, t.skip);
	}
}

impl ThemeCodePatch {
	fn apply(&self, t: &mut ThemeCode) {
		patch_merge!(self.size, t.size);
		patch_merge!(self.background, t.background);
	}
}

impl ThemeCalloutPatch {
	fn apply(&self, t: &mut ThemeCallout) {
		patch_merge!(self.fill, t.fill);
	}
}

impl ThemeEquationPatch {
	fn apply(&self, t: &mut ThemeEquation) {
		patch_merge!(self.numbering, t.numbering);
	}
}

impl ThemePagePartPatch {
	fn apply(&self, t: &mut ThemePagePart) {
		patch_merge!(self.width, t.width);
		patch_merge!(self.height, t.height);
		patch_merge!(self.margin_inside, t.margin_inside);
		patch_merge!(self.margin_outside, t.margin_outside);
		patch_merge!(self.margin_top, t.margin_top);
		patch_merge!(self.margin_bottom, t.margin_bottom);
	}
}

impl ThemePagePatch {
	fn apply(&self, t: &mut ThemePage) {
		self.front.apply(&mut t.front);
		self.body.apply(&mut t.body);
		self.back.apply(&mut t.back);
	}
}

impl ThemeFurniturePatch {
	fn apply(&self, t: &mut ThemeFurniture) {
		patch_merge!(self.header_size, t.header_size);
		patch_merge!(self.folio_size, t.folio_size);
		patch_merge!(self.foot_size, t.foot_size);
		patch_merge!(self.foot_leading, t.foot_leading);
	}
}

impl ThemeColoursPatch {
	fn apply(&self, t: &mut ThemeColours) {
		patch_merge!(self.chap_num_grey, t.chap_num_grey);
		patch_merge!(self.header_fill, t.header_fill);
	}
}

impl ThemeCalibrationPatch {
	fn apply(&self, t: &mut ThemeCalibration) {
		patch_merge!(self.line_box_em, t.line_box_em);
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ SERIALISATION HELPERS                                                      │
// └───────────────────────────────────────────────────────────────────────────┘

fn sp_dat(s: Sp) -> Dat { dat!(s.raw()) }

fn sp_from(d: Dat) -> Outcome<Sp> {
	Ok(Sp(try_extract_dat!(d, I32)))
}

fn opt_sp_dat(o: Option<Sp>) -> Dat {
	Dat::Opt(Box::new(o.map(sp_dat)))
}

fn opt_sp_from(d: Dat) -> Outcome<Option<Sp>> {
	match d {
		Dat::Opt(b)	=> match *b {
			Some(inner)	=> Ok(Some(res!(sp_from(inner)))),
			None		=> Ok(None),
		},
		other	=> Err(err!("A theme optional length must be a jdat Opt, found a {:?}.", other.kind(); Input, Invalid)),
	}
}

fn opt_str_dat(o: &Option<String>) -> Dat {
	Dat::Opt(Box::new(o.as_ref().map(|s| dat!(s.clone()))))
}

fn opt_str_from(d: Dat) -> Outcome<Option<String>> {
	match d {
		Dat::Opt(b)	=> match *b {
			Some(inner)	=> Ok(Some(try_extract_dat!(inner, Str))),
			None		=> Ok(None),
		},
		other	=> Err(err!("A theme optional string must be a jdat Opt, found a {:?}.", other.kind(); Input, Invalid)),
	}
}

fn opt_u16_dat(o: Option<u16>) -> Dat {
	Dat::Opt(Box::new(o.map(|v| dat!(v))))
}

fn opt_u16_from(d: Dat) -> Outcome<Option<u16>> {
	match d {
		Dat::Opt(b)	=> match *b {
			Some(inner)	=> Ok(Some(try_extract_dat!(inner, U16))),
			None		=> Ok(None),
		},
		other	=> Err(err!("A theme optional weight must be a jdat Opt, found a {:?}.", other.kind(); Input, Invalid)),
	}
}

fn rgba_dat(c: Rgba) -> Dat {
	omapdat!{
		"r"	=> dat!(c.r),
		"g"	=> dat!(c.g),
		"b"	=> dat!(c.b),
		"a"	=> dat!(c.a),
	}
}

fn rgba_from(mut d: Dat) -> Outcome<Rgba> {
	let r	= try_extract_dat!(res!(d.map_remove_must(&dat!("r"))), U8);
	let g	= try_extract_dat!(res!(d.map_remove_must(&dat!("g"))), U8);
	let b	= try_extract_dat!(res!(d.map_remove_must(&dat!("b"))), U8);
	let a	= try_extract_dat!(res!(d.map_remove_must(&dat!("a"))), U8);
	Ok(Rgba::new(r, g, b, a))
}

fn sp4_dat(a: &[Sp; 4]) -> Dat {
	Dat::List(a.iter().map(|s| sp_dat(*s)).collect())
}

fn sp4_from(d: Dat) -> Outcome<[Sp; 4]> {
	let list = try_extract_dat!(d, List);
	if list.len() != 4 {
		return Err(err!("A theme length quartet must hold 4 entries, found {}.", list.len(); Input, Invalid));
	}
	let mut it	= list.into_iter();
	let a0		= res!(sp_from(res!(it.next().ok_or_else(|| err!("missing quartet entry 0"; Input, Missing)))));
	let a1		= res!(sp_from(res!(it.next().ok_or_else(|| err!("missing quartet entry 1"; Input, Missing)))));
	let a2		= res!(sp_from(res!(it.next().ok_or_else(|| err!("missing quartet entry 2"; Input, Missing)))));
	let a3		= res!(sp_from(res!(it.next().ok_or_else(|| err!("missing quartet entry 3"; Input, Missing)))));
	Ok([a0, a1, a2, a3])
}

fn heading_kind_dat(k: HeadingStyle) -> Dat {
	let s = match k {
		HeadingStyle::BookOpener	=> "book-opener",
		HeadingStyle::DocBanner		=> "doc-banner",
		HeadingStyle::DocInline		=> "doc-inline",
	};
	dat!(s.to_string())
}

fn heading_kind_from(d: Dat) -> Outcome<HeadingStyle> {
	let s = try_extract_dat!(d, Str);
	match s.as_str() {
		"book-opener"	=> Ok(HeadingStyle::BookOpener),
		"doc-banner"	=> Ok(HeadingStyle::DocBanner),
		"doc-inline"	=> Ok(HeadingStyle::DocInline),
		other			=> Err(err!("A heading opener kind must be one of book-opener/doc-banner/doc-inline, found {:?}.", other; Input, Invalid)),
	}
}

fn map_must(d: &mut Dat, key: &str) -> Outcome<Dat> {
	d.map_remove_must(&dat!(key.to_string()))
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ SUB-STRUCT SERIALISATION                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

impl FaceSet {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(omapdat!{
			"body"		=> opt_str_dat(&self.body),
			"emphasis"	=> opt_str_dat(&self.emphasis),
			"heading"	=> opt_str_dat(&self.heading),
			"mono"		=> opt_str_dat(&self.mono),
		})
	}
	fn from_dat(mut d: Dat) -> Outcome<Self> {
		Ok(Self {
			body:		res!(opt_str_from(res!(map_must(&mut d, "body")))),
			emphasis:	res!(opt_str_from(res!(map_must(&mut d, "emphasis")))),
			heading:	res!(opt_str_from(res!(map_must(&mut d, "heading")))),
			mono:		res!(opt_str_from(res!(map_must(&mut d, "mono")))),
		})
	}
}

impl ThemeText {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(omapdat!{
			"body_size"	=> sp_dat(self.body_size),
			"leading"	=> sp_dat(self.leading),
			"tracking"	=> sp_dat(self.tracking),
			"ligatures"	=> dat!(self.ligatures),
			"hyphenate"	=> dat!(self.hyphenate),
			"justify"	=> dat!(self.justify),
			"faces"		=> res!(self.faces.to_dat()),
		})
	}
	fn from_dat(mut d: Dat) -> Outcome<Self> {
		Ok(Self {
			body_size:	res!(sp_from(res!(map_must(&mut d, "body_size")))),
			leading:	res!(sp_from(res!(map_must(&mut d, "leading")))),
			tracking:	res!(sp_from(res!(map_must(&mut d, "tracking")))),
			ligatures:	try_extract_dat!(res!(map_must(&mut d, "ligatures")), Bool),
			hyphenate:	try_extract_dat!(res!(map_must(&mut d, "hyphenate")), Bool),
			justify:	try_extract_dat!(res!(map_must(&mut d, "justify")), Bool),
			faces:		res!(FaceSet::from_dat(res!(map_must(&mut d, "faces")))),
		})
	}
}

impl ThemePar {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(omapdat!{
			"skip"		=> sp_dat(self.skip),
			"indent"	=> sp_dat(self.indent),
		})
	}
	fn from_dat(mut d: Dat) -> Outcome<Self> {
		Ok(Self {
			skip:	res!(sp_from(res!(map_must(&mut d, "skip")))),
			indent:	res!(sp_from(res!(map_must(&mut d, "indent")))),
		})
	}
}

impl ThemeHeadingLevel {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(omapdat!{
			"size"			=> sp_dat(self.size),
			"face"			=> opt_str_dat(&self.face),
			"weight"		=> opt_u16_dat(self.weight),
			"italic"		=> dat!(self.italic),
			"smallcaps"		=> dat!(self.smallcaps),
			"numbering"		=> opt_str_dat(&self.numbering),
			"space_above"	=> sp_dat(self.space_above),
			"space_below"	=> sp_dat(self.space_below),
		})
	}
	fn from_dat(mut d: Dat) -> Outcome<Self> {
		Ok(Self {
			size:			res!(sp_from(res!(map_must(&mut d, "size")))),
			face:			res!(opt_str_from(res!(map_must(&mut d, "face")))),
			weight:			res!(opt_u16_from(res!(map_must(&mut d, "weight")))),
			italic:			try_extract_dat!(res!(map_must(&mut d, "italic")), Bool),
			smallcaps:		try_extract_dat!(res!(map_must(&mut d, "smallcaps")), Bool),
			numbering:		res!(opt_str_from(res!(map_must(&mut d, "numbering")))),
			space_above:	res!(sp_from(res!(map_must(&mut d, "space_above")))),
			space_below:	res!(sp_from(res!(map_must(&mut d, "space_below")))),
		})
	}
}

impl ThemeHeading {
	fn to_dat(&self) -> Outcome<Dat> {
		let mut levels = Vec::with_capacity(4);
		for l in &self.levels {
			levels.push(res!(l.to_dat()));
		}
		Ok(omapdat!{
			"kind"			=> heading_kind_dat(self.kind),
			"levels"		=> Dat::List(levels),
			"chap_num_size"	=> sp_dat(self.chap_num_size),
			"chap_grid"		=> sp4_dat(&self.chap_grid),
		})
	}
	fn from_dat(mut d: Dat) -> Outcome<Self> {
		let kind		= res!(heading_kind_from(res!(map_must(&mut d, "kind"))));
		let levels_list	= try_extract_dat!(res!(map_must(&mut d, "levels")), List);
		if levels_list.len() != 4 {
			return Err(err!("A theme heading must hold 4 levels, found {}.", levels_list.len(); Input, Invalid));
		}
		let mut it		= levels_list.into_iter();
		let l0			= res!(ThemeHeadingLevel::from_dat(res!(it.next().ok_or_else(|| err!("missing heading level 1"; Input, Missing)))));
		let l1			= res!(ThemeHeadingLevel::from_dat(res!(it.next().ok_or_else(|| err!("missing heading level 2"; Input, Missing)))));
		let l2			= res!(ThemeHeadingLevel::from_dat(res!(it.next().ok_or_else(|| err!("missing heading level 3"; Input, Missing)))));
		let l3			= res!(ThemeHeadingLevel::from_dat(res!(it.next().ok_or_else(|| err!("missing heading level 4"; Input, Missing)))));
		Ok(Self {
			kind,
			levels:			[l0, l1, l2, l3],
			chap_num_size:	res!(sp_from(res!(map_must(&mut d, "chap_num_size")))),
			chap_grid:		res!(sp4_from(res!(map_must(&mut d, "chap_grid")))),
		})
	}
}

impl ThemeList {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(omapdat!{
			"marker_gap"	=> sp_dat(self.marker_gap),
			"item_skip"		=> sp_dat(self.item_skip),
		})
	}
	fn from_dat(mut d: Dat) -> Outcome<Self> {
		Ok(Self {
			marker_gap:	res!(sp_from(res!(map_must(&mut d, "marker_gap")))),
			item_skip:	res!(sp_from(res!(map_must(&mut d, "item_skip")))),
		})
	}
}

impl ThemeEnum {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(omapdat!{
			"marker_gap"	=> sp_dat(self.marker_gap),
			"item_skip"		=> sp_dat(self.item_skip),
			"numbering"		=> opt_str_dat(&self.numbering),
		})
	}
	fn from_dat(mut d: Dat) -> Outcome<Self> {
		Ok(Self {
			marker_gap:	res!(sp_from(res!(map_must(&mut d, "marker_gap")))),
			item_skip:	res!(sp_from(res!(map_must(&mut d, "item_skip")))),
			numbering:	res!(opt_str_from(res!(map_must(&mut d, "numbering")))),
		})
	}
}

impl ThemeTable {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(omapdat!{
			"skip"			=> sp_dat(self.skip),
			"cell_pad_x"	=> sp_dat(self.cell_pad_x),
			"cell_pad_y"	=> sp_dat(self.cell_pad_y),
			"line_gap"		=> sp_dat(self.line_gap),
			"rule_thin"		=> sp_dat(self.rule_thin),
			"rule_thick"	=> sp_dat(self.rule_thick),
		})
	}
	fn from_dat(mut d: Dat) -> Outcome<Self> {
		Ok(Self {
			skip:		res!(sp_from(res!(map_must(&mut d, "skip")))),
			cell_pad_x:	res!(sp_from(res!(map_must(&mut d, "cell_pad_x")))),
			cell_pad_y:	res!(sp_from(res!(map_must(&mut d, "cell_pad_y")))),
			line_gap:	res!(sp_from(res!(map_must(&mut d, "line_gap")))),
			rule_thin:	res!(sp_from(res!(map_must(&mut d, "rule_thin")))),
			rule_thick:	res!(sp_from(res!(map_must(&mut d, "rule_thick")))),
		})
	}
}

impl ThemeFigure {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(omapdat!{
			"caption_size"	=> sp_dat(self.caption_size),
			"skip"			=> sp_dat(self.skip),
		})
	}
	fn from_dat(mut d: Dat) -> Outcome<Self> {
		Ok(Self {
			caption_size:	res!(sp_from(res!(map_must(&mut d, "caption_size")))),
			skip:			res!(sp_from(res!(map_must(&mut d, "skip")))),
		})
	}
}

impl ThemeCode {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(omapdat!{
			"size"			=> sp_dat(self.size),
			"background"	=> rgba_dat(self.background),
		})
	}
	fn from_dat(mut d: Dat) -> Outcome<Self> {
		Ok(Self {
			size:		res!(sp_from(res!(map_must(&mut d, "size")))),
			background:	res!(rgba_from(res!(map_must(&mut d, "background")))),
		})
	}
}

impl ThemeCallout {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(omapdat!{ "fill" => rgba_dat(self.fill) })
	}
	fn from_dat(mut d: Dat) -> Outcome<Self> {
		Ok(Self { fill: res!(rgba_from(res!(map_must(&mut d, "fill")))) })
	}
}

impl ThemeEquation {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(omapdat!{ "numbering" => opt_str_dat(&self.numbering) })
	}
	fn from_dat(mut d: Dat) -> Outcome<Self> {
		Ok(Self { numbering: res!(opt_str_from(res!(map_must(&mut d, "numbering")))) })
	}
}

impl ThemePagePart {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(omapdat!{
			"width"				=> opt_sp_dat(self.width),
			"height"			=> opt_sp_dat(self.height),
			"margin_inside"		=> opt_sp_dat(self.margin_inside),
			"margin_outside"	=> opt_sp_dat(self.margin_outside),
			"margin_top"		=> opt_sp_dat(self.margin_top),
			"margin_bottom"		=> opt_sp_dat(self.margin_bottom),
		})
	}
	fn from_dat(mut d: Dat) -> Outcome<Self> {
		Ok(Self {
			width:			res!(opt_sp_from(res!(map_must(&mut d, "width")))),
			height:			res!(opt_sp_from(res!(map_must(&mut d, "height")))),
			margin_inside:	res!(opt_sp_from(res!(map_must(&mut d, "margin_inside")))),
			margin_outside:	res!(opt_sp_from(res!(map_must(&mut d, "margin_outside")))),
			margin_top:		res!(opt_sp_from(res!(map_must(&mut d, "margin_top")))),
			margin_bottom:	res!(opt_sp_from(res!(map_must(&mut d, "margin_bottom")))),
		})
	}
}

impl ThemePage {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(omapdat!{
			"front"	=> res!(self.front.to_dat()),
			"body"	=> res!(self.body.to_dat()),
			"back"	=> res!(self.back.to_dat()),
		})
	}
	fn from_dat(mut d: Dat) -> Outcome<Self> {
		Ok(Self {
			front:	res!(ThemePagePart::from_dat(res!(map_must(&mut d, "front")))),
			body:	res!(ThemePagePart::from_dat(res!(map_must(&mut d, "body")))),
			back:	res!(ThemePagePart::from_dat(res!(map_must(&mut d, "back")))),
		})
	}
}

impl ThemeFurniture {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(omapdat!{
			"header_size"	=> sp_dat(self.header_size),
			"folio_size"	=> sp_dat(self.folio_size),
			"foot_size"		=> sp_dat(self.foot_size),
			"foot_leading"	=> sp_dat(self.foot_leading),
		})
	}
	fn from_dat(mut d: Dat) -> Outcome<Self> {
		Ok(Self {
			header_size:	res!(sp_from(res!(map_must(&mut d, "header_size")))),
			folio_size:		res!(sp_from(res!(map_must(&mut d, "folio_size")))),
			foot_size:		res!(sp_from(res!(map_must(&mut d, "foot_size")))),
			foot_leading:	res!(sp_from(res!(map_must(&mut d, "foot_leading")))),
		})
	}
}

impl ThemeColours {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(omapdat!{
			"chap_num_grey"	=> rgba_dat(self.chap_num_grey),
			"header_fill"	=> rgba_dat(self.header_fill),
		})
	}
	fn from_dat(mut d: Dat) -> Outcome<Self> {
		Ok(Self {
			chap_num_grey:	res!(rgba_from(res!(map_must(&mut d, "chap_num_grey")))),
			header_fill:	res!(rgba_from(res!(map_must(&mut d, "header_fill")))),
		})
	}
}

impl ThemeCalibration {
	// Serialised as the f64's raw IEEE-754 bits (a `U64`) so the value round-trips bit-for-bit without a
	// float dependency, and the canonical byte order the block-address hash later reads is exact.
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(omapdat!{ "line_box_em_bits" => dat!(self.line_box_em.to_bits()) })
	}
	fn from_dat(mut d: Dat) -> Outcome<Self> {
		let bits = try_extract_dat!(res!(map_must(&mut d, "line_box_em_bits")), U64);
		Ok(Self { line_box_em: f64::from_bits(bits) })
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TOP-LEVEL SERIALISATION                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

impl ToDat for Theme {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(omapdat!{
			"text"			=> res!(self.text.to_dat()),
			"par"			=> res!(self.par.to_dat()),
			"heading"		=> res!(self.heading.to_dat()),
			"list"			=> res!(self.list.to_dat()),
			"enumeration"	=> res!(self.enumeration.to_dat()),
			"table"			=> res!(self.table.to_dat()),
			"figure"		=> res!(self.figure.to_dat()),
			"code"			=> res!(self.code.to_dat()),
			"callout"		=> res!(self.callout.to_dat()),
			"equation"		=> res!(self.equation.to_dat()),
			"page"			=> res!(self.page.to_dat()),
			"furniture"		=> res!(self.furniture.to_dat()),
			"colours"		=> res!(self.colours.to_dat()),
			"calibration"	=> res!(self.calibration.to_dat()),
		})
	}
}

impl FromDat for Theme {
	fn from_dat(mut dat: Dat) -> Outcome<Self> {
		if dat.kind() != Kind::OrdMap && dat.kind() != Kind::Map {
			return Err(err!(
				"A theme must decode from a jdat map, found a {:?}.", dat.kind();
				Input, Invalid, Mismatch));
		}
		Ok(Self {
			text:			res!(ThemeText::from_dat(res!(map_must(&mut dat, "text")))),
			par:			res!(ThemePar::from_dat(res!(map_must(&mut dat, "par")))),
			heading:		res!(ThemeHeading::from_dat(res!(map_must(&mut dat, "heading")))),
			list:			res!(ThemeList::from_dat(res!(map_must(&mut dat, "list")))),
			enumeration:	res!(ThemeEnum::from_dat(res!(map_must(&mut dat, "enumeration")))),
			table:			res!(ThemeTable::from_dat(res!(map_must(&mut dat, "table")))),
			figure:			res!(ThemeFigure::from_dat(res!(map_must(&mut dat, "figure")))),
			code:			res!(ThemeCode::from_dat(res!(map_must(&mut dat, "code")))),
			callout:		res!(ThemeCallout::from_dat(res!(map_must(&mut dat, "callout")))),
			equation:		res!(ThemeEquation::from_dat(res!(map_must(&mut dat, "equation")))),
			page:			res!(ThemePage::from_dat(res!(map_must(&mut dat, "page")))),
			furniture:		res!(ThemeFurniture::from_dat(res!(map_must(&mut dat, "furniture")))),
			colours:		res!(ThemeColours::from_dat(res!(map_must(&mut dat, "colours")))),
			calibration:	res!(ThemeCalibration::from_dat(res!(map_must(&mut dat, "calibration")))),
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The default theme survives a jdat round-trip unchanged -- the identity the block-address hash
	/// later relies on, and the guard that every field is both written and read.
	#[test]
	fn theme_round_trips_through_jdat() -> Outcome<()> {
		let theme	= Theme::default();
		let dat		= res!(theme.to_dat());
		let back	= res!(Theme::from_dat(dat));
		if theme != back {
			return Err(err!("The default theme did not round-trip through jdat: {:?} vs {:?}", theme, back; Test, Mismatch));
		}
		Ok(())
	}

	/// A theme with every group nudged off its default still round-trips, so the identity is not an
	/// accident of the defaults (an unread field would pass the default test but fail here).
	#[test]
	fn theme_round_trips_when_populated() -> Outcome<()> {
		let mut theme = Theme::default();
		theme.text.tracking			= Sp::from_pt(0.5);
		theme.text.justify				= false;
		theme.text.faces.body			= Some("Libertinus Serif".to_string());
		theme.heading.kind				= HeadingStyle::DocInline;
		theme.heading.levels[0].numbering	= Some("1.1".to_string());
		theme.heading.levels[2].weight		= Some(700);
		theme.equation.numbering		= Some("(1)".to_string());
		theme.page.body.margin_inside	= Some(Sp::from_pt(19.0));
		theme.calibration.line_box_em	= 0.682;
		let dat		= res!(theme.to_dat());
		let back	= res!(Theme::from_dat(dat));
		if theme != back {
			return Err(err!("A populated theme did not round-trip through jdat."; Test, Mismatch));
		}
		Ok(())
	}

	/// A `ThemePatch` folds only the fields it names onto a theme: a `Some` leaf overwrites, a `None` leaf
	/// leaves the theme's own value, and an `Option<Option<..>>` leaf can set a face or clear it. An empty
	/// patch is a no-op -- the identity the byte-identical gate rests on.
	#[test]
	fn theme_patch_applies_only_named_fields() {
		let mut theme	= Theme::default();
		let before		= theme.clone();

		// An empty patch changes nothing.
		theme.apply(&ThemePatch::default());
		assert_eq!(theme, before);

		// A patch naming a handful of fields across groups overwrites exactly those.
		let mut patch = ThemePatch::default();
		patch.text.body_size				= Some(Sp::from_pt(12.0));
		patch.text.faces.body				= Some(Some("Libertinus Serif".to_string()));
		patch.par.indent					= Some(Sp::from_pt(18.0));
		patch.heading.levels[0].numbering	= Some(Some("1.1".to_string()));
		patch.equation.numbering			= Some(Some("(1)".to_string()));
		patch.page.body.width				= Some(Some(Sp::from_pt(400.0)));
		theme.apply(&patch);

		assert_eq!(theme.text.body_size,				Sp::from_pt(12.0));
		assert_eq!(theme.text.faces.body,				Some("Libertinus Serif".to_string()));
		assert_eq!(theme.par.indent,					Sp::from_pt(18.0));
		assert_eq!(theme.heading.levels[0].numbering,	Some("1.1".to_string()));
		assert_eq!(theme.equation.numbering,			Some("(1)".to_string()));
		assert_eq!(theme.page.body.width,				Some(Sp::from_pt(400.0)));
		// A field the patch did not name kept its default.
		assert_eq!(theme.text.leading,					Theme::default().text.leading);
		assert_eq!(theme.heading.levels[1].numbering,	None);

		// An `Option<Option<..>>` leaf set to `Some(None)` clears the theme's own value.
		let mut clear = ThemePatch::default();
		clear.text.faces.body = Some(None);
		theme.apply(&clear);
		assert_eq!(theme.text.faces.body, None);
	}
}
