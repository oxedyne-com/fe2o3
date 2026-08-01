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
	/// Whether the scaling lists are in use at all, and whether this set carries its own.
	pub scaling_lists:	bool,
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

/// Steps over a scaling list (§7.3.4).
fn scaling_list(b: &mut Bits) -> Outcome<()> {
	for size in 0..4 {
		let mut id = 0;
		while id < 6 {
			let predicted = !res!(b.flag());
			if predicted {
				let _delta = res!(b.ue());
			} else {
				let coefficients = 64.min(1 << (4 + (size << 1)));
				if size > 1 {
					let _dc = res!(b.se());
				}
				for _ in 0..coefficients {
					let _delta = res!(b.se());
				}
			}
			// The 32 by 32 lists come in twos rather than sixes.
			id += if size == 3 { 3 } else { 1 };
		}
	}
	Ok(())
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
	if scaling_lists && res!(b.flag()) {
		res!(scaling_list(&mut b));
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
	if pps.slice_chroma_qp {
		let _cb = res!(b.se());
		let _cr = res!(b.se());
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
		sao_luma,
		sao_chroma,
		data_at,
		deblocking,
		entries,
	})
}

/// The probability state a context variable is in: an index 0 to 62, and the more probable symbol.
///
/// One byte rather than two fields, because there are hundreds of these and they are copied whole
/// at the start of every row of blocks under wavefront coding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ctx(u8);

impl Ctx {

	/// The state a context starts in, from its initialisation value and the slice's quantisation
	/// parameter (§9.3.2.2).
	///
	/// The arithmetic is the specification's, and the clamp on the quantisation parameter is
	/// load-bearing rather than defensive: a slice may legally start at a negative one on a
	/// high-bit-depth picture, and the table this indexes has no entries there.
	pub fn start(init: u8, qp: i32) -> Self {
		let q = qp.clamp(0, 51);
		let slope = ((init >> 4) as i32) * 5 - 45;
		let offset = (((init & 15) as i32) << 3) - 16;
		let pre = ((slope * q) >> 4) + offset;
		let pre = pre.clamp(1, 126);
		if pre <= 63 {
			Self((((63 - pre) as u8) << 1) & 0x7e)
		} else {
			Self(((((pre - 64) as u8) << 1) | 1) & 0x7f)
		}
	}

	/// The probability state index, 0 to 62.
	fn state(self) -> usize {
		(self.0 >> 1) as usize
	}

	/// The more probable symbol, 0 or 1.
	fn mps(self) -> u32 {
		(self.0 & 1) as u32
	}
}

/// How the range is narrowed for the less probable symbol, indexed by state and by the two bits
/// the current range contributes (§9.3.4.3.2.1, Table 9-46).
const LPS: [[u8; 4]; 64] = [
	[128, 176, 208, 240], [128, 167, 197, 227], [128, 158, 187, 216], [123, 150, 178, 205],
	[116, 142, 169, 195], [111, 135, 160, 185], [105, 128, 152, 175], [100, 122, 144, 166],
	[95, 116, 137, 158],  [90, 110, 130, 150],  [85, 104, 123, 142],  [81, 99, 117, 135],
	[77, 94, 111, 128],   [73, 89, 105, 122],   [69, 85, 100, 116],   [66, 80, 95, 110],
	[62, 76, 90, 104],    [59, 72, 86, 99],     [56, 69, 81, 94],     [53, 65, 77, 89],
	[51, 62, 73, 85],     [48, 59, 69, 80],     [46, 56, 66, 76],     [43, 53, 63, 72],
	[41, 50, 59, 69],     [39, 48, 56, 65],     [37, 45, 54, 62],     [35, 43, 51, 59],
	[33, 41, 48, 56],     [32, 39, 46, 53],     [30, 37, 43, 50],     [29, 35, 41, 48],
	[27, 33, 39, 45],     [26, 31, 37, 43],     [24, 30, 35, 41],     [23, 28, 33, 39],
	[22, 27, 32, 37],     [21, 26, 30, 35],     [20, 24, 29, 33],     [19, 23, 27, 31],
	[18, 22, 26, 30],     [17, 21, 25, 28],     [16, 20, 23, 27],     [15, 19, 22, 25],
	[14, 18, 21, 24],     [14, 17, 20, 23],     [13, 16, 19, 22],     [12, 15, 18, 21],
	[12, 14, 17, 20],     [11, 14, 16, 19],     [11, 13, 15, 18],     [10, 12, 15, 17],
	[10, 12, 14, 16],     [9, 11, 13, 15],      [9, 11, 12, 14],      [8, 10, 12, 14],
	[8, 9, 11, 13],       [7, 9, 11, 12],       [7, 9, 10, 12],       [7, 8, 10, 11],
	[6, 8, 9, 11],        [6, 7, 9, 10],        [6, 7, 8, 9],         [2, 2, 2, 2],
];

/// The state to move to after coding the more probable symbol (Table 9-47).
const NEXT_MPS: [u8; 64] = [
	1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
	17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
	33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48,
	49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 62, 63,
];

/// The state to move to after coding the less probable symbol (Table 9-47).
const NEXT_LPS: [u8; 64] = [
	0, 0, 1, 2, 2, 4, 4, 5, 6, 7, 8, 9, 9, 11, 11, 12,
	13, 13, 15, 15, 16, 16, 18, 18, 19, 19, 21, 21, 22, 22, 23, 24,
	24, 25, 26, 26, 27, 27, 28, 29, 29, 30, 30, 30, 31, 32, 32, 33,
	33, 33, 34, 34, 35, 35, 35, 36, 36, 36, 37, 37, 37, 38, 38, 63,
];

/// The arithmetic decoder itself (§9.3.4.3).
///
/// It reads bits and answers questions of the form "was the next symbol a one?", where the odds are
/// carried by whichever context variable the syntax says applies. There are three ways to ask: with
/// a context, which is the usual one and adapts as it goes; **bypassed**, at even odds, for the
/// parts of a value that carry no useful correlation; and **terminating**, which is how the end of
/// a slice or a piece of one is found.
pub struct Cabac<'a> {
	/// The bytes being read.
	buf:	&'a [u8],
	/// The next byte to be taken into the window.
	at:	usize,
	/// The current interval's width.
	range:	u32,
	/// Where in the interval the coded value sits.
	offset:	u32,
	/// How many bits of the window have been used.
	bits:	i32,
}

impl<'a> Cabac<'a> {

	/// Starts the decoder at the beginning of a piece of entropy-coded data (§9.3.2.5).
	pub fn new(buf: &'a [u8]) -> Outcome<Self> {
		if buf.len() < 2 {
			return Err(err!(
				"An arithmetic decoder was started on {} bytes, and it reads two before it \
				answers anything.", buf.len();
			Invalid, Input, Decode));
		}
		Ok(Self {
			buf,
			at:	2,
			range:	510,
			offset:	((buf[0] as u32) << 1) | ((buf[1] as u32) >> 7),
			bits:	7,
		})
	}

	/// The next byte, or zeroes past the end.
	///
	/// A decoder is allowed to read a little past the last byte of a slice -- the final bins are
	/// coded against bits the encoder never had to write -- so running out is not a fault. What
	/// would be a fault is reading far past it, and that is caught by the terminating bin, which
	/// says where the data ends.
	fn byte(&mut self) -> u32 {
		let b = self.buf.get(self.at).copied().unwrap_or(0) as u32;
		self.at += 1;
		b
	}

	/// One bin against a context, which is then moved on (§9.3.4.3.2).
	pub fn bin(&mut self, ctx: &mut Ctx) -> u32 {
		let state = ctx.state();
		let lps = LPS[state][((self.range >> 6) & 3) as usize] as u32;
		self.range -= lps;
		let value;
		if self.offset >= (self.range << self.bits) {
			// The less probable symbol.
			self.offset -= self.range << self.bits;
			value = 1 - ctx.mps();
			self.range = lps;
			if state == 0 {
				// State zero is where the two symbols are equally likely, so being wrong there
				// exchanges which one is called the more probable.
				ctx.0 ^= 1;
			}
			ctx.0 = (NEXT_LPS[state] << 1) | (ctx.0 & 1);
		} else {
			value = ctx.mps();
			ctx.0 = (NEXT_MPS[state] << 1) | (ctx.0 & 1);
		}
		// Renormalise: the interval is kept at nine bits or more.
		while self.range < 256 {
			self.range <<= 1;
			self.bits -= 1;
			if self.bits < 0 {
				self.offset = (self.offset << 8) | self.byte();
				self.bits += 8;
			}
		}
		value
	}

	/// One bin at even odds, with no context to move on (§9.3.4.3.4).
	pub fn bypass(&mut self) -> u32 {
		self.bits -= 1;
		if self.bits < 0 {
			self.offset = (self.offset << 8) | self.byte();
			self.bits += 8;
		}
		let scaled = self.range << self.bits;
		if self.offset >= scaled {
			self.offset -= scaled;
			1
		} else {
			0
		}
	}

	/// `n` bins at even odds, most significant first.
	pub fn bypass_bits(&mut self, n: usize) -> u32 {
		let mut v = 0u32;
		for _ in 0..n.min(32) {
			v = (v << 1) | self.bypass();
		}
		v
	}

	/// The bin that says whether this is the end (§9.3.4.3.5).
	///
	/// One at the end of a slice, and at the end of each piece of one under wavefront coding.
	pub fn terminate(&mut self) -> u32 {
		self.range -= 2;
		if self.offset >= (self.range << self.bits) {
			1
		} else {
			while self.range < 256 {
				self.range <<= 1;
				self.bits -= 1;
				if self.bits < 0 {
					self.offset = (self.offset << 8) | self.byte();
					self.bits += 8;
				}
			}
			0
		}
	}

	/// How many bytes have been taken out of the buffer.
	///
	/// After a terminating bin says the piece has ended, this is where the next piece begins --
	/// which is how the entry point offsets in the slice header are checked against the data.
	pub fn consumed(&self) -> usize {
		self.at
	}
}

// ------------------------------------------------------- the context variables themselves

/// A set of context variables belonging to one syntax element (§9.3.2.2, Table 9-4).
///
/// **Only the sets an intra still picture uses, and only their intra initialisation values.** The
/// specification gives three initialisation types -- one for I slices and two for P and B -- and a
/// picture out of a HEIC file is one intra slice, so the other two are as much use here as the
/// motion vector syntax they mostly belong to. Where a table's intra column is a subset of its rows
/// (`sig_coeff_flag` uses 0 to 41 and then 126 and 127, and nothing between), that is what is kept,
/// and the gap is recorded in [`Set::runs`] so the whole lot can be checked back against the
/// published table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Set {
	/// Whether this block reuses the block to its left's or above's sample adaptive offset.
	SaoMerge,
	/// Which kind of sample adaptive offset a block has.
	SaoType,
	/// Whether a block of the coding quadtree splits into four.
	SplitCu,
	/// Whether a coding unit skips the transform and the quantiser entirely.
	TransquantBypass,
	/// How a coding unit is divided into prediction units.
	PartMode,
	/// Whether a block's intra mode is one of the three its neighbours suggest.
	PrevIntraLumaPred,
	/// Which intra mode the chroma blocks take.
	IntraChromaPredMode,
	/// Whether a block of the transform tree splits into four.
	SplitTransform,
	/// Whether a luma transform block has any coefficient in it at all.
	CbfLuma,
	/// The same for the two chroma blocks.
	CbfChroma,
	/// How far this block's quantisation parameter is from the one predicted for it.
	CuQpDeltaAbs,
	/// Whether a transform block is coded without its transform.
	TransformSkip,
	/// Where the last coefficient of a block sits, across.
	LastSigX,
	/// And down.
	LastSigY,
	/// Whether a four-by-four group within a transform block holds anything.
	CodedSubBlock,
	/// Whether one coefficient is not zero.
	SigCoeff,
	/// Whether a coefficient's magnitude is more than one.
	Greater1,
	/// Whether it is more than two.
	Greater2,
}

impl Set {

	/// Every set, in the order their context variables are laid out.
	pub const ALL: [Self; 18] = [
		Self::SaoMerge,
		Self::SaoType,
		Self::SplitCu,
		Self::TransquantBypass,
		Self::PartMode,
		Self::PrevIntraLumaPred,
		Self::IntraChromaPredMode,
		Self::SplitTransform,
		Self::CbfLuma,
		Self::CbfChroma,
		Self::CuQpDeltaAbs,
		Self::TransformSkip,
		Self::LastSigX,
		Self::LastSigY,
		Self::CodedSubBlock,
		Self::SigCoeff,
		Self::Greater1,
		Self::Greater2,
	];

	/// The initialisation values of this set's context variables, for an intra slice.
	pub const fn init(self) -> &'static [u8] {
		match self {
			Self::SaoMerge		=> &[153],
			Self::SaoType		=> &[200],
			Self::SplitCu		=> &[139, 141, 157],
			Self::TransquantBypass	=> &[154],
			Self::PartMode		=> &[184],
			Self::PrevIntraLumaPred	=> &[184],
			Self::IntraChromaPredMode => &[63],
			Self::SplitTransform	=> &[153, 138, 138],
			Self::CbfLuma		=> &[111, 141],
			// Four by depth in the transform tree, and a fifth for the second chroma block of a
			// 4:2:2 picture, which this decoder will not meet but which sits in the same table.
			Self::CbfChroma		=> &[94, 138, 182, 154, 154],
			Self::CuQpDeltaAbs	=> &[154, 154],
			// One for luma and one for chroma; the specification numbers them 0 and 3.
			Self::TransformSkip	=> &[139, 139],
			Self::LastSigX		=> &[
				110, 110, 124, 125, 140, 153, 125, 127, 140,
				109, 111, 143, 127, 111, 79, 108, 123, 63,
			],
			Self::LastSigY		=> &[
				110, 110, 124, 125, 140, 153, 125, 127, 140,
				109, 111, 143, 127, 111, 79, 108, 123, 63,
			],
			Self::CodedSubBlock	=> &[91, 171, 134, 141],
			Self::SigCoeff		=> &[
				111, 111, 125, 110, 110, 94, 124, 108,
				124, 107, 125, 141, 179, 153, 125, 107,
				125, 141, 179, 153, 125, 107, 125, 141,
				179, 153, 125, 140, 139, 182, 182, 152,
				136, 152, 136, 153, 136, 139, 111, 136,
				139, 111,
				// The two the specification puts at 126 and 127, for a block coded without its
				// transform.
				141, 111,
			],
			Self::Greater1		=> &[
				140, 92, 137, 138, 140, 152, 138, 139,
				153, 74, 149, 92, 139, 107, 122, 152,
				140, 179, 166, 182, 140, 227, 122, 197,
			],
			Self::Greater2		=> &[138, 153, 136, 167, 152, 152],
		}
	}

	/// How many context variables the set holds.
	pub const fn len(self) -> usize {
		self.init().len()
	}

	/// Whether it holds none, which none of these do.
	pub const fn is_empty(self) -> bool {
		self.len() == 0
	}

	/// Where the set's variables begin in the flat array [`Contexts`] holds.
	pub const fn base(self) -> usize {
		let mut at = 0;
		let mut i = 0;
		while i < Self::ALL.len() {
			if Self::ALL[i] as u8 == self as u8 {
				return at;
			}
			at += Self::ALL[i].len();
			i += 1;
		}
		at
	}

	/// Which published table the values in [`Set::init`] were taken from, as its number within
	/// clause 9 -- `5` for Table 9-5.
	///
	/// Kept so that the transcription can be checked against the document rather than against
	/// itself; `tests` does exactly that where a copy of the specification is to hand.
	pub const fn table(self) -> usize {
		match self {
			Self::SaoMerge		=> 5,
			Self::SaoType		=> 6,
			Self::SplitCu		=> 7,
			Self::TransquantBypass	=> 8,
			Self::PartMode		=> 11,
			Self::PrevIntraLumaPred	=> 12,
			Self::IntraChromaPredMode => 13,
			Self::SplitTransform	=> 20,
			Self::CbfLuma		=> 21,
			Self::CbfChroma		=> 22,
			Self::CuQpDeltaAbs	=> 24,
			Self::TransformSkip	=> 25,
			Self::LastSigX		=> 26,
			Self::LastSigY		=> 27,
			Self::CodedSubBlock	=> 28,
			Self::SigCoeff		=> 29,
			Self::Greater1		=> 30,
			Self::Greater2		=> 31,
		}
	}

	/// Which of that table's entries an intra slice takes, as runs of `(first, how many)`.
	///
	/// Table 9-4 gives these as ranges against the initialisation type, and for an intra slice they
	/// are the first ones -- except where a table serves two syntax elements at once, or where the
	/// entries a still picture wants are not next to each other.
	pub const fn runs(self) -> &'static [(usize, usize)] {
		match self {
			// Four by depth at 0..3, and the odd one out at 12.
			Self::CbfChroma		=> &[(0, 4), (12, 1)],
			// Luma at 0 and chroma at 3.
			Self::TransformSkip	=> &[(0, 1), (3, 1)],
			// Nought to forty-one, and then two a long way further on.
			Self::SigCoeff		=> &[(0, 42), (126, 2)],
			// Everything else is a run from the start of its table.
			other			=> match other.len() {
				// One run, as long as the set is. Written this way because a `const fn` cannot
				// hold a reference to a temporary, so each length that occurs gets its own.
				1	=> &[(0, 1)],
				2	=> &[(0, 2)],
				3	=> &[(0, 3)],
				4	=> &[(0, 4)],
				6	=> &[(0, 6)],
				18	=> &[(0, 18)],
				24	=> &[(0, 24)],
				_	=> &[],
			},
		}
	}
}

/// How many context variables a picture's decoder carries altogether.
pub const CONTEXTS: usize = {
	let mut at = 0;
	let mut i = 0;
	while i < Set::ALL.len() {
		at += Set::ALL[i].len();
		i += 1;
	}
	at
};

/// Every context variable a still picture's decoder needs, in one array.
///
/// One flat array with a base per set, rather than a struct of named arrays, because the whole lot
/// is **copied** at the start of every row of blocks under wavefront coding -- which every
/// photograph in the corpus this was written against uses -- and a copy of one fixed-size array is
/// as cheap as copying gets.
#[derive(Clone, Copy, Debug)]
pub struct Contexts {
	v:	[Ctx; CONTEXTS],
}

impl Contexts {

	/// The state every context starts a slice in, given that slice's quantisation parameter.
	pub fn start(qp: i32) -> Self {
		let mut v = [Ctx::start(154, 26); CONTEXTS];
		for set in Set::ALL {
			let base = set.base();
			let init = set.init();
			let mut i = 0;
			while i < init.len() {
				v[base + i] = Ctx::start(init[i], qp);
				i += 1;
			}
		}
		Self { v }
	}

	/// One context variable: which set, and which of that set's variables the syntax says applies.
	///
	/// An index past the end of its set is a fault in whoever worked out the increment, not a thing
	/// to be clamped quietly into range: a decoder that reads the wrong context produces a picture
	/// rather than an error, and a picture that is subtly wrong is the hardest kind of fault to
	/// find. So it is refused, and the message says which set and which index.
	pub fn at(&mut self, set: Set, i: usize) -> Outcome<&mut Ctx> {
		if i >= set.len() {
			return Err(err!(
				"Context {} of {:?} was asked for, and that set holds {}.", i, set, set.len();
			Invalid, Input, Decode));
		}
		Ok(&mut self.v[set.base() + i])
	}
}

/// The context state carried from one row of blocks to the next, under wavefront coding.
///
/// Every photograph in the corpus is coded in wavefronts (`entropy_coding_sync_enabled_flag`),
/// which is the surprise that shapes this decoder: the arithmetic coder is **restarted at every row
/// of coding tree blocks**, and the contexts it restarts with are not fresh ones but the ones saved
/// after the *second* block of the row above. That is what lets an encoder code the rows in
/// parallel while still learning from what came before, and it means a decoder that treats a row
/// boundary as a fresh start decodes plausible rubbish from the second row onward.
///
/// This holds the one-slice, one-tile case, which is every picture in the corpus and every picture a
/// still image is likely to be. A second tile would need one of these each, since a tile boundary
/// breaks the dependency; the widening is a field, not a redesign, and is left until a picture
/// wants it.
#[derive(Clone, Copy, Debug)]
pub struct Rows {
	/// What a slice starts from, for the first row, which has nothing above it.
	qp:	i32,
	/// What was saved after the second block of the row above.
	saved:	Option<Contexts>,
}

impl Rows {

	/// A picture whose first row has nothing to inherit.
	pub fn new(qp: i32) -> Self {
		Self { qp, saved: None }
	}

	/// The contexts a row of blocks begins with.
	///
	/// The row above's second block where there was one, and a fresh set where there was not --
	/// which is the first row, and only the first row.
	pub fn begin(&self) -> Contexts {
		match self.saved {
			Some(ctxs) => ctxs,
			None => Contexts::start(self.qp),
		}
	}

	/// Keeps the state as it stands, which the caller does once the **second** block of a row has
	/// been decoded (§9.3.2.3).
	///
	/// Not the first: the row above has to be two blocks ahead before the row below may start, or
	/// the two would be coding the same neighbourhood at once.
	pub fn after_second(&mut self, ctxs: &Contexts) {
		self.saved = Some(*ctxs);
	}
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
	fn test_every_context_starts_in_a_state_that_exists_03() -> Outcome<()> {
		// Two hundred and fifty-six initialisation values against every quantisation parameter a
		// slice may carry, including the negative ones a high-bit-depth picture allows. The state
		// this yields indexes a table of sixty-four rows, and the arithmetic that produces it is
		// the specification's own -- so an index outside it is a transcription error, and the only
		// way to find one before the whole decoder exists is to try them all.
		for init in 0..=255u8 {
			for qp in -12..=51i32 {
				let ctx = Ctx::start(init, qp);
				let state = ctx.state();
				if state > 62 {
					return Err(err!(
						"An initialisation value of {} at a quantisation parameter of {} starts in \
						state {}, and 62 is the highest.", init, qp, state;
					Test, Invalid));
				}
			}
		}
		Ok(())
	}

	#[test]
	fn test_the_interval_is_renormalised_after_every_bin_04() -> Outcome<()> {
		// The invariant the whole arithmetic decoder rests on: the interval is at least 256 and at
		// most 510 whenever a bin has been answered. A renormalisation that stops one shift short
		// decodes plausible rubbish rather than failing, which is exactly the sort of fault that
		// survives until a picture comes out wrong, so it is asserted directly.
		//
		// The data is a run of bytes from a small linear congruential sequence: it is not a coded
		// picture and does not have to be, since the invariant holds over any input at all.
		let mut seed = 0x2545_f491_4f6c_dd1du64;
		let mut data = Vec::with_capacity(4096);
		for _ in 0..4096 {
			seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
			data.push((seed >> 33) as u8);
		}
		let mut cabac = res!(Cabac::new(&data));
		let mut ctxs: Vec<Ctx> = (0..16).map(|i| Ctx::start(i * 17, 26)).collect();
		let mut ones = 0usize;
		let total = 20_000usize;
		for i in 0..total {
			match i % 8 {
				7 => {
					// A terminating bin, which is asked once a block. Where it says the data has
					// ended, the decoder stops -- and on random bytes it will, eventually.
					if cabac.terminate() == 1 {
						break;
					}
				},
				6 => {
					ones += cabac.bypass() as usize;
				},
				k => {
					ones += cabac.bin(&mut ctxs[k * 2]) as usize;
				},
			}
			if cabac.range < 256 || cabac.range > 510 {
				return Err(err!(
					"After {} bins the interval is {}, outside 256 to 510.", i + 1, cabac.range;
				Test, Invalid));
			}
		}
		// A decoder that answered every bin the same way would satisfy the invariant above and be
		// useless, so the answers are required to be mixed.
		if ones == 0 {
			return Err(err!("Not one bin came back as a one."; Test, Invalid));
		}
		Ok(())
	}

	#[test]
	fn test_the_context_tables_are_the_published_ones_06() -> Outcome<()> {
		// Two hundred and thirty numbers copied out of a document by hand, every one of which
		// silently ruins a picture if it is wrong. Checking them against the decoder that uses them
		// proves nothing at all -- the only thing worth checking them against is the specification
		// they came from, so this reads it.
		//
		//   pdftotext -layout T-REC-H.265-202108.pdf h265.txt
		//   HEVC_SPEC_TEXT=~/.cache/specs/h265.txt cargo test -p oxedyne_fe2o3_graphics hevc
		//
		// Absent, it says so rather than passing quietly: a check that skipped in silence would be
		// a check nobody ran.
		let path = match std::env::var("HEVC_SPEC_TEXT") {
			Ok(p) => p,
			Err(_) => {
				println!("  skipped: set HEVC_SPEC_TEXT to a text rendering of Rec. ITU-T H.265");
				return Ok(());
			},
		};
		let text = match std::fs::read_to_string(&path) {
			Ok(t) => t,
			Err(e) => {
				println!("  skipped: {} would not read ({})", path, e);
				return Ok(());
			},
		};
		let lines: Vec<&str> = text.lines().collect();

		for set in Set::ALL {
			// The table's own numbers, in ctxIdx order: every "initValue" row under the heading,
			// until the next table begins. The document lays a wide table out as alternating rows
			// of indices and values, so the values arrive in several pieces and in order.
			let heading = fmt!("Table 9-{} – Values of initValue", set.table());
			// The heading occurs twice: once in the table of contents, trailing dot leaders and a
			// page number, and once over the table itself. Taking whichever one has values under it
			// needs no rule about which is which.
			let mut published: Vec<u8> = Vec::new();
			for (start, _) in lines.iter().enumerate().filter(|(_, l)| l.contains(&heading)) {
				let mut found: Vec<u8> = Vec::new();
				for line in lines.iter().skip(start + 1) {
					let trimmed = line.trim_start();
					if trimmed.starts_with("Table 9-") {
						break;
					}
					if !trimmed.starts_with("initValue") {
						continue;
					}
					for word in trimmed.trim_start_matches("initValue").split_whitespace() {
						match word.parse::<u16>() {
							// Every initValue is a byte. A page number caught in the same row
							// would not be, and shows up here rather than as a wrong picture.
							Ok(n) if n <= 255 => found.push(n as u8),
							_ => return Err(err!(
								"Table 9-{} holds {:?}, which is not an initialisation value.",
								set.table(), word; Test, Invalid)),
						}
					}
				}
				if !found.is_empty() {
					published = found;
					break;
				}
			}
			if published.is_empty() {
				return Err(err!(
					"Table 9-{} is not in {}, or holds no values.", set.table(), path;
				Test, Missing));
			}
			// What an intra slice takes out of it, per Table 9-4.
			let mut wanted: Vec<u8> = Vec::new();
			for (first, count) in set.runs() {
				let end = first + count;
				if end > published.len() {
					return Err(err!(
						"{:?} wants entries {}..{} of Table 9-{}, which holds {}.",
						set, first, end, set.table(), published.len(); Test, Invalid));
				}
				wanted.extend_from_slice(&published[*first..end]);
			}
			let held: Vec<u8> = set.init().to_vec();
			if held != wanted {
				return Err(err!(
					"{:?} is initialised from {:?}, and Table 9-{} entries {:?} are {:?}.",
					set, held, set.table(), set.runs(), wanted; Test, Mismatch));
			}
		}
		Ok(())
	}

	#[test]
	fn test_a_row_of_blocks_inherits_the_row_above_it_07() -> Outcome<()> {
		// The wavefront rule, which is the one every photograph in the corpus depends on: a row of
		// blocks starts from the contexts saved after the *second* block of the row above, not from
		// fresh ones. A decoder that starts each row afresh decodes rubbish from the second row on,
		// and this is the smallest statement of the difference.
		let mut rows = Rows::new(26);
		let fresh = rows.begin();
		let mut moved = fresh;
		// Something the first row learned, which the second must not lose.
		let ctx = res!(moved.at(Set::SigCoeff, 0));
		let before = *ctx;
		*ctx = Ctx::start(200, 51);
		let learned = *res!(moved.at(Set::SigCoeff, 0));
		let changed = learned != before;
		req!(changed, true, "the fixture did not change anything, so it proves nothing");

		rows.after_second(&moved);
		let next = rows.begin();
		let carried = next.v[Set::SigCoeff.base()];
		req!(carried, learned, "a row began afresh instead of from the row above it");

		// And a picture whose first row has nothing above it starts from the table.
		let first = Rows::new(26).begin();
		req!(first.v[Set::SigCoeff.base()], before);
		Ok(())
	}

	#[test]
	fn test_the_context_sets_do_not_overlap_or_leave_gaps_08() -> Outcome<()> {
		// The bases are worked out by summing the lengths in front of each set, so a set added in
		// the middle of the list moves every one after it. That is the intended behaviour and it is
		// also exactly how one set would come to share variables with another, so it is asserted.
		let mut at = 0usize;
		for set in Set::ALL {
			req!(set.base(), at, "{:?} does not begin where the set before it ends", set);
			at += set.len();
			let empty = set.is_empty();
			req!(empty, false, "{:?} holds no context variables at all", set);
		}
		req!(at, CONTEXTS);
		// And the last one is reachable, while one past it is refused rather than read.
		let mut ctxs = Contexts::start(26);
		let last = Set::Greater2.len() - 1;
		req!(ctxs.at(Set::Greater2, last).is_ok(), true);
		req!(ctxs.at(Set::Greater2, last + 1).is_err(), true,
			"a context past the end of its set was handed out");
		Ok(())
	}

	#[test]
	fn test_a_decoder_needs_two_bytes_to_begin_05() -> Outcome<()> {
		req!(Cabac::new(&[0x40]).is_err(), true,
			"A decoder started on one byte, and it reads two before answering anything.");
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
