//! A buffer of pixels, and the painting done onto it.

use crate::{
	colour::Rgba,
	path::{
		Bounds,
		Path,
		Pt,
		TOLERANCE,
	},
	png,
	raster::{
		FillRule,
		Raster,
	},
	stroke::Stroke,
	transform::Transform,
};

use oxedyne_fe2o3_core::prelude::*;

use std::path::Path as FilePath;

/// The most pixels a pixmap may hold, a ceiling against a size that is a mistake or an attack.
/// A 16k by 16k image sits just under it.
pub const MAX_PIXELS: usize = 1 << 28;

/// A rectangular buffer of RGBA pixels, eight bits per channel, with straight alpha.
///
/// The layout is row-major, four bytes per pixel, which is what a PNG wants and what a GPU or a
/// window surface will take without a further copy.
#[derive(Clone, Debug, PartialEq)]
pub struct Pixmap {
	/// Width in pixels.
	w:	usize,
	/// Height in pixels.
	h:	usize,
	/// RGBA bytes, `w * h * 4` of them.
	data:	Vec<u8>,
}

impl Pixmap {

	/// Creates a transparent pixmap of the given size.
	pub fn new(w: usize, h: usize) -> Outcome<Self> {
		if w == 0 || h == 0 {
			return Err(err!(
				"A pixmap must have a positive size, but {} by {} was asked for.", w, h;
			Invalid, Input));
		}
		let n = match w.checked_mul(h) {
			Some(n) => n,
			None => return Err(err!(
				"A pixmap of {} by {} pixels overflows a count of pixels.", w, h;
			Invalid, Input, Overflow)),
		};
		if n > MAX_PIXELS {
			return Err(err!(
				"A pixmap of {} by {} pixels holds {} pixels, over the ceiling of {}.",
				w, h, n, MAX_PIXELS;
			Invalid, Input, Excessive));
		}
		Ok(Self {
			w,
			h,
			data: vec![0; n * 4],
		})
	}

	/// Creates a pixmap of the given size, filled with a colour.
	pub fn filled(w: usize, h: usize, colour: Rgba) -> Outcome<Self> {
		let mut pm = res!(Self::new(w, h));
		pm.fill(colour);
		Ok(pm)
	}

	/// The width in pixels.
	pub fn width(&self) -> usize {
		self.w
	}

	/// The height in pixels.
	pub fn height(&self) -> usize {
		self.h
	}

	/// The raw RGBA bytes.
	pub fn data(&self) -> &[u8] {
		&self.data
	}

	/// The raw RGBA bytes, mutably.
	pub fn data_mut(&mut self) -> &mut [u8] {
		&mut self.data
	}

	/// Consumes the pixmap, yielding its bytes.
	pub fn into_data(self) -> Vec<u8> {
		self.data
	}

	/// The whole pixmap as a bounding box.
	pub fn bounds(&self) -> Bounds {
		Bounds { x0: 0.0, y0: 0.0, x1: self.w as f32, y1: self.h as f32 }
	}

	/// Replaces every pixel with a colour.
	pub fn fill(&mut self, colour: Rgba) {
		for px in self.data.chunks_exact_mut(4) {
			px[0] = colour.r;
			px[1] = colour.g;
			px[2] = colour.b;
			px[3] = colour.a;
		}
	}

	/// The colour at a pixel, or `None` if the coordinates fall outside.
	pub fn pixel(&self, x: usize, y: usize) -> Option<Rgba> {
		if x >= self.w || y >= self.h {
			return None;
		}
		let i = (y * self.w + x) * 4;
		Some(Rgba::new(self.data[i], self.data[i + 1], self.data[i + 2], self.data[i + 3]))
	}

	/// Replaces the colour at a pixel, ignoring coordinates that fall outside.
	pub fn set_pixel(&mut self, x: usize, y: usize, colour: Rgba) {
		if x >= self.w || y >= self.h {
			return;
		}
		let i = (y * self.w + x) * 4;
		self.data[i] = colour.r;
		self.data[i + 1] = colour.g;
		self.data[i + 2] = colour.b;
		self.data[i + 3] = colour.a;
	}

	/// Composites a colour over the pixel already there, ignoring coordinates outside.
	pub fn blend_pixel(&mut self, x: usize, y: usize, src: Rgba) {
		if src.is_transparent() || x >= self.w || y >= self.h {
			return;
		}
		let i = (y * self.w + x) * 4;
		let dst = Rgba::new(self.data[i], self.data[i + 1], self.data[i + 2], self.data[i + 3]);
		let out = src.over(dst);
		self.data[i] = out.r;
		self.data[i + 1] = out.g;
		self.data[i + 2] = out.b;
		self.data[i + 3] = out.a;
	}

	/// Fills a path with a colour, anti-aliased, under the non-zero winding rule.
	///
	/// The clip, if given, is taken at pixel granularity, so it is expected to fall on pixel
	/// boundaries; layout produces such rectangles, and a fractional clip edge rounds outwards to
	/// the pixel that contains it.
	pub fn fill_path(
		&mut self,
		path:	&Path,
		t:	&Transform,
		colour:	Rgba,
		clip:	Option<Bounds>,
	)
		-> Outcome<()>
	{
		self.fill_path_with(path, t, colour, clip, FillRule::NonZero)
	}

	/// Fills a path with a colour, anti-aliased, under a fill rule.
	///
	/// Non-zero is what a glyph outline or a box wants, and is what [`Pixmap::fill_path`] takes.
	/// Even-odd is for a shape whose overlaps are meant to read as holes: see [`FillRule`].
	pub fn fill_path_with(
		&mut self,
		path:	&Path,
		t:	&Transform,
		colour:	Rgba,
		clip:	Option<Bounds>,
		rule:	FillRule,
	)
		-> Outcome<()>
	{
		if colour.is_transparent() || path.is_empty() {
			return Ok(());
		}
		let bb = match path.bounds(t) {
			Some(bb) => bb,
			None => return Ok(()),
		};
		let mut win = bb.intersect(self.bounds());
		if let Some(c) = clip {
			win = win.intersect(c);
		}
		if win.is_empty() {
			return Ok(());
		}
		// The window in whole pixels: any pixel the shape touches at all.
		let ix0 = win.x0.floor().max(0.0) as usize;
		let iy0 = win.y0.floor().max(0.0) as usize;
		let ix1 = (win.x1.ceil() as usize).min(self.w);
		let iy1 = (win.y1.ceil() as usize).min(self.h);
		if ix1 <= ix0 || iy1 <= iy0 {
			return Ok(());
		}
		let (ww, wh) = (ix1 - ix0, iy1 - iy0);

		let mut r = Raster::new(ww, wh);
		let (ox, oy) = (ix0 as f32, iy0 as f32);
		for contour in path.flatten(t, TOLERANCE) {
			let local: Vec<Pt> = contour
				.into_iter()
				.map(|p| Pt::new(p.x - ox, p.y - oy))
				.collect();
			r.add_contour(&local);
		}
		let cov = r.coverage_with(rule);

		for wy in 0..wh {
			for wx in 0..ww {
				let c = cov[wy * ww + wx];
				if c > 0.0 {
					self.blend_pixel(ix0 + wx, iy0 + wy, colour.with_coverage(c));
				}
			}
		}
		Ok(())
	}

	/// Strokes a path with a pen and fills the ink it leaves.
	///
	/// The pen's width is in the path's own coordinates, so the transform scales the line along
	/// with the shape, which is what a caller drawing the same diagram at two sizes wants. The pen's
	/// tolerance is taken in pixels and divided by the transform's scale, as [`Path::flatten`] does
	/// with its own, so that a shape enlarged tenfold is stroked ten times more finely rather than
	/// coming out faceted.
	pub fn stroke_path(
		&mut self,
		path:	&Path,
		t:	&Transform,
		colour:	Rgba,
		clip:	Option<Bounds>,
		pen:	&Stroke,
	)
		-> Outcome<()>
	{
		let mut pen = pen.clone();
		pen.tol = (pen.tol / t.scale_factor().max(f32::EPSILON)).max(f32::EPSILON);
		let outline = res!(path.stroke(&pen));
		// Non-zero, always: the outline is a union of overlapping pieces. See [`crate::stroke`].
		self.fill_path(&outline, t, colour, clip)
	}

	/// Fills an axis-aligned rectangle with a colour, anti-aliased at fractional edges.
	pub fn fill_bounds(&mut self, b: Bounds, colour: Rgba, clip: Option<Bounds>) -> Outcome<()> {
		if b.is_empty() {
			return Ok(());
		}
		let path = res!(Path::rect(b));
		self.fill_path(&path, &Transform::IDENTITY, colour, clip)
	}

	/// Composites another pixmap over this one, with its top-left corner at `(x, y)`.
	pub fn blit(&mut self, src: &Pixmap, x: i32, y: i32, clip: Option<Bounds>) {
		for sy in 0..src.h {
			for sx in 0..src.w {
				let dx = x + (sx as i32);
				let dy = y + (sy as i32);
				if dx < 0 || dy < 0 {
					continue;
				}
				let (dx, dy) = (dx as usize, dy as usize);
				if let Some(c) = clip {
					let (fx, fy) = ((dx as f32) + 0.5, (dy as f32) + 0.5);
					if fx < c.x0 || fx >= c.x1 || fy < c.y0 || fy >= c.y1 {
						continue;
					}
				}
				if let Some(s) = src.pixel(sx, sy) {
					self.blend_pixel(dx, dy, s);
				}
			}
		}
	}

	/// Encodes the pixmap as a PNG.
	pub fn to_png(&self) -> Outcome<Vec<u8>> {
		png::encode(self)
	}

	/// Decodes a PNG into a pixmap.
	pub fn from_png(buf: &[u8]) -> Outcome<Self> {
		png::decode(buf)
	}

	/// Writes the pixmap to a file as a PNG.
	pub fn save_png<P: AsRef<FilePath>>(&self, path: P) -> Outcome<()> {
		let buf = res!(self.to_png());
		res!(std::fs::write(path.as_ref(), &buf));
		Ok(())
	}

	/// Reads a PNG file into a pixmap.
	pub fn load_png<P: AsRef<FilePath>>(path: P) -> Outcome<Self> {
		let buf = res!(std::fs::read(path.as_ref()));
		Self::from_png(&buf)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The colour at a pixel a test asserts is in range.
	fn px(pm: &Pixmap, x: usize, y: usize) -> Outcome<Rgba> {
		match pm.pixel(x, y) {
			Some(c) => Ok(c),
			None => Err(err!(
				"The pixel ({}, {}) lies outside a pixmap of {} by {}.",
				x, y, pm.width(), pm.height();
			Invalid, Input, Range)),
		}
	}

	#[test]
	fn test_a_zero_sized_pixmap_is_refused_00() {
		assert!(Pixmap::new(0, 10).is_err());
		assert!(Pixmap::new(10, 0).is_err());
	}

	#[test]
	fn test_an_absurd_pixmap_is_refused_01() {
		assert!(Pixmap::new(1 << 20, 1 << 20).is_err());
	}

	#[test]
	fn test_fill_sets_every_pixel_02() -> Outcome<()> {
		let mut pm = res!(Pixmap::new(4, 4));
		pm.fill(Rgba::WHITE);
		for y in 0..4 {
			for x in 0..4 {
				assert_eq!(res!(px(&pm, x, y)), Rgba::WHITE);
			}
		}
		Ok(())
	}

	#[test]
	fn test_a_filled_rect_lands_where_it_should_03() -> Outcome<()> {
		let mut pm = res!(Pixmap::filled(10, 10, Rgba::WHITE));
		res!(pm.fill_bounds(Bounds::new(2.0, 2.0, 8.0, 8.0), Rgba::BLACK, None));
		assert_eq!(res!(px(&pm, 5, 5)), Rgba::BLACK, "inside");
		assert_eq!(res!(px(&pm, 0, 0)), Rgba::WHITE, "outside");
		assert_eq!(res!(px(&pm, 1, 1)), Rgba::WHITE, "just outside");
		assert_eq!(res!(px(&pm, 2, 2)), Rgba::BLACK, "just inside");
		Ok(())
	}

	#[test]
	fn test_a_clip_holds_paint_back_04() -> Outcome<()> {
		let mut pm = res!(Pixmap::filled(10, 10, Rgba::WHITE));
		let clip = Bounds::new(0.0, 0.0, 5.0, 10.0);
		res!(pm.fill_bounds(Bounds::new(0.0, 0.0, 10.0, 10.0), Rgba::BLACK, Some(clip)));
		assert_eq!(res!(px(&pm, 4, 5)), Rgba::BLACK, "inside the clip");
		assert_eq!(res!(px(&pm, 6, 5)), Rgba::WHITE, "outside the clip");
		Ok(())
	}

	#[test]
	fn test_a_half_pixel_edge_is_soft_05() -> Outcome<()> {
		let mut pm = res!(Pixmap::filled(4, 4, Rgba::WHITE));
		res!(pm.fill_bounds(Bounds::new(0.0, 0.0, 0.5, 4.0), Rgba::BLACK, None));
		let p = res!(px(&pm, 0, 0));
		assert!(p.r > 100 && p.r < 160, "expected a half-covered grey, found {}", p.r);
		Ok(())
	}

	#[test]
	fn test_transparent_paint_changes_nothing_06() -> Outcome<()> {
		let mut pm = res!(Pixmap::filled(4, 4, Rgba::WHITE));
		let before = pm.clone();
		res!(pm.fill_bounds(Bounds::new(0.0, 0.0, 4.0, 4.0), Rgba::TRANSPARENT, None));
		assert_eq!(pm, before);
		Ok(())
	}

	#[test]
	fn test_blit_composites_and_clips_07() -> Outcome<()> {
		let mut dst = res!(Pixmap::filled(8, 8, Rgba::WHITE));
		let src = res!(Pixmap::filled(4, 4, Rgba::BLACK));
		dst.blit(&src, 6, 6, None); // Half of it hangs off the edge.
		assert_eq!(res!(px(&dst, 7, 7)), Rgba::BLACK);
		assert_eq!(res!(px(&dst, 5, 5)), Rgba::WHITE);
		assert_eq!(dst.width(), 8, "the destination must not have grown");
		Ok(())
	}
}
