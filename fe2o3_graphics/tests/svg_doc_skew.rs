//! An SVG `skewX`/`skewY` transform shears the outline it frames. Inkscape writes them for isometric
//! labels; read as the identity, the PreSeed AI infographic's "WEIGHTS" and white "COMPUTE" labels on
//! the server were drawn unsheared, far from their places (30 Oct 2026).
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::{
	path::Bounds,
	svg_doc::{
		self,
		SvgOp,
	},
	transform::Transform,
};

/// The bounds of each filled path in the document, in the picture frame.
fn fills(src: &str) -> Outcome<Vec<Bounds>> {
	let pic = res!(svg_doc::read_document(src));
	let mut out = Vec::new();
	for op in &pic.ops {
		if let SvgOp::Fill { path, .. } = op {
			out.push(res!(path.bounds(&Transform::IDENTITY).ok_or_else(|| err!("Empty fill"; Missing))));
		}
	}
	Ok(out)
}

fn near(a: f32, b: f32) -> bool {
	(a - b).abs() < 1e-3
}

#[test]
fn a_skew_y_and_a_skew_x_shear_their_outlines() -> Outcome<()> {
	let src = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
		<path transform="skewY(-30)" d="M10 50h10v10h-10z"/>
		<path transform="skewX(45)" d="M0 10h10v10h-10z"/>
	</svg>"#;
	let b = res!(fills(src));
	if b.len() != 2 {
		return Err(err!("Expected 2 fills, read {}", b.len(); Invalid, Output));
	}
	// skewY(-30): y' = y - tan(30°) x, so x in [10, 20] lifts the box's top from 50 to 50 - 11.547.
	let t = 30f32.to_radians().tan();
	let (y0, y1) = (50.0 - 20.0 * t, 60.0 - 10.0 * t);
	if !(near(b[0].x0, 10.0) && near(b[0].x1, 20.0) && near(b[0].y0, y0) && near(b[0].y1, y1)) {
		return Err(err!("skewY(-30) box is {:?}, expected x 10..20, y {}..{}", b[0], y0, y1; Invalid, Output));
	}
	// skewX(45): x' = x + y, so the box spans x 10..30 at y 10..20.
	if !(near(b[1].x0, 10.0) && near(b[1].x1, 30.0) && near(b[1].y0, 10.0) && near(b[1].y1, 20.0)) {
		return Err(err!("skewX(45) box is {:?}, expected x 10..30, y 10..20", b[1]; Invalid, Output));
	}
	Ok(())
}
