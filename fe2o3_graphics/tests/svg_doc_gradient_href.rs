//! A gradient with no stops of its own takes them from the gradient its `xlink:href` (or `href`) names,
//! per SVG 1.1 §13.2.2. Inkscape writes every positioned gradient this way, a stopless
//! `<linearGradient gradientTransform=.. xlink:href="#redBody"/>` over a shared swatch. Read as a
//! gradient with no stops, the fill drew nothing, and the red fire hydrant in the PreSeed AI infographic
//! came out transparent (UNBLOCK-85, 1 Nov 2026).
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::{
	colour::Rgba,
	svg_doc::{
		self,
		SvgOp,
	},
};

/// The mean colour of each shaded path in the document, in document order.
fn fill_colours(src: &str) -> Outcome<Vec<Rgba>> {
	let pic = res!(svg_doc::read_document(src));
	let mut out = Vec::new();
	for op in &pic.ops {
		if let SvgOp::Shade { shading, .. } = op {
			out.push(shading.mean());
		}
	}
	Ok(out)
}

#[test]
fn a_stopless_gradient_inherits_the_stops_its_href_names() -> Outcome<()> {
	// `#a` is a stopless link to `#b`, which links on to the swatch `#s`: the chain resolves to the
	// swatch's mean, (200, 0, 0) and (0, 0, 100) averaged. `#own` has stops of its own, which win over
	// its link. `#loop` and `#pool` name each other with no stops anywhere, and draw nothing.
	let src = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 100 100">
		<defs>
			<linearGradient id="a" x1="0" x2="10" gradientUnits="userSpaceOnUse" xlink:href="#b"/>
			<linearGradient id="s"><stop stop-color="#c80000" offset="0"/><stop stop-color="#000064" offset="1"/></linearGradient>
			<linearGradient id="b" gradientTransform="scale(2 1)" href="#s"/>
			<linearGradient id="own" xlink:href="#s"><stop stop-color="#00ff00" offset="0"/></linearGradient>
			<linearGradient id="loop" xlink:href="#pool"/>
			<linearGradient id="pool" xlink:href="#loop"/>
		</defs>
		<path d="M0 0h10v10h-10z" fill="url(#a)"/>
		<path d="M20 0h10v10h-10z" fill="url(#own)"/>
		<path d="M40 0h10v10h-10z" fill="url(#loop)"/>
	</svg>"##;
	let c = res!(fill_colours(src));
	let want = vec![Rgba::new(100, 0, 50, 255), Rgba::new(0, 255, 0, 255)];
	if c != want {
		return Err(err!("Fills read {:?}, expected {:?}", c, want; Invalid, Output));
	}
	Ok(())
}
