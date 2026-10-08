//! A reader of ICC profiles: the header, the tag table, and the handful of tags a colour transform
//! is built from.
//!
//! Every read is checked against the tag's declared offset and size, and every tag against the
//! length of the file, before a byte is taken or a table allocated. A profile that is cut short, or
//! whose tag directory points past its own end, is refused with a message naming the tag and the
//! profile; nothing is read from a neighbour's bytes instead.
//!
//! # What is read
//!
//! The header; the tag table; and, of the tags in it, `desc`, `wtpt`, `chad`, the colorants
//! `rXYZ gXYZ bXYZ`, the curves `rTRC gTRC bTRC kTRC` (`curv`, and `para` of a version 4 profile),
//! and the lookup tables `A2B0 A2B1 A2B2` and `B2A0 B2A1 B2A2` of types `mft1` and `mft2`. The tables
//! are kept as the integers the file holds, with the shape that says how to walk them; this module
//! does no interpolation.
//!
//! # What is refused
//!
//! The multi-process-element tables `mAB` and `mBA` of a version 4 profile, which this reader does
//! not read; a truncated profile or tag; a lookup table whose channel counts do not fit the
//! profile's data space and connection space; a profile of a version other than 2 or 4. A profile
//! that is refused is never replaced by another.
//!
//! # References
//!
//! ICC.1:2004-10 (version 4.2) for the header (§7.2), the tag table (§7.3) and the tag types
//! (§10); ICC.1:2001-04 (version 2.4) §6.5.5 and §6.5.6 for the `lut8` and `lut16` layouts and
//! the legacy 16-bit Lab encoding (annex A.3).
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

// Header and tag table geometry.
const HEAD_LEN:		usize = 128;	// the profile header
const ENTRY_LEN:	usize = 12;		// signature, offset, size
const MAX_CH:		usize = 4;		// the widest lookup table a colour transform here uses

pub type Sig = [u8; 4];

/// FNV-1a over `bytes`, 64 bits: a stable key for a cache, not a defence against forgery.
pub fn fnv64(bytes: &[u8]) -> u64 {
	let mut h = 0xcbf2_9ce4_8422_2325u64;
	for b in bytes {
		h ^= *b as u64;
		h = h.wrapping_mul(0x0000_0100_0000_01b3);
	}
	h
}

/// Names a signature for a message, with any byte outside printable ASCII shown as `?`.
pub fn sig_str(sig: Sig) -> String {
	sig.iter().map(|b| if (0x20..0x7f).contains(b) { *b as char } else { '?' }).collect()
}

// What the profile is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
	Input,
	Display,
	Output,
	Link,
	Abstract,
	Space,
	Named,
	Other(Sig),
}

impl Class {
	fn from_sig(sig: Sig) -> Self {
		match &sig {
			b"scnr"	=> Self::Input,
			b"mntr"	=> Self::Display,
			b"prtr"	=> Self::Output,
			b"link"	=> Self::Link,
			b"abst"	=> Self::Abstract,
			b"spac"	=> Self::Space,
			b"nmcl"	=> Self::Named,
			_		=> Self::Other(sig),
		}
	}
}

// A colour space: the profile's own, or the connection space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
	Xyz,
	Lab,
	Rgb,
	Gray,
	Cmyk,
	Other(Sig),
}

impl Space {
	fn from_sig(sig: Sig) -> Self {
		match &sig {
			b"XYZ "	=> Self::Xyz,
			b"Lab "	=> Self::Lab,
			b"RGB "	=> Self::Rgb,
			b"GRAY"	=> Self::Gray,
			b"CMYK"	=> Self::Cmyk,
			_		=> Self::Other(sig),
		}
	}

	/// The number of channels a colour in this space has, where this reader knows it.
	pub fn channels(&self) -> Option<usize> {
		match self {
			Self::Xyz | Self::Lab | Self::Rgb	=> Some(3),
			Self::Gray							=> Some(1),
			Self::Cmyk							=> Some(4),
			Self::Other(_)						=> None,
		}
	}
}

#[derive(Clone, Debug)]
pub struct Header {
	pub size:	u32,			// the profile's length in bytes, as it declares it
	pub major:	u8,
	pub minor:	u8,
	pub class:	Class,
	pub space:	Space,			// the data colour space
	pub pcs:	Space,			// the profile connection space
	pub intent:	u32,			// the rendering intent the profile prefers
	pub illum:	[f64; 3],		// the PCS illuminant, D50 in every profile in use
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TagEntry {
	pub sig:	Sig,
	pub off:	usize,
	pub len:	usize,
}

// A tone curve from 0..1 to 0..1.
#[derive(Clone, Debug, PartialEq)]
pub enum Curve {
	Identity,
	Gamma(f64),							// curv with one entry, u8Fixed8
	Table(Vec<u16>),					// curv with two or more entries, sampled evenly
	Para { func: u16, p: [f64; 7] },	// para, parameters g a b c d e f in that order
}

impl Curve {
	/// The curve's value at `x`, which is clamped to `0..=1` as the result is.
	pub fn eval(&self, x: f64) -> f64 {
		let x = x.clamp(0.0, 1.0);
		let y = match self {
			Self::Identity		=> x,
			Self::Gamma(g)		=> x.powf(*g),
			Self::Table(t) if t.len() < 2	=> x,	// the reader never makes one
			Self::Table(t)		=> {
				let at = x * (t.len() - 1) as f64;
				let i = (at.floor() as usize).min(t.len() - 2);
				let f = at - i as f64;
				let lo = t[i] as f64 / 65535.0;
				let hi = t[i + 1] as f64 / 65535.0;
				lo + (hi - lo) * f
			},
			Self::Para { func, p }	=> {
				let (g, a, b, c, d, e, f) = (p[0], p[1], p[2], p[3], p[4], p[5], p[6]);
				let base = (a * x + b).max(0.0);
				match func {
					0	=> x.powf(g),
					1	=> if x >= -b / a { base.powf(g) } else { 0.0 },
					2	=> if x >= -b / a { base.powf(g) + c } else { c },
					3	=> if x >= d { base.powf(g) } else { c * x },
					_	=> if x >= d { base.powf(g) + e } else { c * x + f },
				}
			},
		};
		y.clamp(0.0, 1.0)
	}

	/// Does the curve end no lower than it starts?
	///
	/// A tone reproduction curve that falls has no inverse worth taking.
	pub fn rises(&self) -> bool {
		self.eval(0.0) <= self.eval(1.0)
	}

	/// The `x` with `eval(x) = y`, found by bisection on a curve that [`Curve::rises`].
	pub fn inverse(&self, y: f64) -> f64 {
		let y = y.clamp(0.0, 1.0);
		let (mut lo, mut hi) = (0.0f64, 1.0f64);
		for _ in 0..60 {
			let mid = 0.5 * (lo + hi);
			if self.eval(mid) < y { lo = mid; } else { hi = mid; }
		}
		0.5 * (lo + hi)
	}
}

// A lut8 or lut16 table: input curves, a lattice, output curves.
#[derive(Clone, Debug)]
pub struct Lut {
	pub bits:	u8,			// 8 for mft1, 16 for mft2
	pub nin:	usize,
	pub nout:	usize,
	pub grid:	usize,		// lattice nodes along each input axis
	pub mat:	[f64; 9],	// row major; applies to XYZ input only
	pub lab_in:	bool,		// is the input side Lab, which LittleCMS interpolates trilinearly
	pub ine:	usize,		// entries in each input curve
	pub oute:	usize,		// entries in each output curve
	pub ins:	Vec<u16>,	// nin curves of ine entries
	pub clut:	Vec<u16>,	// grid^nin nodes of nout values, the first input slowest
	pub outs:	Vec<u16>,	// nout curves of oute entries
}

impl Lut {
	/// The largest value a table entry can hold, 255 or 65535.
	pub fn max(&self) -> f64 {
		if self.bits == 8 { 255.0 } else { 65535.0 }
	}

	/// The `nout` values of the lattice node at `idx`, one index per input, the first input slowest.
	pub fn node(&self, idx: &[usize]) -> Outcome<&[u16]> {
		if idx.len() != self.nin {
			return Err(err!(
				"A lookup table of {} inputs was asked for a node of {} indices.",
				self.nin, idx.len(); Invalid, Input, Mismatch));
		}
		let mut at = 0usize;
		for i in idx {
			if *i >= self.grid {
				return Err(err!(
					"Index {} is beyond the {} nodes of a lookup table axis.", i, self.grid;
					Invalid, Input, Index));
			}
			at = at * self.grid + *i;
		}
		let a = at * self.nout;
		match self.clut.get(a..a + self.nout) {
			Some(s)	=> Ok(s),
			None	=> Err(err!(
				"Node {} lies outside a lookup table of {} entries.", at, self.clut.len();
				Invalid, Input, Index)),
		}
	}

	/// A Lab colour in the version 2 encoding of this table, as L*, a*, b*.
	///
	/// A lut16 holds the legacy encoding, where L* of 100 is 0xFF00 and a* or b* of zero is 0x8000;
	/// a lut8 holds L* of 100 as 255 and a* or b* of zero as 128.
	pub fn lab(&self, v: [u16; 3]) -> [f64; 3] {
		let (l, a, b) = (v[0] as f64, v[1] as f64, v[2] as f64);
		if self.bits == 8 {
			[l * 100.0 / 255.0, a - 128.0, b - 128.0]
		} else {
			[l * 100.0 / 65280.0, a / 256.0 - 128.0, b / 256.0 - 128.0]
		}
	}
}

#[derive(Clone, Debug)]
pub struct Profile {
	pub name:	String,					// what the caller calls it, quoted in every message
	pub id:		u64,					// FNV-1a of the profile's bytes, the identity a transform is cached by
	pub head:	Header,
	pub tags:	Vec<TagEntry>,
	pub desc:	Option<String>,
	pub wtpt:	Option<[f64; 3]>,
	pub chad:	Option<[f64; 9]>,		// row major
	pub cols:	[Option<[f64; 3]>; 3],	// rXYZ gXYZ bXYZ
	pub trcs:	[Option<Curve>; 3],		// rTRC gTRC bTRC
	pub ktrc:	Option<Curve>,
	pub a2b:	[Option<Lut>; 3],		// A2B0 A2B1 A2B2
	pub b2a:	[Option<Lut>; 3],		// B2A0 B2A1 B2A2
}

// A view of one tag's bytes, whose every read is checked against its length.
struct Rd<'a> {
	b:		&'a [u8],
	what:	&'a str,
}

impl<'a> Rd<'a> {
	fn take(&self, at: usize, n: usize) -> Outcome<&'a [u8]> {
		match at.checked_add(n).and_then(|end| self.b.get(at..end)) {
			Some(s)	=> Ok(s),
			None	=> Err(err!(
				"The {} is {} bytes and is truncated: a read of {} bytes at byte {} runs past \
				its end.", self.what, self.b.len(), n, at; Invalid, Input, Decode, Size)),
		}
	}

	fn sig(&self, at: usize) -> Outcome<Sig> {
		let s = res!(self.take(at, 4));
		Ok([s[0], s[1], s[2], s[3]])
	}

	fn u8(&self, at: usize) -> Outcome<u8> {
		Ok(res!(self.take(at, 1))[0])
	}

	fn u16(&self, at: usize) -> Outcome<u16> {
		let s = res!(self.take(at, 2));
		Ok(u16::from_be_bytes([s[0], s[1]]))
	}

	fn u32(&self, at: usize) -> Outcome<u32> {
		let s = res!(self.take(at, 4));
		Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
	}

	// An s15Fixed16 number.
	fn s15(&self, at: usize) -> Outcome<f64> {
		let s = res!(self.take(at, 4));
		Ok(i32::from_be_bytes([s[0], s[1], s[2], s[3]]) as f64 / 65536.0)
	}

	fn xyz(&self, at: usize) -> Outcome<[f64; 3]> {
		Ok([res!(self.s15(at)), res!(self.s15(at + 4)), res!(self.s15(at + 8))])
	}

	// The 16-bit big-endian run of `n` values at `at`.
	fn run16(&self, at: usize, n: usize) -> Outcome<Vec<u16>> {
		let len = res!(n.checked_mul(2).ok_or_else(|| err!(
			"The {} asks for {} 16-bit values, more than a profile can hold.", self.what, n;
			Invalid, Input, Decode, Size)));
		let s = res!(self.take(at, len));
		Ok(s.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect())
	}

	fn run8(&self, at: usize, n: usize) -> Outcome<Vec<u16>> {
		let s = res!(self.take(at, n));
		Ok(s.iter().map(|b| *b as u16).collect())
	}
}

impl Profile {
	/// Reads a profile from `data`; `name` is what every message calls it.
	pub fn read(name: &str, data: &[u8]) -> Outcome<Self> {
		let head = res!(read_header(name, data));
		let size = head.size as usize;
		let whole = match data.get(..size) {
			Some(s)	=> s,
			None	=> return Err(err!(
				"Profile '{}' is truncated: its header declares {} bytes and {} are present.",
				name, size, data.len(); Invalid, Input, Decode, Size)),
		};
		let tags = res!(read_table(name, whole));

		let mut p = Self {
			name:	name.to_string(),
			id:		fnv64(whole),
			head,
			tags,
			desc:	None,
			wtpt:	None,
			chad:	None,
			cols:	[None, None, None],
			trcs:	[None, None, None],
			ktrc:	None,
			a2b:	[None, None, None],
			b2a:	[None, None, None],
		};
		p.desc = res!(with_tag(name, whole, &p.tags, *b"desc", read_text)).flatten();
		p.wtpt = res!(with_tag(name, whole, &p.tags, *b"wtpt", read_xyz));
		p.chad = res!(with_tag(name, whole, &p.tags, *b"chad", read_chad));
		for (i, sig) in [*b"rXYZ", *b"gXYZ", *b"bXYZ"].into_iter().enumerate() {
			p.cols[i] = res!(with_tag(name, whole, &p.tags, sig, read_xyz));
		}
		for (i, sig) in [*b"rTRC", *b"gTRC", *b"bTRC"].into_iter().enumerate() {
			p.trcs[i] = res!(with_tag(name, whole, &p.tags, sig, read_curve));
		}
		p.ktrc = res!(with_tag(name, whole, &p.tags, *b"kTRC", read_curve));
		for (i, sig) in [*b"A2B0", *b"A2B1", *b"A2B2"].into_iter().enumerate() {
			p.a2b[i] = res!(with_tag(name, whole, &p.tags, sig, read_lut));
			if let Some(lut) = &mut p.a2b[i] {
				res!(check_lut(name, sig, lut, &p.head, true));
				lut.lab_in = p.head.space == Space::Lab;
			}
		}
		for (i, sig) in [*b"B2A0", *b"B2A1", *b"B2A2"].into_iter().enumerate() {
			p.b2a[i] = res!(with_tag(name, whole, &p.tags, sig, read_lut));
			if let Some(lut) = &mut p.b2a[i] {
				res!(check_lut(name, sig, lut, &p.head, false));
				lut.lab_in = p.head.pcs == Space::Lab;
			}
		}
		Ok(p)
	}

	/// Does the tag table hold `sig`?
	pub fn has(&self, sig: Sig) -> bool {
		self.tags.iter().any(|e| e.sig == sig)
	}
}

fn read_header(name: &str, data: &[u8]) -> Outcome<Header> {
	let what = fmt!("header of profile '{}'", name);
	let h = match data.get(..HEAD_LEN) {
		Some(h)	=> h,
		None	=> return Err(err!(
			"Profile '{}' is truncated: {} bytes are present and its header alone is {}.",
			name, data.len(), HEAD_LEN; Invalid, Input, Decode, Size)),
	};
	let rd = Rd { b: h, what: &what };
	if res!(rd.sig(36)) != *b"acsp" {
		return Err(err!(
			"Profile '{}' lacks the 'acsp' signature at byte 36, so it is not an ICC profile.",
			name; Invalid, Input, Decode));
	}
	let size = res!(rd.u32(0));
	if (size as usize) < HEAD_LEN + 4 {
		return Err(err!(
			"Profile '{}' declares a length of {} bytes, which cannot hold a header and a tag count.",
			name, size; Invalid, Input, Decode, Size));
	}
	let major = res!(rd.u8(8));
	let minor = res!(rd.u8(9)) >> 4;
	if major != 2 && major != 4 {
		return Err(err!(
			"Profile '{}' is version {}.{}, and this reader reads versions 2 and 4.",
			name, major, minor; Invalid, Input, Decode, NoImpl));
	}
	Ok(Header {
		size,
		major,
		minor,
		class:	Class::from_sig(res!(rd.sig(12))),
		space:	Space::from_sig(res!(rd.sig(16))),
		pcs:	Space::from_sig(res!(rd.sig(20))),
		intent:	res!(rd.u32(64)),
		illum:	res!(rd.xyz(68)),
	})
}

fn read_table(name: &str, whole: &[u8]) -> Outcome<Vec<TagEntry>> {
	let what = fmt!("tag table of profile '{}'", name);
	let rd = Rd { b: whole, what: &what };
	let n = res!(rd.u32(HEAD_LEN)) as usize;
	let mut tags = Vec::new();
	for i in 0..n {
		let at = HEAD_LEN + 4 + i * ENTRY_LEN;
		let sig = res!(rd.sig(at));
		let off = res!(rd.u32(at + 4)) as usize;
		let len = res!(rd.u32(at + 8)) as usize;
		match off.checked_add(len) {
			Some(end) if end <= whole.len()	=> (),
			_ => return Err(err!(
				"Tag '{}' of profile '{}' runs from byte {} for {} bytes, past the profile's end \
				at {}, so the profile is truncated.", sig_str(sig), name, off, len, whole.len();
				Invalid, Input, Decode, Size)),
		}
		tags.push(TagEntry { sig, off, len });
	}
	Ok(tags)
}

// Reads the first tag named `sig` with `f`, handing it a view bounded by the tag's declared size.
fn with_tag<T>(
	name:	&str,
	whole:	&[u8],
	tags:	&[TagEntry],
	sig:	Sig,
	f:		impl FnOnce(&Rd) -> Outcome<T>,
)
	-> Outcome<Option<T>>
{
	let e = match tags.iter().find(|e| e.sig == sig) {
		Some(e)	=> e,
		None	=> return Ok(None),
	};
	let what = fmt!("tag '{}' of profile '{}'", sig_str(sig), name);
	let b = match whole.get(e.off..e.off + e.len) {
		Some(b)	=> b,
		None	=> return Err(err!(
			"The {} lies outside the profile.", what; Invalid, Input, Decode, Size)),
	};
	Ok(Some(res!(f(&Rd { b, what: &what }))))
}

fn read_xyz(rd: &Rd) -> Outcome<[f64; 3]> {
	let kind = res!(rd.sig(0));
	if kind != *b"XYZ " {
		return Err(err!(
			"The {} is of type '{}', and an XYZ number tag is of type 'XYZ '.",
			rd.what, sig_str(kind); Invalid, Input, Decode));
	}
	rd.xyz(8)
}

fn read_chad(rd: &Rd) -> Outcome<[f64; 9]> {
	let kind = res!(rd.sig(0));
	if kind != *b"sf32" {
		return Err(err!(
			"The {} is of type '{}', and a chromatic adaptation tag is of type 'sf32'.",
			rd.what, sig_str(kind); Invalid, Input, Decode));
	}
	let mut m = [0.0; 9];
	for (i, v) in m.iter_mut().enumerate() {
		*v = res!(rd.s15(8 + 4 * i));
	}
	Ok(m)
}

// The description, from a version 2 `desc` or a version 4 `mluc`; any other type reads as none.
fn read_text(rd: &Rd) -> Outcome<Option<String>> {
	let kind = res!(rd.sig(0));
	if kind == *b"desc" {
		let n = res!(rd.u32(8)) as usize;
		let s = res!(rd.take(12, n));
		let end = s.iter().position(|b| *b == 0).unwrap_or(s.len());
		return Ok(Some(String::from_utf8_lossy(&s[..end]).into_owned()));
	}
	if kind == *b"mluc" {
		let count = res!(rd.u32(8));
		if count == 0 {
			return Ok(None);
		}
		// The first record: language, country, byte length, byte offset from the tag's start.
		let len = res!(rd.u32(20)) as usize;
		let off = res!(rd.u32(24)) as usize;
		let s = res!(rd.take(off, len));
		let units: Vec<u16> = s.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
		return Ok(Some(String::from_utf16_lossy(&units)));
	}
	Ok(None)
}

fn read_curve(rd: &Rd) -> Outcome<Curve> {
	let kind = res!(rd.sig(0));
	if kind == *b"curv" {
		let n = res!(rd.u32(8)) as usize;
		return match n {
			0	=> Ok(Curve::Identity),
			1	=> Ok(Curve::Gamma(res!(rd.u16(12)) as f64 / 256.0)),
			_	=> Ok(Curve::Table(res!(rd.run16(12, n)))),
		};
	}
	if kind == *b"para" {
		let func = res!(rd.u16(8));
		let count = match func {
			0	=> 1,
			1	=> 3,
			2	=> 4,
			3	=> 5,
			4	=> 7,
			_	=> return Err(err!(
				"The {} is a parametric curve of function type {}, and the types are 0 to 4.",
				rd.what, func; Invalid, Input, Decode, NoImpl)),
		};
		let mut p = [0.0; 7];
		for (i, v) in p.iter_mut().enumerate().take(count) {
			*v = res!(rd.s15(12 + 4 * i));
		}
		if (func == 1 || func == 2) && p[1] == 0.0 {
			return Err(err!(
				"The {} is a parametric curve of function type {} with a = 0, which has no \
				threshold -b/a.", rd.what, func; Invalid, Input, Decode));
		}
		return Ok(Curve::Para { func, p });
	}
	Err(err!(
		"The {} is of type '{}', and this reader reads a curve of type 'curv' or 'para'.",
		rd.what, sig_str(kind); Invalid, Input, Decode, NoImpl))
}

fn read_lut(rd: &Rd) -> Outcome<Lut> {
	let kind = res!(rd.sig(0));
	let bits = match &kind {
		b"mft1"	=> 8u8,
		b"mft2"	=> 16u8,
		b"mAB " | b"mBA "	=> return Err(err!(
			"The {} is of type '{}', a multi-process-element table of a version 4 profile, which \
			this reader does not read. No other profile is substituted for it.",
			rd.what, sig_str(kind); Invalid, Input, Decode, NoImpl)),
		_	=> return Err(err!(
			"The {} is of type '{}', and this reader reads a lookup table of type 'mft1' or \
			'mft2'.", rd.what, sig_str(kind); Invalid, Input, Decode, NoImpl)),
	};
	let nin = res!(rd.u8(8)) as usize;
	let nout = res!(rd.u8(9)) as usize;
	let grid = res!(rd.u8(10)) as usize;
	if nin == 0 || nin > MAX_CH || nout == 0 || nout > MAX_CH || grid < 2 {
		return Err(err!(
			"The {} has {} input and {} output channels and {} nodes an axis, and this reader \
			reads 1 to {} channels and 2 or more nodes.", rd.what, nin, nout, grid, MAX_CH;
			Invalid, Input, Decode, NoImpl));
	}
	let mut mat = [0.0; 9];
	for (i, v) in mat.iter_mut().enumerate() {
		*v = res!(rd.s15(12 + 4 * i));
	}
	let (ine, oute, start) = if bits == 8 {
		(256usize, 256usize, 48usize)
	} else {
		let ine = res!(rd.u16(48)) as usize;
		let oute = res!(rd.u16(50)) as usize;
		if ine < 2 || oute < 2 {
			return Err(err!(
				"The {} has curves of {} input and {} output entries, and a curve needs 2 or more.",
				rd.what, ine, oute; Invalid, Input, Decode));
		}
		(ine, oute, 52usize)
	};
	let width = (bits / 8) as usize;
	let nodes = res!(grid.checked_pow(nin as u32).ok_or_else(|| err!(
		"The {} has {} nodes on each of {} axes, more than a profile can hold.",
		rd.what, grid, nin; Invalid, Input, Decode, Size)));
	let n_in = nin * ine;
	let n_clut = res!(nodes.checked_mul(nout).ok_or_else(|| err!(
		"The {} has {} nodes of {} values, more than a profile can hold.",
		rd.what, nodes, nout; Invalid, Input, Decode, Size)));
	let n_out = nout * oute;
	let read = |at: usize, n: usize| if bits == 8 { rd.run8(at, n) } else { rd.run16(at, n) };
	let ins = res!(read(start, n_in));
	let clut = res!(read(start + n_in * width, n_clut));
	let outs = res!(read(start + (n_in + n_clut) * width, n_out));
	Ok(Lut { bits, nin, nout, grid, mat, lab_in: false, ine, oute, ins, clut, outs })
}

// Does a lookup table's shape fit the profile it is in?
fn check_lut(name: &str, sig: Sig, lut: &Lut, head: &Header, a2b: bool) -> Outcome<()> {
	let (dev_side, pcs_side) = if a2b { (lut.nin, lut.nout) } else { (lut.nout, lut.nin) };
	let dev_want = head.space.channels();
	let pcs_want = head.pcs.channels();
	if dev_want.map_or(false, |n| n != dev_side) || pcs_want.map_or(false, |n| n != pcs_side) {
		return Err(err!(
			"Tag '{}' of profile '{}' maps {} channels to {}, which does not fit a {:?} profile \
			with a {:?} connection space.", sig_str(sig), name, lut.nin, lut.nout, head.space,
			head.pcs; Invalid, Input, Decode, Mismatch));
	}
	Ok(())
}
