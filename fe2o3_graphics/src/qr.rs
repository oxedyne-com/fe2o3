//! A QR Code encoder: text or bytes in, a grid of dark and light modules out.
//!
//! This is a from-scratch, zero-dependency port of the algorithm published by Project Nayuki
//! (MIT licence), whose structure this follows closely because the QR Code standard is a frozen
//! ISO/IEC 18004 specification and correctness is the whole point. The encoder owns every step:
//! byte-mode segment packing, Reed-Solomon error correction over GF(256), version selection,
//! function-pattern layout, the eight data masks, and the penalty-driven choice between them.
//!
//! Only the module matrix is produced. Turning that grid into pixels, an SVG, or a printed square
//! is a rendering concern that belongs to the caller, so no image output lives here.
//!
//! # What is encoded
//!
//! Byte mode (arbitrary octets, which for text means its UTF-8 bytes) is the only segment mode
//! implemented, because the reason this exists is to carry short URLs for device pairing, and a
//! URL is bytes. Numeric and alphanumeric modes, which pack decimal digits or a restricted
//! 45-character set more tightly, are deliberately omitted; a caller who needs the extra density
//! for a purely numeric payload is the signal to add them.
//!
//! # Example
//!
//! ```
//! use oxedyne_fe2o3_graphics::qr::{encode, QrEcc};
//!
//! if let Ok(qr) = encode("https://example.org", QrEcc::Medium) {
//! 	assert!(qr.get(0, 0)); // The finder pattern's outer ring is dark at the corner.
//! 	assert!(qr.size() >= 21); // Version 1 is 21 by 21, and larger versions only grow.
//! }
//! ```

use oxedyne_fe2o3_core::prelude::*;

/// The four error-correction levels a QR Code may carry, from the least redundancy to the most.
///
/// A higher level survives more damage to the printed symbol but leaves fewer bits for the
/// payload at a given version, so the encoder may have to step up to a larger version to fit the
/// same data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QrEcc {
	/// Tolerates about 7% of the codewords being corrupted.
	Low,
	/// Tolerates about 15% of the codewords being corrupted.
	Medium,
	/// Tolerates about 25% of the codewords being corrupted.
	Quartile,
	/// Tolerates about 30% of the codewords being corrupted.
	High,
}

impl QrEcc {

	/// The index of this level in the standard's per-version parameter tables, where Low is 0 and
	/// High is 3.
	fn ordinal(self) -> usize {
		match self {
			Self::Low	=> 0,
			Self::Medium	=> 1,
			Self::Quartile	=> 2,
			Self::High	=> 3,
		}
	}

	/// The two-bit value that names this level inside the format-information field. Note that this
	/// is not the same order as [`QrEcc::ordinal`]: the standard assigns Medium the value 0.
	fn format_bits(self) -> u32 {
		match self {
			Self::Low	=> 1,
			Self::Medium	=> 0,
			Self::Quartile	=> 3,
			Self::High	=> 2,
		}
	}
}

/// The smallest QR Code version, a 21 by 21 grid.
pub const MIN_VERSION: u8 = 1;
/// The largest QR Code version, a 177 by 177 grid.
pub const MAX_VERSION: u8 = 40;

/// Penalty weight for a run of five or more same-coloured modules in a line.
const PENALTY_N1: i32 = 3;
/// Penalty weight for a two-by-two block of one colour.
const PENALTY_N2: i32 = 3;
/// Penalty weight for a finder-like 1:1:3:1:1 pattern in a line.
const PENALTY_N3: i32 = 40;
/// Penalty weight for each 5% the dark-module proportion strays from one half.
const PENALTY_N4: i32 = 10;

/// Number of error-correction codewords in each block, indexed by `[ecc ordinal][version]`, with
/// version 0 unused and set to an illegal value so a stray index is caught rather than silently
/// wrong.
static ECC_CODEWORDS_PER_BLOCK: [[i8; 41]; 4] = [
	// 0   1   2   3   4   5   6   7   8   9  10  11  12  13  14  15  16  17  18  19  20  21  22  23  24  25  26  27  28  29  30  31  32  33  34  35  36  37  38  39  40
	[-1,  7, 10, 15, 20, 26, 18, 20, 24, 30, 18, 20, 24, 26, 30, 22, 24, 28, 30, 28, 28, 28, 28, 30, 30, 26, 28, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30], // Low
	[-1, 10, 16, 26, 18, 24, 16, 18, 22, 22, 26, 30, 22, 22, 24, 24, 28, 28, 26, 26, 26, 26, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28], // Medium
	[-1, 13, 22, 18, 26, 18, 24, 18, 22, 20, 24, 28, 26, 24, 20, 30, 24, 28, 28, 26, 30, 28, 30, 30, 30, 30, 28, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30], // Quartile
	[-1, 17, 28, 22, 16, 22, 28, 26, 26, 24, 28, 24, 28, 22, 24, 24, 30, 28, 28, 26, 28, 30, 24, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30], // High
];

/// Number of error-correction blocks, indexed by `[ecc ordinal][version]`, with version 0 unused.
static NUM_ERROR_CORRECTION_BLOCKS: [[i8; 41]; 4] = [
	// 0  1  2  3  4  5  6  7  8  9 10  11  12  13  14  15  16  17  18  19  20  21  22  23  24  25  26  27  28  29  30  31  32  33  34  35  36  37  38  39  40
	[-1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 4,  4,  4,  4,  4,  6,  6,  6,  6,  7,  8,  8,  9,  9, 10, 12, 12, 12, 13, 14, 15, 16, 17, 18, 19, 19, 20, 21, 22, 24, 25], // Low
	[-1, 1, 1, 1, 2, 2, 4, 4, 4, 5, 5,  5,  8,  9,  9, 10, 10, 11, 13, 14, 16, 17, 17, 18, 20, 21, 23, 25, 26, 28, 29, 31, 33, 35, 37, 38, 40, 43, 45, 47, 49], // Medium
	[-1, 1, 1, 2, 2, 4, 4, 6, 6, 8, 8,  8, 10, 12, 16, 12, 17, 16, 18, 21, 20, 23, 23, 25, 27, 29, 34, 34, 35, 38, 40, 43, 45, 48, 51, 53, 56, 59, 62, 65, 68], // Quartile
	[-1, 1, 1, 2, 4, 4, 4, 5, 6, 8, 8, 11, 11, 16, 16, 18, 16, 19, 21, 25, 25, 25, 34, 30, 32, 35, 37, 40, 42, 45, 48, 51, 54, 57, 60, 63, 66, 70, 74, 77, 81], // High
];

/// A finished QR Code as a square grid of modules, each either dark (true) or light (false).
///
/// The grid alone is deliberately all this exposes: it is what a renderer needs, and everything
/// used to build it (function-pattern bookkeeping, the codeword stream) is discarded once the
/// modules are fixed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QrMatrix {
	/// The side length in modules, always odd and in the range 21 to 177.
	size:	usize,
	/// The modules in row-major order, `size * size` of them, true meaning dark.
	mods:	Vec<bool>,
	/// The version, 1 to 40.
	ver:	u8,
	/// The error-correction level actually used, which may exceed the one requested.
	ecc:	QrEcc,
}

impl QrMatrix {

	/// The side length of the grid in modules.
	pub fn size(&self) -> usize {
		self.size
	}

	/// The version number, 1 to 40, where the side length is `version * 4 + 17`.
	pub fn version(&self) -> u8 {
		self.ver
	}

	/// The error-correction level the symbol carries, which may be higher than the one asked for
	/// when the chosen version had spare capacity.
	pub fn ecc(&self) -> QrEcc {
		self.ecc
	}

	/// Returns whether the module at column `x`, row `y` is dark. Coordinates outside the grid
	/// are light, matching the quiet zone a decoder assumes surrounds the symbol.
	pub fn get(&self, x: usize, y: usize) -> bool {
		if x >= self.size || y >= self.size {
			return false;
		}
		self.mods[y * self.size + x]
	}
}

/// Encodes text as a QR Code in byte mode, choosing the smallest version that fits at the given
/// error-correction level and the mask with the lowest penalty.
///
/// The text is taken as its UTF-8 bytes. The error-correction level is a floor, not an exact
/// setting: when the chosen version leaves room, the level is raised for free, so the returned
/// [`QrMatrix::ecc`] may exceed `ecc`.
///
/// # Errors
///
/// Fails if the byte length does not fit in even a version-40 symbol at the requested level.
pub fn encode(text: &str, ecc: QrEcc) -> Outcome<QrMatrix> {
	encode_bytes(text.as_bytes(), ecc)
}

/// Encodes arbitrary bytes as a QR Code in byte mode. See [`encode`] for the version and mask
/// selection, which is identical; this is the entry point when the payload is not text.
///
/// # Errors
///
/// Fails if the byte length does not fit in even a version-40 symbol at the requested level.
pub fn encode_bytes(data: &[u8], ecc: QrEcc) -> Outcome<QrMatrix> {
	encode_bytes_advanced(data, ecc, MIN_VERSION, MAX_VERSION, None, true)
}

/// Encodes bytes with full control over the version range, the mask, and error-correction
/// boosting, for callers that need a specific symbol rather than the smallest convenient one.
///
/// * `minver` and `maxver` bound the version search, each in 1 to 40 with `minver <= maxver`.
/// * `mask` forces one of the eight masks (0 to 7) when `Some`, or asks for automatic selection
///   when `None`.
/// * `boost` raises the error-correction level to fill spare capacity when true.
///
/// # Errors
///
/// Fails on an out-of-range argument, or if the data does not fit within `maxver`.
pub fn encode_bytes_advanced(
	data:	&[u8],
	ecc:	QrEcc,
	minver:	u8,
	maxver:	u8,
	mask:	Option<u8>,
	boost:	bool,
)
	-> Outcome<QrMatrix>
{
	if minver < MIN_VERSION || maxver > MAX_VERSION || minver > maxver {
		return Err(err!(
			"The version range {} to {} lies outside 1 to 40, or is inverted.", minver, maxver;
		Invalid, Input, Range));
	}
	if let Some(m) = mask {
		if m > 7 {
			return Err(err!(
				"The mask {} is not one of the eight masks 0 to 7.", m;
			Invalid, Input, Range));
		}
	}

	// The bits a byte-mode segment occupies at a version depend only on the character-count field
	// width, which itself depends on the version band, so widen the version until the data fits.
	let mut ver = minver;
	let used = loop {
		let cap = res!(num_data_codewords(ver, ecc)) * 8; // Bits available.
		let need = segment_bits(data.len(), ver); // Bits the segment needs.
		if let Some(n) = need {
			if n <= cap {
				break n;
			}
		}
		if ver >= maxver {
			return Err(err!(
				"{} bytes do not fit a QR Code of version {} at the requested error-correction \
				level.", data.len(), maxver;
			Input, Excessive, Size));
		}
		ver += 1;
	};

	// With the version fixed, spend any slack on stronger error correction, since a symbol that
	// can fit more redundancy for free may as well.
	let mut ecc = ecc;
	if boost {
		for cand in [QrEcc::Medium, QrEcc::Quartile, QrEcc::High] {
			if used <= res!(num_data_codewords(ver, cand)) * 8 {
				ecc = cand;
			}
		}
	}

	// Build the bit stream: the byte-mode indicator, the character count, then the data bytes.
	let mut bits: Vec<bool> = Vec::new();
	append_bits(&mut bits, 0x4, 4); // Byte-mode indicator.
	append_bits(&mut bits, data.len() as u32, char_count_bits(ver));
	for &b in data {
		append_bits(&mut bits, u32::from(b), 8);
	}

	// Pad: a terminator of up to four zero bits, zeros to the next byte boundary, then the two
	// alternating pad bytes until the data capacity is full.
	let cap = res!(num_data_codewords(ver, ecc)) * 8;
	let term = std::cmp::min(4, cap - bits.len());
	append_bits(&mut bits, 0, term as u8);
	let pad_to_byte = bits.len().wrapping_neg() & 7;
	append_bits(&mut bits, 0, pad_to_byte as u8);
	let mut pads = [0xEC_u32, 0x11].into_iter().cycle();
	while bits.len() < cap {
		if let Some(p) = pads.next() {
			append_bits(&mut bits, p, 8);
		}
	}

	// Pack the bits into codeword bytes, most-significant bit first.
	let mut codewords = vec![0u8; bits.len() / 8];
	for (i, bit) in bits.iter().enumerate() {
		if *bit {
			codewords[i >> 3] |= 1 << (7 - (i & 7));
		}
	}

	build(ver, ecc, &codewords, mask)
}

/// Appends the low `len` bits of `val`, most-significant first, to a bit vector.
fn append_bits(bits: &mut Vec<bool>, val: u32, len: u8) {
	let n = len as i32;
	for i in (0 .. n).rev() {
		bits.push((val >> i) & 1 != 0);
	}
}

/// Returns bit `i` of `x`.
fn get_bit(x: u32, i: i32) -> bool {
	(x >> i) & 1 != 0
}

/// The width of the byte-mode character-count field at a version: eight bits for versions 1 to 9,
/// sixteen bits thereafter.
fn char_count_bits(ver: u8) -> u8 {
	if ver <= 9 { 8 } else { 16 }
}

/// The total bits a single byte-mode segment of `len` bytes occupies at a version, or `None` when
/// the length overflows the character-count field.
fn segment_bits(len: usize, ver: u8) -> Option<usize> {
	let ccbits = char_count_bits(ver);
	if len >= (1usize << ccbits) {
		return None; // The length does not fit the field.
	}
	Some(4 + usize::from(ccbits) + len * 8)
}

/// Reads a per-version table value, returning it as a `usize`. The version is assumed valid, and
/// the illegal padding entries are never reached on a valid version.
fn table_get(table: &[[i8; 41]; 4], ver: u8, ecc: QrEcc) -> usize {
	let v = table[ecc.ordinal()][ver as usize];
	if v < 0 { 0 } else { v as usize }
}

/// The number of bits a version can hold before error correction and function patterns are
/// removed, that is, the count of data and error-correction modules together.
fn num_raw_data_modules(ver: u8) -> usize {
	let v = ver as usize;
	let mut n = (16 * v + 128) * v + 64;
	if v >= 2 {
		let numalign = v / 7 + 2;
		n -= (25 * numalign - 10) * numalign - 55;
		if v >= 7 {
			n -= 36; // Two version-information blocks of eighteen bits.
		}
	}
	n
}

/// The number of eight-bit data codewords a version holds at an error-correction level, once the
/// error-correction codewords are set aside.
fn num_data_codewords(ver: u8, ecc: QrEcc) -> Outcome<usize> {
	if !(MIN_VERSION ..= MAX_VERSION).contains(&ver) {
		return Err(err!("Version {} lies outside 1 to 40.", ver; Invalid, Input, Range));
	}
	let raw = num_raw_data_modules(ver) / 8;
	let ecc_words = table_get(&ECC_CODEWORDS_PER_BLOCK, ver, ecc)
		* table_get(&NUM_ERROR_CORRECTION_BLOCKS, ver, ecc);
	Ok(raw - ecc_words)
}

// -------------------------------------------------------------------------------------------------
// Reed-Solomon error correction over GF(256) with the QR reducing polynomial x^8 + x^4 + x^3 +
// x^2 + 1 (0x11D).
// -------------------------------------------------------------------------------------------------

/// Multiplies two field elements of GF(256) by Russian-peasant multiplication, reducing modulo
/// the QR field's primitive polynomial.
fn gf_mul(x: u8, y: u8) -> u8 {
	let mut z: u8 = 0;
	for i in (0 .. 8).rev() {
		// Double, reducing when the high bit falls off, then add x when this bit of y is set.
		z = (z << 1) ^ (((z >> 7) & 1) * 0x1D);
		z ^= ((y >> i) & 1) * x;
	}
	z
}

/// Computes the divisor polynomial for `degree` error-correction codewords: the product of the
/// monomials (x - r^i) for i in 0 to degree-1, with the leading coefficient of 1 dropped and the
/// remaining coefficients stored from the second-highest degree down to the constant term.
fn rs_divisor(degree: usize) -> Vec<u8> {
	let mut result = vec![0u8; degree];
	if degree == 0 {
		return result;
	}
	result[degree - 1] = 1; // Start with the monomial x^0.
	let mut root: u8 = 1;
	for _ in 0 .. degree {
		// Multiply the current product by (x - r^i).
		for j in 0 .. result.len() {
			result[j] = gf_mul(result[j], root);
			if j + 1 < result.len() {
				result[j] ^= result[j + 1];
			}
		}
		root = gf_mul(root, 0x02);
	}
	result
}

/// Divides the data polynomial by the divisor and returns the remainder, which is the block's
/// error-correction codewords.
fn rs_remainder(data: &[u8], divisor: &[u8]) -> Vec<u8> {
	let mut result = vec![0u8; divisor.len()];
	for &b in data {
		let factor = b ^ result.remove(0);
		result.push(0);
		for (x, &y) in result.iter_mut().zip(divisor.iter()) {
			*x ^= gf_mul(y, factor);
		}
	}
	result
}

// -------------------------------------------------------------------------------------------------
// Grid construction.
// -------------------------------------------------------------------------------------------------

/// The working grid during construction: the modules plus a parallel record of which cells are
/// function patterns and so must not carry data or be masked.
struct Grid {
	/// Side length in modules.
	size:	i32,
	/// Modules in row-major order, true meaning dark.
	mods:	Vec<bool>,
	/// Whether each cell is a function module, in the same order as `mods`.
	func:	Vec<bool>,
}

impl Grid {

	/// Creates an all-light grid of the given side length with no function modules yet.
	fn new(size: i32) -> Self {
		let n = (size * size) as usize;
		Self {
			size,
			mods:	vec![false; n],
			func:	vec![false; n],
		}
	}

	/// Reads a module, treating anything outside the grid as light.
	fn get(&self, x: i32, y: i32) -> bool {
		if x < 0 || y < 0 || x >= self.size || y >= self.size {
			return false;
		}
		self.mods[(y * self.size + x) as usize]
	}

	/// Sets a module and records whether it is a function pattern. Coordinates outside the grid
	/// are ignored, which lets a pattern near an edge be drawn with a single unconditional loop.
	fn set(&mut self, x: i32, y: i32, dark: bool, is_func: bool) {
		if x < 0 || y < 0 || x >= self.size || y >= self.size {
			return;
		}
		let i = (y * self.size + x) as usize;
		self.mods[i] = dark;
		self.func[i] = is_func;
	}

	/// Whether a cell is a function module, treating anything outside the grid as one.
	fn is_func(&self, x: i32, y: i32) -> bool {
		if x < 0 || y < 0 || x >= self.size || y >= self.size {
			return true;
		}
		self.func[(y * self.size + x) as usize]
	}
}

/// Assembles the whole symbol: draws the function patterns, interleaves the data with its error
/// correction, lays the codewords into the data region, then applies and records the mask.
fn build(ver: u8, ecc: QrEcc, codewords: &[u8], mask: Option<u8>)
	-> Outcome<QrMatrix>
{
	let size = (ver as i32) * 4 + 17;
	let mut g = Grid::new(size);

	draw_function_patterns(&mut g, ver, ecc);
	let all = res!(add_ecc_and_interleave(ver, ecc, codewords));
	draw_codewords(&mut g, &all);

	// Choose a mask: the requested one, or the lowest-penalty one found by trying all eight.
	let chosen = match mask {
		Some(m) => m,
		None => {
			let mut best = 0u8;
			let mut best_pen = i32::MAX;
			for m in 0 .. 8u8 {
				apply_mask(&mut g, m);
				draw_format_bits(&mut g, ecc, m);
				let pen = penalty_score(&g);
				if pen < best_pen {
					best_pen = pen;
					best = m;
				}
				apply_mask(&mut g, m); // The mask is its own inverse, so this undoes it.
			}
			best
		},
	};
	apply_mask(&mut g, chosen);
	draw_format_bits(&mut g, ecc, chosen);

	Ok(QrMatrix {
		size:	size as usize,
		mods:	g.mods,
		ver,
		ecc,
	})
}

/// Draws the timing lines, the three finder patterns and their separators, the alignment
/// patterns, the dark module, and placeholder format and version information.
fn draw_function_patterns(g: &mut Grid, ver: u8, ecc: QrEcc) {
	let size = g.size;

	// Timing patterns: alternating modules along row six and column six.
	for i in 0 .. size {
		g.set(6, i, i % 2 == 0, true);
		g.set(i, 6, i % 2 == 0, true);
	}

	// The three finder patterns, at every corner but the bottom right, with their separators.
	draw_finder(g, 3, 3);
	draw_finder(g, size - 4, 3);
	draw_finder(g, 3, size - 4);

	// Alignment patterns at the grid of standard positions, skipping the three finder corners.
	let pos = alignment_positions(ver);
	let n = pos.len();
	for i in 0 .. n {
		for j in 0 .. n {
			let corner = (i == 0 && j == 0)
				|| (i == 0 && j == n - 1)
				|| (i == n - 1 && j == 0);
			if !corner {
				draw_alignment(g, pos[i], pos[j]);
			}
		}
	}

	// Format and version information: a placeholder now, drawn for real once the mask is known.
	draw_format_bits(g, ecc, 0);
	draw_version(g, ver);
}

/// Draws a seven-by-seven finder pattern centred at the given coordinates, together with the
/// one-module light separator that rings it.
fn draw_finder(g: &mut Grid, cx: i32, cy: i32) {
	for dy in -4i32 ..= 4 {
		for dx in -4i32 ..= 4 {
			let dist = std::cmp::max(dx.abs(), dy.abs()); // Chebyshev distance.
			g.set(cx + dx, cy + dy, dist != 2 && dist != 4, true);
		}
	}
}

/// Draws a five-by-five alignment pattern centred at the given coordinates.
fn draw_alignment(g: &mut Grid, cx: i32, cy: i32) {
	for dy in -2i32 ..= 2 {
		for dx in -2i32 ..= 2 {
			g.set(cx + dx, cy + dy, std::cmp::max(dx.abs(), dy.abs()) != 1, true);
		}
	}
}

/// The centre coordinates of the alignment patterns for a version. Version 1 has none; from
/// version 2 the coordinates form an evenly spaced grid whose spacing the standard fixes.
fn alignment_positions(ver: u8) -> Vec<i32> {
	if ver == 1 {
		return Vec::new();
	}
	let v = ver as i32;
	let num = v / 7 + 2; // The number of coordinates along one side.
	let step = if ver == 32 {
		26
	} else {
		(v * 4 + num * 2 + 1) / (num * 2 - 2) * 2
	};
	let size = v * 4 + 17;
	let mut result: Vec<i32> = (0 .. num - 1).map(|i| size - 7 - i * step).collect();
	result.push(6);
	result.reverse();
	result
}

/// Draws the fifteen-bit format information, a BCH-protected code naming the error-correction
/// level and the mask, in its two copies around the finder patterns.
fn draw_format_bits(g: &mut Grid, ecc: QrEcc, mask: u8) {
	let data = (ecc.format_bits() << 3) | u32::from(mask);
	let mut rem = data;
	for _ in 0 .. 10 {
		rem = (rem << 1) ^ (((rem >> 9) & 1) * 0x537);
	}
	let bits = ((data << 10) | rem) ^ 0x5412; // The standard's mask against an all-zero code.

	// First copy, split around the top-left finder.
	for i in 0 .. 6 {
		g.set(8, i, get_bit(bits, i), true);
	}
	g.set(8, 7, get_bit(bits, 6), true);
	g.set(8, 8, get_bit(bits, 7), true);
	g.set(7, 8, get_bit(bits, 8), true);
	for i in 9 .. 15 {
		g.set(14 - i, 8, get_bit(bits, i), true);
	}

	// Second copy, along the edges beside the other two finders.
	let size = g.size;
	for i in 0 .. 8 {
		g.set(size - 1 - i, 8, get_bit(bits, i), true);
	}
	for i in 8 .. 15 {
		g.set(8, size - 15 + i, get_bit(bits, i), true);
	}
	g.set(8, size - 8, true, true); // The dark module, always set.
}

/// Draws the eighteen-bit version information, present only from version 7, in its two copies near
/// the bottom-left and top-right finders.
fn draw_version(g: &mut Grid, ver: u8) {
	if ver < 7 {
		return;
	}
	let data = u32::from(ver);
	let mut rem = data;
	for _ in 0 .. 12 {
		rem = (rem << 1) ^ (((rem >> 11) & 1) * 0x1F25);
	}
	let bits = (data << 12) | rem;

	let size = g.size;
	for i in 0 .. 18 {
		let bit = get_bit(bits, i);
		let a = size - 11 + i % 3;
		let b = i / 3;
		g.set(a, b, bit, true);
		g.set(b, a, bit, true);
	}
}

/// Splits the data codewords into blocks, appends each block's Reed-Solomon error correction, then
/// interleaves the blocks into the single codeword sequence the standard lays into the grid.
fn add_ecc_and_interleave(ver: u8, ecc: QrEcc, data: &[u8])
	-> Outcome<Vec<u8>>
{
	let expect = res!(num_data_codewords(ver, ecc));
	if data.len() != expect {
		return Err(err!(
			"The data has {} codewords but version {} needs {}.", data.len(), ver, expect;
		Bug, Mismatch, Size));
	}

	let numblocks = table_get(&NUM_ERROR_CORRECTION_BLOCKS, ver, ecc);
	let blockecc = table_get(&ECC_CODEWORDS_PER_BLOCK, ver, ecc);
	let rawcw = num_raw_data_modules(ver) / 8;
	let numshort = numblocks - rawcw % numblocks; // Blocks one codeword shorter than the rest.
	let shortlen = rawcw / numblocks; // Total codewords in a short block.

	let divisor = rs_divisor(blockecc);
	let mut blocks: Vec<Vec<u8>> = Vec::with_capacity(numblocks);
	let mut k = 0usize; // Read cursor into the data.
	for i in 0 .. numblocks {
		let datlen = shortlen - blockecc + if i < numshort { 0 } else { 1 };
		let mut blk = data[k .. k + datlen].to_vec();
		k += datlen;
		let ecc_bytes = rs_remainder(&blk, &divisor);
		if i < numshort {
			blk.push(0); // Pad short blocks so every block interleaves at the same width.
		}
		blk.extend_from_slice(&ecc_bytes);
		blocks.push(blk);
	}

	// Interleave: take the ith codeword of every block in turn, skipping the padding cell that
	// short blocks carry in the data region.
	let mut result: Vec<u8> = Vec::with_capacity(rawcw);
	let width = shortlen + 1; // The length of the longest block.
	for i in 0 .. width {
		for (j, blk) in blocks.iter().enumerate() {
			if i != shortlen - blockecc || j >= numshort {
				result.push(blk[i]);
			}
		}
	}
	Ok(result)
}

/// Lays the interleaved codeword bytes into the data region in the standard's zigzag order: up and
/// down pairs of columns, right to left, skipping the vertical timing column and every function
/// module.
fn draw_codewords(g: &mut Grid, data: &[u8]) {
	let size = g.size;
	let mut i = 0usize; // Bit index into the data.
	let mut right = size - 1; // The right column of the current pair.
	while right >= 1 {
		if right == 6 {
			right = 5; // Skip the vertical timing column.
		}
		for v in 0 .. size {
			for j in 0 .. 2 {
				let x = right - j;
				let upward = ((right + 1) & 2) == 0;
				let y = if upward { size - 1 - v } else { v };
				if !g.is_func(x, y) && i < data.len() * 8 {
					let dark = get_bit(u32::from(data[i >> 3]), 7 - (i & 7) as i32);
					g.set(x, y, dark, false);
					i += 1;
				}
			}
		}
		right -= 2;
	}
}

/// Applies one of the eight data masks in place, flipping data modules where the mask condition
/// holds and leaving function modules untouched. Applying the same mask twice restores the grid.
fn apply_mask(g: &mut Grid, mask: u8) {
	let size = g.size;
	for y in 0 .. size {
		for x in 0 .. size {
			if g.is_func(x, y) {
				continue;
			}
			let (xl, yl) = (x as i64, y as i64);
			let invert = match mask {
				0 => (xl + yl) % 2 == 0,
				1 => yl % 2 == 0,
				2 => xl % 3 == 0,
				3 => (xl + yl) % 3 == 0,
				4 => (xl / 3 + yl / 2) % 2 == 0,
				5 => xl * yl % 2 + xl * yl % 3 == 0,
				6 => (xl * yl % 2 + xl * yl % 3) % 2 == 0,
				_ => ((xl + yl) % 2 + xl * yl % 3) % 2 == 0,
			};
			if invert {
				let idx = (y * size + x) as usize;
				g.mods[idx] = !g.mods[idx];
			}
		}
	}
}

/// The total penalty score of a grid, the sum of the standard's four rules. A lower score is a
/// more robust symbol, which is how the best mask is chosen.
fn penalty_score(g: &Grid) -> i32 {
	let size = g.size;
	let mut result: i32 = 0;

	// Rule one and rule three, scanning each row then each column: runs of five or more same
	// modules, and finder-like patterns.
	for y in 0 .. size {
		let mut colour = false;
		let mut run = 0i32;
		let mut hist = FinderRun::new(size);
		for x in 0 .. size {
			if g.get(x, y) == colour {
				run += 1;
				if run == 5 {
					result += PENALTY_N1;
				} else if run > 5 {
					result += 1;
				}
			} else {
				hist.add(run);
				if !colour {
					result += hist.count() * PENALTY_N3;
				}
				colour = g.get(x, y);
				run = 1;
			}
		}
		result += hist.terminate(colour, run) * PENALTY_N3;
	}
	for x in 0 .. size {
		let mut colour = false;
		let mut run = 0i32;
		let mut hist = FinderRun::new(size);
		for y in 0 .. size {
			if g.get(x, y) == colour {
				run += 1;
				if run == 5 {
					result += PENALTY_N1;
				} else if run > 5 {
					result += 1;
				}
			} else {
				hist.add(run);
				if !colour {
					result += hist.count() * PENALTY_N3;
				}
				colour = g.get(x, y);
				run = 1;
			}
		}
		result += hist.terminate(colour, run) * PENALTY_N3;
	}

	// Rule two: every two-by-two block of one colour.
	for y in 0 .. size - 1 {
		for x in 0 .. size - 1 {
			let c = g.get(x, y);
			if c == g.get(x + 1, y) && c == g.get(x, y + 1) && c == g.get(x + 1, y + 1) {
				result += PENALTY_N2;
			}
		}
	}

	// Rule four: how far the proportion of dark modules strays from one half, in 5% steps.
	let dark: i32 = g.mods.iter().filter(|&&m| m).count() as i32;
	let total = size * size;
	let k = ((dark * 20 - total * 10).abs() + total - 1) / total - 1;
	result += k * PENALTY_N4;

	result
}

/// A sliding record of the last seven run lengths along a line, used to detect the finder-like
/// 1:1:3:1:1 pattern that rule three penalises.
struct FinderRun {
	/// The grid side length, added as the light border a decoder sees around the symbol.
	size:	i32,
	/// The seven most recent run lengths, most recent first.
	hist:	[i32; 7],
}

impl FinderRun {

	/// Creates an empty history for a line of the given length.
	fn new(size: i32) -> Self {
		Self { size, hist: [0i32; 7] }
	}

	/// Pushes a run length onto the history, dropping the oldest, and folds in the light border
	/// at the very start of a line.
	fn add(&mut self, run: i32) {
		let mut run = run;
		if self.hist[0] == 0 {
			run += self.size; // The quiet zone before the first run.
		}
		for i in (1 .. self.hist.len()).rev() {
			self.hist[i] = self.hist[i - 1];
		}
		self.hist[0] = run;
	}

	/// Counts the finder-like patterns ending at the current position, either zero, one, or two.
	/// Only meaningful immediately after a light run has been added.
	fn count(&self) -> i32 {
		let h = &self.hist;
		let n = h[1];
		let core = n > 0 && h[2] == n && h[3] == n * 3 && h[4] == n && h[5] == n;
		i32::from(core && h[0] >= n * 4 && h[6] >= n)
			+ i32::from(core && h[6] >= n * 4 && h[0] >= n)
	}

	/// Terminates the line, folding in the final run and the trailing light border, and returns
	/// the finder-like patterns that close it out.
	fn terminate(mut self, colour: bool, run: i32) -> i32 {
		let mut run = run;
		if colour {
			self.add(run); // Close a dark run first.
			run = 0;
		}
		run += self.size; // The quiet zone after the last run.
		self.add(run);
		self.count()
	}
}

#[cfg(test)]
mod tests;
