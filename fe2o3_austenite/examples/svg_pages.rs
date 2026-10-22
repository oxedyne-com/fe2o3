//! Compiles a document through the evaluator to one SVG per page, the pages the live view receives
//! (`compileProjectVector`), and writes them as `OUT_DIR/page-NNN.svg`. The command line writes only the
//! PDF; this is the native way to see the browser's pages, for the SVG size and pixel gates.
//!
//! `cargo run --release --example svg_pages -- SOURCE.typ OUT_DIR [--root DIR] [--font-path DIR]...`

use oxedyne_fe2o3_austenite::{
	compile,
	emit::sinks::VectorSink,
	flow::text::FontStore,
};

use oxedyne_fe2o3_core::prelude::*;

use std::path::PathBuf;

fn main() -> Outcome<()> {
	let mut pos:	Vec<String>		= Vec::new();
	let mut root:	Option<String>	= None;
	let mut fonts	= FontStore::default();
	let mut args	= std::env::args().skip(1);
	while let Some(a) = args.next() {
		match a.as_str() {
			"--root"		=> root = Some(res!(args.next().ok_or_else(|| err!(
				"--root needs a directory argument."; Input, Missing)))),
			"--font-path"	=> fonts.add_dir(PathBuf::from(res!(args.next().ok_or_else(|| err!(
				"--font-path needs a directory argument."; Input, Missing))))),
			_				=> pos.push(a),
		}
	}
	if pos.len() != 2 {
		return Err(err!("Usage: svg_pages SOURCE.typ OUT_DIR [--root DIR] [--font-path DIR]..."; Input, Invalid));
	}
	let main	= PathBuf::from(&pos[0]);
	let out		= PathBuf::from(&pos[1]);
	let root	= match root {
		Some(r)	=> PathBuf::from(r),
		None	=> main.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from(".")),
	};
	compile::supply_typst_package_cache();
	let mut session	= compile::Session::new(fonts);
	let mut sink	= VectorSink::default();
	let done		= res!(session.compile(&main, &root, &mut sink, None, None, true));
	if let Err(e) = &done.laid {
		return Err(err!("{} did not lay out: {}", main.display(), e.plain(); Input, Invalid));
	}
	let pages = res!(sink.into_pages().ok_or_else(|| err!(
		"The fixpoint ended without a finished pass for {}.", main.display(); Bug)));
	res!(std::fs::create_dir_all(&out));
	for (i, svg) in pages.iter().enumerate() {
		res!(std::fs::write(out.join(format!("page-{:03}.svg", i + 1)), svg));
	}
	println!("{}: {} page(s) to {}", main.display(), pages.len(), out.display());
	Ok(())
}
