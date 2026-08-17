//! An archive is a filesystem in a file, which is why this sits here rather than anywhere else.
//!
//! A ZIP archive held wholly in memory over `&[u8]`, with one property the ordinary archive library
//! does not offer and which everything above this depends on: **a member nobody touched is written
//! back byte for byte**. Not re-compressed to the same content -- copied. Its stored DEFLATE stream,
//! its local header, its extra fields, its data descriptor and its central directory entry are all
//! the bytes that were read.
//!
//! # Why that matters more than it sounds
//!
//! The thing above this reads and edits Office documents, which are ZIPs of XML. A `.docx` a
//! colleague sent carries parts nothing here understands -- a theme, custom XML, content controls,
//! tracked changes, a signature. Anything that parsed the archive into a model and wrote the model
//! back would lose every one of them, silently, and the person who found out would be the colleague.
//! So the archive is held whole and only what is touched is rebuilt.
//!
//! The property is checkable, and callers should check it: read an archive, write it straight back
//! out, and the bytes are identical. [`Zip::is_pristine`] says whether anything was touched at all.
//!
//! # Target-neutral
//!
//! Nothing here reaches the filesystem. It takes bytes and returns bytes, so the same code runs in a
//! browser, where a `.docx` arrives from a file picker and never has a path.
//!
//! # What it does not do
//!
//! ZIP64 is read but not written: an archive that needed the ZIP64 records to be read is refused on
//! write, by name, rather than written back wrong. Encryption is refused. Multi-disk archives are
//! refused. DEFLATE itself is `flate2`'s, and is not hand-rolled here.
//!
//! # Usage
//!
//! ```ignore
//! use oxedyne_fe2o3_file::zip::{Method, Zip};
//!
//! let mut zip = res!(Zip::read(bytes));
//! let xml = res!(zip.text("word/document.xml"));
//! zip.set("word/document.xml", edited.into_bytes(), Method::Deflate);
//! let out = res!(zip.write());	// Every other member is the bytes it was.
//! ```

pub mod read;
pub mod write;

use oxedyne_fe2o3_core::prelude::*;

use std::ops::Range;

/// The most a single member is inflated to before [`Zip::content`] refuses it.
///
/// A `.xlsx` part inflates to many times its compressed size as a matter of course, so the ceiling
/// has to be generous; a hostile archive inflates to whatever it likes, so there has to be one. A
/// caller with its own idea of the ceiling uses [`Zip::content_capped`] and says what it is.
pub const MAX_INFLATE: u64 = 256 * 1024 * 1024;

/// How a member's bytes are held in the archive.
///
/// The numeric code is kept for a method this does not decode, so an archive holding one still reads
/// its directory, still names the member, and still copies it through untouched.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Method {
	/// Stored: the bytes in the archive are the content.
	Store,
	/// DEFLATE, which is what all but the smallest members of an Office document use.
	Deflate,
	/// A method this does not decode, by its code.
	Other(u16),
}

impl Method {

	/// The method's code as the archive writes it.
	pub fn code(&self) -> u16 {
		match self {
			Self::Store	=> 0,
			Self::Deflate	=> 8,
			Self::Other(c)	=> *c,
		}
	}

	/// The method a code names.
	pub fn of(code: u16) -> Self {
		match code {
			0	=> Self::Store,
			8	=> Self::Deflate,
			c	=> Self::Other(c),
		}
	}
}

/// The DOS date of 1 January 1980, the earliest a ZIP can express.
///
/// A fresh member is stamped with this rather than with the clock, so writing the same archive twice
/// gives the same bytes. A build that has to be reproducible cannot have the time of day in it, and a
/// caller who wants a real timestamp sets one with [`Member::stamp`].
pub const EPOCH_DATE: u16 = 0x0021;

/// Where a member's bytes come from when the archive is written.
#[derive(Clone, Debug)]
pub enum Body {
	/// A member read from an archive and not since changed, as byte ranges into the source.
	///
	/// Writing it copies `whole` verbatim, which is what makes an untouched member survive a round
	/// trip exactly. `data` addresses the compressed bytes alone, for reading the content.
	Held {
		/// The local header, the data, and any data descriptor: everything the member occupies.
		whole:	Range<usize>,
		/// The compressed data alone.
		data:	Range<usize>,
		/// The member's entry in the central directory.
		cen:	Range<usize>,
	},
	/// A member this archive was given, which is written afresh from its uncompressed bytes.
	Fresh {
		/// The content, uncompressed.
		data:	Vec<u8>,
		/// The DOS time and date to stamp it with.
		stamp:	(u16, u16),
	},
}

/// One member of an archive.
#[derive(Clone, Debug)]
pub struct Member {
	/// The member's name: its path within the archive, with forward slashes.
	pub name:	String,
	/// How its bytes are held.
	pub method:	Method,
	/// The CRC-32 of its uncompressed content.
	pub crc:	u32,
	/// The size of its uncompressed content.
	pub size:	u64,
	/// The size of its bytes as the archive holds them.
	pub csize:	u64,
	/// The general purpose bit flag the archive recorded.
	pub flags:	u16,
	/// Where the bytes come from.
	pub body:	Body,
}

impl Member {

	/// Whether the member is encrypted, which is bit 0 of the general purpose flag.
	pub fn is_encrypted(&self) -> bool {
		self.flags & 1 != 0
	}

	/// Whether the member is a directory entry rather than a file: a trailing slash and no content.
	pub fn is_dir(&self) -> bool {
		self.name.ends_with('/') && self.size == 0
	}

	/// Stamps a fresh member with a DOS time and date. A held member's stamp is in bytes that are
	/// copied, so setting one would be a lie about what will be written.
	pub fn stamp(&mut self, time: u16, date: u16) {
		if let Body::Fresh { stamp, .. } = &mut self.body {
			*stamp = (time, date);
		}
	}

	/// Whether the member's bytes are the ones it was read with.
	pub fn is_held(&self) -> bool {
		matches!(self.body, Body::Held { .. })
	}

	/// The member's bytes exactly as the archive holds them, compressed and not decoded. For a member
	/// the caller supplied, its content, which has not been compressed yet.
	///
	/// What a check that an untouched member was *copied* rather than rebuilt compares. Two members
	/// can hold the same content and different bytes -- another compression level gives another
	/// stream -- and it is the bytes a colleague's reader parses.
	pub fn raw<'a>(&'a self, zip: &'a Zip) -> Outcome<&'a [u8]> {
		match &self.body {
			Body::Held { data, .. }	=> Ok(res!(zip.src.get(data.clone()).ok_or_else(|| err!(
				"'{}' addresses bytes {}..{} of an archive of {} bytes.",
				self.name, data.start, data.end, zip.src.len(); Bug, Range)))),
			Body::Fresh { data, .. }	=> Ok(data),
		}
	}
}

/// A ZIP archive held in memory, with the bytes it was read from.
#[derive(Clone, Debug, Default)]
pub struct Zip {
	/// The bytes the archive was read from, which every held member addresses into.
	pub(crate) src:		Vec<u8>,
	/// The members, in the order they occupy the archive.
	pub(crate) members:	Vec<Member>,
	/// The archive comment, which the end record carries.
	pub(crate) comment:	Vec<u8>,
	/// Whether reading needed the ZIP64 records. Such an archive is refused on write rather than
	/// written back without them.
	pub(crate) zip64:	bool,
	/// Whether any member has been added, replaced or removed.
	pub(crate) touched:	bool,
}

impl Zip {

	/// An empty archive.
	pub fn new() -> Self {
		Self::default()
	}

	/// The number of members.
	pub fn len(&self) -> usize {
		self.members.len()
	}

	/// Whether the archive holds nothing.
	pub fn is_empty(&self) -> bool {
		self.members.is_empty()
	}

	/// Whether every member is still the bytes it was read with, so writing reproduces the source.
	pub fn is_pristine(&self) -> bool {
		!self.touched
	}

	/// The members, in the order they occupy the archive.
	pub fn members(&self) -> &[Member] {
		&self.members
	}

	/// The bytes the archive was read from, empty where it was built rather than read.
	pub fn source(&self) -> &[u8] {
		&self.src
	}

	/// The names of the members, in archive order.
	pub fn names(&self) -> Vec<&str> {
		self.members.iter().map(|m| m.name.as_str()).collect()
	}

	/// Where a named member sits, if the archive holds one.
	pub fn index_of(&self, name: &str) -> Option<usize> {
		self.members.iter().position(|m| m.name == name)
	}

	/// Whether the archive holds a member of that name.
	pub fn has(&self, name: &str) -> bool {
		self.index_of(name).is_some()
	}

	/// A named member.
	pub fn member(&self, name: &str) -> Option<&Member> {
		self.index_of(name).and_then(|i| self.members.get(i))
	}

	/// The uncompressed content of a named member, up to [`MAX_INFLATE`].
	pub fn content(&self, name: &str) -> Outcome<Vec<u8>> {
		self.content_capped(name, MAX_INFLATE)
	}

	/// The uncompressed content of a named member, refusing anything above the stated ceiling.
	///
	/// The declared size is checked against the ceiling before a byte is inflated, so a member that
	/// claims to be enormous costs nothing to refuse, and the inflated length is checked again after,
	/// so a member that lied about its size is refused too.
	pub fn content_capped(&self, name: &str, cap: u64) -> Outcome<Vec<u8>> {
		let i = res!(self.index_of(name).ok_or_else(|| err!(
			"The archive holds no member named '{}'.", name; Missing)));
		self.content_at(i, cap)
	}

	/// The uncompressed content of the member at an index, refusing anything above the ceiling.
	pub fn content_at(&self, i: usize, cap: u64) -> Outcome<Vec<u8>> {
		let m = res!(self.members.get(i).ok_or_else(|| err!(
			"The archive has {} members, so there is none at index {}.", self.members.len(), i;
			Missing, Range)));
		if m.is_encrypted() {
			return Err(err!(
				"'{}' is encrypted. This reads no encrypted archive, so its content is not \
				available; the member is still copied through untouched.", m.name;
				Invalid, Input, Unimplemented));
		}
		if m.size > cap {
			return Err(err!(
				"'{}' says it holds {} bytes, over the {} byte ceiling for reading one.",
				m.name, m.size, cap; Excessive, Size));
		}
		let raw = match &m.body {
			Body::Held { data, .. }	=> res!(self.src.get(data.clone()).ok_or_else(|| err!(
				"'{}' addresses bytes {}..{} of an archive of {} bytes.",
				m.name, data.start, data.end, self.src.len(); Bug, Range))),
			Body::Fresh { data, .. }	=> return Ok(data.clone()),
		};
		let out = match m.method {
			Method::Store	=> raw.to_vec(),
			Method::Deflate	=> res!(inflate(raw, cap, &m.name)),
			Method::Other(c)	=> return Err(err!(
				"'{}' is held by compression method {}, which this does not decode. The member \
				is still copied through untouched.", m.name, c; Unimplemented)),
		};
		if out.len() as u64 > cap {
			return Err(err!(
				"'{}' inflated to more than the {} byte ceiling for reading one.", m.name, cap;
				Excessive, Size));
		}
		let mut crc = flate2::Crc::new();
		crc.update(&out);
		if crc.sum() != m.crc {
			return Err(err!(
				"'{}' does not match its own checksum: the directory says {:08x} and the bytes \
				give {:08x}. The archive is damaged.", m.name, m.crc, crc.sum();
				Invalid, Data, Mismatch));
		}
		Ok(out)
	}

	/// The content of a named member as text, which it must be.
	pub fn text(&self, name: &str) -> Outcome<String> {
		let bytes = res!(self.content(name));
		Ok(res!(String::from_utf8(bytes), Decode, String))
	}

	/// Adds a member, or replaces the one of that name where there is one.
	///
	/// Replacing puts the new member where the old one was, so a caller editing one part of a
	/// document does not reorder the archive.
	pub fn set(&mut self, name: &str, data: Vec<u8>, method: Method) {
		let mut crc = flate2::Crc::new();
		crc.update(&data);
		let m = Member {
			name:	name.to_string(),
			method,
			crc:	crc.sum(),
			size:	data.len() as u64,
			csize:	0,	// Not known until it is written.
			flags:	0,
			body:	Body::Fresh { data, stamp: (0, EPOCH_DATE) },
		};
		self.touched = true;
		match self.index_of(name) {
			Some(i)	=> self.members[i] = m,
			None		=> self.members.push(m),
		}
	}

	/// Adds a member at the head of the archive, before every other.
	///
	/// OpenDocument needs this and needs it stored rather than compressed: a reader identifies an
	/// `.odt` by finding `mimetype` first in the archive and uncompressed, and one written anywhere
	/// else is a file that opens as a plain ZIP.
	pub fn set_first(&mut self, name: &str, data: Vec<u8>, method: Method) {
		self.set(name, data, method);
		if let Some(i) = self.index_of(name) {
			let m = self.members.remove(i);
			self.members.insert(0, m);
		}
	}

	/// Removes a named member, saying whether there was one.
	pub fn remove(&mut self, name: &str) -> bool {
		match self.index_of(name) {
			Some(i)	=> {
				self.members.remove(i);
				self.touched = true;
				true
			}
			None		=> false,
		}
	}
}

/// Inflates a raw DEFLATE stream, refusing to grow past the ceiling.
///
/// The ceiling is enforced as it goes rather than after, because a member that claims a small size
/// and inflates without end would otherwise take the machine down before anything checked it.
fn inflate(raw: &[u8], cap: u64, name: &str) -> Outcome<Vec<u8>> {
	use std::io::Read;
	let mut out = Vec::new();
	// One byte over the ceiling is enough to tell a member at the ceiling from one past it.
	let lim = cap.saturating_add(1);
	let mut r = flate2::read::DeflateDecoder::new(raw).take(lim);
	res!(r.read_to_end(&mut out), IO, Decode);
	if out.len() as u64 > cap {
		return Err(err!(
			"'{}' inflates to more than the {} byte ceiling for reading one.", name, cap;
			Excessive, Size));
	}
	Ok(out)
}

/// Reads a little-endian `u16` at an offset.
pub(crate) fn u16le(b: &[u8], i: usize) -> Outcome<u16> {
	match b.get(i..i + 2) {
		Some(s)	=> Ok(u16::from_le_bytes([s[0], s[1]])),
		None		=> Err(err!(
			"An archive of {} bytes has no 16-bit field at offset {}.", b.len(), i;
			Invalid, Input, Range)),
	}
}

/// Reads a little-endian `u32` at an offset.
pub(crate) fn u32le(b: &[u8], i: usize) -> Outcome<u32> {
	match b.get(i..i + 4) {
		Some(s)	=> Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]])),
		None		=> Err(err!(
			"An archive of {} bytes has no 32-bit field at offset {}.", b.len(), i;
			Invalid, Input, Range)),
	}
}

/// Reads a little-endian `u64` at an offset.
pub(crate) fn u64le(b: &[u8], i: usize) -> Outcome<u64> {
	match b.get(i..i + 8) {
		Some(s)	=> Ok(u64::from_le_bytes([
			s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
		])),
		None		=> Err(err!(
			"An archive of {} bytes has no 64-bit field at offset {}.", b.len(), i;
			Invalid, Input, Range)),
	}
}
