//! The Lempel-Ziv-Welch decoder GIF and TIFF share.
//!
//! The two formats agree on the algorithm and disagree on three details, which is why one decoder
//! takes them as arguments rather than each format owning a copy: GIF packs its codes least
//! significant bit first and TIFF most significant bit first; GIF starts at any code size from two
//! to eight bits and TIFF always at eight; and TIFF widens the code one entry early, when the table
//! is one short of the next power of two, a quirk of the first encoder that the specification then
//! wrote down as the rule.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

const MAX_BITS:		u32	= 12;
const TABLE:		usize	= 1 << MAX_BITS;

/// Which end of each byte the codes are read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitOrder {
	Lsb,	// GIF
	Msb,	// TIFF
}

/// Decodes an LZW stream into at most `limit` bytes.
///
/// A stream that ends without its end-of-information code is taken as ended rather than refused:
/// GIF writers and TIFF writers alike are known to leave it off, and every byte before it decoded
/// correctly. A code that names an entry not yet in the table is refused, since nothing after it can
/// be trusted.
///
/// # Arguments
/// * `min` - The literal code width: GIF's minimum code size, or 8 for TIFF.
/// * `early` - Whether the code widens one entry early, as TIFF's does.
pub fn decode(data: &[u8], min: u8, order: BitOrder, early: bool, limit: usize)
	-> Outcome<Vec<u8>>
{
	if min < 1 || min as u32 >= MAX_BITS {
		return Err(err!(
			"An LZW literal width of {} bits is outside 1 to {}.", min, MAX_BITS - 1;
		Invalid, Input, Decode));
	}
	let clear = 1usize << min;
	let eoi = clear + 1;
	let mut prefix = vec![0u16; TABLE];
	let mut suffix = vec![0u8; TABLE];
	let mut first = vec![0u8; TABLE];
	let mut length = vec![0u16; TABLE];
	for i in 0..clear {
		suffix[i] = i as u8;
		first[i] = i as u8;
		length[i] = 1;
	}
	let mut next = eoi + 1;
	let mut size = min as u32 + 1;
	let mut prev: Option<usize> = None;
	let mut out: Vec<u8> = Vec::with_capacity(limit.min(data.len().saturating_mul(4)));

	let mut acc: u32 = 0;	// bits not yet consumed
	let mut have: u32 = 0;	// how many of them there are
	let mut pos = 0;

	loop {
		while have < size && pos < data.len() {
			match order {
				BitOrder::Lsb => acc |= (data[pos] as u32) << have,
				BitOrder::Msb => acc = (acc << 8) | data[pos] as u32,
			}
			have += 8;
			pos += 1;
		}
		if have < size {
			break; // the stream ran out without its end code
		}
		let mask = (1u32 << size) - 1;
		let code = match order {
			BitOrder::Lsb => {
				let c = acc & mask;
				acc >>= size;
				c
			}
			BitOrder::Msb => {
				let c = (acc >> (have - size)) & mask;
				acc &= (1u32 << (have - size)) - 1;
				c
			}
		} as usize;
		have -= size;

		if code == clear {
			next = eoi + 1;
			size = min as u32 + 1;
			prev = None;
			continue;
		}
		if code == eoi {
			break;
		}
		let p = match prev {
			None => {
				if code >= clear {
					return Err(err!(
						"An LZW stream opens with code {}, which is not a literal (below {}).",
						code, clear;
					Invalid, Input, Decode));
				}
				if out.len() >= limit {
					break;
				}
				out.push(code as u8);
				prev = Some(code);
				continue;
			}
			Some(p) => p,
		};
		// The entry this code adds is the previous string and the first byte of this one, which
		// for the code not yet in the table is the previous string's own first byte.
		let head = if code < next {
			first[code]
		} else if code == next && next < TABLE {
			first[p]
		} else {
			return Err(err!(
				"An LZW code of {} arrived when the table held only {} entries.", code, next;
			Invalid, Input, Decode));
		};
		if next < TABLE {
			prefix[next] = p as u16;
			suffix[next] = head;
			first[next] = first[p];
			length[next] = length[p] + 1;
			next += 1;
		}
		let n = length[code] as usize;
		let start = out.len();
		if start + n > limit {
			return Err(err!(
				"An LZW stream expands past the {} bytes its image can hold.", limit;
			Invalid, Input, Decode, Excessive));
		}
		out.resize(start + n, 0);
		let mut c = code;
		for i in (0..n).rev() {
			out[start + i] = suffix[c];
			c = prefix[c] as usize;
		}
		let widen = if early { next + 1 } else { next };
		if widen >= (1usize << size) && size < MAX_BITS {
			size += 1;
		}
		prev = Some(code);
	}
	Ok(out)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Packs codes of the given widths the way the named order reads them.
	fn pack(codes: &[(u32, u32)], order: BitOrder) -> Vec<u8> {
		let mut out = Vec::new();
		let mut acc: u64 = 0;
		let mut have = 0u32;
		for &(c, w) in codes {
			match order {
				BitOrder::Lsb => acc |= (c as u64) << have,
				BitOrder::Msb => acc = (acc << w) | c as u64,
			}
			have += w;
			while have >= 8 {
				match order {
					BitOrder::Lsb => {
						out.push((acc & 0xff) as u8);
						acc >>= 8;
					}
					BitOrder::Msb => {
						out.push(((acc >> (have - 8)) & 0xff) as u8);
					}
				}
				have -= 8;
			}
		}
		if have > 0 {
			match order {
				BitOrder::Lsb => out.push(acc as u8),
				BitOrder::Msb => out.push(((acc << (8 - have)) & 0xff) as u8),
			}
		}
		out
	}

	#[test]
	fn test_gif_style_stream_with_the_unknown_code_case_00() -> Outcome<()> {
		// Literals 0..3, clear 4, eoi 5. "ABABABA" as 0 1 6 8 with A=0, B=1.
		let codes = [(4, 3), (0, 3), (1, 3), (6, 3), (8, 4), (5, 4)];
		let out = res!(decode(&pack(&codes, BitOrder::Lsb), 2, BitOrder::Lsb, false, 64));
		assert_eq!(out, vec![0, 1, 0, 1, 0, 1, 0]);
		Ok(())
	}

	#[test]
	fn test_tiff_style_stream_widens_early_00() -> Outcome<()> {
		// 8-bit literals, clear 256, eoi 257. Enough literals to fill the table to 511, at which
		// point TIFF moves to ten bits one entry early.
		let mut codes = vec![(256u32, 9u32)];
		let mut want = Vec::new();
		// Each literal after the first adds one entry: 258 + 253 = 511 after 254 literals, and
		// with 511 next the code is ten bits wide, where GIF would wait for 512.
		for i in 0..254u32 {
			codes.push((i, 9));
			want.push(i as u8);
		}
		codes.push((257, 10));
		let out = res!(decode(&pack(&codes, BitOrder::Msb), 8, BitOrder::Msb, true, 4096));
		assert_eq!(out, want);
		Ok(())
	}

	#[test]
	fn test_a_code_beyond_the_table_is_refused_00() {
		let codes = [(4, 3), (0, 3), (7, 3)];
		assert!(decode(&pack(&codes, BitOrder::Lsb), 2, BitOrder::Lsb, false, 64).is_err());
	}
}
