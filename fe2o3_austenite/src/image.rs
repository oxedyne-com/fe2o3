//! External image loading for figures.
//!
//! A `#figure(...)` body may be an `image("...")` or `padded-image("...")` call naming a file in the
//! book's asset tree. This module resolves that path against the book root and loads it by type: a
//! raster (PNG or JPEG) is decoded to straight RGBA through `fe2o3_graphics`'s [`Pixmap`] and handed to
//! the block layer as a [`RasterImage`] it wraps in a [`DrawOp::Image`](crate::ir::DrawOp); an SVG is
//! read as native vectors through `fe2o3_graphics`'s [`svg_doc`](oxedyne_fe2o3_graphics::svg_doc) into an
//! [`SvgPicture`] the block layer scales to the figure width and maps to filled and stroked ops. The
//! typesetter's SVG bakes its text to glyph outlines, so no font is needed to read it back.
//!
//! [`load_figure`] is the type-dispatching entry the figure path uses. [`load`] remains for the cover and
//! logo, which are rasters; it still serves an SVG there from a same-stem raster, and where none exists
//! the caller keeps its placeholder.
//!
//! The book root is not threaded through the block layer -- the reader sets one file with no notion of
//! where the tree lives -- so the binary records it once with [`set_base_dir`] before authoring, and the
//! render reads it back here. A source path is Typst-root-relative (`/assets/...`); it is resolved
//! against the recorded base and, failing that, a couple of enclosing directories, so a chapter compiled
//! on its own finds the assets through the book's `assets` symlink just as a whole-book compile does.

use crate::ir::RasterImage;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::pixmap::Pixmap;
use oxedyne_fe2o3_graphics::svg_doc::{
	self,
	SvgPicture,
};

use std::path::{
	Path,
	PathBuf,
};
use std::sync::RwLock;

// The book root the render resolves image paths against, set once by the binary before authoring. A
// process-global rather than a threaded argument because the block layer, which sets one file at a time,
// carries no path of its own.
static BASE: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Records the directory image paths resolve against -- the book's source directory, whose `assets`
/// entry (a real tree or a symlink to the shared one) roots the `/assets/...` paths the figures name.
pub fn set_base_dir(dir: PathBuf) -> Outcome<()> {
	let mut guard = lock_write!(BASE, "While recording the image base directory");
	*guard = Some(dir);
	Ok(())
}

/// The recorded base directory, or `None` when the binary set none.
fn base_dir() -> Outcome<Option<PathBuf>> {
	let guard = lock_read!(BASE, "While reading the image base directory");
	Ok(guard.clone())
}

/// Resolves a Typst image path to a file on disk, or `None` when none of the candidate roots holds it.
///
/// A leading `/` makes the path root-relative in Typst, not filesystem-absolute, so it is stripped and
/// tried against the recorded base and two of its ancestors; a plain relative path is tried against the
/// base and the working directory. The first candidate that exists wins.
pub fn resolve(src: &str) -> Outcome<Option<PathBuf>> {
	let rel			= src.trim();
	let stripped	= rel.strip_prefix('/').unwrap_or(rel);
	let mut cands:	Vec<PathBuf> = Vec::new();
	if let Some(base) = res!(base_dir()) {
		cands.push(base.join(stripped));
		if let Some(p) = base.parent() {
			cands.push(p.join(stripped));
			if let Some(pp) = p.parent() {
				cands.push(pp.join(stripped));
			}
		}
	}
	cands.push(PathBuf::from(stripped));
	Ok(cands.into_iter().find(|p| p.exists()))
}

/// A loaded figure: a decoded raster, or an SVG read as a resolution-independent [`SvgPicture`] the
/// caller sizes and maps to drawing ops. The two are kept apart because a raster fills a rectangle and a
/// vector carries its own paths, and the block layer draws them by different routes.
pub enum Figure {
	Raster(RasterImage),
	Vector(SvgPicture),
}

/// Loads a figure by type: an SVG read as native vectors, a PNG or JPEG decoded to a raster. The path is
/// resolved against the book root, and a type that is neither is an error the caller turns into a
/// placeholder.
pub fn load_figure(src: &str) -> Outcome<Figure> {
	let path = res!(res!(resolve(src)).ok_or_else(|| err!(
		"Could not resolve the figure image path {:?} against the book root.", src;
		Input, Missing, File)));
	let ext = path.extension()
		.and_then(|e| e.to_str())
		.unwrap_or("")
		.to_lowercase();
	if ext == "svg" {
		let src = match std::fs::read_to_string(&path) {
			Ok(s)	=> s,
			Err(e)	=> return Err(err!(e, "Could not read the SVG figure {:?}.", path; File, Read)),
		};
		return Ok(Figure::Vector(res!(svg_doc::read_document(&src))));
	}
	Ok(Figure::Raster(res!(load_file(&path))))
}

/// Loads the raster a figure names: a PNG or JPEG decoded straight, or -- for an SVG, which has no
/// reader here -- the same-stem raster the book ships beside it. An SVG with no such raster, or a path
/// that resolves to nothing, is an error the caller turns back into a placeholder.
pub fn load(src: &str) -> Outcome<RasterImage> {
	let path = res!(res!(resolve(src)).ok_or_else(|| err!(
		"Could not resolve the figure image path {:?} against the book root.", src;
		Input, Missing, File)));
	load_file(&path)
}

/// Loads one resolved file: a raster decoded, an SVG served by its same-stem raster, anything else
/// refused.
fn load_file(path: &Path) -> Outcome<RasterImage> {
	let ext = path.extension()
		.and_then(|e| e.to_str())
		.unwrap_or("")
		.to_lowercase();
	match ext.as_str() {
		"png" | "jpg" | "jpeg"	=> decode_raster(path),
		"svg" => {
			// No SVG document reader exists in the workspace; the books ship a same-stem raster beside
			// each vector figure, so that is loaded in its place. A missing one is reported, not guessed.
			for alt in ["png", "jpg", "jpeg"] {
				let raster = path.with_extension(alt);
				if raster.exists() {
					return decode_raster(&raster);
				}
			}
			Err(err!(
				"The figure image {:?} is an SVG, which has no reader here, and no same-stem raster \
				sits beside it to load instead.", path; Input, Invalid, Missing))
		},
		other => Err(err!(
			"The figure image {:?} has an unsupported type {:?}.", path, other; Input, Invalid)),
	}
}

/// Decodes a PNG or JPEG file to straight RGBA, choosing the decoder by the file's own magic bytes and
/// falling back to its extension.
fn decode_raster(path: &Path) -> Outcome<RasterImage> {
	let bytes = match std::fs::read(path) {
		Ok(b)	=> b,
		Err(e)	=> return Err(err!(e, "Could not read the image file {:?}.", path; File, Read)),
	};
	let pm = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
		res!(Pixmap::from_png(&bytes))
	} else if bytes.starts_with(&[0xFF, 0xD8]) {
		res!(Pixmap::from_jpeg(&bytes))
	} else {
		let ext = path.extension()
			.and_then(|e| e.to_str())
			.unwrap_or("")
			.to_lowercase();
		match ext.as_str() {
			"png"			=> res!(Pixmap::from_png(&bytes)),
			"jpg" | "jpeg"	=> res!(Pixmap::from_jpeg(&bytes)),
			_ => return Err(err!(
				"The image {:?} is neither PNG nor JPEG by its bytes or its extension.", path;
				Input, Invalid)),
		}
	};
	Ok(RasterImage { width: pm.width(), height: pm.height(), rgba: pm.into_data() })
}

/// Splits straight RGBA into the packed RGB the image XObject and `<image>` writers want, and a grey
/// soft mask when any sample is translucent. An all-opaque image returns `None` for the mask, so the
/// common case carries no extra channel.
pub fn split_rgba(img: &RasterImage) -> (Vec<u8>, Option<Vec<u8>>) {
	let n			= img.width * img.height;
	let mut rgb		= Vec::with_capacity(n * 3);
	let mut alpha	= Vec::with_capacity(n);
	let mut any		= false;
	for px in img.rgba.chunks_exact(4) {
		rgb.push(px[0]);
		rgb.push(px[1]);
		rgb.push(px[2]);
		alpha.push(px[3]);
		if px[3] != 255 {
			any = true;
		}
	}
	(rgb, if any { Some(alpha) } else { None })
}
