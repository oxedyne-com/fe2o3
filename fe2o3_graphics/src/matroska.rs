//! Matroska: the size, the running time and the streams of a film in an EBML
//! container.
//!
//! Matroska is what a film collection is mostly written in, and `.mkv` is the
//! extension a catalogue meets far more often than `.avi`. Like [`crate::avi`]
//! this reads the header and nothing else: **no Matroska is decoded here**. What
//! is inside one is H.264, HEVC or AV1, and a caller wanting a frame has a
//! decoder for that stream's own codec or has none.
//!
//! What a catalogue needs is the size of the picture, how long it runs, and
//! **what every stream is coded in** -- the last because a browser will play a
//! film or refuse it on the strength of the codec names alone, and a library
//! that cannot say which of its films will play is a library that offers a
//! black rectangle. So every track is reported, not only the picture: the
//! audio, where AC-3 is common and no browser decodes it, and the subtitles,
//! which are worth offering separately.
//!
//! # The shape of the file
//!
//! EBML is a tree of elements, each an identifier, a length, and that many
//! bytes. Both identifier and length are variable-width integers whose first
//! byte says how wide they are, by the position of its highest set bit: a
//! leading `1` is one byte, `01` two, and so on. An identifier keeps those
//! marker bits -- they are part of it -- while a length drops them, and a
//! length whose value bits are all set means *unknown*, which is how a file
//! still being written says "to the end".
//!
//! Every element that matters here is near the front:
//!
//! - `EBML`, holding `DocType`, which separates Matroska from WebM.
//! - `Segment`, the body of the file, holding
//!   - `Info` -- `TimestampScale` and `Duration`, whose product is the running
//!     time; the duration is a *float* in scale units, not a count of anything.
//!   - `Tracks` -- a `TrackEntry` a stream, each with its `TrackType`,
//!     `CodecID`, language, and, for a picture, its pixel size.
//!
//! [`Matroska::read`] skips the `Cluster`s, which are the film itself, by their
//! length without looking at them. [`Clusters`] is the other half, for a caller
//! that wants the coded frames rather than the description -- repackaging the
//! streams into another container, which needs every frame and decodes none of
//! them.
//!
//! # References
//!
//! IETF RFC 8794 for EBML, and the Matroska specification (RFC 9559) for the
//! element identifiers and the meaning of `TimestampScale`.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

// the first four bytes of every EBML file: the EBML element's identifier
const MAGIC: [u8; 4] = [0x1A, 0x45, 0xDF, 0xA3];

// The most elements walked at one level before a file is given up on. Without a
// bound, a length of nought -- which a truncated or malformed file readily
// supplies -- is an endless walk. A Segment holds thousands of clusters, so this
// must be generous enough that Tracks is still reached in a file that puts
// clusters before it.
const ELEMENT_LIMIT: usize = 4096;

// The deepest the walk descends. Segment → Tracks → TrackEntry → Video is four,
// and nothing wanted here is deeper. A file claiming an element contains itself
// is one that would otherwise be walked for ever.
const DEPTH_LIMIT: usize = 6;

// The default TimestampScale, in nanoseconds, where a file states none. A
// million nanoseconds is a millisecond, so a duration in scale units is a
// duration in milliseconds unless the file says otherwise. Nearly every file
// leaves it at this and states it anyway.
const DEFAULT_SCALE: u64 = 1_000_000;

const ID_EBML:				u64 = 0x1A45DFA3;
const ID_DOC_TYPE:			u64 = 0x4282;
const ID_SEGMENT:			u64 = 0x18538067;
const ID_INFO:				u64 = 0x1549A966;
const ID_TIMESTAMP_SCALE:	u64 = 0x2AD7B1;
const ID_DURATION:			u64 = 0x4489;
const ID_TITLE:				u64 = 0x7BA9;
const ID_TRACKS:			u64 = 0x1654AE6B;
const ID_TRACK_ENTRY:		u64 = 0xAE;
const ID_TRACK_NUMBER:		u64 = 0xD7;
const ID_TRACK_TYPE:		u64 = 0x83;
const ID_CODEC_ID:			u64 = 0x86;
const ID_CODEC_PRIVATE:		u64 = 0x63A2;
const ID_NAME:				u64 = 0x536E;
const ID_LANGUAGE:			u64 = 0x22B59C;
const ID_LANGUAGE_BCP47:	u64 = 0x22B59D;
const ID_FLAG_DEFAULT:		u64 = 0x88;
const ID_DEFAULT_DURATION:	u64 = 0x23E383;
const ID_FLAG_FORCED:		u64 = 0x55AA;
const ID_VIDEO:				u64 = 0xE0;
const ID_PIXEL_W:			u64 = 0xB0;
const ID_PIXEL_H:			u64 = 0xBA;
const ID_DISPLAY_W:			u64 = 0x54B0;
const ID_DISPLAY_H:			u64 = 0x54BA;
const ID_AUDIO:				u64 = 0xE1;
const ID_CHANNELS:			u64 = 0x9F;
const ID_SAMPLING:			u64 = 0xB5;
const ID_CLUSTER:			u64 = 0x1F43B675;
const ID_TIMESTAMP:			u64 = 0xE7;
const ID_SIMPLE_BLOCK:		u64 = 0xA3;
const ID_BLOCK_GROUP:		u64 = 0xA0;
const ID_BLOCK:				u64 = 0xA1;
const ID_REFERENCE_BLOCK:	u64 = 0xFB;

// The widest element header: a four-byte identifier and an eight-byte length.
// What a caller must have in hand before Clusters::feed can say anything about
// the element in front of it, and therefore the smallest useful window.
const HEADER_MAX: usize = 12;

/// What a stream is for.
///
/// The numbers are the file's own. `Other` keeps a value this reader does not
/// name rather than discarding it, because a track it cannot classify is still
/// a track that is there.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackKind {
	Video,
	Audio,
	Subtitle,
	Other(u64),	// logos, buttons, control and metadata tracks, and anything later
}

impl TrackKind {

	/// The kind a `TrackType` value names.
	fn of(n: u64) -> Self {
		match n {
			1	=> Self::Video,
			2	=> Self::Audio,
			0x11	=> Self::Subtitle,
			other	=> Self::Other(other),
		}
	}

	/// The short name used in the interface.
	pub fn name(&self) -> &'static str {
		match self {
			Self::Video		=> "video",
			Self::Audio		=> "audio",
			Self::Subtitle	=> "subtitle",
			Self::Other(_)	=> "other",
		}
	}
}

/// One stream, as its `TrackEntry` describes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
	number:		u64,				// the file's own, which its blocks are keyed by
	kind:		Option<TrackKind>,	// absent until the file says
	codec:		String,				// as the file names it: V_MPEG4/ISO/AVC, A_AC3
	// The codec's own configuration -- an avcC or hvcC record for a picture.
	// Kept because it is what a caller repackaging the stream into another
	// container must copy across, and it is tens of bytes.
	private:	Vec<u8>,
	w:			u32,				// pixel size, nought if not a picture
	h:			u32,
	dw:			u32,				// the shape to show it at, if it differs
	dh:			u32,
	channels:	u64,				// how many channels the audio carries
	rate:		f64,				// samples a second, which a repackager must state
	lang:		String,				// as the file states it, preferring a BCP 47 tag
	name:		String,				// what a player shows for the stream
	default:	bool,				// should a player choose it unasked?
	forced:		bool,				// must a player show it whatever was asked for?
	// Nanoseconds one frame of this stream lasts, wanted for one reason and it
	// is not decorative: the frames of a laced block are spaced by it. A block
	// carrying six frames of sound states one timestamp, and the five after
	// the first are that stamp plus one, two, three of these. Without it they
	// all appear at the same instant and a repackaged film's sound walks away
	// from its picture.
	frame_ns:	u64,
}

impl Track {

	pub fn number(&self) -> u64 { self.number }

	pub fn kind(&self) -> Option<TrackKind> { self.kind }

	/// The codec, as the file names it.
	///
	/// The name is not interpreted here. A caller deciding whether it can play
	/// the stream is the one that knows, and the names are stable strings the
	/// specification fixes.
	pub fn codec(&self) -> &str { &self.codec }

	/// The codec's own configuration record, empty where the file carried none.
	pub fn private(&self) -> &[u8] { &self.private }

	/// The size of the picture in pixels, nought where the stream is not one.
	pub fn size(&self) -> (u32, u32) { (self.w, self.h) }

	/// The shape the picture is meant to be shown at.
	///
	/// Falls back to the pixel size, which is what a file states nothing means.
	pub fn display(&self) -> (u32, u32) {
		let w = if self.dw > 0 { self.dw } else { self.w };
		let h = if self.dh > 0 { self.dh } else { self.h };
		(w, h)
	}

	/// How many channels the audio carries, nought where the file said none.
	pub fn channels(&self) -> u64 { self.channels }

	/// Samples a second, nought where the file said none.
	pub fn rate(&self) -> f64 { self.rate }

	/// The language the file states, empty where it states none.
	pub fn language(&self) -> &str { &self.lang }

	/// The name a player shows for the stream, empty where it has none.
	pub fn name(&self) -> &str { &self.name }

	/// Should a player choose this stream without being asked?
	pub fn is_default(&self) -> bool { self.default }

	/// Must a player show it whatever the viewer asked for?
	pub fn is_forced(&self) -> bool { self.forced }

	/// Nanoseconds one frame lasts, nought where the file states none.
	///
	/// See the field: this is what spaces the frames of a laced block.
	pub fn frame_nanos(&self) -> u64 { self.frame_ns }
}

/// What a film's header says about it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Matroska {
	doctype:	String,			// matroska or webm; the two share this structure
	title:		String,			// the title the file carries, where it carries one
	scale:		u64,			// nanoseconds a timestamp unit stands for
	duration:	f64,			// in scale units; a float, and the file's own word
	tracks:		Vec<Track>,		// in the order the Tracks element described them
}

impl Matroska {

	/// Reads what the header says, from the front of a file.
	///
	/// The buffer need not be the whole film. Everything wanted here is at the
	/// front, and an element running past the end of what is in hand simply ends
	/// the walk -- so a caller holding a sniffing buffer gets the same answer as
	/// one holding the file, provided `Tracks` was within it. A file that put its
	/// cover art before its tracks may need more than a scanner's usual head, so
	/// a caller that finds no tracks and cares should read further and ask again.
	pub fn read(bytes: &[u8]) -> Outcome<Self> {
		if !is_matroska(bytes) {
			return Err(err!(
				"Not a Matroska file: the EBML identifier 1A 45 DF A3 was expected.";
				Invalid, Input, Format));
		}
		let mut out = Self {
			scale: DEFAULT_SCALE,
			..Default::default()
		};
		res!(out.walk(bytes, Scope::Top, 0));
		if out.doctype.is_empty() {
			return Err(err!(
				"The EBML header stated no document type.";
				Invalid, Input, Missing));
		}
		Ok(out)
	}

	/// Walks a run of elements, descending only into those that hold something
	/// wanted.
	///
	/// `scope` is the element this run sits inside, which is what makes the
	/// short identifiers unambiguous -- `0xE0` is `Video` inside a `TrackEntry`
	/// and something else elsewhere.
	fn walk(&mut self, mut b: &[u8], scope: Scope, depth: usize) -> Outcome<()> {
		if depth > DEPTH_LIMIT {
			return Ok(());
		}
		let mut seen = 0usize;
		while b.len() >= 2 && seen < ELEMENT_LIMIT {
			seen += 1;
			let (id, id_len) = match vint_id(b) {
				Some(v) => v,
				None => break,
			};
			let (size, size_len) = match vint_size(&b[id_len..]) {
				Some(v) => v,
				None => break,
			};
			let at = id_len + size_len;
			if at > b.len() {
				break;
			}
			let rest = &b[at..];
			// An element longer than what is in hand is not a fault: the caller
			// may be holding only the front of the file. An unknown length runs
			// to the end of what there is, and nothing can follow it.
			let (take, unknown) = match size {
				Some(n) => ((n as usize).min(rest.len()), false),
				None => (rest.len(), true),
			};
			let body = &rest[..take];

			match (scope, id) {
				(Scope::Top, ID_EBML)			=> res!(self.walk(body, Scope::Head, depth + 1)),
				(Scope::Top, ID_SEGMENT)		=> res!(self.walk(body, Scope::Segment, depth + 1)),
				(Scope::Head, ID_DOC_TYPE)		=> self.doctype = text(body),
				(Scope::Segment, ID_INFO)		=> res!(self.walk(body, Scope::Info, depth + 1)),
				(Scope::Segment, ID_TRACKS)		=> res!(self.walk(body, Scope::Tracks, depth + 1)),
				(Scope::Info, ID_TIMESTAMP_SCALE) => {
					// A scale of nought would make every running time nought.
					let n = uint(body);
					if n > 0 {
						self.scale = n;
					}
				},
				(Scope::Info, ID_DURATION)		=> self.duration = float(body),
				(Scope::Info, ID_TITLE)			=> self.title = text(body),
				(Scope::Tracks, ID_TRACK_ENTRY)	=> {
					let mut track = Track::default();
					res!(read_entry(&mut track, body, depth + 1));
					self.tracks.push(track);
				},
				_ => {},
			}

			if unknown {
				break;
			}
			let step = at.saturating_add(take);
			if step == 0 || step > b.len() {
				break;
			}
			b = &b[step..];
		}
		Ok(())
	}

	/// `matroska` or `webm`.
	pub fn doctype(&self) -> &str { &self.doctype }

	/// Does the file call itself WebM, the subset a browser plays natively?
	pub fn is_webm(&self) -> bool { self.doctype == "webm" }

	/// The title the file carries, empty where it carries none.
	pub fn title(&self) -> &str { &self.title }

	/// Every stream, in the order the file listed them.
	pub fn tracks(&self) -> &[Track] { &self.tracks }

	/// The first picture stream, which is the one a catalogue shows.
	///
	/// A file with two is a file with an alternative take in it, and the
	/// catalogue wants the one it will show. A default-flagged picture wins over
	/// an earlier one that is not flagged, because that is the file saying which
	/// it means.
	pub fn video(&self) -> Option<&Track> {
		let mut first = None;
		for t in &self.tracks {
			if t.kind != Some(TrackKind::Video) {
				continue;
			}
			if t.default {
				return Some(t);
			}
			if first.is_none() {
				first = Some(t);
			}
		}
		first
	}

	pub fn audio(&self) -> Vec<&Track> {
		self.tracks.iter().filter(|t| t.kind == Some(TrackKind::Audio)).collect()
	}

	pub fn subtitles(&self) -> Vec<&Track> {
		self.tracks.iter().filter(|t| t.kind == Some(TrackKind::Subtitle)).collect()
	}

	/// The size of the picture, in pixels, or nought where there is none.
	pub fn size(&self) -> (u32, u32) {
		match self.video() {
			Some(t) => t.size(),
			None => (0, 0),
		}
	}

	/// How long the film runs, in milliseconds, where the header says.
	///
	/// The duration is stated in timestamp units and the scale says how many
	/// nanoseconds a unit is, so the two are needed together; a file still being
	/// written states neither and gets `None` rather than nought, which is a
	/// different claim.
	pub fn millis(&self) -> Option<u64> {
		if self.duration <= 0.0 || self.scale == 0 {
			return None;
		}
		let ns = self.duration * self.scale as f64;
		if !ns.is_finite() || ns < 0.0 {
			return None;
		}
		Some((ns / 1_000_000.0).round() as u64)
	}
}

// ------------------------------------------------------------- the frames

/// One coded frame, exactly as a cluster carries it.
///
/// The bytes are the codec's own and nothing here touches them: a caller
/// repackaging the stream writes them into the new container unchanged, which is
/// what makes a repackaging lossless and what makes it possible at all without a
/// decoder.
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
	pub track:		u64,		// which stream it belongs to, matching Track::number
	// When it is shown, in timestamp scale units from the start of the film.
	// Signed because a block states its own time as a difference from its
	// cluster's, and the difference is signed -- a cluster may carry a frame
	// shown fractionally before the cluster's own stamp.
	pub time:		i64,
	pub key:		bool,		// may decoding begin here?
	pub invisible:	bool,		// decode it but do not show it
	pub data:		&'a [u8],	// the coded bytes
}

/// What one feed of bytes yielded.
///
/// The two numbers are what lets a caller hold a window rather than a film. See
/// [`Clusters::feed`] for the loop they are meant to drive.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Fed {
	pub used:	usize,	// bytes from the front dealt with, which may now be dropped
	// How many bytes must be in hand, counting from the new front, before the
	// next element can be read. Nought means the window merely ran out between
	// elements and any further bytes at all will make progress. A caller that
	// cannot supply want -- because the file ended -- has a truncated file.
	pub want:	usize,
}

/// A reader over the coded frames in a film's clusters.
///
/// # Why this is fed rather than handed the file
///
/// A film is gigabytes and a repackager must never hold one; the whole reason
/// for repackaging in a photo library is to avoid decoding, so spending the
/// memory a decode would have cost defeats it. So this descends into `Segment`
/// and `Cluster` by consuming their **headers alone** and reading their children
/// as though they sat at the top level. What a caller must keep in hand is
/// therefore one *block* -- a frame, a few hundred kilobytes at worst -- and
/// never one cluster, which is megabytes, and never the film.
///
/// Elements that are not wanted are passed over by their stated length without
/// ever being in the window at all, which matters more than it sounds: a film
/// with cover art carries an `Attachments` element of some megabytes, and a
/// reader that had to hold what it skips would be holding that.
///
/// # The loop
///
/// ```ignore
/// let mkv = res!(Matroska::read(&head));
/// let mut cl = Clusters::new(&mkv);
/// let mut buf = Vec::new();
/// loop {
///     // Top the window up, then let the reader take what it can.
///     let fed = res!(cl.feed(&buf, &mut |frame| { … ; Ok(()) }));
///     buf.drain(..fed.used);
///     if fed.want > buf.len() { /* read at least fed.want bytes into buf */ }
/// }
/// ```
#[derive(Clone, Debug, Default)]
pub struct Clusters {
	now:	i64,				// the timestamp of the cluster being read, in scale units
	// Bytes still to be passed over before an element begins again, kept as a
	// count rather than by holding the bytes, so that skipping a large element
	// costs nothing and needs no window.
	skip:	u64,
	scale:	u64,				// nanoseconds a timestamp unit stands for
	// How long one frame of each stream lasts, in nanoseconds, by track number.
	// A short list walked linearly, because a film has a handful of streams and
	// a map would cost more than it saved.
	frames:	Vec<(u64, u64)>,
}

impl Clusters {

	/// A reader positioned at the start of a file.
	///
	/// The header is wanted rather than optional: the timestamp scale and each
	/// stream's frame duration are stated there, and without them the frames of
	/// a laced block cannot be given their own times. A caller has read the
	/// header already, since nothing else says which track numbers mean what.
	pub fn new(mkv: &Matroska) -> Self {
		Self {
			now:	0,
			skip:	0,
			scale:	mkv.scale,
			frames:	mkv.tracks.iter().map(|t| (t.number, t.frame_ns)).collect(),
		}
	}

	/// Reads whole elements from the front of `b`, handing every frame to `f`.
	///
	/// Stops at the first element it cannot complete, and says in [`Fed`] both
	/// what it consumed and what it needs. It never partially reports a frame:
	/// a block that is not wholly in the window is left for the next feed.
	pub fn feed<F>(&mut self, b: &[u8], f: &mut F) -> Outcome<Fed>
	where
		F: FnMut(Frame) -> Outcome<()>,
	{
		let mut i = 0usize;

		// Whatever is left of an element being passed over goes first, and it
		// is counted rather than held -- `skip` may be larger than any window.
		if self.skip > 0 {
			let n = self.skip.min(b.len() as u64) as usize;
			i += n;
			self.skip -= n as u64;
			if self.skip > 0 {
				return Ok(Fed { used: i, want: 0 });
			}
		}

		while i < b.len() {
			let rest = &b[i..];
			let (id, id_len) = match vint_id(rest) {
				Some(v) => v,
				None => return Ok(Fed { used: i, want: HEADER_MAX }),
			};
			let (size, size_len) = match vint_size(&rest[id_len..]) {
				Some(v) => v,
				None => return Ok(Fed { used: i, want: HEADER_MAX }),
			};
			let at = id_len + size_len;

			// A `Segment` states an unknown length in a file still being
			// written, and it is descended into regardless -- which is the one
			// case where an unknown length is not the end of the walk.
			let take = match size {
				Some(n) => n,
				None => 0,
			};

			match id {
				// Descended into by consuming the header alone, so their
				// children are read as though they sat at the top. This is what
				// keeps the window down to one block.
				ID_SEGMENT | ID_CLUSTER => {
					i += at;
					continue;
				},
				// Inside a `Cluster`, and unambiguous *because* nothing else is
				// descended into: `Info` holds elements of its own but is passed
				// over whole, so a bare `0xE7` here can only be a cluster's.
				ID_TIMESTAMP | ID_SIMPLE_BLOCK | ID_BLOCK_GROUP => {
					let whole = at.saturating_add(take as usize);
					if take > b.len() as u64 || whole > rest.len() {
						return Ok(Fed { used: i, want: whole });
					}
					let body = &rest[at..whole];
					match id {
						ID_TIMESTAMP	=> self.now = uint(body) as i64,
						ID_SIMPLE_BLOCK	=> res!(self.block(body, None, f)),
						_				=> res!(self.group(body, f)),
					}
					i += whole;
				},
				// Everything else is passed over by its length, and the bytes
				// never enter the window.
				_ => {
					i += at;
					self.skip = take;
					let n = self.skip.min((b.len() - i) as u64) as usize;
					i += n;
					self.skip -= n as u64;
					if self.skip > 0 {
						return Ok(Fed { used: i, want: 0 });
					}
				},
			}
		}
		Ok(Fed { used: i, want: 0 })
	}

	/// Reads a `BlockGroup`, whose `Block` is a keyframe only if nothing in the
	/// group refers to another frame.
	///
	/// The reference has to be looked for **before** the block is reported,
	/// which is why a group is read whole rather than descended into: a
	/// `ReferenceBlock` is a sibling of the `Block` and may follow it, so a
	/// reader that reported the block on sight would have to take it back.
	fn group<F>(&mut self, b: &[u8], f: &mut F) -> Outcome<()>
	where
		F: FnMut(Frame) -> Outcome<()>,
	{
		// Walked here rather than through [`each`], which hands its closure a
		// slice of anonymous lifetime and so cannot be used to *keep* one.
		let mut block: Option<&[u8]> = None;
		let mut refers = false;
		let mut i = 0usize;
		while i < b.len() {
			let rest = &b[i..];
			let (id, id_len) = match vint_id(rest) {
				Some(v) => v,
				None => break,
			};
			let (size, size_len) = match vint_size(&rest[id_len..]) {
				Some(v) => v,
				None => break,
			};
			let at = id_len + size_len;
			if at > rest.len() {
				break;
			}
			// An unknown length inside a group ends the walk: nothing may follow
			// an element that runs to the end of what there is.
			let take = match size {
				Some(n) => (n as usize).min(rest.len() - at),
				None => break,
			};
			let body = &rest[at..at + take];
			match id {
				ID_BLOCK			=> block = Some(body),
				ID_REFERENCE_BLOCK	=> refers = true,
				_ => {},
			}
			let step = at.saturating_add(take);
			if step == 0 {
				break;
			}
			i += step;
		}
		match block {
			Some(body) => self.block(body, Some(!refers), f),
			// A group with no block in it is not a fault; it is a file carrying
			// something this reader does not want.
			None => Ok(()),
		}
	}

	/// Reads one block, whether laced or not, and reports each frame in it.
	///
	/// `key` is the answer a `BlockGroup` worked out from its references; a
	/// `SimpleBlock` states its own in its flags, so `None` means read it here.
	fn block<F>(&mut self, b: &[u8], key: Option<bool>, f: &mut F) -> Outcome<()>
	where
		F: FnMut(Frame) -> Outcome<()>,
	{
		let (track, n) = match vint_size(b) {
			Some((Some(t), n)) => (t, n),
			// An unknown-length track number is not a thing the format allows.
			_ => return Err(err!(
				"A block stated no readable track number.";
				Invalid, Input, Format)),
		};
		if b.len() < n + 3 {
			return Err(err!(
				"A block of {} bytes is too short to carry a track number, a \
				timestamp and its flags.", b.len();
				Invalid, Input, Format));
		}
		// The block's own time is a *difference* from its cluster's, and it is
		// signed: two bytes, big-endian, two's complement.
		let rel = i16::from_be_bytes([b[n], b[n + 1]]) as i64;
		let flags = b[n + 2];
		let body = &b[n + 3..];

		let key = match key {
			Some(k) => k,
			None => flags & 0x80 != 0,
		};
		let frame = Frame {
			track,
			time:		self.now + rel,
			key,
			invisible:	flags & 0x08 != 0,
			data:		&[],
		};

		// Lacing packs several frames of sound into one block to save the
		// per-block overhead. It is uncommon in a film muxed by a modern tool
		// and it is not rare enough to refuse: a reader that ignored it would
		// hand a caller one frame made of six glued together, which no decoder
		// and no container would accept and nothing would say why.
		match (flags >> 1) & 0x03 {
			0 => res!(f(Frame { data: body, ..frame })),
			lacing => {
				// A laced block states ONE time for all of its frames, and the
				// rest follow it a frame duration apart. Giving them all the
				// block's stamp is what the first version of this did, and the
				// sound of a repackaged film then arrived in bursts: the sizes
				// were right, every frame was there, and only the clock was
				// wrong -- which is why the oracle compares times and not just
				// bytes.
				let step = self.frames.iter()
					.find(|(n, _)| *n == track)
					.map(|(_, d)| *d)
					.unwrap_or(0);
				let parts = res!(laced(body, lacing));
				let laces = parts.len() as u64;
				// The block's whole span first, then that divided among its
				// frames -- **not** the frame duration multiplied up. The two
				// differ because a timestamp unit cannot represent a frame of
				// sound: AAC at 48 kHz lasts 21⅓ milliseconds, and eight of
				// them are 170⅔. Multiplying accumulates the third of a
				// millisecond until the last frame of a block is stamped later
				// than dividing says, and in the limit past the start of the
				// block after it. Dividing the span keeps every frame inside
				// the block that carries it, which is the property that matters
				// to whatever plays the result, and it is what a player does.
				let span = if self.scale > 0 {
					step.saturating_mul(laces) / self.scale
				} else {
					0
				};
				for (i, part) in parts.iter().enumerate() {
					let on = if laces > 0 {
						(i as u64).saturating_mul(span) / laces
					} else {
						0
					};
					res!(f(Frame {
						data: part,
						time: frame.time.saturating_add(on as i64),
						..frame
					}));
				}
			},
		}
		Ok(())
	}
}

/// Splits a laced block body into its frames.
///
/// The three schemes differ only in how the sizes are written; the last frame's
/// size is never stated in any of them, because it is whatever is left.
fn laced(b: &[u8], lacing: u8) -> Outcome<Vec<&[u8]>> {
	let count = match b.first() {
		Some(n) => *n as usize + 1,
		None => return Err(err!(
			"A laced block stated no frame count."; Invalid, Input, Format)),
	};
	let mut at = 1usize;
	let mut sizes = Vec::with_capacity(count);
	match lacing {
		// Fixed: every frame the same size, so nothing is written at all.
		2 => {
			let rest = b.len() - at;
			if count == 0 || rest % count != 0 {
				return Err(err!(
					"A fixed-laced block of {} bytes does not divide into {} \
					frames.", rest, count;
					Invalid, Input, Format));
			}
			for _ in 0..count {
				sizes.push(rest / count);
			}
		},
		// Xiph: each size is a run of bytes, ended by one below 255.
		1 => {
			for _ in 0..count - 1 {
				let mut n = 0usize;
				loop {
					let byte = match b.get(at) {
						Some(v) => *v,
						None => return Err(err!(
							"A Xiph-laced block ended inside a frame size.";
							Invalid, Input, Format)),
					};
					at += 1;
					n += byte as usize;
					if byte < 255 {
						break;
					}
				}
				sizes.push(n);
			}
		},
		// EBML: the first size is a plain variable-width integer and every one
		// after it is a *signed difference* from the one before.
		_ => {
			let (first, n) = match vint_size(&b[at..]) {
				Some((Some(v), n)) => (v as i64, n),
				_ => return Err(err!(
					"An EBML-laced block stated no readable first frame size.";
					Invalid, Input, Format)),
			};
			at += n;
			sizes.push(first as usize);
			let mut prev = first;
			for _ in 0..count.saturating_sub(2) {
				let (delta, n) = match svint(&b[at..]) {
					Some(v) => v,
					None => return Err(err!(
						"An EBML-laced block ended inside a frame size.";
						Invalid, Input, Format)),
				};
				at += n;
				prev += delta;
				if prev < 0 {
					return Err(err!(
						"An EBML-laced block gave a frame a negative size.";
						Invalid, Input, Format));
				}
				sizes.push(prev as usize);
			}
		},
	}

	// The last frame is whatever the stated ones leave, and a file whose stated
	// sizes overrun the block is one this must not read past the end of.
	let stated: usize = sizes.iter().take(count - 1).sum();
	if at.saturating_add(stated) > b.len() {
		return Err(err!(
			"A laced block states {} bytes of frames in {} bytes of body.",
			stated, b.len() - at.min(b.len());
			Invalid, Input, Format));
	}
	if sizes.len() < count {
		sizes.push(b.len() - at - stated);
	}

	let mut out = Vec::with_capacity(count);
	for size in sizes.iter().take(count) {
		let end = at.saturating_add(*size);
		if end > b.len() {
			return Err(err!(
				"A laced block's frame runs past the end of it.";
				Invalid, Input, Format));
		}
		out.push(&b[at..end]);
		at = end;
	}
	Ok(out)
}

/// A signed variable-width integer, as EBML lacing writes a size difference.
///
/// The unsigned value is read exactly as a length is, then shifted down by half
/// its range, which is what makes the differences either way round representable.
fn svint(b: &[u8]) -> Option<(i64, usize)> {
	let (v, n) = match vint_size(b) {
		Some((Some(v), n)) => (v, n),
		_ => return None,
	};
	let bias = (1i64 << (7 * n as u32 - 1)) - 1;
	Some((v as i64 - bias, n))
}

/// Which element a run of elements sits inside.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scope {
	Top,
	Head,
	Segment,
	Info,
	Tracks,
}

/// Fills a track from its `TrackEntry`, descending into `Video` and `Audio`.
///
/// This is walked apart from [`Matroska::walk`] because everything it reads
/// belongs to the one track being built, and threading a half-built track
/// through the file-level walk would mean the file-level walk could put a
/// picture's size on whichever track happened to be open.
fn read_entry(track: &mut Track, mut b: &[u8], depth: usize) -> Outcome<()> {
	if depth > DEPTH_LIMIT {
		return Ok(());
	}
	let mut seen = 0usize;
	while b.len() >= 2 && seen < ELEMENT_LIMIT {
		seen += 1;
		let (id, id_len) = match vint_id(b) {
			Some(v) => v,
			None => break,
		};
		let (size, size_len) = match vint_size(&b[id_len..]) {
			Some(v) => v,
			None => break,
		};
		let at = id_len + size_len;
		if at > b.len() {
			break;
		}
		let rest = &b[at..];
		let (take, unknown) = match size {
			Some(n) => ((n as usize).min(rest.len()), false),
			None => (rest.len(), true),
		};
		let body = &rest[..take];

		match id {
			ID_TRACK_NUMBER		=> track.number = uint(body),
			ID_TRACK_TYPE		=> track.kind = Some(TrackKind::of(uint(body))),
			ID_CODEC_ID			=> track.codec = text(body),
			ID_CODEC_PRIVATE	=> track.private = body.to_vec(),
			ID_NAME				=> track.name = text(body),
			// A BCP 47 tag is the later field and the more precise, so it wins;
			// the older three-letter field must not overwrite it.
			ID_LANGUAGE_BCP47	=> track.lang = text(body),
			ID_LANGUAGE			=> if track.lang.is_empty() { track.lang = text(body) },
			ID_FLAG_DEFAULT		=> track.default = uint(body) != 0,
			ID_FLAG_FORCED		=> track.forced = uint(body) != 0,
			ID_DEFAULT_DURATION	=> track.frame_ns = uint(body),
			ID_VIDEO			=> res!(read_video(track, body, depth + 1)),
			ID_AUDIO			=> res!(read_audio(track, body, depth + 1)),
			_ => {},
		}

		if unknown {
			break;
		}
		let step = at.saturating_add(take);
		if step == 0 || step > b.len() {
			break;
		}
		b = &b[step..];
	}
	Ok(())
}

/// Fills a track's picture size from its `Video` element.
fn read_video(track: &mut Track, b: &[u8], depth: usize) -> Outcome<()> {
	each(b, depth, &mut |id, body| {
		match id {
			ID_PIXEL_W		=> track.w = uint(body) as u32,
			ID_PIXEL_H		=> track.h = uint(body) as u32,
			ID_DISPLAY_W	=> track.dw = uint(body) as u32,
			ID_DISPLAY_H	=> track.dh = uint(body) as u32,
			_ => {},
		}
	})
}

/// Fills a track's channel count and sampling rate from its `Audio` element.
fn read_audio(track: &mut Track, b: &[u8], depth: usize) -> Outcome<()> {
	each(b, depth, &mut |id, body| {
		match id {
			ID_CHANNELS	=> track.channels = uint(body),
			ID_SAMPLING	=> track.rate = float(body),
			_ => {},
		}
	})
}

/// Walks a run of leaf elements, handing each identifier and body to `f`.
///
/// The two innermost elements hold nothing but leaves, so they need none of the
/// descent [`Matroska::walk`] carries and share this instead.
fn each(mut b: &[u8], depth: usize, f: &mut dyn FnMut(u64, &[u8])) -> Outcome<()> {
	if depth > DEPTH_LIMIT {
		return Ok(());
	}
	let mut seen = 0usize;
	while b.len() >= 2 && seen < ELEMENT_LIMIT {
		seen += 1;
		let (id, id_len) = match vint_id(b) {
			Some(v) => v,
			None => break,
		};
		let (size, size_len) = match vint_size(&b[id_len..]) {
			Some(v) => v,
			None => break,
		};
		let at = id_len + size_len;
		if at > b.len() {
			break;
		}
		let rest = &b[at..];
		let take = match size {
			Some(n) => (n as usize).min(rest.len()),
			None => break,
		};
		f(id, &rest[..take]);
		let step = at.saturating_add(take);
		if step == 0 || step > b.len() {
			break;
		}
		b = &b[step..];
	}
	Ok(())
}

/// Does a head begin an EBML file?
///
/// This does not say the file is Matroska rather than WebM, or that it is
/// either: both, and any other EBML document, open the same way. The `DocType`
/// inside is what separates them, and [`Matroska::read`] reports it.
pub fn is_matroska(head: &[u8]) -> bool {
	head.len() >= 4 && head[..4] == MAGIC
}

/// An element identifier, with its marker bits kept.
///
/// The marker is part of the identifier -- `0x83` is `TrackType`, not `0x03` --
/// which is why this differs from [`vint_size`] in exactly that respect.
fn vint_id(b: &[u8]) -> Option<(u64, usize)> {
	let first = match b.first() {
		Some(f) => *f,
		None => return None,
	};
	if first == 0 {
		// No marker bit in the first byte means an identifier wider than the
		// four bytes the specification allows.
		return None;
	}
	let len = first.leading_zeros() as usize + 1;
	if len > 4 || b.len() < len {
		return None;
	}
	let mut v = 0u64;
	for byte in &b[..len] {
		v = (v << 8) | *byte as u64;
	}
	Some((v, len))
}

/// An element length, with its marker bits removed.
///
/// `None` for the length means the file stated *unknown*: every value bit set,
/// which is how an element that runs to the end of the file is written.
fn vint_size(b: &[u8]) -> Option<(Option<u64>, usize)> {
	let first = match b.first() {
		Some(f) => *f,
		None => return None,
	};
	if first == 0 {
		return None;
	}
	let len = first.leading_zeros() as usize + 1;
	if len > 8 || b.len() < len {
		return None;
	}
	// The marker bit and everything above it leave the first byte. At eight
	// bytes the marker is the lowest bit of the first byte, so nothing of the
	// value is in it -- and `0xFF >> 8` is not a shift Rust will perform.
	let mask = if len >= 8 { 0u8 } else { 0xFFu8 >> len };
	let mut v = (first & mask) as u64;
	for byte in &b[1..len] {
		v = (v << 8) | *byte as u64;
	}
	let bits = 7 * len as u32;
	let ones = if bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 };
	Some((if v == ones { None } else { Some(v) }, len))
}

/// A big-endian unsigned integer of whatever width the element gave it.
///
/// EBML writes the narrowest form that fits, so the same field is one byte in
/// one file and four in the next; a reader expecting a fixed width is wrong on
/// half the corpus. An empty body is nought, which is what the specification
/// says a stated-but-empty integer means.
fn uint(b: &[u8]) -> u64 {
	let mut v = 0u64;
	for byte in b.iter().take(8) {
		v = (v << 8) | *byte as u64;
	}
	v
}

/// A big-endian IEEE float, four or eight bytes wide.
///
/// Any other width is nought: the specification allows only these two, and a
/// duration read out of a body of the wrong width would be a plausible number
/// rather than an obviously missing one.
fn float(b: &[u8]) -> f64 {
	match b.len() {
		4 => {
			let mut a = [0u8; 4];
			a.copy_from_slice(b);
			f32::from_be_bytes(a) as f64
		},
		8 => {
			let mut a = [0u8; 8];
			a.copy_from_slice(b);
			f64::from_be_bytes(a)
		},
		_ => 0.0,
	}
}

/// A string, with the padding a writer may have left on the end removed.
///
/// Element strings are UTF-8 and may be padded with nulls to a length the
/// writer reserved. Lossy conversion is deliberate: a mis-encoded title is
/// worth showing with a replacement character in it, and is not worth refusing
/// a whole film over.
fn text(b: &[u8]) -> String {
	let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
	String::from_utf8_lossy(&b[..end]).trim().to_string()
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The narrowest length encoding a value fits in.
	///
	/// The marker is the `len`-th bit from the top, so it lands in the first
	/// byte at `1 << (8 - len)`, and the value fills the `7 * len` bits below.
	fn size_of(n: u64) -> Vec<u8> {
		for len in 1..=8usize {
			let bits = 7 * len as u32;
			let ones = (1u64 << bits) - 1;
			// The all-ones value means "unknown", so a length that would encode
			// as it needs the next width up.
			if n < ones {
				let mut buf = n.to_be_bytes().to_vec();
				buf.drain(..8 - len);
				buf[0] |= 1u8 << (8 - len);
				return buf;
			}
		}
		vec![0xFF]
	}

	/// An element: its identifier as written, its length, and its body.
	fn el(id: u64, body: &[u8]) -> Vec<u8> {
		let mut out = Vec::new();
		let idb = id.to_be_bytes();
		let first = idb.iter().position(|b| *b != 0).unwrap_or(7);
		out.extend_from_slice(&idb[first..]);
		out.extend_from_slice(&size_of(body.len() as u64));
		out.extend_from_slice(body);
		out
	}

	/// An unsigned integer element, written as narrowly as it fits.
	fn u(id: u64, n: u64) -> Vec<u8> {
		let b = n.to_be_bytes();
		let first = b.iter().position(|x| *x != 0).unwrap_or(7);
		el(id, &b[first..])
	}

	fn s(id: u64, v: &str) -> Vec<u8> {
		el(id, v.as_bytes())
	}

	fn f64_el(id: u64, v: f64) -> Vec<u8> {
		el(id, &v.to_be_bytes())
	}

	fn head(doctype: &str) -> Vec<u8> {
		el(ID_EBML, &s(ID_DOC_TYPE, doctype))
	}

	/// A file: an EBML header and a segment holding whatever is given.
	fn file(doctype: &str, inner: Vec<u8>) -> Vec<u8> {
		let mut out = head(doctype);
		out.extend_from_slice(&el(ID_SEGMENT, &inner));
		out
	}

	fn info(scale: u64, duration: f64) -> Vec<u8> {
		let mut b = u(ID_TIMESTAMP_SCALE, scale);
		b.extend_from_slice(&f64_el(ID_DURATION, duration));
		el(ID_INFO, &b)
	}

	fn video_track(number: u64, codec: &str, w: u64, h: u64) -> Vec<u8> {
		let mut v = u(ID_PIXEL_W, w);
		v.extend_from_slice(&u(ID_PIXEL_H, h));
		let mut b = u(ID_TRACK_NUMBER, number);
		b.extend_from_slice(&u(ID_TRACK_TYPE, 1));
		b.extend_from_slice(&s(ID_CODEC_ID, codec));
		b.extend_from_slice(&el(ID_VIDEO, &v));
		el(ID_TRACK_ENTRY, &b)
	}

	fn audio_track(number: u64, codec: &str, channels: u64, lang: &str) -> Vec<u8> {
		let a = u(ID_CHANNELS, channels);
		let mut b = u(ID_TRACK_NUMBER, number);
		b.extend_from_slice(&u(ID_TRACK_TYPE, 2));
		b.extend_from_slice(&s(ID_CODEC_ID, codec));
		b.extend_from_slice(&s(ID_LANGUAGE, lang));
		b.extend_from_slice(&el(ID_AUDIO, &a));
		el(ID_TRACK_ENTRY, &b)
	}

	#[test]
	fn the_size_and_running_time_are_read() -> Outcome<()> {
		// An hour and a half at the usual scale: 5,400,000 milliseconds.
		let mut inner = info(1_000_000, 5_400_000.0);
		inner.extend_from_slice(&el(ID_TRACKS, &video_track(1, "V_MPEG4/ISO/AVC", 1920, 1080)));
		let mkv = res!(Matroska::read(&file("matroska", inner)));

		req!(mkv.doctype(), "matroska");
		req!(mkv.size(), (1920u32, 1080u32));
		req!(mkv.millis(), Some(5_400_000u64));
		Ok(())
	}

	#[test]
	fn the_timestamp_scale_is_applied() -> Outcome<()> {
		// A file whose unit is a microsecond, not a millisecond. The duration
		// reads the same either way, and only the scale tells them apart -- a
		// reader ignoring it says this film is a thousand times too long.
		let mut inner = info(1_000, 5_400_000.0);
		inner.extend_from_slice(&el(ID_TRACKS, &video_track(1, "V_MPEGH/ISO/HEVC", 1280, 720)));
		let mkv = res!(Matroska::read(&file("matroska", inner)));

		req!(mkv.millis(), Some(5_400u64),
			"The timestamp scale was ignored, so the running time is out by the \
			ratio between a microsecond and a millisecond.");
		Ok(())
	}

	#[test]
	fn every_stream_is_reported_not_only_the_picture() -> Outcome<()> {
		// The reason the whole track list matters: this film's picture is one a
		// browser plays and its sound is one no browser decodes, and a reader
		// that looked only at the picture would call the film playable.
		let mut tracks = video_track(1, "V_MPEG4/ISO/AVC", 1920, 800);
		tracks.extend_from_slice(&audio_track(2, "A_AC3", 6, "eng"));
		tracks.extend_from_slice(&audio_track(3, "A_AAC", 2, "fre"));
		let mut inner = info(1_000_000, 1_000.0);
		inner.extend_from_slice(&el(ID_TRACKS, &tracks));
		let mkv = res!(Matroska::read(&file("matroska", inner)));

		req!(mkv.tracks().len(), 3);
		let sound = mkv.audio();
		req!(sound.len(), 2);
		req!(sound[0].codec(), "A_AC3");
		req!(sound[0].channels(), 6u64);
		req!(sound[0].language(), "eng");
		req!(sound[1].codec(), "A_AAC");
		Ok(())
	}

	#[test]
	fn a_subtitle_track_is_not_taken_for_the_picture() -> Outcome<()> {
		// Subtitles come first in a good many real files, and a reader taking
		// the first track as the picture reports a film with no size at all.
		let mut sub = u(ID_TRACK_NUMBER, 1);
		sub.extend_from_slice(&u(ID_TRACK_TYPE, 0x11));
		sub.extend_from_slice(&s(ID_CODEC_ID, "S_TEXT/UTF8"));
		sub.extend_from_slice(&s(ID_LANGUAGE, "eng"));
		let mut tracks = el(ID_TRACK_ENTRY, &sub);
		tracks.extend_from_slice(&video_track(2, "V_MPEG4/ISO/AVC", 720, 576));
		let mut inner = info(1_000_000, 1_000.0);
		inner.extend_from_slice(&el(ID_TRACKS, &tracks));
		let mkv = res!(Matroska::read(&file("matroska", inner)));

		req!(mkv.size(), (720u32, 576u32),
			"The subtitle track was read as the picture.");
		req!(mkv.subtitles().len(), 1);
		req!(mkv.subtitles()[0].language(), "eng");
		Ok(())
	}

	#[test]
	fn a_segment_of_unknown_length_is_still_walked() -> Outcome<()> {
		// How a file still being written states its segment: every value bit of
		// the length set. The children follow all the same, and a reader that
		// treats the length as a number skips the entire file.
		let mut inner = info(1_000_000, 60_000.0);
		inner.extend_from_slice(&el(ID_TRACKS, &video_track(1, "V_AV1", 3840, 2160)));
		let mut bytes = head("matroska");
		bytes.extend_from_slice(&ID_SEGMENT.to_be_bytes()[4..]);
		bytes.push(0xFF);
		bytes.extend_from_slice(&inner);

		let mkv = res!(Matroska::read(&bytes));
		req!(mkv.size(), (3840u32, 2160u32));
		req!(mkv.millis(), Some(60_000u64));
		Ok(())
	}

	#[test]
	fn a_head_answers_as_well_as_the_whole_file() -> Outcome<()> {
		// The clusters follow the tracks and are not in hand. The walk must
		// answer from what it has rather than refusing.
		let mut inner = info(1_000_000, 120_000.0);
		inner.extend_from_slice(&el(ID_TRACKS, &video_track(1, "V_MPEG4/ISO/AVC", 640, 480)));
		let mut bytes = file("matroska", inner);
		// A cluster claiming a great deal more than is present, as a real file
		// does once only its front has been read.
		bytes.extend_from_slice(&[0x1F, 0x43, 0xB6, 0x75]);
		bytes.extend_from_slice(&[0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00]);
		bytes.extend_from_slice(&[0u8; 32]);

		let mkv = res!(Matroska::read(&bytes));
		req!(mkv.size(), (640u32, 480u32));
		req!(mkv.millis(), Some(120_000u64));
		Ok(())
	}

	#[test]
	fn webm_names_itself() -> Outcome<()> {
		let mut inner = info(1_000_000, 1_000.0);
		inner.extend_from_slice(&el(ID_TRACKS, &video_track(1, "V_VP9", 854, 480)));
		let mkv = res!(Matroska::read(&file("webm", inner)));
		req!(mkv.is_webm(), true);
		Ok(())
	}

	#[test]
	fn a_flagged_picture_wins_over_an_earlier_one() -> Outcome<()> {
		// Two picture streams, the second flagged default: a film with an
		// alternative take in it, where the file has said which it means.
		let mut tracks = video_track(1, "V_MPEG4/ISO/AVC", 320, 240);
		let mut second = u(ID_TRACK_NUMBER, 2);
		second.extend_from_slice(&u(ID_TRACK_TYPE, 1));
		second.extend_from_slice(&s(ID_CODEC_ID, "V_MPEGH/ISO/HEVC"));
		second.extend_from_slice(&u(ID_FLAG_DEFAULT, 1));
		let mut v = u(ID_PIXEL_W, 1920);
		v.extend_from_slice(&u(ID_PIXEL_H, 1080));
		second.extend_from_slice(&el(ID_VIDEO, &v));
		tracks.extend_from_slice(&el(ID_TRACK_ENTRY, &second));
		let mut inner = info(1_000_000, 1_000.0);
		inner.extend_from_slice(&el(ID_TRACKS, &tracks));
		let mkv = res!(Matroska::read(&file("matroska", inner)));

		req!(mkv.size(), (1920u32, 1080u32),
			"The unflagged first picture was preferred to the one the file marked.");
		Ok(())
	}

	#[test]
	fn a_bcp47_tag_beats_the_older_language_field() -> Outcome<()> {
		// Files carry both, and the older field is written for players that do
		// not know the newer one -- so the newer must win whichever comes first.
		let mut b = u(ID_TRACK_NUMBER, 1);
		b.extend_from_slice(&u(ID_TRACK_TYPE, 2));
		b.extend_from_slice(&s(ID_CODEC_ID, "A_AAC"));
		b.extend_from_slice(&s(ID_LANGUAGE_BCP47, "pt-BR"));
		b.extend_from_slice(&s(ID_LANGUAGE, "por"));
		let mut inner = info(1_000_000, 1_000.0);
		inner.extend_from_slice(&el(ID_TRACKS, &el(ID_TRACK_ENTRY, &b)));
		let mkv = res!(Matroska::read(&file("matroska", inner)));

		req!(mkv.audio()[0].language(), "pt-BR");
		Ok(())
	}

	#[test]
	fn a_file_still_being_written_has_no_running_time() -> Outcome<()> {
		// Nought is not a running time, and reporting it as one puts "0:00"
		// under every film a recorder has not closed.
		let mut inner = el(ID_INFO, &u(ID_TIMESTAMP_SCALE, 1_000_000));
		inner.extend_from_slice(&el(ID_TRACKS, &video_track(1, "V_MPEG4/ISO/AVC", 640, 480)));
		let mkv = res!(Matroska::read(&file("matroska", inner)));

		req!(mkv.millis(), None::<u64>);
		Ok(())
	}

	#[test]
	fn an_mp4_is_not_a_matroska() -> Outcome<()> {
		let bytes = b"\0\0\0\x18ftypmp42\0\0\0\0mp42isom".to_vec();
		req!(is_matroska(&bytes), false);
		req!(Matroska::read(&bytes).is_err(), true,
			"An MP4 was read as a Matroska file.");
		Ok(())
	}

	#[test]
	fn a_length_of_eight_bytes_does_not_panic() -> Outcome<()> {
		// The widest length the format allows leaves nothing of the value in the
		// first byte, and the mask that clears the marker is a shift of eight.
		let b = [0x01u8, 0, 0, 0, 0, 0, 0x40, 0x00];
		match vint_size(&b) {
			Some((Some(n), len)) => {
				req!(len, 8usize);
				req!(n, 0x4000u64);
			},
			other => return Err(err!(
				"An eight-byte length read as {:?}.", other; Invalid)),
		}
		Ok(())
	}

	#[test]
	fn an_unknown_length_is_told_from_a_large_one() -> Outcome<()> {
		// Every value bit set means unknown. A reader taking it as a number gets
		// 2^56-1 and skips the rest of the file.
		match vint_size(&[0xFFu8]) {
			Some((None, 1)) => {},
			other => return Err(err!(
				"A one-byte unknown length read as {:?}.", other; Invalid)),
		}
		match vint_size(&[0xFEu8]) {
			Some((Some(126), 1)) => {},
			other => return Err(err!(
				"A one-byte length of 126 read as {:?}.", other; Invalid)),
		}
		Ok(())
	}
}
