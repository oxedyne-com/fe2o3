//! An MP4 writer: the ISO base media file format boxes that wrap an already-encoded video track.
//!
//! This module writes a container around a stream it can neither encode nor decode. That is
//! unusual for a graphics crate and it is deliberate. A video codec is months of rate control,
//! motion estimation and entropy coding, at a quality that would be visibly worse than the encoder
//! already sitting in every browser and in most machines' silicon; a container is a few hundred
//! lines of length-prefixed boxes with no compression in it at all, and it is the part that
//! describes *our* frames and *our* timing. So the caller encodes -- through `VideoEncoder` in a
//! browser, or through whatever hardware path it has -- and hands the encoded samples and the
//! decoder configuration here, and gets back a file.
//!
//! # What is written
//!
//! `ftyp`, then `moov` carrying one video track's whole sample table, then `mdat` carrying the
//! samples. Not fragmented: the caller holds every sample's size and duration before the first byte
//! is written, so the sample table can be exact and the file needs no `moof` machinery, no
//! duration-unknown placeholder and no rewrite at the end.
//!
//! `moov` is written *before* `mdat`, which is what a progressive download wants: a reader has the
//! whole index in hand after the first few kilobytes and can start playing without seeking to the
//! end. It costs a second pass over the box tree, because the chunk offsets in `stco` are absolute
//! file offsets and cannot be known until the size of the index that precedes them is.
//!
//! # What is refused
//!
//! A track with no samples; a timescale of zero; a sample of zero duration; a sample whose bytes
//! are not exactly tiled by the length-prefixed NAL units the decoder configuration says they are;
//! a decoder configuration record that is malformed or truncated; frame dimensions that disagree
//! with the ones coded in the sequence parameter set; a first sample that is not a sync sample,
//! since a track no reader can begin decoding is not a track; and a total duration too large for
//! the 32-bit fields the version-0 header boxes carry.
//!
//! # References
//!
//! `ftyp`, `moov` and everything under it, and `mdat`, are ISO/IEC 14496-12 (the ISO base media
//! file format). The `avc1` sample entry and the `avcC` configuration box it carries are ISO/IEC
//! 14496-15 (the AVC file format). The sequence parameter set whose geometry is checked against the
//! caller's declared dimensions is ITU-T H.264 §7.3.2.1.1. Each non-obvious constant below names
//! the clause it comes from.

use oxedyne_fe2o3_core::prelude::*;

/// The most samples one track may hold, a ceiling against a length that is a mistake.
///
/// A million frames is about eleven and a half hours at twenty-four a second, which is longer than
/// anything a single non-fragmented file is the right shape for.
pub const MAX_SAMPLES: usize = 1_000_000;

/// The timescale of the movie header, in ticks a second.
///
/// The movie has a timescale of its own, separate from each track's, and every duration in `mvhd`
/// and `tkhd` is expressed in it while every duration in `mdhd` and `stts` is expressed in the
/// track's. A thousand -- milliseconds -- is the conventional choice and is what makes the movie
/// header readable when a second track arrives on a different timescale from the first.
pub const MOVIE_TIMESCALE: u32 = 1000;

/// The unity transformation a `tkhd` and an `mvhd` carry.
///
/// Nine values in the order `a`, `b`, `u`, `c`, `d`, `v`, `x`, `y`, `w` (ISO/IEC 14496-12 §8.2.2.3
/// for `mvhd` and §8.3.2.3 for `tkhd`). The six that scale and rotate are 16.16 fixed point, so one
/// is `0x00010000`; the three of the projection column are 2.30, so one is `0x40000000`. Written
/// as unity because a track that is played the way it was drawn needs no transformation, and a
/// non-unity matrix here is how a video ends up rotated in one player and not another.
const UNITY: [u32; 9] = [
	0x0001_0000,	0,		0,
	0,		0x0001_0000,	0,
	0,		0,		0x4000_0000,
];

/// The language of the media, packed as three five-bit letters offset from `0x60`, per ISO/IEC
/// 14496-12 §8.4.2.3: `und`, undetermined, which is what a video track without speech in it is.
///
/// `u` is 21, `n` is 14 and `d` is 4, so the packed value is `(21 << 10) | (14 << 5) | 4`.
const LANG_UND: u16 = 0x55C4;

/// The horizontal and vertical resolution of a visual sample entry, in 16.16 fixed point dots an
/// inch: 72, which ISO/IEC 14496-12 §8.5.2.3 gives as the value to write.
const RESOLUTION_72: u32 = 0x0048_0000;

/// A sample of an encoded track: the bytes of one access unit, how long it is shown, and whether a
/// reader may begin decoding at it.
///
/// A sample is one coded picture. Its duration is in the track's timescale rather than in seconds,
/// because the rates that matter divide badly -- a 24000/1001 frame rate is exact on a timescale of
/// 24000 and is nothing at all in milliseconds -- and because that is the unit the sample table
/// stores.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
	/// The coded bytes, as a chain of length-prefixed NAL units.
	pub data:	Vec<u8>,
	/// How long the sample is shown, in the track's timescale.
	pub dur:	u32,
	/// Whether decoding may begin here: a sync sample, which for AVC is an IDR picture.
	pub sync:	bool,
}

impl Sample {

	/// A sync sample: one a reader may begin decoding at.
	pub fn key(data: Vec<u8>, dur: u32) -> Self {
		Self { data, dur, sync: true }
	}

	/// A sample that depends on those before it.
	pub fn delta(data: Vec<u8>, dur: u32) -> Self {
		Self { data, dur, sync: false }
	}

	/// The size of the sample in bytes.
	pub fn len(&self) -> usize {
		self.data.len()
	}

	/// Whether the sample carries no bytes, which no legal coded picture does.
	pub fn is_empty(&self) -> bool {
		self.data.is_empty()
	}
}

/// How a track's samples are coded, and the decoder configuration that goes with them.
///
/// An enum rather than a trait object, so that adding HEVC later is a variant and a match arm
/// rather than a second dispatch mechanism, and so that a caller can see from the type what the
/// writer will accept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Codec {
	/// H.264, carrying the `AVCDecoderConfigurationRecord` of ISO/IEC 14496-15 §5.3.3.1 verbatim:
	/// the bytes a `VideoEncoder` hands back as its output's description, or the `avcC` box body
	/// lifted out of another file.
	Avc(Vec<u8>),
}

impl Codec {

	/// The four-character code of the sample entry this codec is described by, ISO/IEC 14496-15
	/// §5.4.2.1.
	fn entry(&self) -> &'static [u8; 4] {
		match self {
			Self::Avc(_) => b"avc1",
		}
	}

	/// The four-character code of the configuration box carried inside that sample entry.
	fn config(&self) -> &'static [u8; 4] {
		match self {
			Self::Avc(_) => b"avcC",
		}
	}

	/// The configuration record's bytes, written into the configuration box unchanged.
	fn record(&self) -> &[u8] {
		match self {
			Self::Avc(rec) => rec,
		}
	}

	/// How many bytes prefix each NAL unit in a sample, which the configuration record states and
	/// the sample data must obey.
	fn nal_len(&self) -> Outcome<usize> {
		match self {
			Self::Avc(rec) => {
				if rec.len() < 7 {
					return Err(err!(
						"An AVC decoder configuration record is at least 7 bytes and this one is \
						{}.", rec.len();
					Invalid, Input, Size));
				}
				// `lengthSizeMinusOne` occupies the low two bits of byte 4; a value of 2, naming a
				// three-byte length, is forbidden by ISO/IEC 14496-15 §5.3.3.1.
				let n = (rec[4] & 0x03) as usize + 1;
				if n == 3 {
					return Err(err!(
						"The AVC decoder configuration record names a NAL length of 3 bytes, which \
						ISO/IEC 14496-15 does not allow; it must be 1, 2 or 4.";
					Invalid, Input));
				}
				Ok(n)
			},
		}
	}

	/// Checks the configuration record is well formed, and gives the frame geometry it codes.
	///
	/// The record is walked rather than trusted, because every field of the sample table below is
	/// derived from it and a truncated record produces a file that is well formed and unplayable.
	fn geometry(&self) -> Outcome<(u16, u16)> {
		match self {
			Self::Avc(rec) => {
				let _ = res!(self.nal_len());
				if rec[0] != 1 {
					return Err(err!(
						"The AVC decoder configuration record's version is {}, and 1 is the only \
						one ISO/IEC 14496-15 defines.", rec[0];
					Invalid, Input));
				}
				// Byte 5's top three bits are reserved and set, and its low five hold the count of
				// sequence parameter sets.
				let n_sps = (rec[5] & 0x1F) as usize;
				if n_sps == 0 {
					return Err(err!(
						"The AVC decoder configuration record carries no sequence parameter set, \
						so the frame geometry it should describe is absent.";
					Invalid, Input, Missing));
				}
				let mut at = 6usize;
				let mut first: Option<&[u8]> = None;
				for i in 0..n_sps {
					if at + 2 > rec.len() {
						return Err(err!(
							"The AVC decoder configuration record ends {} bytes into a run of {} \
							sequence parameter sets, before the length of set {}.",
							at, n_sps, i;
						Invalid, Input, Size));
					}
					let len = ((rec[at] as usize) << 8) | rec[at + 1] as usize;
					at += 2;
					if at + len > rec.len() {
						return Err(err!(
							"Sequence parameter set {} of the AVC decoder configuration record \
							claims {} bytes but only {} remain.", i, len, rec.len() - at;
						Invalid, Input, Size));
					}
					if first.is_none() {
						first = Some(&rec[at..at + len]);
					}
					at += len;
				}
				if at >= rec.len() {
					return Err(err!(
						"The AVC decoder configuration record ends after its sequence parameter \
						sets, before the count of picture parameter sets.";
					Invalid, Input, Size));
				}
				let n_pps = rec[at] as usize;
				at += 1;
				if n_pps == 0 {
					return Err(err!(
						"The AVC decoder configuration record carries no picture parameter set.";
					Invalid, Input, Missing));
				}
				for i in 0..n_pps {
					if at + 2 > rec.len() {
						return Err(err!(
							"The AVC decoder configuration record ends before the length of \
							picture parameter set {}.", i;
						Invalid, Input, Size));
					}
					let len = ((rec[at] as usize) << 8) | rec[at + 1] as usize;
					at += 2;
					if at + len > rec.len() {
						return Err(err!(
							"Picture parameter set {} of the AVC decoder configuration record \
							claims {} bytes but only {} remain.", i, len, rec.len() - at;
						Invalid, Input, Size));
					}
					at += len;
				}
				let sps = match first {
					Some(s)	=> s,
					None	=> return Err(err!(
						"The first sequence parameter set was not taken."; Bug, Unreachable)),
				};
				sps_geometry(sps)
			},
		}
	}

	/// Checks that a sample's bytes are exactly tiled by length-prefixed NAL units.
	///
	/// The tiling is the test, and a start code at the front is only a diagnosis of why it failed.
	/// It cannot be the test: a four-byte length between `0x00000100` and `0x000001FF` -- any NAL
	/// between 256 and 511 bytes, which is a great many of them -- has the same first three bytes
	/// as a three-byte start code, so a sample refused on that alone would be a perfectly good one.
	/// A sample that genuinely does not tile is nearly always Annex B, an elementary stream
	/// separated by start codes rather than lengths, handed straight through; that produces a file
	/// every demuxer accepts and no decoder plays, so it is worth naming.
	fn check_sample(&self, i: usize, data: &[u8]) -> Outcome<()> {
		let n = res!(self.nal_len());
		let mut at = 0usize;
		let mut nals = 0usize;
		let mut why: Option<String> = None;
		while at < data.len() {
			if at + n > data.len() {
				why = Some(fmt!(
					"it ends {} bytes into a {}-byte length field, after {} whole NAL units",
					data.len() - at, n, nals));
				break;
			}
			let mut len = 0usize;
			for k in 0..n {
				len = (len << 8) | data[at + k] as usize;
			}
			at += n;
			if len == 0 {
				why = Some(fmt!("NAL unit {} declares a length of zero", nals));
				break;
			}
			if at + len > data.len() {
				why = Some(fmt!(
					"NAL unit {} declares {} bytes and only {} remain",
					nals, len, data.len() - at));
				break;
			}
			at += len;
			nals += 1;
		}
		if why.is_none() && nals == 0 {
			why = Some(fmt!("it carries no NAL units at all"));
		}
		match why {
			None		=> Ok(()),
			Some(why)	=> {
				let start = data.len() >= 4 && data[0] == 0 && data[1] == 0
					&& (data[2] == 1 || (data[2] == 0 && data[3] == 1));
				if start {
					Err(err!(
						"Sample {} is not tiled by {}-byte NAL length prefixes -- {} -- and it \
						begins with an Annex B start code, so it is most likely an elementary \
						stream handed over without conversion.", i, n, why;
					Invalid, Input, Mismatch))
				} else {
					Err(err!(
						"Sample {} is not tiled by {}-byte NAL length prefixes: {}.", i, n, why;
					Invalid, Input, Size))
				}
			},
		}
	}
}

/// A video track, written as a whole MP4: samples pushed one at a time, and the file's bytes taken
/// at the end.
///
/// One track is the whole of what this writes today, so the track is the file. A second track --
/// narration, which wants its own timescale and its own sample table beside this one -- is a
/// `Movie` taking two of these, and is not built until there is audio to put in it.
///
/// # Why the samples are held
///
/// The sample table states every sample's size, its duration and the file offset of the chunk it
/// sits in, and the offsets are absolute, so none of them is known until the size of the table
/// itself is. Holding the samples is what buys an exact table and a `moov` that precedes the media,
/// which is the layout a reader can start playing before the download finishes.
pub struct Track {
	/// Frame width in pixels.
	w:		u16,
	/// Frame height in pixels.
	h:		u16,
	/// Ticks a second, in which every sample duration is expressed.
	timescale:	u32,
	/// The codec and its decoder configuration.
	codec:		Codec,
	/// The samples, in decode order, which for a track without B-pictures is also display order.
	samples:	Vec<Sample>,
	/// The total size of the samples in bytes, kept as they arrive.
	bytes:		u64,
	/// The total duration in the track's timescale, kept as they arrive.
	ticks:		u64,
}

impl Track {

	/// Begins a video track of the given size and timescale, coded as the given codec says.
	///
	/// The dimensions are checked against the geometry coded in the decoder configuration's
	/// sequence parameter set, and a disagreement is refused: the two describe the same pictures,
	/// and where they differ it is the caller's bookkeeping that is wrong, not the stream's.
	pub fn new(w: u16, h: u16, timescale: u32, codec: Codec) -> Outcome<Self> {
		if w == 0 || h == 0 {
			return Err(err!(
				"A track of {} by {} pixels has no picture in it.", w, h;
			Invalid, Input, Range));
		}
		if timescale == 0 {
			return Err(err!(
				"A timescale of zero ticks a second names no rate, so no sample duration written \
				against it would mean anything.";
			Invalid, Input, Range));
		}
		let (cw, ch) = res!(codec.geometry());
		if cw != w || ch != h {
			return Err(err!(
				"The track is declared {} by {} pixels, but the sequence parameter set in the \
				decoder configuration codes {} by {}.", w, h, cw, ch;
			Invalid, Input, Mismatch));
		}
		Ok(Self {
			w,
			h,
			timescale,
			codec,
			samples:	Vec::new(),
			bytes:		0,
			ticks:		0,
		})
	}

	/// Adds a sample to the end of the track.
	pub fn push(&mut self, s: Sample) -> Outcome<()> {
		let i = self.samples.len();
		if i >= MAX_SAMPLES {
			return Err(err!(
				"A track may hold {} samples, and this is sample {}.", MAX_SAMPLES, i + 1;
			Invalid, Input, Excessive));
		}
		if s.dur == 0 {
			return Err(err!(
				"Sample {} is given a duration of zero ticks, so it is shown for no time at all.",
				i;
			Invalid, Input, Range));
		}
		if s.data.is_empty() {
			return Err(err!("Sample {} carries no bytes.", i; Invalid, Input, Missing));
		}
		res!(self.codec.check_sample(i, &s.data));
		self.bytes += s.data.len() as u64;
		self.ticks += s.dur as u64;
		self.samples.push(s);
		Ok(())
	}

	/// The number of samples pushed so far.
	pub fn samples(&self) -> usize {
		self.samples.len()
	}

	/// Whether no sample has been pushed.
	pub fn is_empty(&self) -> bool {
		self.samples.is_empty()
	}

	/// The total duration so far, in the track's own timescale.
	pub fn duration(&self) -> u64 {
		self.ticks
	}

	/// The total size of the samples so far, in bytes.
	pub fn media_bytes(&self) -> u64 {
		self.bytes
	}

	/// Finishes the track and gives the file's bytes.
	pub fn finish(self) -> Outcome<Vec<u8>> {
		if self.samples.is_empty() {
			return Err(err!(
				"A track must hold at least one sample, and none were pushed.";
			Invalid, Input, Missing));
		}
		if !self.samples[0].sync {
			return Err(err!(
				"Sample 0 is not a sync sample, so there is nowhere in the track a reader may \
				begin decoding.";
			Invalid, Input));
		}
		if self.ticks > u32::MAX as u64 {
			return Err(err!(
				"The track runs {} ticks at {} a second, which will not fit the 32-bit duration a \
				version-0 media header carries.", self.ticks, self.timescale;
			Invalid, Input, Excessive));
		}
		let movie_ticks = res!(rescale(self.ticks, self.timescale, MOVIE_TIMESCALE));
		if movie_ticks > u32::MAX as u64 {
			return Err(err!(
				"The track runs {} milliseconds, which will not fit the 32-bit duration a \
				version-0 movie header carries.", movie_ticks;
			Invalid, Input, Excessive));
		}

		let ftyp = res!(ftyp());
		// The `mdat` header is eight bytes, unless the media will not fit a 32-bit size, in which
		// case ISO/IEC 14496-12 §4.2 puts a 64-bit `largesize` after the type and writes 1 in the
		// size field.
		let mdat_hdr = if self.bytes + 8 > u32::MAX as u64 { 16u64 } else { 8u64 };

		// Two passes. The first sizes the index with a 32-bit offset table and zeroed offsets; if
		// the last byte of media then falls beyond what a 32-bit offset can name, the second uses
		// the 64-bit table instead. Widening the table only pushes the media further out, so this
		// settles after one look.
		let probe = res!(self.moov(0, false));
		let head = ftyp.len() as u64 + probe.len() as u64 + mdat_hdr;
		let wide = head + self.bytes > u32::MAX as u64;
		let index = if wide {
			let probe64 = res!(self.moov(0, true));
			let head64 = ftyp.len() as u64 + probe64.len() as u64 + mdat_hdr;
			res!(self.moov(head64, true))
		} else {
			res!(self.moov(head, false))
		};

		// The rebuilt index must be the size the offsets were computed against, or every one of
		// them is wrong by the difference. The table's entries are a fixed width, so this holds by
		// construction; it is asserted rather than assumed because nothing downstream would catch
		// it.
		let probe_len = if wide { res!(self.moov(0, true)).len() } else { probe.len() };
		if index.len() != probe_len {
			return Err(err!(
				"The movie index came to {} bytes when sized and {} bytes when written, so the \
				chunk offsets in it are wrong by {}.",
				probe_len, index.len(), index.len() as i64 - probe_len as i64;
			Bug, Unreachable));
		}

		let total = ftyp.len() as u64 + index.len() as u64 + mdat_hdr + self.bytes;
		let mut out = Vec::with_capacity(total as usize);
		out.extend_from_slice(&ftyp);
		out.extend_from_slice(&index);
		if mdat_hdr == 16 {
			out.extend_from_slice(&1u32.to_be_bytes());
			out.extend_from_slice(b"mdat");
			out.extend_from_slice(&(self.bytes + 16).to_be_bytes());
		} else {
			out.extend_from_slice(&((self.bytes + 8) as u32).to_be_bytes());
			out.extend_from_slice(b"mdat");
		}
		for s in &self.samples {
			out.extend_from_slice(&s.data);
		}
		Ok(out)
	}

	/// The `moov` box, with the chunk offsets placed against a media that begins at `base`.
	fn moov(&self, base: u64, wide: bool) -> Outcome<Vec<u8>> {
		let movie_ticks = res!(rescale(self.ticks, self.timescale, MOVIE_TIMESCALE)) as u32;
		let mut body = Vec::new();
		body.extend_from_slice(&res!(self.mvhd(movie_ticks)));
		body.extend_from_slice(&res!(self.trak(movie_ticks, base, wide)));
		bx(b"moov", &body)
	}

	/// The movie header, ISO/IEC 14496-12 §8.2.2, in its version-0 form with 32-bit times.
	fn mvhd(&self, movie_ticks: u32) -> Outcome<Vec<u8>> {
		let mut b = Vec::with_capacity(100);
		b.extend_from_slice(&full(0, 0));
		b.extend_from_slice(&0u32.to_be_bytes());	// Creation time, unset.
		b.extend_from_slice(&0u32.to_be_bytes());	// Modification time, unset.
		b.extend_from_slice(&MOVIE_TIMESCALE.to_be_bytes());
		b.extend_from_slice(&movie_ticks.to_be_bytes());
		b.extend_from_slice(&0x0001_0000u32.to_be_bytes());	// Rate: 1.0 in 16.16.
		b.extend_from_slice(&0x0100u16.to_be_bytes());		// Volume: 1.0 in 8.8.
		b.extend_from_slice(&0u16.to_be_bytes());		// Reserved.
		b.extend_from_slice(&[0u8; 8]);				// Reserved.
		for v in UNITY {
			b.extend_from_slice(&v.to_be_bytes());
		}
		b.extend_from_slice(&[0u8; 24]);		// Pre-defined.
		b.extend_from_slice(&2u32.to_be_bytes());	// Next track ID: one past the only track.
		bx(b"mvhd", &b)
	}

	/// The track box: its header and its media.
	fn trak(&self, movie_ticks: u32, base: u64, wide: bool) -> Outcome<Vec<u8>> {
		let mut body = Vec::new();
		body.extend_from_slice(&res!(self.tkhd(movie_ticks)));
		body.extend_from_slice(&res!(self.mdia(base, wide)));
		bx(b"trak", &body)
	}

	/// The track header, ISO/IEC 14496-12 §8.3.2, version 0.
	///
	/// The flags are `0x000007`: enabled, in the movie, and in the preview. A track written without
	/// `track_enabled` is present in the file and played by nothing.
	fn tkhd(&self, movie_ticks: u32) -> Outcome<Vec<u8>> {
		let mut b = Vec::with_capacity(84);
		b.extend_from_slice(&full(0, 0x0000_0007));
		b.extend_from_slice(&0u32.to_be_bytes());	// Creation time, unset.
		b.extend_from_slice(&0u32.to_be_bytes());	// Modification time, unset.
		b.extend_from_slice(&1u32.to_be_bytes());	// Track ID; zero is not allowed.
		b.extend_from_slice(&0u32.to_be_bytes());	// Reserved.
		b.extend_from_slice(&movie_ticks.to_be_bytes());
		b.extend_from_slice(&[0u8; 8]);			// Reserved.
		b.extend_from_slice(&0u16.to_be_bytes());	// Layer: the front.
		b.extend_from_slice(&0u16.to_be_bytes());	// Alternate group: no alternatives.
		b.extend_from_slice(&0u16.to_be_bytes());	// Volume, which is zero for a visual track.
		b.extend_from_slice(&0u16.to_be_bytes());	// Reserved.
		for v in UNITY {
			b.extend_from_slice(&v.to_be_bytes());
		}
		// The presentation size, in 16.16 fixed point. It is the size the track is drawn at, which
		// need not be the coded size the sample entry carries -- an anamorphic stream differs in
		// exactly this field -- but for square pixels the two agree.
		b.extend_from_slice(&((self.w as u32) << 16).to_be_bytes());
		b.extend_from_slice(&((self.h as u32) << 16).to_be_bytes());
		bx(b"tkhd", &b)
	}

	/// The media box: the media header, the handler, and the media information.
	fn mdia(&self, base: u64, wide: bool) -> Outcome<Vec<u8>> {
		let mut body = Vec::new();
		body.extend_from_slice(&res!(self.mdhd()));
		body.extend_from_slice(&res!(hdlr()));
		body.extend_from_slice(&res!(self.minf(base, wide)));
		bx(b"mdia", &body)
	}

	/// The media header, ISO/IEC 14496-12 §8.4.2, version 0, carrying the track's own timescale.
	fn mdhd(&self) -> Outcome<Vec<u8>> {
		let mut b = Vec::with_capacity(24);
		b.extend_from_slice(&full(0, 0));
		b.extend_from_slice(&0u32.to_be_bytes());	// Creation time, unset.
		b.extend_from_slice(&0u32.to_be_bytes());	// Modification time, unset.
		b.extend_from_slice(&self.timescale.to_be_bytes());
		b.extend_from_slice(&(self.ticks as u32).to_be_bytes());
		b.extend_from_slice(&LANG_UND.to_be_bytes());
		b.extend_from_slice(&0u16.to_be_bytes());	// Pre-defined.
		bx(b"mdhd", &b)
	}

	/// The media information box: the video media header, where the media lives, and the sample
	/// table.
	fn minf(&self, base: u64, wide: bool) -> Outcome<Vec<u8>> {
		let mut body = Vec::new();
		body.extend_from_slice(&res!(vmhd()));
		body.extend_from_slice(&res!(dinf()));
		body.extend_from_slice(&res!(self.stbl(base, wide)));
		bx(b"minf", &body)
	}

	/// The sample table: what the samples are, how long each lasts, how big it is, which chunk it
	/// is in, where the chunks are, and which samples a reader may begin at.
	fn stbl(&self, base: u64, wide: bool) -> Outcome<Vec<u8>> {
		let mut body = Vec::new();
		body.extend_from_slice(&res!(self.stsd()));
		body.extend_from_slice(&res!(self.stts()));
		body.extend_from_slice(&res!(self.stsz()));
		body.extend_from_slice(&res!(stsc()));
		body.extend_from_slice(&res!(self.offsets(base, wide)));
		if let Some(stss) = res!(self.stss()) {
			body.extend_from_slice(&stss);
		}
		bx(b"stbl", &body)
	}

	/// The sample description: one entry, describing every sample in the track.
	fn stsd(&self) -> Outcome<Vec<u8>> {
		let mut b = Vec::new();
		b.extend_from_slice(&full(0, 0));
		b.extend_from_slice(&1u32.to_be_bytes());	// Entry count.
		b.extend_from_slice(&res!(self.entry()));
		bx(b"stsd", &b)
	}

	/// The visual sample entry, ISO/IEC 14496-12 §8.5.2, carrying the codec's configuration box.
	fn entry(&self) -> Outcome<Vec<u8>> {
		let mut b = Vec::with_capacity(86);
		b.extend_from_slice(&[0u8; 6]);			// Reserved.
		b.extend_from_slice(&1u16.to_be_bytes());	// Data reference index: the first `dref` entry.
		b.extend_from_slice(&0u16.to_be_bytes());	// Pre-defined.
		b.extend_from_slice(&0u16.to_be_bytes());	// Reserved.
		b.extend_from_slice(&[0u8; 12]);		// Pre-defined.
		b.extend_from_slice(&self.w.to_be_bytes());
		b.extend_from_slice(&self.h.to_be_bytes());
		b.extend_from_slice(&RESOLUTION_72.to_be_bytes());
		b.extend_from_slice(&RESOLUTION_72.to_be_bytes());
		b.extend_from_slice(&0u32.to_be_bytes());	// Reserved.
		b.extend_from_slice(&1u16.to_be_bytes());	// Frames a sample: one coded picture each.
		// A fixed 32-byte field holding a counted string: a length byte, then that many bytes of
		// name, then padding. Not a null-terminated string, and writing one there is a common way
		// to put rubbish in front of a viewer that displays the field.
		let name = b"AVC Coding";
		let mut cname = [0u8; 32];
		cname[0] = name.len() as u8;
		cname[1..1 + name.len()].copy_from_slice(name);
		b.extend_from_slice(&cname);
		b.extend_from_slice(&0x0018u16.to_be_bytes());	// Depth: colour with no alpha.
		b.extend_from_slice(&0xFFFFu16.to_be_bytes());	// Pre-defined: -1.
		b.extend_from_slice(&res!(bx(self.codec.config(), self.codec.record())));
		bx(self.codec.entry(), &b)
	}

	/// The decoding time to sample table, ISO/IEC 14496-12 §8.6.1.2, run-length coded.
	///
	/// Consecutive samples of equal duration share one entry, so a track at a constant frame rate
	/// has exactly one entry however many frames it holds.
	fn stts(&self) -> Outcome<Vec<u8>> {
		let mut runs: Vec<(u32, u32)> = Vec::new();
		for s in &self.samples {
			match runs.last_mut() {
				Some((n, d)) if *d == s.dur	=> *n += 1,
				_				=> runs.push((1, s.dur)),
			}
		}
		let mut b = Vec::with_capacity(8 + runs.len() * 8);
		b.extend_from_slice(&full(0, 0));
		b.extend_from_slice(&(runs.len() as u32).to_be_bytes());
		for (n, d) in runs {
			b.extend_from_slice(&n.to_be_bytes());
			b.extend_from_slice(&d.to_be_bytes());
		}
		bx(b"stts", &b)
	}

	/// The sample size table, ISO/IEC 14496-12 §8.7.3.2.
	///
	/// The common size field is written as zero, meaning the sizes vary and are listed one by one.
	/// A coded picture stream where every sample is the same length would be a remarkable
	/// coincidence, so the branch that would save the table is not taken.
	fn stsz(&self) -> Outcome<Vec<u8>> {
		let mut b = Vec::with_capacity(12 + self.samples.len() * 4);
		b.extend_from_slice(&full(0, 0));
		b.extend_from_slice(&0u32.to_be_bytes());	// Sizes vary.
		b.extend_from_slice(&(self.samples.len() as u32).to_be_bytes());
		for s in &self.samples {
			if s.data.len() > u32::MAX as usize {
				return Err(err!(
					"A sample of {} bytes will not fit the 32-bit size a sample size table holds.",
					s.data.len();
				Invalid, Input, Excessive));
			}
			b.extend_from_slice(&(s.data.len() as u32).to_be_bytes());
		}
		bx(b"stsz", &b)
	}

	/// The chunk offset table, as either the 32-bit `stco` or the 64-bit `co64` of ISO/IEC
	/// 14496-12 §8.7.5.
	///
	/// One sample a chunk. That costs four bytes a sample over packing the whole track into one
	/// chunk, and it buys a table a second track can be interleaved into without the first being
	/// rewritten -- which is what adding narration will want.
	fn offsets(&self, base: u64, wide: bool) -> Outcome<Vec<u8>> {
		let n = self.samples.len();
		let mut b = Vec::with_capacity(8 + n * if wide { 8 } else { 4 });
		b.extend_from_slice(&full(0, 0));
		b.extend_from_slice(&(n as u32).to_be_bytes());
		let mut at = base;
		for s in &self.samples {
			if wide {
				b.extend_from_slice(&at.to_be_bytes());
			} else {
				if at > u32::MAX as u64 {
					return Err(err!(
						"A chunk begins at byte {}, beyond what the 32-bit offset table can name.",
						at;
					Invalid, Input, Excessive));
				}
				b.extend_from_slice(&(at as u32).to_be_bytes());
			}
			at += s.data.len() as u64;
		}
		bx(if wide { b"co64" } else { b"stco" }, &b)
	}

	/// The sync sample table, ISO/IEC 14496-12 §8.6.2, or `None` where every sample is a sync
	/// sample.
	///
	/// Its absence is not an omission: the specification says that where there is no sync sample
	/// box, every sample is a sync sample. Writing one that lists all of them says the same thing
	/// at four bytes a frame.
	fn stss(&self) -> Outcome<Option<Vec<u8>>> {
		if self.samples.iter().all(|s| s.sync) {
			return Ok(None);
		}
		let keys: Vec<u32> = self.samples.iter()
			.enumerate()
			.filter(|(_, s)| s.sync)
			.map(|(i, _)| i as u32 + 1)	// Sample numbers are one-based.
			.collect();
		let mut b = Vec::with_capacity(8 + keys.len() * 4);
		b.extend_from_slice(&full(0, 0));
		b.extend_from_slice(&(keys.len() as u32).to_be_bytes());
		for k in keys {
			b.extend_from_slice(&k.to_be_bytes());
		}
		Ok(Some(res!(bx(b"stss", &b))))
	}
}

/// The file type box, ISO/IEC 14496-12 §4.3.
///
/// `isom` as the major brand with a minor version of 512 is what the reference muxers write, and
/// the compatible brands list `isom`, `iso2`, `avc1` and `mp41`: a reader that knows only the AVC
/// file format, and one that knows only version 1 of MP4, can both see a brand they recognise.
fn ftyp() -> Outcome<Vec<u8>> {
	let mut b = Vec::with_capacity(24);
	b.extend_from_slice(b"isom");
	b.extend_from_slice(&512u32.to_be_bytes());
	b.extend_from_slice(b"isom");
	b.extend_from_slice(b"iso2");
	b.extend_from_slice(b"avc1");
	b.extend_from_slice(b"mp41");
	bx(b"ftyp", &b)
}

/// The handler reference, ISO/IEC 14496-12 §8.4.3, declaring a visual track.
///
/// The trailing name is a null-terminated UTF-8 string. A counted string is written there by some
/// tools, following an older convention, and a reader that takes the specification at its word then
/// shows the count byte as the first character of the name.
fn hdlr() -> Outcome<Vec<u8>> {
	let mut b = Vec::with_capacity(33);
	b.extend_from_slice(&full(0, 0));
	b.extend_from_slice(&0u32.to_be_bytes());	// Pre-defined.
	b.extend_from_slice(b"vide");			// Handler type: visual.
	b.extend_from_slice(&[0u8; 12]);		// Reserved.
	b.extend_from_slice(b"VideoHandler\0");
	bx(b"hdlr", &b)
}

/// The video media header, ISO/IEC 14496-12 §8.4.5.2.
///
/// Its flags must be 1, which the specification states outright and gives no meaning for; a zero
/// there is rejected by some readers.
fn vmhd() -> Outcome<Vec<u8>> {
	let mut b = Vec::with_capacity(12);
	b.extend_from_slice(&full(0, 1));
	b.extend_from_slice(&0u16.to_be_bytes());	// Graphics mode: copy.
	b.extend_from_slice(&[0u8; 6]);			// Operation colour, unused by copy.
	bx(b"vmhd", &b)
}

/// The data information box: where the media of this track is to be found.
///
/// One `url ` entry with the self-contained flag set, meaning the media is in this file. The
/// four-character code has a trailing space, which is not a typographic accident.
fn dinf() -> Outcome<Vec<u8>> {
	let url = res!(bx(b"url ", &full(0, 1)));
	let mut dref = Vec::with_capacity(8 + url.len());
	dref.extend_from_slice(&full(0, 0));
	dref.extend_from_slice(&1u32.to_be_bytes());	// Entry count.
	dref.extend_from_slice(&url);
	let dref = res!(bx(b"dref", &dref));
	bx(b"dinf", &dref)
}

/// The sample to chunk table, ISO/IEC 14496-12 §8.7.4, for one sample a chunk.
///
/// A single entry: from chunk 1 onward, one sample a chunk, described by sample description 1.
/// Entries are only written where the run changes, so one entry covers every chunk in the track.
fn stsc() -> Outcome<Vec<u8>> {
	let mut b = Vec::with_capacity(20);
	b.extend_from_slice(&full(0, 0));
	b.extend_from_slice(&1u32.to_be_bytes());	// Entry count.
	b.extend_from_slice(&1u32.to_be_bytes());	// First chunk; one-based.
	b.extend_from_slice(&1u32.to_be_bytes());	// Samples a chunk.
	b.extend_from_slice(&1u32.to_be_bytes());	// Sample description index; one-based.
	bx(b"stsc", &b)
}

/// Wraps a payload in a box header: a 32-bit size covering the whole box, then its four-character
/// type. ISO/IEC 14496-12 §4.2.
fn bx(kind: &[u8; 4], body: &[u8]) -> Outcome<Vec<u8>> {
	let size = body.len() + 8;
	if size > u32::MAX as usize {
		return Err(err!(
			"The '{}' box comes to {} bytes, which will not fit the 32-bit size a box header \
			carries; only 'mdat' is written in the 64-bit form.",
			String::from_utf8_lossy(kind), size;
		Invalid, Input, Excessive));
	}
	let mut out = Vec::with_capacity(size);
	out.extend_from_slice(&(size as u32).to_be_bytes());
	out.extend_from_slice(kind);
	out.extend_from_slice(body);
	Ok(out)
}

/// The version and flags a full box begins with: one byte of version, then three of flags. ISO/IEC
/// 14496-12 §4.2.
fn full(ver: u8, flags: u32) -> [u8; 4] {
	let f = flags.to_be_bytes();
	[ver, f[1], f[2], f[3]]
}

/// A duration counted in one timescale, expressed in another, rounded to nearest.
fn rescale(ticks: u64, from: u32, to: u32) -> Outcome<u64> {
	if from == 0 {
		return Err(err!("A duration cannot be rescaled from a timescale of zero."; Invalid, Input));
	}
	let n = (ticks as u128) * (to as u128) + (from as u128) / 2;
	Ok((n / from as u128) as u64)
}

/// The raw byte sequence payload of a NAL unit: its body with the emulation prevention bytes taken
/// out, so that a `00 00 03` written to keep a start code out of the stream reads back as the
/// `00 00` it stands for. ITU-T H.264 §7.4.1.
fn rbsp(nal: &[u8]) -> Vec<u8> {
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

/// A reader of the bits of an RBSP, most significant first, which is how H.264 codes its syntax
/// elements.
struct Bits<'a> {
	/// The bytes being read.
	buf:	&'a [u8],
	/// The next bit, counted from the first bit of the first byte.
	pos:	usize,
}

impl<'a> Bits<'a> {

	/// A reader positioned at the first bit.
	fn new(buf: &'a [u8]) -> Self {
		Self { buf, pos: 0 }
	}

	/// The next `n` bits as an unsigned integer, most significant first.
	fn u(&mut self, n: usize) -> Outcome<u32> {
		if n > 32 {
			return Err(err!("A field of {} bits was asked for, and 32 is the widest.", n; Bug));
		}
		let mut v = 0u32;
		for _ in 0..n {
			let byte = self.pos >> 3;
			if byte >= self.buf.len() {
				return Err(err!(
					"The sequence parameter set ends after {} bits, before its frame geometry was \
					read.", self.buf.len() * 8;
				Invalid, Input, Decode));
			}
			let bit = (self.buf[byte] >> (7 - (self.pos & 7))) & 1;
			v = (v << 1) | bit as u32;
			self.pos += 1;
		}
		Ok(v)
	}

	/// The next bit as a flag.
	fn flag(&mut self) -> Outcome<bool> {
		Ok(res!(self.u(1)) == 1)
	}

	/// An unsigned Exp-Golomb code, ITU-T H.264 §9.1.
	fn ue(&mut self) -> Outcome<u32> {
		let mut zeros = 0usize;
		while res!(self.u(1)) == 0 {
			zeros += 1;
			if zeros > 31 {
				return Err(err!(
					"An Exp-Golomb code in the sequence parameter set is prefixed by more than 31 \
					zeroes, which no legal value is.";
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
				"An Exp-Golomb code in the sequence parameter set decodes to {}, beyond what any \
				of its fields may hold.", v;
			Invalid, Input, Decode));
		}
		Ok(v as u32)
	}

	/// A signed Exp-Golomb code, ITU-T H.264 §9.1.1.
	fn se(&mut self) -> Outcome<i32> {
		let k = res!(self.ue());
		let m = ((k as i64 + 1) / 2) as i32;
		Ok(if k % 2 == 1 { m } else { -m })
	}
}

/// Steps over a scaling list without keeping it, ITU-T H.264 §7.3.2.1.1.1.
///
/// The list has to be walked rather than skipped by a byte count, because it is coded as a run of
/// variable-length differences and its end is only found by decoding all of them.
fn skip_scaling_list(b: &mut Bits, size: usize) -> Outcome<()> {
	let mut last = 8i32;
	let mut next = 8i32;
	for _ in 0..size {
		if next != 0 {
			let delta = res!(b.se());
			next = (last + delta + 256).rem_euclid(256);
		}
		last = if next == 0 { last } else { next };
	}
	Ok(())
}

/// The frame width and height coded in a sequence parameter set, in pixels.
///
/// This reads the geometry and nothing else: the macroblock counts, whether the stream is coded in
/// frames or in fields, and the cropping window. It is not a decoder and does not become one -- the
/// point of it is that the caller's declared dimensions can be checked against the stream's own,
/// rather than written into the container on trust.
///
/// The frame is `(pic_width_in_mbs_minus1 + 1) * 16` wide before cropping, and
/// `(2 - frame_mbs_only_flag) * (pic_height_in_map_units_minus1 + 1) * 16` high, and the crop
/// offsets are then subtracted in units of the chroma sampling, per ITU-T H.264 §7.4.2.1.1.
fn sps_geometry(sps: &[u8]) -> Outcome<(u16, u16)> {
	if sps.is_empty() {
		return Err(err!("The sequence parameter set is empty."; Invalid, Input, Missing));
	}
	let kind = sps[0] & 0x1F;
	if kind != 7 {
		return Err(err!(
			"The first parameter set in the decoder configuration is NAL unit type {}, and a \
			sequence parameter set is type 7.", kind;
		Invalid, Input, Mismatch));
	}
	let body = rbsp(&sps[1..]);
	let mut b = Bits::new(&body);

	let profile = res!(b.u(8));
	let _constraints = res!(b.u(8));
	let _level = res!(b.u(8));
	let _sps_id = res!(b.ue());

	// The profiles that carry a chroma format and scaling lists in the parameter set, ITU-T H.264
	// §7.3.2.1.1. Every other profile is 4:2:0 with no lists.
	let mut chroma = 1u32;
	let mut separate_planes = false;
	if matches!(profile, 100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135) {
		chroma = res!(b.ue());
		if chroma == 3 {
			separate_planes = res!(b.flag());
		}
		let _bit_depth_luma = res!(b.ue());
		let _bit_depth_chroma = res!(b.ue());
		let _qpprime_bypass = res!(b.flag());
		if res!(b.flag()) {
			let lists = if chroma != 3 { 8 } else { 12 };
			for i in 0..lists {
				if res!(b.flag()) {
					res!(skip_scaling_list(&mut b, if i < 6 { 16 } else { 64 }));
				}
			}
		}
	}

	let _log2_max_frame_num = res!(b.ue());
	let poc_type = res!(b.ue());
	match poc_type {
		0 => {
			let _log2_max_poc_lsb = res!(b.ue());
		},
		1 => {
			let _delta_always_zero = res!(b.flag());
			let _offset_non_ref = res!(b.se());
			let _offset_top_bottom = res!(b.se());
			let cycle = res!(b.ue());
			if cycle > 255 {
				return Err(err!(
					"The sequence parameter set names {} entries in its picture order count cycle, \
					and 255 is the most allowed.", cycle;
				Invalid, Input, Decode));
			}
			for _ in 0..cycle {
				let _offset = res!(b.se());
			}
		},
		2 => {},
		other => return Err(err!(
			"The sequence parameter set names picture order count type {}, and 0, 1 and 2 are the \
			only ones defined.", other;
		Invalid, Input, Decode)),
	}

	let _max_ref_frames = res!(b.ue());
	let _gaps_allowed = res!(b.flag());
	let mbs_wide = res!(b.ue()) as u64 + 1;
	let map_units_high = res!(b.ue()) as u64 + 1;
	let frame_mbs_only = res!(b.flag());
	if !frame_mbs_only {
		let _mb_adaptive = res!(b.flag());
	}
	let _direct_8x8 = res!(b.flag());

	let (mut left, mut right, mut top, mut bottom) = (0u64, 0u64, 0u64, 0u64);
	if res!(b.flag()) {
		left = res!(b.ue()) as u64;
		right = res!(b.ue()) as u64;
		top = res!(b.ue()) as u64;
		bottom = res!(b.ue()) as u64;
	}

	// The crop offsets are counted in chroma samples, so they scale by the chroma subsampling.
	// Monochrome, and a stream whose colour planes are coded separately, crop in luma samples.
	let (sub_w, sub_h) = match chroma {
		0 => (1u64, 1u64),
		1 => (2, 2),
		2 => (2, 1),
		3 => (1, 1),
		other => return Err(err!(
			"The sequence parameter set names chroma format {}, and 0 to 3 are the only ones \
			defined.", other;
		Invalid, Input, Decode)),
	};
	let (crop_x, crop_y) = if chroma == 0 || separate_planes {
		(1u64, if frame_mbs_only { 1 } else { 2 })
	} else {
		(sub_w, sub_h * if frame_mbs_only { 1 } else { 2 })
	};

	let raw_w = mbs_wide * 16;
	let raw_h = map_units_high * 16 * if frame_mbs_only { 1 } else { 2 };
	let cut_w = crop_x * (left + right);
	let cut_h = crop_y * (top + bottom);
	if cut_w >= raw_w || cut_h >= raw_h {
		return Err(err!(
			"The sequence parameter set crops {} by {} coded pixels down to nothing, taking {} \
			from the width and {} from the height.", raw_w, raw_h, cut_w, cut_h;
		Invalid, Input, Range));
	}
	let w = raw_w - cut_w;
	let h = raw_h - cut_h;
	if w > u16::MAX as u64 || h > u16::MAX as u64 {
		return Err(err!(
			"The sequence parameter set codes a frame of {} by {} pixels, beyond what a visual \
			sample entry can state.", w, h;
		Invalid, Input, Excessive));
	}
	Ok((w as u16, h as u16))
}

// ------------------------------------------------------------------------- reading a film

/// The deepest a box tree may nest before it is called a mistake.
const MAX_DEPTH: usize = 16;

/// Which codec a track's samples are coded in, as its sample entry names it.
///
/// An enum rather than the four-character code, so that a caller matches on a thing the reader has
/// already recognised rather than on a byte string of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
	/// H.264, described by an `avcC` record.
	Avc,
	/// HEVC, described by an `hvcC` record.
	Hevc,
	/// Motion JPEG: every sample is a whole JPEG and there is no configuration record at all.
	Mjpeg,
	/// Something else, carrying its four-character code so that a refusal can name it.
	Other([u8; 4]),
}

impl Kind {

	/// The codec a sample entry's four-character code names.
	fn of(code: [u8; 4]) -> Self {
		match &code {
			// `avc1` carries its parameter sets in the configuration record only; `avc3` may also
			// carry them in the samples. Both are read the same way here, because the sample is
			// walked for parameter sets in either case.
			b"avc1" | b"avc3"	=> Self::Avc,
			b"hvc1" | b"hev1"	=> Self::Hevc,
			b"jpeg" | b"mjpa" | b"mjpb"	=> Self::Mjpeg,
			_			=> Self::Other(code),
		}
	}
}

/// One video track of a film, as its sample table describes it.
///
/// This is the reading half of this module, and it exists for one job: telling a decoder where a
/// film's first coded picture is. It holds an **index and not the film**: the sample table of a
/// four-gigabyte film is a few tens of kilobytes, and a caller that wants one frame of it should
/// never have to hold the other four gigabytes to get it. So the samples are spans, and the bytes
/// they name are the caller's to fetch.
#[derive(Clone, Debug)]
pub struct Film {
	/// Which codec the track is coded in.
	kind:	Kind,
	/// The configuration record out of the sample entry, where the codec has one.
	config:	Vec<u8>,
	/// The coded width the sample entry declares.
	width:	u16,
	/// The coded height it declares.
	height:	u16,
	/// How far the picture is to be turned before it is shown, in degrees clockwise.
	rotation:	u16,
	/// Each sample's offset in the file and its length.
	samples:	Vec<(u64, u32)>,
	/// Which samples a reader may begin decoding at, as `stss` lists them, counted from nought.
	///
	/// Empty means the box was absent, which per ISO/IEC 14496-12 §8.6.2 means **every** sample is
	/// a sync sample -- the opposite of what an empty list would otherwise suggest, and the reason
	/// this is not an `Option` the caller has to remember to check.
	sync:	Vec<u32>,
}

impl Film {

	/// Reads a film's first video track out of a whole file.
	///
	/// A file with no video track, or with one whose sample table is incomplete, is refused: a
	/// track whose samples cannot be located is not a track a picture can be drawn from.
	pub fn read(bytes: &[u8]) -> Outcome<Self> {
		let mut at = 0usize;
		while at + 8 <= bytes.len() {
			let (size, head) = res!(box_head(bytes, at, bytes.len()));
			if &bytes[at + 4..at + 8] == b"moov" {
				return match res!(movie(bytes, at + head, at + size)) {
					Some(f) => Ok(f),
					None => Err(err!(
						"The file's movie box carries no video track. A film needs a track whose \
						handler is `vide`.";
					Invalid, Input, Missing)),
				};
			}
			at += size;
		}
		Err(err!(
			"The file carries no `moov` box, so nothing says where its samples are.";
		Invalid, Input, Missing))
	}

	/// The same, from a `moov` box a caller has lifted out of a file on its own.
	///
	/// This is what a caller with a file handle rather than a buffer uses: the chunk offsets in
	/// `stco` are absolute file offsets, so the index does not depend on where the movie box itself
	/// sat, and a film of any size can be indexed by reading its metadata alone. QuickTime writes
	/// `moov` at the end of the file as often as at the front, so this is not a rare path.
	pub fn from_moov(moov: &[u8]) -> Outcome<Self> {
		match res!(movie(moov, 0, moov.len())) {
			Some(f) => Ok(f),
			None => Err(err!(
				"The movie box carries no video track. A film needs a track whose handler is \
				`vide`.";
			Invalid, Input, Missing)),
		}
	}

	/// Which codec the track is coded in.
	pub fn kind(&self) -> Kind {
		self.kind
	}

	/// The decoder configuration record out of the sample entry: `avcC` or `hvcC`.
	pub fn config(&self) -> &[u8] {
		&self.config
	}

	/// The coded size the sample entry declares, which is not the cropped size the parameter set
	/// implies and should not be shown as though it were.
	pub fn size(&self) -> (u16, u16) {
		(self.width, self.height)
	}

	/// How far the picture is to be turned before it is shown: 0, 90, 180 or 270 degrees clockwise.
	///
	/// A phone writes the angle it was held at into the track header's transformation matrix rather
	/// than turning the samples, so a decoder's output is the picture as it was *coded* and this is
	/// what a viewer must do with it. Ignoring it shows a great many holiday films on their side.
	/// It also hides itself well at ninety degrees, where the turned picture has exactly as many
	/// samples as the untured one.
	pub fn rotation(&self) -> u16 {
		self.rotation
	}

	/// How many samples the track holds.
	pub fn samples(&self) -> usize {
		self.samples.len()
	}

	/// Where one sample sits in the file, and how long it is.
	pub fn span(&self, i: usize) -> Outcome<(u64, u32)> {
		match self.samples.get(i) {
			Some(s) => Ok(*s),
			None => Err(err!(
				"Sample {} was asked for and the track holds {}.", i, self.samples.len();
			Invalid, Input, Range)),
		}
	}

	/// One sample's bytes, out of the file the index was read from.
	pub fn sample<'b>(&self, bytes: &'b [u8], i: usize) -> Outcome<&'b [u8]> {
		let (off, len) = res!(self.span(i));
		let from = off as usize;
		let to = match from.checked_add(len as usize) {
			Some(to) if to <= bytes.len() => to,
			_ => return Err(err!(
				"Sample {} sits at byte {} and is {} long, in a buffer of {}. A file read in part \
				cannot give up its samples.", i, off, len, bytes.len();
			Invalid, Input, Decode)),
		};
		Ok(&bytes[from..to])
	}

	/// Which sample a decoder may begin at.
	///
	/// Almost always the first sample of the track, since a film that cannot be played from its
	/// start is a film no player will open, but a track whose `stss` says otherwise is followed
	/// rather than assumed about.
	pub fn first_sync(&self) -> Outcome<usize> {
		match self.sync.first() {
			Some(n) => Ok(*n as usize),
			// An absent `stss` means every sample is a sync sample.
			None if !self.samples.is_empty() => Ok(0),
			None => Err(err!("The track holds no samples."; Invalid, Input, Missing)),
		}
	}
}

/// Reads a box header, giving its whole length and the length of the header itself.
fn box_head(bytes: &[u8], at: usize, to: usize) -> Outcome<(usize, usize)> {
	if at + 8 > to {
		return Err(err!("A box header runs past the end of its parent."; Invalid, Input, Decode));
	}
	let size = u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
	let (size, head) = match size {
		// A size of one means the real one is the next eight bytes (ISO/IEC 14496-12 §4.2).
		1 => {
			if at + 16 > to {
				return Err(err!(
					"A box says its length is sixty-four bits and its parent ends inside it.";
				Invalid, Input, Decode));
			}
			let mut wide = [0u8; 8];
			wide.copy_from_slice(&bytes[at + 8..at + 16]);
			(u64::from_be_bytes(wide) as usize, 16usize)
		},
		// A size of zero means the box runs to the end of its parent.
		0 => (to - at, 8usize),
		n => (n as usize, 8usize),
	};
	if size < head || at.saturating_add(size) > to {
		return Err(err!(
			"A {} box at byte {} says it is {} bytes long, and its parent ends at {}.",
			String::from_utf8_lossy(&bytes[at + 4..at + 8]), at, size, to;
		Invalid, Input, Decode));
	}
	Ok((size, head))
}

/// Walks the children of a box, handing each one's four-character code and body to a visitor.
///
/// The visitor answers whether the walk should descend into it. Every box is length-prefixed and
/// the lengths have to tile the parent; a box that claims to end beyond its parent is a malformed
/// file, and is refused rather than clamped, since clamping turns one wrong length into a
/// plausible-looking picture.
fn walk<F>(bytes: &[u8], parent: [u8; 4], from: usize, to: usize, depth: usize, visit: &mut F)
	-> Outcome<()>
where
	F: FnMut([u8; 4], [u8; 4], usize, usize) -> Outcome<bool>,
{
	if depth > MAX_DEPTH {
		return Err(err!(
			"The box tree nests more than {} deep, which no legal file does.", MAX_DEPTH;
		Invalid, Input, Decode));
	}
	let mut at = from;
	while at + 8 <= to {
		let (size, head) = res!(box_head(bytes, at, to));
		let mut kind = [0u8; 4];
		kind.copy_from_slice(&bytes[at + 4..at + 8]);
		if res!(visit(kind, parent, at + head, at + size)) {
			res!(walk(bytes, kind, at + head, at + size, depth + 1, visit));
		}
		at += size;
	}
	Ok(())
}

/// Reads a four-byte big-endian number out of a box body.
fn be32(bytes: &[u8], at: usize) -> Outcome<u32> {
	match bytes.get(at..at + 4) {
		Some(s) => Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]])),
		None => Err(err!("A box ends inside a four-byte field."; Invalid, Input, Decode)),
	}
}

/// Reads an eight-byte big-endian number out of a box body.
fn be64(bytes: &[u8], at: usize) -> Outcome<u64> {
	match bytes.get(at..at + 8) {
		Some(s) => Ok(u64::from_be_bytes(
			[s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]])),
		None => Err(err!("A box ends inside an eight-byte field."; Invalid, Input, Decode)),
	}
}

/// The tables one track's sample table holds, before they are turned into sample positions.
#[derive(Default)]
struct Tables {
	/// The handler type: `vide` for a video track.
	handler:	[u8; 4],
	/// Where the sample description sits, to be read once the handler says this is video.
	stsd:		Option<(usize, usize)>,
	/// The codec, from the sample entry.
	kind:		Option<Kind>,
	/// The configuration record's span in the file.
	config:		Option<(usize, usize)>,
	/// The coded size the sample entry declares.
	size:		(u16, u16),
	/// Each sample's length in bytes, from `stsz`.
	sizes:		Vec<u32>,
	/// The runs of `(first_chunk, samples_per_chunk)` from `stsc`, first chunk counted from one.
	runs:		Vec<(u32, u32)>,
	/// Each chunk's offset in the file, from `stco` or `co64`.
	chunks:		Vec<u64>,
	/// The sync sample numbers, counted from one as `stss` writes them.
	sync:		Vec<u32>,
	/// The rotation the track header's matrix codes, in degrees clockwise.
	rotation:	u16,
}

/// Reads the first video track out of a `moov` box.
fn movie(bytes: &[u8], from: usize, to: usize) -> Outcome<Option<Film>> {
	let mut out: Option<Film> = None;
	let mut at = from;
	while at + 8 <= to {
		let (size, head) = res!(box_head(bytes, at, to));
		if &bytes[at + 4..at + 8] == b"trak" {
			let mut t = Tables::default();
			res!(track(bytes, at + head, at + size, &mut t));
			if &t.handler == b"vide" {
				if let Some((body, end)) = t.stsd {
					res!(sample_description(bytes, body, end, &mut t));
				}
				out = Some(res!(assemble(bytes, t)));
				break;
			}
		}
		at += size;
	}
	Ok(out)
}

/// Reads one track's handler and sample table.
fn track(bytes: &[u8], from: usize, to: usize, t: &mut Tables) -> Outcome<()> {
	walk(bytes, *b"trak", from, to, 0, &mut |kind, parent, body, end| {
		match &kind {
			b"mdia" | b"minf" | b"stbl" => return Ok(true),
			b"hdlr" => {
				// version and flags, then a pre-defined word, then the handler type.
				//
				// **Only the one directly inside `mdia`.** QuickTime puts a second `hdlr` inside
				// `minf` naming the *data* handler -- `alis` for a file, `url ` for a reference --
				// and a walk that takes whichever it meets last decides a video track is not one.
				// That is how a 2003 camcorder's film comes to be refused as having no video in it.
				if &parent == b"mdia" {
					if let Some(s) = bytes.get(body + 8..body + 12) {
						t.handler.copy_from_slice(s);
					}
				}
			},
			b"tkhd" => {
				// The transformation matrix sits at a fixed offset that depends on the version:
				// the version-1 header carries 64-bit times and is twelve bytes longer. Only the
				// four rotation entries are read, because a matrix that is not a rotation is not
				// something a photograph library can act on anyway.
				let ver = bytes.get(body).copied().unwrap_or(0);
				let at = body + if ver == 1 { 52 } else { 40 };
				let mut m = [0i32; 4];
				for (i, v) in m.iter_mut().enumerate() {
					*v = res!(be32(bytes, at + i * 4)) as i32;
				}
				// The four are a, b, c, d in 16.16 fixed point.
				let one = 0x0001_0000i32;
				t.rotation = match (m[0], m[1], m[2], m[3]) {
					(0, x, y, 0) if x == one && y == -one	=> 90,
					(x, 0, 0, y) if x == -one && y == -one	=> 180,
					(0, x, y, 0) if x == -one && y == one	=> 270,
					_					=> 0,
				};
			},
			b"stsd" => {
				// Not read here. A track's boxes arrive in whatever order the writer chose, and a
				// sound track's sample entry is not a visual one; reading every `stsd` as though it
				// were refuses a film for the shape of a track nobody asked about.
				t.stsd = Some((body, end));
			},
			b"stsz" => {
				let sample_size = res!(be32(bytes, body + 4));
				let count = res!(be32(bytes, body + 8)) as usize;
				if count > MAX_SAMPLES {
					return Err(err!(
						"A track holds {} samples, and {} is this reader's ceiling.",
						count, MAX_SAMPLES;
					Invalid, Input, Excessive));
				}
				t.sizes = if sample_size != 0 {
					vec![sample_size; count]
				} else {
					let mut v = Vec::with_capacity(count);
					for i in 0..count {
						v.push(res!(be32(bytes, body + 12 + i * 4)));
					}
					v
				};
			},
			b"stsc" => {
				let count = res!(be32(bytes, body + 4)) as usize;
				if count > MAX_SAMPLES {
					return Err(err!(
						"A sample-to-chunk table holds {} runs, and {} is this reader's ceiling.",
						count, MAX_SAMPLES;
					Invalid, Input, Excessive));
				}
				t.runs = Vec::with_capacity(count);
				for i in 0..count {
					let first = res!(be32(bytes, body + 8 + i * 12));
					let per = res!(be32(bytes, body + 12 + i * 12));
					t.runs.push((first, per));
				}
			},
			b"stco" | b"co64" => {
				let wide = &kind == b"co64";
				let count = res!(be32(bytes, body + 4)) as usize;
				if count > MAX_SAMPLES {
					return Err(err!(
						"A chunk offset table holds {} entries, and {} is this reader's ceiling.",
						count, MAX_SAMPLES;
					Invalid, Input, Excessive));
				}
				t.chunks = Vec::with_capacity(count);
				for i in 0..count {
					t.chunks.push(if wide {
						res!(be64(bytes, body + 8 + i * 8))
					} else {
						res!(be32(bytes, body + 8 + i * 4)) as u64
					});
				}
			},
			b"stss" => {
				let count = res!(be32(bytes, body + 4)) as usize;
				if count > MAX_SAMPLES {
					return Err(err!(
						"A sync sample table holds {} entries, and {} is this reader's ceiling.",
						count, MAX_SAMPLES;
					Invalid, Input, Excessive));
				}
				t.sync = Vec::with_capacity(count);
				for i in 0..count {
					t.sync.push(res!(be32(bytes, body + 8 + i * 4)));
				}
			},
			_ => {},
		}
		Ok(false)
	})
}

/// Reads the first sample entry of a sample description, and the configuration record inside it.
fn sample_description(bytes: &[u8], body: usize, end: usize, t: &mut Tables) -> Outcome<()> {
	let count = res!(be32(bytes, body + 4));
	if count == 0 {
		return Ok(());
	}
	let at = body + 8;
	let (size, head) = res!(box_head(bytes, at, end));
	let mut code = [0u8; 4];
	code.copy_from_slice(&bytes[at + 4..at + 8]);
	t.kind = Some(Kind::of(code));
	// A visual sample entry: six reserved bytes and a two-byte data reference index, then 70 bytes
	// of visual fields, of which the width and height sit at 16 and 18 (ISO/IEC 14496-12 §8.5.2).
	let visual = at + head + 8;
	if visual + 70 > at + size {
		return Err(err!(
			"A {} sample entry is too short to be a visual one.", String::from_utf8_lossy(&code);
		Invalid, Input, Decode));
	}
	t.size = (
		u16::from_be_bytes([bytes[visual + 16], bytes[visual + 17]]),
		u16::from_be_bytes([bytes[visual + 18], bytes[visual + 19]]),
	);
	// The configuration box sits among the sample entry's own children.
	let mut conf = None;
	res!(walk(bytes, code, visual + 70, at + size, 0, &mut |kind, _parent, cbody, cend| {
		if matches!(&kind, b"avcC" | b"hvcC") && conf.is_none() {
			conf = Some((cbody, cend));
		}
		Ok(false)
	}));
	t.config = conf;
	Ok(())
}

/// Turns a sample table into the position and length of every sample.
fn assemble(bytes: &[u8], t: Tables) -> Outcome<Film> {
	let kind = match t.kind {
		Some(k) => k,
		None => return Err(err!(
			"A video track carries no sample description, so nothing says how it is coded.";
		Invalid, Input, Missing)),
	};
	if t.runs.is_empty() || t.chunks.is_empty() {
		return Err(err!(
			"A video track's sample table has no sample-to-chunk or chunk offset box, so no \
			sample can be located.";
		Invalid, Input, Missing));
	}
	// Walk the runs, laying samples into chunks. `stsc` names the first chunk of each run counted
	// from one, and a run continues until the next one begins (ISO/IEC 14496-12 §8.7.4).
	let mut samples: Vec<(u64, u32)> = Vec::with_capacity(t.sizes.len());
	let mut n = 0usize;
	for (r, (first, per)) in t.runs.iter().enumerate() {
		if *first == 0 {
			return Err(err!(
				"A sample-to-chunk run begins at chunk 0, and the chunks are counted from one.";
			Invalid, Input, Decode));
		}
		let start = (*first - 1) as usize;
		let stop = match t.runs.get(r + 1) {
			Some((next, _)) if *next >= 1 => ((*next - 1) as usize).min(t.chunks.len()),
			_ => t.chunks.len(),
		};
		for c in start..stop {
			let base = t.chunks[c];
			let mut off = base;
			for _ in 0..*per {
				let len = match t.sizes.get(n) {
					Some(l) => *l,
					// A chunk table that runs on past the sample sizes is not a fault: the sizes
					// are the authority on how many samples there are.
					None => break,
				};
				samples.push((off, len));
				off = off.saturating_add(len as u64);
				n += 1;
			}
		}
	}
	if samples.len() != t.sizes.len() {
		return Err(err!(
			"A sample table lays out {} samples and names the size of {}. The two disagree, so \
			no sample can be trusted to be where the table says.", samples.len(), t.sizes.len();
		Invalid, Input, Mismatch));
	}
	let config = match t.config {
		Some((from, to)) => match bytes.get(from..to) {
			Some(s) => s.to_vec(),
			None => return Err(err!(
				"A decoder configuration record runs past the end of the file.";
			Invalid, Input, Decode)),
		},
		// Motion JPEG has none, and needs none.
		None => Vec::new(),
	};
	// `stss` counts from one and everything else here counts from nought.
	let mut sync = Vec::with_capacity(t.sync.len());
	for s in &t.sync {
		if *s == 0 {
			return Err(err!(
				"A sync sample table names sample 0, and the samples are counted from one.";
			Invalid, Input, Decode));
		}
		sync.push(*s - 1);
	}
	Ok(Film {
		kind,
		config,
		width:	t.size.0,
		height:	t.size.1,
		rotation: t.rotation,
		samples,
		sync,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The sequence parameter set of a 64 by 48 stream, as libx264 wrote it: the NAL unit that
	/// followed the first start code of
	/// `ffmpeg -f lavfi -i testsrc=size=64x48:rate=10 -c:v libx264 -f h264`.
	///
	/// It carries an emulation prevention byte -- the `03` in `00 00 03 00` at offset 11 -- so
	/// reading its geometry exercises the unescaping as well as the bit reader.
	const SPS: [u8; 21] = [
		0x67, 0x42, 0xC0, 0x0A, 0xDA, 0x11, 0xEC, 0x04, 0x40, 0x00, 0x00,
		0x03, 0x00, 0x40, 0x00, 0x00, 0x05, 0x03, 0xC4, 0x89, 0xA8,
	];

	/// The matching picture parameter set.
	const PPS: [u8; 4] = [0x68, 0xCE, 0x0F, 0xC8];

	/// An `AVCDecoderConfigurationRecord` around the fixture parameter sets, with a four-byte NAL
	/// length: version 1, the three profile bytes copied from the set, `0xFF` for the reserved bits
	/// and a length of four, `0xE1` for the reserved bits and one sequence parameter set.
	fn avcc() -> Vec<u8> {
		let mut rec = vec![1, SPS[1], SPS[2], SPS[3], 0xFF, 0xE1];
		rec.extend_from_slice(&(SPS.len() as u16).to_be_bytes());
		rec.extend_from_slice(&SPS);
		rec.push(1);
		rec.extend_from_slice(&(PPS.len() as u16).to_be_bytes());
		rec.extend_from_slice(&PPS);
		rec
	}

	/// A sample of `n` bytes, wrapped as one NAL unit with a four-byte length prefix.
	fn nal(n: usize) -> Vec<u8> {
		let mut v = ((n as u32).to_be_bytes()).to_vec();
		v.extend(std::iter::repeat(0x41).take(n));
		v
	}

	/// Finds the first box of the given type at the top level of `buf`, giving its body.
	fn top(buf: &[u8], kind: &[u8; 4]) -> Option<Vec<u8>> {
		let mut at = 0usize;
		while at + 8 <= buf.len() {
			let size = u32::from_be_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]]) as usize;
			if size < 8 || at + size > buf.len() {
				return None;
			}
			if &buf[at + 4..at + 8] == kind {
				return Some(buf[at + 8..at + size].to_vec());
			}
			at += size;
		}
		None
	}

	/// Finds the first box of the given type anywhere in `buf`, giving the offset of its body, and
	/// fails the test where there is none.
	fn want_box(buf: &[u8], kind: &[u8; 4]) -> Outcome<usize> {
		match buf.windows(4).position(|w| w == kind) {
			Some(i)	=> Ok(i + 4),
			None	=> Err(err!(
				"No '{}' box was written.", String::from_utf8_lossy(kind); Test, Missing)),
		}
	}

	/// The sequence parameter set of a 64 by 48 stream reads back as 64 by 48. The value is not
	/// this crate's: it is the size given to FFmpeg on the command line that produced the bytes.
	#[test]
	fn test_sps_geometry_00() -> Outcome<()> {
		let (w, h) = res!(sps_geometry(&SPS));
		req!(w, 64u16);
		req!(h, 48u16);
		Ok(())
	}

	/// The geometry read through the configuration record is the same as the geometry read from the
	/// set directly, and the record's NAL length field says four.
	#[test]
	fn test_avcc_walk_01() -> Outcome<()> {
		let c = Codec::Avc(avcc());
		req!(res!(c.geometry()), (64u16, 48u16));
		req!(res!(c.nal_len()), 4usize);
		Ok(())
	}

	/// Every field of the header boxes has a size the specification fixes, so the boxes have sizes
	/// that can be added up by hand.
	///
	/// `ftyp`: 8 header + 4 major brand + 4 minor version + 4 compatible brands of 4 = 32.
	/// `mvhd`: 8 header + 4 version and flags + 4 + 4 + 4 + 4 times and scales + 4 rate + 2 volume
	/// + 2 reserved + 8 reserved + 36 matrix + 24 pre-defined + 4 next track = 108.
	/// `tkhd`: 8 + 4 + 4 + 4 + 4 + 4 + 4 + 8 + 2 + 2 + 2 + 2 + 36 + 4 + 4 = 92.
	/// `mdhd`: 8 + 4 + 4 + 4 + 4 + 4 + 2 + 2 = 32.
	/// `hdlr`: 8 + 4 + 4 + 4 + 12 + 13 for "VideoHandler" and its terminator = 45.
	/// `vmhd`: 8 + 4 + 2 + 6 = 20.
	/// `stsc`: 8 + 4 + 4 count + 12 for the one entry = 28.
	#[test]
	fn test_fixed_box_sizes_02() -> Outcome<()> {
		req!(res!(ftyp()).len(), 32usize);
		req!(res!(hdlr()).len(), 45usize);
		req!(res!(vmhd()).len(), 20usize);
		req!(res!(stsc()).len(), 28usize);

		let mut t = res!(Track::new(64, 48, 1000, Codec::Avc(avcc())));
		res!(t.push(Sample::key(nal(10), 100)));
		req!(res!(t.mvhd(100)).len(), 108usize);
		req!(res!(t.tkhd(100)).len(), 92usize);
		req!(res!(t.mdhd()).len(), 32usize);
		Ok(())
	}

	/// `stts` merges consecutive samples of equal duration into one entry.
	///
	/// Durations 10, 10, 10, 20, 20, 10 give three entries -- (3, 10), (2, 20), (1, 10) -- so the
	/// box is 8 header + 4 version and flags + 4 entry count + 3 times 8 = 40 bytes, and the last
	/// entry's count is 1.
	#[test]
	fn test_stts_run_length_03() -> Outcome<()> {
		let mut t = res!(Track::new(64, 48, 1000, Codec::Avc(avcc())));
		for (i, d) in [10u32, 10, 10, 20, 20, 10].into_iter().enumerate() {
			res!(t.push(Sample { data: nal(4 + i), dur: d, sync: i == 0 }));
		}
		let b = res!(t.stts());
		req!(b.len(), 40usize);
		req!(&b[4..8], b"stts" as &[u8]);
		req!(u32::from_be_bytes([b[12], b[13], b[14], b[15]]), 3u32);
		let entries: Vec<(u32, u32)> = b[16..].chunks_exact(8)
			.map(|c| (
				u32::from_be_bytes([c[0], c[1], c[2], c[3]]),
				u32::from_be_bytes([c[4], c[5], c[6], c[7]]),
			))
			.collect();
		req!(entries, vec![(3u32, 10u32), (2, 20), (1, 10)]);
		Ok(())
	}

	/// A run of samples of one duration gives exactly one `stts` entry, however many there are.
	#[test]
	fn test_stts_constant_rate_is_one_entry_04() -> Outcome<()> {
		let mut t = res!(Track::new(64, 48, 1000, Codec::Avc(avcc())));
		for i in 0..500 {
			res!(t.push(Sample { data: nal(4), dur: 40, sync: i == 0 }));
		}
		let b = res!(t.stts());
		req!(b.len(), 24usize);
		req!(u32::from_be_bytes([b[12], b[13], b[14], b[15]]), 1u32);
		req!(u32::from_be_bytes([b[16], b[17], b[18], b[19]]), 500u32);
		Ok(())
	}

	/// `stss` is absent where every sample is a sync sample, since the specification reads that
	/// absence as exactly that claim, and present listing one-based sample numbers otherwise.
	#[test]
	fn test_stss_presence_05() -> Outcome<()> {
		let mut all = res!(Track::new(64, 48, 1000, Codec::Avc(avcc())));
		for _ in 0..3 {
			res!(all.push(Sample::key(nal(4), 10)));
		}
		req!(res!(all.stss()).is_none(), true);

		let mut some = res!(Track::new(64, 48, 1000, Codec::Avc(avcc())));
		for i in 0..5 {
			res!(some.push(Sample { data: nal(4), dur: 10, sync: i == 0 || i == 3 }));
		}
		let b = match res!(some.stss()) {
			Some(b)	=> b,
			None	=> return Err(err!(
				"The sync sample table should have been written."; Test)),
		};
		// 8 header + 4 version and flags + 4 count + 2 entries of 4 = 24.
		req!(b.len(), 24usize);
		req!(u32::from_be_bytes([b[12], b[13], b[14], b[15]]), 2u32);
		req!(u32::from_be_bytes([b[16], b[17], b[18], b[19]]), 1u32);
		req!(u32::from_be_bytes([b[20], b[21], b[22], b[23]]), 4u32);
		Ok(())
	}

	/// Every chunk offset names the first byte of its sample, and the first names the first byte
	/// after the `mdat` header.
	#[test]
	fn test_chunk_offsets_point_at_the_samples_06() -> Outcome<()> {
		let sizes = [11usize, 5, 23, 7];
		let mut t = res!(Track::new(64, 48, 600, Codec::Avc(avcc())));
		for (i, n) in sizes.into_iter().enumerate() {
			res!(t.push(Sample { data: nal(n), dur: 60, sync: i == 0 }));
		}
		let file = res!(t.finish());

		let at = res!(want_box(&file, b"stco"));
		let count = u32::from_be_bytes([file[at + 4], file[at + 5], file[at + 6], file[at + 7]]);
		req!(count, 4u32);

		let mdat = res!(want_box(&file, b"mdat"));
		let mut want = mdat as u32;
		for (i, n) in sizes.into_iter().enumerate() {
			let o = at + 8 + i * 4;
			let got = u32::from_be_bytes([file[o], file[o + 1], file[o + 2], file[o + 3]]);
			req!(got, want);
			// The sample's own first byte is its NAL length field, which is what was written.
			let len = u32::from_be_bytes([
				file[got as usize], file[got as usize + 1],
				file[got as usize + 2], file[got as usize + 3],
			]);
			req!(len, n as u32);
			want += (n + 4) as u32;
		}
		req!(want as usize, file.len());
		Ok(())
	}

	/// The top-level layout is `ftyp`, then `moov`, then `mdat`, and the three account for the
	/// whole file.
	#[test]
	fn test_top_level_layout_07() -> Outcome<()> {
		let mut t = res!(Track::new(64, 48, 90_000, Codec::Avc(avcc())));
		res!(t.push(Sample::key(nal(30), 3000)));
		res!(t.push(Sample::delta(nal(9), 3000)));
		let file = res!(t.finish());

		let mut kinds = Vec::new();
		let mut at = 0usize;
		while at + 8 <= file.len() {
			let size = u32::from_be_bytes([file[at], file[at + 1], file[at + 2], file[at + 3]])
				as usize;
			kinds.push(String::from_utf8_lossy(&file[at + 4..at + 8]).to_string());
			at += size;
		}
		req!(at, file.len());
		req!(kinds, vec!["ftyp".to_string(), "moov".to_string(), "mdat".to_string()]);

		// The media box holds exactly the samples: two NAL units of 30 and 9 bytes, each with a
		// four-byte length in front of it.
		let mdat = match top(&file, b"mdat") {
			Some(b)	=> b,
			None	=> return Err(err!("No media box was written."; Test)),
		};
		req!(mdat.len(), 47usize);
		Ok(())
	}

	/// The media header keeps the track's timescale, and the movie header restates the same
	/// duration in milliseconds.
	///
	/// Thirty samples of 3003 ticks at 90000 a second is 90090 ticks, which is 1001 milliseconds.
	#[test]
	fn test_durations_in_their_own_timescales_08() -> Outcome<()> {
		let mut t = res!(Track::new(64, 48, 90_000, Codec::Avc(avcc())));
		for i in 0..30 {
			res!(t.push(Sample { data: nal(6), dur: 3003, sync: i == 0 }));
		}
		req!(t.duration(), 90_090u64);
		let file = res!(t.finish());

		let mdhd = res!(want_box(&file, b"mdhd"));
		req!(u32::from_be_bytes([
			file[mdhd + 12], file[mdhd + 13], file[mdhd + 14], file[mdhd + 15],
		]), 90_000u32);
		req!(u32::from_be_bytes([
			file[mdhd + 16], file[mdhd + 17], file[mdhd + 18], file[mdhd + 19],
		]), 90_090u32);

		let mvhd = res!(want_box(&file, b"mvhd"));
		req!(u32::from_be_bytes([
			file[mvhd + 12], file[mvhd + 13], file[mvhd + 14], file[mvhd + 15],
		]), 1000u32);
		req!(u32::from_be_bytes([
			file[mvhd + 16], file[mvhd + 17], file[mvhd + 18], file[mvhd + 19],
		]), 1001u32);
		Ok(())
	}

	/// The track header carries the unity matrix and the frame size in 16.16 fixed point.
	#[test]
	fn test_tkhd_matrix_and_size_09() -> Outcome<()> {
		let mut t = res!(Track::new(64, 48, 1000, Codec::Avc(avcc())));
		res!(t.push(Sample::key(nal(8), 40)));
		let b = res!(t.tkhd(40));
		// 8 header + 4 version and flags + 36 fixed fields brings the matrix to offset 48.
		let m: Vec<u32> = b[48..84].chunks_exact(4)
			.map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
			.collect();
		req!(m, vec![0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000]);
		req!(u32::from_be_bytes([b[84], b[85], b[86], b[87]]), 64u32 << 16);
		req!(u32::from_be_bytes([b[88], b[89], b[90], b[91]]), 48u32 << 16);
		Ok(())
	}

	/// A track that names dimensions the stream does not code is refused, and the message names
	/// both.
	#[test]
	fn test_refuses_a_geometry_mismatch_10() -> Outcome<()> {
		match Track::new(1920, 1080, 1000, Codec::Avc(avcc())) {
			Ok(_) => Err(err!("A 1920 by 1080 track over a 64 by 48 stream was accepted."; Test)),
			Err(e) => {
				let msg = e.to_string();
				req!(msg.contains("1920"), true, "The message does not name the declared width.");
				req!(msg.contains("64"), true, "The message does not name the coded width.");
				Ok(())
			},
		}
	}

	/// A timescale of zero, a zero dimension, an empty track, a sample of no duration, a track that
	/// does not begin at a sync sample, and a sample handed over in Annex B are each refused.
	#[test]
	fn test_refusals_11() -> Outcome<()> {
		req!(Track::new(64, 48, 0, Codec::Avc(avcc())).is_err(), true, "A zero timescale passed.");
		req!(Track::new(0, 48, 1000, Codec::Avc(avcc())).is_err(), true, "A zero width passed.");

		let empty = res!(Track::new(64, 48, 1000, Codec::Avc(avcc()))).finish().is_err();
		req!(empty, true, "A track with no samples passed.");

		let mut t = res!(Track::new(64, 48, 1000, Codec::Avc(avcc())));
		req!(t.push(Sample::key(nal(4), 0)).is_err(), true, "A sample of no duration passed.");
		req!(t.push(Sample::key(Vec::new(), 40)).is_err(), true, "A sample of no bytes passed.");

		let mut annexb = res!(Track::new(64, 48, 1000, Codec::Avc(avcc())));
		let e = annexb.push(Sample::key(vec![0, 0, 0, 1, 0x65, 0x88, 0x84], 40));
		match e {
			Ok(_) => return Err(err!("An Annex B sample was accepted."; Test)),
			Err(e) => req!(e.to_string().contains("Annex B"), true,
				"The message does not name the format that was handed over."),
		}

		let mut short = res!(Track::new(64, 48, 1000, Codec::Avc(avcc())));
		req!(short.push(Sample::key(vec![0x00, 0x00, 0x00, 0x40, 0x65], 40)).is_err(), true,
			"A sample whose NAL runs past its end passed.");

		let mut nokey = res!(Track::new(64, 48, 1000, Codec::Avc(avcc())));
		res!(nokey.push(Sample::delta(nal(4), 40)));
		let begun = nokey.finish().is_err();
		req!(begun, true, "A track that begins at a delta sample passed.");
		Ok(())
	}

	/// A configuration record that is truncated, versioned wrongly, or names the forbidden
	/// three-byte NAL length is refused before a file is built around it.
	#[test]
	fn test_refuses_a_bad_configuration_record_12() -> Outcome<()> {
		let stub = Codec::Avc(vec![1, 66, 192, 10]).geometry().is_err();
		req!(stub, true, "A stub record passed.");

		let mut bad_ver = avcc();
		bad_ver[0] = 2;
		let bad_ver = Codec::Avc(bad_ver).geometry().is_err();
		req!(bad_ver, true, "Version 2 passed.");

		let mut bad_len = avcc();
		bad_len[4] = 0xFE;	// lengthSizeMinusOne = 2, a three-byte length.
		let bad_len = Codec::Avc(bad_len).nal_len().is_err();
		req!(bad_len, true, "A three-byte NAL length passed.");

		let mut cut = avcc();
		cut.truncate(12);
		let cut = Codec::Avc(cut).geometry().is_err();
		req!(cut, true, "A truncated parameter set passed.");

		let mut no_pps = vec![1, SPS[1], SPS[2], SPS[3], 0xFF, 0xE1];
		no_pps.extend_from_slice(&(SPS.len() as u16).to_be_bytes());
		no_pps.extend_from_slice(&SPS);
		no_pps.push(0);
		let no_pps = Codec::Avc(no_pps).geometry().is_err();
		req!(no_pps, true, "A record with no picture set passed.");
		Ok(())
	}

	/// Rescaling rounds to nearest rather than truncating, so a duration does not creep short.
	///
	/// 90090 ticks at 90000 a second is 1001 milliseconds exactly; 1 tick at 3 a second is 333.33
	/// milliseconds, which rounds to 333; 2 ticks at 3 is 666.67, which rounds to 667.
	#[test]
	fn test_rescale_rounds_to_nearest_13() -> Outcome<()> {
		req!(res!(rescale(90_090, 90_000, 1000)), 1001u64);
		req!(res!(rescale(1, 3, 1000)), 333u64);
		req!(res!(rescale(2, 3, 1000)), 667u64);
		req!(res!(rescale(0, 1000, 1000)), 0u64);
		req!(rescale(1, 0, 1000).is_err(), true, "A zero source timescale passed.");
		Ok(())
	}

	/// The emulation prevention bytes come out and nothing else does: `00 00 03 00` is `00 00 00`,
	/// and a `03` that does not follow two zeroes is kept.
	#[test]
	fn test_rbsp_unescaping_14() -> Outcome<()> {
		req!(rbsp(&[0x00, 0x00, 0x03, 0x00, 0x01]), vec![0x00u8, 0x00, 0x00, 0x01]);
		req!(rbsp(&[0x01, 0x03, 0x02]), vec![0x01u8, 0x03, 0x02]);
		req!(rbsp(&[0x00, 0x00, 0x03, 0x03]), vec![0x00u8, 0x00, 0x03]);
		Ok(())
	}

	/// The 64-bit chunk offset table is the same table in a wider field: the same entry count, eight
	/// bytes an entry rather than four, under the type `co64`.
	///
	/// The choice between the two is made in `finish` by where the last byte of media falls, so it
	/// cannot be reached without building a file of four gibibytes. The table itself is written
	/// here directly instead, which is the part that would be wrong.
	#[test]
	fn test_the_64_bit_offset_table_16() -> Outcome<()> {
		let mut t = res!(Track::new(64, 48, 1000, Codec::Avc(avcc())));
		for i in 0..3 {
			res!(t.push(Sample { data: nal(20), dur: 40, sync: i == 0 }));
		}
		let base = 5_000_000_000u64;
		let b = res!(t.offsets(base, true));
		// 8 header + 4 version and flags + 4 count + 3 entries of 8 = 40.
		req!(b.len(), 40usize);
		req!(&b[4..8], b"co64" as &[u8]);
		req!(u32::from_be_bytes([b[12], b[13], b[14], b[15]]), 3u32);
		let entries: Vec<u64> = b[16..].chunks_exact(8)
			.map(|c| u64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
			.collect();
		req!(entries, vec![base, base + 24, base + 48]);

		// The 32-bit table refuses the same offsets rather than truncating them.
		req!(t.offsets(base, false).is_err(), true, "A 32-bit table took a 5 GB offset.");
		Ok(())
	}

	/// A file written twice from the same samples is the same file: the header times are written as
	/// unset rather than taken from the clock, so a build is reproducible.
	#[test]
	fn test_output_is_deterministic_15() -> Outcome<()> {
		let build = || -> Outcome<Vec<u8>> {
			let mut t = res!(Track::new(64, 48, 1000, Codec::Avc(avcc())));
			for i in 0..8 {
				res!(t.push(Sample { data: nal(12 + i), dur: 40, sync: i % 4 == 0 }));
			}
			t.finish()
		};
		req!(res!(build()), res!(build()));
		Ok(())
	}
}
