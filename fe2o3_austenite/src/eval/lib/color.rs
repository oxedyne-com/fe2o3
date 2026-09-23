// U3 owns this file. Colour constructors and methods, gradients, and the named colours (`red`, `luma`...).
// `Color::to_rgba` is the one conversion flow and the emitters use.

use crate::eval::args::Args;
use crate::eval::func::unimplemented;
use crate::eval::scope::Scope;
use crate::eval::value::Value;
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

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
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn method(_name: &str) -> Option<ColorFn> { None }

pub fn call(f: ColorFn, _engine: &mut Engine, _args: Args) -> Outcome<Value> {
	Err(unimplemented("color", f.name()))
}

impl crate::eval::value::Color {
	/// The colour as eight-bit sRGB with alpha, for the back end.
	pub fn to_rgba(&self) -> Outcome<oxedyne_fe2o3_graphics::colour::Rgba> {
		Err(unimplemented("color", "to-rgba"))
	}
}
