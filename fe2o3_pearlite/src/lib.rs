//! Pearlite: a native reader for the `.prl` format `fe2o3_austenite::emit::pearl` writes.
//!
//! Everything that reads or renders a `.prl` already lives in [`oxedyne_fe2o3_austenite::emit::pearl`]
//! (the format's own decoder and SVG reconstruction) and [`oxedyne_fe2o3_graphics`] (the anti-aliased
//! rasteriser and the flat SVG-document reader). This crate adds only what those two do not: a
//! pixmap-driving loop that turns the reconstructed SVG into a PNG at a chosen DPI ([`raster`]), and a
//! loopback app shell that serves the existing browser reader to a local tab so the same one JS reader
//! is the desktop app too ([`shell`]).
//!
//! Two phases only. A true OS window (rather than a browser tab) and any collaboration feature are later,
//! separately decided work -- not attempted here.

pub mod raster;
pub mod shell;
