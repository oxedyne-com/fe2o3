//! GIF, as its first frame.
//!
//! An animated GIF is a sequence of frames each drawn over the last, and what a reader that does not
//! animate shows is the first of them on the logical screen. That is what comes back here: the
//! screen, cleared, with the first image drawn onto it at its offset, its transparent index left
//! clear and its rows put back in order if they were stored interlaced.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
	lzw::{
		self,
		BitOrder,
	},
	pixmap::Pixmap,
};

use oxedyne_fe2o3_core::prelude::*;

fn u16le(b: &[u8], at: usize) -> Outcome<u16> {
	match b.get(at..at + 2) {
		Some(s) => Ok(u16::from_le_bytes([s[0], s[1]])),
		None => Err(err!(
			"A GIF needs two bytes at offset {}, but holds {}.", at, b.len();
		Invalid, Input, Decode)),
	}
}

fn byte(b: &[u8], at: usize) -> Outcome<u8> {
	match b.get(at) {
		Some(v) => Ok(*v),
		None => Err(err!(
			"A GIF ends at {} bytes where offset {} was needed.", b.len(), at;
		Invalid, Input, Decode)),
	}
}

/// Reads a colour table of `n` entries at `at`.
fn table(b: &[u8], at: usize, n: usize) -> Outcome<Vec<[u8; 3]>> {
	match b.get(at..at + 3 * n) {
		Some(s) => Ok(s.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect()),
		None => Err(err!(
			"A GIF colour table of {} entries at offset {} passes the end of the file.", n, at;
		Invalid, Input, Decode)),
	}
}

/// Gathers a run of data sub-blocks, returning the bytes and the offset after the terminator.
fn blocks(b: &[u8], mut at: usize) -> Outcome<(Vec<u8>, usize)> {
	let mut out = Vec::new();
	loop {
		let n = match b.get(at) {
			Some(n) => *n as usize,
			None => return Ok((out, at)), // a file cut short keeps what it has
		};
		at += 1;
		if n == 0 {
			return Ok((out, at));
		}
		let end = (at + n).min(b.len());
		out.extend_from_slice(&b[at..end]);
		at += n;
	}
}

/// The logical screen size a GIF declares.
pub fn dimensions(buf: &[u8]) -> Outcome<(usize, usize)> {
	if buf.len() < 10 || (&buf[..6] != b"GIF87a" && &buf[..6] != b"GIF89a") {
		return Err(err!("The bytes do not begin with a GIF signature."; Invalid, Input, Decode));
	}
	Ok((res!(u16le(buf, 6)) as usize, res!(u16le(buf, 8)) as usize))
}

/// Decodes a GIF's first frame onto its logical screen.
pub fn decode(buf: &[u8]) -> Outcome<Pixmap> {
	let (sw, sh) = res!(dimensions(buf));
	let flags = res!(byte(buf, 10));
	let mut at = 13;
	let global = if flags & 0x80 != 0 {
		let n = 2usize << (flags & 7);
		let t = res!(table(buf, at, n));
		at += 3 * n;
		t
	} else {
		Vec::new()
	};
	let mut transparent: Option<u8> = None;
	loop {
		match res!(byte(buf, at)) {
			0x21 => {
				let label = res!(byte(buf, at + 1));
				let (data, next) = res!(blocks(buf, at + 2));
				if label == 0xf9 && data.len() >= 4 && data[0] & 1 == 1 {
					transparent = Some(data[3]);
				}
				at = next;
			}
			0x2c => break,
			0x3b => return Err(err!("A GIF ends before its first image."; Invalid, Input, Decode)),
			b => return Err(err!(
				"A GIF holds block type 0x{:02x} at offset {}, which is none of image, extension \
				or trailer.", b, at;
			Invalid, Input, Decode)),
		}
	}
	let left = res!(u16le(buf, at + 1)) as usize;
	let top = res!(u16le(buf, at + 3)) as usize;
	let fw = res!(u16le(buf, at + 5)) as usize;
	let fh = res!(u16le(buf, at + 7)) as usize;
	let iflags = res!(byte(buf, at + 9));
	at += 10;
	let local = if iflags & 0x80 != 0 {
		let n = 2usize << (iflags & 7);
		let t = res!(table(buf, at, n));
		at += 3 * n;
		Some(t)
	} else {
		None
	};
	let colours = match &local {
		Some(t) => t,
		None => &global,
	};
	let min = res!(byte(buf, at));
	let (data, _) = res!(blocks(buf, at + 1));
	let n = match fw.checked_mul(fh) {
		Some(n) => n,
		None => return Err(err!(
			"A GIF frame of {} by {} overflows.", fw, fh; Invalid, Input, Decode, Overflow)),
	};
	let idx = res!(lzw::decode(&data, min, BitOrder::Lsb, false, n));
	// A screen smaller than its first frame is a writer's mistake; grow it rather than crop.
	let w = sw.max(left + fw).max(1);
	let h = sh.max(top + fh).max(1);
	let mut pm = res!(Pixmap::new(w, h));
	let order = rows(fh, iflags & 0x40 != 0);
	let out = pm.data_mut();
	for (k, &y) in order.iter().enumerate() {
		for x in 0..fw {
			let i = match idx.get(k * fw + x) {
				Some(i) => *i,
				None => continue, // a short stream leaves the rest clear
			};
			if Some(i) == transparent {
				continue;
			}
			let c = match colours.get(i as usize) {
				Some(c) => *c,
				None => [0, 0, 0],
			};
			let p = ((top + y) * w + left + x) * 4;
			out[p..p + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
		}
	}
	Ok(pm)
}

/// The row each stored row belongs on, which for an interlaced image is four passes: every eighth
/// row from 0, every eighth from 4, every fourth from 2, every second from 1.
fn rows(h: usize, interlaced: bool) -> Vec<usize> {
	if !interlaced {
		return (0..h).collect();
	}
	let mut v = Vec::with_capacity(h);
	for &(start, step) in &[(0, 8), (4, 8), (2, 4), (1, 2)] {
		let mut y = start;
		while y < h {
			v.push(y);
			y += step;
		}
	}
	v
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_interlaced_row_order_00() {
		assert_eq!(rows(10, true), vec![0, 8, 4, 2, 6, 1, 3, 5, 7, 9]);
		assert_eq!(rows(3, false), vec![0, 1, 2]);
	}
}
