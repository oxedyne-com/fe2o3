//! The font programs a PDF embeds, and their subsets.
//!
//! A [`FontProgram`] is an OpenType or TrueType file read just far enough to be embedded: its outline
//! flavour, its metrics for the font descriptor, its advance widths, and its PostScript name. The
//! [`subset`](FontProgram::subset) keeps only the glyphs a document shows, yet a glyph id from the shaper
//! is the PDF's CID unchanged either way, so no remapping table sits between the page and the font.
//!
//! TrueType (`glyf`) outlines become a trimmed `sfnt` for `/FontFile2`, the unused glyphs emptied in
//! place. CFF outlines become a bare CID-keyed CFF for `/FontFile3 /CIDFontType0C`, the kept glyphs packed
//! with their subroutines inlined and the charset mapping each back to its original id as its CID. A
//! name-keyed font is re-keyed this way too, because a name-keyed program inside a CID font is read
//! inconsistently across viewers. A `CFF2` (variable) font and a font whose licence forbids embedding
//! are refused with `None`, and the caller draws that face's glyphs as outlines instead.

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeSet;
use std::sync::Arc;

/// The outline flavour of an embeddable program, which decides the PDF font subtype.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outlines {
	TrueType,	// `glyf`, embedded as CIDFontType2 over /FontFile2
	Cff,		// `CFF `, embedded as CIDFontType0 over /FontFile3 /CIDFontType0C
}

/// A font program ready for a PDF font file stream, in the form its `/FontFile` key and subtype need.
#[derive(Clone, Debug)]
pub enum FontFile {
	TrueType(Vec<u8>),	// /FontFile2
	Cff(Vec<u8>),		// /FontFile3 /Subtype /CIDFontType0C
	OpenType(Vec<u8>),	// /FontFile3 /Subtype /OpenType, the whole file when a CFF subset fails
}

/// One embeddable font file, parsed once and shared. The bytes are held whole; only
/// [`subset`](Self::subset) cuts them down, at the end of a document when the glyphs shown are known.
pub struct FontProgram {
	key:			u64,			// content fingerprint, so two loads of one file are one PDF font
	data:			Arc<Vec<u8>>,
	outlines:		Outlines,
	tables:			Vec<Table>,
	upem:			u16,
	advances:		Vec<u16>,		// per glyph, font units
	num_glyphs:		u16,
	bbox:			[i16; 4],		// head xMin, yMin, xMax, yMax
	ascent:			i16,
	descent:		i16,
	cap_height:		i16,
	weight:			u16,
	italic_angle:	f32,
	fixed_pitch:	bool,
	may_subset:		bool,			// OS/2 fsType does not forbid subsetting
	name:			String,			// PostScript name, sanitised for a PDF name object
}

impl std::fmt::Debug for FontProgram {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("FontProgram")
			.field("name", &self.name)
			.field("outlines", &self.outlines)
			.field("glyphs", &self.num_glyphs)
			.field("bytes", &self.data.len())
			.finish()
	}
}

#[derive(Clone, Copy, Debug)]
struct Table {
	tag:	[u8; 4],
	off:	usize,
	len:	usize,
}

/// A 64-bit FNV-1a over the whole file.
fn fingerprint(bytes: &[u8]) -> u64 {
	let mut h: u64 = 0xcbf2_9ce4_8422_2325;
	for &b in bytes {
		h ^= b as u64;
		h = h.wrapping_mul(0x0000_0100_0000_01b3);
	}
	h
}

fn rd_u8(d: &[u8], at: usize) -> Outcome<u8> {
	match d.get(at) {
		Some(b)	=> Ok(*b),
		None	=> Err(err!("A font read at byte {} ran past the end of {} bytes.", at, d.len();
			Invalid, Input, Size)),
	}
}

fn rd_u16(d: &[u8], at: usize) -> Outcome<u16> {
	Ok(((res!(rd_u8(d, at)) as u16) << 8) | res!(rd_u8(d, at + 1)) as u16)
}

fn rd_i16(d: &[u8], at: usize) -> Outcome<i16> {
	Ok(res!(rd_u16(d, at)) as i16)
}

fn rd_u32(d: &[u8], at: usize) -> Outcome<u32> {
	Ok(((res!(rd_u16(d, at)) as u32) << 16) | res!(rd_u16(d, at + 2)) as u32)
}

/// An unsigned big-endian integer of `n` bytes, one to four, as CFF offsets are written.
fn rd_uint(d: &[u8], at: usize, n: usize) -> Outcome<usize> {
	let mut v = 0usize;
	for k in 0..n {
		v = (v << 8) | res!(rd_u8(d, at + k)) as usize;
	}
	Ok(v)
}

fn slice(d: &[u8], at: usize, len: usize) -> Outcome<&[u8]> {
	match at.checked_add(len).and_then(|end| d.get(at..end)) {
		Some(s)	=> Ok(s),
		None	=> Err(err!("A font span {}+{} runs past the end of {} bytes.", at, len, d.len();
			Invalid, Input, Size)),
	}
}

impl FontProgram {

	/// Reads a font file for embedding. `None` when the file is well formed but cannot be embedded: a
	/// `CFF2` variable font, a bitmap-only licence, or a restricted-licence font whose `fsType` forbids
	/// embedding altogether. An error only when the file is not a font this reader understands.
	pub fn parse(data: Arc<Vec<u8>>) -> Outcome<Option<Self>> {
		let d = &data[..];
		let key = fingerprint(d);
		let version = res!(rd_u32(d, 0));
		if version == 0x7474_6366 {	// 'ttcf', a collection: its faces arrive one by one elsewhere
			return Ok(None);
		}
		let count = res!(rd_u16(d, 4)) as usize;
		let mut tables = Vec::with_capacity(count);
		for i in 0..count {
			let rec = 12 + 16 * i;
			let tag = res!(slice(d, rec, 4));
			let off = res!(rd_u32(d, rec + 8)) as usize;
			let len = res!(rd_u32(d, rec + 12)) as usize;
			res!(slice(d, off, len));
			tables.push(Table { tag: [tag[0], tag[1], tag[2], tag[3]], off, len });
		}
		let find = |t: &[u8; 4]| tables.iter().find(|x| &x.tag == t).copied();

		let outlines = if find(b"glyf").is_some() && find(b"loca").is_some() {
			Outlines::TrueType
		} else if find(b"CFF ").is_some() {
			Outlines::Cff
		} else {
			return Ok(None);	// CFF2, or bitmap-only
		};

		let head = match find(b"head") {
			Some(t)	=> t,
			None	=> return Err(err!("The font has no head table."; Invalid, Input, Missing)),
		};
		let hhea = match find(b"hhea") {
			Some(t)	=> t,
			None	=> return Err(err!("The font has no hhea table."; Invalid, Input, Missing)),
		};
		let maxp = match find(b"maxp") {
			Some(t)	=> t,
			None	=> return Err(err!("The font has no maxp table."; Invalid, Input, Missing)),
		};
		let hmtx = match find(b"hmtx") {
			Some(t)	=> t,
			None	=> return Err(err!("The font has no hmtx table."; Invalid, Input, Missing)),
		};

		let upem = res!(rd_u16(d, head.off + 18));
		if upem == 0 {
			return Err(err!("The font declares zero units per em."; Invalid, Input));
		}
		let bbox = [
			res!(rd_i16(d, head.off + 36)),
			res!(rd_i16(d, head.off + 38)),
			res!(rd_i16(d, head.off + 40)),
			res!(rd_i16(d, head.off + 42)),
		];
		let mut ascent	= res!(rd_i16(d, hhea.off + 4));
		let mut descent	= res!(rd_i16(d, hhea.off + 6));
		let n_hm		= res!(rd_u16(d, hhea.off + 34)) as usize;
		let num_glyphs	= res!(rd_u16(d, maxp.off + 4));

		// Advances: one long metric per glyph up to numberOfHMetrics, the last repeated after.
		let mut advances = Vec::with_capacity(num_glyphs as usize);
		let mut last = 0u16;
		for g in 0..num_glyphs as usize {
			if g < n_hm {
				last = res!(rd_u16(d, hmtx.off + 4 * g));
			}
			advances.push(last);
		}

		let mut cap_height	= bbox[3];
		let mut weight		= 400u16;
		let mut may_subset	= true;
		if let Some(os2) = find(b"OS/2") {
			let ver		= res!(rd_u16(d, os2.off));
			weight		= res!(rd_u16(d, os2.off + 4));
			let fs_type	= res!(rd_u16(d, os2.off + 8));
			// Bit 1 alone is a restricted licence; bit 9 permits bitmaps only. Either rules out an outline.
			if fs_type & 0x000f == 0x0002 || fs_type & 0x0200 != 0 {
				return Ok(None);
			}
			may_subset = fs_type & 0x0100 == 0;
			if os2.len >= 72 {
				let typo_asc = res!(rd_i16(d, os2.off + 68));
				let typo_des = res!(rd_i16(d, os2.off + 70));
				if typo_asc != 0 { ascent = typo_asc; }
				if typo_des != 0 { descent = typo_des; }
			}
			if ver >= 2 && os2.len >= 90 {
				let ch = res!(rd_i16(d, os2.off + 88));
				if ch > 0 { cap_height = ch; }
			}
		}

		let mut italic_angle	= 0.0f32;
		let mut fixed_pitch		= false;
		if let Some(post) = find(b"post") {
			italic_angle	= (res!(rd_u32(d, post.off + 4)) as i32) as f32 / 65536.0;
			fixed_pitch		= res!(rd_u32(d, post.off + 12)) != 0;
		}

		let name = match find(b"name") {
			Some(t)	=> postscript_name(d, t).unwrap_or_default(),
			None	=> String::new(),
		};
		let name = if name.is_empty() { "Font".to_string() } else { name };

		Ok(Some(Self {
			key,
			data,
			outlines,
			tables,
			upem,
			advances,
			num_glyphs,
			bbox,
			ascent,
			descent,
			cap_height,
			weight,
			italic_angle,
			fixed_pitch,
			may_subset,
			name,
		}))
	}

	pub fn key(&self) -> u64 { self.key }
	pub fn outlines(&self) -> Outlines { self.outlines }
	pub fn name(&self) -> &str { &self.name }
	pub fn num_glyphs(&self) -> u16 { self.num_glyphs }

	/// A glyph's advance in thousandths of an em, the unit of a PDF `/W` array and a `TJ` adjustment.
	pub fn width(&self, gid: u16) -> i64 {
		let adv = self.advances.get(gid as usize).copied().unwrap_or(0) as f64;
		(adv * 1000.0 / self.upem as f64).round() as i64
	}

	/// A font-unit value in thousandths of an em.
	pub(crate) fn em(&self, v: i16) -> i64 {
		((v as f64) * 1000.0 / self.upem as f64).round() as i64
	}

	pub(crate) fn bbox(&self) -> [i64; 4] {
		[self.em(self.bbox[0]), self.em(self.bbox[1]), self.em(self.bbox[2]), self.em(self.bbox[3])]
	}
	pub(crate) fn ascent(&self) -> i64 { self.em(self.ascent) }
	pub(crate) fn descent(&self) -> i64 { self.em(self.descent) }
	pub(crate) fn cap_height(&self) -> i64 { self.em(self.cap_height) }
	pub(crate) fn italic_angle(&self) -> f32 { self.italic_angle }
	pub(crate) fn fixed_pitch(&self) -> bool { self.fixed_pitch }
	pub(crate) fn weight(&self) -> u16 { self.weight }

	fn table(&self, tag: &[u8; 4]) -> Option<&[u8]> {
		self.tables.iter()
			.find(|t| &t.tag == tag)
			.and_then(|t| self.data.get(t.off..t.off + t.len))
	}

	/// The font program cut down to `gids` (glyph zero is always kept). A font whose licence forbids
	/// subsetting keeps every glyph. Should a subset fail on a construction this reader does not follow --
	/// a `seac` accent, a malformed subroutine -- the whole file is embedded instead, which costs bytes
	/// but still shows every glyph by its id.
	pub fn subset(&self, gids: &BTreeSet<u16>) -> FontFile {
		match self.try_subset(gids) {
			Ok(f)	=> f,
			Err(_)	=> match self.outlines {
				Outlines::TrueType	=> FontFile::TrueType(self.data.to_vec()),
				Outlines::Cff		=> FontFile::OpenType(self.data.to_vec()),
			},
		}
	}

	fn try_subset(&self, gids: &BTreeSet<u16>) -> Outcome<FontFile> {
		let mut keep: BTreeSet<u16> = if self.may_subset {
			gids.iter().copied().filter(|&g| g < self.num_glyphs).collect()
		} else {
			(0..self.num_glyphs).collect()
		};
		keep.insert(0);
		match self.outlines {
			Outlines::TrueType	=> Ok(FontFile::TrueType(res!(self.subset_truetype(keep)))),
			Outlines::Cff		=> {
				let cff = match self.table(b"CFF ") {
					Some(t)	=> t,
					None	=> return Err(err!("The CFF table vanished between parse and subset."; Bug)),
				};
				Ok(FontFile::Cff(res!(subset_cff(cff, &keep, self.num_glyphs as usize))))
			},
		}
	}

	// ┌───────────────────────────────────────────────────────────────────────────┐
	// │ TRUETYPE                                                                   │
	// └───────────────────────────────────────────────────────────────────────────┘

	fn subset_truetype(&self, mut keep: BTreeSet<u16>) -> Outcome<Vec<u8>> {
		let head = match self.table(b"head") {
			Some(t)	=> t,
			None	=> return Err(err!("The font has no head table."; Invalid, Input, Missing)),
		};
		let loca = match self.table(b"loca") {
			Some(t)	=> t,
			None	=> return Err(err!("The font has no loca table."; Invalid, Input, Missing)),
		};
		let glyf = match self.table(b"glyf") {
			Some(t)	=> t,
			None	=> return Err(err!("The font has no glyf table."; Invalid, Input, Missing)),
		};
		let long = res!(rd_i16(head, 50)) != 0;
		let n = self.num_glyphs as usize;
		let loc = |g: usize| -> Outcome<usize> {
			if long {
				Ok(res!(rd_u32(loca, 4 * g)) as usize)
			} else {
				Ok(res!(rd_u16(loca, 2 * g)) as usize * 2)
			}
		};
		let glyph = |g: usize| -> Outcome<&[u8]> {
			let a = res!(loc(g));
			let b = res!(loc(g + 1));
			if b < a {
				return Err(err!("Glyph {} has a negative length in loca.", g; Invalid, Input));
			}
			slice(glyf, a, b - a)
		};

		// A composite glyph draws other glyphs, which must survive the subset with it.
		let mut todo: Vec<u16> = keep.iter().copied().collect();
		while let Some(g) = todo.pop() {
			let bytes = res!(glyph(g as usize));
			if bytes.len() < 10 || res!(rd_i16(bytes, 0)) >= 0 {
				continue;
			}
			let mut at = 10;
			loop {
				let flags	= res!(rd_u16(bytes, at));
				let comp	= res!(rd_u16(bytes, at + 2));
				if (comp as usize) < n && keep.insert(comp) {
					todo.push(comp);
				}
				at += 4;
				at += if flags & 0x0001 != 0 { 4 } else { 2 };	// ARG_1_AND_2_ARE_WORDS
				if flags & 0x0008 != 0 {						// WE_HAVE_A_SCALE
					at += 2;
				} else if flags & 0x0040 != 0 {					// WE_HAVE_AN_X_AND_Y_SCALE
					at += 4;
				} else if flags & 0x0080 != 0 {					// WE_HAVE_A_TWO_BY_TWO
					at += 8;
				}
				if flags & 0x0020 == 0 {						// MORE_COMPONENTS
					break;
				}
			}
		}

		let mut new_glyf: Vec<u8> = Vec::new();
		let mut new_loca: Vec<u8> = Vec::with_capacity(4 * (n + 1));
		for g in 0..n {
			new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
			if keep.contains(&(g as u16)) {
				new_glyf.extend_from_slice(res!(glyph(g)));
				while new_glyf.len() % 4 != 0 {
					new_glyf.push(0);
				}
			}
		}
		new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());

		let mut new_head = head.to_vec();
		if new_head.len() < 54 {
			return Err(err!("The head table is {} bytes, too short.", new_head.len(); Invalid, Input));
		}
		new_head[8..12].copy_from_slice(&[0, 0, 0, 0]);	// checkSumAdjustment, recomputed below
		new_head[50..52].copy_from_slice(&1u16.to_be_bytes());	// long loca

		// The metrics of an emptied glyph are zeroed, so the runs of them compress to almost nothing; a
		// viewer takes its widths from the PDF's /W, never from here.
		let mut new_hmtx = match self.table(b"hmtx") {
			Some(t)	=> t.to_vec(),
			None	=> return Err(err!("The font has no hmtx table."; Invalid, Input, Missing)),
		};
		let n_hm = match self.table(b"hhea") {
			Some(t)	=> res!(rd_u16(t, 34)) as usize,
			None	=> return Err(err!("The font has no hhea table."; Invalid, Input, Missing)),
		};
		for g in 0..n {
			if keep.contains(&(g as u16)) {
				continue;
			}
			let (at, len) = if g < n_hm { (4 * g, 4) } else { (4 * n_hm + 2 * (g - n_hm), 2) };
			if let Some(span) = new_hmtx.get_mut(at..at + len) {
				span.fill(0);
			}
		}

		let mut out_tables: Vec<([u8; 4], Vec<u8>)> = Vec::new();
		for tag in [b"cvt ", b"fpgm", b"glyf", b"head", b"hhea", b"hmtx", b"loca", b"maxp", b"prep"] {
			let body = match tag {
				b"glyf"	=> std::mem::take(&mut new_glyf),
				b"loca"	=> std::mem::take(&mut new_loca),
				b"head"	=> std::mem::take(&mut new_head),
				b"hmtx"	=> std::mem::take(&mut new_hmtx),
				_		=> match self.table(tag) {
					Some(t)	=> t.to_vec(),
					None	=> continue,
				},
			};
			out_tables.push((*tag, body));
		}
		Ok(write_sfnt(0x0001_0000, out_tables))
	}
}

/// The sum of a table's big-endian words, zero padded, as `sfnt` checksums are taken.
fn checksum(b: &[u8]) -> u32 {
	let mut sum = 0u32;
	for chunk in b.chunks(4) {
		let mut w = [0u8; 4];
		w[..chunk.len()].copy_from_slice(chunk);
		sum = sum.wrapping_add(u32::from_be_bytes(w));
	}
	sum
}

/// Serialises an `sfnt` from tables already sorted by tag, filling in the directory, the checksums and
/// `head`'s whole-file adjustment.
fn write_sfnt(version: u32, tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
	let n = tables.len();
	let mut pow = 1usize;
	let mut sel = 0u16;
	while pow * 2 <= n {
		pow *= 2;
		sel += 1;
	}
	let range = (pow * 16) as u16;
	let mut out = Vec::new();
	out.extend_from_slice(&version.to_be_bytes());
	out.extend_from_slice(&(n as u16).to_be_bytes());
	out.extend_from_slice(&range.to_be_bytes());
	out.extend_from_slice(&sel.to_be_bytes());
	out.extend_from_slice(&((n * 16) as u16).wrapping_sub(range).to_be_bytes());

	let mut off = 12 + 16 * n;
	let mut head_at = None;
	for (tag, body) in &tables {
		out.extend_from_slice(tag);
		out.extend_from_slice(&checksum(body).to_be_bytes());
		out.extend_from_slice(&(off as u32).to_be_bytes());
		out.extend_from_slice(&(body.len() as u32).to_be_bytes());
		if tag == b"head" {
			head_at = Some(off);
		}
		off += (body.len() + 3) & !3;
	}
	for (_, body) in &tables {
		out.extend_from_slice(body);
		while out.len() % 4 != 0 {
			out.push(0);
		}
	}
	if let Some(h) = head_at {
		let adj = 0xb1b0_afbau32.wrapping_sub(checksum(&out));
		if let Some(slot) = out.get_mut(h + 8..h + 12) {
			slot.copy_from_slice(&adj.to_be_bytes());
		}
	}
	out
}

/// The PostScript name (name id 6), preferring the Windows Unicode record, reduced to the characters
/// a PDF name may carry unescaped.
fn postscript_name(d: &[u8], t: Table) -> Option<String> {
	let count	= rd_u16(d, t.off + 2).ok()? as usize;
	let store	= t.off + rd_u16(d, t.off + 4).ok()? as usize;
	let mut best: Option<String> = None;
	for i in 0..count {
		let rec = t.off + 6 + 12 * i;
		let platform	= rd_u16(d, rec).ok()?;
		let name_id		= rd_u16(d, rec + 6).ok()?;
		let len			= rd_u16(d, rec + 8).ok()? as usize;
		let off			= rd_u16(d, rec + 10).ok()? as usize;
		if name_id != 6 {
			continue;
		}
		let raw = slice(d, store + off, len).ok()?;
		let s: String = match platform {
			0 | 3 => {
				let units: Vec<u16> = raw.chunks(2)
					.filter(|c| c.len() == 2)
					.map(|c| ((c[0] as u16) << 8) | c[1] as u16)
					.collect();
				String::from_utf16_lossy(&units)
			},
			_ => raw.iter().map(|&b| b as char).collect(),
		};
		let clean: String = s.chars()
			.filter(|c| c.is_ascii_graphic() && !"()<>[]{}/%#".contains(*c))
			.take(63)
			.collect();
		if !clean.is_empty() {
			if platform == 3 {
				return Some(clean);
			}
			if best.is_none() {
				best = Some(clean);
			}
		}
	}
	best
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CFF                                                                        │
// └───────────────────────────────────────────────────────────────────────────┘

// DICT operators this rewriter touches; a two-byte operator `12 x` is `1200 + x`.
const OP_CHARSET:		u16 = 15;
const OP_ENCODING:		u16 = 16;
const OP_CHARSTRINGS:	u16 = 17;
const OP_PRIVATE:		u16 = 18;
const OP_SUBRS:			u16 = 19;
const OP_ROS:			u16 = 1230;
const OP_CIDCOUNT:		u16 = 1234;
const OP_FDARRAY:		u16 = 1236;
const OP_FDSELECT:		u16 = 1237;
// The dict operators whose operand is a string id: version, Notice, FullName, FamilyName, Weight,
// Copyright, PostScript, BaseFontName and FontName.
const SID_OPS:			[u16; 9] = [0, 1, 2, 3, 4, 1200, 1221, 1222, 1238];

/// Reads a CFF INDEX at `at`: its items and the offset just past it.
fn read_index(d: &[u8], at: usize) -> Outcome<(Vec<&[u8]>, usize)> {
	let count = res!(rd_u16(d, at)) as usize;
	if count == 0 {
		return Ok((Vec::new(), at + 2));
	}
	let osz = res!(rd_u8(d, at + 2)) as usize;
	if osz == 0 || osz > 4 {
		return Err(err!("A CFF INDEX at {} declares offset size {}.", at, osz; Invalid, Input));
	}
	let offs_at	= at + 3;
	let data_at	= offs_at + (count + 1) * osz - 1;	// offsets are one-based
	let mut offs = Vec::with_capacity(count + 1);
	for i in 0..=count {
		offs.push(res!(rd_uint(d, offs_at + i * osz, osz)));
	}
	let mut items = Vec::with_capacity(count);
	for i in 0..count {
		if offs[i + 1] < offs[i] {
			return Err(err!("A CFF INDEX at {} has a decreasing offset at item {}.", at, i;
				Invalid, Input));
		}
		items.push(res!(slice(d, data_at + offs[i], offs[i + 1] - offs[i])));
	}
	Ok((items, data_at + offs[count]))
}

fn write_index<T: AsRef<[u8]>>(items: &[T]) -> Vec<u8> {
	let mut out = Vec::new();
	out.extend_from_slice(&(items.len() as u16).to_be_bytes());
	if items.is_empty() {
		return out;
	}
	let total: usize = items.iter().map(|i| i.as_ref().len()).sum::<usize>() + 1;
	let osz = if total < 0x100 { 1 } else if total < 0x1_0000 { 2 } else if total < 0x100_0000 { 3 } else { 4 };
	out.push(osz as u8);
	let mut off = 1usize;
	let put = |out: &mut Vec<u8>, v: usize| {
		for k in (0..osz).rev() {
			out.push((v >> (8 * k)) as u8);
		}
	};
	put(&mut out, off);
	for it in items {
		off += it.as_ref().len();
		put(&mut out, off);
	}
	for it in items {
		out.extend_from_slice(it.as_ref());
	}
	out
}

/// One DICT entry: its operator, its operands' raw bytes (re-emitted untouched when the entry is kept)
/// and their integer values (the only kind this rewriter needs to read).
struct DictEntry {
	op:		u16,
	raw:	Vec<u8>,
	ints:	Vec<i64>,
}

fn parse_dict(d: &[u8]) -> Outcome<Vec<DictEntry>> {
	let mut out = Vec::new();
	let mut raw = Vec::new();
	let mut ints = Vec::new();
	let mut i = 0;
	while i < d.len() {
		let b = d[i];
		let start = i;
		match b {
			0..=21 => {
				let op = if b == 12 {
					i += 1;
					1200 + res!(rd_u8(d, i)) as u16
				} else {
					b as u16
				};
				i += 1;
				out.push(DictEntry { op, raw: std::mem::take(&mut raw), ints: std::mem::take(&mut ints) });
				continue;
			},
			28 => {
				ints.push(res!(rd_i16(d, i + 1)) as i64);
				i += 3;
			},
			29 => {
				ints.push(res!(rd_u32(d, i + 1)) as i32 as i64);
				i += 5;
			},
			30 => {
				// A real: nibbles to the terminator 0xf. Its value is never an offset, so it reads as 0.
				i += 1;
				loop {
					let n = res!(rd_u8(d, i));
					i += 1;
					if n & 0x0f == 0x0f || n >> 4 == 0x0f {
						break;
					}
				}
				ints.push(0);
			},
			32..=246 => {
				ints.push(b as i64 - 139);
				i += 1;
			},
			247..=250 => {
				ints.push((b as i64 - 247) * 256 + res!(rd_u8(d, i + 1)) as i64 + 108);
				i += 2;
			},
			251..=254 => {
				ints.push(-(b as i64 - 251) * 256 - res!(rd_u8(d, i + 1)) as i64 - 108);
				i += 2;
			},
			_ => return Err(err!("A CFF DICT holds the reserved byte {} at {}.", b, i; Invalid, Input)),
		}
		raw.extend_from_slice(res!(slice(d, start, i - start)));
	}
	Ok(out)
}

fn int5(v: usize) -> [u8; 5] {
	let b = (v as u32).to_be_bytes();
	[29, b[0], b[1], b[2], b[3]]
}

fn put_op(out: &mut Vec<u8>, op: u16) {
	if op >= 1200 {
		out.push(12);
		out.push((op - 1200) as u8);
	} else {
		out.push(op as u8);
	}
}

fn entry_int(entries: &[DictEntry], op: u16, k: usize) -> Option<i64> {
	entries.iter().find(|e| e.op == op).and_then(|e| e.ints.get(k).copied())
}

/// The bias a charstring adds to a subroutine number, by the size of the INDEX it calls into.
fn bias(n: usize) -> i64 {
	if n < 1240 { 107 } else if n < 33900 { 1131 } else { 32768 }
}

/// Flattens a Type 2 charstring: every subroutine call is replaced by the body it calls, so the subset
/// needs no subroutines at all. For the few dozen glyphs a document shows this is far smaller than the
/// thousands of shared subroutines a whole font carries. The operand stack is followed only as far as
/// the flattening needs: its depth, for the stem count a `hintmask` sizes its mask by, and the byte where
/// the last operand began, so a call's subroutine number can be cut back out.
struct Flatten<'a> {
	gsubrs:	&'a [&'a [u8]],
	lsubrs:	&'a [&'a [u8]],
	out:	Vec<u8>,
	stems:	usize,
	stack:	usize,	// operand depth
	last:	i64,	// the top operand, the subroutine number a call pops
	last_at:	usize,	// where in `out` the top operand's bytes begin
}

enum Flow {
	Return,
	End,
}

impl<'a> Flatten<'a> {

	fn new(gsubrs: &'a [&'a [u8]], lsubrs: &'a [&'a [u8]]) -> Self {
		Self { gsubrs, lsubrs, out: Vec::new(), stems: 0, stack: 0, last: 0, last_at: 0 }
	}

	fn operand(&mut self, cs: &[u8], i: usize, len: usize, v: i64) -> Outcome<()> {
		self.last		= v;
		self.last_at	= self.out.len();
		self.stack		+= 1;
		self.out.extend_from_slice(res!(slice(cs, i, len)));
		Ok(())
	}

	fn run(&mut self, cs: &[u8], depth: usize) -> Outcome<Flow> {
		if depth > 10 {
			return Err(err!("Charstring subroutines nest past the limit of ten."; Invalid, Input));
		}
		let mut i = 0;
		while i < cs.len() {
			let b = cs[i];
			match b {
				28 => {
					let v = res!(rd_i16(cs, i + 1)) as i64;
					res!(self.operand(cs, i, 3, v));
					i += 3;
				},
				32..=246 => {
					res!(self.operand(cs, i, 1, b as i64 - 139));
					i += 1;
				},
				247..=250 => {
					let v = (b as i64 - 247) * 256 + res!(rd_u8(cs, i + 1)) as i64 + 108;
					res!(self.operand(cs, i, 2, v));
					i += 2;
				},
				251..=254 => {
					let v = -(b as i64 - 251) * 256 - res!(rd_u8(cs, i + 1)) as i64 - 108;
					res!(self.operand(cs, i, 2, v));
					i += 2;
				},
				255 => {
					let v = (res!(rd_u32(cs, i + 1)) as i32 >> 16) as i64;
					res!(self.operand(cs, i, 5, v));
					i += 5;
				},
				1 | 3 | 18 | 23 => {	// hstem, vstem, hstemhm, vstemhm
					self.stems += self.stack / 2;
					self.stack = 0;
					self.out.push(b);
					i += 1;
				},
				19 | 20 => {			// hintmask, cntrmask: an implicit vstem, then the mask bytes
					self.stems += self.stack / 2;
					self.stack = 0;
					let n = 1 + (self.stems + 7) / 8;
					self.out.extend_from_slice(res!(slice(cs, i, n)));
					i += n;
				},
				10 | 29 => {			// callsubr, callgsubr
					if self.stack == 0 {
						return Err(err!("A subroutine call has no operand."; Invalid, Input));
					}
					self.stack -= 1;
					self.out.truncate(self.last_at);
					let subrs = if b == 29 { self.gsubrs } else { self.lsubrs };
					let idx = self.last + bias(subrs.len());
					let body = match usize::try_from(idx).ok().and_then(|k| subrs.get(k)) {
						Some(s)	=> *s,
						None	=> return Err(err!(
							"A charstring calls subroutine {} of {}.", idx, subrs.len(); Invalid, Input)),
					};
					i += 1;
					if let Flow::End = res!(self.run(body, depth + 1)) {
						return Ok(Flow::End);
					}
				},
				11 => return Ok(Flow::Return),
				14 => {
					// Four operands (five with a width) is the deprecated `seac`, which names its accent and
					// base by standard-encoding code -- a name-keyed idea with no meaning in a CID font.
					if self.stack >= 4 {
						return Err(err!("The glyph is an accented seac composite, which a CID-keyed \
							subset cannot express."; Invalid, Input, Unimplemented));
					}
					self.out.push(14);
					return Ok(Flow::End);
				},
				12 => {
					self.stack = 0;
					self.out.extend_from_slice(res!(slice(cs, i, 2)));
					i += 2;
				},
				_ => {
					self.stack = 0;
					self.out.push(b);
					i += 1;
				},
			}
		}
		Ok(Flow::Return)
	}
}

/// Cuts a CFF table down to `keep` as a CID-keyed font with no subroutines. The kept glyphs are packed
/// into consecutive glyph slots and the charset maps each slot back to its original glyph id as its CID,
/// so a PDF still shows a glyph by the id the shaper gave it.
fn subset_cff(d: &[u8], keep: &BTreeSet<u16>, num_glyphs: usize) -> Outcome<Vec<u8>> {
	let hdr_size = res!(rd_u8(d, 2)) as usize;
	let (names, at)		= res!(read_index(d, hdr_size));
	let (tops, at)		= res!(read_index(d, at));
	let (_strings, at)	= res!(read_index(d, at));
	let (gsubrs, _)		= res!(read_index(d, at));
	let top_raw = match tops.first() {
		Some(t)	=> *t,
		None	=> return Err(err!("The CFF table holds no Top DICT."; Invalid, Input, Missing)),
	};
	let top = res!(parse_dict(top_raw));

	let cs_off = match entry_int(&top, OP_CHARSTRINGS, 0) {
		Some(o) if o > 0	=> o as usize,
		_					=> return Err(err!("The CFF Top DICT names no CharStrings."; Invalid, Input)),
	};
	let (charstrings, _) = res!(read_index(d, cs_off));
	if charstrings.len() != num_glyphs {
		return Err(err!("The CFF holds {} charstrings but maxp counts {} glyphs.",
			charstrings.len(), num_glyphs; Invalid, Input));
	}

	// The font dicts, each with its private dict and local subroutines, and which glyph uses which.
	struct Fd<'a> {
		dict:		Vec<DictEntry>,	// the font dict's own entries, Private excluded
		private:	Vec<DictEntry>,	// Subrs excluded: the flattened glyphs call none
		subrs:		Vec<&'a [u8]>,
	}
	let read_private = |entries: &[DictEntry]| -> Outcome<(Vec<DictEntry>, Vec<&[u8]>)> {
		let size	= entry_int(entries, OP_PRIVATE, 0).unwrap_or(0).max(0) as usize;
		let off		= entry_int(entries, OP_PRIVATE, 1).unwrap_or(0).max(0) as usize;
		if size == 0 {
			return Ok((Vec::new(), Vec::new()));
		}
		let pd = res!(parse_dict(res!(slice(d, off, size))));
		let subrs = match entry_int(&pd, OP_SUBRS, 0) {
			Some(rel) if rel > 0	=> res!(read_index(d, off + rel as usize)).0,
			_						=> Vec::new(),
		};
		Ok((pd.into_iter().filter(|e| e.op != OP_SUBRS).collect(), subrs))
	};

	let mut fds: Vec<Fd> = Vec::new();
	let mut fd_of: Vec<u8> = vec![0; num_glyphs];
	if top.iter().any(|e| e.op == OP_ROS) {
		let fda_off = match entry_int(&top, OP_FDARRAY, 0) {
			Some(o) if o > 0	=> o as usize,
			_					=> return Err(err!("A CID-keyed CFF names no FDArray."; Invalid, Input)),
		};
		for raw in res!(read_index(d, fda_off)).0 {
			let fd = res!(parse_dict(raw));
			let (private, subrs) = res!(read_private(&fd));
			fds.push(Fd { dict: fd.into_iter().filter(|e| e.op != OP_PRIVATE).collect(), private, subrs });
		}
		let sel = match entry_int(&top, OP_FDSELECT, 0) {
			Some(o) if o > 0	=> o as usize,
			_					=> return Err(err!("A CID-keyed CFF names no FDSelect."; Invalid, Input)),
		};
		match res!(rd_u8(d, sel)) {
			0 => for g in 0..num_glyphs {
				fd_of[g] = res!(rd_u8(d, sel + 1 + g));
			},
			3 => {
				let n = res!(rd_u16(d, sel + 1)) as usize;
				for r in 0..n {
					let first	= res!(rd_u16(d, sel + 3 + 3 * r)) as usize;
					let fd		= res!(rd_u8(d, sel + 5 + 3 * r));
					let end		= res!(rd_u16(d, sel + 6 + 3 * r)) as usize;
					for g in first..end.min(num_glyphs) {
						fd_of[g] = fd;
					}
				}
			},
			f => return Err(err!("FDSelect format {} is not one CFF defines.", f; Invalid, Input)),
		}
	} else {
		let (private, subrs) = res!(read_private(&top));
		fds.push(Fd { dict: Vec::new(), private, subrs });
	}

	// The kept glyphs, flattened, in glyph-id order: slot k holds CID keep[k], and slot 0 is glyph 0.
	let cids: Vec<u16> = keep.iter().copied().collect();
	let mut new_cs: Vec<Vec<u8>> = Vec::with_capacity(cids.len());
	let mut slot_fd: Vec<u8> = Vec::with_capacity(cids.len());
	for &g in &cids {
		let fd = fd_of.get(g as usize).copied().unwrap_or(0);
		let lsubrs: &[&[u8]] = match fds.get(fd as usize) {
			Some(f)	=> &f.subrs,
			None	=> return Err(err!("Glyph {} names font dict {} of {}.", g, fd, fds.len(); Invalid, Input)),
		};
		let mut flat = Flatten::new(&gsubrs, lsubrs);
		if let Err(e) = flat.run(charstrings[g as usize], 0) {
			return Err(err!(e, "Glyph {} could not be flattened.", g; Invalid, Input));
		}
		new_cs.push(flat.out);
		slot_fd.push(fd);
	}

	// Strings: only the registry and ordering of the identity ROS. The originals are mostly glyph names,
	// which a CID font has no use for; the few a dict names -- a notice, a family name -- are dropped
	// with the entries that name them.
	let new_strings: Vec<&[u8]> = vec![b"Adobe", b"Identity"];
	let sid_adobe		= 391;
	let sid_identity	= 392;

	// Charset format 0: each slot past the first names its CID.
	let mut charset = vec![0u8];
	for &cid in cids.iter().skip(1) {
		charset.extend_from_slice(&cid.to_be_bytes());
	}
	// FDSelect format 3: one range per run of slots sharing a font dict.
	let mut ranges: Vec<(u16, u8)> = Vec::new();
	for (k, &fd) in slot_fd.iter().enumerate() {
		if ranges.last().map_or(true, |r| r.1 != fd) {
			ranges.push((k as u16, fd));
		}
	}
	let mut fdselect = vec![3u8];
	fdselect.extend_from_slice(&(ranges.len() as u16).to_be_bytes());
	for (first, fd) in &ranges {
		fdselect.extend_from_slice(&first.to_be_bytes());
		fdselect.push(*fd);
	}
	fdselect.extend_from_slice(&(cids.len() as u16).to_be_bytes());

	let privates: Vec<Vec<u8>> = fds.iter().map(|fd| {
		let mut p = Vec::new();
		for e in &fd.private {
			p.extend_from_slice(&e.raw);
			put_op(&mut p, e.op);
		}
		p
	}).collect();

	// Two passes: sizes with placeholder offsets, then the same layout with the real ones. Every offset
	// is a five-byte integer, so the sizes cannot change between the passes.
	let build_top = |charset_at: usize, cs_at: usize, fda_at: usize, sel_at: usize| -> Vec<u8> {
		let mut t = Vec::new();
		t.extend_from_slice(&int5(sid_adobe));
		t.extend_from_slice(&int5(sid_identity));
		t.push(139);	// supplement 0
		put_op(&mut t, OP_ROS);
		for e in &top {
			if matches!(e.op, OP_CHARSET | OP_ENCODING | OP_CHARSTRINGS | OP_PRIVATE | OP_ROS
				| OP_CIDCOUNT | OP_FDARRAY | OP_FDSELECT) || SID_OPS.contains(&e.op)
			{
				continue;
			}
			t.extend_from_slice(&e.raw);
			put_op(&mut t, e.op);
		}
		t.extend_from_slice(&int5(num_glyphs));
		put_op(&mut t, OP_CIDCOUNT);
		t.extend_from_slice(&int5(charset_at));
		put_op(&mut t, OP_CHARSET);
		t.extend_from_slice(&int5(cs_at));
		put_op(&mut t, OP_CHARSTRINGS);
		t.extend_from_slice(&int5(fda_at));
		put_op(&mut t, OP_FDARRAY);
		t.extend_from_slice(&int5(sel_at));
		put_op(&mut t, OP_FDSELECT);
		t
	};
	let build_fda = |priv_at: &[usize]| -> Vec<u8> {
		let dicts: Vec<Vec<u8>> = fds.iter().enumerate().map(|(k, fd)| {
			let mut f = Vec::new();
			for e in fd.dict.iter().filter(|e| !SID_OPS.contains(&e.op)) {
				f.extend_from_slice(&e.raw);
				put_op(&mut f, e.op);
			}
			f.extend_from_slice(&int5(privates.get(k).map_or(0, |p| p.len())));
			f.extend_from_slice(&int5(priv_at.get(k).copied().unwrap_or(0)));
			put_op(&mut f, OP_PRIVATE);
			f
		}).collect();
		write_index(&dicts)
	};

	let no_subrs: [&[u8]; 0] = [];
	let name_idx	= write_index(&names);
	let string_idx	= write_index(&new_strings);
	let gsubr_idx	= write_index(&no_subrs);
	let cs_idx		= write_index(&new_cs);
	let top_len		= write_index(&[build_top(0, 0, 0, 0)]).len();
	let fda_len		= build_fda(&vec![0; fds.len()]).len();

	let charset_at	= 4 + name_idx.len() + top_len + string_idx.len() + gsubr_idx.len();
	let sel_at		= charset_at + charset.len();
	let cs_at		= sel_at + fdselect.len();
	let fda_at		= cs_at + cs_idx.len();
	let mut priv_at	= Vec::with_capacity(privates.len());
	let mut next	= fda_at + fda_len;
	for p in &privates {
		priv_at.push(next);
		next += p.len();
	}

	let mut out = Vec::with_capacity(next);
	out.extend_from_slice(&[1, 0, 4, 4]);
	out.extend_from_slice(&name_idx);
	out.extend_from_slice(&write_index(&[build_top(charset_at, cs_at, fda_at, sel_at)]));
	out.extend_from_slice(&string_idx);
	out.extend_from_slice(&gsubr_idx);
	out.extend_from_slice(&charset);
	out.extend_from_slice(&fdselect);
	out.extend_from_slice(&cs_idx);
	out.extend_from_slice(&build_fda(&priv_at));
	for p in &privates {
		out.extend_from_slice(p);
	}
	if out.len() != next {
		return Err(err!("The rewritten CFF is {} bytes where its layout planned {}.", out.len(), next; Bug));
	}
	Ok(out)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_an_index_round_trips_00() -> Outcome<()> {
		let items: Vec<&[u8]> = vec![b"abc", b"", b"defg"];
		let bytes = write_index(&items);
		let (back, end) = res!(read_index(&bytes, 0));
		assert_eq!(back, items);
		assert_eq!(end, bytes.len());
		Ok(())
	}

	#[test]
	fn test_a_dict_keeps_its_operands_01() -> Outcome<()> {
		// 100 200 Private, then 12 30 (ROS) with three small ints.
		let mut d = Vec::new();
		d.extend_from_slice(&[239, 247, 92, 18]);	// 100, 200, Private
		d.extend_from_slice(&[140, 141, 139, 12, 30]);
		let e = res!(parse_dict(&d));
		assert_eq!(e.len(), 2);
		assert_eq!(e[0].op, OP_PRIVATE);
		assert_eq!(e[0].ints, vec![100, 200]);
		assert_eq!(e[1].op, OP_ROS);
		assert_eq!(e[1].ints, vec![1, 2, 0]);
		Ok(())
	}
}
