//! A colour transform between two profiles, built once and keyed by what it was built from.
//!
//! A [`Transform`] takes a colour of a source space to XYZ (D50), compensates for the difference
//! between the two media in XYZ, and takes the result to the destination space: through the output
//! lookup table of a CMYK profile, by way of Lab, or through the inverse grey curve of a grey one.
//! It follows LittleCMS 2: the intent picks the table (`A2B0` or `B2A0` for perceptual, `1` for
//! relative colorimetric and for absolute, `2` for saturation, the first table standing in for one
//! the profile lacks), absolute colorimetric scales XYZ by the ratio of the two media whites, and
//! the other intents scale it between the two black points when black-point compensation is on.
//! Compensation is forced on for perceptual and saturation when the destination is a version 4
//! profile, as the built-in spaces are, and is never applied to absolute colorimetric.
//!
//! The transform has an identity, [`Transform::id`], a function of the two profiles' identities, the
//! intent and the compensation, so that a cache can hold one transform for each distinct request.
//!
//! # What is refused
//!
//! An RGB destination, a CMYK profile whose connection space is XYZ, and a CMYK profile without the
//! table the transform needs.
//!
//! # Images
//!
//! [`crate::icc::grid::Grid`] samples an RGB source on a 33-step lattice, so that a bitmap is
//! converted by interpolation rather than by the full pipeline for each pixel. A CMYK destination's
//! output table is a lattice of its own, with creases where the inks turn, and no coarser lattice
//! follows them, so the grid holds the colour as Lab and the table is read exactly.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::icc::{
	builtin,
	grid::Grid,
	lab::{
		self,
		D50,
	},
	matrix::{
		Grey,
		RgbMatrix,
	},
	read::{
		Class,
		Lut,
		Profile,
		Space,
		fnv64,
	},
};

use oxedyne_fe2o3_core::prelude::*;

use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
	Perceptual,
	Relative,
	Saturation,
	Absolute,
}

impl Intent {
	/// The index of the `A2B` or `B2A` table this intent reads.
	pub fn tag(self) -> usize {
		match self {
			Self::Perceptual	=> 0,
			Self::Relative		=> 1,
			Self::Saturation	=> 2,
			Self::Absolute		=> 1,
		}
	}

	fn code(self) -> u8 {
		match self {
			Self::Perceptual	=> 0,
			Self::Relative		=> 1,
			Self::Saturation	=> 2,
			Self::Absolute		=> 3,
		}
	}
}

// What a device colour is made of.
#[derive(Clone, Debug)]
pub enum Kind {
	Rgb(RgbMatrix),
	Grey(Grey),
	Cmyk(Box<Profile>),
}

/// A colour space ready to be one end of a transform.
#[derive(Clone, Debug)]
pub struct Dev {
	pub kind:	Kind,
	pub id:		u64,
	pub v4:		bool,			// version 4 or built in, so perceptual compensates
	pub white:	[f64; 3],		// the media white, D50 where the profile gives none
	pub name:	String,
}

impl Dev {
	/// The built-in sRGB space.
	pub fn srgb() -> Outcome<Self> {
		Ok(Self {
			kind:	Kind::Rgb(res!(builtin::srgb())),
			id:		fnv64(b"oxedyne_fe2o3_graphics::icc::builtin::srgb"),
			v4:		true,
			white:	D50,
			name:	"sRGB (built in)".to_string(),
		})
	}

	/// The built-in sGray space.
	pub fn sgray() -> Self {
		Self {
			kind:	Kind::Grey(builtin::sgray()),
			id:		fnv64(b"oxedyne_fe2o3_graphics::icc::builtin::sgray"),
			v4:		true,
			white:	D50,
			name:	"sGray (built in)".to_string(),
		}
	}

	/// The space of a profile: its matrix path, its grey curve, or its tables.
	pub fn from_profile(p: &Profile) -> Outcome<Self> {
		let kind = match p.head.space {
			Space::Rgb	=> Kind::Rgb(res!(RgbMatrix::read(p))),
			Space::Gray	=> Kind::Grey(res!(Grey::read(p))),
			Space::Cmyk	=> {
				if p.head.pcs != Space::Lab {
					return Err(err!(
						"Profile '{}' is a CMYK profile with a {:?} connection space, and the \
						table path here is of a Lab one.", p.name, p.head.pcs;
						Invalid, Input, Mismatch));
				}
				Kind::Cmyk(Box::new(p.clone()))
			},
			other	=> return Err(err!(
				"Profile '{}' is a {:?} profile, and a transform is made of RGB, grey and CMYK ones.",
				p.name, other; Invalid, Input, Mismatch)),
		};
		let v2_display = p.head.major < 4 && p.head.class == Class::Display;
		Ok(Self {
			kind,
			id:		p.id,
			v4:		p.head.major >= 4,
			white:	match p.wtpt {
				Some(w) if !v2_display	=> w,
				_						=> D50,
			},
			name:	p.name.clone(),
		})
	}

	// The XYZ of the darkest colour the space can show, as a source.
	fn black_src(&self, intent: Intent) -> [f64; 3] {
		match &self.kind {
			Kind::Rgb(m)	=> lab::dark_colorant(m.xyz([0.0; 3])),
			Kind::Grey(g)	=> {
				let y = g.y(0.0);
				lab::dark_colorant([D50[0] * y, D50[1] * y, D50[2] * y])
			},
			Kind::Cmyk(p)	=> lab::cmyk_black_src(p, intent),
		}
	}

	// The same as a destination, where a CMYK profile's ink limits show.
	fn black_dst(&self, intent: Intent) -> [f64; 3] {
		match &self.kind {
			Kind::Cmyk(p)	=> lab::cmyk_black_dst(p, intent),
			_				=> self.black_src(intent),
		}
	}

	fn channels(&self) -> usize {
		match &self.kind {
			Kind::Rgb(_)	=> 3,
			Kind::Grey(_)	=> 1,
			Kind::Cmyk(_)	=> 4,
		}
	}
}

/// A conversion from one space to another, at one intent and one setting of compensation.
#[derive(Debug)]
pub struct Transform {
	src:	Dev,
	dst:	Dev,
	intent:	Intent,
	bpc:	bool,				// as applied, after the forcing and the exemption
	scale:	[f64; 3],			// XYZ is scaled by these, then offset
	off:	[f64; 3],
	id:		u64,
	grid:	OnceLock<Grid>,
}

impl Transform {
	/// Builds the transform, refusing a pair of spaces it cannot join.
	pub fn new(src: &Dev, dst: &Dev, intent: Intent, bpc: bool) -> Outcome<Self> {
		if let Kind::Rgb(_) = dst.kind {
			return Err(err!(
				"A transform to the RGB space '{}' is not made: the destination is a grey or CMYK one.",
				dst.name; Invalid, Input, Unimplemented));
		}
		if let Kind::Cmyk(p) = &src.kind {
			if lab::pick(&p.a2b, intent.tag()).is_none() {
				return Err(err!(
					"Profile '{}' has no 'A2B{}' or 'A2B0' table to take its CMYK to Lab.",
					p.name, intent.tag(); Invalid, Input, Missing));
			}
		}
		if let Kind::Cmyk(p) = &dst.kind {
			if lab::pick(&p.b2a, intent.tag()).is_none() {
				return Err(err!(
					"Profile '{}' has no 'B2A{}' or 'B2A0' table to take Lab to its CMYK.",
					p.name, intent.tag(); Invalid, Input, Missing));
			}
		}
		let bpc = intent != Intent::Absolute
			&& (bpc || (dst.v4 && matches!(intent, Intent::Perceptual | Intent::Saturation)));
		let mut scale = [1.0; 3];
		let mut off = [0.0; 3];
		if intent == Intent::Absolute {
			for i in 0..3 {
				scale[i] = src.white[i] / dst.white[i];
			}
		} else if bpc {
			let (bi, bo) = (src.black_src(intent), dst.black_dst(intent));
			if bi != bo {
				for i in 0..3 {
					let t = bi[i] - D50[i];
					scale[i] = (bo[i] - D50[i]) / t;
					off[i] = -D50[i] * (bo[i] - bi[i]) / t;
				}
			}
		}
		let mut key = Vec::with_capacity(18);
		key.extend_from_slice(&src.id.to_be_bytes());
		key.extend_from_slice(&dst.id.to_be_bytes());
		key.push(intent.code());
		key.push(bpc as u8);
		Ok(Self {
			src:	src.clone(),
			dst:	dst.clone(),
			intent,
			bpc,
			scale,
			off,
			id:		fnv64(&key),
			grid:	OnceLock::new(),
		})
	}

	/// The identity a cache holds the transform by: of the two profiles, the intent and the compensation.
	pub fn id(&self) -> u64 {
		self.id
	}

	pub fn intent(&self) -> Intent {
		self.intent
	}

	/// Is black-point compensation applied, whether asked for or forced?
	pub fn bpc(&self) -> bool {
		self.bpc
	}

	pub fn src(&self) -> &Dev {
		&self.src
	}

	pub fn dst(&self) -> &Dev {
		&self.dst
	}

	/// The destination colour of the source colour `v`, whose first channels are read; all as
	/// fractions of full scale, and only the destination's channels meaningful.
	pub fn eval(&self, v: [f64; 4]) -> [f64; 4] {
		let x = self.pcs(v);
		match &self.dst.kind {
			Kind::Cmyk(_)	=> match self.tail() {
				Some(b)	=> {
					let e = lab::lab_enc(b.bits, lab::xyz_to_lab(x));
					b.eval([e[0], e[1], e[2], 0.0])
				},
				None	=> [0.0; 4],	// refused by new
			},
			Kind::Grey(g)	=> [g.level(x[1]), 0.0, 0.0, 0.0],
			Kind::Rgb(_)	=> [0.0; 4],	// refused by new
		}
	}

	// The source colour `v` as XYZ (D50), scaled and offset for the intent and the compensation.
	fn pcs(&self, v: [f64; 4]) -> [f64; 3] {
		let xyz = match &self.src.kind {
			Kind::Rgb(m)	=> m.xyz([v[0], v[1], v[2]]),
			Kind::Grey(g)	=> {
				let y = g.y(v[0]);
				[D50[0] * y, D50[1] * y, D50[2] * y]
			},
			Kind::Cmyk(p)	=> match lab::pick(&p.a2b, self.intent.tag()) {
				Some(a)	=> {
					let g = a.eval(v);
					lab::lab_to_xyz(lab::lab_dec(a.bits, [g[0], g[1], g[2]]))
				},
				None	=> [0.0; 3],	// refused by new
			},
		};
		let mut x = [0.0; 3];
		for i in 0..3 {
			x[i] = xyz[i] * self.scale[i] + self.off[i];
		}
		x
	}

	// The table that takes Lab to the inks, where the destination is CMYK.
	pub(crate) fn tail(&self) -> Option<&Lut> {
		match &self.dst.kind {
			Kind::Cmyk(p)	=> lab::pick(&p.b2a, self.intent.tag()),
			_				=> None,
		}
	}

	// What the image grid holds for the source colour `v`: the colour in the table's Lab encoding
	// where the destination has a table, which the grid then reads exactly, else the result.
	pub(crate) fn node(&self, v: [f64; 4]) -> [f64; 4] {
		match self.tail() {
			Some(b)	=> {
				let e = lab::lab_enc(b.bits, lab::xyz_to_lab(self.pcs(v)));
				[e[0], e[1], e[2], 0.0]
			},
			None	=> self.eval(v),
		}
	}

	fn shape(&self, nin: usize, nout: usize) -> Outcome<()> {
		let (a, b) = (self.src.channels(), self.dst.channels());
		if a != nin || b != nout {
			return Err(err!(
				"The transform from '{}' ({} channels) to '{}' ({} channels) was asked to convert \
				{} channels to {}.", self.src.name, a, self.dst.name, b, nin, nout;
				Invalid, Input, Mismatch));
		}
		Ok(())
	}

	/// RGB to CMYK, as `f32` in 0..=1.
	pub fn rgb_to_cmyk(&self, rgb: [f64; 3]) -> Outcome<[f32; 4]> {
		res!(self.shape(3, 4));
		Ok(to_f32(self.eval([rgb[0], rgb[1], rgb[2], 0.0])))
	}

	/// A grey level to CMYK through the profiles, which makes a rich black; `grey_to_k` makes K alone.
	pub fn grey_to_cmyk(&self, g: f64) -> Outcome<[f32; 4]> {
		res!(self.shape(1, 4));
		Ok(to_f32(self.eval([g, 0.0, 0.0, 0.0])))
	}

	/// CMYK to a grey level in 0..=1.
	pub fn cmyk_to_grey(&self, cmyk: [f64; 4]) -> Outcome<f32> {
		res!(self.shape(4, 1));
		Ok(self.eval(cmyk)[0] as f32)
	}

	/// RGB to a grey level in 0..=1.
	pub fn rgb_to_grey(&self, rgb: [f64; 3]) -> Outcome<f32> {
		res!(self.shape(3, 1));
		Ok(self.eval([rgb[0], rgb[1], rgb[2], 0.0])[0] as f32)
	}

	/// The image grid of an RGB source, built on first use.
	pub fn grid(&self) -> Outcome<&Grid> {
		res!(self.shape(3, self.dst.channels()));
		Ok(self.grid.get_or_init(|| Grid::build(self)))
	}
}

fn to_f32(v: [f64; 4]) -> [f32; 4] {
	[v[0] as f32, v[1] as f32, v[2] as f32, v[3] as f32]
}

/// A grey level as K alone, the black of a page whose greys are printed with the black ink only.
pub fn grey_to_k(g: f64) -> [f32; 4] {
	[0.0, 0.0, 0.0, (1.0 - g.clamp(0.0, 1.0)) as f32]
}
