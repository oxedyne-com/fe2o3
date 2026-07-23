//! The types a caller almost always wants.

pub use crate::{
	blur::Shadow,
	colour::{
		ColourVision,
		Rgba,
	},
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
