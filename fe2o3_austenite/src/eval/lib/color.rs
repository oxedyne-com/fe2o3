// U3 owns this file. Colour constructors and methods, gradients, colour maps and the named colours (`red`,
// `luma`...). `Color::to_rgba` is the one conversion flow and the emitters use.
//
// Arithmetic is in `f32`, as Typst's (palette's) is, so a component read back with `components()`
// carries the same rounding. sRGB, linear sRGB, luma, Oklab/Oklch, HSL and HSV convert by formula
// (Oklab by Ottosson's matrices, luminance by the sRGB primaries). CMYK to RGB is an ICC transform in
// Typst; it is reproduced here from a 17-point-per-axis grid sampled from the `typst` 0.15.1 oracle
// (`cmyk_srgb.bin`) with quadrilinear interpolation, within two eight-bit steps of Typst everywhere the
// oracle was probed. RGB to CMYK is Typst's naive formula; luma to CMYK its fixed ink ratios.

use crate::eval::args::Args;
use crate::eval::func::{
	Func,
	NativeFunc,
};
use crate::eval::lib::foundations::{
	finish,
	format_float,
	mismatch,
	need,
	no_method,
	receiver,
	repr_angle,
	repr_ratio,
	repr_str,
};
use crate::eval::scope::Scope;
use crate::eval::value::{
	Angle,
	Color,
	ColorSpace,
	Direction,
	Gradient,
	GradientKind,
	Module,
	Ratio,
	RelativeTo,
	Type,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

native_fns! {
	pub enum ColorFn {
		Rgb					=> "rgb",
		Luma				=> "luma",
		Cmyk				=> "cmyk",
		Oklab				=> "oklab",
		Oklch				=> "oklch",
		LinearRgb			=> "linear-rgb",
		Hsl					=> "hsl",
		Hsv					=> "hsv",
		Mix					=> "mix",
		Lighten				=> "lighten",
		Darken				=> "darken",
		Saturate			=> "saturate",
		Desaturate			=> "desaturate",
		Negate				=> "negate",
		Rotate				=> "rotate",
		Components			=> "components",
		Space				=> "space",
		ToHex				=> "to-hex",
		Transparentize		=> "transparentize",
		Opacify				=> "opacify",
		GradientLinear		=> "linear",
		GradientRadial		=> "radial",
		GradientConic		=> "conic",
		GradientSample		=> "sample",
		GradientSamples		=> "samples",
		GradientStops		=> "stops",
		GradientSharp		=> "sharp",
		GradientRepeat		=> "repeat",
		GradientKindOf		=> "kind",
		GradientSpace		=> "space",
		GradientRelative	=> "relative",
		GradientAngle		=> "angle",
		GradientCenter		=> "center",
		GradientRadius		=> "radius",
		GradientFocalCenter	=> "focal-center",
		GradientFocalRadius	=> "focal-radius",
	}
}

mod data {
	include!("color_data.rs");
}

static CMYK_GRID: &[u8] = include_bytes!("cmyk_srgb.bin");
const GRID: usize = 17;

// Typst's own literals, not eight-bit values over 255: `red`'s green is 0.254902, not 65/255, and a mix
// or conversion rounds on that difference.
const NAMED: [(&str, ColorSpace, [f32; 3]); 18] = [
	("black",	ColorSpace::Luma,	[0.0, 0.0, 0.0]),
	("gray",	ColorSpace::Luma,	[0.6666666, 0.0, 0.0]),
	("silver",	ColorSpace::Luma,	[0.8666667, 0.0, 0.0]),
	("white",	ColorSpace::Luma,	[1.0, 0.0, 0.0]),
	("navy",	ColorSpace::Rgb,	[0.0, 0.121569, 0.247059]),
	("blue",	ColorSpace::Rgb,	[0.0, 0.454902, 0.85098]),
	("aqua",	ColorSpace::Rgb,	[0.4980392, 0.858823, 1.0]),
	("teal",	ColorSpace::Rgb,	[0.223529, 0.8, 0.8]),
	("eastern",	ColorSpace::Rgb,	[0.13725, 0.615686, 0.678431]),
	("purple",	ColorSpace::Rgb,	[0.694118, 0.050980, 0.788235]),
	("fuchsia",	ColorSpace::Rgb,	[0.941177, 0.070588, 0.745098]),
	("maroon",	ColorSpace::Rgb,	[0.521569, 0.078431, 0.294118]),
	("red",		ColorSpace::Rgb,	[1.0, 0.254902, 0.211765]),
	("orange",	ColorSpace::Rgb,	[1.0, 0.521569, 0.105882]),
	("yellow",	ColorSpace::Rgb,	[1.0, 0.8627451, 0.0]),
	("olive",	ColorSpace::Rgb,	[0.239216, 0.6, 0.4392157]),
	("green",	ColorSpace::Rgb,	[0.1803922, 0.8, 0.2509804]),
	("lime",	ColorSpace::Rgb,	[0.0039216, 1.0, 0.4392157]),
];

fn func(f: ColorFn) -> Value { Value::Func(Func::Native(NativeFunc::Color(f))) }

pub fn define(scope: &mut Scope) {
	for f in [ColorFn::Rgb, ColorFn::Luma, ColorFn::Cmyk, ColorFn::Oklab, ColorFn::Oklch] {
		scope.define(f.name(), func(f));
	}
	for (name, space, c) in NAMED {
		let col = Color { space, c: [c[0], c[1], c[2], 0.0], alpha: 1.0 };
		scope.define(name, Value::Color(col));
	}
}

/// `color.name`: the constructors Typst scopes under the type, `mix`, and the colour maps.
pub fn color_static(name: &str) -> Option<Value> {
	let f = match name {
		"linear-rgb"	=> ColorFn::LinearRgb,
		"hsl"			=> ColorFn::Hsl,
		"hsv"			=> ColorFn::Hsv,
		"mix"			=> ColorFn::Mix,
		"rgb"			=> ColorFn::Rgb,
		"luma"			=> ColorFn::Luma,
		"cmyk"			=> ColorFn::Cmyk,
		"oklab"			=> ColorFn::Oklab,
		"oklch"			=> ColorFn::Oklch,
		"map"			=> return Some(Value::Module(Arc::new(color_maps()))),
		_				=> return None,
	};
	Some(func(f))
}

fn color_maps() -> Module {
	let mut s = Scope::new();
	for (n, (name, cols)) in data::COLOR_MAPS.iter().enumerate() {
		let arr = cols.iter().map(|c| Value::Color(rgb8(c[0], c[1], c[2], 255))).collect();
		crate::eval::lib::foundations::define_nth(&mut s, n, name, Value::array(arr));
	}
	Module::new("map", s)
}

pub fn gradient_static(name: &str) -> Option<Value> {
	let f = match name {
		"linear"	=> ColorFn::GradientLinear,
		"radial"	=> ColorFn::GradientRadial,
		"conic"		=> ColorFn::GradientConic,
		_			=> return None,
	};
	Some(func(f))
}

pub fn method(name: &str) -> Option<ColorFn> {
	let f = match name {
		"lighten"			=> ColorFn::Lighten,
		"darken"			=> ColorFn::Darken,
		"saturate"			=> ColorFn::Saturate,
		"desaturate"		=> ColorFn::Desaturate,
		"negate"			=> ColorFn::Negate,
		"rotate"			=> ColorFn::Rotate,
		"components"		=> ColorFn::Components,
		"space"				=> ColorFn::Space,
		"to-hex"			=> ColorFn::ToHex,
		"transparentize"	=> ColorFn::Transparentize,
		"opacify"			=> ColorFn::Opacify,
		other				=> return gradient_method(other),
	};
	Some(f)
}

pub fn gradient_method(name: &str) -> Option<ColorFn> {
	let f = match name {
		"sample"		=> ColorFn::GradientSample,
		"samples"		=> ColorFn::GradientSamples,
		"stops"			=> ColorFn::GradientStops,
		"sharp"			=> ColorFn::GradientSharp,
		"repeat"		=> ColorFn::GradientRepeat,
		"kind"			=> ColorFn::GradientKindOf,
		"space"			=> ColorFn::GradientSpace,
		"relative"		=> ColorFn::GradientRelative,
		"angle"			=> ColorFn::GradientAngle,
		"center"		=> ColorFn::GradientCenter,
		"radius"		=> ColorFn::GradientRadius,
		"focal-center"	=> ColorFn::GradientFocalCenter,
		"focal-radius"	=> ColorFn::GradientFocalRadius,
		_				=> return None,
	};
	Some(f)
}

// Colour arithmetic.

fn rgb8(r: u8, g: u8, b: u8, a: u8) -> Color {
	Color { space: ColorSpace::Rgb, c: [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 0.0], alpha: a as f32 / 255.0 }
}

fn decode(x: f32) -> f32 { if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) } }

fn encode(x: f32) -> f32 { if x <= 0.0031308 { x * 12.92 } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 } }

fn norm_hue(h: f32) -> f32 {
	let r = h % 360.0;
	if r < 0.0 { r + 360.0 } else { r }
}

fn lin_to_oklab(r: f32, g: f32, b: f32) -> [f32; 3] {
	let l = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
	let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
	let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;
	let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
	[
		0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
		1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
		0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
	]
}

fn oklab_to_lin(l: f32, a: f32, b: f32) -> [f32; 3] {
	let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
	let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
	let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
	let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
	[
		4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
		-1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
		-0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
	]
}

fn rgb_to_hsx(r: f32, g: f32, b: f32, hsv: bool) -> [f32; 3] {
	let max = r.max(g).max(b);
	let min = r.min(g).min(b);
	let chroma = max - min;
	let hue = if chroma == 0.0 {
		0.0
	} else if max == r {
		60.0 * ((g - b) / chroma)
	} else if max == g {
		60.0 * ((b - r) / chroma + 2.0)
	} else {
		60.0 * ((r - g) / chroma + 4.0)
	};
	if hsv {
		let s = if max == 0.0 { 0.0 } else { chroma / max };
		[norm_hue(hue), s, max]
	} else {
		let l = (max + min) / 2.0;
		let s = if chroma == 0.0 { 0.0 } else { chroma / (1.0 - (2.0 * l - 1.0).abs()) };
		[norm_hue(hue), s, l]
	}
}

fn hsx_to_rgb(h: f32, s: f32, x: f32, hsv: bool) -> [f32; 3] {
	let (c, m) = if hsv {
		let c = x * s;
		(c, x - c)
	} else {
		let c = (1.0 - (2.0 * x - 1.0).abs()) * s;
		(c, x - c / 2.0)
	};
	let hp = norm_hue(h) / 60.0;
	let xx = c * (1.0 - (hp % 2.0 - 1.0).abs());
	let (r, g, b) = match hp as u32 {
		0	=> (c, xx, 0.0),
		1	=> (xx, c, 0.0),
		2	=> (0.0, c, xx),
		3	=> (0.0, xx, c),
		4	=> (xx, 0.0, c),
		_	=> (c, 0.0, xx),
	};
	[r + m, g + m, b + m]
}

// CMYK to sRGB through the sampled ICC grid, interpolated quadrilinearly.
fn cmyk_to_rgb(c: [f32; 4]) -> [f32; 3] {
	let n = GRID - 1;
	let mut idx = [0usize; 4];
	let mut fr = [0f32; 4];
	for d in 0..4 {
		let x = c[d].clamp(0.0, 1.0) * n as f32;
		let i = (x as usize).min(n - 1);
		idx[d] = i;
		fr[d] = x - i as f32;
	}
	let mut out = [0f32; 3];
	for corner in 0..16usize {
		let mut w = 1f32;
		let mut k = 0usize;
		for d in 0..4 {
			let bit = (corner >> (3 - d)) & 1;
			w *= if bit == 1 { fr[d] } else { 1.0 - fr[d] };
			k = k * GRID + idx[d] + bit;
		}
		for (ch, o) in out.iter_mut().enumerate() {
			*o += w * CMYK_GRID.get(k * 3 + ch).copied().unwrap_or(0) as f32;
		}
	}
	// The transform's output is eight-bit.
	[(out[0].round()) / 255.0, (out[1].round()) / 255.0, (out[2].round()) / 255.0]
}

fn rgb_to_cmyk(r: f32, g: f32, b: f32) -> [f32; 4] {
	let k = 1.0 - r.max(g).max(b);
	if k == 1.0 {
		return [0.0, 0.0, 0.0, 1.0];
	}
	[(1.0 - r - k) / (1.0 - k), (1.0 - g - k) / (1.0 - k), (1.0 - b - k) / (1.0 - k), k]
}

/// The colour as gamma-encoded sRGB components.
fn to_srgb(c: &Color) -> [f32; 3] {
	let [a, b, d, e] = c.c;
	match c.space {
		ColorSpace::Rgb			=> [a, b, d],
		ColorSpace::Luma		=> [a, a, a],
		ColorSpace::Hsl			=> hsx_to_rgb(a, b, d, false),
		ColorSpace::Hsv			=> hsx_to_rgb(a, b, d, true),
		ColorSpace::Cmyk		=> cmyk_to_rgb([a, b, d, e]),
		ColorSpace::LinearRgb	=> [encode(a), encode(b), encode(d)],
		ColorSpace::Oklab | ColorSpace::Oklch => {
			let [r, g, bl] = to_linear(c);
			[encode(r), encode(g), encode(bl)]
		}
	}
}

fn to_linear(c: &Color) -> [f32; 3] {
	let [a, b, d, _] = c.c;
	match c.space {
		ColorSpace::LinearRgb	=> [a, b, d],
		ColorSpace::Oklab		=> oklab_to_lin(a, b, d),
		ColorSpace::Oklch		=> {
			let h = d.to_radians();
			oklab_to_lin(a, b * h.cos(), b * h.sin())
		}
		ColorSpace::Luma		=> {
			let l = decode(a);
			[l, l, l]
		}
		_ => {
			let [r, g, bl] = to_srgb(c);
			[decode(r), decode(g), decode(bl)]
		}
	}
}

fn to_oklab(c: &Color) -> [f32; 3] {
	match c.space {
		ColorSpace::Oklab	=> [c.c[0], c.c[1], c.c[2]],
		ColorSpace::Oklch	=> {
			let h = c.c[2].to_radians();
			[c.c[0], c.c[1] * h.cos(), c.c[1] * h.sin()]
		}
		// A grey is neutral in Oklab: its lightness alone.
		ColorSpace::Luma	=> [decode(c.c[0]).cbrt(), 0.0, 0.0],
		_ => {
			let [r, g, b] = to_linear(c);
			lin_to_oklab(r, g, b)
		}
	}
}

/// The colour in another space, alpha carried over (CMYK has none).
pub fn to_space(c: &Color, space: ColorSpace) -> Color {
	if c.space == space {
		return *c;
	}
	let alpha = if c.space == ColorSpace::Cmyk { 1.0 } else { c.alpha };
	let v: [f32; 4] = match space {
		ColorSpace::Rgb			=> {
			let [r, g, b] = to_srgb(c);
			[r, g, b, 0.0]
		}
		ColorSpace::LinearRgb	=> {
			let [r, g, b] = to_linear(c);
			[r, g, b, 0.0]
		}
		ColorSpace::Luma		=> {
			let [r, g, b] = to_linear(c);
			let y = 0.212_672_9 * r + 0.715_152_2 * g + 0.072_175 * b;
			[encode(y), 0.0, 0.0, 0.0]
		}
		ColorSpace::Oklab		=> {
			let [l, a, b] = to_oklab(c);
			[l, a, b, 0.0]
		}
		ColorSpace::Oklch		=> {
			let [l, a, b] = to_oklab(c);
			[l, (a * a + b * b).sqrt(), norm_hue(b.atan2(a).to_degrees()), 0.0]
		}
		ColorSpace::Hsl | ColorSpace::Hsv => {
			let [r, g, b] = to_srgb(c);
			let [h, s, x] = rgb_to_hsx(r, g, b, space == ColorSpace::Hsv);
			[h, s, x, 0.0]
		}
		ColorSpace::Cmyk		=> {
			if c.space == ColorSpace::Luma {
				let l = 1.0 - c.c[0];
				[l * 0.75, l * 0.68, l * 0.67, l * 0.90]
			} else {
				let [r, g, b] = to_srgb(c);
				rgb_to_cmyk(r, g, b)
			}
		}
	};
	Color { space, c: v, alpha: if space == ColorSpace::Cmyk { 1.0 } else { alpha } }
}

// Eight-bit quantisation rounds half to even, as Typst's does: 30% is 0x4c, 10% is 0x1a.
fn to_u8(x: f32) -> u8 { (x.clamp(0.0, 1.0) * 255.0).round_ties_even() as u8 }

impl Color {
	/// The colour as eight-bit sRGB with alpha, for the back end.
	pub fn to_rgba(&self) -> Outcome<oxedyne_fe2o3_graphics::colour::Rgba> {
		let [r, g, b] = to_srgb(self);
		let a = if self.space == ColorSpace::Cmyk { 1.0 } else { self.alpha };
		Ok(oxedyne_fe2o3_graphics::colour::Rgba::new(to_u8(r), to_u8(g), to_u8(b), to_u8(a)))
	}
}

pub fn to_hex(c: &Color) -> String {
	let [r, g, b] = to_srgb(c);
	let a = if c.space == ColorSpace::Cmyk { 1.0 } else { c.alpha };
	let a8 = to_u8(a);
	if a8 == 255 {
		fmt!("#{:02x}{:02x}{:02x}", to_u8(r), to_u8(g), to_u8(b))
	} else {
		fmt!("#{:02x}{:02x}{:02x}{:02x}", to_u8(r), to_u8(g), to_u8(b), a8)
	}
}

fn ratio(x: f32) -> String { repr_ratio(Ratio(x as f64)) }

fn component(x: f32) -> String { format_float(x as f64, Some(3), false, "") }

fn hue(x: f32) -> String { repr_angle(Angle((norm_hue(x) as f64).to_radians())) }

pub fn repr_color(c: &Color) -> String {
	let [a, b, d, e] = c.c;
	let alpha = c.alpha != 1.0;
	match c.space {
		ColorSpace::Rgb => fmt!("rgb({})", repr_str(&to_hex(c))),
		ColorSpace::Luma => if alpha {
			fmt!("luma({}, {})", ratio(a), ratio(c.alpha))
		} else {
			fmt!("luma({})", ratio(a))
		},
		ColorSpace::LinearRgb => if alpha {
			fmt!("color.linear-rgb({}, {}, {}, {})", ratio(a), ratio(b), ratio(d), ratio(c.alpha))
		} else {
			fmt!("color.linear-rgb({}, {}, {})", ratio(a), ratio(b), ratio(d))
		},
		ColorSpace::Cmyk => fmt!("cmyk({}, {}, {}, {})", ratio(a), ratio(b), ratio(d), ratio(e)),
		ColorSpace::Oklab => if alpha {
			fmt!("oklab({}, {}, {}, {})", ratio(a), component(b), component(d), ratio(c.alpha))
		} else {
			fmt!("oklab({}, {}, {})", ratio(a), component(b), component(d))
		},
		ColorSpace::Oklch => if alpha {
			fmt!("oklch({}, {}, {}, {})", ratio(a), component(b), hue(d), ratio(c.alpha))
		} else {
			fmt!("oklch({}, {}, {})", ratio(a), component(b), hue(d))
		},
		ColorSpace::Hsl | ColorSpace::Hsv => {
			let name = if c.space == ColorSpace::Hsl { "hsl" } else { "hsv" };
			if alpha {
				fmt!("color.{}({}, {}, {}, {})", name, hue(a), ratio(b), ratio(d), ratio(c.alpha))
			} else {
				fmt!("color.{}({}, {}, {})", name, hue(a), ratio(b), ratio(d))
			}
		}
	}
}

fn space_fn(s: ColorSpace) -> ColorFn {
	match s {
		ColorSpace::Rgb			=> ColorFn::Rgb,
		ColorSpace::Luma		=> ColorFn::Luma,
		ColorSpace::Cmyk		=> ColorFn::Cmyk,
		ColorSpace::Oklab		=> ColorFn::Oklab,
		ColorSpace::Oklch		=> ColorFn::Oklch,
		ColorSpace::LinearRgb	=> ColorFn::LinearRgb,
		ColorSpace::Hsl			=> ColorFn::Hsl,
		ColorSpace::Hsv			=> ColorFn::Hsv,
	}
}

/// A colour space named by its constructor (`space: rgb`).
fn space_of(engine: &mut Engine, span: Span, v: &Value) -> Outcome<ColorSpace> {
	let s = match v {
		Value::Func(Func::Native(NativeFunc::Color(f))) => match f {
			ColorFn::Rgb		=> Some(ColorSpace::Rgb),
			ColorFn::Luma		=> Some(ColorSpace::Luma),
			ColorFn::Cmyk		=> Some(ColorSpace::Cmyk),
			ColorFn::Oklab		=> Some(ColorSpace::Oklab),
			ColorFn::Oklch		=> Some(ColorSpace::Oklch),
			ColorFn::LinearRgb	=> Some(ColorSpace::LinearRgb),
			ColorFn::Hsl		=> Some(ColorSpace::Hsl),
			ColorFn::Hsv		=> Some(ColorSpace::Hsv),
			_					=> None,
		},
		_ => None,
	};
	match s {
		Some(s)	=> Ok(s),
		None	=> Err(engine.error(span,
			"expected `rgb`, `luma`, `cmyk`, `oklab`, `oklch`, `color.linear-rgb`, `color.hsl`, or `color.hsv`")),
	}
}

// Constructors' component casts.

// An eight-bit int or a ratio.
fn comp(engine: &mut Engine, span: Span, v: Value) -> Outcome<f32> {
	match v {
		Value::Int(i) if (0..=255).contains(&i)	=> Ok(i as f32 / 255.0),
		Value::Int(_)	=> Err(engine.error(span, "number must be between 0 and 255")),
		Value::Ratio(r)	=> ratio_comp(engine, span, r),
		other			=> Err(mismatch(engine, span, "integer or ratio", &other)),
	}
}

fn ratio_comp(engine: &mut Engine, span: Span, r: Ratio) -> Outcome<f32> {
	if (0.0..=1.0).contains(&r.0) {
		Ok(r.0 as f32)
	} else {
		Err(engine.error(span, "ratio must be between 0% and 100%"))
	}
}

fn ratio_only(engine: &mut Engine, span: Span, v: Value) -> Outcome<f32> {
	match v {
		Value::Ratio(r)	=> ratio_comp(engine, span, r),
		other			=> Err(mismatch(engine, span, "ratio", &other)),
	}
}

// Oklab's a and b and Oklch's chroma: a float, or a ratio of 0.4.
fn chroma(engine: &mut Engine, span: Span, v: Value) -> Outcome<f32> {
	match v {
		Value::Float(f)	=> Ok(f as f32),
		Value::Int(i)	=> Ok(i as f32),
		Value::Ratio(r)	=> Ok((r.0 * 0.4) as f32),
		other			=> Err(mismatch(engine, span, "float or ratio", &other)),
	}
}

fn angle_deg(engine: &mut Engine, span: Span, v: Value) -> Outcome<f32> {
	match v {
		Value::Angle(a)	=> Ok(a.0.to_degrees() as f32),
		other			=> Err(mismatch(engine, span, "angle", &other)),
	}
}

fn color_of(engine: &mut Engine, span: Span, v: Value) -> Outcome<Color> {
	match v {
		Value::Color(c)	=> Ok(c),
		other			=> Err(mismatch(engine, span, "color", &other)),
	}
}

fn ratio_arg(engine: &mut Engine, span: Span, v: Value) -> Outcome<f32> {
	match v {
		Value::Ratio(r)	=> Ok(r.0 as f32),
		other			=> Err(mismatch(engine, span, "ratio", &other)),
	}
}

fn parse_hex(engine: &mut Engine, span: Span, s: &str) -> Outcome<Color> {
	let h = s.strip_prefix('#').unwrap_or(s);
	if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
		return Err(engine.error(span, "color string contains non-hexadecimal letters"));
	}
	let nib = |i: usize| u8::from_str_radix(&h[i..i + 1], 16).map(|x| x * 17).unwrap_or(0);
	let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0);
	let c = match h.len() {
		3	=> rgb8(nib(0), nib(1), nib(2), 255),
		4	=> rgb8(nib(0), nib(1), nib(2), nib(3)),
		6	=> rgb8(byte(0), byte(2), byte(4), 255),
		8	=> rgb8(byte(0), byte(2), byte(4), byte(6)),
		_	=> return Err(engine.error(span, "color string has wrong length")),
	};
	Ok(c)
}

fn construct(f: ColorFn, engine: &mut Engine, args: &mut Args) -> Outcome<Color> {
	let span = args.span;
	let vals = res!(args.all::<Value>());
	if vals.len() == 1 {
		if let Value::Color(c) = &vals[0] {
			let space = match f {
				ColorFn::Rgb		=> ColorSpace::Rgb,
				ColorFn::Luma		=> ColorSpace::Luma,
				ColorFn::Cmyk		=> ColorSpace::Cmyk,
				ColorFn::Oklab		=> ColorSpace::Oklab,
				ColorFn::Oklch		=> ColorSpace::Oklch,
				ColorFn::LinearRgb	=> ColorSpace::LinearRgb,
				ColorFn::Hsl		=> ColorSpace::Hsl,
				_					=> ColorSpace::Hsv,
			};
			return Ok(to_space(c, space));
		}
		if f == ColorFn::Rgb {
			if let Value::Str(s) = &vals[0] {
				return parse_hex(engine, span, s);
			}
		}
	}
	let (need_n, names): (usize, &[&str]) = match f {
		ColorFn::Luma	=> (1, &["lightness", "alpha"]),
		ColorFn::Cmyk	=> (4, &["cyan", "magenta", "yellow", "key"]),
		ColorFn::Oklab	=> (3, &["lightness", "a", "b", "alpha"]),
		ColorFn::Oklch	=> (3, &["lightness", "chroma", "hue", "alpha"]),
		ColorFn::Hsl	=> (3, &["hue", "saturation", "lightness", "alpha"]),
		ColorFn::Hsv	=> (3, &["hue", "saturation", "value", "alpha"]),
		_				=> (3, &["red", "green", "blue", "alpha"]),
	};
	if vals.len() < need_n {
		return Err(engine.error(span, fmt!("missing argument: {}", names[vals.len()])));
	}
	if vals.len() > names.len() {
		return Err(engine.error(span, "unexpected argument"));
	}
	let mut it = vals.into_iter();
	let mut next = || it.next().unwrap_or(Value::None);
	let c = match f {
		ColorFn::Luma => {
			let l = res!(comp(engine, span, next()));
			let a = match next() {
				Value::None	=> 1.0,
				v			=> res!(comp(engine, span, v)),
			};
			Color { space: ColorSpace::Luma, c: [l, 0.0, 0.0, 0.0], alpha: a }
		}
		ColorFn::Cmyk => {
			let c = res!(ratio_only(engine, span, next()));
			let m = res!(ratio_only(engine, span, next()));
			let y = res!(ratio_only(engine, span, next()));
			let k = res!(ratio_only(engine, span, next()));
			Color { space: ColorSpace::Cmyk, c: [c, m, y, k], alpha: 1.0 }
		}
		ColorFn::Oklab | ColorFn::Oklch => {
			let l = res!(ratio_only(engine, span, next()));
			let x = res!(chroma(engine, span, next()));
			let y = if f == ColorFn::Oklab {
				res!(chroma(engine, span, next()))
			} else {
				norm_hue(res!(angle_deg(engine, span, next())))
			};
			let a = match next() {
				Value::None	=> 1.0,
				v			=> res!(ratio_only(engine, span, v)),
			};
			let space = if f == ColorFn::Oklab { ColorSpace::Oklab } else { ColorSpace::Oklch };
			Color { space, c: [l, x, y, 0.0], alpha: a }
		}
		ColorFn::Hsl | ColorFn::Hsv => {
			let h = norm_hue(res!(angle_deg(engine, span, next())));
			let s = res!(comp(engine, span, next()));
			let x = res!(comp(engine, span, next()));
			let a = match next() {
				Value::None	=> 1.0,
				v			=> res!(comp(engine, span, v)),
			};
			let space = if f == ColorFn::Hsl { ColorSpace::Hsl } else { ColorSpace::Hsv };
			Color { space, c: [h, s, x, 0.0], alpha: a }
		}
		_ => {
			let r = res!(comp(engine, span, next()));
			let g = res!(comp(engine, span, next()));
			let b = res!(comp(engine, span, next()));
			let a = match next() {
				Value::None	=> 1.0,
				v			=> res!(comp(engine, span, v)),
			};
			let space = if f == ColorFn::LinearRgb { ColorSpace::LinearRgb } else { ColorSpace::Rgb };
			Color { space, c: [r, g, b, 0.0], alpha: a }
		}
	};
	Ok(c)
}

// Each space's components as a four-vector, hue in degrees, for mixing.
fn to_vec4(c: &Color) -> [f32; 4] {
	match c.space {
		ColorSpace::Luma	=> [c.c[0], c.c[0], c.c[0], c.alpha],
		ColorSpace::Cmyk	=> c.c,
		_					=> [c.c[0], c.c[1], c.c[2], c.alpha],
	}
}

fn from_vec4(space: ColorSpace, v: [f32; 4]) -> Color {
	match space {
		ColorSpace::Luma					=> Color { space, c: [v[0], 0.0, 0.0, 0.0], alpha: v[3] },
		ColorSpace::Cmyk					=> Color { space, c: v, alpha: 1.0 },
		ColorSpace::Hsl | ColorSpace::Hsv	=> Color { space, c: [norm_hue(v[0]), v[1], v[2], 0.0], alpha: v[3] },
		ColorSpace::Oklch					=> Color { space, c: [v[0], v[1], norm_hue(v[2]), 0.0], alpha: v[3] },
		_									=> Color { space, c: [v[0], v[1], v[2], 0.0], alpha: v[3] },
	}
}

fn hue_index(space: ColorSpace) -> Option<usize> {
	match space {
		ColorSpace::Hsl | ColorSpace::Hsv	=> Some(0),
		ColorSpace::Oklch					=> Some(2),
		_									=> None,
	}
}

/// Mixes weighted colours in a space, hue the short way round for polar spaces, as Typst's `mix_iter`.
pub fn mix(colors: &[(Color, f32)], space: ColorSpace) -> std::result::Result<Color, &'static str> {
	if hue_index(space).is_some() && colors.len() > 2 {
		return Err("cannot mix more than two colors in a hue-based space");
	}
	let m = if let (Some(hi), [(c0, w0), (c1, w1)]) = (hue_index(space), colors) {
		let a = to_vec4(&to_space(c0, space));
		let b = to_vec4(&to_space(c1, space));
		if w0 + w1 <= 0.0 {
			return Err("sum of weights must be positive");
		}
		let mut m = [0f32; 4];
		for i in 0..4 {
			m[i] = (w0 * a[i] + w1 * b[i]) / (w0 + w1);
		}
		if (a[hi] - b[hi]).abs() > 180.0 {
			let (h0, h1) = if a[hi] < b[hi] { (a[hi] + 360.0, b[hi]) } else { (a[hi], b[hi] + 360.0) };
			m[hi] = (w0 * h0 + w1 * h1) / (w0 + w1);
		}
		m
	} else {
		let mut total = 0f32;
		let mut acc = [0f32; 4];
		for (c, w) in colors {
			let v = to_vec4(&to_space(c, space));
			for i in 0..4 {
				acc[i] += w * v[i];
			}
			total += w;
		}
		if total <= 0.0 {
			return Err("sum of weights must be positive");
		}
		acc.map(|x| x / total)
	};
	Ok(from_vec4(space, m))
}

// Relative lighten and darken, palette's: move towards the maximum or minimum by a factor of the gap.
fn lighten_ch(x: f32, f: f32) -> f32 { if f >= 0.0 { x + (1.0 - x) * f } else { x + x * f } }

fn darken_ch(x: f32, f: f32) -> f32 { if f >= 0.0 { x - x * f } else { x - (1.0 - x) * f } }

fn lighten(c: &Color, f: f32, dark: bool) -> Color {
	let op = |x: f32| if dark { darken_ch(x, f) } else { lighten_ch(x, f) };
	let mut out = *c;
	match c.space {
		ColorSpace::Luma | ColorSpace::Oklab | ColorSpace::Oklch => out.c[0] = op(c.c[0]),
		ColorSpace::Rgb | ColorSpace::LinearRgb => for i in 0..3 {
			out.c[i] = op(c.c[i]);
		},
		ColorSpace::Hsl | ColorSpace::Hsv => out.c[2] = op(c.c[2]),
		ColorSpace::Cmyk => for i in 0..4 {
			out.c[i] = if dark {
				(c.c[i] + (1.0 - c.c[i]) * f).clamp(0.0, 1.0)
			} else {
				(c.c[i] - c.c[i] * f).clamp(0.0, 1.0)
			};
		},
	}
	out
}

fn saturate(c: &Color, f: f32, de: bool) -> Color {
	let op = |s: f32| if de { darken_ch(s, f) } else { lighten_ch(s, f) };
	match c.space {
		ColorSpace::Hsl | ColorSpace::Hsv => {
			let mut out = *c;
			out.c[1] = op(c.c[1]);
			out
		}
		_ => {
			let mut hsv = to_space(c, ColorSpace::Hsv);
			hsv.c[1] = op(hsv.c[1]);
			to_space(&hsv, c.space)
		}
	}
}

fn negate(c: &Color, space: ColorSpace) -> Color {
	let s = to_space(c, space);
	let mut n = s;
	match space {
		ColorSpace::Luma	=> n.c[0] = 1.0 - s.c[0],
		ColorSpace::Oklab	=> n.c = [1.0 - s.c[0], -s.c[1], -s.c[2], 0.0],
		ColorSpace::Oklch	=> n.c = [1.0 - s.c[0], s.c[1], norm_hue(s.c[2] + 180.0), 0.0],
		ColorSpace::Rgb | ColorSpace::LinearRgb => n.c = [1.0 - s.c[0], 1.0 - s.c[1], 1.0 - s.c[2], 0.0],
		ColorSpace::Cmyk	=> n.c = [1.0 - s.c[0], 1.0 - s.c[1], 1.0 - s.c[2], s.c[3]],
		ColorSpace::Hsl | ColorSpace::Hsv => n.c[0] = norm_hue(s.c[0] + 180.0),
	}
	to_space(&n, c.space)
}

fn scale_alpha(alpha: f32, factor: f32) -> f32 {
	let a = if factor >= 0.0 { alpha * (1.0 - factor) } else { alpha + (-factor) * (1.0 - alpha) };
	a.clamp(0.0, 1.0)
}

fn components(c: &Color, alpha: bool) -> Vec<Value> {
	let r = |x: f32| Value::Ratio(Ratio(x as f64));
	let fl = |x: f32| Value::Float(x as f64);
	let an = |x: f32| Value::Angle(Angle((x as f64).to_radians()));
	let mut v = match c.space {
		ColorSpace::Luma		=> vec![r(c.c[0])],
		ColorSpace::Oklab		=> vec![r(c.c[0]), fl(c.c[1]), fl(c.c[2])],
		ColorSpace::Oklch		=> vec![r(c.c[0]), fl(c.c[1]), an(c.c[2])],
		ColorSpace::Rgb | ColorSpace::LinearRgb => vec![r(c.c[0]), r(c.c[1]), r(c.c[2])],
		ColorSpace::Cmyk		=> return vec![r(c.c[0]), r(c.c[1]), r(c.c[2]), r(c.c[3])],
		ColorSpace::Hsl | ColorSpace::Hsv => vec![an(c.c[0]), r(c.c[1]), r(c.c[2])],
	};
	if alpha {
		v.push(r(c.alpha));
	}
	v
}

// Gradients.

fn stops_of(engine: &mut Engine, span: Span, vals: Vec<Value>, conic: bool) -> Outcome<Vec<(Color, Ratio)>> {
	let mut raw: Vec<(Color, Option<Ratio>)> = Vec::with_capacity(vals.len());
	for v in vals {
		match v {
			Value::Color(c) => raw.push((c, None)),
			Value::Array(a) if a.len() == 2 => {
				let c = res!(color_of(engine, span, a[0].clone()));
				let o = match &a[1] {
					Value::Ratio(r)				=> *r,
					Value::Angle(x) if conic	=> Ratio(x.0 / std::f64::consts::TAU),
					other						=> return Err(mismatch(engine, span, "ratio", other)),
				};
				raw.push((c, Some(o)));
			}
			other => return Err(mismatch(engine, span, "color or array", &other)),
		}
	}
	if raw.len() < 2 {
		return Err(engine.error_hint(span, "a gradient must have at least two stops",
			"try filling the shape with a single color instead"));
	}
	if raw.iter().any(|(_, o)| o.is_some()) {
		let mut last = f64::NEG_INFINITY;
		for (_, o) in &raw {
			match o {
				None => return Err(engine.error_hint(span, "either all stops must have an offset or none of them can",
					"try adding an offset to all stops")),
				Some(r) => {
					if r.0 < last {
						return Err(engine.error(span, "offsets must be in monotonic order"));
					}
					last = r.0;
				}
			}
		}
		let out: Vec<(Color, Ratio)> = raw.into_iter().map(|(c, o)| (c, o.unwrap_or(Ratio(0.0)))).collect();
		for (_, o) in &out {
			if o.0 < 0.0 || o.0 > 1.0 {
				return Err(engine.error(span, "offset must be between 0 and 1"));
			}
		}
		if out.first().map(|s| s.1 .0 != 0.0).unwrap_or(false) {
			return Err(engine.error_hint(span, "first stop must have an offset of 0", "try setting this stop to `0%`"));
		}
		if out.last().map(|s| s.1 .0 != 1.0).unwrap_or(false) {
			return Err(engine.error_hint(span, "last stop must have an offset of 100%", "try setting this stop to `100%`"));
		}
		return Ok(out);
	}
	let n = raw.len();
	Ok(raw.into_iter().enumerate().map(|(i, (c, _))| (c, Ratio(i as f64 / (n - 1) as f64))).collect())
}

fn relative_of(engine: &mut Engine, span: Span, v: Option<Value>) -> Outcome<RelativeTo> {
	match v {
		None | Some(Value::Auto)	=> Ok(RelativeTo::Auto),
		Some(Value::Str(s)) => match s.as_str() {
			"self"		=> Ok(RelativeTo::SelfBox),
			"parent"	=> Ok(RelativeTo::Parent),
			_			=> Err(engine.error(span, "expected \"self\" or \"parent\"")),
		},
		Some(other) => Err(mismatch(engine, span, "auto or string", &other)),
	}
}

fn pair_of(engine: &mut Engine, span: Span, v: Value) -> Outcome<(Ratio, Ratio)> {
	match v {
		Value::Array(a) if a.len() == 2 => match (&a[0], &a[1]) {
			(Value::Ratio(x), Value::Ratio(y))	=> Ok((*x, *y)),
			_									=> Err(engine.error(span, "expected a pair of ratios")),
		},
		other => Err(mismatch(engine, span, "array", &other)),
	}
}

fn make_gradient(f: ColorFn, engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	let space = match res!(args.named::<Value>("space")) {
		None	=> ColorSpace::Oklab,
		Some(v)	=> res!(space_of(engine, span, &v)),
	};
	let rel = res!(args.named::<Value>("relative"));
	let relative = res!(relative_of(engine, span, rel));
	let kind = match f {
		ColorFn::GradientLinear => {
			let angle = res!(args.named::<Value>("angle"));
			let dir = res!(args.named::<Value>("dir"));
			let a = match (angle, dir) {
				(Some(Value::Angle(a)), _)	=> a,
				(Some(other), _)			=> return Err(mismatch(engine, span, "angle", &other)),
				(None, Some(Value::Direction(d))) => Angle(match d {
					Direction::Ltr	=> 0.0,
					Direction::Rtl	=> std::f64::consts::PI,
					Direction::Ttb	=> std::f64::consts::FRAC_PI_2,
					Direction::Btt	=> 3.0 * std::f64::consts::FRAC_PI_2,
				}),
				(None, Some(other))		=> return Err(mismatch(engine, span, "direction", &other)),
				(None, None)			=> Angle(0.0),
			};
			GradientKind::Linear { angle: a }
		}
		ColorFn::GradientRadial => {
			let center = match res!(args.named::<Value>("center")) {
				None	=> (Ratio(0.5), Ratio(0.5)),
				Some(v)	=> res!(pair_of(engine, span, v)),
			};
			let radius = match res!(args.named::<Value>("radius")) {
				None				=> Ratio(0.5),
				Some(Value::Ratio(r))	=> r,
				Some(other)			=> return Err(mismatch(engine, span, "ratio", &other)),
			};
			let focal_center = match res!(args.named::<Value>("focal-center")) {
				None | Some(Value::Auto)	=> center,
				Some(v)						=> res!(pair_of(engine, span, v)),
			};
			let focal_radius = match res!(args.named::<Value>("focal-radius")) {
				None				=> Ratio(0.0),
				Some(Value::Ratio(r))	=> r,
				Some(other)			=> return Err(mismatch(engine, span, "ratio", &other)),
			};
			if focal_radius.0 > radius.0 {
				return Err(engine.error_hint(span, "the focal radius must be smaller than the end radius",
					"try using a focal radius of `0%` instead"));
			}
			let d = ((focal_center.0 .0 - center.0 .0).powi(2) + (focal_center.1 .0 - center.1 .0).powi(2)).sqrt();
			if d + focal_radius.0 > radius.0 {
				return Err(engine.error_hint(span, "the focal circle must be inside of the end circle",
					"try using a focal center of `auto` instead"));
			}
			GradientKind::Radial { center, radius, focal_center, focal_radius }
		}
		_ => {
			let angle = match res!(args.named::<Value>("angle")) {
				None				=> Angle(0.0),
				Some(Value::Angle(a))	=> a,
				Some(other)			=> return Err(mismatch(engine, span, "angle", &other)),
			};
			let center = match res!(args.named::<Value>("center")) {
				None	=> (Ratio(0.5), Ratio(0.5)),
				Some(v)	=> res!(pair_of(engine, span, v)),
			};
			GradientKind::Conic { center, angle }
		}
	};
	let vals = res!(args.all::<Value>());
	res!(finish(engine, args));
	let stops = res!(stops_of(engine, span, vals, f == ColorFn::GradientConic));
	let stops = stops.into_iter().map(|(c, r)| (to_space(&c, space), r)).collect();
	Ok(Value::Gradient(Arc::new(Gradient { kind, stops, space, relative })))
}

/// The colour at `t` (0 to 1) along a gradient's stops, mixed in its space.
pub fn sample(g: &Gradient, t: f64) -> Color {
	let t = t.clamp(0.0, 1.0);
	let stops = &g.stops;
	let mut lo = 0;
	let mut hi = stops.len();
	while lo < hi {
		let mid = (lo + hi) / 2;
		if stops[mid].1 .0 < t {
			lo = mid + 1;
		} else {
			hi = mid;
		}
	}
	if lo == 0 {
		lo = 1;
	}
	if lo >= stops.len() {
		lo = stops.len() - 1;
	}
	let (c0, p0) = stops[lo - 1];
	let (c1, p1) = stops[lo];
	let span = p1.0 - p0.0;
	let u = if span == 0.0 { 0.0 } else { (t - p0.0) / span };
	match mix(&[(c0, (1.0 - u) as f32), (c1, u as f32)], g.space) {
		Ok(c)	=> c,
		Err(_)	=> c0,
	}
}

fn sample_at(engine: &mut Engine, span: Span, g: &Gradient, v: Value) -> Outcome<Color> {
	let t = match v {
		Value::Ratio(r)	=> r.0,
		Value::Angle(a)	=> a.0.to_degrees().rem_euclid(360.0) / 360.0,
		other			=> return Err(mismatch(engine, span, "ratio or angle", &other)),
	};
	Ok(sample(g, t))
}

fn same_stop(a: &(Color, Ratio), b: &(Color, Ratio)) -> bool { a.0 == b.0 && a.1 == b.1 }

fn dedup(v: Vec<(Color, Ratio)>) -> Vec<(Color, Ratio)> {
	let mut out: Vec<(Color, Ratio)> = Vec::with_capacity(v.len());
	for s in v {
		if out.last().map(|l| same_stop(l, &s)).unwrap_or(false) {
			continue;
		}
		out.push(s);
	}
	out
}

pub fn repr_gradient(g: &Gradient) -> String {
	let mut r = String::new();
	let pair = |p: (Ratio, Ratio)| fmt!("({}, {})", repr_ratio(p.0), repr_ratio(p.1));
	match &g.kind {
		GradientKind::Linear { angle } => {
			r.push_str("gradient.linear(");
			let a = angle.0.rem_euclid(std::f64::consts::TAU);
			let eps = f64::EPSILON;
			if a.abs() < eps {
			} else if (a - std::f64::consts::FRAC_PI_2).abs() < eps {
				r.push_str("dir: rtl, ");
			} else if (a - std::f64::consts::PI).abs() < eps {
				r.push_str("dir: ttb, ");
			} else if (a - 3.0 * std::f64::consts::FRAC_PI_2).abs() < eps {
				r.push_str("dir: btt, ");
			} else {
				r.push_str(&fmt!("angle: {}, ", repr_angle(*angle)));
			}
		}
		GradientKind::Radial { center, radius, focal_center, focal_radius } => {
			r.push_str("gradient.radial(");
			if center.0 .0 != 0.5 || center.1 .0 != 0.5 {
				r.push_str(&fmt!("center: {}, ", pair(*center)));
			}
			if radius.0 != 0.5 {
				r.push_str(&fmt!("radius: {}, ", repr_ratio(*radius)));
			}
			if focal_center != center {
				r.push_str(&fmt!("focal-center: {}, ", pair(*focal_center)));
			}
			if focal_radius.0 != 0.0 {
				r.push_str(&fmt!("focal-radius: {}, ", repr_ratio(*focal_radius)));
			}
		}
		GradientKind::Conic { center, angle } => {
			r.push_str("gradient.conic(");
			if angle.0 != 0.0 {
				r.push_str(&fmt!("angle: {}, ", repr_angle(*angle)));
			}
			if center.0 .0 != 0.5 || center.1 .0 != 0.5 {
				r.push_str(&fmt!("center: {}, ", pair(*center)));
			}
		}
	}
	if g.space != ColorSpace::Oklab {
		r.push_str(&fmt!("space: {}, ", space_fn(g.space).name()));
	}
	match g.relative {
		RelativeTo::Auto	=> (),
		RelativeTo::SelfBox	=> r.push_str("relative: \"self\", "),
		RelativeTo::Parent	=> r.push_str("relative: \"parent\", "),
	}
	let conic = matches!(g.kind, GradientKind::Conic { .. });
	let stops: Vec<String> = g.stops.iter().map(|(c, o)| if conic {
		fmt!("({}, {})", repr_color(c), repr_angle(Angle(o.0 * std::f64::consts::TAU)))
	} else {
		fmt!("({}, {})", repr_color(c), repr_ratio(*o))
	}).collect();
	r.push_str(&stops.join(", "));
	r.push(')');
	r
}

fn with_stops(g: &Gradient, stops: Vec<(Color, Ratio)>) -> Value {
	Value::Gradient(Arc::new(Gradient { kind: g.kind.clone(), stops, space: g.space, relative: g.relative }))
}

fn gradient_call(f: ColorFn, engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	let g = match res!(receiver(&mut args)) {
		Value::Gradient(g)	=> g,
		other				=> return Err(mismatch(engine, span, "gradient", &other)),
	};
	let out = match f {
		ColorFn::GradientStops => Value::array(g.stops.iter()
			.map(|(c, r)| Value::array(vec![Value::Color(*c), Value::Ratio(*r)])).collect()),
		ColorFn::GradientSpace => func(space_fn(g.space)),
		ColorFn::GradientRelative => match g.relative {
			RelativeTo::Auto	=> Value::Auto,
			RelativeTo::SelfBox	=> Value::str("self"),
			RelativeTo::Parent	=> Value::str("parent"),
		},
		ColorFn::GradientKindOf => func(match g.kind {
			GradientKind::Linear { .. }	=> ColorFn::GradientLinear,
			GradientKind::Radial { .. }	=> ColorFn::GradientRadial,
			GradientKind::Conic { .. }	=> ColorFn::GradientConic,
		}),
		ColorFn::GradientAngle => match g.kind {
			GradientKind::Linear { angle } | GradientKind::Conic { angle, .. } => Value::Angle(angle),
			_ => Value::None,
		},
		ColorFn::GradientCenter => match g.kind {
			GradientKind::Radial { center, .. } | GradientKind::Conic { center, .. } =>
				Value::array(vec![Value::Ratio(center.0), Value::Ratio(center.1)]),
			_ => Value::None,
		},
		ColorFn::GradientRadius => match g.kind {
			GradientKind::Radial { radius, .. }	=> Value::Ratio(radius),
			_									=> Value::None,
		},
		ColorFn::GradientFocalCenter => match g.kind {
			GradientKind::Radial { focal_center, .. } =>
				Value::array(vec![Value::Ratio(focal_center.0), Value::Ratio(focal_center.1)]),
			_ => Value::None,
		},
		ColorFn::GradientFocalRadius => match g.kind {
			GradientKind::Radial { focal_radius, .. }	=> Value::Ratio(focal_radius),
			_											=> Value::None,
		},
		ColorFn::GradientSample => {
			let t = res!(need(engine, &mut args, "t"));
			Value::Color(res!(sample_at(engine, span, &g, t)))
		}
		ColorFn::GradientSamples => {
			let ts = res!(args.all::<Value>());
			let mut out = Vec::with_capacity(ts.len());
			for t in ts {
				out.push(Value::Color(res!(sample_at(engine, span, &g, t))));
			}
			Value::array(out)
		}
		ColorFn::GradientSharp => {
			let n = match res!(need(engine, &mut args, "steps")) {
				Value::Int(i)	=> i,
				other			=> return Err(mismatch(engine, span, "integer", &other)),
			};
			let smooth = match res!(args.named::<Value>("smoothness")) {
				None				=> 0.0,
				Some(Value::Ratio(r))	=> r.0,
				Some(other)			=> return Err(mismatch(engine, span, "ratio", &other)),
			};
			if n < 2 {
				return Err(engine.error(span, "sharp gradients must have at least two stops"));
			}
			if !(0.0..=1.0).contains(&smooth) {
				return Err(engine.error(span, "smoothness must be between 0 and 1"));
			}
			let n = n as usize;
			let colors: Vec<Color> = (0..n).flat_map(|i| {
				let c = sample(&g, i as f64 / (n - 1) as f64);
				[c, c]
			}).collect();
			let progress = smooth / (4.0 * n as f64);
			let mut pos = vec![0f64; 2 * n];
			for i in 0..n {
				let j = 2 * i;
				pos[j] = i as f64 / n as f64;
				if j > 0 {
					pos[j] += progress;
				}
				pos[j + 1] = (i + 1) as f64 / n as f64;
				if j + 1 < colors.len() - 1 {
					pos[j + 1] -= progress;
				}
			}
			let stops = colors.into_iter().zip(pos).map(|(c, p)| (c, Ratio(p))).collect();
			with_stops(&g, dedup(stops))
		}
		ColorFn::GradientRepeat => {
			let n = match res!(need(engine, &mut args, "repetitions")) {
				Value::Int(i)	=> i,
				other			=> return Err(mismatch(engine, span, "integer", &other)),
			};
			let mirror = match res!(args.named::<Value>("mirror")) {
				None				=> false,
				Some(Value::Bool(b))	=> b,
				Some(other)			=> return Err(mismatch(engine, span, "boolean", &other)),
			};
			if n <= 0 {
				return Err(engine.error(span, "must repeat at least once"));
			}
			let n = n as usize;
			let mut stops = Vec::new();
			for i in 0..n {
				let mut part: Vec<(Color, Ratio)> = g.stops.iter().map(|(c, o)| {
					if i % 2 == 1 && mirror {
						(*c, Ratio((i as f64 + 1.0 - o.0) / n as f64))
					} else {
						(*c, Ratio((i as f64 + o.0) / n as f64))
					}
				}).collect();
				if i % 2 == 1 && mirror {
					part.reverse();
				}
				stops.extend(part);
			}
			with_stops(&g, dedup(stops))
		}
		_ => Value::None,
	};
	res!(finish(engine, args));
	Ok(out)
}

pub fn call(f: ColorFn, engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	// Colour and gradient share one method table; `space` is both types', every other name one type's.
	let recv_ty = args.items.iter().find(|a| a.name.is_none()).map(|a| a.value.ty());
	let f = match (f, recv_ty) {
		(ColorFn::Space, Some(Type::Gradient))	=> ColorFn::GradientSpace,
		(f, _)									=> f,
	};
	if let Some(ty) = recv_ty {
		let owner = if gradient_method(f.name()) == Some(f) {
			Some(Type::Gradient)
		} else if method(f.name()) == Some(f) {
			Some(Type::Color)
		} else {
			None
		};
		if let Some(owner) = owner {
			if owner != ty {
				return Err(no_method(engine, span, ty, f.name()));
			}
		}
	}
	match f {
		ColorFn::Rgb | ColorFn::Luma | ColorFn::Cmyk | ColorFn::Oklab | ColorFn::Oklch
			| ColorFn::LinearRgb | ColorFn::Hsl | ColorFn::Hsv => {
			let c = res!(construct(f, engine, &mut args));
			res!(finish(engine, args));
			return Ok(Value::Color(c));
		}
		ColorFn::Mix => {
			let space = match res!(args.named::<Value>("space")) {
				None	=> ColorSpace::Oklab,
				Some(v)	=> res!(space_of(engine, span, &v)),
			};
			let vals = res!(args.all::<Value>());
			res!(finish(engine, args));
			let mut cs = Vec::with_capacity(vals.len());
			for v in vals {
				match v {
					Value::Color(c) => cs.push((c, 1.0f32)),
					Value::Array(a) if a.len() == 2 => {
						let c = res!(color_of(engine, span, a[0].clone()));
						let w = match &a[1] {
							Value::Ratio(r)	=> r.0 as f32,
							Value::Float(x)	=> *x as f32,
							Value::Int(i)	=> *i as f32,
							other			=> return Err(mismatch(engine, span, "ratio or float", other)),
						};
						cs.push((c, w));
					}
					other => return Err(mismatch(engine, span, "color or array", &other)),
				}
			}
			return match mix(&cs, space) {
				Ok(c)	=> Ok(Value::Color(c)),
				Err(m)	=> Err(engine.error(span, m)),
			};
		}
		ColorFn::GradientLinear | ColorFn::GradientRadial | ColorFn::GradientConic => {
			return make_gradient(f, engine, args);
		}
		ColorFn::GradientSample | ColorFn::GradientSamples | ColorFn::GradientStops | ColorFn::GradientSharp
			| ColorFn::GradientRepeat | ColorFn::GradientKindOf | ColorFn::GradientSpace
			| ColorFn::GradientRelative | ColorFn::GradientAngle | ColorFn::GradientCenter
			| ColorFn::GradientRadius | ColorFn::GradientFocalCenter | ColorFn::GradientFocalRadius => {
			return gradient_call(f, engine, args);
		}
		_ => (),
	}
	let c = match res!(receiver(&mut args)) {
		Value::Color(c)	=> c,
		other			=> return Err(mismatch(engine, span, "color", &other)),
	};
	let out = match f {
		ColorFn::Lighten | ColorFn::Darken => {
			let x = res!(need(engine, &mut args, "factor"));
			let x = res!(ratio_arg(engine, span, x));
			Value::Color(lighten(&c, x, f == ColorFn::Darken))
		}
		ColorFn::Saturate | ColorFn::Desaturate => {
			let x = res!(need(engine, &mut args, "factor"));
			let x = res!(ratio_arg(engine, span, x));
			if c.space == ColorSpace::Luma {
				let verb = if f == ColorFn::Saturate { "saturate" } else { "desaturate" };
				return Err(engine.error_hint(span, fmt!("cannot {} grayscale color", verb),
					"try converting your color to RGB first"));
			}
			Value::Color(saturate(&c, x, f == ColorFn::Desaturate))
		}
		ColorFn::Negate => {
			let space = match res!(args.named::<Value>("space")) {
				None	=> ColorSpace::Oklab,
				Some(v)	=> res!(space_of(engine, span, &v)),
			};
			Value::Color(negate(&c, space))
		}
		ColorFn::Rotate => {
			let a = res!(need(engine, &mut args, "angle"));
			let deg = res!(angle_deg(engine, span, a));
			let space = match res!(args.named::<Value>("space")) {
				None	=> ColorSpace::Oklch,
				Some(v)	=> res!(space_of(engine, span, &v)),
			};
			let hi = match hue_index(space) {
				Some(h)	=> h,
				None	=> return Err(engine.error(span, "this color space does not support hue rotation")),
			};
			let mut s = to_space(&c, space);
			s.c[hi] = norm_hue(s.c[hi] + deg);
			Value::Color(to_space(&s, c.space))
		}
		ColorFn::Components => {
			let alpha = match res!(args.named::<Value>("alpha")) {
				None				=> true,
				Some(Value::Bool(b))	=> b,
				Some(other)			=> return Err(mismatch(engine, span, "boolean", &other)),
			};
			Value::array(components(&c, alpha))
		}
		ColorFn::Space => func(space_fn(c.space)),
		ColorFn::ToHex => Value::str(to_hex(&c)),
		ColorFn::Transparentize | ColorFn::Opacify => {
			let x = res!(need(engine, &mut args, "scale"));
			let x = res!(ratio_arg(engine, span, x));
			if c.space == ColorSpace::Cmyk {
				return Err(engine.error(span, "CMYK does not have an alpha component"));
			}
			let factor = if f == ColorFn::Transparentize { x } else { -x };
			let mut out = c;
			out.alpha = scale_alpha(c.alpha, factor);
			Value::Color(out)
		}
		_ => Value::None,
	};
	res!(finish(engine, args));
	Ok(out)
}
