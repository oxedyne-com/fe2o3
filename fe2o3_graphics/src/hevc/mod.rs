//! An HEVC decoder, for the still pictures inside a HEIC file.
//!
//! HEVC (ITU-T H.265) is the codec a phone's photograph is coded in once it stops being JPEG, and
//! there is no way to read one without decoding it. This module is that decoder. It is built for
//! **intra** coding only -- a still picture refers to nothing but itself, so everything about
//! motion, reference pictures and prediction between frames is absent by construction rather than
//! unimplemented.
//!
//! # What is here
//!
//! The whole of it, for the intra case. The bitstream side: splitting a stream into NAL units,
//! undoing the emulation-prevention bytes, and reading the sequence and picture parameter sets that
//! say how big the picture is and how it is cut up. That is the part every later stage is written
//! against, and the part that can be checked before any pixel exists: the size a sequence parameter
//! set codes must agree with the size the HEIF container's `ispe` property declares, and those two
//! numbers are written into the file by different parts of an encoder.
//!
//! Then the slice segment header, including the entry points that say where each row of the picture
//! begins; the CABAC arithmetic decoder; the context variables each syntax element uses; the coding
//! quadtree; residual coding; dequantisation and the inverse transforms; all thirty-five intra
//! prediction modes; deblocking and the sample adaptive offset; and the conversion out of 4:2:0
//! into red, green and blue. [`picture`] is the entry point and runs the lot.
//!
//! The arithmetic decoder is the last piece that can be held to a standard before a picture comes
//! out, and it is held to two: every context starts in a state the probability tables actually
//! have -- all 256 initialisation values against every quantisation parameter a slice may carry --
//! and the coding interval is between 256 and 510 after every bin, whatever is fed in. A
//! renormalisation one shift short satisfies neither and decodes plausible rubbish rather than
//! failing, which is the kind of fault that otherwise survives until a photograph comes out
//! wrong.
//!
//! Everything after it is held to another decoder instead, because by then there is a picture to
//! compare: `tests/hevc_tiles.rs` puts every brightness and colour sample beside what FFmpeg makes
//! of the same file, with both loop filters running at both ends.
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
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

pub mod cabac;
pub mod colour;
pub mod decode;
pub mod filter;
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

// The largest picture this decoder will describe, in luma samples each way. Sixteen thousand is
// past every camera and well inside what the level limits allow; it is a ceiling against a
// parameter set that is a mistake, not a limit on real photographs.
pub const MAX_SIDE: u32 = 16_384;

/// NAL unit types this decoder cares about (H.265 Table 7-1).
pub mod nal {
	pub const IDR_W_RADL: u8 = 19;	// an IDR picture with no leading pictures, which a still is
	pub const IDR_N_LP: u8 = 20;	// the other IDR form
	pub const VPS: u8 = 32;
	pub const SPS: u8 = 33;
	pub const PPS: u8 = 34;
}

/// One NAL unit: what it is, and its payload with the emulation prevention undone.
#[derive(Clone, Debug)]
pub struct Unit {
	pub kind:	u8,			// the type, from the two-byte NAL unit header
	pub layer:	u8,			// temporal sub-layer, plus one as the header codes it
	pub body:	Vec<u8>,	// after the header, every emulation prevention byte removed
	// The payload as it arrived, escaping and all, because the entry point offsets in a slice
	// header are counted in escaped bytes: "emulation prevention bytes that appear in the slice
	// segment data portion of the coded slice segment NAL unit are counted as part of the slice
	// segment data for purposes of subset identification" (§7.4.7.1). Splitting the unescaped
	// payload at those offsets puts every row of blocks after the first escaped byte in the
	// wrong place.
	pub raw:	Vec<u8>,
}

/// What a sequence parameter set says about the pictures that follow it.
///
/// Only the fields a still picture's decoder acts on are kept. The rest are read past, because a
/// parameter set is a run of variable-length codes and there is no skipping to a field without
/// decoding everything before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sps {
	pub id:		u8,						// which set this is, as a picture parameter set names it
	pub chroma:	u8,						// 0 monochrome, 1 for 4:2:0, 2 for 4:2:2, 3 for 4:4:4
	pub coded_w:	u32,				// coded width in luma samples, before the conformance window
	pub coded_h:	u32,				// and coded height
	pub width:	u32,					// the width the picture is meant to be shown at
	pub height:	u32,					// and the height as shown
	pub luma_bits:	u8,					// bits a luma sample
	pub chroma_bits:	u8,				// bits a chroma sample
	pub ctb_size:	u32,				// a coding tree block, in luma samples: 16, 32 or 64
	pub min_cb:	u32,					// the smallest coding block, in luma samples
	pub min_tb:	u32,					// the smallest transform block, in luma samples
	pub max_tb:	u32,					// and the largest
	pub max_depth_intra:	u8,			// how deep the transform tree may go in an intra unit
	pub sao:	bool,					// is the sample adaptive offset filter on?
	pub pcm:	bool,					// may coding units carry raw samples?
	pub strong_smoothing:	bool,		// the stronger intra smoothing filter, at 32 by 32
	// On does not mean bespoke. Every photograph in the corpus turns the scaling lists on and
	// carries none of its own, which means the default lists apply -- and those are not flat, so
	// a decoder that reads this as "no scaling" quantises every block wrongly and produces a
	// picture that is recognisable and wrong.
	pub scaling_lists:	bool,			// are the scaling lists in use at all?
	pub weights:	Option<Scaling>,	// this sequence's own lists, or the default ones
	// Both windows may sit off the top left corner: the conformance window usually does not and
	// the default display window of a stabilised film always does, since it is centred in a
	// picture coded larger than it shows. Cropping from the corner instead moves the whole
	// picture by that offset.
	pub show_x0:	u32,				// where the shown picture begins, in luma samples
	pub show_y0:	u32,				// the same downwards
	// Out of the video usability information, where a stream says how it is to be shown. A
	// conversion into red, green and blue that guesses this wrong makes a photograph with no
	// real black in it, or one whose blacks are crushed.
	pub full_range:	bool,				// full range rather than the studio one
	pub matrix:	u8,						// ISO/IEC 23091-2: 1 high definition, 5 and 6 standard
	// A slice header carries the picture order count for every picture except an IDR, which has
	// none to state. A film's first frame is very often a clean random access picture rather
	// than an IDR, and a header read as though it were an IDR's is read out of step from this
	// field on.
	pub poc_bits:	u8,					// bits the count's lower part is coded in
	// A still picture references nothing and needs none of these sets; what they are for is the
	// slice header, which may name one or write a new one predicted from them, and either way
	// the bits cannot be stepped over without knowing how large the set referred to is.
	pub st_sets:	Vec<(u32, u32)>,	// pictures each short-term set names, negative and positive
	pub long_term:	bool,				// may a slice header name long-term reference pictures?
	pub temporal_mvp:	bool,			// does a slice header carry the temporal predictor flag?
}

/// What a picture parameter set says about the slices that reference it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pps {
	pub id:		u8,						// which set this is, as a slice header names it
	pub sps_id:	u8,						// which sequence parameter set it belongs to
	pub init_qp:	i32,				// already offset by the 26 the syntax subtracts
	pub cu_qp_delta:	bool,			// may a coding unit carry its own quantisation delta?
	pub qp_delta_depth:	u8,				// how far down the quadtree a delta may be sent
	pub cb_qp_offset:	i32,			// the chroma quantisation offsets
	pub cr_qp_offset:	i32,			// and for the other chroma channel
	pub transform_skip:	bool,			// may a block skip the transform entirely?
	pub sign_hiding:	bool,			// the last coefficient's sign inferred rather than coded
	pub transquant_bypass:	bool,		// an intra residual coded across the transform tree
	pub tiles:	bool,					// is the picture cut into tiles?
	pub wavefront:	bool,				// entropy coding synchronised at each row of blocks
	pub deblocking:	bool,				// does the deblocking filter run?
	pub slice_chroma_qp:	bool,		// may a slice header carry a further chroma offset?
	// The slice header cannot be read without the count of reserved flags: they are bits to be
	// stepped over, and stepping over the wrong number puts every field after them one place
	// out.
	pub extra_header_bits:	u8,			// reserved flags a slice header carries first
	pub output_flag:	bool,			// does a slice header carry a picture output flag?
	pub deblocking_override:	bool,	// may a slice header override the settings?
	pub filter_across_slices:	bool,	// and therefore whether a slice carries its own flag
	// The slice header cannot be read without this either: a segment that is not the first of
	// its picture carries a flag saying whether it continues the header before it, and only
	// where this says one may.
	pub dependent_slices:	bool,		// may a segment continue the header before it?
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
	pub length_size:	usize,	// bytes prefixing each NAL unit in the picture's own data
	pub sets:	Vec<Unit>,		// every parameter set, in the order the record carries them
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
	buf:	&'a [u8],
	pos:	usize,		// the next bit, counted from the first bit of the first byte
}

impl<'a> Bits<'a> {

	pub fn new(buf: &'a [u8]) -> Self {
		Self { buf, pos: 0 }
	}

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

	pub fn flag(&mut self) -> Outcome<bool> {
		Ok(res!(self.u(1)) == 1)
	}

	pub fn consumed(&self) -> Outcome<usize> {
		Ok(self.pos)
	}

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
	pub list:	[[[u8; 64]; 6]; 4],		// ScalingList[sizeId][matrixId][i], in diagonal scan order
	pub dc:		[[u8; 6]; 2],			// the corner at the two largest sizes, coded on its own
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
/// A still picture references nothing, so no *picture* is kept -- but how many the set names is,
/// because the next set may be coded as a difference from this one and a slice header may be coded
/// as a difference from any of them, and neither can be stepped over without the count.
fn short_term_ref_pic_set(b: &mut Bits, idx: usize, count: usize, previous: &mut Vec<(u32, u32)>)
	-> Outcome<()>
{
	let mut predicted = false;
	if idx != 0 {
		predicted = res!(b.flag());
	}
	if predicted {
		// Which earlier set this one is a difference from. Only a set written in a slice header
		// says so; a set in the sequence parameter set is always a difference from the one before
		// it (§7.4.8).
		let mut back = 1usize;
		if idx == count {
			back = res!(b.ue()) as usize + 1;
		}
		let _delta_rps_sign = res!(b.flag());
		let _abs_delta_rps = res!(b.ue());
		let (negative, positive) = match idx.checked_sub(back).and_then(|at| previous.get(at)) {
			Some(pair) => *pair,
			None => return Err(err!(
				"A reference picture set is coded as a difference from set {} of {}, which is not \
				there.", idx as i64 - back as i64, previous.len();
			Invalid, Input, Decode)),
		};
		// One flag pair for each picture of the set referred to, and one for the picture that set
		// is itself relative to. The ones kept are what this set names, which is what the next
		// difference will be measured against.
		let mut kept = 0u32;
		for _ in 0..(negative + positive + 1) {
			let used = res!(b.flag());
			let mut keep = used;
			if !used {
				keep = res!(b.flag());
			}
			if keep {
				kept += 1;
			}
		}
		previous.push((kept, 0));
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
	let poc_bits = res!(b.ue()) as u8 + 4;
	if poc_bits > 16 {
		return Err(err!(
			"A picture order count of {} bits, and 16 is the most.", poc_bits;
		Invalid, Input, Decode));
	}
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
	let long_term_present = res!(b.flag());
	if long_term_present {
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
	let temporal_mvp = res!(b.flag());
	let strong_smoothing = res!(b.flag());
	// The video usability information, which is where a stream says how it is to be *shown*: which
	// weights its colour was coded against, whether its samples run the full range, and -- the one
	// that changes the picture's size -- the default display window.
	//
	// **A phone's stabilised film carries one.** Stabilisation works by coding a picture larger
	// than it shows and moving the window about inside it, and the window is written here. A
	// decoder that ignores it hands back the wobbly margin as though it were part of the film,
	// about nine per cent wider and taller than every player shows.
	let (mut full_range, mut matrix) = (false, 2u8);
	let (mut show_x, mut show_y) = (0u32, 0u32);
	let (mut show_x0, mut show_y0) = (0u32, 0u32);
	if res!(b.flag()) {
		if res!(b.flag()) {
			// The sample aspect ratio, read past: a picture is drawn at the size it is coded and
			// stretching it is the caller's business.
			let idc = res!(b.u(8));
			if idc == 255 {
				let _sar_w = res!(b.u(16));
				let _sar_h = res!(b.u(16));
			}
		}
		if res!(b.flag()) {
			let _overscan_appropriate = res!(b.flag());
		}
		if res!(b.flag()) {
			let _video_format = res!(b.u(3));
			full_range = res!(b.flag());
			if res!(b.flag()) {
				let _primaries = res!(b.u(8));
				let _transfer = res!(b.u(8));
				matrix = res!(b.u(8)) as u8;
			}
		}
		if res!(b.flag()) {
			let _chroma_loc_top = res!(b.ue());
			let _chroma_loc_bottom = res!(b.ue());
		}
		let _neutral_chroma = res!(b.flag());
		let _field_seq = res!(b.flag());
		let _frame_field_info = res!(b.flag());
		if res!(b.flag()) {
			let dw_left = res!(b.ue());
			let dw_right = res!(b.ue());
			let dw_top = res!(b.ue());
			let dw_bottom = res!(b.ue());
			show_x = dw_left.saturating_add(dw_right).saturating_mul(sub_w);
			show_y = dw_top.saturating_add(dw_bottom).saturating_mul(sub_h);
			show_x0 = dw_left.saturating_mul(sub_w);
			show_y0 = dw_top.saturating_mul(sub_h);
		}
		// Nothing after the window is read: the timing information, the bitstream restrictions and
		// the hypothetical reference decoder say nothing about the samples.
	}
	let width = coded_w - trim_x;
	let height = coded_h - trim_y;
	if show_x >= width || show_y >= height {
		return Err(err!(
			"A default display window trims {} by {} from a picture of {} by {}.",
			show_x, show_y, width, height;
		Invalid, Input, Range));
	}
	Ok(Sps {
		id: id as u8,
		chroma: chroma as u8,
		coded_w,
		coded_h,
		width: width - show_x,
		height: height - show_y,
		show_x0: left.saturating_mul(sub_w) + show_x0,
		show_y0: top.saturating_mul(sub_h) + show_y0,
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
		poc_bits,
		full_range,
		matrix,
		st_sets: previous,
		long_term: long_term_present,
		temporal_mvp,
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
	let dependent_slices = res!(b.flag());
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
		dependent_slices,
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
	pub first:	bool,			// is this segment the first of its picture?
	// Nought for the first segment of a picture, which is every segment of a picture that is
	// one slice -- which every photograph is and many films are not.
	pub address:	u32,		// the coding tree block it begins at, raster order from nought
	pub across_slices:	bool,	// the picture parameter set's answer where absent (§7.4.7.1)
	pub pps_id:	u8,				// which picture parameter set the slice references
	pub kind:	u8,				// 2 is intra, and this decoder reads no other
	pub qp:		i32,			// the quantisation parameter this slice starts at
	pub sao_luma:	bool,		// does the sample adaptive offset run on luma here?
	pub sao_chroma:	bool,		// and on chroma?
	pub data_at:	usize,		// the header's end rounded up to a byte, where §9.3.1 starts
	pub deblocking:	bool,		// does the deblocking filter run on this slice?
	pub cb_qp_offset:	i32,	// what this slice adds to the picture's chroma offsets
	pub cr_qp_offset:	i32,	// and for the other chroma component
	// One a row of coding tree blocks, under wavefront coding, which is what every photograph
	// measured uses.
	pub entries:	Vec<u64>,	// where each piece begins, as the length of the one before
}

/// Which picture parameter set a slice names, read without the set itself.
///
/// The identifier is the third element of the header and none of the three before it depends on a
/// parameter set, so it can be had before choosing one -- which is the point: a caller holding
/// several sets has to know which it is being asked for. It sits before the segment address, so
/// this reads the same three elements whether or not the segment is the first of its picture.
pub fn slice_pps_id(body: &[u8]) -> Outcome<u8> {
	let mut b = Bits::new(body);
	let _first = res!(b.flag());
	// Only an IRAP picture carries this flag, and every still is one.
	let _no_output_of_prior_pics = res!(b.flag());
	Ok(res!(b.ue()) as u8)
}

/// Reads a slice segment header (§7.3.6.1).
///
/// Only the independent, intra case: a **dependent** slice segment carries no header of its own but
/// continues the one before it, and is refused by name.
///
/// A segment that is not the first of its picture carries the coding tree block it begins at, in as
/// many bits as it takes to count the picture's blocks -- which is why the sequence parameter set is
/// needed to read a header at all.
pub fn slice(body: &[u8], sps: &Sps, pps: &Pps) -> Outcome<Slice> {
	slice_of(nal::IDR_W_RADL, body, sps, pps)
}

/// The same, for a slice of a picture that may not be an IDR.
///
/// **A film's first frame very often is not one.** A clean random access picture opens a stream
/// just as an IDR does and is decoded exactly as one -- it references nothing before itself -- but
/// its slice header carries the picture order count and the reference picture set that an IDR's
/// does not, because the pictures *after* it may reference what it names. A header read as though
/// it were an IDR's is read out of step from that field onwards, and what comes out is a plausible
/// number of entry points and a picture of noise.
///
/// `kind` is the NAL unit type, which is the only thing that says which of the two this is.
pub fn slice_of(kind: u8, body: &[u8], sps: &Sps, pps: &Pps) -> Outcome<Slice> {
	let mut b = Bits::new(body);
	let first = res!(b.flag());
	// Only an IRAP picture carries this flag, and every still is one.
	let _no_output_of_prior_pics = res!(b.flag());
	let pps_id = res!(b.ue());
	let mut address = 0u32;
	if !first {
		if pps.dependent_slices && res!(b.flag()) {
			return Err(err!(
				"A dependent slice segment, which carries no header of its own but continues the \
				one before it."; Unimplemented));
		}
		// As many bits as it takes to count the picture's coding tree blocks (§7.4.7.1).
		let ctb = sps.ctb_size.max(1);
		let blocks = ((sps.coded_w + ctb - 1) / ctb) as u64 * ((sps.coded_h + ctb - 1) / ctb) as u64;
		let mut width = 0usize;
		while (1u64 << width) < blocks {
			width += 1;
		}
		address = res!(b.u(width)) as u32;
		if address as u64 >= blocks {
			return Err(err!(
				"A slice segment begins at block {} of a picture holding {}.", address, blocks;
			Invalid, Input, Decode));
		}
	}
	if pps_id as u8 != pps.id {
		return Err(err!(
			"A slice references picture parameter set {} and the one in hand is {}.",
			pps_id, pps.id;
		Invalid, Input, Missing));
	}
	// Reserved, and to be stepped over rather than understood. Stepping over the wrong number of
	// them puts every field after them one place out, which is why the count is carried here from
	// the picture parameter set rather than assumed to be zero. They come **before** the slice
	// type (§7.3.6.1).
	res!(b.skip(pps.extra_header_bits as usize));
	let slice_kind = res!(b.ue());
	if slice_kind != 2 {
		return Err(err!(
			"A slice of type {}, and a still picture's slices are all intra (type 2).", slice_kind;
		Invalid, Input, Unknown));
	}
	if pps.output_flag {
		let _pic_output_flag = res!(b.flag());
	}
	// What an IDR does not carry, and everything else does: where this picture sits in output
	// order, and which pictures the ones after it may reference.
	if kind != nal::IDR_W_RADL && kind != nal::IDR_N_LP {
		let _poc_lsb = res!(b.u(sps.poc_bits as usize));
		let from_sps = res!(b.flag());
		if !from_sps {
			// A set of its own, written here and coded as a difference from one of the sequence's.
			let mut sets = sps.st_sets.clone();
			let count = sets.len();
			res!(short_term_ref_pic_set(&mut b, count, count, &mut sets));
		} else if sps.st_sets.len() > 1 {
			// As many bits as it takes to count them (§7.4.7.1).
			let mut width = 0usize;
			while (1usize << width) < sps.st_sets.len() {
				width += 1;
			}
			let _which = res!(b.u(width));
		}
		if sps.long_term {
			// A sequence carrying long-term reference pictures is refused where it is read, so
			// reaching this means the flag is set and the sequence names none of them.
			let _num_long_term_pics = res!(b.ue());
			return Err(err!(
				"A slice names long-term reference pictures, which a picture decoded on its own \
				has no use for and this reader does not follow."; Unimplemented));
		}
		if sps.temporal_mvp {
			let _temporal_mvp = res!(b.flag());
		}
	}
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
	// Whether the loop filters run across this slice's boundaries. Where the header does not carry
	// it, the picture parameter set's answer stands (§7.4.7.1).
	let mut across_slices = pps.filter_across_slices;
	if pps.filter_across_slices && (sao_luma || sao_chroma || deblocking) {
		across_slices = res!(b.flag());
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
		//
		// A segment covering part of a picture names fewer pieces than the picture has rows, and
		// how many fewer is not knowable from the header alone -- so what is checked here is that
		// it names no more, and the exact form is checked by [`whole_picture_rows`] once the number
		// of segments is known.
		if pps.wavefront && !pps.tiles {
			let rows = ((sps.coded_h + sps.ctb_size - 1) / sps.ctb_size) as usize;
			let over = entries.len() + 1 > rows;
			if over {
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
		first,
		address,
		across_slices,
		pps_id: pps_id as u8,
		kind: slice_kind as u8,
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
	let (pic, _sps) = res!(coded(record, data));
	Ok(pic)
}

/// The same picture, cropped to the size it is meant to be **shown** at.
///
/// A coded picture is a whole number of coding tree blocks and a shown one is not: a 1920 by 1080
/// film is coded 1920 by 1088, and the sequence parameter set's conformance window says which of
/// those rows are the picture. [`picture`] hands back what was coded, because the HEIC path crops to
/// the size the container declares instead and cropping twice would take the same rows off again.
/// A caller with no container to ask -- a film's first frame -- wants this one.
pub fn picture_shown(record: &[u8], data: &[u8]) -> Outcome<decode::Picture> {
	let (pic, sps) = res!(coded(record, data));
	let (w, h) = (sps.width as usize, sps.height as usize);
	let (x0, y0) = (sps.show_x0 as usize, sps.show_y0 as usize);
	if x0 == 0 && y0 == 0 && w >= pic.y.w && h >= pic.y.h {
		return Ok(pic);
	}
	Ok(pic.window(x0, y0, w.min(pic.y.w), h.min(pic.y.h)))
}

/// Decodes one coded picture, and answers the sequence parameter set it was coded against.
///
/// The set is handed back because what a caller does with the picture next depends on it: the
/// conformance window is in it, and so is everything a caller would otherwise have to parse the
/// parameter sets again to learn.
fn coded(record: &[u8], data: &[u8]) -> Outcome<(decode::Picture, Sps)> {
	let cfg = res!(config(record));
	// Every set the record carries, not the last of each. A photograph out of a
	// camera carries one apiece and either would do; a film carries several, and
	// a slice names which one it was coded against. Keeping the last read meant
	// four films in ten were refused for referring to a set that was in hand all
	// along.
	let mut seqs: Vec<Sps> = Vec::new();
	let mut pics: Vec<Pps> = Vec::new();
	for unit in &cfg.sets {
		match unit.kind {
			nal::SPS => seqs.push(res!(sps(&unit.body))),
			nal::PPS => pics.push(res!(pps(&unit.body))),
			_ => {},
		}
	}
	if seqs.is_empty() {
		return Err(err!(
			"The decoder configuration carries no sequence parameter set."; Invalid, Input));
	}
	if pics.is_empty() {
		return Err(err!(
			"The decoder configuration carries no picture parameter set."; Invalid, Input));
	}

	// And the slices are in the item's own bytes. **Every** slice of the picture, not the first:
	// a photograph is one slice and a film's frame need not be, and a picture read from one of
	// four segments is a quarter of a picture.
	let units = res!(split_lengthed(data, cfg.length_size));
	let mut heads: Vec<(Slice, usize)> = Vec::new();
	let mut chosen: Option<(Sps, Pps)> = None;
	for (i, unit) in units.iter().enumerate() {
		match unit.kind {
			nal::IDR_W_RADL | nal::IDR_N_LP | 21 => {
				// Which sets this slice was coded against: the picture set it
				// names, and the sequence set that one belongs to.
				let want = res!(slice_pps_id(&unit.body));
				let pps = match pics.iter().find(|p| p.id == want) {
					Some(p) => p.clone(),
					None => return Err(err!(
						"A slice references picture parameter set {}, and the configuration \
						carries {}.", want,
						pics.iter().map(|p| p.id.to_string()).collect::<Vec<_>>().join(", ");
					Invalid, Input, Missing)),
				};
				let sps = match seqs.iter().find(|s| s.id == pps.sps_id) {
					Some(s) => s.clone(),
					None => return Err(err!(
						"Picture parameter set {} belongs to sequence parameter set {}, and the \
						configuration carries {}.", pps.id, pps.sps_id,
						seqs.iter().map(|s| s.id.to_string()).collect::<Vec<_>>().join(", ");
					Invalid, Input, Missing)),
				};
				let head = res!(slice_of(unit.kind, &unit.body, &sps, &pps));
				// A second coded picture in the same access unit is somebody else's frame: this
				// reads the first picture, and the first picture ends where the next one begins.
				if head.first && !heads.is_empty() {
					break;
				}
				match &chosen {
					Some((have_sps, have_pps)) => {
						if have_sps.id != sps.id || have_pps.id != pps.id {
							return Err(err!(
								"Two slices of one picture reference different parameter sets.";
							Invalid, Input, Mismatch));
						}
					},
					None => chosen = Some((sps, pps)),
				}
				heads.push((head, i));
			},
			_ => {},
		}
	}
	let (sps, pps) = match chosen {
		Some(pair) => pair,
		None => return Err(err!("Those bytes hold no coded slice."; Invalid, Input, Decode)),
	};
	// A picture that is one slice must name one piece a row of blocks, since that is what
	// wavefront coding is. It is the cheapest check there is on the whole header -- the count and
	// the geometry come out of different NAL units written at different times -- and it is what
	// caught the reading that refused every photograph in the corpus.
	if heads.len() == 1 && pps.wavefront && !pps.tiles {
		let rows = ((sps.coded_h + sps.ctb_size - 1) / sps.ctb_size) as usize;
		let named = heads[0].0.entries.len() + 1;
		if named != rows {
			return Err(err!(
				"A slice names {} pieces and the picture is {} rows of coding tree blocks deep. \
				The header has been read out of step.", named, rows;
			Invalid, Input, Decode));
		}
	}
	// The header was read from the unescaped payload; the data after it has to be handed over
	// escaped, because that is what the entry point offsets count.
	let parts: Vec<(&Slice, &[u8])> = heads.iter()
		.map(|(head, i)| {
			let raw = &units[*i].raw;
			(head, &raw[escaped_at(raw, head.data_at).min(raw.len())..])
		})
		.collect();
	let pic = res!(decode::picture_of(&sps, &pps, &parts));
	Ok((pic, sps))
}
