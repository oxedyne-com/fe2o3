//! An H.264 decoder, for the first frame of a film.
//!
//! A photograph library that holds films has to draw something for each of them, and the something
//! is the first frame. Getting it means decoding H.264 (ITU-T H.264 | ISO/IEC 14496-10), because
//! there is no thumbnail in the file to read instead. This module is that decoder. It is built for
//! **intra** coding only: the first coded picture of every film is an IDR, an IDR refers to nothing
//! but itself, and so everything about motion, reference pictures and prediction between frames is
//! absent by construction rather than unimplemented.
//!
//! # What the films in one real library actually are
//!
//! Every film in a family library was measured before any of this was designed: 7,242 files, 6,036
//! named `.mp4` and 1,206 named `.mov`. The `avcC` or `hvcC` record of each was read out of its
//! sample description and its parameter sets parsed. They are not one thing:
//!
//! | Codec | Films |
//! |---|---|
//! | HEVC (H.265) | 5,385 |
//! | **H.264** | **1,658** |
//! | Motion JPEG | 196 |
//! | MPEG-4 Part 2 | 2 |
//!
//! Of the 1,658 that are H.264, the split that shapes this module is the **entropy coder**, and it
//! is the measurement that mattered most:
//!
//! | Profile | Films | Entropy coder | 8x8 transform |
//! |---|---|---|---|
//! | Baseline (66) | 711 | CAVLC | no |
//! | Main (77) | 86 | CABAC | no |
//! | High (100) | 861 | CABAC | yes |
//!
//! **947 films are CABAC and 711 are CAVLC.** H.264 has two entropy coders where HEVC has one, and
//! neither of these is the rare case: a decoder that implements only the arithmetic coder, on the
//! reasoning that HEVC has no other, would refuse two films in every five. Both are needed, and
//! they share nothing but the syntax elements they code -- CAVLC is a set of published
//! variable-length code tables read with a bit reader, CABAC is an adaptive arithmetic decoder.
//! That is the equivalent here of the wavefront discovery in [`crate::hevc`], and it doubles the
//! entropy layer rather than widening it.
//!
//! Everything else about the corpus is uniform, and each of these is asserted where it is read
//! rather than assumed:
//!
//! - **Eight bits**, luma and chroma alike, in all 1,658.
//! - **4:2:0** (`chroma_format_idc` of 1) in all 1,658.
//! - **Frames, never fields**: `frame_mbs_only_flag` is 1 in all 1,658, so there is no field
//!   coding and no macroblock-adaptive frame/field coding anywhere in the library.
//! - **One slice group** in all 1,658, so no slice-group map and no macroblock-to-slice-group
//!   indirection.
//! - `constrained_intra_pred_flag` **off** in all 1,658, which for an all-intra picture changes
//!   nothing but is checked because it would if a P slice ever arrived.
//! - **Four-byte NAL length prefixes** (`lengthSizeMinusOne` of 3) in all 1,658.
//! - **Scaling lists**: 1,625 films carry none at all, 24 carry sequence-level lists and 9 carry
//!   picture-level lists. Absent is the common case but not the only one, and the fall-back rules
//!   of Table 7-2 mean "not present" does not mean "flat" -- it means "inherit", and at the head of
//!   each fall-back chain it means the *default* matrices of Tables 7-3 and 7-4, which are not
//!   flat. A decoder that reads absence as no scaling quantises every block of those 33 films
//!   wrongly and produces pictures that are recognisable and wrong.
//!
//! And the first coded picture of every one of the 1,658 was extracted and its NAL units read:
//! **every one is an IDR carrying I slices only**. Most are one slice -- 1,566 of them -- but 83
//! carry two and 9 carry eight, so slices are not a formality: each restarts the entropy coder, and
//! a macroblock in another slice is unavailable for prediction however close it sits.
//!
//! Resolutions run 1920x1088 (669 films), 1280x720 (462), 640x480 (103), 1088x1920 (96), 720x480
//! (85), 848x480 (77), down a tail to 176x144 (21), with 9 at 3840x2160.
//!
//! # References
//!
//! Rec. ITU-T H.264 (08/2021). The NAL unit header is §7.3.1, the sequence parameter set §7.3.2.1.1,
//! the picture parameter set §7.3.2.2, the slice header §7.3.3, the macroblock layer §7.3.5, CAVLC
//! §9.2 and CABAC §9.3. The `avcC` record the parameter sets arrive in is ISO/IEC 14496-15 §5.3.3.1.
//! Every constant below names the clause it comes from.

pub mod cavlc;
pub mod decode;
pub mod filter;
pub mod intra;
pub mod transform;

use oxedyne_fe2o3_core::prelude::*;

/// The largest picture this decoder will describe, in luma samples each way.
///
/// Sixteen thousand is past every camera and well inside what the level limits allow; it is a
/// ceiling against a parameter set that is a mistake, not a limit on real films.
pub const MAX_SIDE: u32 = 16_384;

/// NAL unit types this decoder cares about (Table 7-1).
pub mod nal {
	/// A coded slice of a picture that is not an IDR.
	pub const SLICE: u8 = 1;
	/// A coded slice of an IDR picture, which is what the first frame of a film is.
	pub const IDR: u8 = 5;
	/// Supplemental enhancement information, which changes no sample.
	pub const SEI: u8 = 6;
	/// A sequence parameter set.
	pub const SPS: u8 = 7;
	/// A picture parameter set.
	pub const PPS: u8 = 8;
	/// An access unit delimiter.
	pub const AUD: u8 = 9;
	/// A prefix NAL unit, which precedes a slice in a scalable stream.
	pub const PREFIX: u8 = 14;
	/// A subset sequence parameter set, for a scalable or multiview layer this decoder ignores.
	pub const SUBSET_SPS: u8 = 15;
}

/// One NAL unit: what it is, and its payload with the emulation prevention undone.
#[derive(Clone, Debug)]
pub struct Unit {
	/// The type, from the low five bits of the one-byte NAL unit header (§7.3.1).
	pub kind:	u8,
	/// `nal_ref_idc`: nought where nothing refers to this unit.
	pub ref_idc:	u8,
	/// The payload, after the header and with every emulation prevention byte removed.
	pub body:	Vec<u8>,
}

/// Which kind of slice this is (§7.4.3, Table 7-6).
///
/// The values run 0 to 9, where 5 to 9 repeat 0 to 4 with the added meaning that every slice of the
/// picture is of that type. Only the intra ones are decoded here; the rest are named so that a
/// refusal can say what it met.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SliceType {
	/// Predicted from one earlier picture.
	P,
	/// Predicted from two.
	B,
	/// Intra: predicted only from this picture.
	I,
	/// A switching P slice.
	Sp,
	/// A switching I slice.
	Si,
}

impl SliceType {

	/// The type a `slice_type` codes, whichever of its two values it uses.
	pub fn of(code: u32) -> Outcome<Self> {
		Ok(match code % 5 {
			0	=> Self::P,
			1	=> Self::B,
			2	=> Self::I,
			3	=> Self::Sp,
			4	=> Self::Si,
			_	=> return Err(err!(
				"A slice_type of {} is outside the 0 to 9 the syntax allows.", code;
			Invalid, Input, Decode)),
		})
	}

	/// Whether every macroblock of the slice is coded without reference to another picture.
	pub fn is_intra(self) -> bool {
		matches!(self, Self::I | Self::Si)
	}
}

/// What a sequence parameter set says about the pictures that follow it (§7.3.2.1.1).
///
/// Only the fields a still frame's decoder acts on are kept, plus the few that must be read to know
/// where the next field begins. A parameter set is a run of variable-length codes and there is no
/// skipping to a field without decoding everything in front of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sps {
	/// Which set this is, as a picture parameter set names it.
	pub id:		u8,
	/// The profile, as `profile_idc`.
	pub profile:	u8,
	/// The level, as `level_idc`.
	pub level:	u8,
	/// The chroma sampling: 0 monochrome, 1 for 4:2:0, 2 for 4:2:2, 3 for 4:4:4.
	pub chroma:	u8,
	/// Whether the three colour planes are coded as separate monochrome pictures.
	pub separate_planes:	bool,
	/// Bits a luma sample.
	pub luma_bits:	u8,
	/// Bits a chroma sample.
	pub chroma_bits:	u8,
	/// Whether a lossless macroblock skips the transform when its quantisation parameter is nought.
	pub qpprime_bypass:	bool,
	/// The picture's width in macroblocks.
	pub mbs_w:	u32,
	/// The height of the picture in map units, which for a frame-coded stream is macroblock rows.
	pub map_units_h:	u32,
	/// Whether every picture is a frame rather than a field.
	pub frame_mbs_only:	bool,
	/// Whether a macroblock pair may be coded as two fields.
	pub mbaff:	bool,
	/// How many bits `frame_num` occupies in a slice header.
	pub frame_num_bits:	u32,
	/// Which of the three picture order count schemes is in use.
	pub poc_type:	u32,
	/// How many bits the picture order count's low half occupies, where the scheme has one.
	pub poc_lsb_bits:	u32,
	/// Whether the picture order count's deltas are all nought, under scheme one.
	pub delta_poc_always_zero:	bool,
	/// The cropping window, in the units §7.4.2.1.1 counts it in: left, right, top, bottom.
	pub crop:	[u32; 4],
	/// The scaling lists this sequence carries, where it carries any.
	pub scaling:	Option<Scaling>,
	/// The coded width in luma samples, before cropping.
	pub coded_w:	u32,
	/// The coded height, likewise.
	pub coded_h:	u32,
	/// The width of the picture as it is meant to be shown.
	pub width:	u32,
	/// The height as shown.
	pub height:	u32,
}

/// What a picture parameter set says about the slices that reference it (§7.3.2.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pps {
	/// Which set this is, as a slice header names it.
	pub id:		u8,
	/// Which sequence parameter set it belongs to.
	pub sps_id:	u8,
	/// Whether the slice data is coded with the arithmetic coder rather than the length tables.
	pub cabac:	bool,
	/// Whether a slice header carries a second picture order count delta.
	pub bottom_field_order:	bool,
	/// How many slice groups the picture is cut into.
	pub slice_groups:	u32,
	/// The starting quantisation parameter, already offset by the 26 the syntax subtracts.
	pub init_qp:	i32,
	/// The offset applied to the luma quantisation parameter to get the Cb one.
	pub cb_qp_offset:	i32,
	/// The same for Cr, which defaults to the Cb one where the set does not carry it.
	pub cr_qp_offset:	i32,
	/// Whether a slice header carries its own deblocking settings.
	pub deblocking_control:	bool,
	/// Whether an intra macroblock may predict from an inter-coded neighbour.
	pub constrained_intra:	bool,
	/// Whether a slice header carries a redundant picture count.
	pub redundant_pic_cnt:	bool,
	/// Whether a macroblock may use the eight-by-eight transform.
	pub transform_8x8:	bool,
	/// The scaling lists this picture carries, where it carries any.
	pub scaling:	Option<Scaling>,
}

/// What one slice header says (§7.3.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slice {
	/// The first macroblock of the picture this slice codes, in raster order.
	pub first_mb:	u32,
	/// Which kind of slice this is.
	pub kind:	SliceType,
	/// Whether every slice of the picture is of that kind, which the second range of `slice_type`
	/// codes.
	pub all_same:	bool,
	/// Which picture parameter set it references.
	pub pps_id:	u8,
	/// Whether this slice belongs to an IDR picture.
	pub idr:	bool,
	/// The quantisation parameter the slice starts at, already summed with the set's.
	pub qp:		i32,
	/// Which of the three deblocking dispositions applies: 0 on, 1 off, 2 off across slice edges.
	pub deblocking:	u32,
	/// The offset added to the deblocking filter's first threshold.
	pub alpha_offset:	i32,
	/// The offset added to its second.
	pub beta_offset:	i32,
	/// Which context initialisation table a non-intra slice's arithmetic coder starts from.
	pub cabac_init_idc:	u32,
	/// Where the slice's entropy-coded data begins, in bits from the start of the unescaped payload.
	pub data_bit:	usize,
}

/// A set of quantisation weights (§7.4.2.1.1.1, §8.5.9).
///
/// Six four-by-four lists and six eight-by-eight ones, each held **in the order the syntax codes
/// them**, which is the inverse scanning order rather than raster. They are inverse-scanned into a
/// weight matrix where they are used, in [`transform`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scaling {
	/// `ScalingList4x4[0..6]`, for intra and inter Y, Cb and Cr in that order.
	pub l4:	[[u8; 16]; 6],
	/// `ScalingList8x8[0..6]`, in the same order. Only the first and second are used by a 4:2:0
	/// picture, and only the first by an intra one.
	pub l8:	[[u8; 64]; 6],
}

/// The default four-by-four weights for an intra macroblock (Table 7-3).
pub const DEFAULT_4X4_INTRA: [u8; 16] = [
	6, 13, 13, 20, 20, 20, 28, 28, 28, 28, 32, 32, 32, 37, 37, 42,
];

/// The default four-by-four weights for an inter macroblock (Table 7-3).
pub const DEFAULT_4X4_INTER: [u8; 16] = [
	10, 14, 14, 20, 20, 20, 24, 24, 24, 24, 27, 27, 27, 30, 30, 34,
];

/// The default eight-by-eight weights for an intra macroblock (Table 7-4).
pub const DEFAULT_8X8_INTRA: [u8; 64] = [
	6, 10, 10, 13, 11, 13, 16, 16, 16, 16, 18, 18, 18, 18, 18, 23,
	23, 23, 23, 23, 23, 25, 25, 25, 25, 25, 25, 25, 27, 27, 27, 27,
	27, 27, 27, 27, 29, 29, 29, 29, 29, 29, 29, 31, 31, 31, 31, 31,
	31, 33, 33, 33, 33, 33, 36, 36, 36, 36, 38, 38, 38, 40, 40, 42,
];

/// The default eight-by-eight weights for an inter macroblock (Table 7-4).
pub const DEFAULT_8X8_INTER: [u8; 64] = [
	9, 13, 13, 15, 13, 15, 17, 17, 17, 17, 19, 19, 19, 19, 19, 21,
	21, 21, 21, 21, 21, 22, 22, 22, 22, 22, 22, 22, 24, 24, 24, 24,
	24, 24, 24, 24, 25, 25, 25, 25, 25, 25, 25, 27, 27, 27, 27, 27,
	27, 28, 28, 28, 28, 28, 30, 30, 30, 30, 32, 32, 32, 33, 33, 35,
];

impl Scaling {

	/// Flat weights, which is what a stream carrying no lists at all quantises against.
	pub fn flat() -> Self {
		Self { l4: [[16u8; 16]; 6], l8: [[16u8; 64]; 6] }
	}

	/// The weights a sequence or picture parameter set codes (§7.4.2.1.1.1, Table 7-2).
	///
	/// `fallback_a` chooses between the two fall-back rules: rule A is what a *sequence* parameter
	/// set uses, and what a picture parameter set uses when its sequence carried no lists; rule B is
	/// what a picture parameter set uses when its sequence did carry them, and inherits from the
	/// sequence rather than from the defaults.
	///
	/// **Not present does not mean flat.** At the head of each fall-back chain -- lists 0, 3, 6 and
	/// 7 -- an absent list means the *default* matrix of Table 7-3 or 7-4, which is not flat;
	/// elsewhere it means the list before it in the chain. Reading absence as no scaling produces a
	/// picture that is recognisable and wrong.
	fn read(b: &mut Bits, count: usize, prev: Option<&Scaling>, fallback_a: bool) -> Outcome<Self> {
		let mut out = match (fallback_a, prev) {
			(false, Some(p))	=> p.clone(),
			_			=> Self::flat(),
		};
		// Where a list is absent, what it falls back to.
		for i in 0..count {
			let present = res!(b.flag());
			if i < 6 {
				let mut list = [0u8; 16];
				let mut default = false;
				if present {
					default = res!(read_list(b, &mut list));
				}
				out.l4[i] = if !present {
					match (i, fallback_a, prev) {
						// The head of a chain, falling back on the defaults.
						(0, _, _)		=> DEFAULT_4X4_INTRA,
						(3, _, _)		=> DEFAULT_4X4_INTER,
						// A picture set whose sequence carried lists inherits them.
						(_, false, Some(p))	=> p.l4[i],
						// Otherwise the list before it in the chain.
						_			=> out.l4[i - 1],
					}
				} else if default {
					if i < 3 { DEFAULT_4X4_INTRA } else { DEFAULT_4X4_INTER }
				} else {
					list
				};
			} else {
				let j = i - 6;
				let mut list = [0u8; 64];
				let mut default = false;
				if present {
					default = res!(read_list(b, &mut list));
				}
				out.l8[j] = if !present {
					match (j, fallback_a, prev) {
						(0, _, _)		=> DEFAULT_8X8_INTRA,
						(1, _, _)		=> DEFAULT_8X8_INTER,
						(_, false, Some(p))	=> p.l8[j],
						// The eight-by-eight chain steps two at a time, since the lists alternate
						// intra and inter by colour component.
						_			=> out.l8[j - 2],
					}
				} else if default {
					if j % 2 == 0 { DEFAULT_8X8_INTRA } else { DEFAULT_8X8_INTER }
				} else {
					list
				};
			}
		}
		Ok(out)
	}
}

/// Reads one scaling list, and says whether it asked for the default matrix (§7.3.2.1.1.1).
///
/// A first delta that takes the running value to nought is the encoder's way of naming the default
/// matrix without carrying it; a later one means the list stops there and every entry after it
/// repeats the last.
fn read_list(b: &mut Bits, list: &mut [u8]) -> Outcome<bool> {
	let mut last = 8i32;
	let mut next = 8i32;
	let mut default = false;
	for j in 0..list.len() {
		if next != 0 {
			let delta = res!(b.se());
			next = (last + delta + 256).rem_euclid(256);
			if j == 0 && next == 0 {
				default = true;
			}
		}
		let v = if next == 0 { last } else { next };
		if !(1..=255).contains(&v) {
			return Err(err!(
				"A scaling list entry of {} was coded, and the weights run from 1 to 255.", v;
			Invalid, Input, Decode));
		}
		list[j] = v as u8;
		last = v;
	}
	Ok(default)
}

/// The parameter sets carried in an `avcC` decoder configuration record (ISO/IEC 14496-15 §5.3.3.1).
#[derive(Clone, Debug)]
pub struct Config {
	/// How many bytes prefix each NAL unit in the film's own samples.
	pub length_size:	usize,
	/// The sequence parameter sets.
	pub sps:	Vec<Unit>,
	/// The picture parameter sets.
	pub pps:	Vec<Unit>,
}

/// Reads an `avcC` record.
pub fn config(bytes: &[u8]) -> Outcome<Config> {
	// version, profile, compatibility, level, then the length size and the parameter set counts.
	if bytes.len() < 7 {
		return Err(err!(
			"An AVC decoder configuration record is {} bytes, and its fixed fields alone are 7.",
			bytes.len();
		Invalid, Input, Decode));
	}
	if bytes[0] != 1 {
		return Err(err!(
			"An AVC decoder configuration record of version {}, and this reads version 1.",
			bytes[0];
		Invalid, Input, Unknown));
	}
	let length_size = (bytes[4] & 0x03) as usize + 1;
	if length_size == 3 {
		return Err(err!(
			"The configuration record names a NAL length of 3 bytes, which ISO/IEC 14496-15 does \
			not allow; it must be 1, 2 or 4.";
		Invalid, Input, Decode));
	}
	let mut at = 5usize;
	let take = |at: &mut usize, count: usize, into: &mut Vec<Unit>| -> Outcome<()> {
		for _ in 0..count {
			if *at + 2 > bytes.len() {
				return Err(err!(
					"A configuration record ends inside a parameter set's length.";
				Invalid, Input, Decode));
			}
			let len = u16::from_be_bytes([bytes[*at], bytes[*at + 1]]) as usize;
			*at += 2;
			let end = match at.checked_add(len) {
				Some(end) if end <= bytes.len() => end,
				_ => return Err(err!(
					"A parameter set says it is {} bytes and {} remain.",
					len, bytes.len() - *at;
				Invalid, Input, Decode)),
			};
			into.push(res!(unit(&bytes[*at..end])));
			*at = end;
		}
		Ok(())
	};
	let mut sps = Vec::new();
	let mut pps = Vec::new();
	let n_sps = (bytes[5] & 0x1f) as usize;
	at += 1;
	res!(take(&mut at, n_sps, &mut sps));
	if at >= bytes.len() {
		return Err(err!(
			"A configuration record ends before its picture parameter set count.";
		Invalid, Input, Decode));
	}
	let n_pps = bytes[at] as usize;
	at += 1;
	res!(take(&mut at, n_pps, &mut pps));
	Ok(Config { length_size, sps, pps })
}

/// Splits a byte-stream of length-prefixed NAL units, as a sample carries them.
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

/// Reads one NAL unit: its one-byte header, and its payload unescaped (§7.3.1).
pub fn unit(raw: &[u8]) -> Outcome<Unit> {
	if raw.len() < 2 {
		return Err(err!(
			"A NAL unit is {} bytes, and its header alone is one.", raw.len();
		Invalid, Input, Decode));
	}
	if raw[0] & 0x80 != 0 {
		return Err(err!(
			"A NAL unit's forbidden bit is set, so this is not an H.264 stream.";
		Invalid, Input, Decode));
	}
	Ok(Unit {
		ref_idc:	(raw[0] >> 5) & 0x03,
		kind:		raw[0] & 0x1f,
		body:		rbsp(&raw[1..]),
	})
}

/// Removes the emulation prevention bytes from a payload (§7.4.1).
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

// ------------------------------------------------------------------------ reading the bits

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

	/// A reader positioned at a given bit.
	pub fn at(buf: &'a [u8], pos: usize) -> Self {
		Self { buf, pos }
	}

	/// How many bits are left.
	pub fn left(&self) -> usize {
		(self.buf.len() * 8).saturating_sub(self.pos)
	}

	/// How many bits have been read.
	pub fn consumed(&self) -> usize {
		self.pos
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
					"The payload ends after {} bits, inside a field.", self.buf.len() * 8;
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

	/// The next `n` bits without moving on, zero-padded past the end.
	///
	/// This is what a variable-length code table is looked up with: the longest code is peeked at
	/// whole, matched, and only then are the bits it used given up.
	pub fn peek(&self, n: usize) -> u32 {
		let mut v = 0u32;
		for i in 0..n.min(32) {
			let p = self.pos + i;
			let byte = p >> 3;
			let bit = match self.buf.get(byte) {
				Some(b) => (*b >> (7 - (p & 7))) & 1,
				None => 0,
			};
			v = (v << 1) | bit as u32;
		}
		v
	}

	/// Steps over `n` bits, refusing to step past the end.
	pub fn skip(&mut self, n: usize) -> Outcome<()> {
		if n > self.left() {
			return Err(err!(
				"{} bits were stepped over and {} remain.", n, self.left();
			Invalid, Input, Decode));
		}
		self.pos += n;
		Ok(())
	}

	/// An unsigned Exp-Golomb code, §9.1.
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

	/// A signed Exp-Golomb code, §9.1.1.
	pub fn se(&mut self) -> Outcome<i32> {
		let k = res!(self.ue());
		let m = ((k as i64 + 1) / 2) as i32;
		Ok(if k % 2 == 1 { m } else { -m })
	}

	/// Whether any syntax remains before the trailing bits (§7.2, `more_rbsp_data`).
	///
	/// The payload ends with a one bit and then zeroes to the byte boundary, so what is left is
	/// syntax only if there is a set bit somewhere after the current position other than that one.
	/// The picture parameter set's last three fields are read or not on this answer alone, and
	/// getting it wrong loses the eight-by-eight transform flag on every High profile film.
	pub fn more_data(&self) -> bool {
		let total = self.buf.len() * 8;
		if self.pos >= total {
			return false;
		}
		// The last set bit in the payload is the stop bit.
		let mut last = total;
		while last > 0 {
			let p = last - 1;
			let byte = p >> 3;
			let bit = match self.buf.get(byte) {
				Some(b) => (*b >> (7 - (p & 7))) & 1,
				None => 0,
			};
			if bit == 1 {
				break;
			}
			last -= 1;
		}
		// `last` is one past the stop bit, so the syntax runs out at `last - 1`.
		self.pos + 1 <= last.saturating_sub(1)
	}
}

// ------------------------------------------------------------------ the parameter sets

/// Reads a sequence parameter set (§7.3.2.1.1).
pub fn sps(body: &[u8]) -> Outcome<Sps> {
	let mut b = Bits::new(body);
	let profile = res!(b.u(8)) as u8;
	// Six constraint flags and two reserved bits.
	res!(b.skip(8));
	let level = res!(b.u(8)) as u8;
	let id = res!(b.ue());
	if id > 31 {
		return Err(err!(
			"A sequence parameter set numbered {}, and 31 is the highest.", id;
		Invalid, Input, Decode));
	}
	let mut chroma = 1u32;
	let mut separate_planes = false;
	let mut luma_bits = 8u32;
	let mut chroma_bits = 8u32;
	let mut qpprime_bypass = false;
	let mut scaling = None;
	// The profiles whose parameter sets carry a chroma format and scaling lists. Every other
	// profile is 4:2:0 at eight bits with no lists.
	if matches!(profile, 100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135) {
		chroma = res!(b.ue());
		if chroma == 3 {
			separate_planes = res!(b.flag());
		}
		luma_bits = res!(b.ue()) + 8;
		chroma_bits = res!(b.ue()) + 8;
		qpprime_bypass = res!(b.flag());
		if res!(b.flag()) {
			let count = if chroma != 3 { 8 } else { 12 };
			scaling = Some(res!(Scaling::read(&mut b, count, None, true)));
		}
	}
	if chroma > 3 {
		return Err(err!(
			"A chroma_format_idc of {} was coded, and 0 to 3 are the only ones defined.", chroma;
		Invalid, Input, Decode));
	}
	let frame_num_bits = res!(b.ue()) + 4;
	if frame_num_bits > 16 {
		return Err(err!(
			"frame_num is coded in {} bits, and 16 is the most allowed.", frame_num_bits;
		Invalid, Input, Decode));
	}
	let poc_type = res!(b.ue());
	let mut poc_lsb_bits = 0u32;
	let mut delta_poc_always_zero = false;
	match poc_type {
		0 => {
			poc_lsb_bits = res!(b.ue()) + 4;
			if poc_lsb_bits > 16 {
				return Err(err!(
					"The picture order count's low half is {} bits, and 16 is the most allowed.",
					poc_lsb_bits;
				Invalid, Input, Decode));
			}
		},
		1 => {
			delta_poc_always_zero = res!(b.flag());
			let _offset_non_ref = res!(b.se());
			let _offset_top_bottom = res!(b.se());
			let cycle = res!(b.ue());
			if cycle > 255 {
				return Err(err!(
					"A picture order count cycle of {} entries was coded, and 255 is the most \
					allowed.", cycle;
				Invalid, Input, Decode));
			}
			for _ in 0..cycle {
				let _offset = res!(b.se());
			}
		},
		2 => {},
		other => return Err(err!(
			"A picture order count type of {} was coded, and 0, 1 and 2 are the only ones \
			defined.", other;
		Invalid, Input, Decode)),
	}
	let _max_num_ref_frames = res!(b.ue());
	let _gaps_allowed = res!(b.flag());
	let mbs_w = res!(b.ue()) + 1;
	let map_units_h = res!(b.ue()) + 1;
	let frame_mbs_only = res!(b.flag());
	let mbaff = if frame_mbs_only { false } else { res!(b.flag()) };
	let _direct_8x8 = res!(b.flag());
	let mut crop = [0u32; 4];
	if res!(b.flag()) {
		for c in crop.iter_mut() {
			*c = res!(b.ue());
		}
	}
	// The video usability information changes no sample, so it is not read.

	let coded_w = mbs_w * 16;
	let coded_h = map_units_h * 16 * if frame_mbs_only { 1 } else { 2 };
	if coded_w > MAX_SIDE || coded_h > MAX_SIDE {
		return Err(err!(
			"A picture of {} by {} luma samples was coded, and {} each way is this decoder's \
			ceiling.", coded_w, coded_h, MAX_SIDE;
		Invalid, Input, Size));
	}
	// The crop offsets are counted in chroma samples across and in chroma samples times the field
	// factor down (§7.4.2.1.1).
	let (cw, ch) = match chroma {
		0	=> (1u32, 1u32),
		1	=> (2, 2),
		2	=> (2, 1),
		_	=> (1, 1),
	};
	let (unit_x, unit_y) = if chroma == 0 || separate_planes {
		(1u32, if frame_mbs_only { 1 } else { 2 })
	} else {
		(cw, ch * if frame_mbs_only { 1 } else { 2 })
	};
	let cut_w = unit_x * (crop[0] + crop[1]);
	let cut_h = unit_y * (crop[2] + crop[3]);
	if cut_w >= coded_w || cut_h >= coded_h {
		return Err(err!(
			"A cropping window takes {} by {} from a picture of {} by {}, leaving nothing.",
			cut_w, cut_h, coded_w, coded_h;
		Invalid, Input, Decode));
	}
	Ok(Sps {
		id:		id as u8,
		profile,
		level,
		chroma:		chroma as u8,
		separate_planes,
		luma_bits:	luma_bits as u8,
		chroma_bits:	chroma_bits as u8,
		qpprime_bypass,
		mbs_w,
		map_units_h,
		frame_mbs_only,
		mbaff,
		frame_num_bits,
		poc_type,
		poc_lsb_bits,
		delta_poc_always_zero,
		crop,
		scaling,
		coded_w,
		coded_h,
		width:		coded_w - cut_w,
		height:		coded_h - cut_h,
	})
}

/// Reads a picture parameter set (§7.3.2.2).
///
/// The sequence parameter set it references has to be in hand, because the number of scaling lists
/// the set may carry depends on the chroma format, and because the lists themselves fall back on
/// the sequence's where it has any.
pub fn pps(body: &[u8], sets: &[Sps]) -> Outcome<Pps> {
	let mut b = Bits::new(body);
	let id = res!(b.ue());
	if id > 255 {
		return Err(err!(
			"A picture parameter set numbered {}, and 255 is the highest.", id;
		Invalid, Input, Decode));
	}
	let sps_id = res!(b.ue());
	let sps = match sets.iter().find(|s| s.id as u32 == sps_id) {
		Some(s) => s,
		None => return Err(err!(
			"A picture parameter set references sequence parameter set {}, and the stream carries \
			{:?}.", sps_id, sets.iter().map(|s| s.id).collect::<Vec<_>>();
		Invalid, Input, Missing)),
	};
	let cabac = res!(b.flag());
	let bottom_field_order = res!(b.flag());
	let slice_groups = res!(b.ue()) + 1;
	if slice_groups > 1 {
		return Err(err!(
			"A picture is cut into {} slice groups, and this decoder reads one. Slice groups \
			reorder macroblocks through a map, and every film in the corpus this was written \
			against uses a single group.", slice_groups;
		Invalid, Input, Unimplemented));
	}
	let _num_ref_idx_l0 = res!(b.ue());
	let _num_ref_idx_l1 = res!(b.ue());
	let _weighted_pred = res!(b.flag());
	let _weighted_bipred = res!(b.u(2));
	let init_qp = res!(b.se()) + 26;
	let _init_qs = res!(b.se()) + 26;
	let cb_qp_offset = res!(b.se());
	let deblocking_control = res!(b.flag());
	let constrained_intra = res!(b.flag());
	let redundant_pic_cnt = res!(b.flag());
	let mut transform_8x8 = false;
	let mut scaling = sps.scaling.clone();
	let mut cr_qp_offset = cb_qp_offset;
	if b.more_data() {
		transform_8x8 = res!(b.flag());
		if res!(b.flag()) {
			let count = 6 + if sps.chroma != 3 { 2 } else { 6 } * usize::from(transform_8x8);
			scaling = Some(res!(Scaling::read(
				&mut b, count, sps.scaling.as_ref(), sps.scaling.is_none())));
		}
		cr_qp_offset = res!(b.se());
	}
	for (name, v) in [("chroma_qp_index_offset", cb_qp_offset),
			("second_chroma_qp_index_offset", cr_qp_offset)] {
		if !(-12..=12).contains(&v) {
			return Err(err!(
				"A {} of {} was coded, and it runs from -12 to 12.", name, v;
			Invalid, Input, Decode));
		}
	}
	if !(0..=51).contains(&init_qp) {
		return Err(err!(
			"A picture starts at a quantisation parameter of {}, and it runs from 0 to 51.",
			init_qp;
		Invalid, Input, Decode));
	}
	Ok(Pps {
		id:	id as u8,
		sps_id:	sps_id as u8,
		cabac,
		bottom_field_order,
		slice_groups,
		init_qp,
		cb_qp_offset,
		cr_qp_offset,
		deblocking_control,
		constrained_intra,
		redundant_pic_cnt,
		transform_8x8,
		scaling,
	})
}

/// Reads a slice header (§7.3.3), leaving the reader at the first bit of the slice data.
///
/// The header is read past rather than into wherever a field changes no sample: reference picture
/// list modification and the decoded reference picture marking both have to be *walked*, because
/// they are runs of variable-length codes and there is no skipping to what follows them.
pub fn slice(u: &Unit, sets: &[Sps], pics: &[Pps]) -> Outcome<Slice> {
	let idr = u.kind == nal::IDR;
	let mut b = Bits::new(&u.body);
	let first_mb = res!(b.ue());
	let code = res!(b.ue());
	if code > 9 {
		return Err(err!(
			"A slice_type of {} was coded, and 0 to 9 are the only ones defined.", code;
		Invalid, Input, Decode));
	}
	let kind = res!(SliceType::of(code));
	let pps_id = res!(b.ue());
	let pps = match pics.iter().find(|p| p.id as u32 == pps_id) {
		Some(p) => p,
		None => return Err(err!(
			"A slice references picture parameter set {}, and the stream carries {:?}.",
			pps_id, pics.iter().map(|p| p.id).collect::<Vec<_>>();
		Invalid, Input, Missing)),
	};
	let sps = match sets.iter().find(|s| s.id == pps.sps_id) {
		Some(s) => s,
		None => return Err(err!(
			"A picture parameter set references sequence parameter set {}, which the stream does \
			not carry.", pps.sps_id;
		Invalid, Input, Missing)),
	};
	if sps.separate_planes {
		let _colour_plane_id = res!(b.u(2));
	}
	let _frame_num = res!(b.u(sps.frame_num_bits as usize));
	let mut field_pic = false;
	if !sps.frame_mbs_only {
		field_pic = res!(b.flag());
		if field_pic {
			let _bottom_field = res!(b.flag());
		}
	}
	if field_pic {
		return Err(err!(
			"A slice codes a field rather than a frame. This decoder reads frames, and every film \
			in the corpus it was written against sets frame_mbs_only_flag.";
		Invalid, Input, Unimplemented));
	}
	if idr {
		let _idr_pic_id = res!(b.ue());
	}
	if sps.poc_type == 0 {
		let _poc_lsb = res!(b.u(sps.poc_lsb_bits as usize));
		if pps.bottom_field_order && !field_pic {
			let _delta_poc_bottom = res!(b.se());
		}
	}
	if sps.poc_type == 1 && !sps.delta_poc_always_zero {
		let _delta_poc_0 = res!(b.se());
		if pps.bottom_field_order && !field_pic {
			let _delta_poc_1 = res!(b.se());
		}
	}
	if pps.redundant_pic_cnt {
		let redundant = res!(b.ue());
		if redundant != 0 {
			return Err(err!(
				"A redundant coded picture was met, at redundant_pic_cnt {}. This decoder reads \
				the primary picture only.", redundant;
			Invalid, Input, Unimplemented));
		}
	}
	if !kind.is_intra() {
		return Err(err!(
			"A {:?} slice was met, and this decoder reads intra slices. The first coded picture \
			of a film is an IDR and every slice of it is intra; a {:?} slice means the caller \
			handed over a picture that is not the first.", kind, kind;
		Invalid, Input, Unimplemented));
	}
	// `ref_pic_list_modification`: an I slice codes only the two flags' absence, so for a slice
	// whose type is intra there is nothing here at all (§7.3.3.1).
	if u.ref_idc != 0 {
		// `dec_ref_pic_marking` (§7.3.3.3).
		if idr {
			let _no_output_of_prior_pics = res!(b.flag());
			let _long_term_reference = res!(b.flag());
		} else {
			if res!(b.flag()) {
				loop {
					let op = res!(b.ue());
					if op == 0 {
						break;
					}
					if op > 6 {
						return Err(err!(
							"A memory management control operation of {} was coded, and 0 to 6 \
							are the only ones defined.", op;
						Invalid, Input, Decode));
					}
					if matches!(op, 1 | 3) {
						let _difference_of_pic_nums = res!(b.ue());
					}
					if op == 2 {
						let _long_term_pic_num = res!(b.ue());
					}
					if matches!(op, 3 | 6) {
						let _long_term_frame_idx = res!(b.ue());
					}
					if op == 4 {
						let _max_long_term_frame_idx = res!(b.ue());
					}
				}
			}
		}
	}
	// `cabac_init_idc` is coded only for a slice that is not intra, so it is never read here; it
	// is kept in the header for the shape of the thing and is always nought.
	let cabac_init_idc = 0u32;
	let qp_delta = res!(b.se());
	let qp = pps.init_qp + qp_delta;
	if !(0..=51).contains(&qp) {
		return Err(err!(
			"A slice starts at a quantisation parameter of {}, and it runs from 0 to 51.", qp;
		Invalid, Input, Decode));
	}
	let mut deblocking = 0u32;
	let mut alpha_offset = 0i32;
	let mut beta_offset = 0i32;
	if pps.deblocking_control {
		deblocking = res!(b.ue());
		if deblocking > 2 {
			return Err(err!(
				"A disable_deblocking_filter_idc of {} was coded, and 0, 1 and 2 are the only \
				ones defined.", deblocking;
			Invalid, Input, Decode));
		}
		if deblocking != 1 {
			alpha_offset = res!(b.se()) * 2;
			beta_offset = res!(b.se()) * 2;
		}
	}
	Ok(Slice {
		first_mb,
		kind,
		all_same:	code >= 5,
		pps_id:		pps_id as u8,
		idr,
		qp,
		deblocking,
		alpha_offset,
		beta_offset,
		cabac_init_idc,
		data_bit:	b.consumed(),
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_a_payload_gives_up_its_escapes_01() -> Outcome<()> {
		// Emulation prevention is the one transformation that stands between the bytes in the file
		// and every field read out of them, so it is checked on the three shapes that occur: a byte
		// that is escaped, a byte that looks escaped and is not, and a run of zeroes.
		req!(rbsp(&[0x00, 0x00, 0x03, 0x01]), vec![0x00, 0x00, 0x01]);
		// Not after two zeroes, so not an escape.
		req!(rbsp(&[0x00, 0x03, 0x01]), vec![0x00, 0x03, 0x01]);
		// The escape resets the count, so the next 0x03 needs two more zeroes in front of it.
		req!(rbsp(&[0x00, 0x00, 0x03, 0x00, 0x03]), vec![0x00, 0x00, 0x00, 0x03]);
		req!(rbsp(&[0x00, 0x00, 0x03, 0x00, 0x00, 0x03]), vec![0x00, 0x00, 0x00, 0x00]);
		Ok(())
	}

	#[test]
	fn test_more_data_stops_at_the_trailing_bits_02() -> Outcome<()> {
		// The picture parameter set's last three fields -- the eight-by-eight transform flag among
		// them -- are read or not on this answer alone. On a High profile film the flag is on, so a
		// reader that says "no more data" one bit early decodes every one of them as a picture with
		// no eight-by-eight transform, which is a wrong picture rather than an error.
		//
		// A payload of one byte 0b1011_0000: three bits of syntax, then the stop bit, then padding.
		let buf = [0b1011_0000u8];
		let mut b = Bits::new(&buf);
		for i in 0..3 {
			let more = b.more_data();
			req!(more, true, "the payload ran out after {} bits of syntax and it holds three", i);
			let _ = res!(b.u(1));
		}
		let more = b.more_data();
		req!(more, false, "the stop bit was read as syntax");

		// And a payload whose stop bit is the last bit of the last byte.
		let buf = [0xFFu8, 0x81];
		let mut b = Bits::new(&buf);
		res!(b.skip(14));
		req!(b.more_data(), true);
		res!(b.skip(1));
		req!(b.more_data(), false, "the stop bit at the very end was read as syntax");
		Ok(())
	}

	#[test]
	fn test_an_absent_scaling_list_is_not_a_flat_one_03() -> Outcome<()> {
		// Table 7-2's fall-back rules. A sequence that turns the scaling matrices on and carries no
		// list of its own does not mean "no scaling"; it means the default matrices, which are not
		// flat. This is the difference, stated at the smallest scale: eight absent lists.
		//
		// One byte of eight zero bits: eight `seq_scaling_list_present_flag`s, all off.
		let buf = [0x00u8];
		let mut b = Bits::new(&buf);
		let s = res!(Scaling::read(&mut b, 8, None, true));
		req!(s.l4[0], DEFAULT_4X4_INTRA, "list 0 fell back on something other than the default");
		// Lists 1 and 2 inherit from the one before them, so they are the intra default too.
		req!(s.l4[1], DEFAULT_4X4_INTRA);
		req!(s.l4[2], DEFAULT_4X4_INTRA);
		req!(s.l4[3], DEFAULT_4X4_INTER, "list 3 heads its own chain and did not take the default");
		req!(s.l4[4], DEFAULT_4X4_INTER);
		req!(s.l4[5], DEFAULT_4X4_INTER);
		req!(s.l8[0], DEFAULT_8X8_INTRA);
		req!(s.l8[1], DEFAULT_8X8_INTER);
		// And the thing this is a guard against: none of them is flat.
		let flat = Scaling::flat();
		let same = s.l4[0] == flat.l4[0];
		req!(same, false, "an absent scaling list was read as no scaling at all");
		Ok(())
	}
}
