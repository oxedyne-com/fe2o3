//! Hand an animation we wrote to two readers that know nothing about us, and check they see the
//! frames we drew.
//!
//! The unit tests in `png.rs` assert on the chunks the encoder emitted -- their order, their
//! sequence numbers, the rectangle each frame claims. That is a fair check of the framing and no
//! check at all of whether the file animates: a frame written at the wrong offset, a difference
//! rectangle a pixel short on its right edge, or a blend operation that composites where it should
//! replace all produce perfectly well-formed chunks, and every one of those tests still passes.
//! The encoder and its tests share a hand, so they share any misreading of what the fields mean.
//!
//! So the file is decoded here by FFmpeg and by Pillow, and the pixels compared are the ones the
//! frames were filled with before any of this ran. Nothing in the comparison passes through this
//! crate's own decoder. The three frames are chosen to exercise the part most likely to be wrong:
//! the second differs from the first only in a small off-centre rectangle, so a difference computed
//! or placed incorrectly puts the block somewhere the readers will not find it, and the third
//! restores the first, so the canvas has to be repainted where the block was.
//!
//! Both readers are optional. A machine without them skips its half and says so.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::{
	colour::Rgba,
	path::Bounds,
	pixmap::Pixmap,
	png::{
		Animation,
		Delay,
	},
};

use std::{
	fs,
	path::PathBuf,
	process::Command,
};

// The canvas the fixtures are drawn on.
const W: usize = 24;
const H: usize = 16;

// The ground colour, and the block that moves onto it.
const GROUND: Rgba = Rgba { r: 220, g: 30, b: 40, a: 255 };
const BLOCK: Rgba = Rgba { r: 0, g: 0, b: 255, a: 255 };

// The block's rectangle: from (5, 3) to (11, 9), so six wide and six high.
const BX0: f32 = 5.0;
const BY0: f32 = 3.0;
const BX1: f32 = 11.0;
const BY1: f32 = 9.0;

/// The three frames the animation holds, in order.
fn frames() -> Outcome<Vec<Pixmap>> {
	let a = res!(Pixmap::filled(W, H, GROUND));
	let mut b = a.clone();
	res!(b.fill_bounds(Bounds::new(BX0, BY0, BX1, BY1), BLOCK, None));
	let c = a.clone();
	Ok(vec![a, b, c])
}

/// What each frame's pixel at `(x, y)` should be, taken from the drawing rather than from the file.
fn expected(frame: usize, x: usize, y: usize) -> Rgba {
	let inside = frame == 1
		&& (x as f32) >= BX0 && (x as f32) < BX1
		&& (y as f32) >= BY0 && (y as f32) < BY1;
	if inside { BLOCK } else { GROUND }
}

fn write_fixture() -> Outcome<PathBuf> {
	let mut anim = res!(Animation::new(W, H)).plays(0);
	for pm in res!(frames()) {
		res!(anim.push(&pm, res!(Delay::fps(20))));
	}
	let buf = res!(anim.finish());
	let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("apng_oracle.png");
	res!(fs::write(&path, &buf));
	Ok(path)
}

/// Is the program on the path?  Both spellings of the flag are tried, since FFmpeg takes one and
/// Python the other.
fn have(prog: &str) -> bool {
	for flag in ["-version", "--version"] {
		let ok = Command::new(prog)
			.arg(flag)
			.output()
			.map(|o| o.status.success())
			.unwrap_or(false);
		if ok {
			return true;
		}
	}
	false
}

#[test]
fn test_ffmpeg_reads_back_the_frames_that_were_drawn_00() -> Outcome<()> {
	if !have("ffmpeg") {
		println!("FFmpeg is not installed, so the animation's first oracle is skipped.");
		return Ok(());
	}
	let path = res!(write_fixture());

	// Raw RGBA on standard output, one output frame per input frame, so nothing between the
	// decoder and the comparison can resample, retime or convert.
	let out = res!(Command::new("ffmpeg")
		.args(["-loglevel", "error", "-i"])
		.arg(&path)
		.args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
		.output());
	if !out.status.success() {
		return Err(err!(
			"FFmpeg refused the animation: {}", String::from_utf8_lossy(&out.stderr);
		Invalid, Input));
	}

	let want = W * H * 4;
	req!(out.stdout.len(), want * 3);

	let mut worst = None;
	for (f, chunk) in out.stdout.chunks_exact(want).enumerate() {
		for y in 0..H {
			for x in 0..W {
				let i = (y * W + x) * 4;
				let got = Rgba::new(chunk[i], chunk[i + 1], chunk[i + 2], chunk[i + 3]);
				let exp = expected(f, x, y);
				if got != exp && worst.is_none() {
					worst = Some((f, x, y, got, exp));
				}
			}
		}
	}
	if let Some((f, x, y, got, exp)) = worst {
		return Err(err!(
			"FFmpeg reads frame {} at ({}, {}) as {:?}, where {:?} was drawn.", f, x, y, got, exp;
		Invalid, Input, Mismatch));
	}
	println!("FFmpeg agrees on all {} pixels of all 3 frames.", want / 4 * 3);
	Ok(())
}

#[test]
fn test_pillow_reads_back_the_count_the_timing_and_the_block_01() -> Outcome<()> {
	if !have("python3") {
		println!("Python is not installed, so the animation's second oracle is skipped.");
		return Ok(());
	}
	let probe = r#"
import sys
try:
    from PIL import Image
except ImportError:
    print("SKIP")
    sys.exit(0)
im = Image.open(sys.argv[1])
out = [str(im.n_frames)]
for i in range(im.n_frames):
    im.seek(i)
    rgba = im.convert("RGBA")
    out.append("%d,%d,%d" % (i, rgba.getpixel((7, 5))[0], rgba.getpixel((7, 5))[2]))
    out.append("d%d,%s" % (i, im.info.get("duration")))
print(";".join(out))
"#;
	let path = res!(write_fixture());
	let out = res!(Command::new("python3").arg("-c").arg(probe).arg(&path).output());
	if !out.status.success() {
		return Err(err!(
			"Pillow refused the animation: {}", String::from_utf8_lossy(&out.stderr);
		Invalid, Input));
	}
	let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
	if text == "SKIP" {
		println!("Pillow is not installed, so the animation's second oracle is skipped.");
		return Ok(());
	}
	let parts: Vec<&str> = text.split(';').collect();

	// The frame count the file declares, read by somebody else.
	req!(parts[0], "3");

	// A pixel inside the block: blue in the middle frame, ground in the two either side. That one
	// pixel is the whole of what the difference rectangle is for.
	req!(parts[1], "0,220,40");
	req!(parts[3], "1,0,255");
	req!(parts[5], "2,220,40");

	// A twentieth of a second is fifty milliseconds, and Pillow reports the delay in milliseconds.
	for i in [2usize, 4, 6] {
		req!(parts[i], &format!("d{},50.0", (i - 2) / 2) as &str);
	}
	println!("Pillow agrees: {}", text);
	Ok(())
}
