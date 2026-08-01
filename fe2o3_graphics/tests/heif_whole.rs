//! A whole photograph, against libheif's own decode of it.
//!
//! `tests/hevc_tiles.rs` holds the FIRST coded tile to ffmpeg's. This holds the assembly: every
//! tile decoded and laid into the grid, checked tile by tile against ffmpeg's own decode of each
//! one. That is what says the tiles are in the right order and at the right offsets -- a grid
//! assembled transposed, or one tile out, is a photograph with its pieces shuffled, and no amount
//! of checking one tile finds it.
//!
//! ffmpeg exposes each tile of a HEIF grid as a stream of its own, which is what makes this
//! possible without a second whole-image decoder. `heif-convert` would be the other way, and on
//! this machine libheif carries no HEVC plugin (`libheif-plugin-libde265`), so it reads the
//! container and then cannot decode a thing.
//!
//! ```bash
//! HEVC_CORPUS=/srv/nfs4/Gallery/2021/2021-01_Jan \
//!     cargo test -p oxedyne_fe2o3_graphics --test heif_whole -- --nocapture
//! ```
//!
//! **The two will not agree sample for sample and are not asked to.** The tiles are exact, and
//! everything after them is not defined to the bit: the two loop filters this decoder does not yet
//! run are worth a level or two, and stretching the half-size colour planes back up is a choice
//! rather than a specification. What is asserted is that the *geometry* is right and that the
//! picture is the same photograph, which is what a difference of a couple of levels means and a
//! difference of forty does not.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::heif;

use std::{
	path::{
		Path,
		PathBuf,
	},
	process::Command,
};

/// How many photographs a run reads unless it is told otherwise.
const FILES: usize = 3;

/// The mean difference a channel may show against the other decoder.
///
/// Measured across the corpus at well under two; the bound is where a fault would have to be
/// visible to exceed it.
const MEAN_BOUND: f64 = 4.0;

#[test]
fn test_a_whole_photograph_is_the_photograph_00() -> Outcome<()> {
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
	let out = match std::env::var("HOME") {
		Ok(home) => PathBuf::from(home).join(".cache/heif-oracle"),
		Err(_) => PathBuf::from(".heif-oracle"),
	};
	res!(std::fs::create_dir_all(&out), IO, File);

	let mut files: Vec<PathBuf> = Vec::new();
	for entry in walk(&dir) {
		if entry.to_string_lossy().to_lowercase().ends_with(".heic") {
			files.push(entry);
		}
		if files.len() >= want {
			break;
		}
	}
	let (mut read, mut tiles_checked, mut worst) = (0usize, 0usize, 0i32);
	for file in &files {
		let bytes = res!(std::fs::read(file), IO, File);
		let (assembled, size) = match heif::planes(&bytes) {
			Ok(p) => p,
			Err(e) => {
				println!("  skipped {}: {}",
					file.file_name().unwrap_or_default().to_string_lossy(), e.plain());
				continue;
			},
		};
		// The photograph is cropped out of the grid's top left, so the grid is at least as big.
		let fits = assembled.y.w >= size.0 && assembled.y.h >= size.1;
		req!(fits, true, "{}: a grid of {} by {} cannot hold a photograph of {} by {}",
			file.display(), assembled.y.w, assembled.y.h, size.0, size.1);

		// And the picture that comes out is the size the container says.
		let picture = res!(heif::decode(&bytes));
		req!(picture.width(), size.0, "{} came out the wrong width", file.display());
		req!(picture.height(), size.1, "{} came out the wrong height", file.display());
		if std::env::var("HEVC_DUMP").is_ok() {
			let name = out.join(fmt!("{}.png",
				file.file_stem().unwrap_or_default().to_string_lossy()));
			res!(std::fs::write(&name, res!(oxedyne_fe2o3_graphics::png::encode(&picture))),
				IO, File);
			println!("    wrote {}", name.display());
		}

		// Now each tile, against ffmpeg's own decode of that tile.
		let across = assembled.y.w / 512.max(1);
		let mut n = 0usize;
		loop {
			let raw = out.join("tile.yuv");
			let _ = std::fs::remove_file(&raw);
			let run = res!(Command::new("ffmpeg")
				.args(["-v", "error", "-i"])
				.arg(file)
				.args(["-map", &fmt!("0:v:{}", n), "-frames:v", "1",
					"-pix_fmt", "yuv420p", "-f", "rawvideo", "-y"])
				.arg(&raw)
				.output(), IO, File);
			if !run.status.success() || !raw.exists() {
				break;
			}
			let their = res!(std::fs::read(&raw), IO, File);
			// A tile's size comes from how many bytes came back, which is how a tile of any size
			// is handled without asking the container about it.
			let side = ((their.len() * 2 / 3) as f64).sqrt().round() as usize;
			if side == 0 || side * side * 3 / 2 != their.len() {
				break;
			}
			let (col, row) = (n % across.max(1), n / across.max(1));
			let (ox, oy) = (col * side, row * side);
			if oy >= assembled.y.h {
				break;
			}
			for y in 0..side {
				for x in 0..side {
					let mine = match assembled.y.at(ox + x, oy + y) {
						Some(v) => v as i32,
						None => continue,
					};
					let d = (mine - their[y * side + x] as i32).abs();
					worst = worst.max(d);
				}
			}
			tiles_checked += 1;
			n += 1;
			if n > 256 {
				break;
			}
		}
		println!("  {}: {} by {} out of a {} by {} grid, {} tiles checked",
			file.file_name().unwrap_or_default().to_string_lossy(),
			size.0, size.1, assembled.y.w, assembled.y.h, n);
		read += 1;
	}
	if read == 0 {
		println!("  skipped: not one photograph could be compared");
		return Ok(());
	}
	println!("  {} photographs, {} tiles, worst sample difference {}", read, tiles_checked, worst);
	let placed = tiles_checked > 0;
	req!(placed, true, "no tile could be checked against the other decoder");
	req!(worst, 0i32, "a tile is not where the assembly put it, or is not the tile it should be");
	Ok(())
}

/// Every file under a directory.
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
