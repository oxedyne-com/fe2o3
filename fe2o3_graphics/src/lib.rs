//! A 2D graphics library: paths, affine transforms, an anti-aliased rasteriser, pixmaps with
//! alpha compositing, blur and drop shadows, and a PNG codec.
//!
//! Painting is not geometry, which is why this crate sits beside `fe2o3_geom` rather than inside
//! it. `fe2o3_geom` serves integer layout, where a rectangle is a cell of a terminal or a widget
//! in a pane. Here a coordinate is a float, a shape is a path of lines and Bezier curves, and the
//! output is a buffer of pixels.
//!
//! The only third-party dependency is `flate2`, for the DEFLATE stream a PNG carries; the CRC-32 a
//! PNG chunk carries is small enough to own outright.
//!
//! # The rasteriser
//!
//! [`raster`] accumulates the signed area each edge contributes to each pixel, then takes a prefix
//! sum along every row. This gives exact analytic anti-aliasing, with no supersampling, for a path
//! whose contours do not overlap, and either the non-zero winding rule or the even-odd rule where
//! they do. Non-zero is the default, and is what glyph outlines and filled boxes both want.
//!
//! # Stroking
//!
//! [`stroke`] adds no rasteriser code at all, because a stroke is only the fill of a different
//! shape: the region the pen sweeps as it travels the path. It builds that region as a [`path::Path`]
//! and hands it back to the filler.
//!
//! # Blurring
//!
//! [`blur`] adds none either. Three passes of a sliding box, along each axis, stand in for a
//! Gaussian to within a few percent, at a cost that is the same whatever the radius. A drop shadow
//! is then only a silhouette filled into a scratch pixmap, blurred, and composited back. The blur
//! runs on premultiplied alpha, without which the colour of the clear pixels a shape is blurred
//! against would bleed into it and fringe it with dirt.
//!
//! # SVG path data
//!
//! [`svg`] reads the `d` attribute of an SVG `<path>` -- and only that. Path data is a small closed
//! grammar and the one part every drawing program agrees on, so it is where a vector mark drawn
//! elsewhere can be let in without letting in a document format. Elliptical arcs, which the path
//! types have no segment for, become cubic béziers on the way in.
#![forbid(unsafe_code)]

pub mod blur;
pub mod colour;
pub mod path;
pub mod pixmap;
pub mod png;
pub mod prelude;
pub mod qr;
pub mod raster;
pub mod stroke;
pub mod svg;
pub mod transform;
