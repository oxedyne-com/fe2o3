//! Feeds a recording of a pseudoterminal to the terminal model and prints the resulting screen.
//!
//! This exists so that the model can be compared with a real terminal without anybody transcribing
//! anything by hand. `script --log-out out.bin -c 'stty rows 24 cols 80; some_programme'` records
//! exactly what a programme writes to a pseudoterminal; feeding that same recording to tmux and to
//! this tool and diffing the two screens is what the oracle cases in `tests/term.rs` were built on.
//!
//! ```text
//! cargo run --example term_dump -- 80 24 out.bin
//! cargo run --example term_dump -- 80 24 out.bin --chunk 7 --scrollback
//! ```
//!
//! `--chunk n` feeds the recording `n` bytes at a time, which cuts characters and control sequences
//! at every kind of boundary and is how the parser's held over state gets exercised. `--resize
//! COLSxROWS` resizes after the feed, which is how the rewrapping is compared with tmux.

use oxedyne_fe2o3_tui::lib_tui::term::Terminal;

use oxedyne_fe2o3_core::prelude::*;

use std::{
	fs,
	process,
};


fn main() {
	match run() {
		Ok(())	=> {}
		Err(e)	=> {
			eprintln!("{}", e);
			process::exit(1);
		}
	}
}

/// Reads the arguments, feeds the recording and prints the screen.
fn run() -> Outcome<()> {
	let args: Vec<String> = std::env::args().collect();
	if args.len() < 4 {
		return Err(err!(
			"Usage: term_dump <cols> <rows> <file> [--chunk n] [--scrollback] \
			[--resize COLSxROWS] [--cursor]";
			Invalid, Input));
	}
	let cols = res!(args[1].parse::<usize>(), Invalid, Input);
	let rows = res!(args[2].parse::<usize>(), Invalid, Input);
	let byts = res!(fs::read(&args[3]), IO, File);
	let mut chunk = 0usize;
	let mut scrollback = false;
	let mut cursor = false;
	let mut resize: Option<(usize, usize)> = None;
	let mut i = 4;
	while i < args.len() {
		match args[i].as_str() {
			"--chunk"	=> {
				i += 1;
				let v = res!(need(args.get(i), "--chunk needs a size"));
				chunk = res!(v.parse::<usize>(), Invalid, Input);
			}
			"--scrollback"	=> scrollback = true,
			"--cursor"	=> cursor = true,
			"--resize"	=> {
				i += 1;
				let v = res!(need(args.get(i), "--resize needs a size"));
				resize = Some(res!(parse_size(v)));
			}
			other	=> return Err(err!("{} is not an argument this tool takes.", other;
				Invalid, Input)),
		}
		i += 1;
	}
	let mut term = res!(Terminal::new(cols, rows));
	if chunk == 0 {
		res!(term.feed(&byts));
	} else {
		let mut a = 0;
		while a < byts.len() {
			let b = (a + chunk).min(byts.len());
			res!(term.feed(&byts[a..b]));
			a = b;
		}
	}
	if let Some((c, r)) = resize {
		res!(term.resize(c, r));
	}
	let scr = term.screen();
	if scrollback {
		for i in 0..scr.scrollback_len() {
			match scr.scrollback_text(i) {
				Some(s)	=> println!("{}", s),
				None	=> {}
			}
		}
	}
	for r in 0..scr.rows() {
		println!("{}", scr.row_text(r));
	}
	if cursor {
		let cur = scr.cursor();
		eprintln!("CUR {},{} HIST {} VIEW {}",
			cur.reported_col(), cur.row, scr.scrollback_len(), scr.view_offset());
	}
	Ok(())
}

/// Reads a `COLSxROWS` argument.
fn parse_size(s: &str) -> Outcome<(usize, usize)> {
	let i = res!(need(s.find('x'), "a size is written as COLSxROWS"));
	let c = res!(s[..i].parse::<usize>(), Invalid, Input);
	let r = res!(s[i + 1..].parse::<usize>(), Invalid, Input);
	Ok((c, r))
}

/// Turns an absent value into an error that names what was missing.
fn need<T>(v: Option<T>, what: &str) -> Outcome<T> {
	match v {
		Some(v)	=> Ok(v),
		None	=> Err(err!("Expected {}.", what; Invalid, Input)),
	}
}
