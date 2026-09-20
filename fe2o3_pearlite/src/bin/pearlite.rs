//! `pearlite` -- the native `.prl` reader's command line.
//!
//! Three subcommands, one per phase this crate covers:
//!
//! - `render <DOC.prl> [OUT_DIR]` -- per-page SVG, via `PearlDoc::render_page` alone (Phase 0's own
//!   round trip, exposed here as a convenience alongside the other two).
//! - `png <DOC.prl> [OUT_DIR] [--dpi N]` -- per-page PNG at a chosen DPI (Phase 1, [`raster`]).
//! - `serve <DOC.prl> [--port N] [--no-open]` -- a loopback browser-shell app serving the existing
//!   `pearl-reader` web assets against this one document (Phase 2, [`shell`]).

use oxedyne_fe2o3_pearlite::raster;
use oxedyne_fe2o3_pearlite::shell::{
	self,
	Shell,
};

use oxedyne_fe2o3_austenite::emit::pearl::PearlDoc;

use oxedyne_fe2o3_core::prelude::*;

fn main() -> Outcome<()> {
	let args: Vec<String> = std::env::args().skip(1).collect();
	let cmd = match args.first() {
		Some(c)	=> c.as_str(),
		None	=> return usage(),
	};
	match cmd {
		"render"	=> cmd_render(&args[1..]),
		"png"		=> cmd_png(&args[1..]),
		"serve"		=> cmd_serve(&args[1..]),
		_			=> usage(),
	}
}

fn usage() -> Outcome<()> {
	println!("usage:");
	println!("  pearlite render <DOC.prl> [OUT_DIR]");
	println!("  pearlite png <DOC.prl> [OUT_DIR] [--dpi N]");
	println!("  pearlite serve <DOC.prl> [--port N] [--no-open]");
	Ok(())
}

/// The document path and the directory to write pages into, from a subcommand's own positional and
/// optional arguments (the pattern `render` and `png` share).
fn doc_and_out_dir(args: &[String]) -> Outcome<(String, String)> {
	let source = match args.first() {
		Some(s)	=> s.clone(),
		None	=> return Err(err!("A .prl path is required."; Input, Missing)),
	};
	let out_dir = match args.get(1).filter(|a| !a.starts_with("--")) {
		Some(s)	=> s.clone(),
		None	=> std::path::Path::new(&source)
			.parent()
			.map(|p| p.to_string_lossy().into_owned())
			.filter(|s| !s.is_empty())
			.unwrap_or_else(|| ".".to_string()),
	};
	Ok((source, out_dir))
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
	args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn has_flag(args: &[String], name: &str) -> bool {
	args.iter().any(|a| a == name)
}

fn cmd_render(args: &[String]) -> Outcome<()> {
	let (source, out_dir) = res!(doc_and_out_dir(args));
	let doc		= res!(PearlDoc::read_file(&source));
	let pages	= res!(doc.page_count());
	res!(std::fs::create_dir_all(&out_dir), IO, File, Write);
	for i in 0..pages {
		let svg		= res!(doc.render_page(i));
		let path	= fmt!("{}/pearl-page-{:03}.svg", out_dir, i + 1);
		res!(std::fs::write(&path, &svg), IO, File, Write);
	}
	println!("pearlite render: {} -> {} page(s); written to {}/", source, pages, out_dir);
	Ok(())
}

fn cmd_png(args: &[String]) -> Outcome<()> {
	let (source, out_dir) = res!(doc_and_out_dir(args));
	let dpi = match flag_value(args, "--dpi") {
		Some(s) => match s.parse::<f32>() {
			Ok(v)	=> v,
			Err(e)	=> return Err(err!("'{}' is not a valid --dpi value: {}.", s, e; Input, Invalid)),
		},
		None => raster::DEFAULT_DPI,
	};
	let doc		= res!(PearlDoc::read_file(&source));
	let pngs	= res!(raster::render_all_pages_to_png(&doc, dpi));
	res!(std::fs::create_dir_all(&out_dir), IO, File, Write);
	for (i, png) in pngs.iter().enumerate() {
		let path = fmt!("{}/pearl-page-{:03}.png", out_dir, i + 1);
		res!(std::fs::write(&path, png), IO, File, Write);
	}
	println!("pearlite png: {} -> {} page(s) at {} dpi; written to {}/", source, pngs.len(), dpi, out_dir);
	Ok(())
}

fn cmd_serve(args: &[String]) -> Outcome<()> {
	let source = match args.first() {
		Some(s)	=> s.clone(),
		None	=> return Err(err!("A .prl path is required."; Input, Missing)),
	};
	let port = match flag_value(args, "--port") {
		Some(s) => match s.parse::<u16>() {
			Ok(v)	=> v,
			Err(e)	=> return Err(err!("'{}' is not a valid --port value: {}.", s, e; Input, Invalid)),
		},
		None => 0,
	};
	let open = !has_flag(args, "--no-open");

	let doc_bytes = res!(std::fs::read(&source), IO, File, Read);
	// The document's own file name is the path the served index.html's `?doc=` query names, so the
	// reader's own status line shows it exactly as `render_page`'s in-browser sibling would.
	let doc_name = std::path::Path::new(&source)
		.file_name()
		.map(|n| n.to_string_lossy().into_owned())
		.unwrap_or_else(|| "opened.prl".to_string());

	let listener	= res!(Shell::bind(port));
	let bound_port	= res!(listener.local_addr(), IO, Network).port();
	let url			= fmt!("http://127.0.0.1:{}/?doc={}", bound_port, doc_name);
	println!("pearlite serve: {} at {}", source, url);

	if open {
		if let Err(e) = shell::open_browser(&url) {
			println!("pearlite serve: could not open a browser automatically ({}); open {} by hand.",
				e, url);
		}
	}

	let shell = Shell::new(doc_name, doc_bytes);
	shell.serve(listener)
}
