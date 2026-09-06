//! Rasterises an SVG file through [`svg_doc`] to a PNG, for eyeballing render fidelity.
//!
//! Usage: `svg_raster <in.svg> <out.png> [width-px]`. The width defaults to 800. The picture is scaled
//! to that width, drawn on a white ground with the same fill/stroke mapping the typesetter uses (a
//! dashed or capped stroke baked to a filled outline), and written out. This is a throwaway verification
//! aid, not part of the crate's public surface.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::colour::Rgba;
use oxedyne_fe2o3_graphics::path::Bounds;
use oxedyne_fe2o3_graphics::pixmap::Pixmap;
use oxedyne_fe2o3_graphics::stroke::Stroke;
use oxedyne_fe2o3_graphics::svg_doc::{
	self,
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
		}
	}

	res!(pm.save_png(&args[2]));
	println!("wrote {} ({}x{}) from {} ops-scaled", args[2], out_w, out_h, args[1]);
	Ok(())
}
