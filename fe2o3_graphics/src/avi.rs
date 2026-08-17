//! AVI: the size and running time of a film in a RIFF container.
//!
//! AVI predates the ISO base media format that [`crate::mp4`] reads, and a
//! camera of the late 2000s wrote it by default. The files are still in family
//! libraries, and a photo library that cannot see them under-reports what is on
//! the disk.
//!
//! This reads the header and nothing else. **No AVI is decoded here**: what is
//! inside one is usually Motion JPEG or DV, and a caller wanting a frame either
//! has a decoder for the stream's own codec or has none. What a catalogue needs
//! is how big the picture is, how long it runs and what it is coded in, and all
//! three are in the header list at the front of the file -- so a head of a few
//! kilobytes answers them without opening the rest.
//!
//! # The shape of the file
//!
//! A RIFF file is `RIFF`, a length, a form type, and then a sequence of chunks,
//! each a four-character code, a length, and that many bytes. A `LIST` chunk
//! begins with a further four-character code and then holds chunks of its own.
//! An AVI's form type is `AVI ` -- with the trailing space, which is not a
//! typographic accident -- and the first `LIST` is `hdrl`, holding:
//!
//! - `avih`, the main header: the size of the picture, how many frames there
//!   are and how long each is shown.
//! - one `LIST strl` a stream, each opening with `strh`, the stream header,
//!   which says whether the stream is video and what codec it carries.
//!
//! # References
//!
//! Microsoft's AVI RIFF File Reference for `avih` (`AVIMAINHEADER`) and `strh`
//! (`AVISTREAMHEADER`), and the OpenDML AVI File Format Extensions v1.02 for why
//! the main header's frame count is not to be trusted on its own.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

const VIDEO: &[u8; 4] = b"vids"; // a strh's stream type, for video

// The header list is at the front, so a file that has not shown it by here is
// not one this reader understands. Without a bound, a length field of nought --
// which a truncated or malformed file readily supplies -- is an endless walk.
const CHUNK_LIMIT: usize = 64;

/// What a film's header says about it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Avi {
	w:		u32,		// picture width, pixels
	h:		u32,		// picture height, pixels
	micros:	u32,		// microseconds a frame is shown, from the main header
	frames:	u32,		// frame count the main header claims
	// Frame rate
	rate:	u32,		// quotient with scale is frames a second
	scale:	u32,
	length:	u32,		// frame count the video stream's own header claims
	codec:	[u8; 4],	// four-character code of the video stream's codec
}

impl Avi {

	/// The buffer need not be the whole film -- the header list is at the front
	/// -- and a chunk running past the end of what is in hand simply ends the
	/// walk, so a caller holding a sniffing buffer gets the same answer as one
	/// holding the file.
	pub fn read(bytes: &[u8]) -> Outcome<Self> {
		if !is_avi(bytes) {
			return Err(err!(
				"Not an AVI: a RIFF file whose form type is 'AVI ' was expected.";
				Invalid, Input, Format));
		}
		let mut out = Self::default();
		// Past `RIFF`, its length, and the form type.
		res!(out.walk(&bytes[12..], 0));
		if out.w == 0 || out.h == 0 {
			return Err(err!(
				"The AVI header list gave no picture size.";
				Invalid, Input, Missing));
		}
		Ok(out)
	}

	/// Every `LIST` is descended into, and `depth` bounds that descent: a `LIST`
	/// claiming to contain itself is a file that would otherwise be walked for
	/// ever.
	fn walk(&mut self, mut at: &[u8], depth: usize) -> Outcome<()> {
		if depth > 3 {
			return Ok(());
		}
		let mut seen = 0usize;
		while at.len() >= 8 && seen < CHUNK_LIMIT {
			seen += 1;
			let id = &at[..4];
			let len = u32_at(at, 4) as usize;
			let body = &at[8..];
			// A chunk longer than what is in hand is not a fault: the caller may
			// be holding only the front of the file.
			let take = len.min(body.len());
			match id {
				b"LIST" if take >= 4 => res!(self.walk(&body[4..take], depth + 1)),
				b"avih" => self.read_avih(&body[..take]),
				b"strh" => self.read_strh(&body[..take]),
				_ => {},
			}
			// Chunks are padded to an even length, and the pad byte is not
			// counted in the length -- a reader that forgets it is one byte out
			// for the rest of the file.
			let step = 8usize.saturating_add(len).saturating_add(len & 1);
			if step == 0 || step > at.len() {
				break;
			}
			at = &at[step..];
		}
		Ok(())
	}

	/// The main header: the size of the picture and how long a frame lasts.
	fn read_avih(&mut self, body: &[u8]) {
		if body.len() < 40 {
			return;
		}
		self.micros = u32_at(body, 0);
		self.frames = u32_at(body, 16);
		self.w = u32_at(body, 32);
		self.h = u32_at(body, 36);
	}

	/// A stream header, taken only where it is the video stream.
	///
	/// The first video stream wins. A file with two is a file with an
	/// alternative take in it, and the catalogue wants the one it will show.
	fn read_strh(&mut self, body: &[u8]) {
		if body.len() < 36 || &body[..4] != VIDEO || self.rate != 0 {
			return;
		}
		self.codec.copy_from_slice(&body[4..8]);
		self.scale = u32_at(body, 20);
		self.rate = u32_at(body, 24);
		self.length = u32_at(body, 32);
	}

	pub fn size(&self) -> (u32, u32) {
		(self.w, self.h)
	}

	/// `MJPG` and `dvsd` are what a camera of this era writes. The code is not
	/// interpreted here; a caller deciding whether it can draw a frame is the
	/// one that knows.
	pub fn codec(&self) -> [u8; 4] {
		self.codec
	}

	/// The stream's own header is preferred over the main one. The main header's
	/// frame count is a single 32-bit field written before the file was
	/// finished, and the OpenDML extensions leave it nought on a file that grew
	/// past four gigabytes; the stream header's length is the one a player
	/// trusts. The main header stands in only where there is no video stream
	/// header to consult.
	pub fn millis(&self) -> Option<u64> {
		if self.rate > 0 && self.scale > 0 && self.length > 0 {
			// length * scale / rate is the running time in seconds, computed in
			// 64 bits because length times scale overflows 32 readily.
			let ms = (self.length as u64)
				.saturating_mul(self.scale as u64)
				.saturating_mul(1000)
				/ (self.rate as u64);
			return Some(ms);
		}
		if self.micros > 0 && self.frames > 0 {
			return Some((self.frames as u64).saturating_mul(self.micros as u64) / 1000);
		}
		None
	}
}

/// Does a head begin a RIFF file whose form type is `AVI `?
///
/// The trailing space is part of the code. `RIFF....WEBP` is the other RIFF form
/// a photo library meets, so the form type is what tells them apart and the
/// leading `RIFF` on its own is not enough.
pub fn is_avi(head: &[u8]) -> bool {
	head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"AVI "
}

/// A little-endian 32-bit value, nought where the buffer is too short.
fn u32_at(b: &[u8], at: usize) -> u32 {
	if b.len() < at + 4 {
		return 0;
	}
	u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Builds a chunk: a code, a little-endian length, the body, and the pad
	/// byte an odd length requires.
	fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
		let mut out = Vec::new();
		out.extend_from_slice(id);
		out.extend_from_slice(&(body.len() as u32).to_le_bytes());
		out.extend_from_slice(body);
		if body.len() & 1 == 1 {
			out.push(0);
		}
		out
	}

	fn avih(micros: u32, frames: u32, w: u32, h: u32) -> Vec<u8> {
		let mut b = vec![0u8; 56];
		b[0..4].copy_from_slice(&micros.to_le_bytes());
		b[16..20].copy_from_slice(&frames.to_le_bytes());
		b[32..36].copy_from_slice(&w.to_le_bytes());
		b[36..40].copy_from_slice(&h.to_le_bytes());
		b
	}

	fn strh(kind: &[u8; 4], codec: &[u8; 4], scale: u32, rate: u32, length: u32) -> Vec<u8> {
		let mut b = vec![0u8; 56];
		b[0..4].copy_from_slice(kind);
		b[4..8].copy_from_slice(codec);
		b[20..24].copy_from_slice(&scale.to_le_bytes());
		b[24..28].copy_from_slice(&rate.to_le_bytes());
		b[32..36].copy_from_slice(&length.to_le_bytes());
		b
	}

	fn file(inner: Vec<u8>) -> Vec<u8> {
		let mut out = Vec::new();
		out.extend_from_slice(b"RIFF");
		out.extend_from_slice(&((inner.len() + 4) as u32).to_le_bytes());
		out.extend_from_slice(b"AVI ");
		out.extend_from_slice(&inner);
		out
	}

	/// A header list as a camera writes one: `hdrl` holding `avih` and a
	/// `LIST strl` whose first chunk is `strh`.
	fn hdrl(main: Vec<u8>, stream: Vec<u8>) -> Vec<u8> {
		let mut strl = b"strl".to_vec();
		strl.extend_from_slice(&chunk(b"strh", &stream));
		let mut body = b"hdrl".to_vec();
		body.extend_from_slice(&chunk(b"avih", &main));
		body.extend_from_slice(&chunk(b"LIST", &strl));
		chunk(b"LIST", &body)
	}

	#[test]
	fn the_size_and_running_time_are_read() -> Outcome<()> {
		// Ten seconds at 25 frames a second, 640 by 480.
		let bytes = file(hdrl(
			avih(40_000, 250, 640, 480),
			strh(b"vids", b"MJPG", 1, 25, 250),
		));
		let avi = res!(Avi::read(&bytes));
		req!(avi.size(), (640u32, 480u32));
		req!(avi.millis(), Some(10_000u64));
		req!(&avi.codec(), b"MJPG");
		Ok(())
	}

	#[test]
	fn the_stream_header_is_preferred_to_the_main_one() -> Outcome<()> {
		// What an OpenDML file looks like: the main header's frame count was
		// never filled in, and only the stream knows the length.
		let bytes = file(hdrl(
			avih(40_000, 0, 720, 576),
			strh(b"vids", b"dvsd", 1, 25, 500),
		));
		let avi = res!(Avi::read(&bytes));
		req!(avi.millis(), Some(20_000u64),
			"The main header's nought frames were used in place of the stream's.");
		Ok(())
	}

	#[test]
	fn an_audio_stream_is_not_mistaken_for_the_picture() -> Outcome<()> {
		// `auds` first, and its rate and scale are nothing to do with frames.
		let mut strl_a = b"strl".to_vec();
		strl_a.extend_from_slice(&chunk(b"strh", &strh(b"auds", b"\0\0\0\0", 1, 44_100, 441_000)));
		let mut body = b"hdrl".to_vec();
		body.extend_from_slice(&chunk(b"avih", &avih(40_000, 250, 640, 480)));
		body.extend_from_slice(&chunk(b"LIST", &strl_a));
		let mut strl_v = b"strl".to_vec();
		strl_v.extend_from_slice(&chunk(b"strh", &strh(b"vids", b"MJPG", 1, 25, 250)));
		body.extend_from_slice(&chunk(b"LIST", &strl_v));
		let bytes = file(chunk(b"LIST", &body));

		let avi = res!(Avi::read(&bytes));
		req!(avi.millis(), Some(10_000u64),
			"The audio stream's rate was read as a frame rate.");
		req!(&avi.codec(), b"MJPG");
		Ok(())
	}

	#[test]
	fn a_head_answers_as_well_as_the_whole_file() -> Outcome<()> {
		// The film's data follows the header list and is not in hand. The walk
		// must answer from what it has rather than refusing.
		let mut bytes = file(hdrl(
			avih(33_333, 300, 1280, 720),
			strh(b"vids", b"MJPG", 1, 30, 300),
		));
		bytes.extend_from_slice(&chunk(b"LIST", b"movi"));
		// Claim a great deal more `movi` than is present, as a real file does
		// once only its front has been read.
		let n = bytes.len();
		bytes[n - 8..n - 4].copy_from_slice(b"LIST");
		let avi = res!(Avi::read(&bytes));
		req!(avi.size(), (1280u32, 720u32));
		Ok(())
	}

	#[test]
	fn a_webp_is_not_an_avi() -> Outcome<()> {
		let mut bytes = b"RIFF".to_vec();
		bytes.extend_from_slice(&64u32.to_le_bytes());
		bytes.extend_from_slice(b"WEBPVP8 ");
		req!(is_avi(&bytes), false);
		req!(Avi::read(&bytes).is_err(), true,
			"A WebP was read as a film.");
		Ok(())
	}
}
