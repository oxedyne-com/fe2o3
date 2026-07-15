//! The types a caller almost always wants.

pub use crate::{
	colour::Rgba,
	path::{
		Bounds,
		Path,
		PathBuilder,
		Polyline,
		Pt,
	},
	pixmap::Pixmap,
	qr::{
		QrEcc,
		QrMatrix,
	},
	raster::FillRule,
	stroke::{
		Cap,
		Dash,
		Join,
		Stroke,
	},
	transform::Transform,
};
