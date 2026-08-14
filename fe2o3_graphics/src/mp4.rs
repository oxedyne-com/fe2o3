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
//! [`Fragments`] writes the other shape, for the film a writer cannot hold: `ftyp` and a `moov`
//! that names the streams and states no duration, then one `moof` and `mdat` for each run of
//! samples handed over. Every sample's timing is stated in the fragment that carries it, so nothing
//! is kept back and nothing is rewritten at the end, and a film of unknown length -- several hours
//! of it, or several streams of it -- can be written a fragment at a time.
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
//! 14496-15 (the AVC file format), and the `hvc1` entry and its `hvcC` box are ISO/IEC 14496-15
//! §8.3 and §8.4. The sequence parameter set whose geometry is checked against the caller's
//! declared dimensions is ITU-T H.264 §7.3.2.1.1 for AVC and ITU-T H.265 §7.3.2.2 for HEVC, the
//! latter read by [`crate::hevc`]. Each non-obvious constant below names the clause it comes from.

use crate::hevc;

use oxedyne_fe2o3_core::prelude::*;

use std::{
	fs::File,
	io::{
		Read,
		Seek,
		SeekFrom,
	},
};

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
	/// Whether decoding may begin here: a sync sample, which is an IDR picture for AVC and an IRAP
	/// picture -- an IDR, a broken link, or a clean random access picture -- for HEVC.
	pub sync:	bool,
	/// How far after its decoding time this sample is shown, in the track's timescale.
	///
	/// Nought for a stream whose pictures are shown in the order they are decoded, which is what a
	/// screen recording or a poster is. **A film is not such a stream.** Where B-pictures are used a
	/// picture is decoded before the ones it is shown between, so the two orders differ and the
	/// container has to state both: `stts` gives the decoding times and this gives the difference.
	/// Writing a reordered stream with this left at nought produces a file that opens, reports the
	/// right number of frames, and plays them in the wrong order.
	pub off:	i32,
}

impl Sample {

	/// A sync sample: one a reader may begin decoding at.
	pub fn key(data: Vec<u8>, dur: u32) -> Self {
		Self { data, dur, sync: true, off: 0 }
	}

	/// A sample that depends on those before it.
	pub fn delta(data: Vec<u8>, dur: u32) -> Self {
		Self { data, dur, sync: false, off: 0 }
	}

	/// The same sample, shown the given distance after it is decoded.
	///
	/// See [`composition_offsets`], which works the offsets out from the presentation times a
	/// container states, since that is the form a film arrives in.
	pub fn shown_after(mut self, off: i32) -> Self {
		self.off = off;
		self
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

/// What a stream carries.
///
/// The distinction decides which media header a track is given, which handler declares it, and the
/// shape of its sample entry -- three boxes that must agree, because a reader told two different
/// things about one track believes whichever it reads last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Media {
	/// Moving pictures of the given size.
	Picture {
		w:	u16,
		h:	u16,
	},
	/// Sound of the given channel count, sampled at the given rate.
	Sound {
		channels:	u16,
		/// Samples a second. A track's timescale is usually this same number, so that a sample's
		/// duration is a count of sound samples and no rounding enters.
		rate:		u32,
	},
}

/// One stream of a film, as its header describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stream {
	/// What it carries.
	pub media:		Media,
	/// Ticks a second the stream's own durations are counted in.
	///
	/// Not required to be the sampling rate of a sound stream, though it often is. A repackaging
	/// keeps the source's unit so that no time is rescaled between the two containers, and a
	/// millisecond timescale against a 44,100 Hz stream is therefore ordinary rather than wrong.
	pub timescale:	u32,
	/// How the samples are coded, and the configuration a decoder needs.
	pub codec:		Codec,
	/// The decode time the stream's first sample lands at, in this stream's own timescale.
	///
	/// Nearly always nought, and it exists for the case that is not: **the streams of a film do not
	/// begin together.** A film's first sound frame is rarely on the same instant as its first
	/// picture, and a picture track shifted so that none of its composition offsets is negative has
	/// moved relative to sound that was not shifted with it. Without this the two are nailed to a
	/// common zero and the film carries an offset between picture and sound that no caller can
	/// remove -- which is the characteristic fault of a bad repackaging and the one nobody notices
	/// until the film is being watched.
	pub start:		u64,
}

/// How a track's samples are coded, and the decoder configuration that goes with them.
///
/// An enum rather than a trait object, so that adding a codec is a variant and a match arm rather
/// than a second dispatch mechanism, and so that a caller can see from the type what the writer
/// will accept. There is deliberately no catch-all arm anywhere it is matched on: the next codec
/// added must be considered at every site rather than take whatever the last one does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Codec {
	/// H.264, carrying the `AVCDecoderConfigurationRecord` of ISO/IEC 14496-15 §5.3.3.1 verbatim:
	/// the bytes a `VideoEncoder` hands back as its output's description, or the `avcC` box body
	/// lifted out of another file.
	Avc(Vec<u8>),
	/// HEVC, carrying the `HEVCDecoderConfigurationRecord` of ISO/IEC 14496-15 §8.3.3.1
	/// verbatim -- the `hvcC` box body lifted out of another file.
	Hevc(Vec<u8>),
	/// AAC, carrying the `AudioSpecificConfig` of ISO/IEC 14496-3 §1.6.2.1 verbatim: two bytes for
	/// the common profiles, naming the object type, the sampling frequency and the channel
	/// configuration. It is what a Matroska track entry's `CodecPrivate` holds for `A_AAC`, so a
	/// repackaging copies it across exactly as the picture's record is copied.
	///
	/// The sample bytes are **raw AAC frames, not ADTS**: an ADTS header states again, once a
	/// frame, what this record states once for the track, and a decoder handed both refuses.
	Aac(Vec<u8>),
}

impl Codec {

	/// Whether the stream is a picture, which decides the boxes its track is described by.
	pub fn is_picture(&self) -> bool {
		match self {
			Self::Avc(_)	=> true,
			Self::Hevc(_)	=> true,
			Self::Aac(_)	=> false,
		}
	}

	/// The four-character code of the sample entry this codec is described by: ISO/IEC 14496-15
	/// §5.4.2.1 for AVC and §8.4.1 for HEVC.
	///
	/// `hvc1` rather than `hev1`. The two differ in one promise: `hvc1` states that every parameter
	/// set is in the sample entry and none arrives in the samples, while `hev1` allows them in
	/// either place. A repackaging from a container that keeps the sets in a `CodecPrivate` -- which
	/// is what Matroska does, and what this writer is handed -- produces exactly the first, so that
	/// is what is claimed. `hev1` would be true as well but weaker, and it makes a reader look for
	/// sets in the samples that are not there.
	fn entry(&self) -> &'static [u8; 4] {
		match self {
			Self::Avc(_)	=> b"avc1",
			Self::Hevc(_)	=> b"hvc1",
			Self::Aac(_)	=> b"mp4a",
		}
	}

	/// The four-character code of the configuration box carried inside that sample entry.
	fn config(&self) -> &'static [u8; 4] {
		match self {
			Self::Avc(_)	=> b"avcC",
			Self::Hevc(_)	=> b"hvcC",
			Self::Aac(_)	=> b"esds",
		}
	}

	/// The configuration record's bytes, written into the configuration box unchanged.
	fn record(&self) -> &[u8] {
		match self {
			Self::Avc(rec)	=> rec,
			Self::Hevc(rec)	=> rec,
			Self::Aac(rec)	=> rec,
		}
	}

	/// The name a visual sample entry's compressor field displays.
	///
	/// Descriptive only -- nothing decodes from it -- but it is a field some viewers show, so it
	/// says what the stream is rather than what the first codec this writer supported was. An empty
	/// name is written as a count of nought, which is what a file with nothing to say there should
	/// carry, and is what sound gets because sound has no visual sample entry to name.
	fn picture_name(&self) -> &'static str {
		match self {
			Self::Avc(_)	=> "AVC Coding",
			Self::Hevc(_)	=> "HEVC Coding",
			Self::Aac(_)	=> "",
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
			// The whole record is walked to reach a field that sits at a fixed offset in it,
			// because [`crate::hevc::config`] is the reader this crate already holds to the
			// specification and a second hand-rolled one would be a second thing to be wrong. It
			// also means a malformed record is refused here rather than trusted as far as byte 21.
			Self::Hevc(rec) => Ok(res!(hevc::config(rec)).length_size),
			Self::Aac(_) => Err(err!(
				"A sound sample is a coded frame and is not tiled by NAL length prefixes, so \
				asking for its prefix width is a question about the wrong kind of stream.";
			Invalid, Input)),
		}
	}

	/// Checks the configuration record is well formed, and gives the frame geometry it codes.
	///
	/// The record is walked rather than trusted, because every field of the sample table below is
	/// derived from it and a truncated record produces a file that is well formed and unplayable.
	fn geometry(&self) -> Outcome<(u16, u16)> {
		match self {
			Self::Aac(_) => Err(err!(
				"A sound stream codes no frame geometry, so a track built from one cannot be \
				checked against a declared picture size.";
			Invalid, Input)),
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
			Self::Hevc(rec) => {
				let cfg = res!(hevc::config(rec));
				// The first sequence parameter set the record carries. A film's record often carries
				// several, and a slice names which of them it was coded against, but every set in
				// one record describes the same pictures at the same size -- a change of geometry
				// mid-film is a new sample entry, not a new set in this one.
				//
				// `Unit::body` is the payload with the emulation prevention **already** undone:
				// `hevc::unit` builds it through `hevc::rbsp` and keeps the escaped form beside it
				// as `Unit::raw`. So nothing is unescaped again here. Doing it twice would take a
				// genuine `00 00 03` out of the syntax and give a size that is wrong and plausible.
				let mut sps = None;
				let mut found: Vec<String> = Vec::with_capacity(cfg.sets.len());
				for unit in &cfg.sets {
					if unit.kind == hevc::nal::SPS && sps.is_none() {
						sps = Some(res!(hevc::sps(&unit.body)));
					}
					found.push(unit.kind.to_string());
				}
				let sps = match sps {
					Some(s) => s,
					None => {
						let carries = if found.is_empty() {
							fmt!("no parameter sets at all")
						} else {
							fmt!("NAL unit types {}", found.join(", "))
						};
						return Err(err!(
							"The HEVC decoder configuration record carries no sequence parameter \
							set, which is NAL unit type {}, so the frame geometry it should \
							describe is absent. It carries {}.", hevc::nal::SPS, carries;
						Invalid, Input, Missing));
					},
				};
				// `hevc::sps` refuses a picture wider or taller than 16,384, so this cannot fire
				// today. It is written because the cast below is silent where that ceiling is not,
				// and the two are in different crates' worth of code from each other.
				if sps.width > u16::MAX as u32 || sps.height > u16::MAX as u32 {
					return Err(err!(
						"The sequence parameter set codes a frame of {} by {} pixels, beyond what a \
						visual sample entry can state.", sps.width, sps.height;
					Invalid, Input, Excessive));
				}
				Ok((sps.width as u16, sps.height as u16))
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
		match self {
			// A coded sound frame has no internal framing to check against: its length is
			// the whole of what says where it ends. Refusing an empty one is done by the
			// caller, and there is nothing else here that can be told from the bytes.
			Self::Aac(_) => return Ok(()),
			// Both picture codecs are checked by the same walk, because both are tiled by
			// length prefixes and the framing is the whole of what is being tested. The NAL
			// unit types inside differ and nothing here reads one. Written as a match rather
			// than a test for sound, so that a codec added later has to say which it is.
			Self::Avc(_) | Self::Hevc(_) => {},
		}
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

		let ftyp = res!(ftyp(&self.codec));
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
		if let Some(ctts) = res!(self.ctts()) {
			body.extend_from_slice(&ctts);
		}
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
		// to put rubbish in front of a viewer that displays the field. The name comes from the
		// codec, because a fixed one would go on saying "AVC Coding" over an HEVC track.
		let name = self.codec.picture_name().as_bytes();
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

	/// The composition time to sample table, ISO/IEC 14496-12 §8.6.1.3, run-length coded.
	///
	/// `None` where every sample is shown in the order it is decoded, because a track that never
	/// reorders must not carry the box at all -- an absent `ctts` is the statement that the two
	/// orders are the same, and writing a table of zeroes says the same thing at the cost of four
	/// bytes a sample.
	///
	/// Written at version 1, whose offsets are **signed**. Version 0's are unsigned, which forces
	/// every decoding time to sit at or before the earliest presentation time and makes the first
	/// pictures of a reordered stream inexpressible without shifting the whole track.
	/// [`composition_offsets`] shifts anyway, so version 0 would serve -- but a signed table states
	/// what is true rather than what has been arranged to be true, and a caller that works its own
	/// offsets out is not forced into the same arrangement.
	fn ctts(&self) -> Outcome<Option<Vec<u8>>> {
		if self.samples.iter().all(|s| s.off == 0) {
			return Ok(None);
		}
		let mut runs: Vec<(u32, i32)> = Vec::new();
		for s in &self.samples {
			match runs.last_mut() {
				Some((n, o)) if *o == s.off	=> *n += 1,
				_				=> runs.push((1, s.off)),
			}
		}
		let mut b = Vec::with_capacity(8 + runs.len() * 8);
		b.extend_from_slice(&full(1, 0));
		b.extend_from_slice(&(runs.len() as u32).to_be_bytes());
		for (n, o) in runs {
			b.extend_from_slice(&n.to_be_bytes());
			b.extend_from_slice(&o.to_be_bytes());
		}
		Ok(Some(res!(bx(b"ctts", &b))))
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

// ------------------------------------------------------------------------- a fragmented film

/// The flags a sync sample carries in a track run: `sample_depends_on` = 2, meaning it refers to no
/// other picture, and `sample_is_non_sync_sample` = 0. ISO/IEC 14496-12 §8.8.3.1.
const SAMPLE_SYNC: u32 = 0x0200_0000;

/// The flags a sample that is not a sync sample carries: `sample_depends_on` = 1, meaning it refers
/// to other pictures, and `sample_is_non_sync_sample` = 1.
///
/// Both halves are stated. A reader deciding where it may begin reads one or the other, and not
/// always the same one, so a sample that says it depends on nothing while also saying it is not a
/// sync sample is a contradiction each reader settles its own way.
const SAMPLE_DELTA: u32 = 0x0101_0000;

/// A film written as a header followed by fragments.
///
/// The counterpart of [`Track`], for the film whose samples are not all in hand. The header states
/// what the streams are and states no duration at all, and each fragment after it carries its own
/// timing for the samples it holds, so nothing is held back and nothing is rewritten at the end:
/// the bytes can go out as they are produced, to a file being appended to or to a reader already
/// playing the fragments before them.
///
/// What that costs against [`Track`] is the index. A fragmented film has no whole-film sample
/// table, so a reader seeking into one walks the fragments to find where it is going. What it buys
/// is that the writer never holds the film, and that a film of unknown length can be written.
pub struct Fragments {
	/// The streams, in the order given, whose track ids are their positions plus one.
	streams:	Vec<Stream>,
	/// Each stream's next decode time, in that stream's own timescale.
	///
	/// Kept here because a fragment states the decode time of its first sample outright, and
	/// nothing in the fragments before it says where that time has got to: a reader handed only
	/// fragment fifty must be able to place it, which is the whole point of the field.
	times:		Vec<u64>,
	/// The sequence number the next fragment carries, counting from one.
	seq:		u32,
}

impl Fragments {

	/// Begins a film of the given streams. Track ids are 1..=n in the order given.
	///
	/// Everything about a stream that can be checked is checked here rather than at the first
	/// fragment, because the header describing it has by then been handed to a reader and cannot be
	/// taken back.
	pub fn new(streams: Vec<Stream>) -> Outcome<Self> {
		if streams.is_empty() {
			return Err(err!(
				"A film is made of streams and none were given.";
			Invalid, Input, Missing));
		}
		for (i, s) in streams.iter().enumerate() {
			if s.timescale == 0 {
				return Err(err!(
					"Stream {} is given a timescale of zero ticks a second, so no sample duration \
					written against it would mean anything.", i;
				Invalid, Input, Range));
			}
			match s.media {
				Media::Picture { w, h } => {
					if !s.codec.is_picture() {
						return Err(err!(
							"Stream {} is declared as pictures and coded by a sound codec, so the \
							handler, the media header and the sample entry its track carries cannot \
							all be right.", i;
						Invalid, Input, Mismatch));
					}
					let (cw, ch) = match s.codec.geometry() {
						Ok(g)	=> g,
						Err(e)	=> return Err(err!(e,
							"Stream {}'s decoder configuration could not be read.", i;
						Invalid, Input)),
					};
					if cw != w || ch != h {
						return Err(err!(
							"Stream {} is declared {} by {} pixels, but the sequence parameter set \
							in its decoder configuration codes {} by {}.", i, w, h, cw, ch;
						Invalid, Input, Mismatch));
					}
				},
				Media::Sound { rate, .. } => {
					if s.codec.is_picture() {
						return Err(err!(
							"Stream {} is declared as sound and coded by a picture codec, so the \
							handler, the media header and the sample entry its track carries cannot \
							all be right.", i;
						Invalid, Input, Mismatch));
					}
					// The sample entry states the rate in 16.16 fixed point, whose whole part is
					// sixteen bits. There is a version 1 entry that carries a wider one, and it is
					// not written here, so a rate that will not fit is refused rather than truncated
					// into a file that plays at the wrong speed.
					if rate >= 1 << 16 {
						return Err(err!(
							"Stream {} is sampled at {} Hz, and the 16.16 fixed point field a sound \
							sample entry states its rate in stops one short of 65536.", i, rate;
						Invalid, Input, Excessive));
					}
				},
			}
		}
		let times = streams.iter().map(|s| s.start).collect();
		Ok(Self {
			streams,
			times,
			seq:	1,
		})
	}

	/// `ftyp` + `moov`: the initialisation segment, carrying no samples.
	///
	/// Every duration in it is nought, and in a fragmented film that is a statement rather than a
	/// gap left to be filled: the length is not known, and a reader is told to take the timing from
	/// the fragments. The sample tables under `stbl` are written empty for the same reason, and they
	/// are written rather than left out because ISO/IEC 14496-12 §8.5.1 requires them present.
	pub fn head(&self) -> Outcome<Vec<u8>> {
		let ftyp = res!(ftyp_frag());
		let moov = res!(self.moov());
		let mut out = Vec::with_capacity(ftyp.len() + moov.len());
		out.extend_from_slice(&ftyp);
		out.extend_from_slice(&moov);
		Ok(out)
	}

	/// One `moof` + `mdat` pair carrying the given samples.
	///
	/// Each entry is (index into the streams given to [`Fragments::new`], that stream's samples in
	/// decode order). The samples are taken by value because they are moved into the media box and
	/// not copied: a `Vec` behind a shared reference cannot be moved out of, so borrowing here would
	/// clone every sample's bytes and carry the whole fragment twice.
	///
	/// An empty `Vec` of samples for a listed stream is legal and writes a track fragment whose
	/// `sample_count` is nought. A stream with nothing in this fragment is ordinary -- sound and
	/// pictures do not divide at the same instants -- and the empty run still says the stream is
	/// there and where its decode time has got to.
	///
	/// Each fragment's decode times carry on from the fragments before it, so the same samples
	/// handed over in two calls and in one produce the same timing.
	pub fn next(&mut self, runs: Vec<(usize, Vec<Sample>)>) -> Outcome<Vec<u8>> {
		let frag = self.seq;
		let mut seen = vec![false; self.streams.len()];
		let mut total = 0u64;
		for (i, samples) in &runs {
			let i = *i;
			if i >= self.streams.len() {
				return Err(err!(
					"Fragment {} names stream {}, and the film has {}.",
					frag, i, self.streams.len();
				Invalid, Input, Index));
			}
			if seen[i] {
				return Err(err!(
					"Fragment {} names stream {} twice, and a stream has at most one track fragment \
					in a movie fragment.", frag, i;
				Invalid, Input, Duplicate));
			}
			seen[i] = true;
			let codec = &self.streams[i].codec;
			for (k, sam) in samples.iter().enumerate() {
				if sam.data.is_empty() {
					return Err(err!(
						"Sample {} of stream {} in fragment {} carries no bytes.", k, i, frag;
					Invalid, Input, Missing));
				}
				if sam.dur == 0 {
					return Err(err!(
						"Sample {} of stream {} in fragment {} is given a duration of zero ticks, \
						so it is shown for no time at all.", k, i, frag;
					Invalid, Input, Range));
				}
				if sam.data.len() > u32::MAX as usize {
					return Err(err!(
						"Sample {} of stream {} in fragment {} is {} bytes, which will not fit the \
						32-bit size a track run states.", k, i, frag, sam.data.len();
					Invalid, Input, Excessive));
				}
				if let Err(e) = codec.check_sample(k, &sam.data) {
					return Err(err!(e,
						"Sample {} of stream {} in fragment {} is not coded the way the stream's \
						decoder configuration says it is.", k, i, frag;
					Invalid, Input));
				}
				total += sam.data.len() as u64;
			}
		}

		// The media box's header is eight bytes, unless its payload will not fit a 32-bit size, in
		// which case ISO/IEC 14496-12 §4.2 writes 1 in the size field and a 64-bit `largesize` after
		// the type. The width is settled here and not at the writing, because every data offset
		// below is measured across it.
		let mdat_hdr = if total + 8 > u32::MAX as u64 { 16u64 } else { 8u64 };

		// Where each stream stands before this fragment adds to it. Taken now because the movie
		// fragment box is built twice and both passes must state the same times.
		let bases: Vec<u64> = runs.iter().map(|(i, _)| self.times[*i]).collect();

		// Two passes. A track run's data offset is measured from the first byte of the movie
		// fragment box that holds it, so it cannot be known until that box's size is -- and the size
		// depends on the offsets only through fields of a fixed width, so sizing the box against
		// placeholders and writing it again with the real values settles at once.
		let blank = vec![0i32; runs.len()];
		let probe = res!(self.moof(frag, &runs, &bases, &blank));
		let mut offs = Vec::with_capacity(runs.len());
		let mut at = probe.len() as u64 + mdat_hdr;
		for (_, samples) in &runs {
			if at > i32::MAX as u64 {
				return Err(err!(
					"Fragment {} puts a track run's data {} bytes past the movie fragment it is \
					measured from, and that offset is a signed 32-bit field.", frag, at;
				Invalid, Input, Excessive));
			}
			offs.push(at as i32);
			for s in samples {
				at += s.data.len() as u64;
			}
		}
		let moof = res!(self.moof(frag, &runs, &bases, &offs));

		// The rebuilt box must be the size the offsets were measured against, or every one of them
		// is wrong by the difference. It holds by construction, and it is asserted because a file
		// whose offsets are all out by a few bytes is well formed, opens, and plays rubbish.
		if moof.len() != probe.len() {
			return Err(err!(
				"Fragment {}'s movie fragment box came to {} bytes when sized and {} bytes when \
				written, so every data offset in it is out by {}.",
				frag, probe.len(), moof.len(), moof.len() as i64 - probe.len() as i64;
			Bug, Unreachable));
		}
		// And the walk that laid the offsets out must have covered exactly the samples that are
		// about to be written, or a later track run points past the end of the media box.
		if at != moof.len() as u64 + mdat_hdr + total {
			return Err(err!(
				"Fragment {} laid its data offsets out to byte {}, and the fragment ends at {}.",
				frag, at, moof.len() as u64 + mdat_hdr + total;
			Bug, Unreachable));
		}

		let mut out = Vec::with_capacity(moof.len() + mdat_hdr as usize + total as usize);
		out.extend_from_slice(&moof);
		if mdat_hdr == 16 {
			out.extend_from_slice(&1u32.to_be_bytes());
			out.extend_from_slice(b"mdat");
			out.extend_from_slice(&(total + 16).to_be_bytes());
		} else {
			out.extend_from_slice(&((total + 8) as u32).to_be_bytes());
			out.extend_from_slice(b"mdat");
		}
		// Track fragment order, then sample order: all of the first stream's bytes, then all of the
		// second's. That is the order the offsets above were counted in.
		for (_, samples) in &runs {
			for s in samples {
				out.extend_from_slice(&s.data);
			}
		}

		// Only now is anything of the film's state moved on, so a refused fragment leaves the writer
		// where it was and the caller may hand over a corrected one.
		for (i, samples) in &runs {
			let mut ticks = 0u64;
			for s in samples {
				ticks += s.dur as u64;
			}
			self.times[*i] += ticks;
		}
		self.seq += 1;
		Ok(out)
	}

	/// The movie box: the movie header, one track for each stream, and the extends box that says
	/// the sample descriptions are completed by fragments rather than by the tables above them.
	fn moov(&self) -> Outcome<Vec<u8>> {
		let mut body = Vec::new();
		body.extend_from_slice(&res!(self.mvhd()));
		for (i, s) in self.streams.iter().enumerate() {
			body.extend_from_slice(&res!(self.trak(i, s)));
		}
		body.extend_from_slice(&res!(self.mvex()));
		bx(b"moov", &body)
	}

	/// The movie header, ISO/IEC 14496-12 §8.2.2, version 0, stating no duration.
	///
	/// A duration of nought is the fragmented film's way of saying the length is not known yet; a
	/// reader that wants it adds the fragments up, or reads an `mfra` at the end if one was written.
	fn mvhd(&self) -> Outcome<Vec<u8>> {
		let mut b = Vec::with_capacity(100);
		b.extend_from_slice(&full(0, 0));
		b.extend_from_slice(&0u32.to_be_bytes());	// Creation time, unset.
		b.extend_from_slice(&0u32.to_be_bytes());	// Modification time, unset.
		b.extend_from_slice(&MOVIE_TIMESCALE.to_be_bytes());
		b.extend_from_slice(&0u32.to_be_bytes());	// Duration, not yet known.
		b.extend_from_slice(&0x0001_0000u32.to_be_bytes());	// Rate: 1.0 in 16.16.
		b.extend_from_slice(&0x0100u16.to_be_bytes());		// Volume: 1.0 in 8.8.
		b.extend_from_slice(&0u16.to_be_bytes());		// Reserved.
		b.extend_from_slice(&[0u8; 8]);				// Reserved.
		for v in UNITY {
			b.extend_from_slice(&v.to_be_bytes());
		}
		b.extend_from_slice(&[0u8; 24]);		// Pre-defined.
		// One past the highest track id in use, which is what the field is defined as. ffmpeg
		// writes the track count itself -- 2 for two tracks, whose ids are 1 and 2 -- and that is a
		// violation: a tool adding a track would take an id already taken. Written correctly here.
		b.extend_from_slice(&(self.streams.len() as u32 + 1).to_be_bytes());
		bx(b"mvhd", &b)
	}

	/// One track: its header and its media.
	fn trak(&self, i: usize, s: &Stream) -> Outcome<Vec<u8>> {
		let mut body = Vec::new();
		body.extend_from_slice(&res!(self.tkhd(i, s)));
		body.extend_from_slice(&res!(self.mdia(i, s)));
		bx(b"trak", &body)
	}

	/// The track header, ISO/IEC 14496-12 §8.3.2, version 0, stating no duration.
	///
	/// The flags are `0x000003`: enabled, and in the movie. The whole-file writer above sets
	/// `track_in_preview` as well; nothing here writes a preview, so the bit is left clear, and
	/// what matters is `track_enabled` -- a track without it is in the file and played by nothing.
	fn tkhd(&self, i: usize, s: &Stream) -> Outcome<Vec<u8>> {
		let mut b = Vec::with_capacity(84);
		b.extend_from_slice(&full(0, 0x0000_0003));
		b.extend_from_slice(&0u32.to_be_bytes());	// Creation time, unset.
		b.extend_from_slice(&0u32.to_be_bytes());	// Modification time, unset.
		b.extend_from_slice(&(i as u32 + 1).to_be_bytes());	// Track id; zero is not allowed.
		b.extend_from_slice(&0u32.to_be_bytes());	// Reserved.
		b.extend_from_slice(&0u32.to_be_bytes());	// Duration, not yet known.
		b.extend_from_slice(&[0u8; 8]);			// Reserved.
		b.extend_from_slice(&0u16.to_be_bytes());	// Layer: the front.
		match s.media {
			// A sound track is put in alternate group 1 and a picture track in none. The group says
			// "play one of these, not both", which is right for the sound tracks of a film in
			// several languages and wrong for a picture track, whose group must not name any
			// alternative to it.
			Media::Picture { .. }	=> b.extend_from_slice(&0u16.to_be_bytes()),
			Media::Sound { .. }	=> b.extend_from_slice(&1u16.to_be_bytes()),
		}
		match s.media {
			Media::Picture { .. }	=> b.extend_from_slice(&0u16.to_be_bytes()),
			Media::Sound { .. }	=> b.extend_from_slice(&0x0100u16.to_be_bytes()),	// 1.0 in 8.8.
		}
		b.extend_from_slice(&0u16.to_be_bytes());	// Reserved.
		for v in UNITY {
			b.extend_from_slice(&v.to_be_bytes());
		}
		// The presentation size in 16.16 fixed point, and nought by nought for sound, which is
		// drawn nowhere. A non-zero size on a sound track makes some players lay out a blank
		// rectangle for it.
		match s.media {
			Media::Picture { w, h } => {
				b.extend_from_slice(&((w as u32) << 16).to_be_bytes());
				b.extend_from_slice(&((h as u32) << 16).to_be_bytes());
			},
			Media::Sound { .. } => {
				b.extend_from_slice(&0u32.to_be_bytes());
				b.extend_from_slice(&0u32.to_be_bytes());
			},
		}
		bx(b"tkhd", &b)
	}

	/// The media box: the media header, the handler that declares what the track is, and the media
	/// information.
	fn mdia(&self, i: usize, s: &Stream) -> Outcome<Vec<u8>> {
		let mut body = Vec::new();
		body.extend_from_slice(&res!(self.mdhd(s)));
		body.extend_from_slice(&res!(match s.media {
			Media::Picture { .. }	=> handler(b"vide", "VideoHandler"),
			Media::Sound { .. }	=> handler(b"soun", "SoundHandler"),
		}));
		body.extend_from_slice(&res!(self.minf(i, s)));
		bx(b"mdia", &body)
	}

	/// The media header, ISO/IEC 14496-12 §8.4.2, version 0, carrying the stream's own timescale.
	///
	/// The timescale is the stream's and not the movie's, and it is the unit every duration in
	/// every fragment of this track is counted in, so it is the one number here a fragment depends
	/// on being right.
	fn mdhd(&self, s: &Stream) -> Outcome<Vec<u8>> {
		let mut b = Vec::with_capacity(24);
		b.extend_from_slice(&full(0, 0));
		b.extend_from_slice(&0u32.to_be_bytes());	// Creation time, unset.
		b.extend_from_slice(&0u32.to_be_bytes());	// Modification time, unset.
		b.extend_from_slice(&s.timescale.to_be_bytes());
		b.extend_from_slice(&0u32.to_be_bytes());	// Duration, not yet known.
		b.extend_from_slice(&LANG_UND.to_be_bytes());
		b.extend_from_slice(&0u16.to_be_bytes());	// Pre-defined.
		bx(b"mdhd", &b)
	}

	/// The media information box: the media header its kind requires, where the media lives, and
	/// the sample table.
	fn minf(&self, i: usize, s: &Stream) -> Outcome<Vec<u8>> {
		let mut body = Vec::new();
		body.extend_from_slice(&res!(match s.media {
			Media::Picture { .. }	=> vmhd(),
			Media::Sound { .. }	=> smhd(),
		}));
		body.extend_from_slice(&res!(dinf()));
		body.extend_from_slice(&res!(self.stbl(i, s)));
		bx(b"minf", &body)
	}

	/// The sample table: what the samples are, and four empty tables where a whole-file writer
	/// would put the timing, the sizes and the offsets.
	///
	/// No `ctts` and no `stss`. A fragmented film states composition offsets and sync flags once a
	/// sample in its track runs, so a table here would be a second, empty, statement of the same
	/// thing -- and an empty `stss` in particular reads as "no sample is a sync sample", which would
	/// leave a reader with nowhere to begin.
	fn stbl(&self, i: usize, s: &Stream) -> Outcome<Vec<u8>> {
		let mut body = Vec::new();
		body.extend_from_slice(&res!(self.stsd(i, s)));
		body.extend_from_slice(&res!(empty_table(b"stts")));
		body.extend_from_slice(&res!(empty_table(b"stsc")));
		body.extend_from_slice(&res!(empty_stsz()));
		body.extend_from_slice(&res!(empty_table(b"stco")));
		bx(b"stbl", &body)
	}

	/// The sample description: one entry, describing every sample the fragments will carry for this
	/// stream.
	fn stsd(&self, i: usize, s: &Stream) -> Outcome<Vec<u8>> {
		let entry = match s.media {
			Media::Picture { w, h }		=> res!(picture_entry(&s.codec, w, h)),
			Media::Sound { channels, rate }	=> res!(sound_entry(
				&s.codec, i as u32 + 1, channels, rate)),
		};
		let mut b = Vec::with_capacity(8 + entry.len());
		b.extend_from_slice(&full(0, 0));
		b.extend_from_slice(&1u32.to_be_bytes());	// Entry count.
		b.extend_from_slice(&entry);
		bx(b"stsd", &b)
	}

	/// The movie extends box: one `trex` a track, and no `mehd`.
	///
	/// Its presence is what tells a reader that the empty sample tables above are not an empty film
	/// but a film continued in fragments. `mehd` would state the whole duration, which is the one
	/// thing a writer that has not seen the end cannot say.
	fn mvex(&self) -> Outcome<Vec<u8>> {
		let mut body = Vec::new();
		for i in 0..self.streams.len() {
			body.extend_from_slice(&res!(trex(i as u32 + 1)));
		}
		bx(b"mvex", &body)
	}

	/// The movie fragment box: its header, then one track fragment for each entry of `runs`, in the
	/// order given.
	///
	/// `bases` and `offs` are read positionally against `runs`: the decode time each track fragment
	/// begins at, and the offset of its samples from the first byte of this box.
	fn moof(
		&self,
		seq:	u32,
		runs:	&[(usize, Vec<Sample>)],
		bases:	&[u64],
		offs:	&[i32],
	)
		-> Outcome<Vec<u8>>
	{
		let mut body = Vec::new();
		body.extend_from_slice(&res!(mfhd(seq)));
		for (n, (i, samples)) in runs.iter().enumerate() {
			let base = match bases.get(n) {
				Some(t)	=> *t,
				None	=> return Err(err!(
					"Fragment {} has {} track runs and {} decode times to place them at.",
					seq, runs.len(), bases.len();
				Bug, Unreachable)),
			};
			let off = match offs.get(n) {
				Some(o)	=> *o,
				None	=> return Err(err!(
					"Fragment {} has {} track runs and {} data offsets for them.",
					seq, runs.len(), offs.len();
				Bug, Unreachable)),
			};
			body.extend_from_slice(&res!(traf(*i as u32 + 1, base, off, samples)));
		}
		bx(b"moof", &body)
	}
}

/// The file type box of a fragmented film.
///
/// Written separately from [`ftyp`] rather than by a flag on it, because the two make different
/// promises and the whole-file writer's must not move: `avc1` in that list promises a track whose
/// sample table is in `moov`, which is exactly what this file does not have. `iso5` with a minor
/// version of 512, and `iso5`, `iso6` and `mp41` behind it, is what ffmpeg writes for a fragmented
/// file and so is the list the readers of one have been tried against.
///
/// **No `hvc1` for an HEVC film**, for three reasons. The brand is a conformance claim about the
/// whole file (ISO/IEC 14496-15 §8.4.1) and a [`Fragments`] may carry several streams, only one of
/// which is the picture; nothing here is handed the codec anyway, which is the same reason `avc1`
/// is absent; and ffmpeg writes these same three brands and no fourth for a fragmented HEVC file,
/// so adding one would be a list no reader of such a file has been tried against. A reader finds
/// the codec in the sample entry, which is where the answer belongs.
fn ftyp_frag() -> Outcome<Vec<u8>> {
	let mut b = Vec::with_capacity(20);
	b.extend_from_slice(b"iso5");
	b.extend_from_slice(&512u32.to_be_bytes());
	b.extend_from_slice(b"iso5");
	b.extend_from_slice(b"iso6");
	b.extend_from_slice(b"mp41");
	bx(b"ftyp", &b)
}

/// A sample table box with no entries: a full box and a count of nought.
///
/// `stts`, `stsc` and `stco` all take that shape, and a fragmented film writes all three empty.
/// They are written rather than left out because ISO/IEC 14496-12 §8.5.1 requires them in every
/// sample table, and readers that check do refuse a table missing one.
fn empty_table(kind: &[u8; 4]) -> Outcome<Vec<u8>> {
	let mut b = Vec::with_capacity(8);
	b.extend_from_slice(&full(0, 0));
	b.extend_from_slice(&0u32.to_be_bytes());	// Entry count.
	bx(kind, &b)
}

/// The sample size table with nothing in it.
///
/// Not the same shape as the other three: it carries a common size before its count, and both are
/// nought here -- no common size, and no samples to give one to.
fn empty_stsz() -> Outcome<Vec<u8>> {
	let mut b = Vec::with_capacity(12);
	b.extend_from_slice(&full(0, 0));
	b.extend_from_slice(&0u32.to_be_bytes());	// Sizes vary, so each is listed.
	b.extend_from_slice(&0u32.to_be_bytes());	// Sample count.
	bx(b"stsz", &b)
}

/// The visual sample entry of a fragmented film, ISO/IEC 14496-12 §8.5.2, carrying the codec's
/// configuration box.
///
/// The same 78-byte lead-in as the whole-file writer's, with one difference: the compressor name is
/// left as 32 zero bytes. The field is a counted string, so a leading nought is a name of no
/// characters, which is what a file that has nothing to say there should write -- and it is what
/// ffmpeg writes for a fragmented file.
fn picture_entry(codec: &Codec, w: u16, h: u16) -> Outcome<Vec<u8>> {
	if !codec.is_picture() {
		return Err(err!(
			"A visual sample entry was asked for around a sound codec."; Bug, Invalid));
	}
	let mut b = Vec::with_capacity(86);
	b.extend_from_slice(&[0u8; 6]);			// Reserved.
	b.extend_from_slice(&1u16.to_be_bytes());	// Data reference index: the first `dref` entry.
	b.extend_from_slice(&0u16.to_be_bytes());	// Pre-defined.
	b.extend_from_slice(&0u16.to_be_bytes());	// Reserved.
	b.extend_from_slice(&[0u8; 12]);		// Pre-defined.
	b.extend_from_slice(&w.to_be_bytes());
	b.extend_from_slice(&h.to_be_bytes());
	b.extend_from_slice(&RESOLUTION_72.to_be_bytes());
	b.extend_from_slice(&RESOLUTION_72.to_be_bytes());
	b.extend_from_slice(&0u32.to_be_bytes());	// Reserved.
	b.extend_from_slice(&1u16.to_be_bytes());	// Frames a sample: one coded picture each.
	b.extend_from_slice(&[0u8; 32]);		// Compressor name: none given.
	b.extend_from_slice(&0x0018u16.to_be_bytes());	// Depth: colour with no alpha.
	b.extend_from_slice(&0xFFFFu16.to_be_bytes());	// Pre-defined: -1.
	b.extend_from_slice(&res!(bx(codec.config(), codec.record())));
	bx(codec.entry(), &b)
}

/// The sound sample entry, ISO/IEC 14496-12 §8.5.2, carrying the elementary stream descriptor.
///
/// The channel count and the rate are stated here as well as inside the `AudioSpecificConfig` the
/// descriptor carries, and the two must agree: a reader shows what this says and a decoder produces
/// what the configuration says, so a disagreement is a file that reports stereo and plays mono.
/// They are not checked against each other here because parsing the configuration is a decoder's
/// job, and the caller that copied both out of one source container has them consistent already.
fn sound_entry(codec: &Codec, track_id: u32, channels: u16, rate: u32) -> Outcome<Vec<u8>> {
	if codec.is_picture() {
		return Err(err!(
			"A sound sample entry was asked for around a picture codec."; Bug, Invalid));
	}
	if rate >= 1 << 16 {
		return Err(err!(
			"Track {} is sampled at {} Hz, which will not fit the whole part of the 16.16 fixed \
			point field a sound sample entry states its rate in.", track_id, rate;
		Invalid, Input, Excessive));
	}
	let mut b = Vec::with_capacity(64);
	b.extend_from_slice(&[0u8; 6]);			// Reserved.
	b.extend_from_slice(&1u16.to_be_bytes());	// Data reference index: the first `dref` entry.
	b.extend_from_slice(&[0u8; 8]);			// Reserved.
	b.extend_from_slice(&channels.to_be_bytes());
	b.extend_from_slice(&16u16.to_be_bytes());	// Sample size in bits, which this field fixes at 16.
	b.extend_from_slice(&0u16.to_be_bytes());	// Pre-defined.
	b.extend_from_slice(&0u16.to_be_bytes());	// Reserved.
	b.extend_from_slice(&(rate << 16).to_be_bytes());
	b.extend_from_slice(&res!(esds(track_id, codec.record())));
	bx(codec.entry(), &b)
}

/// A descriptor's length, in the four-byte form ISO/IEC 14496-1 §8.3.3 allows.
///
/// A length is a run of bytes carrying seven bits each, the top bit saying another follows, so
/// anything under 128 could be written in one byte. Four are written whatever the length, padded
/// with `0x80` bytes that contribute nothing: that is what ffmpeg writes, and a reader that assumed
/// the fixed width and stepped four bytes on regardless has been met often enough that the padded
/// form is the safe one to write.
fn desc_len(n: usize) -> Outcome<[u8; 4]> {
	if n >= 1 << 28 {
		return Err(err!(
			"A descriptor of {} bytes will not fit the four seven-bit length bytes a descriptor \
			header carries.", n;
		Invalid, Input, Excessive));
	}
	Ok([
		0x80 | ((n >> 21) & 0x7F) as u8,
		0x80 | ((n >> 14) & 0x7F) as u8,
		0x80 | ((n >> 7) & 0x7F) as u8,
		(n & 0x7F) as u8,
	])
}

/// The elementary stream descriptor box of a sound sample entry, ISO/IEC 14496-1 §7.2.6.
///
/// Its contents are a tree of descriptors rather than boxes, so nothing inside is length-prefixed
/// the way the rest of the file is, and each length has to be added up from the ones below it. They
/// are computed here rather than written down: 34 and 20 are right for the two-byte configuration
/// of common AAC-LC and wrong for every longer one, and a wrong length there produces a file whose
/// box tree is perfectly well formed and whose audio decoder will not start.
fn esds(track_id: u32, config: &[u8]) -> Outcome<Vec<u8>> {
	if config.is_empty() {
		return Err(err!(
			"Track {} carries no AudioSpecificConfig, and a decoder cannot be started without one.",
			track_id;
		Invalid, Input, Missing));
	}
	// Each descriptor is one tag byte, four length bytes, and its payload, so a descriptor adds
	// five bytes to whatever it holds.
	let dsi = config.len();
	let dcd = 13 + 5 + dsi;
	let es = 3 + 5 + dcd + 5 + 1;
	let mut b = Vec::with_capacity(4 + 5 + es);
	b.extend_from_slice(&full(0, 0));

	b.push(0x03);					// ES_Descriptor.
	b.extend_from_slice(&res!(desc_len(es)));
	// The elementary stream's id, which is the track's: two numbering schemes for one thing, and
	// they are kept equal so that nothing has to map between them.
	b.extend_from_slice(&(track_id as u16).to_be_bytes());
	b.push(0x00);					// No stream dependence, no URL, no OCR stream, priority 0.

	b.push(0x04);					// DecoderConfigDescriptor.
	b.extend_from_slice(&res!(desc_len(dcd)));
	b.push(0x40);					// Object type: MPEG-4 audio.
	// Stream type 5, AudioStream, in the top six bits; upstream 0; and the reserved bit, which the
	// specification requires set. A zero there is what a hand-written `0x14` gives, and some
	// decoders take the whole descriptor as malformed.
	b.push(0x15);
	b.extend_from_slice(&[0u8; 3]);			// Decoding buffer size, unstated.
	b.extend_from_slice(&0u32.to_be_bytes());	// Maximum bitrate, unstated.
	b.extend_from_slice(&0u32.to_be_bytes());	// Average bitrate, unstated.

	b.push(0x05);					// DecoderSpecificInfo.
	b.extend_from_slice(&res!(desc_len(dsi)));
	b.extend_from_slice(config);

	b.push(0x06);					// SLConfigDescriptor.
	b.extend_from_slice(&res!(desc_len(1)));
	b.push(0x02);					// Predefined: the MP4 file setting.

	bx(b"esds", &b)
}

/// A track extends box, ISO/IEC 14496-12 §8.8.3: the defaults a track fragment falls back on.
///
/// Every default but the sample description index is nought, and nothing falls back on them,
/// because each track run below states every value for every sample. Defaults are how a fragmented
/// file is usually made smaller; they are also how a fragment ends up timed by a value set in a
/// header written hours earlier, and the saving is four bytes a sample.
fn trex(track_id: u32) -> Outcome<Vec<u8>> {
	let mut b = Vec::with_capacity(24);
	b.extend_from_slice(&full(0, 0));
	b.extend_from_slice(&track_id.to_be_bytes());
	b.extend_from_slice(&1u32.to_be_bytes());	// Sample description index; one-based.
	b.extend_from_slice(&0u32.to_be_bytes());	// Default sample duration.
	b.extend_from_slice(&0u32.to_be_bytes());	// Default sample size.
	b.extend_from_slice(&0u32.to_be_bytes());	// Default sample flags.
	bx(b"trex", &b)
}

/// The movie fragment header, ISO/IEC 14496-12 §8.8.5, carrying the fragment's sequence number.
///
/// The numbers count from one and rise by one, which is what lets a reader that has been handed
/// fragments out of order, or has missed one, say so.
fn mfhd(seq: u32) -> Outcome<Vec<u8>> {
	let mut b = Vec::with_capacity(8);
	b.extend_from_slice(&full(0, 0));
	b.extend_from_slice(&seq.to_be_bytes());
	bx(b"mfhd", &b)
}

/// One track's part of a fragment: which track it is, where its decode time has got to, and the
/// run of samples itself.
fn traf(track_id: u32, base: u64, off: i32, samples: &[Sample]) -> Outcome<Vec<u8>> {
	let mut body = Vec::new();
	body.extend_from_slice(&res!(tfhd(track_id)));
	body.extend_from_slice(&res!(tfdt(base)));
	body.extend_from_slice(&res!(trun(off, samples)));
	bx(b"traf", &body)
}

/// The track fragment header, ISO/IEC 14496-12 §8.8.7.
///
/// The flags are `0x020000`, `default-base-is-moof`, and nothing else. That fixes the origin every
/// data offset below is measured from at the first byte of the enclosing `moof`, which is a
/// position the fragment knows about itself -- the alternative bases are file offsets, and a
/// fragment that has been cut out and served on its own no longer knows where in a file it was.
///
/// Deliberately simpler than ffmpeg, which also sets the default duration, size and flags. Every
/// per-sample value is written in the track run instead: four bytes a sample against a class of
/// fault where a sample takes a default nobody meant it to have.
fn tfhd(track_id: u32) -> Outcome<Vec<u8>> {
	let mut b = Vec::with_capacity(8);
	b.extend_from_slice(&full(0, 0x0002_0000));
	b.extend_from_slice(&track_id.to_be_bytes());
	bx(b"tfhd", &b)
}

/// The track fragment decode time, ISO/IEC 14496-12 §8.8.12, version 1.
///
/// The absolute decode time of the fragment's first sample, in the track's own timescale. Version 1
/// carries it in 64 bits: a 32-bit field overflows after thirteen hours at 90 kHz, which is a
/// running time a recording reaches, and the box exists precisely so that a reader handed one
/// fragment can place it without the fragments before it.
fn tfdt(base: u64) -> Outcome<Vec<u8>> {
	let mut b = Vec::with_capacity(12);
	b.extend_from_slice(&full(1, 0));
	b.extend_from_slice(&base.to_be_bytes());
	bx(b"tfdt", &b)
}

/// The track run, ISO/IEC 14496-12 §8.8.8: the samples of this fragment, one row each.
///
/// Version 1, so that the composition offsets are **signed**. That is the point of the version: a
/// version-0 run states them unsigned, so a picture shown before the one decoded before it cannot
/// be expressed without shifting the whole track, and [`Sample::off`] is an `i32` exactly because
/// the shift is the caller's business and not the container's.
///
/// The flags are `0x000F01`: the data offset, then a duration, a size, flags and a composition
/// offset for every sample. Nothing is defaulted, so nothing depends on a header written earlier.
fn trun(off: i32, samples: &[Sample]) -> Outcome<Vec<u8>> {
	let mut b = Vec::with_capacity(16 + samples.len() * 16);
	b.extend_from_slice(&full(1, 0x0000_0F01));
	b.extend_from_slice(&(samples.len() as u32).to_be_bytes());
	b.extend_from_slice(&off.to_be_bytes());
	for s in samples {
		if s.data.len() > u32::MAX as usize {
			return Err(err!(
				"A sample of {} bytes will not fit the 32-bit size a track run states.",
				s.data.len();
			Invalid, Input, Excessive));
		}
		b.extend_from_slice(&s.dur.to_be_bytes());
		b.extend_from_slice(&(s.data.len() as u32).to_be_bytes());
		b.extend_from_slice(&(if s.sync { SAMPLE_SYNC } else { SAMPLE_DELTA }).to_be_bytes());
		b.extend_from_slice(&s.off.to_be_bytes());
	}
	bx(b"trun", &b)
}

/// The file type box, ISO/IEC 14496-12 §4.3.
///
/// `isom` as the major brand with a minor version of 512 is what the reference muxers write, and
/// the compatible brands list `isom`, `iso2`, `avc1` and `mp41`: a reader that knows only the AVC
/// file format, and one that knows only version 1 of MP4, can both see a brand they recognise.
/// The third brand names the coding, and naming the wrong one is a false claim.
///
/// A brand in this list is a statement that the file conforms to that specification, so `avc1` on a
/// film coded in HEVC says something untrue about it -- harmlessly to most readers, and the sort of
/// untruth a strict one is entitled to refuse. ffmpeg agrees it matters: for a whole-file HEVC film
/// it writes `isom iso2 mp41` and drops `avc1`, which it does include for H.264.
fn ftyp(codec: &Codec) -> Outcome<Vec<u8>> {
	let mut b = Vec::with_capacity(24);
	b.extend_from_slice(b"isom");
	b.extend_from_slice(&512u32.to_be_bytes());
	b.extend_from_slice(b"isom");
	b.extend_from_slice(b"iso2");
	match codec {
		Codec::Avc(_)	=> b.extend_from_slice(b"avc1"),
		Codec::Hevc(_)	=> b.extend_from_slice(b"hvc1"),
		// Sound alone claims no picture coding, and the brand count changes with
		// it rather than a placeholder being written.
		Codec::Aac(_)	=> {},
	}
	b.extend_from_slice(b"mp41");
	bx(b"ftyp", &b)
}

/// The handler reference, ISO/IEC 14496-12 §8.4.3, declaring a visual track.
///
/// The trailing name is a null-terminated UTF-8 string. A counted string is written there by some
/// tools, following an older convention, and a reader that takes the specification at its word then
/// shows the count byte as the first character of the name.
fn hdlr() -> Outcome<Vec<u8>> {
	handler(b"vide", "VideoHandler")
}

/// The handler reference for a track of the given kind.
///
/// `vide` for a picture and `soun` for sound, which is the field a reader uses to decide which
/// media header to expect and how to present the track at all.
fn handler(kind: &[u8; 4], name: &str) -> Outcome<Vec<u8>> {
	let mut b = Vec::with_capacity(32 + name.len());
	b.extend_from_slice(&full(0, 0));
	b.extend_from_slice(&0u32.to_be_bytes());	// Pre-defined.
	b.extend_from_slice(kind);
	b.extend_from_slice(&[0u8; 12]);		// Reserved.
	b.extend_from_slice(name.as_bytes());
	b.push(0);
	bx(b"hdlr", &b)
}

/// The sound media header, ISO/IEC 14496-12 §8.4.5.3.
///
/// The counterpart of [`vmhd`], and a track carries exactly one of the two: which one is what
/// `hdlr` has just declared, and a reader meeting the wrong one has been told two different things
/// about the same track.
fn smhd() -> Outcome<Vec<u8>> {
	let mut b = Vec::with_capacity(8);
	b.extend_from_slice(&full(0, 0));
	b.extend_from_slice(&0u16.to_be_bytes());	// Balance: centre.
	b.extend_from_slice(&0u16.to_be_bytes());	// Reserved.
	bx(b"smhd", &b)
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
/// Works out each sample's composition offset from the times a source container states.
///
/// # Why a caller needs this at all
///
/// Matroska, and the other containers a film arrives in, state **only when a picture is shown**.
/// MP4 states when it is decoded and how long after that it is shown. The decoding times are
/// already fixed by the durations the caller is writing -- sample `i` is decoded after the sum of
/// the durations before it -- so the offset is the difference, and this computes it.
///
/// `times` are presentation times **in decode order**, which is the order the frames come out of a
/// container and the order they must be written in, in whatever unit the durations are given in.
///
/// # They must be relative to the track's start, and this does not rebase them
///
/// A composition offset is the distance between a picture's decoding and its showing, and it is
/// meant to be a handful of frames. Decoding here begins at nought, so handing this the *absolute*
/// times of a film that starts an hour in gives every sample an offset of an hour: representable,
/// wrong in meaning, and the sort of thing that plays correctly on the machine that wrote it and
/// puzzles everything else. Subtract the first sample's time before calling, and put where the
/// track actually begins in [`Stream::start`], which is what that field is for.
///
/// This deliberately does **not** rebase, because it cannot tell the two cases apart: the smallest
/// raw difference is a real measurement when a track begins at nought, and rebasing on it would
/// quietly discard a genuine reordering delay.
///
/// # The shift, and why the whole track moves
///
/// A picture may be shown *before* a later-decoded picture that precedes it in decode order, so the
/// raw difference is negative for the pictures at the head of a reordered run: they are decoded
/// early precisely so the ones they are shown between can refer to them. A negative offset says a
/// picture is shown before it is decoded, which is not something a decoder can do.
///
/// So every offset is raised by one constant -- the largest shortfall -- which delays the whole
/// track by that much and leaves the intervals between pictures exactly as they were. Nothing about
/// the film changes but the instant it starts, by a few frames.
pub fn composition_offsets(times: &[i64], durs: &[u32]) -> Outcome<Vec<i32>> {
	if times.len() != durs.len() {
		return Err(err!(
			"There are {} presentation times and {} durations, and each sample needs one of each.",
			times.len(), durs.len();
		Invalid, Input, Mismatch));
	}
	let mut raw = Vec::with_capacity(times.len());
	let mut dts = 0i64;
	let mut least = 0i64;
	for (i, t) in times.iter().enumerate() {
		let d = t - dts;
		if d < least {
			least = d;
		}
		raw.push(d);
		dts += durs[i] as i64;
	}
	let mut out = Vec::with_capacity(raw.len());
	for d in raw {
		let v = d - least;
		if v > i32::MAX as i64 {
			return Err(err!(
				"A composition offset of {} ticks will not fit the 32 bits the table holds, so the \
				presentation times given are not those of one film.", v;
			Invalid, Input, Excessive));
		}
		out.push(v as i32);
	}
	Ok(out)
}

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
	/// The rectangle of the coded picture that is the picture: left, top, width and height.
	aperture:	Option<(u32, u32, u32, u32)>,
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

	/// The rectangle of the coded picture that is actually the picture: left, top, width, height.
	///
	/// `None` where the track states none, which means the whole of it. A phone stabilises a film
	/// by coding a picture larger than it shows and moving a window about inside it; the window is
	/// the clean aperture, and a viewer that ignores it shows about nine per cent more of the
	/// frame than the film means to show, wobbling margin and all.
	pub fn aperture(&self) -> Option<(u32, u32, u32, u32)> {
		self.aperture
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

	/// Reads a film's first video track out of a file, holding none of the film.
	///
	/// This is the form a caller wanting one frame of a large film uses: the top-level boxes are
	/// walked by their headers, the `moov` box alone is read, and the handle is left open so that
	/// [`Film::read_sample`] can fetch the one sample the index names. A film of four gigabytes
	/// costs its metadata, and a caller that cannot hold the file -- which is most callers, since
	/// most films are past any sensible buffer -- is not shut out of its first frame.
	pub fn of(f: &mut File) -> Outcome<Self> {
		let moov = match res!(moov_of(f)) {
			Some(m) => m,
			None => return Err(err!(
				"The file carries no `moov` box, so nothing says where its samples are.";
			Invalid, Input, Missing)),
		};
		Self::from_moov(&moov)
	}

	/// One sample's bytes, read out of the file the index was read from.
	///
	/// The counterpart of [`Film::sample`] for a caller with a handle rather than a buffer. The
	/// offsets in a sample table are absolute file offsets, so this needs nothing of where the
	/// movie box sat.
	pub fn read_sample(&self, f: &mut File, i: usize) -> Outcome<Vec<u8>> {
		let (off, len) = res!(self.span(i));
		if len > SAMPLE_MAX {
			return Err(err!(
				"Sample {} says it is {} bytes long, and {} is this reader's ceiling for one \
				coded picture.", i, len, SAMPLE_MAX;
			Invalid, Input, Excessive));
		}
		let mut buf = vec![0u8; len as usize];
		res!(f.seek(SeekFrom::Start(off)), IO, File);
		res!(f.read_exact(&mut buf), IO, File);
		Ok(buf)
	}

	/// The bytes of the first sample a decoder may begin at.
	///
	/// The one call a poster frame needs: which sample to start at, and its bytes.
	pub fn read_first_sync(&self, f: &mut File) -> Outcome<Vec<u8>> {
		let i = res!(self.first_sync());
		self.read_sample(f, i)
	}
}

// ------------------------------------------------------------- a film's index, out of a file

/// The most bytes a movie box may occupy before the file is called a mistake.
///
/// A `moov` is an index and not media: the sample tables of a track at this reader's ceiling of a
/// million samples come to a few tens of megabytes, and anything past this is a length field read
/// out of the wrong place rather than a film.
pub const MOOV_MAX: u64 = 64 * 1024 * 1024;

/// The most bytes one sample may occupy.
///
/// One coded picture. A 4K intra frame is a few megabytes; this is a ceiling against a length that
/// is a mistake, since the length is what a buffer is sized from.
pub const SAMPLE_MAX: u32 = 64 * 1024 * 1024;

/// How much of a movie header is read back, which is more than either version of the box occupies.
pub const MVHD_BYTES: u64 = 256;

/// The most boxes walked at one level looking for one of them.
const MAX_BOXES: usize = 4096;

/// The body of the first box of a given type between two offsets in a file, as its offset and its
/// length.
///
/// **Only the box headers are read**: eight bytes each, or the sixteen a 64-bit length needs. An
/// `mdat` holding two gigabytes of film is stepped over by its declared length and never touched,
/// which is what makes finding a film's index affordable on a file nobody wants in memory.
///
/// A length that does not fit inside the range being walked ends the walk and answers `None`
/// rather than failing. A file truncated in transfer is a thing to report absence for, and the
/// caller asking this question has a plain answer for absence: the box is not there.
pub fn find_box(f: &mut File, want: &[u8; 4], from: u64, to: u64)
	-> Outcome<Option<(u64, u64)>>
{
	let mut at = from;
	for _ in 0..MAX_BOXES {
		if at + 8 > to {
			return Ok(None);
		}
		res!(f.seek(SeekFrom::Start(at)), IO, File);
		let mut head = [0u8; 16];
		if res!(fill(f, &mut head[..8])) != 8 {
			return Ok(None);
		}
		let size = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as u64;
		let mut kind = [0u8; 4];
		kind.copy_from_slice(&head[4..8]);
		// A size of nought means the box runs to the end of its parent; a size of one means the
		// real length is the eight bytes after the type (ISO/IEC 14496-12 §4.2).
		let (body, next) = match size {
			0 => (at + 8, to),
			1 => {
				if res!(fill(f, &mut head[8..16])) != 8 {
					return Ok(None);
				}
				let mut wide = [0u8; 8];
				wide.copy_from_slice(&head[8..16]);
				let wide = u64::from_be_bytes(wide);
				if wide < 16 {
					return Ok(None);
				}
				(at + 16, at.saturating_add(wide))
			},
			n if n < 8 => return Ok(None),
			n => (at + 8, at.saturating_add(n)),
		};
		if next > to || next <= at || body > next {
			return Ok(None);
		}
		if &kind == want {
			return Ok(Some((body, next - body)));
		}
		at = next;
	}
	Ok(None)
}

/// The body of a film's movie box, lifted out of a file.
///
/// What comes back is the box's **children**, which is what [`Film::from_moov`] reads. QuickTime
/// writes `moov` at the end of a file as often as at the front, and either is reached by the same
/// walk over the top-level headers.
pub fn moov_of(f: &mut File) -> Outcome<Option<Vec<u8>>> {
	let end = res!(f.metadata(), IO, File).len();
	let (body, len) = match res!(find_box(f, b"moov", 0, end)) {
		Some(span) => span,
		None => return Ok(None),
	};
	if len > MOOV_MAX {
		return Err(err!(
			"A movie box of {} bytes, and {} is this reader's ceiling. A `moov` is an index and \
			not media.", len, MOOV_MAX;
		Invalid, Input, Excessive));
	}
	let mut buf = vec![0u8; len as usize];
	res!(f.seek(SeekFrom::Start(body)), IO, File);
	res!(f.read_exact(&mut buf), IO, File);
	Ok(Some(buf))
}

/// The payload of a film's movie header, lifted out of a file.
///
/// The header is `moov`'s first child and carries the timescale, the duration and the times the
/// film was recorded and last changed. Reaching it costs a handful of eight-byte reads and as many
/// seeks, and it is deliberately not the whole of `moov`: a caller asking only how long a film runs
/// should not read a long film's sample tables to find out.
pub fn mvhd_of(f: &mut File) -> Outcome<Option<Vec<u8>>> {
	let end = res!(f.metadata(), IO, File).len();
	let (moov, moov_len) = match res!(find_box(f, b"moov", 0, end)) {
		Some(span) => span,
		None => return Ok(None),
	};
	let (body, len) = match res!(find_box(f, b"mvhd", moov, moov + moov_len)) {
		Some(span) => span,
		None => return Ok(None),
	};
	let take = len.min(MVHD_BYTES) as usize;
	res!(f.seek(SeekFrom::Start(body)), IO, File);
	let mut buf = vec![0u8; take];
	let got = res!(fill(f, &mut buf));
	buf.truncate(got);
	Ok(Some(buf))
}

/// The timescale and the duration a movie header carries: ticks a second, and ticks.
///
/// The two are only meaningful together, since the header counts in units of its own choosing.
/// Version 1 of the box widens both the times and the duration while the timescale stays 32 bits in
/// both. A timescale of nought, a duration of nought, and the all-ones a writer that did not know
/// the duration leaves behind are all absence rather than numbers to divide.
pub fn movie_ticks(mvhd: &[u8]) -> Option<(u32, u64)> {
	let (scale, ticks) = match mvhd.first() {
		Some(0) => {
			if mvhd.len() < 20 {
				return None;
			}
			let scale = u32::from_be_bytes([mvhd[12], mvhd[13], mvhd[14], mvhd[15]]);
			let ticks = u32::from_be_bytes([mvhd[16], mvhd[17], mvhd[18], mvhd[19]]);
			if ticks == u32::MAX {
				return None;
			}
			(scale, ticks as u64)
		},
		Some(1) => {
			if mvhd.len() < 32 {
				return None;
			}
			let scale = u32::from_be_bytes([mvhd[20], mvhd[21], mvhd[22], mvhd[23]]);
			let mut wide = [0u8; 8];
			wide.copy_from_slice(&mvhd[24..32]);
			let ticks = u64::from_be_bytes(wide);
			if ticks == u64::MAX {
				return None;
			}
			(scale, ticks)
		},
		_ => return None,
	};
	if scale == 0 || ticks == 0 {
		None
	} else {
		Some((scale, ticks))
	}
}

/// Reads until the buffer is full or the file ends, answering how many bytes arrived.
///
/// A single `read` may come back short of what was asked for without anything being wrong, and a
/// box header read short by one byte is a film refused for nothing.
fn fill(f: &mut File, buf: &mut [u8]) -> Outcome<usize> {
	let mut got = 0usize;
	while got < buf.len() {
		match res!(f.read(&mut buf[got..]), IO, File) {
			0 => break,
			n => got += n,
		}
	}
	Ok(got)
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
	/// The clean aperture, as its box states it: width, height, and the offsets of its centre.
	clap:		Option<(f64, f64, f64, f64)>,
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
				t.rotation = rotation_of(&bytes[body..end.min(bytes.len())]);
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

/// How far a track header's transformation matrix turns the picture, in degrees clockwise.
///
/// A phone writes the angle it was held at here rather than turning the samples, so this is what a
/// viewer must do with a decoder's output. The matrix sits at a fixed offset that depends on the
/// version -- the version-1 header carries 64-bit times and is twelve bytes longer -- and only the
/// four entries that rotate are read, because a matrix that is not a rotation is not something a
/// picture library can act on anyway. Anything else answers nought, which shows the picture as it
/// was coded.
///
/// `tkhd` is the box's payload, from its version byte onwards.
pub fn rotation_of(tkhd: &[u8]) -> u16 {
	let ver = match tkhd.first() {
		Some(v) => *v,
		None => return 0,
	};
	let at = if ver == 1 { 52 } else { 40 };
	// The matrix is nine values in the order a, b, u, c, d, v, x, y, w (§8.3.2.3), and the four
	// that rotate are a, b, c and d -- which are **not** the first four: the projection entry `u`
	// sits between b and c. Reading four in a row instead takes `u` for `c`, and since `u` is
	// nought in every matrix any camera writes, every rotation then looks like no rotation at all.
	let one = 0x0001_0000i32;
	let mut m = [0i32; 4];
	for (i, off) in [0usize, 4, 12, 16].iter().enumerate() {
		m[i] = match tkhd.get(at + off..at + off + 4) {
			Some(s) => i32::from_be_bytes([s[0], s[1], s[2], s[3]]),
			None => return 0,
		};
	}
	match (m[0], m[1], m[2], m[3]) {
		(0, x, y, 0) if x == one && y == -one	=> 90,
		(x, 0, 0, y) if x == -one && y == -one	=> 180,
		(0, x, y, 0) if x == -one && y == one	=> 270,
		_					=> 0,
	}
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
	let mut clap: Option<(f64, f64, f64, f64)> = None;
	res!(walk(bytes, code, visual + 70, at + size, 0, &mut |kind, _parent, cbody, cend| {
		if matches!(&kind, b"avcC" | b"hvcC") && conf.is_none() {
			conf = Some((cbody, cend));
		}
		// The clean aperture: the rectangle of the coded picture that is the picture. A phone
		// stabilises a film by coding it larger than it shows and moving the window about inside
		// it, and this is where the result is written -- eight rationals, four of which are the
		// width and height and four the offset of the window's centre from the picture's
		// (ISO/IEC 14496-12 §12.1.4.3).
		if &kind == b"clap" && clap.is_none() && cend >= cbody + 32 {
			let mut v = [0i64; 8];
			for (i, n) in v.iter_mut().enumerate() {
				*n = match bytes.get(cbody + i * 4..cbody + i * 4 + 4) {
					Some(s) => i32::from_be_bytes([s[0], s[1], s[2], s[3]]) as i64,
					None => return Ok(false),
				};
			}
			// The denominators are the odd entries, and a zero one is a box to be ignored rather
			// than divided by.
			if v[1] != 0 && v[3] != 0 && v[5] != 0 && v[7] != 0 {
				clap = Some((
					v[0] as f64 / v[1] as f64,
					v[2] as f64 / v[3] as f64,
					v[4] as f64 / v[5] as f64,
					v[6] as f64 / v[7] as f64,
				));
			}
		}
		Ok(false)
	}));
	t.config = conf;
	t.clap = clap;
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
	// The aperture is stated as a size and the offset of its centre from the picture's, so the
	// corner it starts at is worked out here rather than by every caller.
	let aperture = t.clap.and_then(|(w, h, dx, dy)| {
		let (full_w, full_h) = (t.size.0 as f64, t.size.1 as f64);
		if w <= 0.0 || h <= 0.0 || w > full_w || h > full_h {
			return None;
		}
		let x = ((full_w - w) / 2.0 + dx).round();
		let y = ((full_h - h) / 2.0 + dy).round();
		if x < 0.0 || y < 0.0 || x + w > full_w || y + h > full_h {
			return None;
		}
		Some((x as u32, y as u32, w.round() as u32, h.round() as u32))
	});
	Ok(Film {
		kind,
		config,
		width:	t.size.0,
		height:	t.size.1,
		rotation: t.rotation,
		aperture,
		samples,
		sync,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_a_quarter_turn_in_a_track_header_is_read_00() -> Outcome<()> {
		// The four entries that rotate are a, b, c and d, and they are not four in a row: the
		// projection entry `u` sits between b and c, and it is nought in every matrix a camera
		// writes. A reader that takes four in a row therefore answers "no rotation" for every
		// turned film there is, which is what this decoder did until a film held sideways said so.
		let one = 0x0001_0000u32;
		let neg = (-(one as i32)) as u32;
		let head = |a: u32, b: u32, c: u32, d: u32| {
			let mut body = vec![0u8; 84];
			body[40..44].copy_from_slice(&a.to_be_bytes());
			body[44..48].copy_from_slice(&b.to_be_bytes());
			// 48 is `u`, and stays nought.
			body[52..56].copy_from_slice(&c.to_be_bytes());
			body[56..60].copy_from_slice(&d.to_be_bytes());
			body
		};
		req!(rotation_of(&head(0, one, neg, 0)), 90u16, "a quarter turn clockwise");
		req!(rotation_of(&head(neg, 0, 0, neg)), 180u16, "a half turn");
		req!(rotation_of(&head(0, neg, one, 0)), 270u16, "three quarters clockwise");
		req!(rotation_of(&head(one, 0, 0, one)), 0u16, "unity is no turn");
		// A matrix that is not a rotation is shown as it was coded rather than guessed at.
		req!(rotation_of(&head(one, one, one, one)), 0u16, "a matrix that is not a rotation");
		req!(rotation_of(&[]), 0u16, "an empty header");
		Ok(())
	}

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
		// Four brands for either picture coding, so the size is the same and it
		// is the third brand that differs; sound alone drops one and is shorter.
		req!(res!(ftyp(&Codec::Avc(avcc()))).len(), 32usize);
		req!(res!(ftyp(&Codec::Hevc(hvcc(&[(hevc::nal::SPS, &HEVC_SPS[2..])])))).len(), 32usize);
		req!(res!(ftyp(&Codec::Aac(vec![0x11, 0x90]))).len(), 28usize);
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
			res!(t.push(Sample { data: nal(4 + i), dur: d, sync: i == 0, off: 0 }));
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
			res!(t.push(Sample { data: nal(4), dur: 40, sync: i == 0, off: 0 }));
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
			res!(some.push(Sample { data: nal(4), dur: 10, sync: i == 0 || i == 3, off: 0 }));
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
			res!(t.push(Sample { data: nal(n), dur: 60, sync: i == 0, off: 0 }));
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
			res!(t.push(Sample { data: nal(6), dur: 3003, sync: i == 0, off: 0 }));
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
			res!(t.push(Sample { data: nal(20), dur: 40, sync: i == 0, off: 0 }));
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
				res!(t.push(Sample { data: nal(12 + i), dur: 40, sync: i % 4 == 0, off: 0 }));
			}
			t.finish()
		};
		req!(res!(build()), res!(build()));
		Ok(())
	}
	#[test]
	fn test_a_written_film_reads_back_17() -> Outcome<()> {
		// The two halves of this module, held to each other. The writer lays out a sample table and
		// the reader walks it back, and what they must agree on is the thing neither can check
		// alone: where each sample's bytes actually are. A chunk offset measured from the wrong
		// origin, or a sample-to-chunk run read as though its first chunk were counted from nought,
		// produces a perfectly well-formed index that points at the wrong bytes.
		let mut t = res!(Track::new(64, 48, 1000, Codec::Avc(avcc())));
		let mut wrote: Vec<Vec<u8>> = Vec::new();
		for i in 0..6usize {
			let data = nal(20 + i);
			wrote.push(data.clone());
			res!(t.push(Sample { data, dur: 40, sync: i == 0, off: 0 }));
		}
		let file = res!(t.finish());
		let film = res!(Film::read(&file));
		req!(film.kind(), Kind::Avc);
		req!(film.samples(), wrote.len());
		req!(film.size(), (64u16, 48u16));
		for (i, want) in wrote.iter().enumerate() {
			let got = res!(film.sample(&file, i));
			req!(got, &want[..], "sample {} came back from the wrong place", i);
		}
		// The first sample is the only sync sample, and a reader must begin there.
		req!(res!(film.first_sync()), 0usize);
		// A sample past the end is refused rather than answered.
		req!(film.span(wrote.len()).is_err(), true, "a sample past the end was handed out");
		Ok(())
	}

	#[test]
	fn test_a_track_that_is_turned_says_so_18() -> Outcome<()> {
		// The writer writes a unity matrix, so a film it wrote is shown as it was coded. What is
		// asserted here is the reading of the four entries that matter, because the fault this
		// guards against hides perfectly: a picture turned by ninety degrees has exactly as many
		// samples as one that is not, so a decoder and a viewer that disagree about the angle
		// produce output of the right size and the wrong shape.
		let mut t = res!(Track::new(64, 48, 1000, Codec::Avc(avcc())));
		res!(t.push(Sample { data: nal(24), dur: 40, sync: true, off: 0 }));
		let mut file = res!(t.finish());
		req!(res!(Film::read(&file)).rotation(), 0u16, "a unity matrix was read as a rotation");

		// Turn it a quarter clockwise by writing the matrix a phone would: a = 0, b = 1, c = -1,
		// d = 0, in 16.16 fixed point.
		//
		// **The four are not four in a row.** The matrix is a, b, u, c, d, v, x, y, w, so c and d
		// are the fourth and fifth entries; this test used to write them third and fourth, which
		// is exactly where the reader used to look for them, so the two agreed with each other and
		// with no real film. Every rotated film in a library of seven thousand was read as
		// upright. The positions below are the specification's.
		let at = match file.windows(4).position(|w| w == b"tkhd") {
			Some(at) => at + 4 + 40,
			None => return Err(err!("the writer emitted no track header."; Test, Missing)),
		};
		let put = |file: &mut Vec<u8>, i: usize, v: i32| {
			file[at + i * 4..at + i * 4 + 4].copy_from_slice(&(v as u32).to_be_bytes());
		};
		let one = 0x0001_0000i32;
		let matrix = |file: &mut Vec<u8>, a: i32, b: i32, c: i32, d: i32| {
			put(file, 0, a);
			put(file, 1, b);
			put(file, 3, c);
			put(file, 4, d);
		};
		matrix(&mut file, 0, one, -one, 0);
		req!(res!(Film::read(&file)).rotation(), 90u16);
		// And a half turn.
		matrix(&mut file, -one, 0, 0, -one);
		req!(res!(Film::read(&file)).rotation(), 180u16);
		// And three quarters.
		matrix(&mut file, 0, -one, one, 0);
		req!(res!(Film::read(&file)).rotation(), 270u16);
		Ok(())
	}

	/// The presentation times are a real film's, read off the first frames of
	/// `Dominion2018.mkv`: shown at 0, 160, 80, 40, 120 while decoded at 0, 40,
	/// 80, 120, 160. A stream that never reordered would state the same list
	/// twice.
	#[test]
	fn test_a_reordered_run_is_offset_forwards_19() -> Outcome<()> {
		let times = [0i64, 160, 80, 40, 120];
		let durs = [40u32; 5];
		let offs = res!(composition_offsets(&times, &durs));

		// Not one of them may be negative, whatever the film does: a negative
		// offset says a picture is shown before it is decoded.
		for (i, o) in offs.iter().enumerate() {
			assert!(*o >= 0, "offset {} of sample {} is negative", o, i);
		}
		// And the intervals must survive: every picture keeps its distance from
		// every other, so the whole run is the source's list plus one constant.
		let mut dts = 0i64;
		let mut shift = None;
		for i in 0..times.len() {
			let shown = dts + offs[i] as i64;
			match shift {
				None => shift = Some(shown - times[i]),
				Some(s) => req!(shown - times[i], s),
			}
			dts += durs[i] as i64;
		}
		req!(shift, Some(80i64));
		Ok(())
	}

	/// A track whose pictures are shown in the order they are decoded must
	/// carry no `ctts` at all -- an absent box is the statement that the two
	/// orders agree, and a table of zeroes says it again for four bytes a
	/// sample.
	#[test]
	fn test_a_stream_in_order_writes_no_offset_table_20() -> Outcome<()> {
		let times = [0i64, 40, 80, 120];
		let durs = [40u32; 4];
		let offs = res!(composition_offsets(&times, &durs));
		req!(offs, vec![0i32, 0, 0, 0]);

		let mut t = res!(Track::new(64, 48, 1000, Codec::Avc(avcc())));
		for i in 0..4 {
			res!(t.push(Sample { data: nal(8), dur: 40, sync: i == 0, off: 0 }));
		}
		req!(res!(t.ctts()).is_none(), true);
		Ok(())
	}

	/// An `AudioSpecificConfig` for AAC-LC at 44,100 Hz in two channels, ISO/IEC 14496-3 §1.6.2.1:
	/// object type 2 in five bits, sampling frequency index 4 in four, channel configuration 2 in
	/// four, and three bits of padding.
	const AAC_LC_44100_STEREO: [u8; 2] = [0x12, 0x10];

	/// A picture stream of the fixture geometry, on a millisecond timescale.
	fn picture_stream() -> Stream {
		Stream {
			media:		Media::Picture { w: 64, h: 48 },
			timescale:	1000,
			codec:		Codec::Avc(avcc()),
			start:		0,
		}
	}

	/// A sound stream on its own sampling rate as its timescale, beginning after the pictures do.
	fn sound_stream(start: u64) -> Stream {
		Stream {
			media:		Media::Sound { channels: 2, rate: 44_100 },
			timescale:	44_100,
			codec:		Codec::Aac(AAC_LC_44100_STEREO.to_vec()),
			start,
		}
	}

	/// How many boxes of the given type sit at the top level of a box body.
	fn count_boxes(buf: &[u8], kind: &[u8; 4]) -> Outcome<usize> {
		let mut at = 0usize;
		let mut n = 0usize;
		while at + 8 <= buf.len() {
			let size = u32::from_be_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]]) as usize;
			if size < 8 || at + size > buf.len() {
				return Err(err!(
					"A box of {} bytes at offset {} does not fit the {} bytes given.",
					size, at, buf.len();
				Test, Invalid));
			}
			if &buf[at + 4..at + 8] == kind {
				n += 1;
			}
			at += size;
		}
		Ok(n)
	}

	/// Each track fragment of a fragment, as the decode time its `tfdt` states and the `data_offset`
	/// its `trun` states, in the order the track fragments are written.
	///
	/// The boxes are walked by their sizes rather than found by searching for their names, because
	/// a fragment holds two of each and a search finds only the first.
	fn frag_runs(frag: &[u8]) -> Outcome<Vec<(u64, i32)>> {
		if frag.len() < 8 || &frag[4..8] != b"moof" {
			return Err(err!("A fragment must begin with a movie fragment box."; Test, Invalid));
		}
		let end = u32::from_be_bytes([frag[0], frag[1], frag[2], frag[3]]) as usize;
		if end > frag.len() {
			return Err(err!(
				"The movie fragment box claims {} bytes and the whole fragment is {}.",
				end, frag.len();
			Test, Invalid));
		}
		let mut out = Vec::new();
		let mut at = 8usize;
		while at + 8 <= end {
			let n = u32::from_be_bytes([frag[at], frag[at + 1], frag[at + 2], frag[at + 3]]) as usize;
			if n < 8 || at + n > end {
				return Err(err!(
					"A box of {} bytes at offset {} does not fit the movie fragment.", n, at;
				Test, Invalid));
			}
			if &frag[at + 4..at + 8] == b"traf" {
				let mut time: Option<u64> = None;
				let mut off: Option<i32> = None;
				let mut k = at + 8;
				while k + 8 <= at + n {
					let m = u32::from_be_bytes([frag[k], frag[k + 1], frag[k + 2], frag[k + 3]])
						as usize;
					if m < 8 || k + m > at + n {
						return Err(err!(
							"A box of {} bytes at offset {} does not fit the track fragment.", m, k;
						Test, Invalid));
					}
					// A `tfdt` body is a full box and a 64-bit time; a `trun` body is a full box,
					// the sample count, and then the offset.
					if &frag[k + 4..k + 8] == b"tfdt" && m >= 20 {
						let o = k + 12;
						time = Some(u64::from_be_bytes([
							frag[o], frag[o + 1], frag[o + 2], frag[o + 3],
							frag[o + 4], frag[o + 5], frag[o + 6], frag[o + 7],
						]));
					}
					if &frag[k + 4..k + 8] == b"trun" && m >= 20 {
						let o = k + 16;
						off = Some(i32::from_be_bytes([
							frag[o], frag[o + 1], frag[o + 2], frag[o + 3],
						]));
					}
					k += m;
				}
				match (time, off) {
					(Some(t), Some(o))	=> out.push((t, o)),
					_			=> return Err(err!(
						"The track fragment at offset {} carries no decode time or no track run.",
						at;
					Test, Missing)),
				}
			}
			at += n;
		}
		Ok(out)
	}

	/// The initialisation segment describes every stream once: a `trak` each under `moov` and a
	/// `trex` each under `mvex`, and a next track id one past the highest in use.
	///
	/// A missing `trex` is the failure worth guarding: the file opens, the track is listed, and
	/// every fragment of it is ignored, because without the extends box the empty sample tables in
	/// `moov` are the whole of what the track is said to hold.
	#[test]
	fn test_a_fragmented_head_describes_every_stream_21() -> Outcome<()> {
		let f = res!(Fragments::new(vec![picture_stream(), sound_stream(0)]));
		let head = res!(f.head());

		let ftyp = match top(&head, b"ftyp") {
			Some(b)	=> b,
			None	=> return Err(err!("No file type box was written."; Test, Missing)),
		};
		req!(&ftyp[0..4], b"iso5" as &[u8], "the major brand is not the fragmented one");

		let moov = match top(&head, b"moov") {
			Some(b)	=> b,
			None	=> return Err(err!("No movie box was written."; Test, Missing)),
		};
		req!(res!(count_boxes(&moov, b"trak")), 2usize, "one track a stream");
		req!(res!(count_boxes(&moov, b"mvex")), 1usize, "one movie extends box");

		let mvex = match top(&moov, b"mvex") {
			Some(b)	=> b,
			None	=> return Err(err!("No movie extends box was written."; Test, Missing)),
		};
		req!(res!(count_boxes(&mvex, b"trex")), 2usize, "one track extends box a stream");

		// The next track id is the last field of the movie header, and it must exceed both ids in
		// use rather than count them.
		let mvhd = match top(&moov, b"mvhd") {
			Some(b)	=> b,
			None	=> return Err(err!("No movie header was written."; Test, Missing)),
		};
		let n = mvhd.len();
		req!(u32::from_be_bytes([mvhd[n - 4], mvhd[n - 3], mvhd[n - 2], mvhd[n - 1]]), 3u32);
		Ok(())
	}

	/// The first track run's data offset clears the movie fragment box and its media header, and
	/// the media box holds exactly the samples.
	///
	/// The offset is measured from the first byte of the `moof`, because `default-base-is-moof` is
	/// the only flag the track fragment header sets. An offset measured from anywhere else is a
	/// file that opens, reports the right number of frames, and decodes rubbish.
	#[test]
	fn test_the_first_data_offset_clears_the_moof_22() -> Outcome<()> {
		let mut f = res!(Fragments::new(vec![picture_stream()]));
		let sizes = [30usize, 9, 17];
		let mut samples = Vec::new();
		for (i, n) in sizes.into_iter().enumerate() {
			samples.push(Sample { data: nal(n), dur: 40, sync: i == 0, off: 0 });
		}
		let frag = res!(f.next(vec![(0, samples)]));

		req!(&frag[4..8], b"moof" as &[u8]);
		let moof = u32::from_be_bytes([frag[0], frag[1], frag[2], frag[3]]) as usize;
		let runs = res!(frag_runs(&frag));
		req!(runs.len(), 1usize);
		req!(runs[0].1, moof as i32 + 8, "the data offset does not clear the moof and mdat header");

		// The media box follows the movie fragment immediately, and its payload is the samples and
		// nothing else: each is its own bytes with a four-byte NAL length in front.
		req!(&frag[moof + 4..moof + 8], b"mdat" as &[u8]);
		let payload: usize = sizes.iter().map(|n| n + 4).sum();
		req!(u32::from_be_bytes([
			frag[moof], frag[moof + 1], frag[moof + 2], frag[moof + 3],
		]) as usize, payload + 8);
		req!(frag.len(), moof + 8 + payload);

		// And the offset names the first byte of the first sample, which is its NAL length field.
		let at = runs[0].1 as usize;
		req!(u32::from_be_bytes([
			frag[at], frag[at + 1], frag[at + 2], frag[at + 3],
		]) as usize, sizes[0]);
		Ok(())
	}

	/// A second track fragment's data offset is the first's plus the first's sample bytes, and each
	/// fragment states where its own streams have got to.
	///
	/// The two halves are one fault in two places. Both offsets are measured from the same origin,
	/// so the second is only right if the first's samples have been counted exactly; and both
	/// streams' decode times carry on across fragments, each in its own timescale and from its own
	/// start, so a stream that began late must still be late in the second fragment.
	#[test]
	fn test_a_later_traf_starts_after_the_earlier_bytes_23() -> Outcome<()> {
		let mut f = res!(Fragments::new(vec![picture_stream(), sound_stream(512)]));
		let pictures = |first: bool| -> Vec<Sample> {
			let mut v = Vec::new();
			for i in 0..3usize {
				v.push(Sample { data: nal(20 + i), dur: 40, sync: first && i == 0, off: 0 });
			}
			v
		};
		let sound = || -> Vec<Sample> {
			vec![Sample::key(vec![0x21; 30], 1024), Sample::key(vec![0x21; 27], 1024)]
		};
		let first = res!(f.next(vec![(0, pictures(true)), (1, sound())]));

		let moof = u32::from_be_bytes([first[0], first[1], first[2], first[3]]) as usize;
		let runs = res!(frag_runs(&first));
		req!(runs.len(), 2usize);
		req!(runs[0].1, moof as i32 + 8);
		// The pictures come to three NAL units of 20, 21 and 22 bytes, each with a four-byte length
		// in front of it: 75 bytes, after which the sound begins.
		req!(runs[1].1 - runs[0].1, 75i32, "the second run does not follow the first's bytes");
		req!(runs[0].0, 0u64, "the pictures do not begin at nought");
		req!(runs[1].0, 512u64, "the sound does not begin where its stream says");

		// The second fragment carries the next sequence number, and each stream's decode time has
		// moved on by that stream's own durations: three pictures of 40 ticks at 1000 a second, and
		// two sound frames of 1024 at 44,100.
		let second = res!(f.next(vec![(0, pictures(false)), (1, sound())]));
		let seq = res!(want_box(&second, b"mfhd"));
		req!(u32::from_be_bytes([
			second[seq + 4], second[seq + 5], second[seq + 6], second[seq + 7],
		]), 2u32);
		let runs = res!(frag_runs(&second));
		req!(runs.len(), 2usize);
		req!(runs[0].0, 120u64);
		req!(runs[1].0, 512u64 + 2048);
		Ok(())
	}

	/// The sequence parameter set of a 96 by 64 HEVC stream, as libx265 wrote it: the NAL unit of
	/// type 33 out of `ffmpeg -f lavfi -i testsrc=size=96x64:rate=10 -frames:v 2 -pix_fmt yuv420p
	/// -c:v libx265 -f hevc`. Main profile, 8-bit 4:2:0, one temporal layer.
	///
	/// Four of its bytes are emulation prevention -- the `03` of each `00 00 03`, at offsets 7, 12,
	/// 15 and 33 -- and the first of them sits well before the field that codes the width, so a
	/// reading that left them in place answers some other size rather than failing. The unescaped
	/// payload happens to carry no `03` at all, so the opposite fault, unescaping twice, is a
	/// no-op on this particular set and is **not** exercised by it. The two bytes at the front are
	/// the NAL unit header, which is what a record's parameter set array carries.
	const HEVC_SPS: [u8; 40] = [
		0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x90,
		0x00, 0x00, 0x03, 0x00, 0x00, 0x03, 0x00, 0x1E, 0xA0, 0x30,
		0x81, 0x05, 0x96, 0x56, 0x69, 0x24, 0xCA, 0xF0, 0x16, 0x80,
		0x80, 0x00, 0x00, 0x03, 0x00, 0x80, 0x00, 0x00, 0x05, 0x04,
	];

	/// The matching picture parameter set, from the same stream.
	///
	/// Nothing under test reads it. It is here so that the record has the shape a real one has, and
	/// so that the refusal below has a parameter set to carry that is not a sequence parameter set.
	const HEVC_PPS: [u8; 7] = [0x44, 0x01, 0xC1, 0x72, 0xB4, 0x22, 0x40];

	/// The 22 fixed fields of an `HEVCDecoderConfigurationRecord`, copied verbatim out of the
	/// `hvcC` box ffmpeg wrote when it muxed that same stream into an MP4.
	///
	/// Version 1, then the profile, tier and level of the sets above, then the sampling and the
	/// depths, and finally `0x0F`: one temporal layer, nested, and a four-byte NAL length.
	const HVCC_HEAD: [u8; 22] = [
		0x01, 0x01, 0x60, 0x00, 0x00, 0x00, 0x90, 0x00, 0x00, 0x00, 0x00,
		0x00, 0x1E, 0xF0, 0x00, 0xFC, 0xFD, 0xF8, 0xF8, 0x00, 0x00, 0x0F,
	];

	/// A record around the given parameter sets, each as `(NAL unit type, bytes)`, one array
	/// apiece: the array's type byte, a count of one, and the set behind a two-byte length.
	///
	/// The type byte's top bit is `array_completeness`, which is set because a `hvc1` entry states
	/// that these are all the sets there are; the bit below it is reserved and nought. That is the
	/// byte ffmpeg writes -- `0xA0`, `0xA1`, `0xA2` for the three arrays of the record above.
	fn hvcc(sets: &[(u8, &[u8])]) -> Vec<u8> {
		let mut rec = HVCC_HEAD.to_vec();
		rec.push(sets.len() as u8);
		for &(kind, set) in sets {
			rec.push(0x80 | kind);
			rec.extend_from_slice(&1u16.to_be_bytes());	// One set in this array.
			rec.extend_from_slice(&(set.len() as u16).to_be_bytes());
			rec.extend_from_slice(set);
		}
		rec
	}

	/// An `hvcC` gives the size its sequence parameter set codes, and a track is held to it.
	///
	/// The size is not this crate's: 96 by 64 is what FFmpeg was given on the command line that
	/// produced the parameter set, and what `ffprobe` reads back out of the stream it produced.
	/// The set carries emulation prevention bytes before the field that codes the width, so a
	/// reading that left them in place answers some other size rather than failing.
	#[test]
	fn test_an_hvcc_yields_the_size_its_sps_codes_24() -> Outcome<()> {
		let sets: [(u8, &[u8]); 2] = [(33, &HEVC_SPS[..]), (34, &HEVC_PPS[..])];
		let c = Codec::Hevc(hvcc(&sets));
		req!(res!(c.geometry()), (96u16, 64u16));
		req!(res!(c.nal_len()), 4usize);
		req!(c.is_picture(), true);
		req!(c.entry(), b"hvc1");
		req!(c.config(), b"hvcC");

		// A track is accepted at the size the set codes and refused at any other, which is the
		// whole-file writer's existing check working over HEVC with nothing added to it.
		req!(Track::new(96, 64, 1000, c.clone()).is_ok(), true, "the coded size was refused");
		req!(Track::new(64, 48, 1000, c.clone()).is_err(), true,
			"a track declared 64 by 48 over a 96 by 64 stream was accepted");

		// And the tiling check is reached for HEVC as it is for AVC. An elementary stream handed
		// straight through is the fault worth naming: it produces a file every demuxer accepts and
		// no decoder plays.
		let mut t = res!(Track::new(96, 64, 1000, c));
		match t.push(Sample::key(vec![0, 0, 0, 1, 0x26, 0x01, 0xAF], 40)) {
			Ok(_)	=> return Err(err!("An Annex B HEVC sample was accepted."; Test)),
			Err(e)	=> req!(e.to_string().contains("Annex B"), true,
				"The message does not name the format that was handed over."),
		}
		Ok(())
	}

	/// A record carrying no sequence parameter set is refused, and the refusal names the type that
	/// is missing and the types that are there.
	///
	/// Naming both is the point. "No geometry" on its own leaves a caller guessing whether the
	/// record was empty, truncated, or full of the wrong sets, and the three are fixed differently.
	#[test]
	fn test_an_hvcc_with_no_sps_is_refused_by_name_25() -> Outcome<()> {
		let only_pps: [(u8, &[u8]); 1] = [(34, &HEVC_PPS[..])];
		let msg = match Codec::Hevc(hvcc(&only_pps)).geometry() {
			Ok((w, h))	=> return Err(err!(
				"A record carrying only a picture parameter set gave a geometry of {} by {}.",
				w, h; Test)),
			Err(e)		=> e.to_string(),
		};
		req!(msg.contains("33"), true, "The message does not name the set that is missing.");
		req!(msg.contains("34"), true, "The message does not name what the record does carry.");

		let msg = match Codec::Hevc(hvcc(&[])).geometry() {
			Ok((w, h))	=> return Err(err!(
				"A record carrying no parameter sets at all gave a geometry of {} by {}.",
				w, h; Test)),
			Err(e)		=> e.to_string(),
		};
		req!(msg.contains("no parameter sets at all"), true,
			"The message does not say that the record carries nothing.");
		Ok(())
	}
}
