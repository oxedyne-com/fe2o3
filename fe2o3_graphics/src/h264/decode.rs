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
	/// Which four-by-four luma blocks hold a coefficient, for the deblocking filter.
	coded:	Vec<u16>,
	/// Whether each macroblock is coded with the eight-by-eight transform, likewise.
	big:	Vec<bool>,
	/// The intra prediction mode of each four-by-four luma block, in the macroblock's own order.
	modes:	Vec<[u8; 16]>,
	/// How many coefficients each four-by-four block holds, for CAVLC's neighbour counts: sixteen
	/// luma then four of each colour difference.
	counts:	Vec<[u8; 24]>,
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
			coded:		vec![0; n],
			big:		vec![false; n],
			modes:		vec![[2u8; 16]; n],
			counts:		vec![[0u8; 24]; n],
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
	let mut deblocking = 0u32;
	let mut alpha = 0i32;
	let mut beta = 0i32;

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
		if index == 0 {
			deblocking = head.deblocking;
			alpha = head.alpha_offset;
			beta = head.beta_offset;
		}
		if pps.cabac {
			return Err(err!(
				"The slice is coded with the arithmetic entropy coder (CABAC), and this decoder \
				reads the variable-length one (CAVLC). 947 films in the corpus this was written \
				against are CABAC and 711 are CAVLC.";
			Invalid, Input, Unimplemented));
		}
		res!(slice_data(&mut frame, &mut run, u, head.first_mb as usize, head.data_bit));
	}
	if deblock && deblocking != 1 {
		let mut view = frame_view(&mut frame);
		view.alpha = alpha;
		view.beta = beta;
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
	let mut coded = 0u16;
	for i in 0..16 {
		let any = if kind == Kind::I16x16 {
			luma_dc.iter().any(|v| *v != 0) || luma[i].iter().any(|v| *v != 0)
		} else if kind == Kind::I8x8 {
			luma8[i / 4].iter().any(|v| *v != 0)
		} else {
			luma[i].iter().any(|v| *v != 0)
		};
		if any {
			coded |= 1 << i;
		}
	}
	f.coded[mb] = coded;
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
	f.coded[mb] = 0xffff;
	f.modes[mb] = [2u8; 16];
	// A raw macroblock counts as sixteen coefficients everywhere, for its neighbours' tables.
	f.counts[mb] = [16u8; 24];
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
	/// Which four-by-four luma blocks hold a coefficient.
	pub coded:	&'a [u16],
	/// Which slice each macroblock belongs to.
	pub slice_of:	&'a [Option<usize>],
	/// Whether each macroblock is coded with the eight-by-eight transform.
	pub big:	&'a [bool],
	/// The offset added to the filter's first threshold.
	pub alpha:	i32,
	/// The offset added to its second.
	pub beta:	i32,
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
		coded:		&f.coded,
		slice_of:	&f.slice_of,
		big:		&f.big,
		alpha:		0,
		beta:		0,
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
