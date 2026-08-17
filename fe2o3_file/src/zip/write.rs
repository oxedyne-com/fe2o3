//! Writing an archive back out, copying everything nobody touched.
//!
//! A held member is written by copying the bytes it was read from -- header, data, descriptor and
//! directory entry alike -- so a member this never understood comes out exactly as it went in. Only a
//! member the caller replaced is built afresh, and only its directory entry is written rather than
//! copied.
//!
//! The one field a copied directory entry has patched is the offset of the member's local header,
//! because inserting or removing a member moves everything after it. Where nothing moved, nothing is
//! patched, and the bytes out are the bytes in.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::zip::{
	Body,
	Member,
	Method,
	Zip,
	u32le,
};

use oxedyne_fe2o3_core::prelude::*;

/// A central directory entry.
const SIG_CEN:	u32 = 0x0201_4b50;
/// A local file header.
const SIG_LOC:	u32 = 0x0403_4b50;
/// The end of central directory record.
const SIG_EOCD:	u32 = 0x0605_4b50;

/// The version that reads a DEFLATE member: 2.0.
const VERSION:	u16 = 20;
/// Bit 11 of the general purpose flag: the name is UTF-8.
const FLAG_UTF8:	u16 = 0x0800;

impl Zip {

	/// An archive read and written straight back gives the bytes it was read from. That is a property
	/// callers should check rather than take on trust: it is what says this build understood the file
	/// well enough to be handed somebody's document.
	pub fn write(&self) -> Outcome<Vec<u8>> {
		if self.zip64 {
			return Err(err!(
				"This archive needed the ZIP64 records to be read -- it holds more than 65,535 \
				members, or more than 4 GB. ZIP64 is read here and not written, so writing it back \
				would produce a file that is not the one that was opened. The archive is refused \
				rather than damaged."; Unimplemented, Excessive));
		}
		let mut out = Vec::with_capacity(self.src.len() + 4096);
		let mut at = Vec::with_capacity(self.members.len());
		for m in &self.members {
			at.push(out.len() as u64);
			match &m.body {
				Body::Held { whole, .. }	=> {
					let bytes = res!(self.src.get(whole.clone()).ok_or_else(|| err!(
						"'{}' addresses bytes {}..{} of an archive of {} bytes.",
						m.name, whole.start, whole.end, self.src.len(); Bug, Range)));
					out.extend_from_slice(bytes);
				}
				Body::Fresh { data, stamp }	=> res!(fresh(&mut out, m, data, *stamp)),
			}
		}
		let cd_at = out.len();
		for (i, m) in self.members.iter().enumerate() {
			let off = at[i];
			if off > u32::MAX as u64 {
				return Err(err!(
					"'{}' would sit at byte {}, past the 4 GB an ordinary ZIP directory can \
					address. ZIP64 is read here and not written.", m.name, off;
					Unimplemented, Excessive));
			}
			match &m.body {
				Body::Held { cen, .. }	=> {
					let bytes = res!(self.src.get(cen.clone()).ok_or_else(|| err!(
						"'{}' addresses directory bytes {}..{} of an archive of {} bytes.",
						m.name, cen.start, cen.end, self.src.len(); Bug, Range)));
					let from = out.len();
					out.extend_from_slice(bytes);
					// The only field that can have moved. Every other byte of the entry is the
					// archive's own.
					let was = res!(u32le(bytes, 42));
					if was as u64 != off {
						let k = from + 42;
						out[k..k + 4].copy_from_slice(&(off as u32).to_le_bytes());
					}
				}
				Body::Fresh { data, stamp }	=> res!(cen_fresh(&mut out, m, data, *stamp, off as u32)),
			}
		}
		let cd_len = out.len() - cd_at;
		if self.members.len() > 0xFFFF {
			return Err(err!(
				"The archive holds {} members, past the 65,535 an ordinary ZIP directory can \
				count. ZIP64 is read here and not written.", self.members.len();
				Unimplemented, Excessive));
		}
		let n = self.members.len() as u16;
		out.extend_from_slice(&SIG_EOCD.to_le_bytes());
		out.extend_from_slice(&0u16.to_le_bytes());	// This disk.
		out.extend_from_slice(&0u16.to_le_bytes());	// The disk the directory starts on.
		out.extend_from_slice(&n.to_le_bytes());
		out.extend_from_slice(&n.to_le_bytes());
		out.extend_from_slice(&(cd_len as u32).to_le_bytes());
		out.extend_from_slice(&(cd_at as u32).to_le_bytes());
		out.extend_from_slice(&(self.comment.len() as u16).to_le_bytes());
		out.extend_from_slice(&self.comment);
		Ok(out)
	}
}

/// Writes a fresh member's local header and its bytes.
fn fresh(
	out:	&mut Vec<u8>,
	m:	&Member,
	data:	&[u8],
	stamp:	(u16, u16),
)
	-> Outcome<()>
{
	let body = res!(pack(m, data));
	let name = m.name.as_bytes();
	out.extend_from_slice(&SIG_LOC.to_le_bytes());
	out.extend_from_slice(&VERSION.to_le_bytes());
	out.extend_from_slice(&flags(&m.name).to_le_bytes());
	out.extend_from_slice(&m.method.code().to_le_bytes());
	out.extend_from_slice(&stamp.0.to_le_bytes());
	out.extend_from_slice(&stamp.1.to_le_bytes());
	out.extend_from_slice(&m.crc.to_le_bytes());
	out.extend_from_slice(&(body.len() as u32).to_le_bytes());
	out.extend_from_slice(&(data.len() as u32).to_le_bytes());
	out.extend_from_slice(&(name.len() as u16).to_le_bytes());
	out.extend_from_slice(&0u16.to_le_bytes());	// No extra field.
	out.extend_from_slice(name);
	out.extend_from_slice(&body);
	Ok(())
}

/// Writes a fresh member's central directory entry.
fn cen_fresh(
	out:	&mut Vec<u8>,
	m:	&Member,
	data:	&[u8],
	stamp:	(u16, u16),
	off:	u32,
)
	-> Outcome<()>
{
	let body = res!(pack(m, data));
	let name = m.name.as_bytes();
	out.extend_from_slice(&SIG_CEN.to_le_bytes());
	out.extend_from_slice(&VERSION.to_le_bytes());	// Made by version 2.0, host MS-DOS.
	out.extend_from_slice(&VERSION.to_le_bytes());
	out.extend_from_slice(&flags(&m.name).to_le_bytes());
	out.extend_from_slice(&m.method.code().to_le_bytes());
	out.extend_from_slice(&stamp.0.to_le_bytes());
	out.extend_from_slice(&stamp.1.to_le_bytes());
	out.extend_from_slice(&m.crc.to_le_bytes());
	out.extend_from_slice(&(body.len() as u32).to_le_bytes());
	out.extend_from_slice(&(data.len() as u32).to_le_bytes());
	out.extend_from_slice(&(name.len() as u16).to_le_bytes());
	out.extend_from_slice(&0u16.to_le_bytes());	// No extra field.
	out.extend_from_slice(&0u16.to_le_bytes());	// No comment.
	out.extend_from_slice(&0u16.to_le_bytes());	// Disk zero.
	out.extend_from_slice(&0u16.to_le_bytes());	// Internal attributes.
	out.extend_from_slice(&0u32.to_le_bytes());	// External attributes.
	out.extend_from_slice(&off.to_le_bytes());
	out.extend_from_slice(name);
	Ok(())
}

/// A fresh member's bytes as the archive will hold them.
///
/// Called twice for the same member -- once for the local header and once for the directory entry,
/// which must agree about the compressed size -- so it is deterministic by construction rather than
/// by a cached size that could go stale.
fn pack(m: &Member, data: &[u8]) -> Outcome<Vec<u8>> {
	match m.method {
		Method::Store	=> Ok(data.to_vec()),
		Method::Deflate	=> {
			use std::io::Write;
			let mut w = flate2::write::DeflateEncoder::new(
				Vec::new(),
				flate2::Compression::default(),
			);
			res!(w.write_all(data), IO);
			Ok(res!(w.finish(), IO))
		}
		Method::Other(c)	=> Err(err!(
			"'{}' was given to the archive under compression method {}, which this does not \
			write.", m.name, c; Unimplemented)),
	}
}

/// Bit 11 is set only where the name needs it. Setting it on an ASCII name would be legal and would
/// still change the bytes of every archive written here, for nothing.
fn flags(name: &str) -> u16 {
	match name.is_ascii() {
		true	=> 0,
		false	=> FLAG_UTF8,
	}
}
