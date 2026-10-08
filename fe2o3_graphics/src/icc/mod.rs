//! ICC profiles: a reader of version 2 profiles, the matrix path, and two spaces built in.
//!
//! [`read`] parses a profile's header, its tag table and the tags a transform is built from, with
//! every read bounded by the tag's declared size and the file's length. [`matrix`] carries the
//! RGB-to-XYZ path through colorants and curves, and RGB to grey through a grey curve's inverse.
//! [`builtin`] holds sRGB and sGray as formulas.
//!
//! # The bundled profile
//!
//! With the `fogra39` feature on, [`FOGRA39L`] holds the bytes of FOGRA39L Coated (122 KB, CC0,
//! from colord; `data/icc/LICENCE.txt`) and [`fogra39l`] reads them. The feature is off by default,
//! and a build for `wasm32` leaves it off, so a wasm package carries no profile bytes.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

pub mod builtin;
pub mod matrix;
pub mod read;

#[cfg(feature = "fogra39")]
use oxedyne_fe2o3_core::prelude::*;

#[cfg(feature = "fogra39")]
pub const FOGRA39L: &[u8] = include_bytes!("../../data/icc/FOGRA39L_coated.icc");

/// Reads the bundled FOGRA39L Coated profile.
#[cfg(feature = "fogra39")]
pub fn fogra39l() -> Outcome<read::Profile> {
	read::Profile::read("FOGRA39L_coated.icc (bundled)", FOGRA39L)
}
