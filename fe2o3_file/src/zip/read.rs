//! Reading an archive's directory, and nothing else.
//!
//! Nothing is inflated here and no content is copied. The pass reads the central directory, finds
//! where each member's bytes sit, and records the byte ranges -- so opening a hundred megabyte
//! archive to look at one small part inside it costs the directory and no more.
//!
//! The ranges are what the writer copies from. A member's `whole` range runs from its local header to
//! the start of whatever follows it, so anything the archive holds between members -- padding, an
//! alignment gap, a data descriptor written in either of its two shapes -- travels with the member
//! before it and survives the round trip without this having to understand it.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::zip::{
	Body,
	Member,
	Method,
	Zip,
	u16le,
	u32le,
	u64le,
};

use oxedyne_fe2o3_core::prelude::*;

/// The end of central directory record.
const SIG_EOCD:	u32 = 0x0605_4b50;
/// The ZIP64 end of central directory record.
const SIG_EOCD64:	u32 = 0x0606_4b50;
/// The ZIP64 end of central directory locator.
const SIG_LOC64:	u32 = 0x0706_4b50;
/// A central directory entry.
const SIG_CEN:	u32 = 0x0201_4b50;
/// A local file header.
const SIG_LOC:	u32 = 0x0403_4b50;

/// The fixed part of a central directory entry.
const CEN_LEN:	usize = 46;
/// The fixed part of a local file header.
const LOC_LEN:	usize = 30;

/// The most an end record's comment may be, which bounds the search for the record itself.
const MAX_COMMENT:	usize = 0xFFFF;

impl Zip {

	/// Reads an archive from the bytes of one.
	///
	/// The bytes are kept, because every member that is not later replaced is written back out of
	/// them. See the module's own note on why that is the whole point.
	pub fn read(src: Vec<u8>) -> Outcome<Self> {
		let eocd = res!(find_eocd(&src));
		let mut count	= res!(u16le(&src, eocd + 10)) as u64;
		let mut cd_len	= res!(u32le(&src, eocd + 12)) as u64;
		let mut cd_at	= res!(u32le(&src, eocd + 16)) as u64;
		let clen	= res!(u16le(&src, eocd + 20)) as usize;
		let comment	= match src.get(eocd + 22..eocd + 22 + clen) {
			Some(s)	=> s.to_vec(),
			None		=> return Err(err!(
				"The end record says its comment is {} bytes, which runs past the end of an \
				archive of {} bytes.", clen, src.len(); Invalid, Input)),
		};
		let disk	= res!(u16le(&src, eocd + 4));
		let cd_disk	= res!(u16le(&src, eocd + 6));

		// A sentinel in any of the three says the real value is in the ZIP64 record, which the locator
		// twenty bytes before the end record points at.
		let mut zip64 = false;
		if count == 0xFFFF || cd_len == 0xFFFF_FFFF || cd_at == 0xFFFF_FFFF || disk == 0xFFFF {
			let (n, len, at) = res!(read_eocd64(&src, eocd));
			count	= n;
			cd_len	= len;
			cd_at	= at;
			zip64	= true;
		}
		if !zip64 && (disk != 0 || cd_disk != 0) {
			return Err(err!(
				"The archive is split across {} disks. This reads no split archive.",
				disk as u32 + 1; Invalid, Input, Unimplemented));
		}
		let cd_at = cd_at as usize;
		let cd_end = cd_at.saturating_add(cd_len as usize);
		if cd_end > src.len() {
			return Err(err!(
				"The directory is said to run to byte {} of an archive of {} bytes.",
				cd_end, src.len(); Invalid, Input, Range));
		}

		// Pass one: the directory, which says what the archive holds and where each member starts.
		let mut raw: Vec<Raw> = Vec::with_capacity(count.min(4096) as usize);
		let mut i = cd_at;
		while i < cd_end {
			if res!(u32le(&src, i)) != SIG_CEN {
				break;
			}
			let ent = res!(read_cen(&src, i));
			i = ent.cen_end;
			raw.push(ent);
		}
		if raw.len() as u64 != count {
			return Err(err!(
				"The end record counts {} members and the directory holds {}.", count, raw.len();
				Invalid, Input, Mismatch));
		}

		// Pass two: where each member's bytes end, which is where the next one's begin. Anything
		// between them travels with the member before it, so padding and data descriptors of either
		// shape survive without this having to read them.
		raw.sort_by_key(|r| r.at);
		let mut members = Vec::with_capacity(raw.len());
		for (n, r) in raw.iter().enumerate() {
			let ends = match raw.get(n + 1) {
				Some(next)	=> next.at as usize,
				None		=> cd_at,
			};
			let at = r.at as usize;
			if at >= ends || ends > src.len() {
				return Err(err!(
					"'{}' is said to start at byte {} and to end at byte {} of an archive of {} \
					bytes.", r.name, at, ends, src.len(); Invalid, Input, Range));
			}
			if res!(u32le(&src, at)) != SIG_LOC {
				return Err(err!(
					"'{}' is said to start at byte {}, where there is no local header.",
					r.name, at; Invalid, Input));
			}
			let nlen = res!(u16le(&src, at + 26)) as usize;
			let elen = res!(u16le(&src, at + 28)) as usize;
			let from = at + LOC_LEN + nlen + elen;
			let to = from.saturating_add(r.csize as usize);
			if to > ends {
				return Err(err!(
					"'{}' says it holds {} compressed bytes, which runs past the {} bytes the \
					archive gives it.", r.name, r.csize, ends - from; Invalid, Input, Range));
			}
			members.push(Member {
				name:	r.name.clone(),
				method:	Method::of(r.method),
				crc:	r.crc,
				size:	r.size,
				csize:	r.csize,
				flags:	r.flags,
				body:	Body::Held {
					whole:	at..ends,
					data:	from..to,
					cen:	r.cen_at..r.cen_end,
				},
			});
		}
		Ok(Self { src, members, comment, zip64, touched: false })
	}
}

/// One central directory entry, as read.
struct Raw {
	name:	String,
	flags:	u16,		// general purpose bit flag
	method:	u16,		// the compression method's code
	crc:	u32,		// of the uncompressed content
	csize:	u64,		// compressed
	size:	u64,		// uncompressed
	at:	u64,		// where the member's local header sits
	cen_at:	usize,		// where this entry begins in the directory
	cen_end:	usize,	// where it ends
}

fn read_cen(src: &[u8], i: usize) -> Outcome<Raw> {
	let flags	= res!(u16le(src, i + 8));
	let method	= res!(u16le(src, i + 10));
	let crc	= res!(u32le(src, i + 16));
	let csize	= res!(u32le(src, i + 20)) as u64;
	let size	= res!(u32le(src, i + 24)) as u64;
	let nlen	= res!(u16le(src, i + 28)) as usize;
	let elen	= res!(u16le(src, i + 30)) as usize;
	let clen	= res!(u16le(src, i + 32)) as usize;
	let at	= res!(u32le(src, i + 42)) as u64;
	let name_at = i + CEN_LEN;
	let name = match src.get(name_at..name_at + nlen) {
		Some(s)	=> String::from_utf8_lossy(s).into_owned(),
		None		=> return Err(err!(
			"A directory entry at byte {} names {} bytes, which runs past the end of the archive.",
			i, nlen; Invalid, Input, Range)),
	};
	let extra_at = name_at + nlen;
	let extra = match src.get(extra_at..extra_at + elen) {
		Some(s)	=> s,
		None		=> return Err(err!(
			"'{}' carries {} bytes of extra field, which runs past the end of the archive.",
			name, elen; Invalid, Input, Range)),
	};
	// ZIP64 puts the real sizes and offset in an extra field, in this order, and only for the fields
	// whose ordinary slot holds the sentinel.
	let (mut size, mut csize, mut at) = (size, csize, at);
	if size == 0xFFFF_FFFF || csize == 0xFFFF_FFFF || at == 0xFFFF_FFFF {
		match find_extra(extra, 0x0001) {
			Some(z)	=> {
				let mut k = 0;
				if size == 0xFFFF_FFFF	{ size = res!(u64le(z, k)); k += 8; }
				if csize == 0xFFFF_FFFF	{ csize = res!(u64le(z, k)); k += 8; }
				if at == 0xFFFF_FFFF	{ at = res!(u64le(z, k)); }
			}
			None		=> return Err(err!(
				"'{}' has a ZIP64 sentinel in its directory entry and no ZIP64 extra field to \
				read the real value from.", name; Invalid, Input, Missing)),
		}
	}
	Ok(Raw {
		name,
		flags,
		method,
		crc,
		csize,
		size,
		at,
		cen_at:	i,
		cen_end:	extra_at + elen + clen,
	})
}

fn find_extra(extra: &[u8], id: u16) -> Option<&[u8]> {
	let mut i = 0;
	while i + 4 <= extra.len() {
		let this = u16::from_le_bytes([extra[i], extra[i + 1]]);
		let len = u16::from_le_bytes([extra[i + 2], extra[i + 3]]) as usize;
		let from = i + 4;
		let to = from.checked_add(len)?;
		if to > extra.len() {
			return None;
		}
		if this == id {
			return Some(&extra[from..to]);
		}
		i = to;
	}
	None
}

/// Found by searching backwards, because the record is last and carries a comment of unknown
/// length after it. The search is bounded by the largest comment the format allows, so a large file
/// that is not an archive is refused after reading its tail rather than after reading all of it.
fn find_eocd(src: &[u8]) -> Outcome<usize> {
	const MIN: usize = 22;
	if src.len() < MIN {
		return Err(err!(
			"{} bytes is too short to be a ZIP archive, which is at least {}.", src.len(), MIN;
			Invalid, Input, Size));
	}
	let floor = src.len().saturating_sub(MIN + MAX_COMMENT);
	let mut i = src.len() - MIN;
	loop {
		if u32::from_le_bytes([src[i], src[i + 1], src[i + 2], src[i + 3]]) == SIG_EOCD {
			// The comment length must account for exactly what follows, or the signature was a
			// coincidence inside somebody's data.
			let clen = res!(u16le(src, i + 20)) as usize;
			if i + MIN + clen == src.len() {
				return Ok(i);
			}
		}
		if i == floor {
			return Err(err!(
				"The bytes end without a ZIP end-of-directory record, so this is not an archive, \
				or it is one that was truncated."; Invalid, Input, Missing));
		}
		i -= 1;
	}
}

/// The member count, directory size and directory offset from the ZIP64 end record.
fn read_eocd64(src: &[u8], eocd: usize) -> Outcome<(u64, u64, u64)> {
	if eocd < 20 {
		return Err(err!(
			"The archive has a ZIP64 sentinel and no room before the end record for the locator \
			that would point at the ZIP64 record."; Invalid, Input, Missing));
	}
	let loc = eocd - 20;
	if res!(u32le(src, loc)) != SIG_LOC64 {
		return Err(err!(
			"The archive has a ZIP64 sentinel and no ZIP64 locator before its end record.";
			Invalid, Input, Missing));
	}
	let at = res!(u64le(src, loc + 8)) as usize;
	if res!(u32le(src, at)) != SIG_EOCD64 {
		return Err(err!(
			"The ZIP64 locator points at byte {}, where there is no ZIP64 end record.", at;
			Invalid, Input));
	}
	let count	= res!(u64le(src, at + 32));
	let cd_len	= res!(u64le(src, at + 40));
	let cd_at	= res!(u64le(src, at + 48));
	Ok((count, cd_len, cd_at))
}
