//! A 2D graphics library: paths, affine transforms, an anti-aliased rasteriser, pixmaps with
//! alpha compositing, blur and drop shadows, and PNG and JPEG codecs.
//!
//! Painting is not geometry, which is why this crate sits beside `fe2o3_geom` rather than inside
//! it. `fe2o3_geom` serves integer layout, where a rectangle is a cell of a terminal or a widget
//! in a pane. Here a coordinate is a float, a shape is a path of lines and Bezier curves, and the
//! output is a buffer of pixels.
//!
//! The only third-party dependency is `flate2`, for the DEFLATE stream a PNG carries; the CRC-32 a
//! PNG chunk carries is small enough to own outright. Nothing in JPEG is a general-purpose
//! compressor that could sensibly be borrowed, so [`jpeg`] owns the whole of it -- Huffman coding,
//! the discrete cosine transform, chroma resampling and the colour transform alike.
//!
//! # Codecs
//!
//! [`png`] and [`jpeg`] present the same pair of functions over the same [`pixmap::Pixmap`], so a
//! caller that reads pictures need not care which it was handed. JPEG adds two entry points a
//! photograph library wants and PNG has no use for: a size probe that stops at the frame header, and
//! a decode at an eighth scale that reads one coefficient a block and never runs a transform.
//!
//! # Animation
//!
//! [`png::Animation`] writes a sequence of pixmaps as one APNG. Only the rectangle in which a frame
//! differs from the one before it is stored, so a drawing that moves one figure across a still
//! background costs the figure rather than the background, and the file's default image is its first
//! frame, so a reader that knows nothing of animation shows that frame and reports no error. It is
//! not a video codec: there is no motion estimation and no lossy transform, which makes it right for
//! line drawing, flat colour and text and wrong for a photographic sequence.
//!
//! # Containers
//!
//! [`heif`] reads the other side of the same box structure: a HEIC file's items, which of them is
//! the photograph, the grid of tiles it is cut into, where each tile's bytes are, and the Exif
//! block the camera wrote. Reading the container decodes nothing -- what it hands back is a run of
//! bytes and the decoder configuration that describes them -- and it was written before any HEVC
//! decoder existed, because the two things a photograph library needs first, the size and the Exif,
//! are in the container and not in the coded picture. [`heif::decode`] now carries the rest of the
//! way, through [`hevc`] and the assembly of the grid, to a picture.
//!
//! # A container, without a codec
//!
//! [`mp4`] writes an MP4 -- ISO base media file format boxes, a sample table and the media -- around
//! a video track it can neither encode nor decode. That is an odd thing for a graphics crate to
//! hold and it is deliberate: an H.264 encoder is months of rate control, motion estimation and
//! entropy coding at a quality the encoder already in the caller's browser or silicon reaches
//! anyway, while a container is a few hundred lines of length-prefixed boxes with no compression in
//! it, and it is the part that describes the caller's own frames and their timing. So the caller
//! encodes and hands the samples and the decoder configuration over, and gets back a file.
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
//! [`svg`] reads the `d` attribute of an SVG `<path>` -- and writes it back -- and only that. Path
//! data is a small closed grammar and the one part every drawing program agrees on, so it is where a
//! vector mark drawn elsewhere can be let in, or handed back out, without letting in a document
//! format. Elliptical arcs, which the path types have no segment for, become cubic béziers on the
//! way in. The paint a [`stroke::Stroke`] and an [`colour::Rgba`] model is rendered as a `<path>`'s
//! presentation attributes beside its geometry.
//!
//! # Colour and accessibility
//!
//! [`colour`] carries `Rgba` and its compositing, and beside it the WCAG relative luminance and
//! contrast ratio a design is checked for legibility against, and a simulation of the three
//! dichromacies for checking that a palette does not lean on a colour distinction a
//! colour-blind viewer cannot see.
#![forbid(unsafe_code)]

pub mod avi;
pub mod blur;
pub mod colour;
pub mod h264;
pub mod heif;
pub mod hevc;
pub mod jpeg;
pub mod mp4;
pub mod path;
pub mod pixmap;
pub mod png;
pub mod prelude;
pub mod qr;
pub mod raster;
pub mod stroke;
pub mod svg;
pub mod transform;
pub mod yuv;
