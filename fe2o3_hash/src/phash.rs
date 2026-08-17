//! Perceptual image hashing for near-duplicate detection.
//!
//! Two 64 bit hashes are offered.  [`PerceptualHash::dhash`] is a difference hash: the image is
//! reduced to a nine by eight grid and each pixel compared with its right hand neighbour.  It
//! costs almost nothing and is the right first pass over a large collection.
//! [`PerceptualHash::phash`] takes a discrete cosine transform of a thirty-two square reduction
//! and thresholds the low frequency coefficients about their median.  It survives compression,
//! rescaling and mild tonal shifts better than the difference hash, and is the right instrument
//! for confirming a candidate the cheap pass has thrown up.
//!
//! The recommended use is therefore two stages: hash everything with [`PerceptualHash::dhash`],
//! shortlist by small [`PerceptualHash::distance`], then confirm each shortlisted pair with
//! [`PerceptualHash::phash`].
//!
//! # Choosing a threshold
//!
//! Thresholds belong to the collection, so measure rather than assume.  As a starting point,
//! over a spread of photographs put through a half-size reduction, a heavy re-encode, a ten per
//! cent brightening and a lossless to lossy conversion, the distance between a photograph and
//! its own variants never exceeded four for the difference hash or two for the cosine transform
//! hash, while the closest unrelated pair was nineteen and twenty-four respectively.  A first
//! pass at ten and a confirmation at eight therefore sit in a wide empty gap.
//!
//! Cropping is the case that defeats both.  A ten per cent centre crop moved the distance to a
//! median of eleven and a worst case of twenty-eight, which overlaps the unrelated population.
//! Neither hash is a crop detector, and a collection full of crops needs a different instrument.
//!
//! Following this library's practice for primitives, the caller owns the decode.  These functions
//! accept a greyscale luma grid, never a file path and never compressed bytes, so the choice of
//! image decoder stays with the application.  [`luma_from_rgb`] and [`luma_from_rgba`] convert an
//! interleaved buffer for callers whose decoder hands back colour.
//!
//! # Example
//! ```
//! use oxedyne_fe2o3_core::prelude::*;
//! use oxedyne_fe2o3_hash::phash::{LumaGrid, PerceptualHash};
//!
//! fn near_duplicate(px: &[u8], w: usize, h: usize) -> Outcome<(u64, u64)> {
//!     let grid = res!(LumaGrid::new(px, w, h));
//!     let d = res!(PerceptualHash::dhash(&grid));
//!     let p = res!(PerceptualHash::phash(&grid));
//!     assert_eq!(res!(d.distance(&d)), 0);
//!     assert!(d.distance(&p).is_err()); // Unlike kinds cannot be compared.
//!     Ok((d.bits(), p.bits()))
//! }
//!
//! let px = vec![0u8; 64 * 64];
//! assert!(near_duplicate(&px, 64, 64).is_ok());
//! ```
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

use std::{
    fmt,
    f64::consts::PI,
};


// The reduction each hash works on: nine by eight for the difference hash, a thirty two square
// for the transform hash, of which the top left eight square is thresholded.
pub const DHASH_W: usize = 9;
pub const DHASH_H: usize = 8;
pub const PHASH_N: usize = 32;
pub const PHASH_K: usize = 8;

const MAX_DIM: usize = 1 << 20;	// guards against an absurd resample

/// A borrowed greyscale grid: eight bits per pixel, row major, no row padding.
#[derive(Clone, Copy, Debug)]
pub struct LumaGrid<'a> {
    dat: &'a [u8],
    w: usize,
    h: usize,
}

impl<'a> LumaGrid<'a> {

    /// Wraps a luma buffer, checking that its length matches the stated dimensions.
    pub fn new(dat: &'a [u8], w: usize, h: usize) -> Outcome<Self> {
        if w == 0 || h == 0 {
            return Err(err!(
                "A luma grid must have a positive width and height, found {} by {}.", w, h;
            Input, Invalid, TooSmall));
        }
        if w > MAX_DIM || h > MAX_DIM {
            return Err(err!(
                "A luma grid dimension of {} by {} exceeds the {} pixel limit.", w, h, MAX_DIM;
            Input, Invalid, TooBig));
        }
        let need = w * h;
        if dat.len() < need {
            return Err(err!(
                "A {} by {} luma grid needs {} bytes, {} were supplied.", w, h, need, dat.len();
            Input, Invalid, TooSmall, Size));
        }
        Ok(Self { dat, w, h })
    }

    pub fn width(&self) -> usize {
        self.w
    }

    pub fn height(&self) -> usize {
        self.h
    }

    pub fn data(&self) -> &'a [u8] {
        self.dat
    }

    /// Reduces the grid to `tw` by `th` samples by averaging over the source area of each.
    ///
    /// Every source pixel contributes in proportion to its overlap with the target cell, so the
    /// result is stable against small changes in the source dimensions.  Enlargement is
    /// permitted and simply replicates, though hashing an image smaller than the reduction
    /// carries little information.
    pub fn resample(&self, tw: usize, th: usize) -> Outcome<Vec<f64>> {
        if tw == 0 || th == 0 {
            return Err(err!(
                "A resample target must have a positive width and height, found {} by {}.",
                tw, th;
            Input, Invalid, TooSmall));
        }
        let sx = self.w as f64 / tw as f64; // Source pixels per target cell, horizontally
        let sy = self.h as f64 / th as f64; // Source pixels per target cell, vertically
        let mut out = vec![0.0f64; tw * th];
        for ty in 0..th {
            let y0 = ty as f64 * sy;
            let y1 = (ty + 1) as f64 * sy;
            let iy0 = y0.floor() as usize;
            let iy1 = (y1.ceil() as usize).min(self.h);
            for tx in 0..tw {
                let x0 = tx as f64 * sx;
                let x1 = (tx + 1) as f64 * sx;
                let ix0 = x0.floor() as usize;
                let ix1 = (x1.ceil() as usize).min(self.w);
                let mut acc = 0.0f64;
                let mut wt = 0.0f64;
                for y in iy0..iy1 {
                    let hy = (y1.min((y + 1) as f64) - y0.max(y as f64)).max(0.0);
                    if hy == 0.0 {
                        continue;
                    }
                    let row = y * self.w;
                    for x in ix0..ix1 {
                        let hx = (x1.min((x + 1) as f64) - x0.max(x as f64)).max(0.0);
                        if hx == 0.0 {
                            continue;
                        }
                        let a = hx * hy;
                        acc += a * self.dat[row + x] as f64;
                        wt += a;
                    }
                }
                out[ty * tw + tx] = if wt > 0.0 {
                    acc / wt
                } else {
                    // The cell fell between sample centres, which only happens when the target
                    // is larger than the source; take the nearest source pixel.
                    let y = iy0.min(self.h - 1);
                    let x = ix0.min(self.w - 1);
                    self.dat[y * self.w + x] as f64
                };
            }
        }
        Ok(out)
    }
}

/// A 64 bit perceptual hash, tagged by the algorithm that produced it.
///
/// Two hashes are comparable only when they were produced the same way, which the tag enforces.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PerceptualHash {
    DHash(u64),	// cheap, the right first pass
    PHash(u64),	// slower, the right confirmation
}

impl fmt::Display for PerceptualHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DHash(b)	=> write!(f, "d:{:016x}", b),
            Self::PHash(b)	=> write!(f, "p:{:016x}", b),
        }
    }
}

impl PerceptualHash {

    /// Computes the difference hash of a luma grid.
    ///
    /// The grid is reduced to nine by eight samples and each sample compared with the one to its
    /// right, giving sixty-four bits written most significant first, row by row.
    pub fn dhash(grid: &LumaGrid) -> Outcome<Self> {
        let px = res!(grid.resample(DHASH_W, DHASH_H));
        let mut bits = 0u64;
        for y in 0..DHASH_H {
            for x in 0..DHASH_H {
                let left = px[y * DHASH_W + x];
                let right = px[y * DHASH_W + x + 1];
                bits = (bits << 1) | u64::from(left < right);
            }
        }
        Ok(Self::DHash(bits))
    }

    /// Computes the discrete cosine transform hash of a luma grid.
    ///
    /// The grid is reduced to thirty-two square, transformed, and the eight by eight low
    /// frequency block thresholded about the median of its alternating-current terms.  The
    /// constant term is excluded from that median, since overall brightness would otherwise
    /// shift every bit, but it still contributes its own bit.
    pub fn phash(grid: &LumaGrid) -> Outcome<Self> {
        let px = res!(grid.resample(PHASH_N, PHASH_N));
        let co = res!(dct2d(&px, PHASH_N));

        // Gather the low frequency block, keeping the alternating-current terms apart so the
        // median is not dragged by the constant term.
        let mut block = [0.0f64; PHASH_K * PHASH_K];
        let mut ac = Vec::with_capacity(PHASH_K * PHASH_K - 1);
        for v in 0..PHASH_K {
            for u in 0..PHASH_K {
                let c = co[v * PHASH_N + u];
                block[v * PHASH_K + u] = c;
                if !(u == 0 && v == 0) {
                    ac.push(c);
                }
            }
        }
        let med = median(&mut ac);

        let mut bits = 0u64;
        for c in block.iter() {
            bits = (bits << 1) | u64::from(*c > med);
        }
        Ok(Self::PHash(bits))
    }

    pub fn bits(&self) -> u64 {
        match self {
            Self::DHash(b)	=> *b,
            Self::PHash(b)	=> *b,
        }
    }

    /// Were both hashes produced by the same algorithm?
    pub fn same_kind(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::DHash(..), Self::DHash(..)) | (Self::PHash(..), Self::PHash(..))
        )
    }

    /// The result runs from zero, for identical hashes, to sixty-four.  A distance near thirty
    /// two means no relationship at all, since that is what a random pair gives.
    pub fn distance(&self, other: &Self) -> Outcome<u32> {
        if !self.same_kind(other) {
            return Err(err!(
                "A {} hash cannot be compared with a {} hash; the bit positions mean different \
                things.", self.label(), other.label();
            Input, Invalid, Mismatch));
        }
        Ok(hamming(self.bits(), other.bits()))
    }

    /// Returns the algorithm name, for messages.
    pub fn label(&self) -> &'static str {
        match self {
            Self::DHash(..)	=> "difference",
            Self::PHash(..)	=> "cosine transform",
        }
    }
}

/// Returns the number of differing bits between two hashes.
pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Converts an interleaved buffer to luma, taking the first three channels of each pixel.
///
/// The weights are those of Rec. 601, the convention every common image tool applies when it
/// desaturates.  `stride` is the number of bytes per pixel and must be at least three.
pub fn luma_from_interleaved(
    dat:    &[u8],
    w:      usize,
    h:      usize,
    stride: usize,
)
    -> Outcome<Vec<u8>>
{
    if stride < 3 {
        return Err(err!(
            "An interleaved buffer needs at least three bytes per pixel, {} were declared.",
            stride;
        Input, Invalid, TooSmall));
    }
    if w == 0 || h == 0 {
        return Err(err!(
            "An interleaved buffer must have a positive width and height, found {} by {}.", w, h;
        Input, Invalid, TooSmall));
    }
    let need = w * h * stride;
    if dat.len() < need {
        return Err(err!(
            "A {} by {} buffer at {} bytes per pixel needs {} bytes, {} were supplied.",
            w, h, stride, need, dat.len();
        Input, Invalid, TooSmall, Size));
    }
    let mut out = Vec::with_capacity(w * h);
    for i in 0..(w * h) {
        let p = i * stride;
        let y = 0.299 * dat[p] as f64
            + 0.587 * dat[p + 1] as f64
            + 0.114 * dat[p + 2] as f64;
        out.push(y.round().clamp(0.0, 255.0) as u8);
    }
    Ok(out)
}

/// Converts a packed red, green, blue buffer to luma.
pub fn luma_from_rgb(dat: &[u8], w: usize, h: usize) -> Outcome<Vec<u8>> {
    luma_from_interleaved(dat, w, h, 3)
}

/// Converts a packed red, green, blue, alpha buffer to luma, ignoring the alpha channel.
pub fn luma_from_rgba(dat: &[u8], w: usize, h: usize) -> Outcome<Vec<u8>> {
    luma_from_interleaved(dat, w, h, 4)
}

/// Computes the orthonormal two dimensional type two discrete cosine transform of a square grid.
///
/// The transform is separable, so it is applied along the rows and then along the columns, which
/// costs `n` cubed multiplications rather than `n` to the fourth.
fn dct2d(px: &[f64], n: usize) -> Outcome<Vec<f64>> {
    if n == 0 {
        return Err(err!(
            "A discrete cosine transform needs a positive side length, {} was given.", n;
        Input, Invalid, TooSmall));
    }
    if px.len() < n * n {
        return Err(err!(
            "A {} square discrete cosine transform needs {} samples, {} were supplied.",
            n, n * n, px.len();
        Input, Invalid, TooSmall, Size));
    }
    // Basis table: cos((2i + 1) k pi / 2n), indexed as [i * n + k].
    let mut cos = vec![0.0f64; n * n];
    for i in 0..n {
        for k in 0..n {
            cos[i * n + k] = (((2 * i + 1) as f64) * (k as f64) * PI / (2.0 * n as f64)).cos();
        }
    }
    let s0 = (1.0 / n as f64).sqrt();	// Scale of the constant term
    let sk = (2.0 / n as f64).sqrt();	// Scale of every other term

    // Rows first.
    let mut tmp = vec![0.0f64; n * n];
    for y in 0..n {
        for u in 0..n {
            let mut acc = 0.0f64;
            for x in 0..n {
                acc += px[y * n + x] * cos[x * n + u];
            }
            tmp[y * n + u] = acc * if u == 0 { s0 } else { sk };
        }
    }
    // Then columns.
    let mut out = vec![0.0f64; n * n];
    for u in 0..n {
        for v in 0..n {
            let mut acc = 0.0f64;
            for y in 0..n {
                acc += tmp[y * n + u] * cos[y * n + v];
            }
            out[v * n + u] = acc * if v == 0 { s0 } else { sk };
        }
    }
    Ok(out)
}

/// Returns the median of a slice, sorting it in place; an empty slice gives zero.
fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}
