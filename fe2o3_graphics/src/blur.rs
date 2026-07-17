//! Blurring, and the drop shadow built on it.
//!
//! # Three boxes make a Gaussian
//!
//! A true Gaussian blur is a convolution with a kernel that never quite reaches zero, so it has to
//! be truncated before it can be used at all, and even separated into a horizontal and a vertical
//! pass it costs a multiply and an add per sample per tap. A box blur -- the plain mean of a window
//! -- costs nothing like as much, but on its own it looks like what it is: its kernel has corners,
//! and a shadow blurred with one comes out with faint square banding that the eye finds at once.
//!
//! Convolve a box with itself and those corners round off; convolve it a third time and what is
//! left is a piecewise cubic sitting within a few percent of the Gaussian it is approaching. This is
//! the central limit theorem doing the work, and three is where the returns stop: no one can tell
//! three passes from thirty, least of all in a shadow, whose whole job is to go unremarked. The
//! variance of a box of radius `r` is `((2r + 1)^2 - 1) / 12`, so [`BOX_PASSES`] of them stand in
//! for a Gaussian of sigma `sqrt(r * (r + 1))`. See [`sigma_for_radius`].
//!
//! # The window slides
//!
//! The mean of a window is not summed afresh at every pixel. The window moves one sample on, so one
//! sample enters on the right and one leaves on the left, and the running sum is corrected by two
//! arithmetic operations however wide the window is. A blur therefore costs the same whether its
//! radius is one pixel or fifty, which is what makes a large soft shadow affordable at all.
//!
//! The sum is kept in `f64` where the samples are `f32`. Sliding a window the length of a row is
//! thousands of additions and subtractions of the one accumulator, and each rounding error stays in
//! it: in `f32` they compound into a drift a screen can show, as a gradient across a field that
//! should be flat. In `f64` the same drift sits some ten orders of magnitude under one step of an
//! eight-bit channel, and the cast back to `f32` erases it.
//!
//! # The blur runs on premultiplied alpha
//!
//! [`Rgba`] carries straight alpha, so a clear pixel still carries a colour, and that colour is
//! usually black, because black is what an untouched buffer holds. Average the channels as they
//! stand and the black is averaged in at its full weight, though the pixel it came from is not
//! there to be seen. A red shape blurred against a clear background then comes out fringed in dark,
//! dirty red: the colour ramps down towards black alongside the alpha, instead of staying red and
//! merely fading. It is the classic bug of this whole area, and it is invisible in a test that only
//! looks at alpha.
//!
//! Premultiplied, a clear pixel contributes nothing to any channel, because every channel has
//! already been scaled by the alpha that says it is not there. So the blur premultiplies on the way
//! in and un-premultiplies on the way out -- the same trick, for the same reason, as [`Rgba::over`].
//!
//! # The edges
//!
//! A window centred near an edge reaches for samples that are not there. Counting them as clear
//! would fade the picture into a border it never had, darkening every edge of an image that ran to
//! its own boundary; counting them as nothing at all and dividing by fewer samples costs a branch in
//! the inner loop and still guesses. The sample at the edge is repeated instead, which is the guess
//! that a field flat at its edge stays flat past it, and so leaves a flat field exactly as it found
//! it.

use crate::{
	colour::Rgba,
	path::{
		Bounds,
		Path,
	},
	pixmap::{
		Pixmap,
		MAX_PIXELS,
	},
	transform::Transform,
};

use oxedyne_fe2o3_core::prelude::*;

/// How many box passes make one blur, along each axis. See the module docs for why three.
pub const BOX_PASSES: usize = 3;

/// A drop shadow: where a shape's silhouette falls, and how far it is softened.
///
/// The counterpart of [`crate::stroke::Stroke`], and passed the same way: the pen says what ink a
/// path leaves, and this says what shade it throws. The colour is not held here, for the same
/// reason a pen does not hold one -- it is the painting that has a colour, not the tool.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shadow {
	/// How far right the silhouette stands from the shape, in pixels. Negative throws it left.
	pub dx:	f32,
	/// How far down the silhouette stands from the shape, in pixels. Negative throws it up.
	pub dy:	f32,
	/// The blur radius, in pixels. Zero throws a hard-edged silhouette, which is a shape in its own
	/// right and not a mistake.
	pub radius:	usize,
}

impl Shadow {

	/// Creates a shadow standing `(dx, dy)` pixels from its shape and softened by a blur of
	/// `radius` pixels.
	pub fn new(dx: f32, dy: f32, radius: usize) -> Self {
		Self { dx, dy, radius }
	}

	/// How far the blur carries paint past where the silhouette stands, in pixels.
	///
	/// Each box pass spreads a sample by the radius and there are [`BOX_PASSES`] of them, so the
	/// blurred silhouette reaches this much further on every side than the sharp one, and no
	/// further: the kernel has finite support, unlike the Gaussian it stands in for. This is what
	/// the scratch buffer must be grown by, and what a caller totalling the damage of a frame needs
	/// in order to know which pixels a shadow dirtied.
	pub fn reach(&self) -> usize {
		self.radius.saturating_mul(BOX_PASSES)
	}
}

/// The standard deviation, in pixels, of the Gaussian that a blur of this radius stands in for.
///
/// A box of radius `r` averages `2r + 1` samples, whose variance is `((2r + 1)^2 - 1) / 12`;
/// [`BOX_PASSES`] of them convolved give three times that, which is `r * (r + 1)`.
pub fn sigma_for_radius(radius: usize) -> f32 {
	let r = radius as f64;
	(r * (r + 1.0)).sqrt() as f32
}

/// The blur radius, in pixels, that best stands in for a Gaussian of this standard deviation.
///
/// The inverse of [`sigma_for_radius`], rounded to the nearest whole pixel, since a sliding window
/// has no fractional width. A caller who thinks in sigmas, as anyone coming from a design tool or a
/// stylesheet does, converts here once and passes the radius thereafter. A sigma that is not a
/// positive number gives a radius of zero, which blurs nothing.
pub fn radius_for_sigma(sigma: f32) -> usize {
	if !sigma.is_finite() || sigma <= 0.0 {
		return 0;
	}
	let s = sigma as f64;
	// The positive root of `r^2 + r - s^2 = 0`.
	let r = (-1.0 + (1.0 + 4.0 * s * s).sqrt()) / 2.0;
	(r + 0.5) as usize
}

/// One box pass along a line: every sample becomes the mean of the `2r + 1` samples centred on it.
///
/// Samples off either end are the end sample repeated. See the module docs for why the window
/// slides, why the accumulator is wider than the samples, and why the edge is clamped rather than
/// counted as clear.
fn box_pass(src: &[f32], dst: &mut [f32], r: usize) {
	let n = src.len();
	if n == 0 || dst.len() < n {
		return; // A private invariant, defended: the two lines are the same length.
	}
	if r == 0 {
		dst[..n].copy_from_slice(src);
		return;
	}
	let first = src[0] as f64;
	let last = src[n - 1] as f64;
	// The window opens centred on the first sample, so it hangs off the left by `r` samples, every
	// one of them the first repeated, and off the right too where the line is shorter than the
	// radius.
	let hi = r.min(n - 1);
	let mut sum = first * (r as f64);
	for s in &src[..=hi] {
		sum += *s as f64;
	}
	sum += last * ((r - hi) as f64);
	// Saturating, so that a radius no one could mean cannot overflow its way into a panic.
	let inv = 1.0 / (r.saturating_mul(2).saturating_add(1) as f64);
	for x in 0..n {
		dst[x] = (sum * inv) as f32;
		// Slide: one sample enters on the right, one leaves on the left, both clamped to the ends.
		let add = src[x.saturating_add(r).saturating_add(1).min(n - 1)] as f64;
		let sub = src[x.saturating_sub(r)] as f64;
		sum += add - sub;
	}
}

/// Blurs one line of a plane, gathering it into a contiguous buffer, running every box pass over
/// it, and scattering it back.
///
/// The line is `n` samples beginning at `start`, each `stride` apart, which lets the one routine
/// serve a row and a column alike. Gathering is not waste: a box pass cannot be done in place,
/// since it needs the samples it is about to overwrite, so a second buffer is wanted anyway, and
/// bringing a column into one makes the passes walk contiguous memory instead of leaping a row at
/// every sample.
fn blur_line(
	p:	&mut [f32],
	start:	usize,
	stride:	usize,
	n:	usize,
	r:	usize,
	buf:	&mut Vec<f32>,
	tmp:	&mut Vec<f32>,
) {
	if n == 0 {
		return;
	}
	buf.clear();
	buf.extend((0..n).map(|i| p[start + i * stride]));
	tmp.clear();
	tmp.resize(n, 0.0);
	// Ping-pong between the two buffers. `in_buf` says which of them holds the latest samples.
	let mut in_buf = true;
	for _ in 0..BOX_PASSES {
		if in_buf {
			box_pass(buf, tmp, r);
		} else {
			box_pass(tmp, buf, r);
		}
		in_buf = !in_buf;
	}
	let out: &[f32] = if in_buf { buf } else { tmp };
	for i in 0..n {
		p[start + i * stride] = out[i];
	}
}

/// Blurs one plane of samples in place: every box pass along every row, then every box pass down
/// every column.
///
/// A blur is separable, which is the only reason it is affordable: the two-dimensional kernel is
/// the product of two one-dimensional ones, so a pass along each axis does what a full
/// two-dimensional convolution would, at a cost that grows with the radius rather than its square.
///
/// The axes are grouped -- three horizontal passes, then three vertical -- rather than interleaved
/// as `HVHVHV`. This is the same blur, not an approximation of it: a convolution along the rows and
/// a convolution down the columns act on independent axes and so commute, and convolution is
/// associative, so the two orders are the same operator. Grouping lets each line be gathered and
/// scattered once rather than once per pass, which is most of the cost of the vertical pass.
fn blur_plane(p: &mut [f32], w: usize, h: usize, r: usize) {
	let mut buf = Vec::with_capacity(w.max(h));
	let mut tmp = Vec::with_capacity(w.max(h));
	// Rows: one sample to the next is one sample along.
	for y in 0..h {
		blur_line(p, y * w, 1, w, r, &mut buf, &mut tmp);
	}
	// Columns: one sample to the next is a whole row along.
	for x in 0..w {
		blur_line(p, x, w, h, r, &mut buf, &mut tmp);
	}
}

impl Pixmap {

	/// Blurs the pixmap in place, with a blur of the given radius in pixels.
	///
	/// Three box passes along each axis stand in for a Gaussian of sigma `sqrt(r * (r + 1))`: see
	/// [`sigma_for_radius`] and the module docs. The cost is the same whatever the radius, since the
	/// window slides rather than being summed afresh at every pixel.
	///
	/// A radius of zero returns without touching a byte. That is not merely an optimisation. The
	/// blur works premultiplied, and premultiplying a clear pixel throws away the colour it was
	/// carrying, which is exactly right for a blur -- the colour of what is not there must not bleed
	/// into what is -- and exactly wrong for a no-op.
	pub fn blur(&mut self, radius: usize) {
		if radius == 0 {
			return;
		}
		let (w, h) = (self.width(), self.height());
		let n = w * h; // Already known not to overflow: see [`Pixmap::new`].
		// Four planes of premultiplied channels, planar rather than interleaved because the sliding
		// window walks one channel at a time and a stride of one is what it wants.
		let mut pl = [
			vec![0.0f32; n],
			vec![0.0f32; n],
			vec![0.0f32; n],
			vec![0.0f32; n],
		];
		for (i, px) in self.data().chunks_exact(4).enumerate() {
			let a = (px[3] as f32) / 255.0;
			pl[0][i] = (px[0] as f32) / 255.0 * a;
			pl[1][i] = (px[1] as f32) / 255.0 * a;
			pl[2][i] = (px[2] as f32) / 255.0 * a;
			pl[3][i] = a;
		}
		for p in &mut pl {
			blur_plane(p, w, h, radius);
		}
		for (i, px) in self.data_mut().chunks_exact_mut(4).enumerate() {
			let a = pl[3][i].clamp(0.0, 1.0);
			if a <= 0.0 {
				// Nothing is there, and the colour of nothing is not a colour anyone can name.
				px[0] = 0;
				px[1] = 0;
				px[2] = 0;
				px[3] = 0;
				continue;
			}
			for c in 0..3 {
				px[c] = ((pl[c][i] / a).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
			}
			px[3] = (a * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
		}
	}

	/// Throws a path's soft drop shadow onto the pixmap.
	///
	/// Only the shadow is painted, so a caller draws this first and the shape itself over the top;
	/// that way round the shape hides the silhouette standing under it, which is what a shadow
	/// looks like, and the caller stays free to paint the shape in a way this could not guess.
	///
	/// # How
	///
	/// The silhouette is filled into a scratch pixmap, blurred there, and composited back. The
	/// scratch is grown by [`Shadow::reach`] on every side, because a blur carries paint that much
	/// past the shape it came from: a scratch merely the size of the shape's own bounding box would
	/// hand back a shadow cut off square at the edges, which is precisely the shape a shadow must
	/// not have. It is trimmed to what the pixmap and the clip could show, grown by the same reach,
	/// since silhouette rasterised beyond that can reach no pixel anyone will see.
	///
	/// # Errors
	///
	/// Refuses an offset that is not finite, and a radius so large that the scratch the shadow needs
	/// is bigger than a pixmap may be.
	pub fn shadow_path(
		&mut self,
		path:	&Path,
		t:	&Transform,
		colour:	Rgba,
		clip:	Option<Bounds>,
		shadow:	&Shadow,
	)
		-> Outcome<()>
	{
		if !shadow.dx.is_finite() || !shadow.dy.is_finite() {
			return Err(err!(
				"A shadow's offset must be finite, but ({}, {}) was given.", shadow.dx, shadow.dy;
			Invalid, Input));
		}
		if colour.is_transparent() || path.is_empty() {
			return Ok(());
		}
		let bb = match path.bounds(t) {
			Some(bb) => bb,
			None => return Ok(()),
		};
		// Where the sharp silhouette stands: the shape, moved by the offset.
		let sil = Bounds::new(
			bb.x0 + shadow.dx,
			bb.y0 + shadow.dy,
			bb.x1 + shadow.dx,
			bb.y1 + shadow.dy,
		);
		let reach = shadow.reach() as f32;
		// Everywhere the blur can carry the silhouette to.
		let spread = sil.grow(reach);
		// Everywhere the caller will let it land.
		let mut want = self.bounds();
		if let Some(c) = clip {
			want = want.intersect(c);
		}
		if want.is_empty() || spread.intersect(want).is_empty() {
			return Ok(());
		}
		// The scratch holds every pixel the shadow lands on, and the reach further out that feeds
		// them, and nothing besides.
		let win = spread.intersect(want.grow(reach));
		let ix0 = win.x0.floor() as i32;
		let iy0 = win.y0.floor() as i32;
		// Widened, because a reach no one could mean would overflow the difference of two `i32`.
		let sw = (win.x1.ceil() as i32 as i64) - (ix0 as i64);
		let sh = (win.y1.ceil() as i32 as i64) - (iy0 as i64);
		if sw <= 0 || sh <= 0 {
			return Ok(());
		}
		if sw > MAX_PIXELS as i64 || sh > MAX_PIXELS as i64 {
			return Err(err!(
				"A shadow of radius {} reaches {} pixels, and needs a scratch of {} by {} pixels.",
				shadow.radius, shadow.reach(), sw, sh;
			Invalid, Input, Excessive));
		}
		let mut scratch = res!(Pixmap::new(sw as usize, sh as usize));
		// The shape, moved by the offset, in the scratch's own coordinates. The filler does the
		// rasterising, the anti-aliasing and the fill rule, exactly as it does for a stroke: a
		// shadow adds no rasteriser code either.
		let st = t.then(&Transform::translate(
			shadow.dx - (ix0 as f32),
			shadow.dy - (iy0 as f32),
		));
		res!(scratch.fill_path(path, &st, colour, None));
		scratch.blur(shadow.radius);
		self.blit(&scratch, ix0, iy0, clip);
		Ok(())
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

	/// A pixmap in which no two pixels agree, so that a blur which moved anything shows. Some of its
	/// pixels are clear but coloured, which a premultiplied round trip cannot preserve.
	fn ramp(w: usize, h: usize) -> Outcome<Pixmap> {
		let mut pm = res!(Pixmap::new(w, h));
		for y in 0..h {
			for x in 0..w {
				pm.set_pixel(x, y, Rgba::new(
					(x * 7 + 3) as u8,
					(y * 11 + 5) as u8,
					((x + y) * 13 + 7) as u8,
					((x * 3 + y * 5) % 256) as u8,
				));
			}
		}
		Ok(pm)
	}

	#[test]
	fn test_a_radius_of_zero_is_an_identity_00() -> Outcome<()> {
		// Not one byte, which is more than "looks the same": the ramp holds clear pixels that still
		// carry a colour, and a blur of no radius that went round through the planes anyway would
		// premultiply that colour away to black.
		let pm = res!(ramp(16, 16));
		let mut out = pm.clone();
		out.blur(0);
		assert_eq!(out, pm, "a blur of no radius must not touch a byte");
		Ok(())
	}

	#[test]
	fn test_a_blurred_edge_is_monotone_and_keeps_its_alpha_01() -> Outcome<()> {
		// A hard step: the left half opaque, the right half not there at all.
		let mut pm = res!(Pixmap::new(64, 8));
		for y in 0..8 {
			for x in 0..32 {
				pm.set_pixel(x, y, Rgba::WHITE);
			}
		}
		let total = |pm: &Pixmap| -> u32 {
			pm.data().chunks_exact(4).map(|p| p[3] as u32).sum()
		};
		let before = total(&pm);
		pm.blur(4);
		let after = total(&pm);
		// The blur moves alpha about; it does not make or destroy any. The step stands far enough
		// from either end that the clamped edges give back exactly what they take, and the kernel is
		// symmetric, so what leaves one side of the step arrives at the other.
		let drift = ((after as f32) - (before as f32)).abs() / (before as f32);
		assert!(drift < 0.01, "alpha ran from {} to {}", before, after);
		// Across the step, never more alpha to the right than to the left.
		for y in 0..8 {
			let mut prev = 255u8;
			for x in 0..64 {
				let c = res!(px(&pm, x, y)).a;
				assert!(c <= prev, "row {} rises at column {}: {} then {}", y, x, prev, c);
				prev = c;
			}
		}
		// And it is a ramp, not a step that stayed hard.
		let soft = (0..64)
			.filter(|x| matches!(pm.pixel(*x, 4), Some(c) if c.a > 8 && c.a < 247))
			.count();
		assert!(soft >= 8, "expected a soft edge, found {} partial columns", soft);
		Ok(())
	}

	#[test]
	fn test_a_clear_neighbour_does_not_darken_the_colour_02() -> Outcome<()> {
		// The classic bug, pinned. The left half is opaque red. The right half is not merely clear
		// but clear *and black*, which is what `Rgba::TRANSPARENT` is and what an untouched pixmap
		// holds. Blur the straight channels and that black is averaged into the red at its full
		// weight, so the edge comes out dark red at half alpha instead of the same red, half there.
		// Blurred premultiplied, the colour cannot move at all: only the alpha ramps.
		let red = Rgba::new(255, 0, 0, 255);
		let mut pm = res!(Pixmap::new(48, 4));
		for y in 0..4 {
			for x in 0..24 {
				pm.set_pixel(x, y, red);
			}
		}
		pm.blur(3);
		let mut ramped = 0;
		for y in 0..4 {
			for x in 0..48 {
				let c = res!(px(&pm, x, y));
				if c.a == 0 {
					continue; // Nothing is there, so it has no colour to have kept.
				}
				assert!(c.r >= 250, "({}, {}) lost its red: {:?}", x, y, c);
				assert_eq!((c.g, c.b), (0, 0), "({}, {}) picked up a cast: {:?}", x, y, c);
				if c.a > 8 && c.a < 247 {
					ramped += 1;
				}
			}
		}
		// Without a soft edge there would have been nowhere for a fringe to appear, and nothing above
		// would have been tested.
		assert!(ramped >= 16, "expected a soft edge to test, found {} partial pixels", ramped);
		Ok(())
	}

	#[test]
	fn test_a_shadow_is_soft_and_falls_where_it_is_thrown_03() -> Outcome<()> {
		let mut pm = res!(Pixmap::filled(64, 64, Rgba::WHITE));
		let card = res!(Path::round_rect(Bounds::new(16.0, 16.0, 48.0, 48.0), 8.0));
		let sh = Shadow::new(4.0, 4.0, 3);
		res!(pm.shadow_path(&card, &Transform::IDENTITY, Rgba::BLACK, None, &sh));
		// Thrown down and to the right, so the card no longer stands in the middle of its own shade.
		// Six pixels past its right edge is shadow; six past its left edge, the same distance out,
		// is nothing. Both lie outside the silhouette's own bounding box, so a scratch that had not
		// been grown by the reach would have cut the shadow off square and left the right one white.
		let right = res!(px(&pm, 54, 32));
		let left = res!(px(&pm, 10, 32));
		assert_eq!(left, Rgba::WHITE, "no shade should reach back up and to the left");
		assert!(right.r < 240, "the shadow should fall past the right edge, found {:?}", right);
		assert!(right.r > 0, "and fade rather than go black, found {:?}", right);
		// The corner is a gradient rather than two flat regions with a step between them.
		let soft = (44..58)
			.filter_map(|k| pm.pixel(k, k))
			.filter(|c| c.r > 8 && c.r < 247)
			.count();
		assert!(soft >= 5, "expected a soft corner, found {} partial pixels", soft);
		// The shade deepens inwards along that corner, all the way and never back.
		let mut prev = 0u8;
		for k in 44..58 {
			let c = res!(px(&pm, k, k)).r;
			assert!(c >= prev, "the corner darkens outwards at {}: {} then {}", k, prev, c);
			prev = c;
		}
		Ok(())
	}

	#[test]
	fn test_a_uniform_field_survives_the_blur_04() -> Outcome<()> {
		// Every sample a window can reach is the same one, including the samples it reaches off the
		// edge for, because those are the edge repeated. So every mean is that same sample and the
		// field must come back exactly as it went in, with no border that has faded into a darkness
		// that was never there. The radius is wider than the pixmap, so almost every window is mostly
		// made of clamped samples and the clamp is what is being tested.
		let c = Rgba::new(10, 200, 30, 128);
		let mut pm = res!(Pixmap::filled(12, 9, c));
		pm.blur(5);
		for y in 0..9 {
			for x in 0..12 {
				assert_eq!(res!(px(&pm, x, y)), c, "the pixel ({}, {}) moved", x, y);
			}
		}
		Ok(())
	}

	#[test]
	fn test_the_radius_and_the_sigma_agree_05() -> Outcome<()> {
		for r in 0..64 {
			let s = sigma_for_radius(r);
			let back = radius_for_sigma(s);
			assert_eq!(back, r, "the radius {} came back as {} through the sigma {}", r, back, s);
		}
		// A sigma that names no blur blurs nothing, rather than reaching for the root of a negative.
		assert_eq!(radius_for_sigma(0.0), 0);
		assert_eq!(radius_for_sigma(-1.0), 0);
		assert_eq!(radius_for_sigma(f32::NAN), 0);
		assert_eq!(sigma_for_radius(0), 0.0);
		Ok(())
	}

	#[test]
	fn test_a_shadow_refuses_what_it_cannot_throw_06() -> Outcome<()> {
		let mut pm = res!(Pixmap::filled(16, 16, Rgba::WHITE));
		let card = res!(Path::rect(Bounds::new(4.0, 4.0, 12.0, 12.0)));
		let bad = Shadow::new(f32::NAN, 0.0, 2);
		assert!(pm.shadow_path(&card, &Transform::IDENTITY, Rgba::BLACK, None, &bad).is_err());
		// A clear shadow and an empty path both paint nothing, and neither is an error.
		let sh = Shadow::new(1.0, 1.0, 2);
		let before = pm.clone();
		res!(pm.shadow_path(&card, &Transform::IDENTITY, Rgba::TRANSPARENT, None, &sh));
		res!(pm.shadow_path(&Path::default(), &Transform::IDENTITY, Rgba::BLACK, None, &sh));
		assert_eq!(pm, before);
		Ok(())
	}
}
