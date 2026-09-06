//! Rasterises an SVG file through [`svg_doc`] to a PNG, for eyeballing render fidelity.
//!
//! Usage: `svg_raster <in.svg> <out.png> [width-px]`. The width defaults to 800. The picture is scaled
//! to that width, drawn on a white ground with the same fill/stroke mapping the typesetter uses (a
//! dashed or capped stroke baked to a filled outline), and written out. This is a throwaway verification
//! aid, not part of the crate's public surface.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::colour::Rgba;
use oxedyne_fe2o3_graphics::path::{
	Bounds,
	PathBuilder,
	Pt,
};
use oxedyne_fe2o3_graphics::pixmap::Pixmap;
use oxedyne_fe2o3_graphics::stroke::Stroke;
use oxedyne_fe2o3_graphics::svg_doc::{
	self,
	Anchor,
	SvgOp,
};
use oxedyne_fe2o3_graphics::transform::Transform;

fn main() -> Outcome<()> {
	let args: Vec<String> = std::env::args().collect();
	if args.len() < 3 {
		println!("usage: svg_raster <in.svg> <out.png> [width-px]");
		return Ok(());
	}
	let src		= res!(std::fs::read_to_string(&args[1]));
	let out_w	= args.get(3).and_then(|s| s.parse::<usize>().ok()).unwrap_or(800);

	let pic		= res!(svg_doc::read_document(&src));
	let s		= (out_w as f32) / pic.width.max(1.0);
	let out_h	= ((pic.height * s).ceil() as usize).max(1);
	let t		= Transform::scale(s, s);

	let mut pm = res!(Pixmap::new(out_w, out_h));
	res!(pm.fill_bounds(Bounds::new(0.0, 0.0, out_w as f32, out_h as f32), Rgba::WHITE, None));

	// Text runs are handed on unshaped, since svg_doc has no font, and this raster has none either.
	// A run is marked with a baseline rule rather than dropped, so the eyeball check sees where text
	// sits without mistaking a shaped-glyph gap for lost content; the tally is reported at the end.
	let mut texts = 0usize;

	for op in pic.ops {
		match op {
			SvgOp::Fill { path, colour } => {
				res!(pm.fill_path(&path, &t, colour, None));
			},
			SvgOp::Stroke { path, colour, stroke } => {
				if stroke.dash.is_some() {
					let outline = res!(path.stroke(&stroke));
					res!(pm.fill_path(&outline, &t, colour, None));
				} else {
					let pen = res!(Stroke::new(stroke.width));
					let pen = pen
						.with_cap(stroke.cap)
						.with_join(stroke.join);
					res!(pm.stroke_path(&path, &t, colour, None, &pen));
				}
			},
			SvgOp::Text { text, local, x, y, size, anchor, colour, .. } => {
				texts += 1;
				// A rough advance: half an em a character is a fair mean for an unshaped run, enough
				// to place a baseline marker of about the right length. The run's frame is `local`;
				// the endpoints are mapped into the picture frame and stroked through `t` like any
				// other path, so the marker lands where the glyphs would.
				let adv = (text.chars().count() as f32) * size * 0.5;
				let (x0, x1) = match anchor {
					Anchor::Start	=> (x, x + adv),
					Anchor::Middle	=> (x - 0.5 * adv, x + 0.5 * adv),
					Anchor::End	=> (x - adv, x),
				};
				let a = local.apply(Pt::new(x0, y));
				let b = local.apply(Pt::new(x1, y));
				let mut pb = PathBuilder::new();
				pb.move_to(a);
				pb.line_to(b);
				let rule = res!(pb.finish());
				let pen = res!(Stroke::new((size * 0.06).max(0.5)));
				// Half coverage, so the marker reads as a placeholder rather than as rendered text.
				res!(pm.stroke_path(&rule, &t, colour.with_coverage(0.5), None, &pen));
			},
			SvgOp::Image { rgba, iw, ih, x, y, w, h } => {
				if iw == 0 || ih == 0 || w <= 0.0 || h <= 0.0 {
					continue;
				}
				let img = res!(Pixmap::from_data(iw, ih, rgba));
				// The placement rectangle, carried into device pixels by the same scale the paths use.
				let dw = (w * s).max(1.0);
				let dh = (h * s).max(1.0);
				let ox = x * s;
				let oy = y * s;
				let px0 = ox.floor().max(0.0) as usize;
				let py0 = oy.floor().max(0.0) as usize;
				let px1 = ((ox + dw).ceil() as usize).min(out_w);
				let py1 = ((oy + dh).ceil() as usize).min(out_h);
				// Nearest-neighbour: enough to eyeball fidelity, and it pulls in no resampler.
				for py in py0..py1 {
					for px in px0..px1 {
						let u = (((px as f32) + 0.5 - ox) / dw) * (iw as f32);
						let v = (((py as f32) + 0.5 - oy) / dh) * (ih as f32);
						if u < 0.0 || v < 0.0 {
							continue;
						}
						let sx = (u as usize).min(iw - 1);
						let sy = (v as usize).min(ih - 1);
						if let Some(c) = img.pixel(sx, sy) {
							pm.blend_pixel(px, py, c);
						}
					}
				}
			},
		}
	}

	res!(pm.save_png(&args[2]));
	println!("wrote {} ({}x{}) from {} ops-scaled", args[2], out_w, out_h, args[1]);
	if texts > 0 {
		println!("{} text run(s) marked at the baseline but not shaped (no font in this raster)", texts);
	}
	Ok(())
}
