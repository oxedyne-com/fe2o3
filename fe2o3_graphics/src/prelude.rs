//! The types a caller almost always wants.

pub use crate::{
	blur::Shadow,
	colour::{
		ColourVision,
		Rgba,
	},
	jpeg::{
		Chroma,
		Options as JpegOptions,
	},
	path::{
		Bounds,
		Path,
		PathBuilder,
		Polyline,
		Pt,
	},
	pixmap::Pixmap,
	png::{
		Animation,
		Delay,
	},
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
