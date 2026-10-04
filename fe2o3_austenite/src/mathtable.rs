//! A reader for the OpenType MATH table and the few other tables maths layout needs, parsed from the
//! font's own bytes: every `MathConstants` value, the glyph information (italics corrections, top accent
//! attachments, extended shapes, cut-in kerning), the vertical and horizontal glyph constructions with
//! their assemblies, and from the rest of the font the advance widths (`hmtx`), the metrics a font
//! without a MATH table falls back to (`OS/2`, `post`, `hhea`) and the GSUB feature tags.
//!
//! No crate in the workspace exposes these tables, so they are read here. Values stay in font design
//! units; a caller divides by [`FontTables::upem`] for ems. Generic enough to move into `fe2o3_font`
//! once a second caller needs it.

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;

/// Every `MathConstants` value as the font stores it, in design units (the two percentages and the
/// raise percent are plain numbers). Field names follow the OpenType specification.
#[derive(Clone, Copy, Debug, Default)]
pub struct Constants {
	pub script_percent_scale_down:					i16,
	pub script_script_percent_scale_down:			i16,
	pub delimited_sub_formula_min_height:			u16,
	pub display_operator_min_height:				u16,
	pub math_leading:								i16,
	pub axis_height:								i16,
	pub accent_base_height:							i16,
	pub flattened_accent_base_height:				i16,
	pub subscript_shift_down:						i16,
	pub subscript_top_max:							i16,
	pub subscript_baseline_drop_min:				i16,
	pub superscript_shift_up:						i16,
	pub superscript_shift_up_cramped:				i16,
	pub superscript_bottom_min:						i16,
	pub superscript_baseline_drop_max:				i16,
	pub sub_superscript_gap_min:					i16,
	pub superscript_bottom_max_with_subscript:		i16,
	pub space_after_script:							i16,
	pub upper_limit_gap_min:						i16,
	pub upper_limit_baseline_rise_min:				i16,
	pub lower_limit_gap_min:						i16,
	pub lower_limit_baseline_drop_min:				i16,
	pub stack_top_shift_up:							i16,
	pub stack_top_display_style_shift_up:			i16,
	pub stack_bottom_shift_down:					i16,
	pub stack_bottom_display_style_shift_down:		i16,
	pub stack_gap_min:								i16,
	pub stack_display_style_gap_min:				i16,
	pub stretch_stack_top_shift_up:					i16,
	pub stretch_stack_bottom_shift_down:			i16,
	pub stretch_stack_gap_above_min:				i16,
	pub stretch_stack_gap_below_min:				i16,
	pub fraction_num_shift_up:						i16,
	pub fraction_num_display_shift_up:				i16,
	pub fraction_den_shift_down:					i16,
	pub fraction_den_display_shift_down:			i16,
	pub fraction_num_gap_min:						i16,
	pub fraction_num_display_gap_min:				i16,
	pub fraction_rule_thickness:					i16,
	pub fraction_denom_gap_min:						i16,
	pub fraction_denom_display_gap_min:				i16,
	pub skewed_fraction_horizontal_gap:				i16,
	pub skewed_fraction_vertical_gap:				i16,
	pub overbar_vertical_gap:						i16,
	pub overbar_rule_thickness:						i16,
	pub overbar_extra_ascender:						i16,
	pub underbar_vertical_gap:						i16,
	pub underbar_rule_thickness:					i16,
	pub underbar_extra_descender:					i16,
	pub radical_vertical_gap:						i16,
	pub radical_display_style_vertical_gap:			i16,
	pub radical_rule_thickness:						i16,
	pub radical_extra_ascender:						i16,
	pub radical_kern_before_degree:					i16,
	pub radical_kern_after_degree:					i16,
	pub radical_degree_bottom_raise_percent:		i16,
}

/// Cut-in kerning at one corner of a glyph: `kerns` has one more entry than `heights`.
#[derive(Clone, Debug, Default)]
pub struct Kern {
	pub heights:	Vec<i16>,
	pub kerns:		Vec<i16>,
}

impl Kern {
	/// The kern at a height, both in design units: the first band whose correction height the height
	/// does not exceed.
	pub fn at(&self, height: f64) -> i16 {
		let mut i = 0;
		while i < self.heights.len() && height > self.heights[i] as f64 {
			i += 1;
		}
		self.kerns.get(i).copied().unwrap_or(0)
	}
}

/// The four corners' kerning of one glyph.
#[derive(Clone, Debug, Default)]
pub struct KernInfo {
	pub top_right:		Option<Kern>,
	pub top_left:		Option<Kern>,
	pub bottom_right:	Option<Kern>,
	pub bottom_left:	Option<Kern>,
}

/// One part of a glyph assembly.
#[derive(Clone, Copy, Debug)]
pub struct Part {
	pub glyph:			u16,
	pub start_connector:	u16,
	pub end_connector:	u16,
	pub full_advance:	u16,
	pub extender:		bool,
}

#[derive(Clone, Debug)]
pub struct Assembly {
	pub italics_correction:	i16,
	pub parts:				Vec<Part>,
}

/// A glyph's construction along one axis: pre-drawn variants (smallest first, the glyph itself first)
/// and an optional assembly from parts.
#[derive(Clone, Debug)]
pub struct Construction {
	pub variants:	Vec<(u16, u16)>,	// (variant glyph, advance along the axis)
	pub assembly:	Option<Assembly>,
}

/// The parsed MATH table.
pub struct MathTable {
	upem:				f32,
	consts:				Constants,
	italics:			HashMap<u16, i16>,
	top_accent:			HashMap<u16, i16>,
	extended:			HashMap<u16, ()>,
	kerns:				HashMap<u16, KernInfo>,
	min_overlap:		u16,
	vertical:			HashMap<u16, Construction>,
	horizontal:			HashMap<u16, Construction>,
}

impl MathTable {
	/// Parses the MATH table from a whole font file, or `None` when the font carries none.
	pub fn parse(font: &[u8]) -> Outcome<Option<Self>> {
		let (math, head) = match (find_table(font, b"MATH"), find_table(font, b"head")) {
			(Some(m), Some(h))	=> (m, h),
			_					=> return Ok(None),
		};
		let upem = res!(be_u16(font, head + 18)) as f32;
		if upem <= 0.0 {
			return Err(err!("MATH: the font declares {} units per em.", upem; Input, Invalid));
		}
		let consts_off	= res!(be_u16(font, math + 4)) as usize;
		let info_off	= res!(be_u16(font, math + 6)) as usize;
		let var_off		= res!(be_u16(font, math + 8)) as usize;
		let consts = if consts_off == 0 { Constants::default() } else {
			res!(parse_constants(font, math + consts_off))
		};
		let mut table = Self {
			upem,
			consts,
			italics:		HashMap::new(),
			top_accent:		HashMap::new(),
			extended:		HashMap::new(),
			kerns:			HashMap::new(),
			min_overlap:	0,
			vertical:		HashMap::new(),
			horizontal:		HashMap::new(),
		};
		if info_off != 0 {
			res!(table.parse_glyph_info(font, math + info_off));
		}
		if var_off != 0 {
			res!(table.parse_variants(font, math + var_off));
		}
		Ok(Some(table))
	}

	/// A design-unit length scaled to a size in points.
	pub fn scaled(&self, du: i16, size_pt: f32) -> f32 {
		du as f32 * size_pt / self.upem
	}

	pub fn constants(&self) -> &Constants { &self.consts }

	pub fn italics_correction(&self, gid: u16) -> Option<i16> { self.italics.get(&gid).copied() }

	pub fn top_accent_attachment(&self, gid: u16) -> Option<i16> { self.top_accent.get(&gid).copied() }

	/// Is the glyph an extended shape (a grown delimiter, a big operator), whose italics correction is
	/// not added to its advance?
	pub fn is_extended_shape(&self, gid: u16) -> bool { self.extended.contains_key(&gid) }

	pub fn kern_info(&self, gid: u16) -> Option<&KernInfo> { self.kerns.get(&gid) }

	pub fn min_connector_overlap(&self) -> u16 { self.min_overlap }

	/// The glyph's construction along an axis, `vertical` or horizontal.
	pub fn construction(&self, gid: u16, vertical: bool) -> Option<&Construction> {
		if vertical { self.vertical.get(&gid) } else { self.horizontal.get(&gid) }
	}

	/// The vertical variant of `base` at least `min_height_pt` tall at a type size, choosing the tightest
	/// that fits (or the tallest available).
	pub fn variant_for(&self, base: u16, min_height_pt: f32, size_pt: f32) -> Option<u16> {
		let min_du = min_height_pt * self.upem / size_pt;
		self.vertical_variant(base, min_du)
	}

	/// The smallest vertical variant of `base` at least `min_du` design units tall, or the tallest when
	/// none reaches that, or `None` when the glyph has no variants.
	pub fn vertical_variant(&self, base: u16, min_du: f32) -> Option<u16> {
		let vars = &self.vertical.get(&base)?.variants;
		let mut best: Option<(u16, u16)> = None;
		for &(gid, h) in vars {
			if (h as f32) >= min_du {
				return Some(gid);
			}
			match best {
				Some((_, bh)) if bh >= h	=> {},
				_							=> best = Some((gid, h)),
			}
		}
		best.map(|(gid, _)| gid)
	}

	fn parse_glyph_info(&mut self, b: &[u8], o: usize) -> Outcome<()> {
		let ital	= res!(be_u16(b, o)) as usize;
		let accent	= res!(be_u16(b, o + 2)) as usize;
		let ext		= res!(be_u16(b, o + 4)) as usize;
		let kern	= res!(be_u16(b, o + 6)) as usize;
		if ital != 0 {
			self.italics = res!(parse_value_map(b, o + ital));
		}
		if accent != 0 {
			self.top_accent = res!(parse_value_map(b, o + accent));
		}
		if ext != 0 {
			for g in res!(parse_coverage(b, o + ext)) {
				self.extended.insert(g, ());
			}
		}
		if kern != 0 {
			let k		= o + kern;
			let cov		= res!(parse_coverage(b, k + res!(be_u16(b, k)) as usize));
			let count	= res!(be_u16(b, k + 2)) as usize;
			for i in 0..count.min(cov.len()) {
				let rec = k + 4 + 8 * i;
				let corner = |j: usize| -> Outcome<Option<Kern>> {
					let off = res!(be_u16(b, rec + 2 * j)) as usize;
					if off == 0 {
						return Ok(None);
					}
					Ok(Some(res!(parse_kern(b, k + off))))
				};
				let info = KernInfo {
					top_right:		res!(corner(0)),
					top_left:		res!(corner(1)),
					bottom_right:	res!(corner(2)),
					bottom_left:	res!(corner(3)),
				};
				self.kerns.insert(cov[i], info);
			}
		}
		Ok(())
	}

	fn parse_variants(&mut self, b: &[u8], v: usize) -> Outcome<()> {
		self.min_overlap	= res!(be_u16(b, v));
		let vcov_off		= res!(be_u16(b, v + 2)) as usize;
		let hcov_off		= res!(be_u16(b, v + 4)) as usize;
		let vcount			= res!(be_u16(b, v + 6)) as usize;
		let hcount			= res!(be_u16(b, v + 8)) as usize;
		let vcov = if vcov_off == 0 { Vec::new() } else { res!(parse_coverage(b, v + vcov_off)) };
		let hcov = if hcov_off == 0 { Vec::new() } else { res!(parse_coverage(b, v + hcov_off)) };
		for i in 0..vcount.min(vcov.len()) {
			let con = v + res!(be_u16(b, v + 10 + 2 * i)) as usize;
			self.vertical.insert(vcov[i], res!(parse_construction(b, con)));
		}
		for i in 0..hcount.min(hcov.len()) {
			let con = v + res!(be_u16(b, v + 10 + 2 * (vcount + i))) as usize;
			self.horizontal.insert(hcov[i], res!(parse_construction(b, con)));
		}
		Ok(())
	}
}

/// The font-wide facts maths layout reads besides the MATH table.
#[derive(Clone, Copy, Debug, Default)]
pub struct FontMetrics {
	pub ascender:				i16,
	pub descender:				i16,	// negative below the baseline
	pub cap_height:				i16,
	pub x_height:				i16,
	pub underline_thickness:	i16,
	pub strikeout_thickness:	i16,
	pub subscript_y_offset:		Option<i16>,
	pub superscript_y_offset:	Option<i16>,
}

/// One font file's tables as maths layout reads them.
pub struct FontTables {
	pub upem:		f64,
	pub math:		Option<MathTable>,
	pub metrics:	FontMetrics,
	advances:		Vec<u16>,
	features:		Vec<[u8; 4]>,
}

impl FontTables {
	pub fn parse(font: &[u8]) -> Outcome<Self> {
		let head = match find_table(font, b"head") {
			Some(h)	=> h,
			None	=> return Err(err!("The font has no head table."; Input, Invalid, Missing)),
		};
		let upem = res!(be_u16(font, head + 18)) as f64;
		if upem <= 0.0 {
			return Err(err!("The font declares {} units per em.", upem; Input, Invalid));
		}
		let math = res!(MathTable::parse(font));
		let mut metrics = FontMetrics::default();
		if let Some(h) = find_table(font, b"hhea") {
			metrics.ascender	= res!(be_i16(font, h + 4));
			metrics.descender	= res!(be_i16(font, h + 6));
		}
		if let Some(os2) = find_table(font, b"OS/2") {
			let version = res!(be_u16(font, os2));
			metrics.subscript_y_offset		= Some(res!(be_i16(font, os2 + 16)));
			metrics.superscript_y_offset	= Some(res!(be_i16(font, os2 + 24)));
			metrics.strikeout_thickness		= res!(be_i16(font, os2 + 26));
			metrics.ascender				= res!(be_i16(font, os2 + 68));
			metrics.descender				= res!(be_i16(font, os2 + 70));
			if version >= 2 {
				metrics.x_height	= res!(be_i16(font, os2 + 86));
				metrics.cap_height	= res!(be_i16(font, os2 + 88));
			}
		}
		if metrics.cap_height <= 0 {
			metrics.cap_height = metrics.ascender;
		}
		if metrics.x_height <= 0 {
			metrics.x_height = metrics.ascender;
		}
		if let Some(post) = find_table(font, b"post") {
			metrics.underline_thickness = res!(be_i16(font, post + 10));
		}
		if metrics.underline_thickness <= 0 {
			metrics.underline_thickness = metrics.strikeout_thickness;
		}
		let mut advances = Vec::new();
		if let (Some(hhea), Some(hmtx)) = (find_table(font, b"hhea"), find_table(font, b"hmtx")) {
			let n = res!(be_u16(font, hhea + 34)) as usize;
			advances.reserve(n);
			for i in 0..n {
				advances.push(res!(be_u16(font, hmtx + 4 * i)));
			}
		}
		let mut features = Vec::new();
		if let Some(gsub) = find_table(font, b"GSUB") {
			let list = gsub + res!(be_u16(font, gsub + 6)) as usize;
			let count = res!(be_u16(font, list)) as usize;
			for i in 0..count {
				if let Some(t) = font.get(list + 2 + 6 * i..list + 6 + 6 * i) {
					let tag = [t[0], t[1], t[2], t[3]];
					if !features.contains(&tag) {
						features.push(tag);
					}
				}
			}
		}
		Ok(Self { upem, math, metrics, advances, features })
	}

	/// A glyph's advance width in design units; glyphs past the metrics run share the last advance.
	pub fn advance(&self, gid: u16) -> u16 {
		match self.advances.get(gid as usize) {
			Some(a)	=> *a,
			None	=> self.advances.last().copied().unwrap_or(0),
		}
	}

	/// Does the font's GSUB offer the feature?
	pub fn has_feature(&self, tag: &[u8; 4]) -> bool { self.features.contains(tag) }
}

/// Reads `MathConstants`: four 16-bit numbers, then a run of `MathValueRecord`s (a value and a device
/// offset), then the radical raise percent.
fn parse_constants(b: &[u8], c: usize) -> Outcome<Constants> {
	let v = |i: usize| -> Outcome<i16> { be_i16(b, c + 8 + 4 * i) };
	Ok(Constants {
		script_percent_scale_down:				res!(be_i16(b, c)),
		script_script_percent_scale_down:		res!(be_i16(b, c + 2)),
		delimited_sub_formula_min_height:		res!(be_u16(b, c + 4)),
		display_operator_min_height:			res!(be_u16(b, c + 6)),
		math_leading:							res!(v(0)),
		axis_height:							res!(v(1)),
		accent_base_height:						res!(v(2)),
		flattened_accent_base_height:			res!(v(3)),
		subscript_shift_down:					res!(v(4)),
		subscript_top_max:						res!(v(5)),
		subscript_baseline_drop_min:			res!(v(6)),
		superscript_shift_up:					res!(v(7)),
		superscript_shift_up_cramped:			res!(v(8)),
		superscript_bottom_min:					res!(v(9)),
		superscript_baseline_drop_max:			res!(v(10)),
		sub_superscript_gap_min:				res!(v(11)),
		superscript_bottom_max_with_subscript:	res!(v(12)),
		space_after_script:						res!(v(13)),
		upper_limit_gap_min:					res!(v(14)),
		upper_limit_baseline_rise_min:			res!(v(15)),
		lower_limit_gap_min:					res!(v(16)),
		lower_limit_baseline_drop_min:			res!(v(17)),
		stack_top_shift_up:						res!(v(18)),
		stack_top_display_style_shift_up:		res!(v(19)),
		stack_bottom_shift_down:				res!(v(20)),
		stack_bottom_display_style_shift_down:	res!(v(21)),
		stack_gap_min:							res!(v(22)),
		stack_display_style_gap_min:			res!(v(23)),
		stretch_stack_top_shift_up:				res!(v(24)),
		stretch_stack_bottom_shift_down:		res!(v(25)),
		stretch_stack_gap_above_min:			res!(v(26)),
		stretch_stack_gap_below_min:			res!(v(27)),
		fraction_num_shift_up:					res!(v(28)),
		fraction_num_display_shift_up:			res!(v(29)),
		fraction_den_shift_down:				res!(v(30)),
		fraction_den_display_shift_down:		res!(v(31)),
		fraction_num_gap_min:					res!(v(32)),
		fraction_num_display_gap_min:			res!(v(33)),
		fraction_rule_thickness:				res!(v(34)),
		fraction_denom_gap_min:					res!(v(35)),
		fraction_denom_display_gap_min:			res!(v(36)),
		skewed_fraction_horizontal_gap:			res!(v(37)),
		skewed_fraction_vertical_gap:			res!(v(38)),
		overbar_vertical_gap:					res!(v(39)),
		overbar_rule_thickness:					res!(v(40)),
		overbar_extra_ascender:					res!(v(41)),
		underbar_vertical_gap:					res!(v(42)),
		underbar_rule_thickness:				res!(v(43)),
		underbar_extra_descender:				res!(v(44)),
		radical_vertical_gap:					res!(v(45)),
		radical_display_style_vertical_gap:		res!(v(46)),
		radical_rule_thickness:					res!(v(47)),
		radical_extra_ascender:					res!(v(48)),
		radical_kern_before_degree:				res!(v(49)),
		radical_kern_after_degree:				res!(v(50)),
		radical_degree_bottom_raise_percent:	res!(be_i16(b, c + 8 + 4 * 51)),
	})
}

// A coverage table and a run of `MathValueRecord`s indexed by it: italics corrections, accent
// attachments.
fn parse_value_map(b: &[u8], o: usize) -> Outcome<HashMap<u16, i16>> {
	let cov		= res!(parse_coverage(b, o + res!(be_u16(b, o)) as usize));
	let count	= res!(be_u16(b, o + 2)) as usize;
	let mut out = HashMap::with_capacity(count);
	for i in 0..count.min(cov.len()) {
		out.insert(cov[i], res!(be_i16(b, o + 4 + 4 * i)));
	}
	Ok(out)
}

fn parse_kern(b: &[u8], o: usize) -> Outcome<Kern> {
	let n = res!(be_u16(b, o)) as usize;
	let mut heights = Vec::with_capacity(n);
	let mut kerns = Vec::with_capacity(n + 1);
	for i in 0..n {
		heights.push(res!(be_i16(b, o + 2 + 4 * i)));
	}
	for i in 0..=n {
		kerns.push(res!(be_i16(b, o + 2 + 4 * n + 4 * i)));
	}
	Ok(Kern { heights, kerns })
}

fn parse_construction(b: &[u8], c: usize) -> Outcome<Construction> {
	let asm_off	= res!(be_u16(b, c)) as usize;
	let count	= res!(be_u16(b, c + 2)) as usize;
	let mut variants = Vec::with_capacity(count);
	for k in 0..count {
		variants.push((res!(be_u16(b, c + 4 + 4 * k)), res!(be_u16(b, c + 6 + 4 * k))));
	}
	let assembly = if asm_off == 0 { None } else {
		let a = c + asm_off;
		let italics	= res!(be_i16(b, a));
		let n		= res!(be_u16(b, a + 4)) as usize;
		let mut parts = Vec::with_capacity(n);
		for i in 0..n {
			let p = a + 6 + 10 * i;
			parts.push(Part {
				glyph:				res!(be_u16(b, p)),
				start_connector:	res!(be_u16(b, p + 2)),
				end_connector:		res!(be_u16(b, p + 4)),
				full_advance:		res!(be_u16(b, p + 6)),
				extender:			res!(be_u16(b, p + 8)) & 1 == 1,
			});
		}
		Some(Assembly { italics_correction: italics, parts })
	};
	Ok(Construction { variants, assembly })
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
		None	=> Err(err!("Font table: 16-bit read past the end at byte {}.", o; Input, Invalid)),
	}
}

fn be_i16(b: &[u8], o: usize) -> Outcome<i16> {
	Ok(res!(be_u16(b, o)) as i16)
}

fn be_u32(b: &[u8], o: usize) -> Outcome<u32> {
	match b.get(o..o + 4) {
		Some(s)	=> Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]])),
		None	=> Err(err!("Font table: 32-bit read past the end at byte {}.", o; Input, Invalid)),
	}
}
