//! Writes the demo oxeweb document: one signed SBJ file, for every renderer to read.
//!
//! There is exactly one demo document, and both engines read it. Kiln renders it to pixels natively
//! and the browser renders it to DOM through WebAssembly, from these same bytes. That is what makes
//! the two comparable: a difference between them is a difference between the renderers, not between
//! two documents that happened to look alike.
//!
//! Run with `cargo run -p oxedyne_fe2o3_sbj --example gen_demo -- <path>`.

use oxedyne_fe2o3_sbj::{
	doc,
	kinds::NodeKind,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::sign::SignatureScheme;
use oxedyne_fe2o3_jdat::{
	prelude::*,
	usr::UsrKindId,
};

/// The time the demo document is signed at, fixed so that a rebuild changes only the signature.
const TIME: u64 = 1_752_500_000_000;

/// The user kind identifier of a node kind: its wire code, its label, and the shape of its payload.
fn ukid(kind: NodeKind) -> UsrKindId {
	let payload = if kind.payload_is_str() {
		Kind::Str
	} else {
		Kind::Map
	};
	UsrKindId::new(kind.code(), Some(&fmt!("sbj_{}", kind.label())), Some(payload))
}

/// A node: a kind code, then its payload.
fn node(kind: NodeKind, payload: Dat) -> Dat {
	Dat::Usr(ukid(kind), Some(Box::new(payload)))
}

/// A text run, the one node whose payload is a bare string.
fn text(s: &str) -> Dat {
	node(NodeKind::Text, Dat::Str(s.to_string()))
}

/// A payload map.
fn map(pairs: Vec<(&str, Dat)>) -> Dat {
	let mut m = DaticleMap::new();
	for (k, v) in pairs {
		m.insert(Dat::Str(k.to_string()), v);
	}
	Dat::Map(m)
}

/// The demo document, exercising every kind the vocabulary holds.
pub fn tree() -> Dat {
	let tree = node(NodeKind::Doc, map(vec![
		("title",	Dat::Str("Style without a cascade".to_string())),
		("lang",	Dat::Str("en".to_string())),
		("styles", map(vec![
			("callout", map(vec![
				("bg",		Dat::Str("muted".to_string())),
				("pad",		Dat::U8(3)),
			])),
			("centred", map(vec![
				("align",	Dat::Str("center".to_string())),
				("fill",	Dat::Str("muted".to_string())),
			])),
			("flush", map(vec![
				("align",	Dat::Str("justify".to_string())),
			])),
		])),
		("children", Dat::List(vec![

			node(NodeKind::Heading, map(vec![
				("level",	Dat::U8(1)),
				("children",	Dat::List(vec![text("Style without a cascade")])),
			])),

			node(NodeKind::Para, map(vec![
				("style",	Dat::Str("flush".to_string())),
				("children", Dat::List(vec![
					text("The oxeweb replaces the web's cascade with "),
					node(NodeKind::Emph, map(vec![
						("strong",	Dat::Bool(false)),
						("children",	Dat::List(vec![text("locality")])),
					])),
					text(". A node names a style; the style is defined once, in the document's \
						style table; and a short inherited set flows down the tree. No rule \
						reaches across the document, so a style error cannot escape the node \
						that made it. This paragraph is long enough that it must wrap, which \
						is the whole point of it."),
				])),
			])),

			node(NodeKind::Para, map(vec![
				("children", Dat::List(vec![
					text("Emphasis can be "),
					node(NodeKind::Emph, map(vec![
						("strong",	Dat::Bool(true)),
						("children",	Dat::List(vec![text("strong")])),
					])),
					text(", and a link points at "),
					node(NodeKind::Link, map(vec![
						("to", map(vec![("name", Dat::Str("news.cricket".to_string()))])),
						("children", Dat::List(vec![text("a name, not a location")])),
					])),
					text("."),
				])),
			])),

			node(NodeKind::Boxx, map(vec![
				("style",	Dat::Str("callout".to_string())),
				("children", Dat::List(vec![
					node(NodeKind::Para, map(vec![
						("children", Dat::List(vec![
							text("Reader preferences are applied after author styles, and \
								always win."),
						])),
					])),
				])),
			])),

			node(NodeKind::Heading, map(vec![
				("level",	Dat::U8(2)),
				("children",	Dat::List(vec![text("What the format holds")])),
			])),

			node(NodeKind::List, map(vec![
				("ordered",	Dat::Bool(false)),
				("children", Dat::List(vec![
					node(NodeKind::Item, map(vec![
						("children", Dat::List(vec![
							node(NodeKind::Para, map(vec![
								("children", Dat::List(vec![
									text("Thirteen node kinds, and no more."),
								])),
							])),
						])),
					])),
					node(NodeKind::Item, map(vec![
						("children", Dat::List(vec![
							node(NodeKind::Para, map(vec![
								("children", Dat::List(vec![
									text("Semantic colours and scale steps, never pixels."),
								])),
							])),
						])),
					])),
					node(NodeKind::Item, map(vec![
						("children", Dat::List(vec![
							node(NodeKind::Para, map(vec![
								("children", Dat::List(vec![
									text("An address that is a hash of what it addresses."),
								])),
							])),
						])),
					])),
				])),
			])),

			node(NodeKind::Quote, map(vec![
				("cite",	Dat::Str("SPEC.md §6".to_string())),
				("children", Dat::List(vec![
					node(NodeKind::Para, map(vec![
						("children", Dat::List(vec![
							text("There is no repair, no quirks mode, and no best effort. The \
								web gave that up in 1993 and spent thirty years paying for it."),
						])),
					])),
				])),
			])),

			node(NodeKind::Code, map(vec![
				("lang",	Dat::Str("jdat".to_string())),
				("text",	Dat::Str(
					"(box|{\n    \"style\": \"callout\",\n    \"children\": [...],\n})".to_string(),
				)),
			])),

			node(NodeKind::Image, map(vec![
				("hash",	Dat::from([0x9fu8; 32])),
				("alt",		Dat::Str("A diagram this reader has no copy of.".to_string())),
			])),

			// Mixed direction, resolved by the bidi algorithm and reordered by rule L2.
			node(NodeKind::Para, map(vec![
				("style",	Dat::Str("centred".to_string())),
				("children", Dat::List(vec![
					text("Arabic runs the other way: مرحبا بالعالم — and back to English."),
				])),
			])),
		])),
	]));
	tree
}

fn main() -> Outcome<()> {
	let args: Vec<String> = std::env::args().collect();
	let path = match args.get(1) {
		Some(p) => p.clone(),
		None => return Err(err!(
			"Give the path to write the document to: \
			`cargo run -p sbj --example gen_demo -- <path>`.";
		Invalid, Input, Missing)),
	};

	let signer = SignatureScheme::new_ed25519();
	let bytes = res!(doc::write(&tree(), "oxeweb/doc/0", &signer, TIME));

	// It must read back the way a reader would, or it is not a document, it is a file.
	res!(doc::read(&bytes));

	if let Some(dir) = std::path::Path::new(&path).parent() {
		res!(std::fs::create_dir_all(dir));
	}
	res!(std::fs::write(&path, &bytes));
	println!("Wrote {} ({} bytes), verified.", path, bytes.len());
	Ok(())
}
