//! Pearlite: a native reader for the `.prl` format `fe2o3_austenite::emit::pearl` writes.
//!
//! Everything that reads or renders a `.prl` already lives in [`oxedyne_fe2o3_austenite::emit::pearl`]
//! (the format's own decoder and SVG reconstruction) and [`oxedyne_fe2o3_graphics`] (the anti-aliased
//! rasteriser and the flat SVG-document reader). This crate adds only what those two do not: a
//! pixmap-driving loop that turns the reconstructed SVG into a PNG at a chosen DPI ([`raster`]), and a
//! loopback app shell that serves the existing browser reader to a local tab so the same one JS reader
//! is the desktop app too ([`shell`]).
//!
//! The reader is two phases: the pixmap-driving raster loop and the loopback app shell. Beside them sits
//! the first increment of a [`collab`] backend -- a signed, mergeable edit stream over a document's
//! annotations, keyed by a stable document identity -- whose transport and browser UI are a later,
//! separately decided increment.

pub mod collab;
pub mod raster;
pub mod shell;

// Phase 3, the native windowed reader (winit + softbuffer). Behind the default-off `gui` feature, the
// sole gate on those two dependencies, so every other path builds without them.
#[cfg(feature = "gui")]
pub mod window;
