//! An HEVC decoder, for the still pictures inside a HEIC file.
//!
//! HEVC (ITU-T H.265) is the codec a phone's photograph is coded in once it stops being JPEG, and
//! there is no way to read one without decoding it. This module is that decoder. It is built for
//! **intra** coding only -- a still picture refers to nothing but itself, so everything about
//! motion, reference pictures and prediction between frames is absent by construction rather than
//! unimplemented.
//!
//! # What is here so far
//!
//! The bitstream side: splitting a stream into NAL units, undoing the emulation-prevention bytes,
//! and reading the sequence and picture parameter sets that say how big the picture is and how it
//! is cut up. That is the part every later stage is written against, and it is the part that can be
//! checked before any pixel exists: the size a sequence parameter set codes must agree with the
//! size the HEIF container's `ispe` property declares, and those two numbers are written into the
//! file by different parts of an encoder.
//!
//! The slice segment header too, including the entry points that say where each row of the picture
//! begins, and the CABAC arithmetic decoder that reads the entropy-coded data after it. Still to
//! come, in the order they are needed: the context variables each syntax element uses, the coding
//! quadtree, residual coding, the inverse transforms, intra prediction, deblocking, the sample
//! adaptive offset, and the conversion out of 4:2:0 into RGB.
//!
//! The arithmetic decoder is the last piece that can be held to a standard before a picture comes
//! out, and it is held to two: every context starts in a state the probability tables actually
//! have -- all 256 initialisation values against every quantisation parameter a slice may carry --
//! and the coding interval is between 256 and 510 after every bin, whatever is fed in. A
//! renormalisation one shift short satisfies neither and decodes plausible rubbish rather than
//! failing, which is the kind of fault that otherwise survives until a photograph comes out
//! wrong.
//!
//! # What the pictures in one real library actually are
//!
//! Every sequence parameter set in 359 HEIC photographs out of a family library was read, and they
//! are uniform: **8-bit 4:2:0, coding tree blocks of 32, the sample adaptive offset on, no PCM, no
//! scaling lists, one tile**. The tiles are 512 by 512 in 350 of them, 1024 by 1024 in eight, and
//! the one photograph not stored as a grid is 720 by 720.
//!
//! One of those measurements was a surprise and it changes the shape of the decoder: **every one of
//! them is coded in wavefronts** (`entropy_coding_sync_enabled_flag`). The arithmetic decoder is
//! therefore reset at the start of every row of coding tree blocks, from the state saved after the
//! second block of the row above, and the slice header carries a byte offset for each row. That is
//! not an exotic case to be refused; it is the case. All 359 slice headers read, and in each the
//! number of rows the header names agrees with the number the sequence parameter set implies --
//! sixteen for a 512-pixel tile at 32, twenty-three for the 720-pixel picture -- which is the
//! check that caught the first reading, where the flag was mistaken for something rare and every
//! photograph in the corpus was refused.
//!
//! # References
//!
//! ITU-T H.265 (ISO/IEC 23008-2). The NAL unit header is §7.3.1.2, the sequence parameter set
//! §7.3.2.2, the picture parameter set §7.3.2.3, the profile-tier-level structure §7.3.3, and the
//! short-term reference picture sets §7.3.7. The `hvcC` record the parameter sets arrive in is
//! ISO/IEC 14496-15 §8.3.3. Every constant below names the clause it comes from.

pub mod cabac;
pub mod colour;
pub mod decode;
pub mod intra;
pub mod scan;
pub mod transform;

pub use cabac::{
	Cabac,
	Contexts,
	Ctx,
	Rows,
	Set,
	CONTEXTS,
};

use oxedyne_fe2o3_core::prelude::*;

/// The largest picture this decoder will describe, in luma samples each way.
///
/// Sixteen thousand is past every camera and well inside what the level limits allow; it is a
/// ceiling against a parameter set that is a mistake, not a limit on real photographs.
pub const MAX_SIDE: u32 = 16_384;

/// NAL unit types this decoder cares about (H.265 Table 7-1).
pub mod nal {
	/// A coded slice of an IDR picture with no leading pictures, which is what a still is.
	pub const IDR_W_RADL: u8 = 19;
	/// The other IDR form.
	pub const IDR_N_LP: u8 = 20;
	/// A video parameter set.
	pub const VPS: u8 = 32;
	/// A sequence parameter set.
	pub const SPS: u8 = 33;
	/// A picture parameter set.
	pub const PPS: u8 = 34;
}

/// One NAL unit: what it is, and its payload with the emulation prevention undone.
#[derive(Clone, Debug)]
pub struct Unit {
	/// The type, from the two-byte NAL unit header.
	pub kind:	u8,
	/// The temporal sub-layer, plus one as the header codes it.
	pub layer:	u8,
	/// The payload, after the header and with every emulation prevention byte removed.
	pub body:	Vec<u8>,
	/// The same payload as it arrived, escaping and all.
	///
	/// Kept because the entry point offsets in a slice header are counted in **escaped** bytes:
	/// "emulation prevention bytes that appear in the slice segment data portion of the coded
	/// slice segment NAL unit are counted as part of the slice segment data for purposes of subset
	/// identification" (§7.4.7.1). Splitting the unescaped payload at those offsets puts every row
	/// of blocks after the first escaped byte in the wrong place.
	pub raw:	Vec<u8>,
}

/// What a sequence parameter set says about the pictures that follow it.
///
/// Only the fields a still picture's decoder acts on are kept. The rest are read past, because a
/// parameter set is a run of variable-length codes and there is no skipping to a field without
/// decoding everything before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sps {
	/// Which set this is, as a picture parameter set names it.
	pub id:		u8,
	/// The chroma sampling: 0 monochrome, 1 for 4:2:0, 2 for 4:2:2, 3 for 4:4:4.
	pub chroma:	u8,
	/// The coded width in luma samples, before the conformance window is applied.
	pub coded_w:	u32,
	/// The coded height, likewise.
	pub coded_h:	u32,
	/// The width of the picture as it is meant to be shown.
	pub width:	u32,
	/// The height as shown.
	pub height:	u32,
	/// Bits a luma sample.
	pub luma_bits:	u8,
	/// Bits a chroma sample.
	pub chroma_bits:	u8,
	/// The size of a coding tree block, in luma samples: 16, 32 or 64.
	pub ctb_size:	u32,
	/// The smallest coding block, in luma samples.
	pub min_cb:	u32,
	/// The smallest transform block, in luma samples.
	pub min_tb:	u32,
	/// The largest transform block.
	pub max_tb:	u32,
	/// How deep the transform tree may go inside an intra coding unit.
	pub max_depth_intra:	u8,
	/// Whether the sample adaptive offset filter is on.
	pub sao:	bool,
	/// Whether coding units may carry raw samples.
	pub pcm:	bool,
	/// Whether the stronger of the two intra smoothing filters may be used at 32 by 32.
	pub strong_smoothing:	bool,
	/// Whether the scaling lists are in use at all.
	///
	/// **On does not mean bespoke.** Every photograph in the corpus turns them on and carries none
	/// of its own, which means the *default* lists apply -- and those are not flat, so a decoder
	/// that reads this as "no scaling" quantises every block wrongly and produces a picture that is
	/// recognisable and wrong.
	pub scaling_lists:	bool,
	/// The weights themselves, where the lists are in use: this sequence's own where it carries
	/// them, and the default ones where it does not.
	pub weights:	Option<Scaling>,
}

/// What a picture parameter set says about the slices that reference it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pps {
	/// Which set this is, as a slice header names it.
	pub id:		u8,
	/// Which sequence parameter set it belongs to.
	pub sps_id:	u8,
	/// The starting quantisation parameter, already offset by the 26 the syntax subtracts.
	pub init_qp:	i32,
	/// Whether a coding unit may carry its own quantisation delta.
	pub cu_qp_delta:	bool,
	/// How far down the coding quadtree a quantisation delta may be sent.
	pub qp_delta_depth:	u8,
	/// The chroma quantisation offsets.
	pub cb_qp_offset:	i32,
	/// The same for the other chroma channel.
	pub cr_qp_offset:	i32,
	/// Whether a transform block may skip the transform entirely.
	pub transform_skip:	bool,
	/// Whether the sign of the last coefficient is inferred rather than coded.
	pub sign_hiding:	bool,
	/// Whether the residual of an intra block is coded across the transform tree.
	pub transquant_bypass:	bool,
	/// Whether the picture is cut into tiles.
	pub tiles:	bool,
	/// Whether entropy coding is synchronised at the start of each row of blocks.
	pub wavefront:	bool,
	/// Whether the deblocking filter runs.
	pub deblocking:	bool,
	/// Whether a slice header may carry a further quantisation offset for chroma.
	pub slice_chroma_qp:	bool,
	/// How many reserved flags a slice header carries before anything else.
	///
	/// Kept because the slice header cannot be read without it: they are bits to be stepped over,
	/// and stepping over the wrong number puts every field after them one place out.
	pub extra_header_bits:	u8,
	/// Whether a slice header carries a picture output flag.
	pub output_flag:	bool,
	/// Whether a slice header may override the deblocking settings.
	pub deblocking_override:	bool,
	/// Whether the loop filter runs across slice boundaries, and therefore whether a slice header
	/// carries a flag of its own about it.
	pub filter_across_slices:	bool,
}

/// Splits a byte-stream of length-prefixed NAL units, as `hvcC` and `mdat` carry them.
///
/// `length_size` comes from the configuration record and is one, two or four. A unit that runs past
/// the end of the buffer is a truncated file and is refused rather than decoded as far as it goes:
/// half a coded picture is not half a picture, it is noise.
pub fn split_lengthed(bytes: &[u8], length_size: usize) -> Outcome<Vec<Unit>> {
	if !matches!(length_size, 1 | 2 | 4) {
		return Err(err!(
			"A NAL unit length is coded in {} bytes, and only one, two and four are legal.",
			length_size;
		Invalid, Input, Decode));
	}
	let mut out = Vec::new();
	let mut at = 0usize;
	while at + length_size <= bytes.len() {
		let mut len = 0usize;
		for i in 0..length_size {
			len = (len << 8) | bytes[at + i] as usize;
		}
		at += length_size;
		if len == 0 {
			return Err(err!("A NAL unit of no length."; Invalid, Input, Decode));
		}
		let end = match at.checked_add(len) {
			Some(end) if end <= bytes.len() => end,
			_ => return Err(err!(
				"A NAL unit says it is {} bytes and {} remain.", len, bytes.len() - at;
			Invalid, Input, Decode)),
		};
		out.push(res!(unit(&bytes[at..end])));
		at = end;
	}
	if at != bytes.len() {
		return Err(err!(
			"{} bytes are left over after the last NAL unit.", bytes.len() - at;
		Invalid, Input, Decode));
	}
	Ok(out)
}

/// Splits an Annex B stream, where units are separated by start codes rather than lengths.
///
/// This is the form a parameter set arrives in inside `hvcC`, and the form a raw `.265` file takes.
pub fn split_annex_b(bytes: &[u8]) -> Outcome<Vec<Unit>> {
	let mut starts: Vec<usize> = Vec::new();
	let mut i = 0usize;
	while i + 3 <= bytes.len() {
		if bytes[i] == 0 && bytes[i + 1] == 0 && bytes[i + 2] == 1 {
			starts.push(i + 3);
			i += 3;
		} else {
			i += 1;
		}
	}
	let mut out = Vec::with_capacity(starts.len());
	for (n, from) in starts.iter().enumerate() {
		let to = match starts.get(n + 1) {
			// Back off the start code of the next unit, and the trailing zero a four-byte start
			// code puts in front of it.
			Some(next) => {
				let mut end = next - 3;
				if end > *from && bytes[end - 1] == 0 {
					end -= 1;
				}
				end
			},
			None => bytes.len(),
		};
		if to > *from {
			out.push(res!(unit(&bytes[*from..to])));
		}
	}
	Ok(out)
}

/// Reads one NAL unit: its two-byte header, and its payload unescaped.
pub fn unit(raw: &[u8]) -> Outcome<Unit> {
	if raw.len() < 3 {
		return Err(err!(
			"A NAL unit is {} bytes, and its header alone is two.", raw.len();
		Invalid, Input, Decode));
	}
	// forbidden_zero_bit, then six bits of type, six of layer, three of temporal id (§7.3.1.2).
	if raw[0] & 0x80 != 0 {
		return Err(err!(
			"A NAL unit's forbidden bit is set, so this is not an HEVC stream.";
		Invalid, Input, Decode));
	}
	Ok(Unit {
		kind:	(raw[0] >> 1) & 0x3f,
		layer:	raw[1] & 0x07,
		body:	rbsp(&raw[2..]),
		raw:	raw[2..].to_vec(),
	})
}

/// Removes the emulation prevention bytes from a payload (§7.4.2).
///
/// A `0x03` after two zero bytes is there only to stop the payload looking like a start code, and
/// is not part of the syntax.
pub fn rbsp(nal: &[u8]) -> Vec<u8> {
	let mut out = Vec::with_capacity(nal.len());
	let mut zeros = 0usize;
	for &b in nal {
		if zeros >= 2 && b == 0x03 {
			zeros = 0;
			continue;
		}
		out.push(b);
		zeros = if b == 0 { zeros + 1 } else { 0 };
	}
	out
}

/// Where an unescaped position sits in the payload it was unescaped from.
///
/// Emulation prevention only ever *removes* bytes, so the escaped position is the unescaped one
/// plus however many were removed before it. This walks the same state machine [`rbsp`] does rather
/// than inverting it, because the two staying in step is the whole point.
pub fn escaped_at(nal: &[u8], unescaped: usize) -> usize {
	let mut out = 0usize;
	let mut zeros = 0usize;
	for (i, b) in nal.iter().enumerate() {
		if out == unescaped {
			return i;
		}
		if *b == 3 && zeros >= 2 {
			zeros = 0;
			continue;
		}
		out += 1;
		zeros = if *b == 0 { zeros + 1 } else { 0 };
	}
	nal.len()
}

/// The parameter sets carried in an `hvcC` decoder configuration record (ISO/IEC 14496-15 §8.3.3).
///
/// The record's own fields describe the stream's profile and the width of the length prefixes; the
/// arrays at the end carry the parameter sets themselves, as Annex B payloads without start codes.
#[derive(Clone, Debug)]
pub struct Config {
	/// How many bytes prefix each NAL unit in the picture's own data.
	pub length_size:	usize,
	/// Every parameter set the record carries, in the order it carries them.
	pub sets:	Vec<Unit>,
}

/// Reads an `hvcC` record.
pub fn config(bytes: &[u8]) -> Outcome<Config> {
	// 22 bytes of fixed fields, then a count of arrays.
	if bytes.len() < 23 {
		return Err(err!(
			"A decoder configuration record is {} bytes, and its fixed fields alone are 22.",
			bytes.len();
		Invalid, Input, Decode));
	}
	if bytes[0] != 1 {
		return Err(err!(
			"A decoder configuration record of version {}, and this reads version 1.", bytes[0];
		Invalid, Input, Unknown));
	}
	let length_size = (bytes[21] & 0x03) as usize + 1;
	let arrays = bytes[22] as usize;
	let mut sets = Vec::new();
	let mut at = 23usize;
	for _ in 0..arrays {
		if at + 3 > bytes.len() {
			return Err(err!(
				"A configuration record ends inside its array of parameter sets.";
			Invalid, Input, Decode));
		}
		let count = u16::from_be_bytes([bytes[at + 1], bytes[at + 2]]) as usize;
		at += 3;
		for _ in 0..count {
			if at + 2 > bytes.len() {
				return Err(err!(
					"A configuration record ends inside a parameter set's length.";
				Invalid, Input, Decode));
			}
			let len = u16::from_be_bytes([bytes[at], bytes[at + 1]]) as usize;
			at += 2;
			let end = match at.checked_add(len) {
				Some(end) if end <= bytes.len() => end,
				_ => return Err(err!(
					"A parameter set says it is {} bytes and {} remain.",
					len, bytes.len() - at;
				Invalid, Input, Decode)),
			};
			sets.push(res!(unit(&bytes[at..end])));
			at = end;
		}
	}
	Ok(Config { length_size, sets })
}

/// A reader of the bits of an RBSP, most significant first.
pub struct Bits<'a> {
	/// The bytes being read.
	buf:	&'a [u8],
	/// The next bit, counted from the first bit of the first byte.
	pos:	usize,
}

impl<'a> Bits<'a> {

	/// A reader positioned at the first bit.
	pub fn new(buf: &'a [u8]) -> Self {
		Self { buf, pos: 0 }
	}

	/// How many bits are left.
	pub fn left(&self) -> usize {
		(self.buf.len() * 8).saturating_sub(self.pos)
	}

	/// The next `n` bits as an unsigned integer, most significant first.
	pub fn u(&mut self, n: usize) -> Outcome<u32> {
		if n > 32 {
			return Err(err!("A field of {} bits was asked for, and 32 is the widest.", n; Bug));
		}
		let mut v = 0u32;
		for _ in 0..n {
			let byte = self.pos >> 3;
			if byte >= self.buf.len() {
				return Err(err!(
					"The parameter set ends after {} bits, inside a field.", self.buf.len() * 8;
				Invalid, Input, Decode));
			}
			let bit = (self.buf[byte] >> (7 - (self.pos & 7))) & 1;
			v = (v << 1) | bit as u32;
			self.pos += 1;
		}
		Ok(v)
	}

	/// The next bit as a flag.
	pub fn flag(&mut self) -> Outcome<bool> {
		Ok(res!(self.u(1)) == 1)
	}

	/// How many bits have been read.
	pub fn consumed(&self) -> Outcome<usize> {
		Ok(self.pos)
	}

	/// Steps over `n` bits.
	pub fn skip(&mut self, n: usize) -> Outcome<()> {
		for _ in 0..n / 32 {
			let _ = res!(self.u(32));
		}
		let _ = res!(self.u(n % 32));
		Ok(())
	}

	/// An unsigned Exp-Golomb code, §9.2.
	pub fn ue(&mut self) -> Outcome<u32> {
		let mut zeros = 0usize;
		while res!(self.u(1)) == 0 {
			zeros += 1;
			if zeros > 31 {
				return Err(err!(
					"An Exp-Golomb code is prefixed by more than 31 zeroes, which no legal value \
					is.";
				Invalid, Input, Decode));
			}
		}
		if zeros == 0 {
			return Ok(0);
		}
		let rest = res!(self.u(zeros)) as u64;
		let v = (1u64 << zeros) - 1 + rest;
		if v > u32::MAX as u64 {
			return Err(err!(
				"An Exp-Golomb code decodes to {}, beyond what any field holds.", v;
			Invalid, Input, Decode));
		}
		Ok(v as u32)
	}

	/// A signed Exp-Golomb code, §9.2.2.
	pub fn se(&mut self) -> Outcome<i32> {
		let k = res!(self.ue());
		let m = ((k as i64 + 1) / 2) as i32;
		Ok(if k % 2 == 1 { m } else { -m })
	}
}

/// Steps over a profile, tier and level structure (§7.3.3).
///
/// Nothing in it changes how a picture is decoded -- it says what a decoder must be capable of, and
/// a decoder that is about to try is going to find out. It has to be walked rather than skipped by
/// a byte count only in the sub-layer case, where the number of flags depends on the flags.
fn profile_tier_level(b: &mut Bits, profile_present: bool, max_sub_layers: usize) -> Outcome<()> {
	if profile_present {
		// 2 + 1 + 5 bits, 32 of compatibility flags, 48 of constraint flags.
		res!(b.skip(8 + 32 + 48));
	}
	res!(b.skip(8));
	if max_sub_layers == 0 {
		return Ok(());
	}
	let mut profile = [false; 8];
	let mut level = [false; 8];
	for i in 0..max_sub_layers.saturating_sub(1).min(8) {
		profile[i] = res!(b.flag());
		level[i] = res!(b.flag());
	}
	if max_sub_layers > 1 {
		// The flags are padded out to eight pairs.
		for _ in max_sub_layers.saturating_sub(1)..8 {
			res!(b.skip(2));
		}
	}
	for i in 0..max_sub_layers.saturating_sub(1).min(8) {
		if profile[i] {
			res!(b.skip(8 + 32 + 48));
		}
		if level[i] {
			res!(b.skip(8));
		}
	}
	Ok(())
}

/// The weights a picture quantises each block against (§7.3.4, §7.4.5).
///
/// Six lists a size -- one each for the three colour components, predicted from within the picture
/// and from another, though a still photograph only ever uses the first three. The numbers climb
/// away from the corner because the eye notices an error in a block's coarse detail more than in
/// its fine, so the fine detail is quantised harder.
///
/// A sequence that turns the lists on and carries none of its own takes the default ones, which are
/// not flat; a decoder that reads "on" as "no scaling" quantises every block wrongly and produces a
/// picture that is recognisable and wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scaling {
	/// `ScalingList[sizeId][matrixId][i]`, in the diagonal scan's order. Sixteen entries are used
	/// at the smallest size and sixty-four at the other three.
	pub list:	[[[u8; 64]; 6]; 4],
	/// What sits in the corner at the two largest sizes, which is coded on its own.
	pub dc:		[[u8; 6]; 2],
}

impl Scaling {

	/// The default lists, which is what a sequence carrying none of its own means.
	pub fn default_lists() -> Self {
		let mut out = Self { list: [[[16u8; 64]; 6]; 4], dc: [[16u8; 6]; 2] };
		for size in 1..4 {
			for id in 0..6 {
				let from = crate::hevc::transform::DEFAULT_LIST[(id >= 3) as usize];
				out.list[size][id] = from;
			}
		}
		out
	}

	/// One weight, by size, matrix and position within the block (equations 7-44 to 7-49).
	///
	/// The sixteen and thirty-two sample matrices are the eight-sample one with each of its values
	/// covering two or four samples each way, and a corner of their own.
	pub fn factor(&self, log2: u32, matrix: usize, x: usize, y: usize, raster: &[u8; 64]) -> i32 {
		match log2 {
			2	=> raster[(y & 3) * 4 + (x & 3)] as i32,
			3	=> raster[y * 8 + x] as i32,
			_ => {
				if x == 0 && y == 0 {
					return self.dc[(log2 - 4) as usize][matrix] as i32;
				}
				let shrink = log2 - 3;
				raster[(y >> shrink) * 8 + (x >> shrink)] as i32
			},
		}
	}

	/// One list laid out in raster order rather than in the diagonal scan's.
	pub fn raster(&self, log2: u32, matrix: usize) -> [u8; 64] {
		let size_id = (log2 - 2).min(3) as usize;
		let side = if size_id == 0 { 4 } else { 8 };
		let list = &self.list[size_id][matrix];
		let mut out = [16u8; 64];
		for (i, (x, y)) in crate::hevc::scan::positions(side, crate::hevc::scan::Order::Diagonal)
			.iter()
			.enumerate()
		{
			out[*y as usize * side + *x as usize] = list[i];
		}
		out
	}
}

/// Reads a scaling list (§7.3.4).
///
/// A list is either coded outright as a chain of differences, or taken from an earlier list in the
/// same set, or -- where it names itself as its own source -- from the default.
fn scaling_list(b: &mut Bits) -> Outcome<Scaling> {
	let mut out = Scaling::default_lists();
	let defaults = Scaling::default_lists();
	for size in 0..4usize {
		let mut id = 0usize;
		while id < 6 {
			let coefficients = 64usize.min(1 << (4 + (size << 1)));
			if !res!(b.flag()) {
				// Taken from another list rather than coded. A delta of nought means the default,
				// which is the one case where "predicted from" does not mean "copied from".
				let delta = res!(b.ue()) as usize;
				if delta == 0 {
					out.list[size][id] = defaults.list[size][id];
					if size > 1 {
						out.dc[size - 2][id] = 16;
					}
				} else {
					let from = id.saturating_sub(delta * if size == 3 { 3 } else { 1 });
					out.list[size][id] = out.list[size][from];
					if size > 1 {
						out.dc[size - 2][id] = out.dc[size - 2][from];
					}
				}
			} else {
				let mut next = 8i32;
				if size > 1 {
					let dc = res!(b.se()) + 8;
					if !(1..=255).contains(&dc) {
						return Err(err!(
							"A scaling list's corner value is {}, outside 1 to 255.", dc;
						Invalid, Input, Decode));
					}
					out.dc[size - 2][id] = dc as u8;
					next = dc;
				}
				for i in 0..coefficients {
					let delta = res!(b.se());
					next = (next + delta + 256).rem_euclid(256);
					out.list[size][id][i] = next as u8;
				}
			}
			// The 32 by 32 lists come in twos rather than sixes.
			id += if size == 3 { 3 } else { 1 };
		}
	}
	// A 32 by 32 chroma list does not exist below 4:4:4, but the arrays are square; filling the
	// gaps from luma keeps a lookup by matrix identifier from finding sixteens.
	for id in [1usize, 2, 4, 5] {
		let from = if id < 3 { 0 } else { 3 };
		out.list[3][id] = out.list[3][from];
		out.dc[1][id] = out.dc[1][from];
	}
	Ok(out)
}

/// Steps over one short-term reference picture set (§7.3.7).
///
/// A still picture references nothing, so nothing here is kept -- but a sequence parameter set is
/// allowed to carry these before fields that *are* wanted, and they are variable-length.
fn short_term_ref_pic_set(b: &mut Bits, idx: usize, count: usize, previous: &mut Vec<(u32, u32)>)
	-> Outcome<()>
{
	let mut predicted = false;
	if idx != 0 {
		predicted = res!(b.flag());
	}
	if predicted {
		if idx == count {
			let _ = res!(b.ue());
		}
		let _delta_rps_sign = res!(b.flag());
		let _abs_delta_rps = res!(b.ue());
		let (negative, positive) = previous.last().copied().unwrap_or((0, 0));
		for _ in 0..(negative + positive + 1) {
			let used = res!(b.flag());
			if !used {
				let _use_delta = res!(b.flag());
			}
		}
		// The count after prediction cannot be worked out without keeping the whole set, and this
		// decoder does not need it: what matters is that the bits have been consumed.
		previous.push((negative, positive));
		return Ok(());
	}
	let negative = res!(b.ue());
	let positive = res!(b.ue());
	if negative > 64 || positive > 64 {
		return Err(err!(
			"A reference picture set names {} and {} pictures, and 64 is the most either may be.",
			negative, positive;
		Invalid, Input, Decode));
	}
	for _ in 0..negative {
		let _delta = res!(b.ue());
		let _used = res!(b.flag());
	}
	for _ in 0..positive {
		let _delta = res!(b.ue());
		let _used = res!(b.flag());
	}
	previous.push((negative, positive));
	Ok(())
}

/// Reads a sequence parameter set (§7.3.2.2).
pub fn sps(body: &[u8]) -> Outcome<Sps> {
	let mut b = Bits::new(body);
	let _vps_id = res!(b.u(4));
	let max_sub_layers = res!(b.u(3)) as usize + 1;
	let _temporal_id_nesting = res!(b.flag());
	res!(profile_tier_level(&mut b, true, max_sub_layers));
	let id = res!(b.ue());
	if id > 15 {
		return Err(err!(
			"A sequence parameter set numbered {}, and 15 is the highest.", id;
		Invalid, Input, Decode));
	}
	let chroma = res!(b.ue());
	if chroma > 3 {
		return Err(err!(
			"A chroma format of {}, and 3 is the highest.", chroma; Invalid, Input, Decode));
	}
	if chroma == 3 {
		let _separate_colour_plane = res!(b.flag());
	}
	let coded_w = res!(b.ue());
	let coded_h = res!(b.ue());
	if coded_w == 0 || coded_h == 0 || coded_w > MAX_SIDE || coded_h > MAX_SIDE {
		return Err(err!(
			"A sequence parameter set codes a picture of {} by {}.", coded_w, coded_h;
		Invalid, Input, Decode));
	}
	// The conformance window trims the coded picture down to what is shown, in units of the chroma
	// sampling: a 1920 by 1080 picture is coded as 1920 by 1088 and trimmed by four rows.
	let (mut left, mut right, mut top, mut bottom) = (0u32, 0u32, 0u32, 0u32);
	if res!(b.flag()) {
		left = res!(b.ue());
		right = res!(b.ue());
		top = res!(b.ue());
		bottom = res!(b.ue());
	}
	let (sub_w, sub_h) = match chroma {
		1 => (2u32, 2u32),
		2 => (2, 1),
		_ => (1, 1),
	};
	let trim_x = left.saturating_add(right).saturating_mul(sub_w);
	let trim_y = top.saturating_add(bottom).saturating_mul(sub_h);
	if trim_x >= coded_w || trim_y >= coded_h {
		return Err(err!(
			"A conformance window trims {} by {} from a picture of {} by {}.",
			trim_x, trim_y, coded_w, coded_h;
		Invalid, Input, Decode));
	}
	let luma_bits = res!(b.ue()) as u8 + 8;
	let chroma_bits = res!(b.ue()) as u8 + 8;
	if luma_bits > 16 || chroma_bits > 16 {
		return Err(err!(
			"A sample of {} bits, and 16 is the most this decoder reads.", luma_bits.max(chroma_bits);
		Invalid, Input, Unknown));
	}
	let _log2_max_poc = res!(b.ue());
	// The ordering information is given either once for the highest sub-layer or once for each.
	let for_each = res!(b.flag());
	let first = if for_each { 0 } else { max_sub_layers - 1 };
	for _ in first..max_sub_layers {
		let _max_dec_pic_buffering = res!(b.ue());
		let _num_reorder = res!(b.ue());
		let _max_latency = res!(b.ue());
	}
	let min_cb = 1u32 << (res!(b.ue()) + 3);
	let ctb_size = min_cb << res!(b.ue());
	let min_tb = 1u32 << (res!(b.ue()) + 2);
	let max_tb = min_tb << res!(b.ue());
	if !matches!(ctb_size, 16 | 32 | 64) || min_cb < 8 || max_tb > 32 || min_tb < 4 {
		return Err(err!(
			"A block geometry of ctb {}, min cb {}, tb {} to {}, which no legal stream has.",
			ctb_size, min_cb, min_tb, max_tb;
		Invalid, Input, Decode));
	}
	let _max_depth_inter = res!(b.ue());
	let max_depth_intra = res!(b.ue()) as u8;
	let scaling_lists = res!(b.flag());
	let mut weights = None;
	if scaling_lists {
		weights = Some(if res!(b.flag()) {
			res!(scaling_list(&mut b))
		} else {
			Scaling::default_lists()
		});
	}
	let _amp = res!(b.flag());
	let sao = res!(b.flag());
	let pcm = res!(b.flag());
	if pcm {
		let _pcm_luma_bits = res!(b.u(4));
		let _pcm_chroma_bits = res!(b.u(4));
		let _log2_min_pcm_cb = res!(b.ue());
		let _log2_diff_pcm_cb = res!(b.ue());
		let _pcm_loop_filter_disabled = res!(b.flag());
	}
	let short_term_sets = res!(b.ue()) as usize;
	if short_term_sets > 64 {
		return Err(err!(
			"A sequence parameter set carries {} reference picture sets, and 64 is the most.",
			short_term_sets;
		Invalid, Input, Decode));
	}
	let mut previous: Vec<(u32, u32)> = Vec::with_capacity(short_term_sets);
	for i in 0..short_term_sets {
		res!(short_term_ref_pic_set(&mut b, i, short_term_sets, &mut previous));
	}
	if res!(b.flag()) {
		let long_term = res!(b.ue()) as usize;
		if long_term > 32 {
			return Err(err!(
				"A sequence parameter set carries {} long-term reference pictures.", long_term;
			Invalid, Input, Decode));
		}
		for _ in 0..long_term {
			let bits = (res!(b.ue()) % 32) as usize;
			let _ = bits;
			// The poc is coded in log2_max_poc bits, which was read past above; a still picture
			// has none of these, and a stream that does is not one this decoder will be handed.
			return Err(err!(
				"A sequence parameter set carries long-term reference pictures, which a still \
				picture does not have.";
			Invalid, Input, Unknown));
		}
	}
	let _temporal_mvp = res!(b.flag());
	let strong_smoothing = res!(b.flag());
	Ok(Sps {
		id: id as u8,
		chroma: chroma as u8,
		coded_w,
		coded_h,
		width: coded_w - trim_x,
		height: coded_h - trim_y,
		luma_bits,
		chroma_bits,
		ctb_size,
		min_cb,
		min_tb,
		max_tb,
		max_depth_intra,
		sao,
		pcm,
		strong_smoothing,
		scaling_lists,
		weights,
	})
}

/// Reads a picture parameter set (§7.3.2.3).
pub fn pps(body: &[u8]) -> Outcome<Pps> {
	let mut b = Bits::new(body);
	let id = res!(b.ue());
	let sps_id = res!(b.ue());
	if id > 63 || sps_id > 15 {
		return Err(err!(
			"A picture parameter set numbered {} against sequence set {}.", id, sps_id;
		Invalid, Input, Decode));
	}
	let _dependent_slice_segments = res!(b.flag());
	let output_flag = res!(b.flag());
	let extra_header_bits = res!(b.u(3)) as u8;
	let sign_hiding = res!(b.flag());
	let _cabac_init_present = res!(b.flag());
	let _num_ref_idx_l0 = res!(b.ue());
	let _num_ref_idx_l1 = res!(b.ue());
	let init_qp = res!(b.se()) + 26;
	let _constrained_intra_pred = res!(b.flag());
	let transform_skip = res!(b.flag());
	let cu_qp_delta = res!(b.flag());
	let qp_delta_depth = if cu_qp_delta { res!(b.ue()) as u8 } else { 0 };
	let cb_qp_offset = res!(b.se());
	let cr_qp_offset = res!(b.se());
	let slice_chroma_qp = res!(b.flag());
	let _weighted_pred = res!(b.flag());
	let _weighted_bipred = res!(b.flag());
	let transquant_bypass = res!(b.flag());
	let tiles = res!(b.flag());
	let wavefront = res!(b.flag());
	if tiles {
		// The geometry of the tiles is read past rather than kept: what this decoder needs from a
		// tiled picture is to know it is one, and to say so.
		let columns = res!(b.ue()) as usize;
		let rows = res!(b.ue()) as usize;
		if columns > 1024 || rows > 1024 {
			return Err(err!(
				"A picture in {} by {} tiles.", columns + 1, rows + 1; Invalid, Input, Decode));
		}
		if !res!(b.flag()) {
			for _ in 0..columns {
				let _width = res!(b.ue());
			}
			for _ in 0..rows {
				let _height = res!(b.ue());
			}
		}
		let _loop_filter_across_tiles = res!(b.flag());
	}
	let filter_across_slices = res!(b.flag());
	let mut deblocking = true;
	let mut deblocking_override = false;
	if res!(b.flag()) {
		deblocking_override = res!(b.flag());
		deblocking = !res!(b.flag());
		if deblocking {
			let _beta_offset = res!(b.se());
			let _tc_offset = res!(b.se());
		}
	}
	Ok(Pps {
		id: id as u8,
		sps_id: sps_id as u8,
		init_qp,
		cu_qp_delta,
		qp_delta_depth,
		cb_qp_offset,
		cr_qp_offset,
		transform_skip,
		sign_hiding,
		transquant_bypass,
		tiles,
		wavefront,
		deblocking,
		slice_chroma_qp,
		extra_header_bits,
		output_flag,
		deblocking_override,
		filter_across_slices,
	})
}

/// What a slice segment header says, for the one kind of slice a still picture has.
///
/// A picture is one slice and the slice is intra, so most of the syntax -- reference lists,
/// weighted prediction, temporal motion vectors -- is not reached at all. What matters here is
/// where the header **ends**: the arithmetic decoder starts at the next byte boundary after it, and
/// a header read one bit short starts the whole of the rest of the decode in the wrong place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slice {
	/// Which picture parameter set the slice references.
	pub pps_id:	u8,
	/// The slice type: 2 is intra, and this decoder reads no other.
	pub kind:	u8,
	/// The quantisation parameter this slice starts at.
	pub qp:		i32,
	/// Whether the sample adaptive offset filter runs on luma in this slice.
	pub sao_luma:	bool,
	/// Whether it runs on chroma.
	pub sao_chroma:	bool,
	/// Where the entropy-coded data begins, as a byte offset into the payload.
	///
	/// This is the header's own end rounded up to a byte, which is where §9.3.1 says the
	/// arithmetic decoder is initialised from.
	pub data_at:	usize,
	/// Whether the deblocking filter runs on this slice.
	pub deblocking:	bool,
	/// What this slice adds to the picture's own chroma quantisation offsets.
	pub cb_qp_offset:	i32,
	/// The same for the other chroma component.
	pub cr_qp_offset:	i32,
	/// Where each piece after the first begins, as the length in bytes of the piece before it.
	///
	/// One a row of coding tree blocks, under wavefront coding, which is what every photograph
	/// measured uses.
	pub entries:	Vec<u64>,
}

/// Reads a slice segment header (§7.3.6.1).
///
/// Only the independent, intra case: a dependent slice segment continues another one's context and
/// a still picture has no reason to carry one, so it is refused by name.
pub fn slice(body: &[u8], sps: &Sps, pps: &Pps) -> Outcome<Slice> {
	let mut b = Bits::new(body);
	let first = res!(b.flag());
	if !first {
		return Err(err!(
			"A slice segment that is not the first of its picture. A still picture is one slice.";
		Invalid, Input, Unknown));
	}
	// Only an IRAP picture carries this flag, and every still is one.
	let _no_output_of_prior_pics = res!(b.flag());
	let pps_id = res!(b.ue());
	if pps_id as u8 != pps.id {
		return Err(err!(
			"A slice references picture parameter set {} and the one in hand is {}.",
			pps_id, pps.id;
		Invalid, Input, Missing));
	}
	let kind = res!(b.ue());
	if kind != 2 {
		return Err(err!(
			"A slice of type {}, and a still picture's slices are all intra (type 2).", kind;
		Invalid, Input, Unknown));
	}
	if pps.output_flag {
		let _pic_output_flag = res!(b.flag());
	}
	// Reserved, and to be stepped over rather than understood. Stepping over the wrong number of
	// them puts every field after them one place out, which is why the count is carried here from
	// the picture parameter set rather than assumed to be zero.
	res!(b.skip(pps.extra_header_bits as usize));
	let mut sao_luma = false;
	let mut sao_chroma = false;
	if sps.sao {
		sao_luma = res!(b.flag());
		if sps.chroma != 0 {
			sao_chroma = res!(b.flag());
		}
	}
	let qp = pps.init_qp + res!(b.se());
	if qp < -(6 * (sps.luma_bits as i32 - 8)) || qp > 51 {
		return Err(err!(
			"A slice starts at a quantisation parameter of {}, outside the legal range.", qp;
		Invalid, Input, Decode));
	}
	// The chroma offsets a slice may add to the picture's own. Kept rather than stepped over:
	// they go into the chroma quantisation parameter of every block, so a picture whose slice
	// carries one and whose decoder ignores it comes out with the wrong colour saturation.
	let (mut cb_offset, mut cr_offset) = (0i32, 0i32);
	if pps.slice_chroma_qp {
		cb_offset = res!(b.se());
		cr_offset = res!(b.se());
	}
	let mut deblocking = pps.deblocking;
	if pps.deblocking_override && res!(b.flag()) {
		deblocking = !res!(b.flag());
		if deblocking {
			let _beta = res!(b.se());
			let _tc = res!(b.se());
		}
	}
	if pps.filter_across_slices && (sao_luma || sao_chroma || deblocking) {
		let _across = res!(b.flag());
	}
	// Where the picture is cut up for parallel decoding, the header says where each piece begins.
	//
	// **A still photograph out of a phone is coded this way.** Every one of the 359 HEIC files
	// measured sets `entropy_coding_sync_enabled_flag`, which is wavefront coding: the arithmetic
	// decoder is reset at the start of every row of coding tree blocks, from the state saved after
	// the second block of the row above. So this is not an exotic case to be refused -- it is the
	// case, and the offsets below are how the rows are found.
	let mut entries: Vec<u64> = Vec::new();
	if pps.tiles || pps.wavefront {
		let count = res!(b.ue()) as usize;
		if count > 4096 {
			return Err(err!(
				"A slice names {} entry points, and no picture this decoder reads has so many.",
				count;
			Invalid, Input, Decode));
		}
		if count > 0 {
			let width = res!(b.ue()) as usize + 1;
			if width > 32 {
				return Err(err!(
					"An entry point offset of {} bits, and 32 is the widest.", width;
				Invalid, Input, Decode));
			}
			for _ in 0..count {
				entries.push(res!(b.u(width)) as u64 + 1);
			}
		}
		// Under wavefront coding there is one piece per row of coding tree blocks, so the count
		// has to agree with the picture's own geometry -- and the geometry came out of the
		// sequence parameter set, a different NAL unit written at a different time. A slice header
		// read one bit out of step produces a count that is nonsense against it, which makes this
		// the cheapest check there is on the whole header: it is what caught the reading that
		// refused every photograph in the corpus rather than reading its entry points.
		if pps.wavefront && !pps.tiles {
			let rows = ((sps.coded_h + sps.ctb_size - 1) / sps.ctb_size) as usize;
			if entries.len() + 1 != rows {
				return Err(err!(
					"A slice names {} pieces and the picture is {} rows of coding tree blocks \
					deep. The header has been read out of step.", entries.len() + 1, rows;
				Invalid, Input, Decode));
			}
		}
	}
	// The header ends with a stop bit and however many zeroes reach the byte boundary, and the
	// arithmetic decoder starts at that boundary (§9.3.1).
	let bits = res!(b.consumed());
	let data_at = (bits + 1 + 7) / 8;
	if data_at >= body.len() {
		return Err(err!(
			"A slice header of {} bits leaves no data in a payload of {} bytes.", bits, body.len();
		Invalid, Input, Decode));
	}
	Ok(Slice {
		pps_id: pps_id as u8,
		kind: kind as u8,
		qp,
		cb_qp_offset:	cb_offset,
		cr_qp_offset:	cr_offset,
		sao_luma,
		sao_chroma,
		data_at,
		deblocking,
		entries,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_emulation_prevention_is_undone_00() -> Outcome<()> {
		// A 0x03 after two zeroes is not payload; one after a single zero is.
		req!(rbsp(&[0, 0, 3, 1, 0, 3, 2]), vec![0u8, 0, 1, 0, 3, 2]);
		Ok(())
	}

	#[test]
	fn test_a_truncated_nal_unit_is_refused_01() -> Outcome<()> {
		// Four bytes of length saying eight, with three following.
		let stream = [0u8, 0, 0, 8, 0x26, 1, 9];
		req!(split_lengthed(&stream, 4).is_err(), true,
			"A unit running past the end of the buffer was read as if it fitted.");
		Ok(())
	}







	#[test]
	fn test_exp_golomb_reads_the_codes_the_specification_names_02() -> Outcome<()> {
		// 1 -> 0, 010 -> 1, 011 -> 2, 00100 -> 3, and the signed mapping 0, 1, -1, 2, -2.
		let mut b = Bits::new(&[0b1010_0110, 0b0100_0000]);
		req!(res!(b.ue()), 0);
		req!(res!(b.ue()), 1);
		req!(res!(b.ue()), 2);
		req!(res!(b.ue()), 3);
		let mut c = Bits::new(&[0b1010_0110, 0b0100_0000]);
		req!(res!(c.se()), 0);
		req!(res!(c.se()), 1);
		req!(res!(c.se()), -1);
		req!(res!(c.se()), 2);
		Ok(())
	}
}

// ---------------------------------------------------------------- the whole of one picture

/// Decodes one coded picture: an HEIC tile, or a whole photograph that was not cut into tiles.
///
/// `config` is the `hvcC` record from the container, which carries the parameter sets, and `data`
/// is the item's bytes -- NAL units with a length prefix each, which is how a HEIF file stores
/// them rather than with start codes.
///
/// The picture comes back in 4:2:0 at whatever depth it was coded, which for every photograph this
/// was written against is eight bits. It has **not** been through the deblocking filter or the
/// sample adaptive offset, which are separate passes over a finished picture.
pub fn picture(record: &[u8], data: &[u8]) -> Outcome<decode::Picture> {
	let cfg = res!(config(record));
	let mut seq: Option<Sps> = None;
	let mut pic: Option<Pps> = None;
	// The parameter sets ride in the configuration record, in Annex B form.
	for unit in &cfg.sets {
		match unit.kind {
			nal::SPS => seq = Some(res!(sps(&unit.body))),
			nal::PPS => pic = Some(res!(pps(&unit.body))),
			_ => {},
		}
	}
	let sps = match seq {
		Some(s) => s,
		None => return Err(err!(
			"The decoder configuration carries no sequence parameter set."; Invalid, Input)),
	};
	let pps = match pic {
		Some(p) => p,
		None => return Err(err!(
			"The decoder configuration carries no picture parameter set."; Invalid, Input)),
	};

	// And the slice is in the item's own bytes.
	let units = res!(split_lengthed(data, cfg.length_size));
	for unit in &units {
		match unit.kind {
			nal::IDR_W_RADL | nal::IDR_N_LP | 21 => {
				let head = res!(slice(&unit.body, &sps, &pps));
				// The header was read from the unescaped payload; the data after it has to be
				// handed over escaped, because that is what the entry point offsets count.
				let at = escaped_at(&unit.raw, head.data_at);
				return decode::picture(&sps, &pps, &head, &unit.raw[at..]);
			},
			_ => {},
		}
	}
	Err(err!("Those bytes hold no coded slice."; Invalid, Input, Decode))
}
