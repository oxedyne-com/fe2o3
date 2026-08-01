//! Walking a coded picture and building the samples back up as it goes.
//!
//! This is where the syntax of clauses 7.3.8.1 to 7.3.8.11 meets the decoding processes of clause 8.
//! The two are interleaved rather than done in turn, and they have to be: every block is predicted
//! from the samples around it, so a block cannot be predicted until the ones before it in coding
//! order have been *reconstructed*, not merely parsed.
//!
//! The shape of the walk, outermost first:
//!
//! - **A row of coding tree blocks at a time**, because every photograph in the corpus is coded in
//!   wavefronts: each row is its own piece of arithmetic-coded data, beginning at a byte offset the
//!   slice header carries, and starting from the context state saved after the second block of the
//!   row above.
//! - **A coding tree block** is a quadtree. Each node either splits into four or becomes a coding
//!   unit, and the depth it stops at is what the picture spends its bits on: flat sky stops early,
//!   an eyelash goes all the way down.
//! - **A coding unit** carries one or four intra prediction modes for luma and one for chroma, and
//!   then a transform tree of its own, which may cut it up again.
//! - **A transform block** is predicted, its residual read, transformed back and added.
//!
//! # What this decodes and what it refuses
//!
//! Intra pictures in 4:2:0 at eight bits, which is every HEIC photograph in the library it was
//! written against. Anything else is refused where it is read rather than decoded into a wrong
//! picture: no scaling lists, no raw sample blocks, no palettes, no cross-component prediction, no
//! residual rotation. A refusal names the tool, so a photograph that needs one says which.
//!
//! # The one thing that has to be got right and cannot be seen
//!
//! Sample availability. A block predicts from its neighbours only where those neighbours have
//! already been decoded, and "already" is in the zig-zag order the quadtree is walked in, not in
//! raster order. A decoder that is careless about it predicts from samples that are still nought
//! and produces a picture with a plausible-looking grid of darker blocks. What is kept here is one
//! flag per four-by-four block, set as that block is written, which is exactly the question being
//! asked and is impossible to get subtly wrong.

use crate::hevc::{
	cabac::{
		Cabac,
		Contexts,
		Rows,
		Set,
	},
	intra,
	scan::{
		Order,
		Scans,
	},
	transform,
	Pps,
	Slice,
	Sps,
};

use oxedyne_fe2o3_core::prelude::*;

/// One component's samples.
#[derive(Clone, Debug)]
pub struct Plane {
	/// Width in samples.
	pub w:	usize,
	/// Height in samples.
	pub h:	usize,
	/// The samples, row by row.
	pub px:	Vec<u16>,
}

impl Plane {

	/// A plane of nothing.
	fn new(w: usize, h: usize) -> Self {
		Self { w, h, px: vec![0; w * h] }
	}

	/// The same, for a caller assembling a grid out of tiles.
	pub fn empty(w: usize, h: usize) -> Self {
		Self::new(w, h)
	}

	/// One sample, or `None` outside the plane.
	pub fn at(&self, x: usize, y: usize) -> Option<u16> {
		if x < self.w && y < self.h {
			self.px.get(y * self.w + x).copied()
		} else {
			None
		}
	}

	/// Writes one sample, ignoring a position outside the plane.
	fn put(&mut self, x: usize, y: usize, v: u16) {
		if x < self.w && y < self.h {
			self.px[y * self.w + x] = v;
		}
	}
}

/// A decoded picture, before it is turned into anything anybody can look at.
#[derive(Clone, Debug)]
pub struct Picture {
	/// Brightness.
	pub y:	Plane,
	/// The two colour difference planes, at half the width and half the height.
	pub cb:	Plane,
	/// The other one.
	pub cr:	Plane,
	/// Bits a sample.
	pub depth:	u32,
}

impl Picture {

	/// Copies another picture into this one at a position, for assembling a grid of tiles.
	///
	/// The colour planes are half size both ways, so the position halves with them.
	pub fn paste(&mut self, from: &Self, x: usize, y: usize) {
		for (dst, src, at) in [
			(&mut self.y, &from.y, (x, y)),
			(&mut self.cb, &from.cb, (x / 2, y / 2)),
			(&mut self.cr, &from.cr, (x / 2, y / 2)),
		] {
			for row in 0..src.h {
				let into = at.1 + row;
				if into >= dst.h {
					break;
				}
				let take = src.w.min(dst.w.saturating_sub(at.0));
				let (a, b) = (into * dst.w + at.0, row * src.w);
				dst.px[a..a + take].copy_from_slice(&src.px[b..b + take]);
			}
		}
	}
}

/// What one coding tree block's sample adaptive offset filter was told to do.
///
/// Parsed with the block and applied to the whole picture at the end, because the filter reads
/// samples the block after this one will write.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sao {
	/// Nought for nothing, one for a band offset, two for an edge offset, per component.
	pub kind:	[u8; 3],
	/// The four offsets, already signed.
	pub offset:	[[i32; 4]; 3],
	/// Which band the four offsets start at, where the kind is a band offset.
	pub band:	[u8; 3],
	/// Which direction the edge is looked for in, where the kind is an edge offset.
	pub class:	[u8; 3],
}

/// The arithmetic decoder and the contexts it reads against, which travel together.
struct Ent<'a> {
	/// The arithmetic decoder over this row's piece of the data.
	cabac:	Cabac<'a>,
	/// The context variables, which carry over between rows.
	ctxs:	Contexts,
}

impl<'a> Ent<'a> {

	/// One bin against a context of `set`.
	fn bin(&mut self, set: Set, inc: usize) -> Outcome<u32> {
		let ctx = res!(self.ctxs.at(set, inc));
		Ok(self.cabac.bin(ctx))
	}

	/// A run of ones ended by a nought, against one context each, up to `most` of them.
	fn unary(&mut self, set: Set, incs: &[usize], most: usize) -> Outcome<u32> {
		let mut n = 0u32;
		while (n as usize) < most {
			let inc = incs[(n as usize).min(incs.len() - 1)];
			if res!(self.bin(set, inc)) == 0 {
				break;
			}
			n += 1;
		}
		Ok(n)
	}

	/// The same at even odds, with no context.
	fn unary_bypass(&mut self, most: usize) -> u32 {
		let mut n = 0u32;
		while (n as usize) < most && self.cabac.bypass() == 1 {
			n += 1;
		}
		n
	}
}

/// Everything a picture's decoding needs to know about what has been decoded so far.
struct Frame<'a> {
	sps:	&'a Sps,
	pps:	&'a Pps,
	slice:	&'a Slice,
	/// The samples, filled in as the walk goes.
	pic:	Picture,
	/// The scan orders, worked out once.
	scans:	Scans,
	/// The weights each block is quantised against, or `None` where the sequence quantises flat.
	weights:	Option<crate::hevc::Scaling>,

	/// The width of the four-by-four grid the per-block records are kept on.
	gw:	usize,
	/// Its height.
	gh:	usize,
	/// How deep in the quadtree the coding unit covering each block sits.
	ct_depth:	Vec<u8>,
	/// The luma prediction mode of the block covering each.
	mode:	Vec<u8>,
	/// The luma quantisation parameter of the coding unit covering each.
	qp:	Vec<i8>,

	/// The filter settings of each coding tree block, for the pass at the end.
	sao:	Vec<Sao>,
	/// Coding tree blocks across the picture.
	ctbs_w:	usize,
	/// And down.
	ctbs_h:	usize,

	/// The parameter the next quantisation group predicts from.
	qp_prev:	i32,
	/// What the current group's syntax added to it.
	qp_delta:	i32,
	/// Whether that delta has been read yet in this group.
	qp_coded:	bool,
	/// The parameter in force for the coding unit being decoded.
	qp_now:	i32,

	/// Whether the current coding unit skips the transform and the quantiser.
	bypass:	bool,
	/// Whether it is split into four prediction blocks, which forces the transform tree one deep.
	split_intra:	bool,
	/// Its luma prediction modes, one or four.
	pred_y:	[u8; 4],
	/// Its chroma prediction mode.
	pred_c:	u8,
	/// Whether each depth of the transform tree has a chroma residual, for the two components.
	cbf_cb:	[bool; 6],
	/// The other one.
	cbf_cr:	[bool; 6],
	/// Whether the block being read now was coded without its transform.
	skip_tr:	bool,
	/// Where the coding unit being decoded starts, for the modes its blocks look up.
	cu_x:	usize,
	/// And down.
	cu_y:	usize,
	/// How wide it is.
	cu_size:	usize,
}

/// Decodes one intra picture.
///
/// `data` is the slice segment's entropy-coded bytes **as they arrived**, escaping and all, from
/// the slice header's end onwards. It has to be the escaped form: the entry point offsets that say
/// where each row of blocks begins are counted in escaped bytes (§7.4.7.1), so the cut is made here
/// and each piece unescaped afterwards.
pub fn picture(sps: &Sps, pps: &Pps, slice: &Slice, data: &[u8]) -> Outcome<Picture> {
	res!(refuse_what_is_not_built(sps, pps));
	let (w, h) = (sps.coded_w as usize, sps.coded_h as usize);
	let ctb = sps.ctb_size as usize;
	let ctbs_w = w.div_ceil(ctb);
	let ctbs_h = h.div_ceil(ctb);
	let (gw, gh) = (w.div_ceil(4), h.div_ceil(4));
	let mut frame = Frame {
		sps,
		pps,
		slice,
		pic: Picture {
			y:	Plane::new(w, h),
			cb:	Plane::new(w / 2, h / 2),
			cr:	Plane::new(w / 2, h / 2),
			depth:	sps.luma_bits as u32,
		},
		scans:	Scans::new(),
		weights: sps.weights.clone(),
		gw,
		gh,
		ct_depth: vec![0; gw * gh],
		mode:	vec![intra::DC; gw * gh],
		qp:	vec![slice.qp as i8; gw * gh],
		sao:	vec![Sao::default(); ctbs_w * ctbs_h],
		ctbs_w,
		ctbs_h,
		qp_prev: slice.qp,
		qp_delta: 0,
		qp_coded: false,
		qp_now: slice.qp,
		bypass:	false,
		split_intra: false,
		pred_y:	[intra::DC; 4],
		pred_c:	intra::DC,
		cbf_cb:	[false; 6],
		cbf_cr:	[false; 6],
		skip_tr: false,
		cu_x:	0,
		cu_y:	0,
		cu_size: ctb,
	};

	// One piece of data a row of blocks, at the offsets the slice header named, each unescaped on
	// its own once the cut has been made in the escaped bytes.
	let pieces: Vec<Vec<u8>> = res!(split_rows(data, slice, ctbs_h))
		.into_iter()
		.map(crate::hevc::rbsp)
		.collect();
	let mut rows = Rows::new(slice.qp);
	for (ry, piece) in pieces.iter().enumerate() {
		let mut ent = Ent {
			cabac:	res!(Cabac::new(piece)),
			ctxs:	rows.begin(),
		};
		// Every row of a wavefront-coded picture starts predicting its quantisation parameter
		// afresh, because the row above may not have been decoded yet where an encoder ran them
		// in parallel (§8.6.1).
		frame.qp_prev = slice.qp;
		for rx in 0..ctbs_w {
			res!(frame.ctu(&mut ent, rx, ry));
			if rx == 1 {
				rows.after_second(&ent.ctxs);
			}
			// The bin that says whether the slice ends here. It has to be read whether or not it
			// says so: it moves the arithmetic decoder on.
			let ended = ent.cabac.terminate();
			if ended == 1 && !(ry == ctbs_h - 1 && rx == ctbs_w - 1) {
				// A slice that stops early is not a fault in a still picture -- it is a picture
				// this decoder has misread, and saying so beats returning half of one.
				return Err(err!(
					"The slice ended at block ({}, {}) of {} by {}.",
					rx, ry, ctbs_w, ctbs_h; Invalid, Input, Decode));
			}
		}
		// A row one block wide never reaches the save above, and the row below it still has to
		// start from somewhere.
		if ctbs_w == 1 {
			rows.after_second(&ent.ctxs);
		}
		// The encoder said how long this row's data is; the decoder has just read it. The two
		// agreeing is the cheapest check there is that the row was read in step, and it is
		// checkable a row at a time rather than only at the end of the picture -- which is what
		// makes it worth having: it names the row that went wrong.
		//
		// A little short is normal. The arithmetic decoder reads ahead into a window it may not
		// use, and the final bins are coded against bits the encoder never had to write.
		let used = ent.cabac.consumed();
		let have = piece.len();
		if used > have + 2 || used + 8 < have {
			return Err(err!(
				"Row {} of blocks is {} bytes and reading it took {}. The row was read out of \
				step.", ry, have, used; Invalid, Input, Decode));
		}
	}
	Ok(frame.pic)
}

/// Says plainly what this decoder does not do, rather than doing it wrongly.
fn refuse_what_is_not_built(sps: &Sps, pps: &Pps) -> Outcome<()> {
	if sps.chroma != 1 {
		return Err(err!(
			"This decoder reads 4:2:0 pictures, and this one is chroma format {}.", sps.chroma;
		Unimplemented));
	}
	if sps.luma_bits != 8 || sps.chroma_bits != 8 {
		return Err(err!(
			"This decoder reads eight-bit pictures, and this one is {} and {}.",
			sps.luma_bits, sps.chroma_bits; Unimplemented));
	}
	if sps.pcm {
		return Err(err!(
			"This picture may carry raw sample blocks, which are not read."; Unimplemented));
	}
	if pps.tiles {
		return Err(err!("This picture is cut into tiles, which are not read."; Unimplemented));
	}
	Ok(())
}

/// Cuts the slice data into one piece a row of blocks, at the entry points the header carries.
fn split_rows<'a>(data: &'a [u8], slice: &Slice, rows: usize) -> Outcome<Vec<&'a [u8]>> {
	if slice.entries.is_empty() {
		if rows != 1 {
			return Err(err!(
				"A picture {} rows tall carries no entry points, so only its first row could be \
				found.", rows; Invalid, Input, Decode));
		}
		return Ok(vec![data]);
	}
	if slice.entries.len() + 1 != rows {
		return Err(err!(
			"The slice header names {} pieces and the picture has {} rows of blocks.",
			slice.entries.len() + 1, rows; Invalid, Input, Decode));
	}
	let mut out = Vec::with_capacity(rows);
	let mut at = 0usize;
	for len in &slice.entries {
		let len = *len as usize;
		let end = at + len;
		if end > data.len() {
			return Err(err!(
				"A row of blocks is said to end at byte {} of {}.", end, data.len();
			Invalid, Input, Decode));
		}
		out.push(&data[at..end]);
		at = end;
	}
	out.push(&data[at..]);
	Ok(out)
}

impl<'a> Frame<'a> {

	/// Where a position sits in the order the picture is decoded in (§6.4.1).
	///
	/// Coding tree blocks in raster order, and within one, four-by-four blocks in **z order** --
	/// the quadtree's own order, so that the four children of a node are visited before anything to
	/// their right or below. Interleaving the bits of the two coordinates is exactly that order.
	fn z_order(&self, x: usize, y: usize) -> u64 {
		let ctb = self.sps.ctb_size as usize;
		let block = (y / ctb) * self.ctbs_w + (x / ctb);
		let (bx, by) = ((x % ctb) / 4, (y % ctb) / 4);
		let mut z = 0u64;
		for i in 0..4 {
			z |= (((bx >> i) & 1) as u64) << (2 * i);
			z |= (((by >> i) & 1) as u64) << (2 * i + 1);
		}
		((block as u64) << 8) | z
	}

	/// Whether a neighbouring position may be used by a block at `(cx, cy)`.
	///
	/// Inside the picture, and **earlier in decoding order**. It is decoding order and not "has
	/// been reconstructed": the four prediction blocks of one coding unit have their modes read
	/// before any of them is reconstructed, and each of them draws its candidate modes from the one
	/// before it. A decoder that asks whether the samples exist yet answers "no" there, hands every
	/// such block the flat mode as its candidate, and picks the wrong mode out of the list -- with
	/// no change to how many bins were read, so the picture stays in step and comes out wrong.
	///
	/// One slice and one tile, so nothing else can make a neighbour unavailable.
	fn available(&self, cx: usize, cy: usize, nx: i32, ny: i32) -> bool {
		if nx < 0 || ny < 0 {
			return false;
		}
		let (nx, ny) = (nx as usize, ny as usize);
		if nx >= self.pic.y.w || ny >= self.pic.y.h {
			return false;
		}
		self.z_order(nx, ny) < self.z_order(cx, cy)
	}

	/// Records how deep in the quadtree a coding unit sits, against every block it covers.
	fn record_depth(&mut self, x: usize, y: usize, size: usize, depth: u8) {
		for gy in (y / 4)..((y + size).div_ceil(4)).min(self.gh) {
			for gx in (x / 4)..((x + size).div_ceil(4)).min(self.gw) {
				self.ct_depth[gy * self.gw + gx] = depth;
			}
		}
	}

	/// The same for its quantisation parameter, which is settled later than its depth is.
	///
	/// Kept apart from the depth deliberately: writing the two together meant that a coding unit
	/// carrying a change of quantisation parameter also wrote a depth of nought over its own, and
	/// the next block's split flag was then read against the wrong context.
	fn record_qp(&mut self, x: usize, y: usize, size: usize, qp: i8) {
		for gy in (y / 4)..((y + size).div_ceil(4)).min(self.gh) {
			for gx in (x / 4)..((x + size).div_ceil(4)).min(self.gw) {
				self.qp[gy * self.gw + gx] = qp;
			}
		}
	}

	/// The luma prediction mode recorded against a position.
	///
	/// **Not guarded by availability.** A block's own mode is written down as its syntax is read,
	/// which is before it has been reconstructed -- so asking whether it is *available* and
	/// answering "flat" where it is not predicts every block in the picture with the flat mode, and
	/// produces a picture that is recognisably the right photograph and wrong everywhere. Where
	/// availability does matter is the candidate list, and [`Frame::mode_candidates`] checks it
	/// there.
	fn mode_of(&self, x: usize, y: usize) -> u8 {
		let (gx, gy) = ((x / 4).min(self.gw - 1), (y / 4).min(self.gh - 1));
		self.mode[gy * self.gw + gx]
	}

	/// One coding tree block: its filter settings, then its quadtree.
	fn ctu(&mut self, ent: &mut Ent, rx: usize, ry: usize) -> Outcome<()> {
		let ctb = self.sps.ctb_size as usize;
		let (x, y) = (rx * ctb, ry * ctb);
		if self.slice.sao_luma || self.slice.sao_chroma {
			res!(self.sao_params(ent, rx, ry));
		}
		// A new coding tree block is a new quantisation group unless the group is larger than one.
		self.quadtree(ent, x, y, self.sps.ctb_size.trailing_zeros(), 0)
	}

	/// The sample adaptive offset settings of one block (§7.3.8.3).
	fn sao_params(&mut self, ent: &mut Ent, rx: usize, ry: usize) -> Outcome<()> {
		let at = ry * self.ctbs_w + rx;
		let mut merge_left = false;
		let mut merge_up = false;
		if rx > 0 {
			merge_left = res!(ent.bin(Set::SaoMerge, 0)) == 1;
		}
		if ry > 0 && !merge_left {
			merge_up = res!(ent.bin(Set::SaoMerge, 0)) == 1;
		}
		if merge_left {
			self.sao[at] = self.sao[at - 1];
			return Ok(());
		}
		if merge_up {
			self.sao[at] = self.sao[at - self.ctbs_w];
			return Ok(());
		}
		let mut sao = Sao::default();
		for c in 0..3usize {
			let wanted = if c == 0 { self.slice.sao_luma } else { self.slice.sao_chroma };
			if !wanted {
				continue;
			}
			// Luma and the first chroma component each carry a type; the second takes the first's.
			if c < 2 {
				// Truncated unary of at most two: nothing, a band offset, or an edge offset.
				let first = res!(ent.bin(Set::SaoType, 0));
				sao.kind[c] = if first == 0 {
					0
				} else if ent.cabac.bypass() == 0 {
					1
				} else {
					2
				};
			} else {
				sao.kind[2] = sao.kind[1];
				sao.class[2] = sao.class[1];
			}
			if sao.kind[c] == 0 {
				continue;
			}
			// The offsets themselves, each a truncated unary at even odds, bounded by what the bit
			// depth allows: seven at eight bits, and never more than thirty-one (§9.3.3, Table
			// 9-43). A bound that is too large reads bins that were never coded and every syntax
			// element after it in the picture is shifted -- which is what this was, at 127.
			let depth = if c == 0 { self.sps.luma_bits } else { self.sps.chroma_bits };
			let most = (1usize << (depth.min(10) as usize - 5)) - 1;
			for i in 0..4 {
				sao.offset[c][i] = ent.unary_bypass(most) as i32;
			}
			if sao.kind[c] == 1 {
				for i in 0..4 {
					if sao.offset[c][i] != 0 && ent.cabac.bypass() == 1 {
						sao.offset[c][i] = -sao.offset[c][i];
					}
				}
				sao.band[c] = ent.cabac.bypass_bits(5) as u8;
			} else {
				// An edge offset's four are two positive and two negative by construction, so no
				// signs are coded.
				sao.offset[c][2] = -sao.offset[c][2];
				sao.offset[c][3] = -sao.offset[c][3];
				if c < 2 {
					sao.class[c] = ent.cabac.bypass_bits(2) as u8;
					if c == 1 {
						sao.class[2] = sao.class[1];
					}
				}
			}
		}
		self.sao[at] = sao;
		Ok(())
	}

	/// One node of the coding quadtree (§7.3.8.4).
	fn quadtree(&mut self, ent: &mut Ent, x: usize, y: usize, log2: u32, depth: u8)
		-> Outcome<()>
	{
		let size = 1usize << log2;
		let (w, h) = (self.pic.y.w, self.pic.y.h);
		let min_cb = self.sps.min_cb.trailing_zeros();
		// A node hanging over the edge of the picture is split without being told to, and one at
		// the smallest coding size cannot split at all. Only in between is a flag coded.
		let mut split = log2 > min_cb;
		if x + size <= w && y + size <= h && log2 > min_cb {
			// The context depends on whether the neighbours went deeper than this node, which is
			// what makes a picture of small blocks cheap to say so about.
			let left = if self.available(x, y, x as i32 - 1, y as i32) {
				(self.ct_depth[(y / 4) * self.gw + (x - 1) / 4] > depth) as usize
			} else {
				0
			};
			let above = if self.available(x, y, x as i32, y as i32 - 1) {
				(self.ct_depth[((y - 1) / 4) * self.gw + x / 4] > depth) as usize
			} else {
				0
			};
			split = res!(ent.bin(Set::SplitCu, left + above)) == 1;
		}
		// A quantisation group begins at whatever depth the picture chose.
		if self.pps.cu_qp_delta && log2 >= self.qp_group_log2() {
			self.qp_coded = false;
			self.qp_delta = 0;
		}
		if split {
			let half = size / 2;
			let next = log2 - 1;
			res!(self.quadtree(ent, x, y, next, depth + 1));
			if x + half < w {
				res!(self.quadtree(ent, x + half, y, next, depth + 1));
			}
			if y + half < h {
				res!(self.quadtree(ent, x, y + half, next, depth + 1));
			}
			if x + half < w && y + half < h {
				res!(self.quadtree(ent, x + half, y + half, next, depth + 1));
			}
			return Ok(());
		}
		self.coding_unit(ent, x, y, log2, depth)
	}

	/// The size of a quantisation group, as a base-two logarithm.
	fn qp_group_log2(&self) -> u32 {
		self.sps.ctb_size.trailing_zeros() - self.pps.qp_delta_depth as u32
	}

	/// One coding unit (§7.3.8.5), for the intra case, which is all a still picture has.
	fn coding_unit(&mut self, ent: &mut Ent, x: usize, y: usize, log2: u32, depth: u8)
		-> Outcome<()>
	{
		let size = 1usize << log2;
		self.bypass = false;
		if self.pps.transquant_bypass {
			self.bypass = res!(ent.bin(Set::TransquantBypass, 0)) == 1;
		}
		// Only at the smallest coding size may an intra unit be split into four prediction blocks,
		// and only then is the partition mode coded at all.
		let min_cb = self.sps.min_cb.trailing_zeros();
		self.split_intra = if log2 == min_cb {
			// One bin: set means the whole unit, clear means four.
			res!(ent.bin(Set::PartMode, 0)) == 0
		} else {
			false
		};
		let parts = if self.split_intra { 4 } else { 1 };
		let step = if self.split_intra { size / 2 } else { size };

		// Whether each prediction block takes one of the three modes its neighbours suggest.
		let mut from_list = [false; 4];
		for p in 0..parts {
			from_list[p] = res!(ent.bin(Set::PrevIntraLumaPred, 0)) == 1;
		}
		for p in 0..parts {
			let (px, py) = (x + (p & 1) * step, y + (p >> 1) * step);
			let list = self.mode_candidates(px, py);
			self.pred_y[p] = if from_list[p] {
				// Two bins at even odds, truncated: 0, 10 or 11.
				let idx = if ent.cabac.bypass() == 0 {
					0
				} else if ent.cabac.bypass() == 0 {
					1
				} else {
					2
				};
				list[idx]
			} else {
				// Five bits at even odds, naming one of the thirty-two that are not in the list.
				let mut sorted = list;
				sorted.sort_unstable();
				let mut mode = ent.cabac.bypass_bits(5) as u8;
				for candidate in sorted {
					if mode >= candidate {
						mode += 1;
					}
				}
				mode
			};
			// Written down as each block's mode is settled, because the next block's candidate
			// list is drawn from it.
			self.put_mode(px, py, step, self.pred_y[p]);
		}
		// One chroma mode for the whole unit in 4:2:0.
		let chroma_syntax = if res!(ent.bin(Set::IntraChromaPredMode, 0)) == 0 {
			4
		} else {
			ent.cabac.bypass_bits(2) as usize
		};
		self.pred_c = chroma_mode(chroma_syntax, self.pred_y[0]);

		self.cu_x = x;
		self.cu_y = y;
		self.cu_size = size;
		self.cbf_cb = [false; 6];
		self.cbf_cr = [false; 6];
		// The parameter this unit will use, unless its own residual carries a change.
		self.qp_now = self.predict_qp(x, y);
		self.record_depth(x, y, size, depth);
		self.record_qp(x, y, size, self.qp_now as i8);

		let max_depth = self.sps.max_depth_intra as u32 + self.split_intra as u32;
		res!(self.transform_tree(ent, x, y, x, y, log2, 0, 0, max_depth));
		// What the next quantisation group predicts from is the last unit of this one, and this is
		// the last unit until another follows it.
		self.qp_prev = self.qp_now;
		Ok(())
	}

	/// Writes a prediction mode against every four-by-four block it covers.
	fn put_mode(&mut self, x: usize, y: usize, size: usize, mode: u8) {
		for gy in (y / 4)..((y + size).div_ceil(4)).min(self.gh) {
			for gx in (x / 4)..((x + size).div_ceil(4)).min(self.gw) {
				self.mode[gy * self.gw + gx] = mode;
			}
		}
	}

	/// The three modes a block's neighbours suggest (§8.4.2).
	///
	/// A block whose left and upper neighbours agree gets that mode and its two nearest angles; one
	/// whose neighbours differ gets both of theirs and a third that is not either. The upper
	/// neighbour is only consulted **within the same row of coding tree blocks**: a decoder running
	/// the rows in parallel cannot see the row above, so the specification does not let it.
	fn mode_candidates(&self, x: usize, y: usize) -> [u8; 3] {
		let ctb = self.sps.ctb_size as usize;
		let left = if self.available(x, y, x as i32 - 1, y as i32) {
			self.mode_of(x - 1, y)
		} else {
			intra::DC
		};
		let above = if y % ctb == 0 || !self.available(x, y, x as i32, y as i32 - 1) {
			intra::DC
		} else {
			self.mode_of(x, y - 1)
		};
		if left == above {
			if left < 2 {
				return [intra::PLANAR, intra::DC, intra::VERTICAL];
			}
			return [
				left,
				2 + ((left as u32 + 29) % 32) as u8,
				2 + ((left as u32 - 2 + 1) % 32) as u8,
			];
		}
		let third = if left != intra::PLANAR && above != intra::PLANAR {
			intra::PLANAR
		} else if left != intra::DC && above != intra::DC {
			intra::DC
		} else {
			intra::VERTICAL
		};
		[left, above, third]
	}

	/// What the quantisation parameter of a coding unit is predicted to be (§8.6.1).
	fn predict_qp(&self, x: usize, y: usize) -> i32 {
		if !self.pps.cu_qp_delta {
			return self.slice.qp;
		}
		let group = 1usize << self.qp_group_log2();
		let (qx, qy) = (x - (x & (group - 1)), y - (y & (group - 1)));
		let ctb = self.sps.ctb_size as usize;
		// A neighbour in another coding tree block does not count: the prediction is meant to stay
		// inside one, so that a row decoded on its own gives the same answer.
		let same_ctb = |nx: i32, ny: i32| {
			nx >= 0 && ny >= 0
				&& (nx as usize) / ctb == x / ctb
				&& (ny as usize) / ctb == y / ctb
		};
		let left = if same_ctb(qx as i32 - 1, qy as i32)
			&& self.available(x, y, qx as i32 - 1, qy as i32)
		{
			self.qp[(qy / 4) * self.gw + (qx - 1) / 4] as i32
		} else {
			self.qp_prev
		};
		let above = if same_ctb(qx as i32, qy as i32 - 1)
			&& self.available(x, y, qx as i32, qy as i32 - 1)
		{
			self.qp[((qy - 1) / 4) * self.gw + qx / 4] as i32
		} else {
			self.qp_prev
		};
		(left + above + 1) >> 1
	}

	/// One node of the transform tree (§7.3.8.8).
	#[allow(clippy::too_many_arguments)]
	fn transform_tree(
		&mut self,
		ent:	&mut Ent,
		x:	usize,
		y:	usize,
		base_x:	usize,
		base_y:	usize,
		log2:	u32,
		depth:	u32,
		blk:	usize,
		max_depth: u32,
	)
		-> Outcome<()>
	{
		let max_tb = self.sps.max_tb.trailing_zeros();
		let min_tb = self.sps.min_tb.trailing_zeros();
		// Forced where the block is too large for one transform, or where a coding unit split
		// into four prediction blocks makes its transform tree follow it down one level; coded
		// where neither applies.
		let coded = log2 <= max_tb && log2 > min_tb && depth < max_depth
			&& !(self.split_intra && depth == 0);
		let split = if coded {
			res!(ent.bin(Set::SplitTransform, (5 - log2) as usize)) == 1
		} else {
			log2 > max_tb || (self.split_intra && depth == 0 && log2 > min_tb)
		};
		// Chroma has a residual flag only where the block is big enough to have chroma of its own.
		let d = depth as usize;
		if log2 > 2 {
			if depth == 0 || self.cbf_cb[d - 1] {
				self.cbf_cb[d] = res!(ent.bin(Set::CbfChroma, d)) == 1;
			} else {
				self.cbf_cb[d] = false;
			}
			if depth == 0 || self.cbf_cr[d - 1] {
				self.cbf_cr[d] = res!(ent.bin(Set::CbfChroma, d)) == 1;
			} else {
				self.cbf_cr[d] = false;
			}
		} else if d > 0 {
			// A four-sample luma block has no chroma of its own; the quad shares its parent's.
			self.cbf_cb[d] = self.cbf_cb[d - 1];
			self.cbf_cr[d] = self.cbf_cr[d - 1];
		} else {
			return Err(err!(
				"A four-sample transform block sits at the top of its tree, which cannot happen: 				the smallest coding block is {} samples.", self.sps.min_cb; Invalid, Input, Decode));
		}
		if split {
			let half = 1usize << (log2 - 1);
			for (i, (dx, dy)) in [(0, 0), (half, 0), (0, half), (half, half)].iter().enumerate() {
				res!(self.transform_tree(
					ent, x + dx, y + dy, x, y, log2 - 1, depth + 1, i, max_depth));
			}
			return Ok(());
		}
		// An intra block always has a luma residual flag; there is nothing else it could carry.
		let cbf_luma = res!(ent.bin(Set::CbfLuma, (depth == 0) as usize)) == 1;
		self.transform_unit(ent, x, y, base_x, base_y, log2, depth, blk, cbf_luma)
	}

	/// One transform unit: the residuals, and the reconstruction they belong to (§7.3.8.10).
	#[allow(clippy::too_many_arguments)]
	fn transform_unit(
		&mut self,
		ent:	&mut Ent,
		x:	usize,
		y:	usize,
		base_x:	usize,
		base_y:	usize,
		log2:	u32,
		depth:	u32,
		blk:	usize,
		cbf_luma: bool,
	)
		-> Outcome<()>
	{
		let d = depth as usize;
		// Where the block is four samples wide the chroma of the whole quad is carried by the last
		// of the four, and the flags belong to the parent.
		let small = log2 == 2;
		let cd = if small { d - 1 } else { d };
		// Whether this unit has any chroma residual **at all**, which is a different question from
		// whether this is the block that carries it. A four-sample quad's chroma is read at the
		// last of the four, but all four of them share the flags -- so the first of them, even
		// with no luma residual of its own, is where a change of quantisation parameter is coded.
		// Gating this on the last block instead read that change at the wrong point in the stream,
		// and everything after it in the picture was decoded from the wrong bins.
		let chroma_here = !small || blk == 3;
		let has_chroma = self.cbf_cb[cd] || self.cbf_cr[cd];
		if cbf_luma || has_chroma {
			if self.pps.cu_qp_delta && !self.qp_coded {
				self.qp_coded = true;
				// A unary run of up to five against contexts, then the rest at even odds.
				let mut abs = res!(ent.unary(Set::CuQpDeltaAbs, &[0, 1], 5));
				if abs == 5 {
					abs += golomb(ent, 0);
				}
				if abs > 0 && ent.cabac.bypass() == 1 {
					self.qp_delta = -(abs as i32);
				} else {
					self.qp_delta = abs as i32;
				}
				let offset = 0; // Eight-bit pictures have no quantisation parameter offset.
				let span = 52 + offset;
				self.qp_now = ((self.predict_qp(self.cu_x, self.cu_y) + self.qp_delta + span
					+ offset) % span) - offset;
				let (cx, cy, cs) = (self.cu_x, self.cu_y, self.cu_size);
				self.record_qp(cx, cy, cs, self.qp_now as i8);
				self.qp_prev = self.qp_now;
			}
		}
		// Luma first, because the chroma of a small quad is written after all four of them.
		let mode_y = self.mode_of(x, y);
		res!(self.block(ent, x, y, log2, 0, mode_y, cbf_luma));

		if !chroma_here {
			return Ok(());
		}
		// In 4:2:0 the chroma block is half the size, and a four-sample luma quad shares one.
		let (cx, cy, clog2) = if small {
			(base_x / 2, base_y / 2, log2)
		} else {
			(x / 2, y / 2, log2 - 1)
		};
		let mode_c = self.pred_c;
		res!(self.block(ent, cx, cy, clog2, 1, mode_c, self.cbf_cb[cd]));
		res!(self.block(ent, cx, cy, clog2, 2, mode_c, self.cbf_cr[cd]));
		Ok(())
	}

	/// Predicts one transform block, reads its residual where it has one, and writes the samples.
	fn block(
		&mut self,
		ent:	&mut Ent,
		x:	usize,
		y:	usize,
		log2:	u32,
		cidx:	usize,
		mode:	u8,
		coded:	bool,
	)
		-> Outcome<()>
	{
		let size = 1usize << log2;
		let chroma = cidx > 0;
		let mut coeffs = [0i32; 32 * 32];
		self.skip_tr = false;
		if coded {
			res!(self.residual(ent, log2, cidx, mode, &mut coeffs));
		}
		// The samples around the block, and whether each of them exists.
		let mut around = intra::Around::new(size);
		let (px, py) = if chroma { (x * 2, y * 2) } else { (x, y) };
		let step = if chroma { 2usize } else { 1 };
		{
			let plane = self.plane(cidx);
			if self.available(px, py, px as i32 - step as i32, py as i32 - step as i32) {
				if let Some(v) = plane.at(x.wrapping_sub(1), y.wrapping_sub(1)) {
					around.set_corner(v as i32);
				}
			}
			for i in 0..size * 2 {
				if self.available(px, py, px as i32 - step as i32, (py + i * step) as i32) {
					if let Some(v) = plane.at(x.wrapping_sub(1), y + i) {
						around.set_left(i, v as i32);
					}
				}
				if self.available(px, py, (px + i * step) as i32, py as i32 - step as i32) {
					if let Some(v) = plane.at(x + i, y.wrapping_sub(1)) {
						around.set_top(i, v as i32);
					}
				}
			}
		}
		let depth = self.pic.depth;
		around.substitute(depth);
		around.smooth(mode, chroma, self.sps.strong_smoothing, depth);
		let mut pred = [0i32; 32 * 32];
		res!(intra::predict(&around, mode, size, chroma, depth, &mut pred));

		if coded {
			let qp = self.block_qp(cidx);
			if self.bypass {
				// Nothing was quantised and nothing transformed: the coefficients are the residual.
			} else {
				let m = self.weights_for(size, log2, cidx);
				transform::scale(&mut coeffs, size, qp, depth, &m);
				if self.skip_tr {
					transform::skipped(&mut coeffs, size);
				} else {
					let kind = transform::Kind::of(true, size, chroma);
					res!(transform::inverse(&mut coeffs, size, kind));
				}
				transform::finish(&mut coeffs, size, depth);
			}
		}
		let top = (1i32 << depth) - 1;
		for j in 0..size {
			for i in 0..size {
				let v = (pred[j * size + i] + coeffs[j * size + i]).clamp(0, top);
				self.plane_mut(cidx).put(x + i, y + j, v as u16);
			}
		}
		Ok(())
	}

	/// The scaling matrix one block is quantised against (§7.4.5, equations 7-44 to 7-49).
	///
	/// A four-sample block is flat; an eight-sample one takes the matrix as it stands; and the two
	/// larger sizes take it with each of its values covering two or four samples each way. All
	/// three colour components share one matrix here, because the default lists give the same
	/// numbers to all three.
	fn weights_for(&self, size: usize, log2: u32, cidx: usize) -> Vec<i32> {
		let scaling = match &self.weights {
			Some(s) => s,
			None => return vec![16; size * size],
		};
		// An intra picture only ever uses the first three of the six lists: the other three belong
		// to blocks predicted from another picture, which a still photograph has none of.
		let matrix = cidx;
		let raster = scaling.raster(log2, matrix);
		let mut out = Vec::with_capacity(size * size);
		for y in 0..size {
			for x in 0..size {
				out.push(scaling.factor(log2, matrix, x, y, &raster));
			}
		}
		out
	}

	/// The quantisation parameter one component's block is scaled by (§8.6.1).
	fn block_qp(&self, cidx: usize) -> i32 {
		if cidx == 0 {
			return self.qp_now.clamp(0, 51);
		}
		let offset = if cidx == 1 {
			self.pps.cb_qp_offset + self.slice.cb_qp_offset
		} else {
			self.pps.cr_qp_offset + self.slice.cr_qp_offset
		};
		chroma_qp((self.qp_now + offset).clamp(0, 57))
	}

	/// One component's samples.
	fn plane(&self, cidx: usize) -> &Plane {
		match cidx {
			0	=> &self.pic.y,
			1	=> &self.pic.cb,
			_	=> &self.pic.cr,
		}
	}

	/// The same, to write to.
	fn plane_mut(&mut self, cidx: usize) -> &mut Plane {
		match cidx {
			0	=> &mut self.pic.y,
			1	=> &mut self.pic.cb,
			_	=> &mut self.pic.cr,
		}
	}

	/// The coefficients of one transform block (§7.3.8.11).
	#[allow(clippy::too_many_arguments)]
	fn residual(
		&mut self,
		ent:	&mut Ent,
		log2:	u32,
		cidx:	usize,
		mode:	u8,
		out:	&mut [i32],
	)
		-> Outcome<()>
	{
		let size = 1usize << log2;
		let chroma = cidx > 0;
		if self.pps.transform_skip && !self.bypass && log2 == 2 {
			self.skip_tr = res!(ent.bin(Set::TransformSkip, (cidx > 0) as usize)) == 1;
		}
		let order = Order::of(log2, chroma, mode);

		// Where the last coefficient in coding order sits. Its prefix is a truncated unary against
		// contexts that depend on the block size, and its suffix is plain bits.
		let (offset, shift) = if cidx == 0 {
			(3 * (log2 as usize - 2) + ((log2 as usize - 1) >> 2), (log2 + 1) >> 2)
		} else {
			(15usize, log2 - 2)
		};
		let most = (log2 as usize) * 2 - 1;
		let incs: Vec<usize> = (0..most).map(|b| (b >> shift) + offset).collect();
		let px = res!(ent.unary(Set::LastSigX, &incs, most));
		let py = res!(ent.unary(Set::LastSigY, &incs, most));
		let last_x = suffix_of(ent, px);
		let last_y = suffix_of(ent, py);
		let (last_x, last_y) = if order == Order::Vertical {
			(last_y, last_x)
		} else {
			(last_x, last_y)
		};

		// Which sub-block that lands in, and where within it.
		let sub_log2 = log2 - 2;
		let subs = self.scans.of(sub_log2, order).to_vec();
		let coeff_scan = self.scans.of(2, order).to_vec();
		let mut last_sub = subs.len() - 1;
		let mut last_pos = 16usize;
		'find: loop {
			if last_pos == 0 {
				last_pos = 16;
				if last_sub == 0 {
					break;
				}
				last_sub -= 1;
			}
			last_pos -= 1;
			let (sx, sy) = subs[last_sub];
			let (cx, cy) = coeff_scan[last_pos];
			if (sx as usize * 4 + cx as usize) == last_x as usize
				&& (sy as usize * 4 + cy as usize) == last_y as usize
			{
				break 'find;
			}
			if last_sub == 0 && last_pos == 0 {
				return Err(err!(
					"The last coefficient of a {0} by {0} block is at ({1}, {2}), which its scan \
					never reaches.", size, last_x, last_y; Invalid, Input, Decode));
			}
		}

		let mut coded_sub = vec![false; subs.len()];
		coded_sub[last_sub] = true;
		if !subs.is_empty() {
			coded_sub[0] = true;
		}
		// Carried between sub-blocks: which context set the magnitudes are read against.
		let mut prev_greater1_ctx = 1i32;

		for i in (0..=last_sub).rev() {
			let (sx, sy) = subs[i];
			let (sx, sy) = (sx as usize, sy as usize);
			let mut infer_dc = false;
			if i < last_sub && i > 0 {
				let right = if sx < (1 << sub_log2) - 1 {
					coded_sub[position_of(&subs, sx + 1, sy)] as usize
				} else {
					0
				};
				let below = if sy < (1 << sub_log2) - 1 {
					coded_sub[position_of(&subs, sx, sy + 1)] as usize
				} else {
					0
				};
				let inc = (right + below).min(1) + if chroma { 2 } else { 0 };
				coded_sub[i] = res!(ent.bin(Set::CodedSubBlock, inc)) == 1;
				infer_dc = true;
			}
			if !coded_sub[i] {
				continue;
			}
			// Which coefficients in this sub-block are not nought.
			let mut sig = [false; 16];
			// The last coefficient is significant by definition -- saying where it was is what
			// the block began with -- so the flags start one before it.
			let start = if i == last_sub { last_pos as i32 - 1 } else { 15 };
			if i == last_sub {
				sig[last_pos] = true;
			}
			for n in (0..=start).rev() {
				let n = n as usize;
				if n > 0 || !infer_dc {
					let (cx, cy) = coeff_scan[n];
					let inc = self.sig_ctx(
						sx, sy, cx as usize, cy as usize, log2, cidx, order, &coded_sub, &subs);
					sig[n] = res!(ent.bin(Set::SigCoeff, inc)) == 1;
					if sig[n] {
						infer_dc = false;
					}
				} else {
					// The only coefficient left in a sub-block known to hold something.
					sig[n] = true;
				}
			}

			// Their magnitudes, in three passes: over one, over two, and the rest.
			let mut ctx_set = if i == 0 || chroma { 0usize } else { 2 };
			if prev_greater1_ctx == 0 {
				ctx_set += 1;
			}
			let mut greater1_ctx = 1i32;
			let mut n_greater1 = 0usize;
			let mut last_greater1 = -1i32;
			let mut greater1 = [false; 16];
			for n in (0..16).rev() {
				if !sig[n] {
					continue;
				}
				if n_greater1 < 8 {
					let inc = ctx_set * 4 + (greater1_ctx.min(3) as usize)
						+ if chroma { 16 } else { 0 };
					greater1[n] = res!(ent.bin(Set::Greater1, inc)) == 1;
					n_greater1 += 1;
					if greater1[n] {
						greater1_ctx = 0;
						if last_greater1 == -1 {
							last_greater1 = n as i32;
						}
					} else if greater1_ctx > 0 {
						greater1_ctx += 1;
					}
				}
			}
			// Carried to the next sub-block, but only where this one asked the question at all:
			// a sub-block whose coefficients are all ones leaves the context where it found it.
			if n_greater1 > 0 {
				prev_greater1_ctx = greater1_ctx;
			}

			let mut greater2 = [false; 16];
			if last_greater1 >= 0 {
				let inc = ctx_set + if chroma { 4 } else { 0 };
				greater2[last_greater1 as usize] = res!(ent.bin(Set::Greater2, inc)) == 1;
			}

			// Where the first and last of them are, which is what decides whether one sign is
			// carried by the parity of the sum rather than by a bit of its own.
			let mut first_sig = 16i32;
			let mut last_sig = -1i32;
			for n in (0..16).rev() {
				if sig[n] {
					if last_sig == -1 {
						last_sig = n as i32;
					}
					first_sig = n as i32;
				}
			}
			let hidden = self.pps.sign_hiding && !self.bypass && last_sig - first_sig > 3;

			let mut signs = [false; 16];
			for n in (0..16).rev() {
				if sig[n] && (!hidden || n as i32 != first_sig) {
					signs[n] = ent.cabac.bypass() == 1;
				}
			}

			// And the magnitudes themselves.
			let mut rice = 0u32;
			let mut n_sig = 0usize;
			let mut sum = 0i64;
			let mut last_abs = 0i64;
			let mut first = true;
			for n in (0..16).rev() {
				if !sig[n] {
					continue;
				}
				let base = 1 + greater1[n] as i32 + greater2[n] as i32;
				let threshold = if n_sig < 8 {
					if n as i32 == last_greater1 { 3 } else { 2 }
				} else {
					1
				};
				let mut level = base as i64;
				if base == threshold {
					if first {
						rice = 0;
					} else {
						rice = (rice + (last_abs > (3 << rice) as i64) as u32).min(4);
					}
					level += remaining(ent, rice) as i64;
					first = false;
					last_abs = level;
				}
				let (cx, cy) = coeff_scan[n];
				let at = (sy * 4 + cy as usize) * size + sx * 4 + cx as usize;
				sum += level;
				let negative = signs[n] || (hidden && n as i32 == first_sig && sum % 2 == 1);
				out[at] = if negative { -(level as i32) } else { level as i32 };
				n_sig += 1;
			}
		}
		Ok(())
	}

	/// The context one significance flag is read against (§9.3.4.2.5).
	#[allow(clippy::too_many_arguments)]
	fn sig_ctx(
		&self,
		sx:	usize,
		sy:	usize,
		cx:	usize,
		cy:	usize,
		log2:	u32,
		cidx:	usize,
		order:	Order,
		coded:	&[bool],
		subs:	&[(u8, u8)],
	)
		-> usize
	{
		let chroma = cidx > 0;
		if self.skip_tr || self.bypass {
			// A block coded without its transform has no corner to speak of, so it has contexts of
			// its own.
			return if chroma { 27 + 16 } else { 42 };
		}
		if log2 == 2 {
			// A four-sample block reads its contexts straight off a map of its sixteen positions.
			const MAP: [usize; 16] = [0, 1, 4, 5, 2, 3, 4, 5, 6, 6, 8, 8, 7, 7, 8, 8];
			let v = MAP[cy * 4 + cx];
			return if chroma { 27 + v } else { v };
		}
		let (x, y) = (sx * 4 + cx, sy * 4 + cy);
		if x + y == 0 {
			return if chroma { 27 } else { 0 };
		}
		let side = 1usize << (log2 - 2);
		let mut prev = 0usize;
		if sx < side - 1 && coded[position_of(subs, sx + 1, sy)] {
			prev += 1;
		}
		if sy < side - 1 && coded[position_of(subs, sx, sy + 1)] {
			prev += 2;
		}
		let (px, py) = (cx, cy);
		let mut ctx = match prev {
			0 => if px + py == 0 { 2 } else if px + py < 3 { 1 } else { 0 },
			1 => if py == 0 { 2 } else if py == 1 { 1 } else { 0 },
			2 => if px == 0 { 2 } else if px == 1 { 1 } else { 0 },
			_ => 2,
		};
		if !chroma {
			if sx + sy > 0 {
				ctx += 3;
			}
			ctx += if log2 == 3 {
				if order == Order::Diagonal { 9 } else { 15 }
			} else {
				21
			};
		} else {
			ctx += if log2 == 3 { 9 } else { 12 };
		}
		if chroma {
			27 + ctx
		} else {
			ctx
		}
	}
}

/// Where a sub-block sits in the scan.
fn position_of(subs: &[(u8, u8)], x: usize, y: usize) -> usize {
	subs.iter()
		.position(|(sx, sy)| *sx as usize == x && *sy as usize == y)
		.unwrap_or(0)
}

/// The plain-bits half of a last-coefficient position, where the prefix says there is one.
fn suffix_of(ent: &mut Ent, prefix: u32) -> u32 {
	if prefix <= 3 {
		return prefix;
	}
	let bits = (prefix >> 1) - 1;
	let suffix = ent.cabac.bypass_bits(bits as usize);
	(1 << bits) * (2 + (prefix & 1)) + suffix
}

/// An exponential Golomb code at even odds, of the given order (§9.3.3.3).
fn golomb(ent: &mut Ent, k: u32) -> u32 {
	let mut k = k;
	let mut value = 0u32;
	while ent.cabac.bypass() == 1 {
		value += 1 << k;
		k += 1;
		if k > 30 {
			return value;
		}
	}
	value + ent.cabac.bypass_bits(k as usize)
}

/// What is left of a coefficient's magnitude past what the flags said (§9.3.3.11).
///
/// A truncated Rice code whose parameter grows with the magnitudes already seen in this sub-block,
/// and past four of those an exponential Golomb code takes over.
fn remaining(ent: &mut Ent, rice: u32) -> u32 {
	let prefix = ent.unary_bypass(4) as u32;
	if prefix < 4 {
		return (prefix << rice) + ent.cabac.bypass_bits(rice as usize);
	}
	(4 << rice) + golomb(ent, rice + 1)
}

/// The chroma prediction mode a coding unit's syntax names (§8.4.3, Table 8-2).
///
/// Four of the five choices are fixed directions and the fifth is "the same as luma"; where a fixed
/// choice happens to be what luma already uses, the mode moves to 34 so that the two are never
/// coded twice.
fn chroma_mode(syntax: usize, luma: u8) -> u8 {
	if syntax == 4 {
		return luma;
	}
	let fixed = [intra::PLANAR, intra::VERTICAL, intra::HORIZONTAL, intra::DC][syntax];
	if fixed == luma {
		34
	} else {
		fixed
	}
}

/// The chroma quantisation parameter for a given luma one (§8.6.1, Table 8-10).
///
/// Chroma is quantised more gently than luma above a parameter of thirty, because the eye is less
/// able to see colour noise than brightness noise -- but only up to a point, past which the two run
/// parallel again six steps apart.
fn chroma_qp(qpi: i32) -> i32 {
	match qpi {
		i32::MIN..=29	=> qpi,
		30..=43		=> [29, 30, 31, 32, 33, 33, 34, 34, 35, 35, 36, 36, 37, 37][(qpi - 30) as usize],
		_		=> qpi - 6,
	}
}
