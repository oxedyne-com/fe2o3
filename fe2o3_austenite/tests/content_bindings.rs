//! Do general `#let` content bindings evaluate, and does `#import` resolve on every compile path?
//!
//! Before this lane the reader lowered only `#let name(...) = block/box(...)` furniture and dropped a
//! `#let name = [...]`/`#let name(p) = [...]` content binding; the lone-file path never walked `#import`;
//! and a bare `#name` reference set as raw prose. These tests drive the SAME [`compile::assemble`] the wasm
//! surface calls, over an injected [`crate::vfs`] source map, so they are the native proof of the browser
//! behaviour -- exactly as `font_inject.rs` is for injected fonts.
//!
//! Two fixtures, pinned in one test because the source-map global forbids two compiles at once:
//!
//!  1. A `#include`-bearing doc root that imports a `#greet(who) = [Hello, #who.]` content function from a
//!     template and calls `#greet("world")`: the import must resolve, the call expand to the paragraph
//!     "Hello, world.", the chapter's own heading and body appear, and no block leak a raw `#`.
//!  2. A lone file (no `#include`) that imports a `#let one = [...]` value binding and references it as a
//!     bare `#one`: the lone path must still walk the `#import`, and the bare reference must expand to the
//!     binding's heading and body -- the two things the lone path could not do before.
//!
//! The paths use a non-existent `/__vfs__/` prefix so a source-map miss cannot fall through to a real file
//! on the build host, and the map is cleared between the two compiles.

use oxedyne_fe2o3_austenite::compile;
use oxedyne_fe2o3_austenite::doc::{
	Block,
	Segment,
};
use oxedyne_fe2o3_austenite::fonts;
use oxedyne_fe2o3_austenite::vfs;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// The plain text of a block -- a paragraph's text, or a heading's segments flattened -- for a content
/// assertion. A block that carries no running text (a rule, an image) yields the empty string.
fn block_text(block: &Block) -> String {
	match block {
		Block::Paragraph { text }		=> text.clone(),
		Block::RichParagraph { segments }
		| Block::Heading { segments, .. }	=> segments_text(segments),
		_								=> String::new(),
	}
}

/// The plain text of a run of segments, keeping the words and dropping the styling.
fn segments_text(segments: &[Segment]) -> String {
	let mut out = String::new();
	for seg in segments {
		match seg {
			Segment::Text(t)
			| Segment::Strong(t)
			| Segment::Emph(t)
			| Segment::BoldItalic(t)
			| Segment::Super(t)
			| Segment::Sub(t)
			| Segment::Code(t)	=> out.push_str(t),
			_					=> {},
		}
	}
	out
}

/// Assembles the document rooted at `/__vfs__/main.typ` from an injected source map, returning its body
/// blocks. Mirrors the wasm surface: the map is installed, [`compile::assemble`] runs with the embedded
/// Libertinus as the lone-file reading set, and the map is cleared afterwards whatever the outcome.
fn assemble_blocks(sources: &[(&str, &str)]) -> Outcome<Vec<Block>> {
	let mut files: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	for (path, src) in sources {
		files.insert(PathBuf::from(path), src.as_bytes().to_vec());
	}
	res!(vfs::install(files));
	let fonts	= Arc::new(res!(fonts::libertinus()));
	let outcome	= compile::assemble(&PathBuf::from("/__vfs__/main.typ"), || Ok(fonts.clone()));
	let _		= vfs::clear();
	let (assembled, _refusals, _skip) = res!(outcome);
	Ok(assembled.blocks)
}

/// Flattens the block tree into a leaf sequence, descending through the `Scoped`/`Box` wrappers the rule
/// engine and furniture set around a heading or body -- so an assertion sees the heading itself, not the
/// scope that carries its re-asserted size.
fn flatten(blocks: &[Block]) -> Vec<&Block> {
	let mut out = Vec::new();
	for b in blocks {
		match b {
			Block::Scoped { blocks, .. }
			| Block::Box { blocks, .. }	=> out.extend(flatten(blocks)),
			_							=> out.push(b),
		}
	}
	out
}

/// Does any (flattened) block yield exactly `text`?
fn has_block_text(blocks: &[Block], text: &str) -> bool {
	flatten(blocks).into_iter().any(|b| block_text(b) == text)
}

/// Does any (flattened) block yield `text` and carry heading level `level`?
fn has_heading(blocks: &[Block], text: &str, level: u8) -> bool {
	flatten(blocks).into_iter().any(|b| matches!(b,
		Block::Heading { level: l, segments, .. } if *l == level && segments_text(segments) == text))
}

/// Both fixtures, pinned together (the source-map global forbids two compiles at once). See the module
/// comment for what each guards.
#[test]
fn content_bindings_evaluate_and_imports_resolve_on_every_path() -> Outcome<()> {
	// Fixture 1: daimond-a's exact repro -- a doc root with an `#include`, importing a content function.
	let blocks1 = res!(assemble_blocks(&[
		("/__vfs__/main.typ",
			"#import \"tmpl.typ\": greet\n\n= Doc\n\n#greet(\"world\")\n\n#include \"ch1.typ\"\n"),
		("/__vfs__/tmpl.typ",
			"#let greet(who) = [Hello, #who.]\n"),
		("/__vfs__/ch1.typ",
			"== Chapter one\n\nBody of chapter one.\n"),
	]));

	assert!(has_heading(&blocks1, "Doc", 1),
		"the root heading `= Doc` must set, got: {:?}", blocks1);
	assert!(has_block_text(&blocks1, "Hello, world."),
		"`#greet(\"world\")` must expand to the paragraph `Hello, world.`, got: {:?}", blocks1);
	assert!(has_heading(&blocks1, "Chapter one", 2),
		"the included chapter's `== Chapter one` heading must set, got: {:?}", blocks1);
	assert!(has_block_text(&blocks1, "Body of chapter one."),
		"the included chapter's body must set, got: {:?}", blocks1);
	// The whole point: nothing leaks a raw `#` -- neither the dropped binding nor the unresolved call.
	for b in flatten(&blocks1) {
		let t = block_text(b);
		assert!(!t.trim_start().starts_with('#'),
			"a block leaked raw markup beginning with `#`: {:?}", t);
	}

	// Fixture 2: the lone path (no `#include`) must still walk the `#import` and expand a bare `#one`.
	let blocks2 = res!(assemble_blocks(&[
		("/__vfs__/main.typ",
			"#import \"one.typ\": one\n\n#one\n"),
		("/__vfs__/one.typ",
			"#let one = [ == Chapter one\n\nBody.\n ]\n"),
	]));

	assert!(has_heading(&blocks2, "Chapter one", 2),
		"the lone path must walk `#import` and the bare `#one` must expand to its heading, got: {:?}", blocks2);
	assert!(has_block_text(&blocks2, "Body."),
		"the bare `#one` must expand to the binding's body, got: {:?}", blocks2);
	for b in flatten(&blocks2) {
		let t = block_text(b);
		assert!(!t.trim_start().starts_with('#'),
			"a block leaked raw markup beginning with `#`: {:?}", t);
	}

	Ok(())
}
