//! A reader for the OpenType MATH table: the layout constants and the vertical glyph variants a
//! mathematics font carries for growing a delimiter or a radical to its content.
//!
//! No crate in the workspace exposes this table, so it is parsed here, from the font's own bytes. Only
//! what the engine uses is read: a handful of `MathConstants` (the axis, the rule thicknesses, the
//! script shifts, the radical gaps) and the vertical `MathVariants` (a base glyph mapped to a list of
//! taller pre-drawn variants). Glyph assembly -- building an arbitrarily tall delimiter from repeating
//! pieces -- is left for later; the discrete variants cover the sizes a set page reaches for. Values
//! are in font design units; a caller scales them by the type size over the units-per-em.
//!
//! This is generic enough to belong in `fe2o3_font` once a second caller needs it; it sits here until
//! then.

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;

/// The subset of `MathConstants` the engine sets to, each a raw design-unit value. Scaled to a size by
/// [`MathTable::scaled`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Constants {
	pub axis_height:				i16,	// the maths axis a fraction bar and a relation centre on
	pub fraction_rule_thickness:	i16,
	pub fraction_num_shift_up:		i16,
	pub fraction_den_shift_down:	i16,
	pub radical_vertical_gap:		i16,	// clearance between the radicand and the rule above it
	pub radical_rule_thickness:		i16,	// the vinculum's thickness
	pub radical_extra_ascender:		i16,	// space above the vinculum
	pub superscript_shift_up:		i16,
	pub subscript_shift_down:		i16,
}

/// The parsed MATH table: the units-per-em its values are in, the constants, and each vertically
/// extensible base glyph mapped to its taller variants as `(variant glyph id, height in design units)`.
pub struct MathTable {
	upem:		f32,
	consts:		Constants,
	vertical:	HashMap<u16, Vec<(u16, u16)>>,
}

impl MathTable {
	/// Parses the MATH table from a whole font file, or `None` when the font carries none. The font's
	/// units-per-em is read from `head` so the values can later be scaled to a type size.
	pub fn parse(font: &[u8]) -> Outcome<Option<Self>> {
		let (math, head) = match (find_table(font, b"MATH"), find_table(font, b"head")) {
			(Some(m), Some(h))	=> (m, h),
			_					=> return Ok(None),
		};
		let upem = res!(be_u16(font, head + 18)) as f32;
		if upem <= 0.0 {
			return Err(err!("MATH: the font declares {} units per em.", upem; Input, Invalid));
		}

		let consts	= res!(parse_constants(font, math + res!(be_u16(font, math + 4)) as usize));
		let vertical	= res!(parse_vertical_variants(font, math + res!(be_u16(font, math + 8)) as usize));
		Ok(Some(Self { upem, consts, vertical }))
	}

	/// A design-unit length scaled to a size in points.
	pub fn scaled(&self, du: i16, size_pt: f32) -> f32 {
		du as f32 * size_pt / self.upem
	}

	pub fn constants(&self) -> &Constants { &self.consts }

	/// The vertical variant of `base` at least `min_height_pt` tall at a type size, choosing the tightest
	/// that fits (or the tallest available). The height is given and compared in points; the conversion
	/// to the design units the table stores is done here.
	pub fn variant_for(&self, base: u16, min_height_pt: f32, size_pt: f32) -> Option<u16> {
		let min_du = min_height_pt * self.upem / size_pt;
		self.vertical_variant(base, min_du)
	}

	/// The glyph id of the smallest vertical variant of `base` at least `min_du` design units tall, or
	/// the tallest variant when none reaches that, or `None` when the glyph has no variants at all. The
	/// base glyph's own record is included by the font as its first (smallest) variant.
	pub fn vertical_variant(&self, base: u16, min_du: f32) -> Option<u16> {
		let vars = self.vertical.get(&base)?;
		let mut best: Option<(u16, u16)> = None;	// the tallest seen, as a fallback
		for &(gid, h) in vars {
			if (h as f32) >= min_du {
				return Some(gid);	// the list is smallest-first, so the first that fits is the tightest
			}
			match best {
				Some((_, bh)) if bh >= h	=> {},
				_							=> best = Some((gid, h)),
			}
		}
		best.map(|(gid, _)| gid)
	}
}

/// Reads the fixed run of `MathConstants` fields the engine uses. The table opens with two `int16` and
/// two `uint16`, then a run of `MathValueRecord`s (an `int16` value and an offset), so field *i* of that
/// run is the value at `8 + 4*i`.
fn parse_constants(b: &[u8], c: usize) -> Outcome<Constants> {
	let mvr = |i: usize| -> Outcome<i16> { be_i16(b, c + 8 + 4 * i) };
	Ok(Constants {
		axis_height:				res!(mvr(1)),
		subscript_shift_down:		res!(mvr(4)),
		superscript_shift_up:		res!(mvr(7)),
		fraction_num_shift_up:		res!(mvr(28)),
		fraction_den_shift_down:	res!(mvr(30)),
		fraction_rule_thickness:	res!(mvr(34)),
		radical_vertical_gap:		res!(mvr(45)),
		radical_rule_thickness:		res!(mvr(47)),
		radical_extra_ascender:		res!(mvr(48)),
	})
}

/// Reads the vertical `MathVariants`: a coverage of base glyph ids, and for each a construction listing
/// its taller variants. The construction offsets are indexed by coverage index, so the *i*th
/// construction belongs to the *i*th glyph in coverage order.
fn parse_vertical_variants(b: &[u8], v: usize) -> Outcome<HashMap<u16, Vec<(u16, u16)>>> {
	let cov_off	= res!(be_u16(b, v + 2)) as usize;
	let count	= res!(be_u16(b, v + 6)) as usize;
	let coverage	= res!(parse_coverage(b, v + cov_off));

	let mut out: HashMap<u16, Vec<(u16, u16)>> = HashMap::with_capacity(count);
	for i in 0..count {
		let base = match coverage.get(i) {
			Some(g) => *g,
			None => break,	// a construction with no covered glyph: nothing to key it by
		};
		// The construction offset array follows the header: minConnectorOverlap, the two coverage
		// offsets, and BOTH counts (vertical then horizontal) -- ten bytes -- before the first offset.
		let con		= v + res!(be_u16(b, v + 10 + 2 * i)) as usize;
		let vcount	= res!(be_u16(b, con + 2)) as usize;
		let mut vars = Vec::with_capacity(vcount);
		for k in 0..vcount {
			let gid = res!(be_u16(b, con + 4 + 4 * k));
			let adv = res!(be_u16(b, con + 4 + 4 * k + 2));
			vars.push((gid, adv));
		}
		out.insert(base, vars);
	}
	Ok(out)
}

/// Reads a coverage table (format 1 or 2) into the glyph ids in coverage-index order.
fn parse_coverage(b: &[u8], o: usize) -> Outcome<Vec<u16>> {
	match res!(be_u16(b, o)) {
		1 => {
			let n = res!(be_u16(b, o + 2)) as usize;
			let mut out = Vec::with_capacity(n);
			for i in 0..n {
				out.push(res!(be_u16(b, o + 4 + 2 * i)));
			}
			Ok(out)
		},
		2 => {
			let n = res!(be_u16(b, o + 2)) as usize;
			let mut placed: Vec<(u16, u16)> = Vec::new();	// (coverage index, glyph id)
			for i in 0..n {
				let start	= res!(be_u16(b, o + 4 + 6 * i));
				let end		= res!(be_u16(b, o + 4 + 6 * i + 2));
				let first	= res!(be_u16(b, o + 4 + 6 * i + 4));
				for (j, g) in (start..=end).enumerate() {
					placed.push((first + j as u16, g));
				}
			}
			placed.sort_by_key(|(idx, _)| *idx);
			Ok(placed.into_iter().map(|(_, g)| g).collect())
		},
		other => Err(err!("MATH: unknown coverage format {}.", other; Input, Invalid)),
	}
}

/// The offset of a table in an sfnt font, by its four-byte tag.
fn find_table(b: &[u8], tag: &[u8; 4]) -> Option<usize> {
	let num = be_u16(b, 4).ok()?;
	for i in 0..num as usize {
		let rec = 12 + i * 16;
		if b.get(rec..rec + 4) == Some(&tag[..]) {
			return be_u32(b, rec + 8).ok().map(|o| o as usize);
		}
	}
	None
}

fn be_u16(b: &[u8], o: usize) -> Outcome<u16> {
	match b.get(o..o + 2) {
		Some(s)	=> Ok(u16::from_be_bytes([s[0], s[1]])),
		None	=> Err(err!("MATH: 16-bit read past the end of the table at byte {}.", o; Input, Invalid)),
	}
}

fn be_i16(b: &[u8], o: usize) -> Outcome<i16> {
	Ok(res!(be_u16(b, o)) as i16)
}

fn be_u32(b: &[u8], o: usize) -> Outcome<u32> {
	match b.get(o..o + 4) {
		Some(s)	=> Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]])),
		None	=> Err(err!("MATH: 32-bit read past the end of the table at byte {}.", o; Input, Invalid)),
	}
}
