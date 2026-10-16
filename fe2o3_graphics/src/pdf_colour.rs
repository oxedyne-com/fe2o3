//! How the PDF writer sets a colour, and the colours it converts.
//!
//! A [`ColourOut`] says what a file holds. `Native` writes each ink in its own space and `Rgb` lowers
//! every ink to sRGB, which is what every file written before an ink knew its space holds. `Cmyk` and
//! `Grey` convert: each fill, stroke and text colour reaches the file in the output space, and each
//! colour raster is taken as sRGB and converted through an ICC [`Transform`], so that nothing in the file is
//! written as `rg`, `RG` or `/DeviceRGB`.
//!
//! A `/DeviceGray` value is a subset of every output space, as Ghostscript keeps it: a grey ink, and a raster
//! whose source held grey alone, reach a grey output unchanged.
//!
//! # Black
//!
//! Under [`Black::KOnly`] a grey ink, and an RGB ink whose three channels are equal, is written as `g`
//! or `G`, which a CMYK device prints with the black ink alone, so that text and rules set in black
//! carry no cyan, magenta or yellow. [`Black::Rich`] sends the same inks through the transform, which
//! gives the profile's own rich black. A CMYK ink is kept as it is under `Cmyk`. A raster from a grey source
//! ([`Tone::Grey`]) is written as `/DeviceGray` under K only and goes through the transform under a rich black,
//! as a grey ink does.
//!
//! # Images
//!
//! A converted image is held by the identity of its samples and of the transform that made it, in a
//! cache the writer shares with every writer cloned from the same [`ColourOut`]. A watch builds one
//! per settings load, so a figure unchanged between two compiles is converted once. The cache is held
//! to a budget of bytes, and drops the image least recently read when a new one would pass it.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::colour::Ink;
use crate::icc::transform::Transform;
use crate::pixmap::Tone;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::fingerprint::{
	Fingerprint,
	Fingerprinter,
};

use std::collections::HashMap;
use std::sync::{
	Arc,
	Mutex,
};

// The bytes of converted samples a cache holds before it drops the least recently read.
const BUDGET:	usize = 128 << 20;

// Pixels converted at a time, so that a large raster never holds its whole result as floats.
const CHUNK:	usize = 1 << 16;

// The inks one page converts before its memo is emptied.
const MEMO:		usize = 256;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Black {
	#[default]
	KOnly,
	Rich,
}

/// How the writer sets a colour. `Rgb` lowers every ink to sRGB and writes `rg` and `RG`, the bytes a
/// file carried before an ink knew its space; `Native` writes each ink in its own space, `g` for a
/// grey, `k` for a CMYK and `rg` for the rest, and the same for the stroke forms. `Cmyk` and `Grey`
/// convert every colour to the output space.
#[derive(Clone, Debug, Default)]
pub enum ColourOut {
	Native,
	#[default]
	Rgb,
	Cmyk {
		xf:		Arc<Transform>,		// sRGB (or the RGB profile) to the CMYK profile
		black:	Black,
		images:	ImageCache,
	},
	Grey {
		xf:		Arc<Transform>,		// sRGB (or the RGB profile) to the grey profile
		cmyk:	Arc<Transform>,		// the CMYK profile to the grey profile, for a CMYK ink
		images:	ImageCache,
	},
}

// Two modes are equal when they write the same file: the transforms are compared by identity, and a cache is
// no part of what is written.
impl PartialEq for ColourOut {
	fn eq(&self, other: &Self) -> bool {
		match (self, other) {
			(Self::Native, Self::Native)	=> true,
			(Self::Rgb, Self::Rgb)			=> true,
			(Self::Cmyk { xf: a, black: p, .. }, Self::Cmyk { xf: b, black: q, .. })
				=> a.id() == b.id() && p == q,
			(Self::Grey { xf: a, cmyk: p, .. }, Self::Grey { xf: b, cmyk: q, .. })
				=> a.id() == b.id() && p.id() == q.id(),
			_								=> false,
		}
	}
}

impl Eq for ColourOut {}

impl ColourOut {

	/// A CMYK output through `xf`, which takes RGB to CMYK, with a cache of its own for images.
	pub fn cmyk(xf: Arc<Transform>, black: Black) -> Outcome<Self> {
		res!(xf.rgb_to_cmyk([0.0; 3]));
		Ok(Self::Cmyk { xf, black, images: ImageCache::default() })
	}

	/// A grey output through `xf`, which takes RGB to grey, and `cmyk`, which takes CMYK to grey.
	pub fn grey(xf: Arc<Transform>, cmyk: Arc<Transform>) -> Outcome<Self> {
		res!(xf.rgb_to_grey([0.0; 3]));
		res!(cmyk.cmyk_to_grey([0.0; 4]));
		Ok(Self::Grey { xf, cmyk, images: ImageCache::default() })
	}

	/// Does this mode convert colours, rather than write them as they come?
	pub fn converts(&self) -> bool {
		matches!(self, Self::Cmyk { .. } | Self::Grey { .. })
	}

	/// The colour space a converting mode names for each page's transparency group.
	pub fn group_space(&self) -> Option<&'static str> {
		match self {
			Self::Cmyk { .. }	=> Some("DeviceCMYK"),
			Self::Grey { .. }	=> Some("DeviceGray"),
			_					=> None,
		}
	}

	/// The cache of converted images, none in a mode that writes images as they come.
	pub fn images(&self) -> Option<&ImageCache> {
		match self {
			Self::Cmyk { images, .. } | Self::Grey { images, .. }	=> Some(images),
			_														=> None,
		}
	}

	/// The ink as this mode writes it. Under `Rgb` an ink is lowered to sRGB, and under `Cmyk` and `Grey` it
	/// is converted: the result is grey or CMYK, never RGB.
	pub fn ink(&self, ink: Ink) -> Outcome<Ink> {
		Ok(match self {
			Self::Native				=> ink,
			Self::Rgb					=> Ink::Rgb(ink.to_rgba()),
			Self::Cmyk { xf, black, .. }	=> match ink {
				Ink::Cmyk { .. }						=> ink,
				Ink::Grey { .. } if *black == Black::KOnly	=> ink,
				Ink::Rgb(c) if *black == Black::KOnly && c.r == c.g && c.g == c.b
					=> Ink::Grey { v: c.r as f32 / 255.0, a: c.a },
				_										=> {
					let k = res!(xf.rgb_to_cmyk(rgb_of(ink)));
					Ink::Cmyk { c: k[0], m: k[1], y: k[2], k: k[3], a: ink.alpha() }
				},
			},
			Self::Grey { xf, cmyk, .. }	=> match ink {
				Ink::Grey { .. }			=> ink,
				Ink::Cmyk { c, m, y, k, a }	=> {
					let v = res!(cmyk.cmyk_to_grey([c as f64, m as f64, y as f64, k as f64]));
					Ink::Grey { v, a }
				},
				_							=> Ink::Grey { v: res!(xf.rgb_to_grey(rgb_of(ink))), a: ink.alpha() },
			},
		})
	}

	/// Converts a raster of packed 8-bit sRGB, returning its channel count and its samples, or none in a mode that
	/// writes a raster as it comes. A raster seen before, through the same transform, is read from the cache. A
	/// raster of `Tone::Grey`, whose three samples are equal, is kept as one channel of grey except under a rich
	/// black.
	pub fn image(&self, rgb: &[u8], iw: usize, ih: usize, tone: Tone) -> Outcome<Option<(usize, Arc<Vec<u8>>)>> {
		let (xf, images, chans, keep) = match self {
			Self::Cmyk { xf, images, black }	=> (xf, images, 4, *black == Black::KOnly),
			Self::Grey { xf, images, .. }		=> (xf, images, 1, true),
			_									=> return Ok(None),
		};
		if rgb.len() != iw * ih * 3 {
			return Err(err!(
				"An image of {} by {} samples holds {} bytes of RGB, not the {} it should.",
				iw, ih, rgb.len(), iw * ih * 3; Invalid, Input, Size));
		}
		if tone == Tone::Grey && keep {
			return Ok(Some((1, Arc::new(rgb.iter().step_by(3).copied().collect()))));
		}
		let mut f = Fingerprinter::new();
		f.write_usize(iw);
		f.write_usize(ih);
		f.write(rgb);
		let key = (f.finish(), xf.id());
		if let Some(hit) = res!(images.get(&key)) {
			return Ok(Some((chans, hit)));
		}
		let grid = res!(xf.grid());
		let mut out = Vec::with_capacity(iw * ih * chans);
		for part in rgb.chunks(CHUNK * 3) {
			for px in res!(grid.convert8(part)) {
				for c in &px[..chans] {
					out.push((c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
				}
			}
		}
		let out = Arc::new(out);
		res!(images.put(key, out.clone()));
		Ok(Some((chans, out)))
	}
}

// The colour as fractions of full scale in sRGB, for an ink that is not CMYK.
fn rgb_of(ink: Ink) -> [f64; 3] {
	let c = ink.to_rgba();
	[c.r as f64 / 255.0, c.g as f64 / 255.0, c.b as f64 / 255.0]
}

/// The inks of one page as the mode writes them, each converted once. A document sets thousands of glyphs
/// in a handful of colours, so a conversion through the profiles is looked up rather than made again.
pub(crate) struct Inks<'a> {
	out:	&'a ColourOut,
	memo:	HashMap<Ink, Ink>,
}

impl<'a> Inks<'a> {

	pub(crate) fn new(out: &'a ColourOut) -> Self {
		Self { out, memo: HashMap::new() }
	}

	pub(crate) fn get(&mut self, ink: Ink) -> Outcome<Ink> {
		if !self.out.converts() {
			return self.out.ink(ink);
		}
		if let Some(done) = self.memo.get(&ink) {
			return Ok(*done);
		}
		let done = res!(self.out.ink(ink));
		if self.memo.len() >= MEMO {
			self.memo.clear();
		}
		self.memo.insert(ink, done);
		Ok(done)
	}
}

type Key = (Fingerprint, u64);	// the samples and size, then the transform

#[derive(Debug)]
struct Entry {
	data:	Arc<Vec<u8>>,
	read:	u64,		// the tick of the last read, for dropping the least recent
}

#[derive(Debug, Default)]
struct Held {
	map:	HashMap<Key, Entry>,
	bytes:	usize,
	tick:	u64,
	hits:	u64,
	misses:	u64,
}

/// What a cache holds and has done.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
	pub hits:	u64,
	pub misses:	u64,
	pub held:	usize,		// images
	pub bytes:	usize,		// of converted samples
}

/// Converted samples, shared by every clone of the handle.
#[derive(Clone, Debug, Default)]
pub struct ImageCache {
	held:	Arc<Mutex<Held>>,
}

impl ImageCache {

	/// What the cache holds, and how often it has been asked for what it held and for what it did not.
	pub fn stats(&self) -> Outcome<CacheStats> {
		let h = lock_mutex!(self.held);
		Ok(CacheStats { hits: h.hits, misses: h.misses, held: h.map.len(), bytes: h.bytes })
	}

	fn get(&self, key: &Key) -> Outcome<Option<Arc<Vec<u8>>>> {
		let mut h = lock_mutex!(self.held);
		h.tick += 1;
		let tick = h.tick;
		let found = h.map.get_mut(key).map(|e| {
			e.read = tick;
			e.data.clone()
		});
		match found {
			Some(_)	=> h.hits += 1,
			None	=> h.misses += 1,
		}
		Ok(found)
	}

	fn put(&self, key: Key, data: Arc<Vec<u8>>) -> Outcome<()> {
		let mut h = lock_mutex!(self.held);
		h.tick += 1;
		let tick = h.tick;
		let size = data.len();
		if let Some(old) = h.map.insert(key, Entry { data, read: tick }) {
			h.bytes -= old.data.len();
		}
		h.bytes += size;
		// An image larger than the whole budget is held alone, as the only one the next compile can reuse.
		while h.bytes > BUDGET && h.map.len() > 1 {
			let oldest = h.map.iter().filter(|(k, _)| **k != key).min_by_key(|(_, e)| e.read).map(|(k, _)| *k);
			match oldest {
				Some(k)	=> {
					if let Some(gone) = h.map.remove(&k) {
						h.bytes -= gone.data.len();
					}
				},
				None	=> break,
			}
		}
		Ok(())
	}
}
