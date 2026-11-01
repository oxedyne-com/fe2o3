//! A gradient fill reads as a shading that varies across its shape, not as the flat mean of its stops
//! (UNBLOCK-85): the PreSeed AI infographic's water jet (`userSpaceOnUse` under group transforms) and the
//! fire hydrant's red (stopless Inkscape links carrying a `gradientTransform`) are the cases in hand.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::{
	colour::{
		Rgba,
		Shading,
	},
	path::Pt,
	svg_doc::{
		self,
		SvgOp,
	},
};

fn shades(src: &str) -> Outcome<Vec<Shading>> {
	let pic = res!(svg_doc::read_document(src));
	let mut out = Vec::new();
	for op in &pic.ops {
		if let SvgOp::Shade { shading, .. } = op {
			out.push(shading.clone());
		}
	}
	Ok(out)
}

fn near(got: Rgba, want: Rgba, what: &str) -> Outcome<()> {
	let d = |a: u8, b: u8| (a as i32 - b as i32).abs();
	if d(got.r, want.r) > 2 || d(got.g, want.g) > 2 || d(got.b, want.b) > 2 || d(got.a, want.a) > 2 {
		return Err(err!("{}: read {:?}, expected {:?}", what, got, want; Invalid, Output));
	}
	Ok(())
}

fn one(src: &str) -> Outcome<Shading> {
	let s = res!(shades(src));
	if s.len() != 1 {
		return Err(err!("Expected one Shade op, read {}", s.len(); Invalid, Output));
	}
	Ok(s[0].clone())
}

const RED:	Rgba = Rgba { r: 255, g: 0, b: 0, a: 255 };
const BLUE:	Rgba = Rgba { r: 0, g: 0, b: 255, a: 255 };
const MID:	Rgba = Rgba { r: 128, g: 0, b: 128, a: 255 };

#[test]
fn a_user_space_gradient_under_a_group_transform_shades_its_shape() -> Outcome<()> {
	let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
		<defs><linearGradient id="j" x1="0" x2="100" y1="0" y2="0" gradientUnits="userSpaceOnUse"
			gradientTransform="scale(0.5 1)">
			<stop offset="0" stop-color="#ff0000"/><stop offset="1" stop-color="#0000ff"/>
		</linearGradient></defs>
		<g transform="translate(10 0)"><rect x="0" y="0" width="100" height="10" fill="url(#j)"/></g>
	</svg>"##;
	let s = res!(one(src));
	res!(near(res!(s.colour_at(Pt::new(10.0, 5.0))), RED, "start"));
	res!(near(res!(s.colour_at(Pt::new(35.0, 5.0))), MID, "middle"));
	res!(near(res!(s.colour_at(Pt::new(80.0, 5.0))), BLUE, "padded past the end"));
	Ok(())
}

#[test]
fn a_stopless_link_inherits_stops_and_keeps_its_own_geometry() -> Outcome<()> {
	// As Inkscape writes the hydrant: a swatch with stops, and a positioned stopless link over it.
	let src = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 100 100">
		<defs>
			<linearGradient id="sw"><stop offset="0" stop-color="#ff0000"/><stop offset="1" stop-color="#0000ff"/></linearGradient>
			<linearGradient id="l" x1="0" x2="0" y1="0" y2="20" gradientTransform="scale(1 2)"
				gradientUnits="userSpaceOnUse" xlink:href="#sw"/>
		</defs>
		<rect x="0" y="0" width="10" height="40" fill="url(#l)"/>
	</svg>"##;
	let s = res!(one(src));
	res!(near(res!(s.colour_at(Pt::new(5.0, 0.0))), RED, "top"));
	res!(near(res!(s.colour_at(Pt::new(5.0, 20.0))), MID, "middle"));
	res!(near(res!(s.colour_at(Pt::new(5.0, 40.0))), BLUE, "bottom"));
	Ok(())
}

#[test]
fn a_bounding_box_gradient_follows_its_shape_and_inherits_along_href() -> Outcome<()> {
	// x2="0" y2="1" on the swatch, as #redNoz carries it; the link inherits that geometry too.
	let src = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 100 100">
		<defs>
			<linearGradient id="n" x2="0" y2="1"><stop offset="0" stop-color="#ff0000"/><stop offset="1" stop-color="#0000ff"/></linearGradient>
			<linearGradient id="k" xlink:href="#n"/>
		</defs>
		<rect x="20" y="10" width="10" height="40" fill="url(#k)" fill-opacity="0.5"/>
	</svg>"##;
	let s = res!(one(src));
	let half = |c: Rgba| Rgba { a: 128, ..c };
	res!(near(res!(s.colour_at(Pt::new(25.0, 10.0))), half(RED), "top"));
	res!(near(res!(s.colour_at(Pt::new(25.0, 30.0))), half(MID), "middle"));
	res!(near(res!(s.colour_at(Pt::new(25.0, 50.0))), half(BLUE), "bottom"));
	Ok(())
}

#[test]
fn a_radial_gradient_defaults_to_the_box_centre_and_half_its_size() -> Outcome<()> {
	let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
		<defs><radialGradient id="r"><stop offset="0" stop-color="#ff0000"/><stop offset="1" stop-color="#0000ff"/></radialGradient></defs>
		<rect x="0" y="0" width="40" height="20" fill="url(#r)"/>
	</svg>"##;
	let s = res!(one(src));
	res!(near(res!(s.colour_at(Pt::new(20.0, 10.0))), RED, "centre"));
	res!(near(res!(s.colour_at(Pt::new(30.0, 10.0))), MID, "half way out along x"));
	res!(near(res!(s.colour_at(Pt::new(20.0, 15.0))), MID, "half way out along y, the box ellipse"));
	res!(near(res!(s.colour_at(Pt::new(40.0, 10.0))), BLUE, "edge"));
	Ok(())
}

#[test]
fn a_gradient_stroke_shades_the_stroked_outline() -> Outcome<()> {
	let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
		<defs><linearGradient id="g" x1="0" x2="100" gradientUnits="userSpaceOnUse">
			<stop offset="0" stop-color="#ff0000"/><stop offset="1" stop-color="#0000ff"/></linearGradient></defs>
		<line x1="0" y1="50" x2="100" y2="50" stroke="url(#g)" stroke-width="4" fill="none"/>
	</svg>"##;
	let s = res!(one(src));
	res!(near(res!(s.colour_at(Pt::new(50.0, 50.0))), MID, "middle of the stroke"));
	Ok(())
}
