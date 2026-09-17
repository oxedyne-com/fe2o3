//! `pearl_json` -- project a `.prl` document to JSON, for a browser reader.
//!
//! A read-only transport bridge: the `.prl` is text jdat, and this decodes it and re-encodes the very
//! same `Dat` as JSON so a browser can `JSON.parse` it and render the document from its real data model
//! -- glyph outlines, placements and paint -- rather than from any pre-rendered SVG. It touches neither
//! the emit nor the reader; it only changes the encoding the same content ships in.
//!
//! Every `.prl` value is JSON-representable already: strings, `u32`/`f32` scalars, ordered maps and
//! lists, with rasters carried as base64 strings. The type annotations jdat writes -- `(u32|..)`,
//! `(f32|..)`, `(omap|..)` -- are dropped by the JSON encoder, leaving plain numbers, objects and
//! arrays. Object key order is preserved, which the glyph, block and index lookups rely on.
//!
//! Usage: `pearl_json <DOCUMENT.prl> [OUTPUT.json]` (default beside the source as `document.json`).

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

fn main() -> Outcome<()> {
	let args: Vec<String> = std::env::args().skip(1).collect();
	let source = match args.first() {
		Some(s)	=> s.clone(),
		None	=> return Err(err!(
			"Usage: pearl_json <DOCUMENT.prl> [OUTPUT.json]"; Input, Invalid, Missing)),
	};
	let out_path = match args.get(1) {
		Some(s)	=> s.clone(),
		None	=> {
			let stem = std::path::Path::new(&source)
				.parent()
				.map(|p| p.to_string_lossy().into_owned())
				.filter(|s| !s.is_empty())
				.unwrap_or_else(|| ".".to_string());
			fmt!("{}/document.json", stem)
		},
	};

	let text	= res!(std::fs::read_to_string(&source));
	let dat		= res!(Dat::decode_string(text));
	let json	= res!(dat.json());
	res!(std::fs::write(&out_path, &json));
	println!("pearl_json: {} -> {} ({} bytes)", source, out_path, json.len());
	Ok(())
}
