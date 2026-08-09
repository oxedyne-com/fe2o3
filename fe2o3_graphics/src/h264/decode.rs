//! Walking a coded picture and building the samples back up as it goes.
//!
//! This is where the syntax of clause 7.3.5 meets the decoding processes of clause 8. The two are
//! interleaved rather than done in turn, and they have to be: every block is predicted from the
//! samples around it, so a block cannot be predicted until the ones before it in decoding order
//! have been **reconstructed**, not merely parsed.
//!
//! The shape of the walk, outermost first:
//!
//! - **A slice at a time.** A slice is its own entropy-coded run, beginning at the macroblock its
//!   header names, and nothing in one slice may be predicted from another. Most pictures in the
//!   corpus are one slice, but 92 of the 1,658 are not.
//! - **A macroblock** is sixteen by sixteen luma samples and, in 4:2:0, eight by eight of each
//!   colour difference. Its type says how it is predicted: as sixteen four-by-four blocks, as four
//!   eight-by-eight ones, as one sixteen-by-sixteen, or as raw samples.
//! - **A block** is predicted from its neighbours, its residual read and transformed back, and the
//!   two added.
//!
//! # What this decodes and what it refuses
//!
//! Intra pictures in 4:2:0 at eight bits, coded in frames, with one slice group -- which is every
//! film in the library it was written against. Anything else is refused where it is read, by name,
//! rather than decoded into a wrong picture: field coding, macroblock-adaptive frame/field coding,
//! monochrome and 4:2:2 and 4:4:4, bit depths above eight, slice groups, and any slice that is not
//! intra.
//!
//! # The two things that have to be got right and cannot be seen
//!
//! **Availability.** A block predicts from its neighbours only where those have already been
//! decoded *and* belong to the same slice. What is kept here is one slice number per four-by-four
//! block, written as that block is reconstructed, which is exactly the question being asked and is
//! impossible to get subtly wrong. A decoder careless about it predicts from samples that are still
//! nought and produces a picture with a plausible grid of dark blocks.
//!
//! **The neighbour counts CAVLC reads its tables with.** Each four-by-four block's `nC` is the mean
//! of the number of coefficients in the blocks above and to the left, and it selects which of six
//! code tables reads the next token. Get it wrong and the right bits are read with the wrong code,
//! which desynchronises everything after it in the slice.

use crate::h264::{
	cabac::{
		self,
		Cat,
	},
	cavlc,
	intra::{
		self,
		Edges,
		Mode,
		Mode16,
		ModeC,
	},
	nal,
	split_lengthed,
	transform::{
		self,
		Weights,
		ZIGZAG_4X4,
		ZIGZAG_8X8,
	},
	Bits,
	Pps,
	Scaling,
	Sps,
	Unit,
};

use oxedyne_fe2o3_core::prelude::*;

/// One component's samples.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plane {
	/// Width in samples.
	pub w:	usize,
	/// Height in samples.
	pub h:	usize,
	/// The samples, row by row.
	pub px:	Vec<u8>,
}

impl Plane {

	/// A plane of nothing.
	fn new(w: usize, h: usize) -> Self {
		Self { w, h, px: vec![0; w * h] }
	}

	/// One sample, or `None` outside the plane.
	pub fn at(&self, x: usize, y: usize) -> Option<u8> {
		if x < self.w && y < self.h {
			self.px.get(y * self.w + x).copied()
		} else {
			None
		}
	}

	/// Writes one sample, ignoring a position outside the plane.
	fn put(&mut self, x: usize, y: usize, v: u8) {
		if x < self.w && y < self.h {
			self.px[y * self.w + x] = v;
		}
	}

	/// The plane cropped to a window, which is what the sequence parameter set's conformance
	/// window asks for.
	fn cropped(&self, w: usize, h: usize) -> Self {
		let mut out = Self::new(w, h);
		for y in 0..h.min(self.h) {
			let from = y * self.w;
			let to = from + w.min(self.w);
			let at = y * w;
			out.px[at..at + (to - from)].copy_from_slice(&self.px[from..to]);
		}
		out
	}
}

/// A decoded picture, before it is turned into anything anybody can look at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
	/// Brightness.
	pub y:	Plane,
	/// The two colour difference planes, at half the width and half the height.
	pub cb:	Plane,
	/// The other one.
	pub cr:	Plane,
}

/// How a macroblock is predicted (§7.4.5, Table 7-11).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
	/// Sixteen four-by-four blocks, each with its own direction.
	I4x4,
	/// Four eight-by-eight blocks, each with its own direction.
	I8x8,
	/// One prediction over the whole macroblock, with the sixteen blocks' direct current terms
	/// transformed together.
	I16x16,
	/// Raw samples, carried uncompressed.
	Pcm,
	/// A macroblock that has not been decoded, or is in another slice.
	Absent,
}

/// The mapping from `coded_block_pattern`'s code number to its value, for an intra macroblock in a
/// picture with colour (Table 9-4(a), the `Intra_4x4, Intra_8x8` column).
const CBP_INTRA: [u8; 48] = [
	47, 31, 15, 0, 23, 27, 29, 30, 7, 11, 13, 14, 39, 43, 45, 46,
	16, 3, 5, 10, 12, 19, 21, 26, 28, 35, 37, 42, 44, 1, 2, 4,
	8, 17, 18, 20, 24, 6, 9, 22, 25, 32, 33, 34, 36, 40, 38, 41,
];

/// Where each four-by-four luma block sits in the macroblock, in blocks (§6.4.3).
///
/// Not raster order: the blocks are walked a quadrant at a time, and within each quadrant a
/// quadrant again. Walking them in raster order instead predicts half the blocks from neighbours
/// that have not been decoded yet.
const fn blk_xy(i: usize) -> (usize, usize) {
	let quad = i / 4;
	let within = i % 4;
	(((quad % 2) * 2) + (within % 2), ((quad / 2) * 2) + (within / 2))
}

/// Which of a macroblock's transform blocks were coded with anything in them (§9.3.3.1.1.9).
///
/// The arithmetic coder reads each block's `coded_block_flag` against a context chosen by the flags
/// of the blocks above and to the left, so a block's answer has to be kept for its neighbours -- and
/// the neighbour may be in another macroblock, which is why this is kept per macroblock rather than
/// discarded with the block.
#[derive(Clone, Copy, Debug, Default)]
struct Cbf {
	/// The block of direct current terms a macroblock predicted whole carries.
	luma_dc:	bool,
	/// The sixteen four-by-four luma blocks, in the macroblock's own block order.
	luma4:		[bool; 16],
	/// The four eight-by-eight ones, where the macroblock uses that transform.
	luma8:		[bool; 4],
	/// Each colour difference component's block of direct current terms.
	chroma_dc:	[bool; 2],
	/// Their four alternating current blocks each.
	chroma_ac:	[[bool; 4]; 2],
}

/// Everything a picture's decoder carries from one macroblock to the next.
struct Frame<'a> {
	/// The sequence parameter set in force.
	sps:	&'a Sps,
	/// The picture parameter set in force.
	pps:	&'a Pps,
	/// The picture being built.
	pic:	Picture,
	/// The picture's width in macroblocks.
	mbs_w:	usize,
	/// Its height in macroblocks.
	mbs_h:	usize,
	/// Which slice each macroblock belongs to, or `None` where none has been decoded.
	slice_of:	Vec<Option<usize>>,
	/// How each macroblock is predicted.
	kind:	Vec<Kind>,
	/// Each macroblock's quantisation parameter, for the deblocking filter.
	qp:	Vec<i32>,
	/// Whether each macroblock is coded with the eight-by-eight transform, which says which of its
	/// internal edges the deblocking filter visits.
	big:	Vec<bool>,
	/// The intra prediction mode of each four-by-four luma block, in the macroblock's own order.
	modes:	Vec<[u8; 16]>,
	/// How many coefficients each four-by-four block holds, for CAVLC's neighbour counts: sixteen
	/// luma then four of each colour difference.
	counts:	Vec<[u8; 24]>,
	/// Each macroblock's luma coded block pattern, which CABAC's neighbour contexts ask about.
	cbp_luma:	Vec<u8>,
	/// The same for the colour difference planes.
	cbp_chroma:	Vec<u8>,
	/// Each macroblock's chroma prediction mode, which is one of CABAC's contexts too.
	chroma_mode:	Vec<u8>,
	/// Whether each macroblock's quantisation parameter moved, which chooses the context the next
	/// one's delta is read against.
	qp_moved:	Vec<bool>,
	/// Which of each macroblock's transform blocks hold a coefficient.
	cbf:	Vec<Cbf>,
	/// The luma weights, already scanned.
	w_luma:	Weights,
	/// The Cb weights.
	w_cb:	Weights,
	/// The Cr weights.
	w_cr:	Weights,
}

impl<'a> Frame<'a> {

	/// A picture with nothing decoded into it.
	fn new(sps: &'a Sps, pps: &'a Pps) -> Outcome<Self> {
		let mbs_w = sps.mbs_w as usize;
		let mbs_h = sps.map_units_h as usize;
		let n = mbs_w * mbs_h;
		let scaling = match &pps.scaling {
			Some(s) => s.clone(),
			None => Scaling::flat(),
		};
		Ok(Self {
			sps,
			pps,
			pic: Picture {
				y:	Plane::new(mbs_w * 16, mbs_h * 16),
				cb:	Plane::new(mbs_w * 8, mbs_h * 8),
				cr:	Plane::new(mbs_w * 8, mbs_h * 8),
			},
			mbs_w,
			mbs_h,
			slice_of:	vec![None; n],
			kind:		vec![Kind::Absent; n],
			qp:		vec![0; n],
			big:		vec![false; n],
			modes:		vec![[2u8; 16]; n],
			counts:		vec![[0u8; 24]; n],
			cbp_luma:	vec![0; n],
			cbp_chroma:	vec![0; n],
			chroma_mode:	vec![0; n],
			qp_moved:	vec![false; n],
			cbf:		vec![Cbf::default(); n],
			w_luma:		Weights::intra(&scaling, 0),
			w_cb:		Weights::intra(&scaling, 1),
			w_cr:		Weights::intra(&scaling, 2),
		})
	}

	/// Whether a macroblock has been decoded and belongs to the given slice.
	fn available(&self, mb: i64, slice: usize) -> bool {
		if mb < 0 || mb as usize >= self.slice_of.len() {
			return false;
		}
		self.slice_of[mb as usize] == Some(slice)
	}

	/// The macroblock to the left, above, above-right and above-left, where each is available.
	fn around(&self, mb: usize, slice: usize) -> [Option<usize>; 4] {
		let w = self.mbs_w as i64;
		let m = mb as i64;
		let col = m % w;
		let a = if col > 0 { m - 1 } else { -1 };
		let b = m - w;
		let c = if col + 1 < w { m - w + 1 } else { -1 };
		let d = if col > 0 { m - w - 1 } else { -1 };
		let mut out = [None; 4];
		for (i, n) in [a, b, c, d].into_iter().enumerate() {
			if self.available(n, slice) {
				out[i] = Some(n as usize);
			}
		}
		out
	}
}

/// The state one slice's decoder carries between macroblocks.
struct SliceRun {
	/// Which slice this is, counted from nought within the picture.
	index:	usize,
	/// The running quantisation parameter.
	qp:	i32,
	/// Whether the picture parameter set allows the eight-by-eight transform.
	transform_8x8:	bool,
}

/// Decodes the first coded picture of a film.
///
/// `config` is the `avcC` decoder configuration record and `sample` is one access unit as the
/// container stores it: NAL units each behind a length prefix. Parameter sets carried in the sample
/// itself override the record's, which is how an `avc3` stream works and which costs nothing to
/// support.
pub fn picture(config: &[u8], sample: &[u8]) -> Outcome<Picture> {
	whole(config, sample, true)
}

/// The same picture, **before** the deblocking filter has run.
///
/// Not a picture anybody should look at: it is the reconstruction the filter is meant to smooth,
/// and it is here because it is the only way to tell a fault in prediction or in the residual from
/// a fault in the filter. FFmpeg will produce the same thing on demand -- `-skip_loop_filter all`
/// -- so the two halves of a decode can be held to it separately, and a mismatch says which half.
pub fn picture_undeblocked(config: &[u8], sample: &[u8]) -> Outcome<Picture> {
	whole(config, sample, false)
}

/// Decodes one access unit, with or without the filter that finishes it.
fn whole(config: &[u8], sample: &[u8], deblock: bool) -> Outcome<Picture> {
	let cfg = res!(crate::h264::config(config));
	let mut sets = Vec::new();
	for u in &cfg.sps {
		sets.push(res!(crate::h264::sps(&u.body)));
	}
	let mut pics = Vec::new();
	for u in &cfg.pps {
		pics.push(res!(crate::h264::pps(&u.body, &sets)));
	}
	let units = res!(split_lengthed(sample, cfg.length_size));
	decode(&units, &mut sets, &mut pics, deblock)
}

/// Decodes one access unit's NAL units into a picture.
pub fn decode(units: &[Unit], sets: &mut Vec<Sps>, pics: &mut Vec<Pps>, deblock: bool)
	-> Outcome<Picture>
{
	// Parameter sets carried in the sample itself come first, so that a slice reads the ones it
	// was coded against.
	for u in units {
		match u.kind {
			nal::SPS => {
				let s = res!(crate::h264::sps(&u.body));
				sets.retain(|o| o.id != s.id);
				sets.push(s);
			},
			nal::PPS => {
				let p = res!(crate::h264::pps(&u.body, sets));
				pics.retain(|o| o.id != p.id);
				pics.push(p);
			},
			_ => {},
		}
	}
	let slices: Vec<&Unit> = units.iter()
		.filter(|u| matches!(u.kind, nal::SLICE | nal::IDR))
		.collect();
	let first = match slices.first() {
		Some(u) => *u,
		None => return Err(err!(
			"The access unit carries no coded slice, only NAL units {:?}.",
			units.iter().map(|u| u.kind).collect::<Vec<_>>();
		Invalid, Input, Missing)),
	};
	let head = res!(crate::h264::slice(first, sets, pics));
	let pps = match pics.iter().find(|p| p.id == head.pps_id) {
		Some(p) => p,
		None => return Err(err!(
			"A slice references picture parameter set {}, which the stream does not carry.",
			head.pps_id; Invalid, Input, Missing)),
	};
	let sps = match sets.iter().find(|s| s.id == pps.sps_id) {
		Some(s) => s,
		None => return Err(err!(
			"A picture parameter set references sequence parameter set {}, which the stream does \
			not carry.", pps.sps_id; Invalid, Input, Missing)),
	};
	res!(refuse_what_is_not_read(sps, pps));
	let mut frame = res!(Frame::new(sps, pps));
	// Each slice carries its own deblocking disposition and its own thresholds, and they are not
	// a formality: 92 pictures in the corpus have more than one slice, and the two films whose
	// decode this was found by both turn the filter off *across slice boundaries only*.
	let mut filters: Vec<Filter> = Vec::with_capacity(slices.len());

	for (index, u) in slices.iter().enumerate() {
		let head = res!(crate::h264::slice(u, sets, pics));
		if head.pps_id != pps.id {
			return Err(err!(
				"Two slices of one picture reference picture parameter sets {} and {}. This \
				decoder reads a picture whose slices agree.", pps.id, head.pps_id;
			Invalid, Input, Unimplemented));
		}
		let mut run = SliceRun {
			index,
			qp:		head.qp,
			transform_8x8:	pps.transform_8x8,
		};
		filters.push(Filter {
			idc:	head.deblocking,
			alpha:	head.alpha_offset,
			beta:	head.beta_offset,
		});
		if pps.cabac {
			res!(slice_data_cabac(&mut frame, &mut run, u, head.first_mb as usize, head.data_bit));
		} else {
			res!(slice_data(&mut frame, &mut run, u, head.first_mb as usize, head.data_bit));
		}
	}
	if deblock {
		let mut view = frame_view(&mut frame);
		view.filters = &filters;
		res!(crate::h264::filter::deblock(&mut view));
	}
	Ok(crop(&frame))
}

/// Refuses, by name, everything the corpus does not contain and this decoder does not read.
fn refuse_what_is_not_read(sps: &Sps, pps: &Pps) -> Outcome<()> {
	if sps.chroma != 1 {
		return Err(err!(
			"The stream is coded at chroma_format_idc {}, and this decoder reads 4:2:0, which is \
			1. All 1,658 H.264 films in the corpus it was written against are 4:2:0.", sps.chroma;
		Invalid, Input, Unimplemented));
	}
	if sps.luma_bits != 8 || sps.chroma_bits != 8 {
		return Err(err!(
			"The stream is coded at {} bits of luma and {} of chroma, and this decoder reads \
			eight of each.", sps.luma_bits, sps.chroma_bits;
		Invalid, Input, Unimplemented));
	}
	if !sps.frame_mbs_only {
		return Err(err!(
			"The stream may code fields as well as frames (frame_mbs_only_flag is 0), and this \
			decoder reads frames.";
		Invalid, Input, Unimplemented));
	}
	if sps.mbaff {
		return Err(err!(
			"The stream uses macroblock-adaptive frame/field coding, and this decoder reads \
			frame macroblocks.";
		Invalid, Input, Unimplemented));
	}
	if pps.slice_groups > 1 {
		return Err(err!(
			"The picture is cut into {} slice groups, and this decoder reads one.",
			pps.slice_groups;
		Invalid, Input, Unimplemented));
	}
	if pps.constrained_intra {
		return Err(err!(
			"constrained_intra_pred_flag is set. For an all-intra picture it changes nothing, but \
			it is refused rather than ignored, because a picture that sets it and is not all \
			intra would decode wrongly.";
		Invalid, Input, Unimplemented));
	}
	if sps.qpprime_bypass {
		return Err(err!(
			"qpprime_y_zero_transform_bypass_flag is set, so a macroblock at a quantisation \
			parameter of nought skips the transform. This decoder does not read that.";
		Invalid, Input, Unimplemented));
	}
	Ok(())
}

/// Walks one slice's macroblocks (§7.3.4).
fn slice_data(f: &mut Frame, run: &mut SliceRun, u: &Unit, first_mb: usize, at: usize)
	-> Outcome<()>
{
	let mut b = Bits::at(&u.body, at);
	let mut mb = first_mb;
	let total = f.mbs_w * f.mbs_h;
	loop {
		if mb >= total {
			return Err(err!(
				"A slice ran past macroblock {} of a picture that holds {}.", mb, total;
			Invalid, Input, Decode));
		}
		res!(macroblock(f, run, &mut b, mb));
		mb += 1;
		// A slice ends where its payload does. `more_rbsp_data` is the whole of the test for a
		// slice coded with the length tables; there is no end-of-slice flag.
		if !b.more_data() {
			break;
		}
	}
	Ok(())
}

/// Reads and reconstructs one macroblock (§7.3.5).
fn macroblock(f: &mut Frame, run: &mut SliceRun, b: &mut Bits, mb: usize) -> Outcome<()> {
	let mb_type = res!(b.ue());
	if mb_type == 25 {
		return pcm(f, run, b, mb);
	}
	if mb_type > 25 {
		return Err(err!(
			"An mb_type of {} was coded in an intra slice, and 0 to 25 are the only ones defined.",
			mb_type; Invalid, Input, Decode));
	}
	let (kind, mut cbp_luma, mut cbp_chroma, pred16) = if mb_type == 0 {
		(Kind::I4x4, 0u8, 0u8, Mode16::Dc)
	} else {
		// Table 7-11: the twenty-four Intra_16x16 types are the prediction mode, the chroma
		// pattern and the luma pattern counted off in that order.
		let k = (mb_type - 1) as usize;
		let pred = res!(Mode16::of((k % 4) as u32));
		let chroma = ((k / 4) % 3) as u8;
		let luma = if k >= 12 { 15u8 } else { 0 };
		(Kind::I16x16, luma, chroma, pred)
	};
	let mut kind = kind;
	// The eight-by-eight transform is chosen per macroblock, and only where the picture parameter
	// set allows it at all.
	if kind == Kind::I4x4 && run.transform_8x8 && res!(b.flag()) {
		kind = Kind::I8x8;
	}
	// The prediction modes.
	let mut modes = [2u8; 16];
	if kind == Kind::I4x4 {
		for i in 0..16 {
			let predicted = res!(predicted_mode(f, run, mb, i, &modes, kind));
			modes[i] = res!(read_mode(b, predicted));
		}
	} else if kind == Kind::I8x8 {
		for i in 0..4 {
			let predicted = res!(predicted_mode(f, run, mb, i * 4, &modes, kind));
			let m = res!(read_mode(b, predicted));
			// An eight-by-eight block's mode is recorded against all four of its four-by-four
			// blocks, because that is where the next macroblock's prediction looks for it.
			for k in 0..4 {
				modes[i * 4 + k] = m;
			}
		}
	}
	let chroma_mode = if matches!(kind, Kind::I4x4 | Kind::I8x8 | Kind::I16x16) {
		res!(ModeC::of(res!(b.ue())))
	} else {
		ModeC::Dc
	};
	if kind != Kind::I16x16 {
		let code = res!(b.ue()) as usize;
		let cbp = match CBP_INTRA.get(code) {
			Some(v) => *v,
			None => return Err(err!(
				"A coded_block_pattern code number of {} was read, and the table holds 48.", code;
			Invalid, Input, Decode)),
		};
		cbp_luma = cbp & 15;
		cbp_chroma = cbp >> 4;
	}
	// Where nothing is coded at all, the quantisation parameter does not move.
	let mut qp = run.qp;
	if cbp_luma > 0 || cbp_chroma > 0 || kind == Kind::I16x16 {
		let delta = res!(b.se());
		if !(-26..=25).contains(&delta) {
			return Err(err!(
				"An mb_qp_delta of {} was coded, and it runs from -26 to 25.", delta;
			Invalid, Input, Decode));
		}
		// The parameter wraps rather than clipping, so that a delta may reach any value from any
		// other in one step (§7.4.5).
		qp = (run.qp + delta + 52).rem_euclid(52);
		run.qp = qp;
	}
	// The residual.
	let mut luma_dc = [0i32; 16];
	let mut luma = [[0i32; 16]; 16];
	let mut luma8 = [[0i32; 64]; 4];
	let mut chroma_dc = [[0i32; 4]; 2];
	let mut chroma = [[[0i32; 16]; 4]; 2];
	let mut counts = [0u8; 24];

	if kind == Kind::I16x16 {
		let nc = res!(luma_nc(f, run, mb, 0, &counts));
		let block = res!(cavlc::residual(b, nc, 16));
		for (i, at) in ZIGZAG_4X4.iter().enumerate() {
			luma_dc[*at] = block.levels[i];
		}
	}
	for i8 in 0..4usize {
		for i4 in 0..4usize {
			let blk = i8 * 4 + i4;
			if cbp_luma & (1 << i8) == 0 {
				continue;
			}
			let nc = res!(luma_nc(f, run, mb, blk, &counts));
			let (start, max) = if kind == Kind::I16x16 { (1usize, 15usize) } else { (0, 16) };
			let block = res!(cavlc::residual(b, nc, max));
			counts[blk] = block.total as u8;
			// A macroblock coded with the eight-by-eight transform still reads four
			// variable-length blocks and interleaves them, because CAVLC has no table for
			// sixty-four coefficients (§7.3.5.3.1).
			if kind == Kind::I8x8 {
				for (i, v) in block.levels.iter().enumerate() {
					luma8[i8][4 * i + i4] = *v;
				}
			} else {
				// Into raster order as they are read, since the scan is the only thing that
				// says where in the block a coefficient belongs. An `Intra_16x16` block's
				// alternating-current terms begin at scan position one, because position nought
				// is the direct current term that was transformed with the other fifteen.
				for (i, v) in block.levels.iter().enumerate() {
					luma[blk][ZIGZAG_4X4[start + i]] = *v;
				}
			}
		}
	}
	if cbp_chroma & 3 != 0 {
		for c in 0..2usize {
			let block = res!(cavlc::residual(b, -1, 4));
			chroma_dc[c].copy_from_slice(&block.levels[..4]);
		}
	}
	if cbp_chroma & 2 != 0 {
		for c in 0..2usize {
			for i in 0..4usize {
				let nc = res!(chroma_nc(f, run, mb, c, i, &counts));
				let block = res!(cavlc::residual(b, nc, 15));
				counts[16 + c * 4 + i] = block.total as u8;
				for (k, v) in block.levels.iter().enumerate() {
					chroma[c][i][ZIGZAG_4X4[1 + k]] = *v;
				}
			}
		}
	}

	// Record what the neighbours will ask about, before reconstruction, since reconstruction of a
	// later block in this macroblock reads it.
	f.slice_of[mb] = Some(run.index);
	f.kind[mb] = kind;
	f.qp[mb] = qp;
	f.modes[mb] = modes;
	f.counts[mb] = counts;
	f.big[mb] = kind == Kind::I8x8;

	if std::env::var("H264_TRACE").is_ok() && mb < 3 {
		eprintln!("mb {} type {} kind {:?} cbpL {} cbpC {} qp {} p16 {:?} ch {:?} modes {:?} \
			counts {:?} dc {:?}",
			mb, mb_type, kind, cbp_luma, cbp_chroma, qp, pred16, chroma_mode, modes,
			&counts[..16], &luma_dc[..4]);
	}
	res!(reconstruct(f, run, mb, kind, qp, pred16, chroma_mode, &modes, &luma_dc, &luma, &luma8,
		&chroma_dc, &chroma));
	Ok(())
}

/// Reads a raw-sample macroblock (§7.3.5).
fn pcm(f: &mut Frame, run: &mut SliceRun, b: &mut Bits, mb: usize) -> Outcome<()> {
	// The samples begin at the next byte boundary.
	let pad = (8 - (b.consumed() % 8)) % 8;
	res!(b.skip(pad));
	let (mx, my) = ((mb % f.mbs_w) * 16, (mb / f.mbs_w) * 16);
	for y in 0..16 {
		for x in 0..16 {
			let v = res!(b.u(8)) as u8;
			f.pic.y.put(mx + x, my + y, v);
		}
	}
	let (cx, cy) = ((mb % f.mbs_w) * 8, (mb / f.mbs_w) * 8);
	for c in 0..2 {
		for y in 0..8 {
			for x in 0..8 {
				let v = res!(b.u(8)) as u8;
				if c == 0 {
					f.pic.cb.put(cx + x, cy + y, v);
				} else {
					f.pic.cr.put(cx + x, cy + y, v);
				}
			}
		}
	}
	f.slice_of[mb] = Some(run.index);
	f.kind[mb] = Kind::Pcm;
	f.qp[mb] = 0;
	f.modes[mb] = [2u8; 16];
	// A raw macroblock counts as sixteen coefficients everywhere, for its neighbours' tables.
	f.counts[mb] = [16u8; 24];
	Ok(())
}

// -------------------------------------------------------------- the arithmetically coded walk

/// The arithmetic coder's state for one slice.
struct Entropy<'a> {
	/// The slice's payload, from the first bit of the NAL unit's own body.
	body:	&'a [u8],
	/// Where in that payload the decoding engine's own buffer begins, in bytes. It moves only for a
	/// raw-sample macroblock, after which the engine is started afresh.
	base:	usize,
	/// The decoding engine.
	c:	cabac::Cabac<'a>,
	/// The context variables.
	x:	cabac::Contexts,
	/// The macroblock decoded immediately before this one **in this slice**, where there was one.
	prev:	Option<usize>,
}

impl<'a> Entropy<'a> {

	/// The coder as a slice's entropy-coded data begins (§7.3.4, §9.3.1).
	///
	/// `at` is where the slice header ended, in bits. The data begins at the next byte boundary, and
	/// the bits between are `cabac_alignment_one_bit`s, which are all ones. They are checked rather
	/// than skipped: a header read one bit short lands here with a zero among them, and saying so is
	/// far better than decoding the whole picture from one bit out.
	fn new(body: &'a [u8], at: usize, qp: i32) -> Outcome<Self> {
		let mut b = Bits::at(body, at);
		while b.consumed() % 8 != 0 {
			if !res!(b.flag()) {
				return Err(err!(
					"A slice's cabac_alignment_one_bit at bit {} is nought, so the slice header was \
					not read to its end.", b.consumed() - 1;
				Invalid, Input, Decode));
			}
		}
		let base = b.consumed() / 8;
		Ok(Self {
			body,
			base,
			c:	res!(cabac::Cabac::new(&body[base..])),
			x:	cabac::Contexts::start(qp),
			prev:	None,
		})
	}

	/// One bin against the context at a `ctxIdx`.
	fn bin(&mut self, ctx_idx: usize) -> Outcome<u32> {
		self.x.bin(&mut self.c, ctx_idx)
	}

	/// Where the engine's next unread bit sits, in bytes from the start of the payload, rounded up.
	fn byte(&self) -> usize {
		self.base + self.c.consumed_bits().div_ceil(8)
	}

	/// Starts the engine afresh at a byte of the payload, which is what follows a raw-sample
	/// macroblock (§9.3.1.2).
	fn restart(&mut self, at: usize) -> Outcome<()> {
		let body = self.body;
		if at >= body.len() {
			return Err(err!(
				"An arithmetic decoder was to restart at byte {} of a payload of {}.", at, body.len();
			Invalid, Input, Decode));
		}
		self.base = at;
		self.c = res!(cabac::Cabac::new(&body[at..]));
		Ok(())
	}
}

/// What a macroblock's own neighbour lookups need before it has been recorded against the picture.
///
/// A block predicts its context from the blocks above and to the left, and half of those are inside
/// the macroblock being read. Reading them out of the picture instead would give every one of them
/// the answer for "not yet decoded", which decodes the first block of each macroblock correctly and
/// the rest wrongly.
struct Partial {
	/// How the macroblock is predicted.
	kind:	Kind,
	/// Its luma coded block pattern, as far as it has been read.
	cbp_luma:	u8,
	/// Its chroma one.
	cbp_chroma:	u8,
	/// Which of its transform blocks have been read, and what they held.
	cbf:	Cbf,
}

/// The macroblock to the left and the macroblock above, where each is available (§6.4.11.1).
fn ab(f: &Frame, run: &SliceRun, mb: usize) -> [Option<usize>; 2] {
	let around = f.around(mb, run.index);
	[around[0], around[1]]
}

/// The macroblock and four-by-four luma block each of a block's two neighbours sits in (§6.4.11.4).
fn luma4_ab(f: &Frame, run: &SliceRun, mb: usize, blk: usize) -> [Option<(usize, usize)>; 2] {
	let (bx, by) = blk_xy(blk);
	let around = f.around(mb, run.index);
	[
		if bx > 0 {
			Some((mb, blk_index(bx - 1, by)))
		} else {
			around[0].map(|n| (n, blk_index(3, by)))
		},
		if by > 0 {
			Some((mb, blk_index(bx, by - 1)))
		} else {
			around[1].map(|n| (n, blk_index(bx, 3)))
		},
	]
}

/// The same for an eight-by-eight luma block, which sit in plain raster order (§6.4.11.2).
fn luma8_ab(f: &Frame, run: &SliceRun, mb: usize, blk: usize) -> [Option<(usize, usize)>; 2] {
	let (bx, by) = (blk % 2, blk / 2);
	let around = f.around(mb, run.index);
	[
		if bx > 0 {
			Some((mb, by * 2 + bx - 1))
		} else {
			around[0].map(|n| (n, by * 2 + 1))
		},
		if by > 0 {
			Some((mb, (by - 1) * 2 + bx))
		} else {
			around[1].map(|n| (n, 2 + bx))
		},
	]
}

/// And for a four-by-four block of a 4:2:0 colour difference plane (§6.4.11.5).
fn chroma4_ab(f: &Frame, run: &SliceRun, mb: usize, blk: usize) -> [Option<(usize, usize)>; 2] {
	// A 4:2:0 macroblock's chroma is eight by eight, so its four blocks tile it two by two, which
	// makes the arithmetic the same as an eight-by-eight luma block's.
	luma8_ab(f, run, mb, blk)
}

/// Adds a neighbour's contribution to a context increment.
///
/// The left neighbour counts once and the upper one twice, wherever the specification writes
/// `condTermFlagA + 2 * condTermFlagB`; where it writes `condTermFlagA + condTermFlagB` the caller
/// sums them itself instead.
fn weigh(terms: [usize; 2]) -> usize {
	terms[0] + 2 * terms[1]
}

/// Reads `mb_type` in an intra slice (§9.3.2.5, Table 9-36, §9.3.3.1.1.3).
fn read_mb_type(f: &Frame, run: &SliceRun, e: &mut Entropy, mb: usize) -> Outcome<u32> {
	let base = cabac::offset::MB_TYPE;
	// A neighbour coded as sixteen four-by-four or four eight-by-eight blocks contributes nothing,
	// and any other available neighbour contributes one.
	let mut inc = 0usize;
	for n in ab(f, run, mb).into_iter().flatten() {
		if !matches!(f.kind[n], Kind::I4x4 | Kind::I8x8) {
			inc += 1;
		}
	}
	if res!(e.bin(base + inc)) == 0 {
		return Ok(0);
	}
	// The second bin is the one that names a raw-sample macroblock, and it is decoded by the
	// terminating process rather than against a context of its own.
	if e.c.terminate() == 1 {
		return Ok(25);
	}
	// Whether all sixteen luma blocks are coded or none of them are.
	let luma = res!(e.bin(base + 3));
	// Whether the colour difference pattern is anything but nought.
	let chroma_any = res!(e.bin(base + 4));
	let first = res!(e.bin(base + if chroma_any != 0 { 5 } else { 6 }));
	let second = res!(e.bin(base + if chroma_any != 0 { 6 } else { 7 }));
	let (chroma, pred) = if chroma_any == 0 {
		(0u32, first * 2 + second)
	} else {
		let third = res!(e.bin(base + 7));
		(first + 1, second * 2 + third)
	};
	// Table 7-11 counts the twenty-four Intra_16x16 types off as the prediction mode, then the
	// chroma pattern, then the luma one.
	Ok(1 + pred + 4 * chroma + 12 * luma)
}

/// Reads `transform_size_8x8_flag` (§9.3.3.1.1.10).
fn read_transform_8x8(f: &Frame, run: &SliceRun, e: &mut Entropy, mb: usize) -> Outcome<bool> {
	let mut inc = 0usize;
	for n in ab(f, run, mb).into_iter().flatten() {
		if f.big[n] {
			inc += 1;
		}
	}
	Ok(res!(e.bin(cabac::offset::TRANSFORM_8X8 + inc)) == 1)
}

/// Reads one block's intra prediction mode, given the mode predicted for it (§9.3.2.4).
fn read_mode_cabac(e: &mut Entropy, predicted: u8) -> Outcome<u8> {
	if res!(e.bin(cabac::offset::PREV_PRED)) == 1 {
		return Ok(predicted);
	}
	// Three bins at one context, least significant first.
	let mut rem = 0u8;
	for i in 0..3 {
		rem |= (res!(e.bin(cabac::offset::REM_PRED)) as u8) << i;
	}
	Ok(if rem < predicted { rem } else { rem + 1 })
}

/// Reads `intra_chroma_pred_mode` (§9.3.3.1.1.8).
fn read_chroma_mode(f: &Frame, run: &SliceRun, e: &mut Entropy, mb: usize) -> Outcome<u32> {
	let base = cabac::offset::CHROMA_PRED;
	let mut inc = 0usize;
	for n in ab(f, run, mb).into_iter().flatten() {
		// A raw-sample neighbour has no mode, and one that predicted along the direct current mode
		// contributes nothing.
		if f.kind[n] != Kind::Pcm && f.chroma_mode[n] != 0 {
			inc += 1;
		}
	}
	if res!(e.bin(base + inc)) == 0 {
		return Ok(0);
	}
	if res!(e.bin(base + 3)) == 0 {
		return Ok(1);
	}
	if res!(e.bin(base + 3)) == 0 {
		return Ok(2);
	}
	Ok(3)
}

/// Reads `coded_block_pattern`, luma part then chroma (§9.3.2.6, §9.3.3.1.1.4).
fn read_cbp(f: &Frame, run: &SliceRun, e: &mut Entropy, mb: usize) -> Outcome<(u8, u8)> {
	let mut luma = 0u8;
	for blk in 0..4usize {
		// `condTermFlagN` is one where the neighbouring eight-by-eight block holds **nothing**, which
		// is the way round that reads oddly and is the specification's.
		let sides = luma8_ab(f, run, mb, blk);
		let mut terms = [0usize; 2];
		for (i, side) in sides.into_iter().enumerate() {
			terms[i] = match side {
				None => 0,
				Some((n, k)) => {
					let empty = if n == mb {
						luma & (1 << k) == 0
					} else if f.kind[n] == Kind::Pcm {
						false
					} else {
						f.cbp_luma[n] & (1 << k) == 0
					};
					usize::from(empty)
				},
			};
		}
		// The four bins are the four bits of the pattern, least significant first.
		if res!(e.bin(cabac::offset::CBP_LUMA + weigh(terms))) == 1 {
			luma |= 1 << blk;
		}
	}
	let mut chroma = 0u8;
	for bin in 0..2usize {
		let mut terms = [0usize; 2];
		for (i, side) in ab(f, run, mb).into_iter().enumerate() {
			terms[i] = match side {
				None => 0,
				Some(n) => {
					let term = if f.kind[n] == Kind::Pcm {
						true
					} else if bin == 0 {
						f.cbp_chroma[n] != 0
					} else {
						f.cbp_chroma[n] == 2
					};
					usize::from(term)
				},
			};
		}
		let inc = weigh(terms) + if bin == 1 { 4 } else { 0 };
		if res!(e.bin(cabac::offset::CBP_CHROMA + inc)) == 0 {
			break;
		}
		chroma = bin as u8 + 1;
	}
	Ok((luma, chroma))
}

/// Reads `mb_qp_delta` (§9.3.2.7, Table 9-3, §9.3.3.1.1.5).
fn read_qp_delta(f: &Frame, e: &mut Entropy) -> Outcome<i32> {
	let base = cabac::offset::MB_QP_DELTA;
	let first = match e.prev {
		None => 0usize,
		Some(p) => {
			if f.kind[p] == Kind::Pcm {
				0
			} else if f.kind[p] != Kind::I16x16 && f.cbp_luma[p] == 0 && f.cbp_chroma[p] == 0 {
				0
			} else {
				usize::from(f.qp_moved[p])
			}
		},
	};
	let mut k = 0u32;
	if res!(e.bin(base + first)) == 1 {
		k = 1;
		if res!(e.bin(base + 2)) == 1 {
			k = 2;
			while res!(e.bin(base + 3)) == 1 {
				k += 1;
				// The delta runs from −26 to 25 at eight bits, so its mapped value runs to 51. A
				// longer run is a decoder that has lost the syntax rather than a legal value.
				if k > 87 {
					return Err(err!(
						"An mb_qp_delta was coded as a unary run of more than 87 bins, which no legal \
						value is.";
					Invalid, Input, Decode));
				}
			}
		}
	}
	// Table 9-3 alternates: nought, then one, then minus one, and so on.
	Ok(if k % 2 == 1 {
		((k + 1) / 2) as i32
	} else {
		-((k / 2) as i32)
	})
}

/// One neighbour's contribution to a `coded_block_flag` context increment (§9.3.3.1.1.9).
///
/// `kind` is how the neighbouring macroblock is predicted, or `None` where there is no such
/// macroblock; `flag` is the neighbouring transform block's own flag, or `None` where that block does
/// not exist. **An absent neighbour counts as coded**, because every macroblock this decoder reads is
/// intra; for an inter one it would count as nought, and reading that way round gives every
/// macroblock along the top and left edges of a picture the wrong context.
fn cbf_term(kind: Option<Kind>, flag: Option<bool>) -> usize {
	match kind {
		None			=> 1,
		// A raw-sample neighbour counts as coded whatever its blocks hold.
		Some(Kind::Pcm)		=> 1,
		Some(_)			=> usize::from(flag.unwrap_or(false)),
	}
}

/// The `coded_block_flag` context increment for a macroblock's block of direct current terms.
fn cbf_inc_luma_dc(f: &Frame, run: &SliceRun, mb: usize) -> usize {
	let mut terms = [0usize; 2];
	for (i, side) in ab(f, run, mb).into_iter().enumerate() {
		terms[i] = match side {
			None => cbf_term(None, None),
			// Only a macroblock predicted whole has a block of direct current terms at all.
			Some(n) => match f.kind[n] {
				Kind::I16x16	=> cbf_term(Some(Kind::I16x16), Some(f.cbf[n].luma_dc)),
				other		=> cbf_term(Some(other), None),
			},
		};
	}
	weigh(terms)
}

/// The same for one of a macroblock's four-by-four luma blocks.
fn cbf_inc_luma4(f: &Frame, run: &SliceRun, mb: usize, blk: usize, here: &Partial) -> usize {
	let mut terms = [0usize; 2];
	for (i, side) in luma4_ab(f, run, mb, blk).into_iter().enumerate() {
		terms[i] = match side {
			None => cbf_term(None, None),
			Some((n, k)) => {
				let (kind, cbp, cbf) = if n == mb {
					(here.kind, here.cbp_luma, &here.cbf)
				} else {
					(f.kind[n], f.cbp_luma[n], &f.cbf[n])
				};
				// The block exists only where the pattern says its quadrant carries anything.
				let flag = if cbp & (1 << (k >> 2)) == 0 {
					None
				} else if kind == Kind::I8x8 {
					// A neighbour that used the eight-by-eight transform offers that block instead,
					// and in 4:2:0 its flag is not coded at all but inferred to be one.
					Some(cbf.luma8[k >> 2])
				} else {
					Some(cbf.luma4[k])
				};
				cbf_term(Some(kind), flag)
			},
		};
	}
	weigh(terms)
}

/// The same for one colour difference component's block of direct current terms.
fn cbf_inc_chroma_dc(f: &Frame, run: &SliceRun, mb: usize, c: usize) -> usize {
	let mut terms = [0usize; 2];
	for (i, side) in ab(f, run, mb).into_iter().enumerate() {
		terms[i] = match side {
			None => cbf_term(None, None),
			Some(n) => {
				let flag = if f.cbp_chroma[n] == 0 {
					None
				} else {
					Some(f.cbf[n].chroma_dc[c])
				};
				cbf_term(Some(f.kind[n]), flag)
			},
		};
	}
	weigh(terms)
}

/// The same for one of its alternating current blocks.
fn cbf_inc_chroma_ac(f: &Frame, run: &SliceRun, mb: usize, c: usize, blk: usize, here: &Partial)
	-> usize
{
	let mut terms = [0usize; 2];
	for (i, side) in chroma4_ab(f, run, mb, blk).into_iter().enumerate() {
		terms[i] = match side {
			None => cbf_term(None, None),
			Some((n, k)) => {
				let (kind, cbp, cbf) = if n == mb {
					(here.kind, here.cbp_chroma, &here.cbf)
				} else {
					(f.kind[n], f.cbp_chroma[n], &f.cbf[n])
				};
				// An alternating current block exists only where the whole chroma pattern is coded.
				let flag = if cbp == 2 { Some(cbf.chroma_ac[c][k]) } else { None };
				cbf_term(Some(kind), flag)
			},
		};
	}
	weigh(terms)
}

/// Walks one arithmetically coded slice's macroblocks (§7.3.4).
///
/// Where a slice coded with the length tables ends at its payload, this one ends where the coder says
/// it does: an `end_of_slice_flag` after every macroblock, decoded by the terminating process. There
/// is no `more_rbsp_data` test to fall back on, because the arithmetic decoder reads a little past
/// the last byte the encoder wrote.
fn slice_data_cabac(f: &mut Frame, run: &mut SliceRun, u: &Unit, first_mb: usize, at: usize)
	-> Outcome<()>
{
	let mut e = res!(Entropy::new(&u.body, at, run.qp));
	let mut mb = first_mb;
	let total = f.mbs_w * f.mbs_h;
	loop {
		if mb >= total {
			return Err(err!(
				"A slice ran past macroblock {} of a picture that holds {}.", mb, total;
			Invalid, Input, Decode));
		}
		res!(macroblock_cabac(f, run, &mut e, mb));
		e.prev = Some(mb);
		mb += 1;
		if e.c.terminate() == 1 {
			break;
		}
	}
	Ok(())
}

/// Reads and reconstructs one macroblock of an arithmetically coded slice (§7.3.5).
fn macroblock_cabac(f: &mut Frame, run: &mut SliceRun, e: &mut Entropy, mb: usize) -> Outcome<()> {
	let mb_type = res!(read_mb_type(f, run, e, mb));
	if mb_type == 25 {
		return pcm_cabac(f, run, e, mb);
	}
	let (kind, mut cbp_luma, mut cbp_chroma, pred16) = if mb_type == 0 {
		(Kind::I4x4, 0u8, 0u8, Mode16::Dc)
	} else {
		let k = (mb_type - 1) as usize;
		let pred = res!(Mode16::of((k % 4) as u32));
		let chroma = ((k / 4) % 3) as u8;
		let luma = if k >= 12 { 15u8 } else { 0 };
		(Kind::I16x16, luma, chroma, pred)
	};
	let mut kind = kind;
	if kind == Kind::I4x4 && run.transform_8x8 && res!(read_transform_8x8(f, run, e, mb)) {
		kind = Kind::I8x8;
	}
	let mut modes = [2u8; 16];
	if kind == Kind::I4x4 {
		for i in 0..16 {
			let predicted = res!(predicted_mode(f, run, mb, i, &modes, kind));
			modes[i] = res!(read_mode_cabac(e, predicted));
		}
	} else if kind == Kind::I8x8 {
		for i in 0..4 {
			let predicted = res!(predicted_mode(f, run, mb, i * 4, &modes, kind));
			let m = res!(read_mode_cabac(e, predicted));
			for k in 0..4 {
				modes[i * 4 + k] = m;
			}
		}
	}
	let chroma_code = res!(read_chroma_mode(f, run, e, mb));
	let chroma_mode = res!(ModeC::of(chroma_code));
	if kind != Kind::I16x16 {
		let (luma, chroma) = res!(read_cbp(f, run, e, mb));
		cbp_luma = luma;
		cbp_chroma = chroma;
	}
	let mut qp = run.qp;
	let mut moved = false;
	if cbp_luma > 0 || cbp_chroma > 0 || kind == Kind::I16x16 {
		let delta = res!(read_qp_delta(f, e));
		if !(-26..=25).contains(&delta) {
			return Err(err!(
				"An mb_qp_delta of {} was coded, and it runs from -26 to 25.", delta;
			Invalid, Input, Decode));
		}
		moved = delta != 0;
		qp = (run.qp + delta + 52).rem_euclid(52);
		run.qp = qp;
	}
	// The residual, block by block, with each block's flag kept for the next block's context.
	let mut luma_dc = [0i32; 16];
	let mut luma = [[0i32; 16]; 16];
	let mut luma8 = [[0i32; 64]; 4];
	let mut chroma_dc = [[0i32; 4]; 2];
	let mut chroma = [[[0i32; 16]; 4]; 2];
	let mut here = Partial {
		kind,
		cbp_luma,
		cbp_chroma,
		cbf: Cbf::default(),
	};

	if kind == Kind::I16x16 {
		let inc = cbf_inc_luma_dc(f, run, mb) as u32;
		let mut out = [0i32; 16];
		here.cbf.luma_dc = res!(cabac::residual(&mut e.c, &mut e.x, Cat::LumaDc, Some(inc), &mut out));
		for (i, at) in ZIGZAG_4X4.iter().enumerate() {
			luma_dc[*at] = out[i];
		}
	}
	for i8 in 0..4usize {
		if cbp_luma & (1 << i8) == 0 {
			continue;
		}
		if kind == Kind::I8x8 {
			// A block of sixty-four coefficients, read whole. CAVLC has no table for that and reads
			// four interleaved blocks of sixteen instead; the arithmetic coder has no such limit,
			// and it carries no coded_block_flag for the block either (§7.3.5.3.3).
			res!(cabac::residual(&mut e.c, &mut e.x, Cat::Luma8x8, None, &mut luma8[i8]));
			here.cbf.luma8[i8] = true;
			continue;
		}
		let (cat, start) = if kind == Kind::I16x16 {
			(Cat::LumaAc, 1usize)
		} else {
			(Cat::Luma4x4, 0)
		};
		for i4 in 0..4usize {
			let blk = i8 * 4 + i4;
			let inc = cbf_inc_luma4(f, run, mb, blk, &here) as u32;
			let mut out = [0i32; 16];
			let held = &mut out[..cat.coeffs()];
			here.cbf.luma4[blk] =
				res!(cabac::residual(&mut e.c, &mut e.x, cat, Some(inc), held));
			for i in 0..cat.coeffs() {
				luma[blk][ZIGZAG_4X4[start + i]] = out[i];
			}
		}
	}
	if cbp_chroma & 3 != 0 {
		for c in 0..2usize {
			let inc = cbf_inc_chroma_dc(f, run, mb, c) as u32;
			here.cbf.chroma_dc[c] = res!(cabac::residual(
				&mut e.c, &mut e.x, Cat::ChromaDc, Some(inc), &mut chroma_dc[c]));
		}
	}
	if cbp_chroma & 2 != 0 {
		for c in 0..2usize {
			for i in 0..4usize {
				let inc = cbf_inc_chroma_ac(f, run, mb, c, i, &here) as u32;
				let mut out = [0i32; 15];
				here.cbf.chroma_ac[c][i] = res!(cabac::residual(
					&mut e.c, &mut e.x, Cat::ChromaAc, Some(inc), &mut out));
				for (k, v) in out.iter().enumerate() {
					chroma[c][i][ZIGZAG_4X4[1 + k]] = *v;
				}
			}
		}
	}

	f.slice_of[mb] = Some(run.index);
	f.kind[mb] = kind;
	f.qp[mb] = qp;
	f.modes[mb] = modes;
	f.big[mb] = kind == Kind::I8x8;
	f.cbp_luma[mb] = cbp_luma;
	f.cbp_chroma[mb] = cbp_chroma;
	f.chroma_mode[mb] = chroma_code as u8;
	f.qp_moved[mb] = moved;
	f.cbf[mb] = here.cbf;

	res!(reconstruct(f, run, mb, kind, qp, pred16, chroma_mode, &modes, &luma_dc, &luma, &luma8,
		&chroma_dc, &chroma));
	Ok(())
}

/// Reads a raw-sample macroblock out of an arithmetically coded slice (§7.3.5, §9.3.1.2).
///
/// The samples are not entropy coded at all. They begin at the next byte boundary after the
/// terminating bin that named the macroblock, and the arithmetic decoder is **started afresh** on the
/// byte after them rather than carried across, which is what makes the bitstream position matter
/// here: get it wrong and everything after this macroblock in the slice is noise.
fn pcm_cabac(f: &mut Frame, run: &mut SliceRun, e: &mut Entropy, mb: usize) -> Outcome<()> {
	let at = e.byte();
	// Two hundred and fifty-six luma samples, then two of sixty-four for a 4:2:0 macroblock.
	let need = 256 + 128;
	let end = match at.checked_add(need) {
		Some(end) if end <= e.body.len() => end,
		_ => return Err(err!(
			"A raw-sample macroblock needs {} bytes from byte {} of a payload of {}.",
			need, at, e.body.len(); Invalid, Input, Decode)),
	};
	let raw = &e.body[at..end];
	let (mx, my) = ((mb % f.mbs_w) * 16, (mb / f.mbs_w) * 16);
	for y in 0..16 {
		for x in 0..16 {
			f.pic.y.put(mx + x, my + y, raw[y * 16 + x]);
		}
	}
	let (cx, cy) = ((mb % f.mbs_w) * 8, (mb / f.mbs_w) * 8);
	for c in 0..2usize {
		for y in 0..8 {
			for x in 0..8 {
				let v = raw[256 + c * 64 + y * 8 + x];
				if c == 0 {
					f.pic.cb.put(cx + x, cy + y, v);
				} else {
					f.pic.cr.put(cx + x, cy + y, v);
				}
			}
		}
	}
	res!(e.restart(end));
	f.slice_of[mb] = Some(run.index);
	f.kind[mb] = Kind::Pcm;
	f.qp[mb] = 0;
	f.modes[mb] = [2u8; 16];
	f.counts[mb] = [16u8; 24];
	f.big[mb] = false;
	// A raw-sample macroblock is named in every neighbour rule of clause 9.3.3.1.1 in its own right,
	// so the patterns and flags recorded here are never read; they are left at nought rather than
	// invented.
	Ok(())
}

/// The mode a four-by-four or eight-by-eight block is predicted to take (§8.3.1.1).
///
/// The smaller of the modes its left and upper neighbours used -- but **only where both of the
/// macroblocks holding them are available**. If either is missing, the specification sets
/// `dcPredModePredictedFlag` and *both* modes become the direct current one, not just the missing
/// side's. Taking the minimum of the one available neighbour and a notional 2 instead gives a
/// different answer whenever that neighbour's mode is below 2, which is every vertical and every
/// horizontal block along the top and left edges of a picture.
fn predicted_mode(f: &Frame, run: &SliceRun, mb: usize, blk: usize, here: &[u8; 16],
	here_kind: Kind) -> Outcome<u8>
{
	let (bx, by) = blk_xy(blk);
	let around = f.around(mb, run.index);
	// Which macroblock holds each neighbour, and which of its blocks.
	let left = if bx > 0 {
		Some((mb, blk_index(bx - 1, by)))
	} else {
		around[0].map(|a| (a, blk_index(3, by)))
	};
	let above = if by > 0 {
		Some((mb, blk_index(bx, by - 1)))
	} else {
		around[1].map(|a| (a, blk_index(bx, 3)))
	};
	let (left, above) = match (left, above) {
		(Some(l), Some(a)) => (l, a),
		// Either one missing, and the prediction is the direct current mode.
		_ => return Ok(2),
	};
	let of = |(a, i): (usize, usize)| -> u8 {
		// A neighbour inside this macroblock has been read but not yet recorded against the
		// picture, so it is taken from the array being built. Reading it from the picture instead
		// gives every one of them the direct current mode, which decodes the first few blocks of a
		// macroblock correctly and the rest wrongly.
		let kind = if a == mb { here_kind } else { f.kind[a] };
		match kind {
			Kind::I4x4 | Kind::I8x8	=> if a == mb { here[i] } else { f.modes[a][i] },
			// A neighbour predicted whole, or carried raw, offers no direction.
			_			=> 2,
		}
	};
	Ok(of(left).min(of(above)))
}

/// The four-by-four block at a position within a macroblock, in the order the blocks are walked.
fn blk_index(bx: usize, by: usize) -> usize {
	let quad = (by / 2) * 2 + (bx / 2);
	let within = (by % 2) * 2 + (bx % 2);
	quad * 4 + within
}

/// Reads one block's prediction mode, given the mode predicted for it (§7.3.5.1).
fn read_mode(b: &mut Bits, predicted: u8) -> Outcome<u8> {
	if res!(b.flag()) {
		return Ok(predicted);
	}
	let rem = res!(b.u(3)) as u8;
	Ok(if rem < predicted { rem } else { rem + 1 })
}

/// The `nC` a luma block's `coeff_token` is read with (§9.2.1).
fn luma_nc(f: &Frame, run: &SliceRun, mb: usize, blk: usize, here: &[u8; 24]) -> Outcome<i32> {
	let (bx, by) = blk_xy(blk);
	let left = if bx > 0 {
		Some(here[blk_index(bx - 1, by)] as usize)
	} else {
		f.around(mb, run.index)[0].map(|a| f.counts[a][blk_index(3, by)] as usize)
	};
	let above = if by > 0 {
		Some(here[blk_index(bx, by - 1)] as usize)
	} else {
		f.around(mb, run.index)[1].map(|a| f.counts[a][blk_index(bx, 3)] as usize)
	};
	Ok(cavlc::nc(left, above))
}

/// The same for a chroma block, whose four blocks sit in plain raster order (§6.4.7).
fn chroma_nc(f: &Frame, run: &SliceRun, mb: usize, c: usize, blk: usize, here: &[u8; 24])
	-> Outcome<i32>
{
	let (bx, by) = (blk % 2, blk / 2);
	let base = 16 + c * 4;
	let left = if bx > 0 {
		Some(here[base + by * 2] as usize)
	} else {
		f.around(mb, run.index)[0].map(|a| f.counts[a][base + by * 2 + 1] as usize)
	};
	let above = if by > 0 {
		Some(here[base + bx] as usize)
	} else {
		f.around(mb, run.index)[1].map(|a| f.counts[a][base + 2 + bx] as usize)
	};
	Ok(cavlc::nc(left, above))
}

/// Builds the edges around a block of the luma plane.
///
/// `n` is how many samples of the row above are wanted -- four for a four-by-four block, eight for
/// an eight-by-eight one and sixteen for a whole macroblock -- and `right` how many more above and
/// to the right. Availability is asked of the four-by-four block grid, which is where the answer
/// actually lives: a block inside this macroblock is available once it has been reconstructed, and
/// one outside it is available once its macroblock has been *and* that macroblock is in this slice.
fn luma_edges(f: &Frame, run: &SliceRun, mb: usize, x: usize, y: usize, n: usize, right: usize,
	done: &[bool; 16]) -> Edges
{
	let (mx, my) = ((mb % f.mbs_w) * 16, (mb / f.mbs_w) * 16);
	let mut e = Edges::none();
	let ok = |px: i64, py: i64| -> bool {
		if px < 0 || py < 0 {
			return false;
		}
		let (px, py) = (px as usize, py as usize);
		let nb = (px / 16) + (py / 16) * f.mbs_w;
		if nb == mb {
			// Inside this macroblock: available once the block holding it has been written.
			done[blk_index((px % 16) / 4, (py % 16) / 4)]
		} else {
			px < f.pic.y.w && py < f.pic.y.h && f.available(nb as i64, run.index)
		}
	};
	let ax = (mx + x) as i64;
	let ay = (my + y) as i64;
	e.top_ok = ok(ax, ay - 1);
	if e.top_ok {
		for i in 0..n {
			e.top[i] = f.pic.y.at(mx + x + i, (my + y).wrapping_sub(1)).unwrap_or(0) as i32;
		}
	}
	if right > 0 {
		e.right_ok = ok(ax + n as i64, ay - 1);
		if e.right_ok {
			for i in 0..right {
				e.top[n + i] = f.pic.y.at(mx + x + n + i, (my + y).wrapping_sub(1)).unwrap_or(0)
					as i32;
			}
		}
	}
	e.left_ok = ok(ax - 1, ay);
	if e.left_ok {
		for i in 0..n {
			e.left[i] = f.pic.y.at((mx + x).wrapping_sub(1), my + y + i).unwrap_or(0) as i32;
		}
	}
	e.corner_ok = ok(ax - 1, ay - 1);
	if e.corner_ok {
		e.corner = f.pic.y.at((mx + x).wrapping_sub(1), (my + y).wrapping_sub(1)).unwrap_or(0)
			as i32;
	}
	if right > 0 && !e.right_ok {
		e.pad_right(n, n + right);
	}
	e
}

/// Builds the edges around a whole chroma block, which is a macroblock's worth.
fn chroma_edges(f: &Frame, run: &SliceRun, mb: usize, c: usize) -> Edges {
	let (cx, cy) = ((mb % f.mbs_w) * 8, (mb / f.mbs_w) * 8);
	let plane = if c == 0 { &f.pic.cb } else { &f.pic.cr };
	let n = f.around(mb, run.index);
	let mut e = Edges::none();
	e.top_ok = n[1].is_some();
	if e.top_ok {
		for i in 0..8 {
			e.top[i] = plane.at(cx + i, cy.wrapping_sub(1)).unwrap_or(0) as i32;
		}
	}
	e.left_ok = n[0].is_some();
	if e.left_ok {
		for i in 0..8 {
			e.left[i] = plane.at(cx.wrapping_sub(1), cy + i).unwrap_or(0) as i32;
		}
	}
	e.corner_ok = n[3].is_some();
	if e.corner_ok {
		e.corner = plane.at(cx.wrapping_sub(1), cy.wrapping_sub(1)).unwrap_or(0) as i32;
	}
	e
}

/// Predicts, transforms and writes one macroblock's samples.
#[allow(clippy::too_many_arguments)]
fn reconstruct(f: &mut Frame, run: &SliceRun, mb: usize, kind: Kind, qp: i32, pred16: Mode16,
	chroma_mode: ModeC, modes: &[u8; 16], luma_dc: &[i32; 16], luma: &[[i32; 16]; 16],
	luma8: &[[i32; 64]; 4], chroma_dc: &[[i32; 4]; 2], chroma: &[[[i32; 16]; 4]; 2]) -> Outcome<()>
{
	let (mx, my) = ((mb % f.mbs_w) * 16, (mb / f.mbs_w) * 16);
	let depth = 8u32;
	let mut done = [false; 16];
	match kind {
		Kind::I16x16 => {
			let e = luma_edges(f, run, mb, 0, 0, 16, 0, &done);
			let pred = intra::pred_16x16(pred16, &e, depth);
			let dc = transform::luma_dc(luma_dc, &f.w_luma, qp);
			for blk in 0..16 {
				let (bx, by) = blk_xy(blk);
				let mut c = luma[blk];
				c[0] = dc[by * 4 + bx];
				let d = transform::scale_4x4(&c, &f.w_luma, qp, true);
				let r = transform::inverse_4x4(&d);
				for yy in 0..4 {
					for xx in 0..4 {
						let px = bx * 4 + xx;
						let py = by * 4 + yy;
						let v = pred[py * 16 + px] + r[yy * 4 + xx];
						f.pic.y.put(mx + px, my + py, v.clamp(0, 255) as u8);
					}
				}
				done[blk] = true;
			}
		},
		Kind::I4x4 => {
			for blk in 0..16 {
				let (bx, by) = blk_xy(blk);
				let (x, y) = (bx * 4, by * 4);
				let e = luma_edges(f, run, mb, x, y, 4, 4, &done);
				let mode = res!(Mode::of(modes[blk] as u32));
				let pred = intra::pred_4x4(mode, &e, depth);
				let d = transform::scale_4x4(&luma[blk], &f.w_luma, qp, false);
				let r = transform::inverse_4x4(&d);
				for yy in 0..4 {
					for xx in 0..4 {
						let v = pred[yy * 4 + xx] + r[yy * 4 + xx];
						f.pic.y.put(mx + x + xx, my + y + yy, v.clamp(0, 255) as u8);
					}
				}
				done[blk] = true;
			}
		},
		Kind::I8x8 => {
			for i8 in 0..4usize {
				let (x, y) = ((i8 % 2) * 8, (i8 / 2) * 8);
				let e = luma_edges(f, run, mb, x, y, 8, 8, &done);
				let mode = res!(Mode::of(modes[i8 * 4] as u32));
				let pred = intra::pred_8x8(mode, &e, depth);
				let mut c = [0i32; 64];
				for (i, at) in ZIGZAG_8X8.iter().enumerate() {
					c[*at] = luma8[i8][i];
				}
				let d = transform::scale_8x8(&c, &f.w_luma, qp);
				let r = transform::inverse_8x8(&d);
				for yy in 0..8 {
					for xx in 0..8 {
						let v = pred[yy * 8 + xx] + r[yy * 8 + xx];
						f.pic.y.put(mx + x + xx, my + y + yy, v.clamp(0, 255) as u8);
					}
				}
				for k in 0..4 {
					done[i8 * 4 + k] = true;
				}
			}
		},
		Kind::Pcm | Kind::Absent => {},
	}
	// Chroma, both components the same way.
	let (cx, cy) = ((mb % f.mbs_w) * 8, (mb / f.mbs_w) * 8);
	for c in 0..2usize {
		let e = chroma_edges(f, run, mb, c);
		let pred = intra::pred_chroma(chroma_mode, &e, depth);
		let offset = if c == 0 { f.pps.cb_qp_offset } else { f.pps.cr_qp_offset };
		let cqp = transform::chroma_qp(qp, offset);
		let w = if c == 0 { &f.w_cb } else { &f.w_cr };
		let dc = transform::chroma_dc(&chroma_dc[c], w, cqp);
		for blk in 0..4usize {
			let (bx, by) = (blk % 2, blk / 2);
			let mut coeffs = chroma[c][blk];
			coeffs[0] = dc[by * 2 + bx];
			let d = transform::scale_4x4(&coeffs, w, cqp, true);
			let r = transform::inverse_4x4(&d);
			for yy in 0..4 {
				for xx in 0..4 {
					let px = bx * 4 + xx;
					let py = by * 4 + yy;
					let v = pred[py * 8 + px] + r[yy * 4 + xx];
					let s = v.clamp(0, 255) as u8;
					if c == 0 {
						f.pic.cb.put(cx + px, cy + py, s);
					} else {
						f.pic.cr.put(cx + px, cy + py, s);
					}
				}
			}
		}
	}
	Ok(())
}

/// What one slice asks of the deblocking filter (§7.4.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Filter {
	/// 0 to filter everything, 1 to filter nothing, 2 to filter everything but the edges between
	/// this slice and another.
	pub idc:	u32,
	/// The offset added to the filter's first threshold.
	pub alpha:	i32,
	/// The offset added to its second.
	pub beta:	i32,
}

/// A borrow of the frame's fields the deblocking filter needs.
pub struct View<'a> {
	/// The picture being filtered.
	pub pic:	&'a mut Picture,
	/// The width in macroblocks.
	pub mbs_w:	usize,
	/// The height in macroblocks.
	pub mbs_h:	usize,
	/// Each macroblock's quantisation parameter.
	pub qp:		&'a [i32],
	/// Which slice each macroblock belongs to.
	pub slice_of:	&'a [Option<usize>],
	/// Whether each macroblock is coded with the eight-by-eight transform.
	pub big:	&'a [bool],
	/// What each slice asks of the deblocking filter, in the order the slices were decoded.
	pub filters:	&'a [Filter],
	/// The offset applied to the Cb quantisation parameter.
	pub cb_qp_offset:	i32,
	/// The same for Cr.
	pub cr_qp_offset:	i32,
}

/// Hands the deblocking filter what it needs out of a frame.
fn frame_view<'b>(f: &'b mut Frame) -> View<'b> {
	View {
		mbs_w:		f.mbs_w,
		mbs_h:		f.mbs_h,
		qp:		&f.qp,
		slice_of:	&f.slice_of,
		big:		&f.big,
		filters:	&[],
		cb_qp_offset:	f.pps.cb_qp_offset,
		cr_qp_offset:	f.pps.cr_qp_offset,
		pic:		&mut f.pic,
	}
}

/// Cuts the picture down to the size the sequence parameter set says it is meant to be shown at.
///
/// A picture is coded in whole macroblocks, so a 1080-line film is coded as 1088 lines and the
/// last eight are not part of it. 669 films in the corpus are exactly that shape.
fn crop(f: &Frame) -> Picture {
	let (w, h) = (f.sps.width as usize, f.sps.height as usize);
	Picture {
		y:	f.pic.y.cropped(w, h),
		cb:	f.pic.cb.cropped(w / 2, h / 2),
		cr:	f.pic.cr.cropped(w / 2, h / 2),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_the_blocks_are_walked_a_quadrant_at_a_time_01() -> Outcome<()> {
		// Not raster order. The sixteen four-by-four blocks of a macroblock are walked as four
		// quadrants of four, and each quadrant as four blocks -- so block 1 sits to the right of
		// block 0 and block 4 sits eight samples to its right, not four. A decoder that walked them
		// in raster order would predict half of them from neighbours it has not decoded.
		let want = [
			(0, 0), (1, 0), (0, 1), (1, 1),
			(2, 0), (3, 0), (2, 1), (3, 1),
			(0, 2), (1, 2), (0, 3), (1, 3),
			(2, 2), (3, 2), (2, 3), (3, 3),
		];
		for (i, xy) in want.iter().enumerate() {
			req!(blk_xy(i), *xy, "block {} sits somewhere else", i);
			// And the inverse agrees, which is what the neighbour lookups rely on.
			req!(blk_index(xy.0, xy.1), i);
		}
		Ok(())
	}

	#[test]
	fn test_the_pattern_table_is_a_permutation_02() -> Outcome<()> {
		// Table 9-4's intra column maps 48 code numbers onto the 48 patterns a macroblock with
		// colour may have, one for one. A transcription that repeated a value would silently
		// decode two different pictures the same way, and one that dropped a value would make a
		// legal picture undecodable, so the check is that it is a permutation of 0 to 47.
		let mut seen = [false; 48];
		for v in CBP_INTRA {
			let v = v as usize;
			let already = seen.get(v).copied().unwrap_or(true);
			req!(already, false, "the pattern {} appears twice in the table", v);
			seen[v] = true;
		}
		req!(seen.iter().all(|s| *s), true, "the table does not cover every pattern");
		Ok(())
	}

	#[test]
	fn test_the_quantiser_wraps_rather_than_clipping_03() -> Outcome<()> {
		// A macroblock's quantisation parameter is the previous one plus a delta, modulo 52. It
		// wraps so that any value is reachable from any other in one step, and a decoder that
		// clipped instead would quantise a macroblock at 51 where the stream asked for 0 -- a
		// block of flat grey in the middle of a detailed picture.
		let step = |prev: i32, delta: i32| (prev + delta + 52).rem_euclid(52);
		req!(step(30, 5), 35);
		req!(step(2, -5), 49, "a delta below nought clipped instead of wrapping");
		req!(step(50, 5), 3, "a delta past 51 clipped instead of wrapping");
		req!(step(0, 0), 0);
		Ok(())
	}
}
