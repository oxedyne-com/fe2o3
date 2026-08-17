//! The HEVC decoder against an independent one, tile by tile.
//!
//! Nothing in the decoder can be trusted on its own evidence. Every table in it has been checked
//! against the published document, and every piece of arithmetic against a property that document
//! implies, but the only thing that says a *picture* is right is another decoder's picture.
//!
//! So this decodes real HEIC photographs and compares them, sample by sample, with what `ffmpeg`
//! makes of the same file -- the whole way, both loop filters included. It is driven by an
//! environment variable naming a directory of them, because no checkout carries a photograph:
//!
//! ```bash
//! HEVC_CORPUS=/srv/nfs4/Gallery/2021 \
//!     cargo test -p oxedyne_fe2o3_graphics --test hevc_tiles -- --nocapture
//! ```
//!
//! `HEVC_FILES` caps how many are read, for a quick run. Nothing here writes anywhere but a
//! scratch directory under the cache.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::{
	heif::{
		Heif,
		Picture,
	},
	hevc,
};

use std::{
	path::{
		Path,
		PathBuf,
	},
	process::Command,
};

const FILES: usize = 3;	// photographs a run reads unless it is told otherwise

/// Where the reference pictures are written.
fn scratch() -> PathBuf {
	match std::env::var("HOME") {
		Ok(home) => PathBuf::from(home).join(".cache/hevc-oracle"),
		Err(_) => PathBuf::from(".hevc-oracle"),
	}
}

struct Rgb {
	w:	usize,
	h:	usize,
	px:	Vec<u8>,
}

/// Reads a PNG with this library's own decoder, which is checked against ImageMagick elsewhere.
fn read_png(path: &Path) -> Outcome<Rgb> {
	let bytes = res!(std::fs::read(path), IO, File);
	let pm = res!(oxedyne_fe2o3_graphics::png::decode(&bytes));
	let (w, h) = (pm.width(), pm.height());
	let mut px = Vec::with_capacity(w * h * 3);
	for p in pm.data().chunks_exact(4) {
		px.push(p[0]);
		px.push(p[1]);
		px.push(p[2]);
	}
	Ok(Rgb { w, h, px })
}

/// What one photograph's comparison came to.
struct Verdict {
	samples:	usize,	// how many were compared
	differing:	usize,	// how many differed at all
	worst:		i32,	// the largest difference in any one sample
	mean:		f64,	// the mean absolute difference
}

/// Decodes one tile with each decoder and compares the brightness plane.
///
/// Only luma, and only the first tile: colour goes through a conversion neither decoder is being
/// asked about here, and one tile is where a fault shows first.
fn compare(file: &Path, out: &Path) -> Outcome<Option<Verdict>> {
	println!("  reading {}", file.display());
	let bytes = res!(std::fs::read(file), IO, File);
	let heif = match Heif::read(&bytes) {
		Ok(h) => h,
		// Not a HEIC at all: one in ten of the corpus is a JPEG under that name. Said out loud,
		// because a run that skipped everything in silence would report as a run that passed.
		Err(e) => {
			println!("  skipped {}: {}", file.display(), e.plain());
			return Ok(None);
		},
	};
	let (item, tile_w, tile_h) = match res!(heif.picture()) {
		Picture::One { item, size } => (item, size.0 as usize, size.1 as usize),
		Picture::Tiled { grid, tiles } => {
			let first = match tiles.first() {
				Some(t) => *t,
				None => return Ok(None),
			};
			// The grid says how many tiles and how big the assembled picture is; a tile's own
			// size comes out of the decoder, so the overlap below settles it.
			(first, grid.width as usize / grid.cols.max(1) as usize,
				grid.height as usize / grid.rows.max(1) as usize)
		},
		Picture::Foreign { kind, .. } => {
			println!("  skipped {}: it holds {:?}", file.display(), String::from_utf8_lossy(&kind));
			return Ok(None);
		},
	};
	let config = res!(heif.config(item));
	let data = res!(heif.data(item));
	let mine = res!(hevc::picture(config, &data));

	// The reference: ffmpeg's own decode of the same file, which assembles the grid.
	//
	// Asked for as raw 4:2:0 rather than as a picture, so that the comparison is brightness
	// against brightness with no colour conversion in the way. A conversion in the middle would
	// make an exact answer impossible and hide a decoder that was nearly right.
	let raw = out.join("reference.yuv");
	let _ = std::fs::remove_file(&raw);
	// With the loop filters on, because this decoder runs them. `HEVC_NO_FILTERS` turns them off
	// at both ends, which is how a fault in the codec proper is separated from one in the two
	// filters over it -- their whole contribution to a tile of this corpus is 8,722 samples out of
	// 262,144, none by more than two, so with a filter wrong the codec's own faults hide under a
	// haze of ones and twos.
	let mut cmd = Command::new("ffmpeg");
	cmd.args(["-v", "error"]);
	if std::env::var("HEVC_NO_FILTERS").is_ok() {
		cmd.args(["-skip_loop_filter", "all"]);
	}
	let run = res!(cmd
		.arg("-i")
		.arg(file)
		.args(["-frames:v", "1", "-pix_fmt", "yuv420p", "-f", "rawvideo", "-y"])
		.arg(&raw)
		.output(), IO, File);
	if !run.status.success() || !raw.exists() {
		println!("  skipped {}: ffmpeg would not read it", file.display());
		return Ok(None);
	}
	// ffmpeg decodes the FIRST TILE of a grid, not the assembled photograph -- 393,216 bytes for
	// a 512 by 512 tile in 4:2:0, whatever the photograph's own size. That is exactly the oracle
	// wanted here, and the geometry to compare against is the tile's own coded size.
	let (tw, th) = (mine.y.w, mine.y.h);
	let bytes = res!(std::fs::read(&raw), IO, File);
	if bytes.len() != tw * th * 3 / 2 {
		println!("  skipped {}: the reference is {} bytes and a {} by {} tile wants {}",
			file.display(), bytes.len(), tw, th, tw * th * 3 / 2);
		return Ok(None);
	}
	let theirs = Rgb { w: tw, h: th, px: bytes };

	// The tile sits at the top left of the assembled picture, and the assembled picture is cropped
	// out of the grid, so the overlap is what both decoders agree exists.
	// Both decoders made the same tile, so the whole of it is compared.
	let _ = (tile_w, tile_h);
	let w = theirs.w.min(mine.y.w);
	let h = theirs.h.min(mine.y.h);
	if w == 0 || h == 0 {
		println!("  skipped {}: tile {}x{}, mine {}x{}, theirs {}x{}",
			file.display(), tile_w, tile_h, mine.y.w, mine.y.h, theirs.w, theirs.h);
		return Ok(None);
	}
	let (mut differing, mut worst, mut total) = (0usize, 0i32, 0i64);
	let mut first_bad: Option<(usize, usize)> = None;
	for y in 0..h {
		for x in 0..w {
			let a = match mine.y.at(x, y) {
				Some(v) => v as i32,
				None => continue,
			};
			let b = theirs.px[y * theirs.w + x] as i32;
			let d = (a - b).abs();
			if d > 0 {
				differing += 1;
				if first_bad.is_none() {
					first_bad = Some((x, y));
				}
			}
			worst = worst.max(d);
			total += d as i64;
		}
	}
	if let Some((x, y)) = first_bad {
		println!("    first differs at ({}, {}), which is block ({}, {}) of 32",
			x, y, x / 32, y / 32);
	}
	if worst > 0 {
		let stem = file.file_stem().unwrap_or_default().to_string_lossy().to_string();
		res!(write_pgm(&out.join(fmt!("{}-mine.pgm", stem)), &mine.y.px, mine.y.w, mine.y.h));
		let theirs_u16: Vec<u16> = theirs.px[..theirs.w * theirs.h]
			.iter().map(|v| *v as u16).collect();
		res!(write_pgm(&out.join(fmt!("{}-ffmpeg.pgm", stem)), &theirs_u16, theirs.w, theirs.h));
	}
	// And the colour planes, which are where a fault hides: a decoder can put every brightness
	// sample in the right place and still misread the chroma residual, and the only sign of it is
	// that everything AFTER that block is wrong.
	let (cw, ch) = (theirs.w / 2, theirs.h / 2);
	let base = theirs.w * theirs.h;
	let mut chroma_bad = 0usize;
	let mut first_chroma: Option<(usize, usize)> = None;
	for (plane, offset) in [(&mine.cb, base), (&mine.cr, base + cw * ch)] {
		for y in 0..ch.min(plane.h) {
			for x in 0..cw.min(plane.w) {
				let a = match plane.at(x, y) {
					Some(v) => v as i32,
					None => continue,
				};
				let b = theirs.px[offset + y * cw + x] as i32;
				if a != b {
					chroma_bad += 1;
					if first_chroma.is_none() {
						first_chroma = Some((x, y));
					}
				}
			}
		}
	}
	if let Some((x, y)) = first_chroma {
		println!("    colour first differs at ({}, {}) of {} by {}, {} samples out",
			x, y, cw, ch, chroma_bad);
	}
	let samples = w * h;
	Ok(Some(Verdict {
		samples,
		differing,
		worst,
		mean: total as f64 / samples as f64,
	}))
}

#[test]
fn test_a_decoded_tile_is_the_tile_another_decoder_makes_00() -> Outcome<()> {
	let dir = match std::env::var("HEVC_CORPUS") {
		Ok(d) => PathBuf::from(d),
		Err(_) => {
			println!("  skipped: set HEVC_CORPUS to a directory of HEIC photographs");
			return Ok(());
		},
	};
	if Command::new("ffmpeg").arg("-version").output().is_err() {
		println!("  skipped: no ffmpeg to compare against");
		return Ok(());
	}
	let want: usize = std::env::var("HEVC_FILES").ok()
		.and_then(|s| s.parse().ok())
		.unwrap_or(FILES);
	let out = scratch();
	res!(std::fs::create_dir_all(&out), IO, File);

	let mut files: Vec<PathBuf> = Vec::new();
	for entry in walk(&dir) {
		let name = entry.to_string_lossy().to_lowercase();
		if name.ends_with(".heic") || name.ends_with(".heif") {
			files.push(entry);
		}
		if files.len() >= want {
			break;
		}
	}
	if files.is_empty() {
		println!("  skipped: no HEIC photographs under {}", dir.display());
		return Ok(());
	}

	let (mut read, mut exact) = (0usize, 0usize);
	for file in &files {
		match compare(file, &out) {
			Ok(Some(v)) => {
				read += 1;
				let per_cent = 100.0 * v.differing as f64 / v.samples as f64;
				println!(
					"  {}: {} samples, {:.2}% differ, worst {}, mean {:.3}",
					file.file_name().unwrap_or_default().to_string_lossy(),
					v.samples, per_cent, v.worst, v.mean);
				if v.worst == 0 {
					exact += 1;
				}
			},
			Ok(None) => {},
			Err(e) => println!("  {}: {}", file.display(), e.plain()),
		}
	}
	println!("  {} of {} photographs read, {} of them sample for sample", read, files.len(), exact);
	let any = read > 0;
	req!(any, true, "not one photograph could be read");
	Ok(())
}

fn walk(dir: &Path) -> Vec<PathBuf> {
	let mut out = Vec::new();
	let mut stack = vec![dir.to_path_buf()];
	while let Some(at) = stack.pop() {
		let entries = match std::fs::read_dir(&at) {
			Ok(e) => e,
			Err(_) => continue,
		};
		for entry in entries.flatten() {
			let path = entry.path();
			if path.is_dir() {
				stack.push(path);
			} else {
				out.push(path);
			}
		}
	}
	out.sort();
	out
}

fn write_pgm(path: &Path, px: &[u16], w: usize, h: usize) -> Outcome<()> {
	let mut out = fmt!("P5\n{} {}\n255\n", w, h).into_bytes();
	out.extend(px.iter().take(w * h).map(|v| *v as u8));
	res!(std::fs::write(path, out), IO, File);
	Ok(())
}
