//! `pearl_render` -- render a `.prl` back to SVG.
//!
//! Reads a Pearl document written by `austenite --pearl` and, for every page in its index, reconstructs
//! the very SVG the engine's SVG arm would have written -- from the stored glyph outlines, geometry and
//! paint alone, without the engine. This is the round-trip the Pearl v0 spike exists to prove: a `.prl`
//! is enough to render the document.
//!
//! Usage: `pearl_render <DOCUMENT.prl> [OUTPUT_DIR]` (default output the `.prl`'s own directory), writing
//! `pearl-page-001.svg`, `pearl-page-002.svg`, and so on.

use oxedyne_fe2o3_austenite::emit::pearl::PearlDoc;

use oxedyne_fe2o3_core::prelude::*;

fn main() -> Outcome<()> {
	let args: Vec<String> = std::env::args().skip(1).collect();
	let source = match args.first() {
		Some(s)	=> s.clone(),
		None	=> return Err(err!(
			"Usage: pearl_render <DOCUMENT.prl> [OUTPUT_DIR]"; Input, Invalid, Missing)),
	};
	// Default the output beside the source, so a bare `pearl_render out/document.prl` drops its pages next
	// to the file it read.
	let out_dir = match args.get(1) {
		Some(s)	=> s.clone(),
		None	=> std::path::Path::new(&source)
			.parent()
			.map(|p| p.to_string_lossy().into_owned())
			.filter(|s| !s.is_empty())
			.unwrap_or_else(|| ".".to_string()),
	};

	let doc		= res!(PearlDoc::read_file(&source));
	let pages	= res!(doc.page_count());
	res!(std::fs::create_dir_all(&out_dir));
	for i in 0..pages {
		let svg		= res!(doc.render_page(i));
		let path	= fmt!("{}/pearl-page-{:03}.svg", out_dir, i + 1);
		res!(std::fs::write(&path, &svg));
	}
	println!("pearl_render: {} -> {} page(s); written to {}/", source, pages, out_dir);

	// Read the annotations back through the format's own decoder, so a `.prl` authored elsewhere -- the
	// browser reader included -- is proven readable here, rectangles and all.
	let anns = res!(doc.annotations());
	println!("pearl_render: {} annotation(s)", anns.len());
	for a in &anns {
		let rect = match a.rect {
			Some((x, y, w, h)) => fmt!("[{} {} {} {}]", x.raw(), y.raw(), w.raw(), h.raw()),
			None => "(whole block)".to_string(),
		};
		println!("  {} @ {} rect {} by {} -- {:?}",
			a.kind.as_str(), &a.anchor[..8.min(a.anchor.len())], rect, a.author, a.payload);
	}
	Ok(())
}
