//! Do general `#let` content bindings evaluate, and does `#import` resolve on every compile path -- and do
//! the pathological forms a live editor can type get refused rather than hang, lose words, or leak raw `#`?
//!
//! These tests drive the SAME [`compile::assemble`] the wasm surface calls, over an injected [`crate::vfs`]
//! source map, so they are the native proof of the browser behaviour -- exactly as `font_inject.rs` is for
//! injected fonts. The source-map global is process-wide, so a [`Mutex`] serialises the compiles (the
//! harness's own guard against two installs racing); a non-existent `/__vfs__/` prefix keeps a map miss from
//! falling through to a real file on the build host.

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
use std::sync::{
	Arc,
	Mutex,
};

/// Serialises access to the process-wide source-map global, so the tests in this binary compile one at a
/// time rather than racing on the shared map.
static VFS_LOCK: Mutex<()> = Mutex::new(());

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
/// blocks and the names of every construct the reader refused. Mirrors the wasm surface: the map is
/// installed, [`compile::assemble`] runs with the embedded Libertinus as the lone-file reading set, and the
/// map is cleared afterwards whatever the outcome. Serialised on [`VFS_LOCK`].
fn assemble_full(sources: &[(&str, &str)]) -> Outcome<(Vec<Block>, Vec<String>)> {
	let _guard = VFS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	let mut files: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	for (path, src) in sources {
		files.insert(PathBuf::from(path), src.as_bytes().to_vec());
	}
	res!(vfs::install(files));
	let fonts	= Arc::new(res!(fonts::libertinus()));
	let outcome	= compile::assemble(&PathBuf::from("/__vfs__/main.typ"), || Ok(fonts.clone()));
	let _		= vfs::clear();
	let (assembled, refusals, _skip) = res!(outcome);
	let names = refusals.entries().into_iter().map(|(n, _)| n).collect();
	Ok((assembled.blocks, names))
}

/// [`assemble_full`] keeping only the blocks.
fn assemble_blocks(sources: &[(&str, &str)]) -> Outcome<Vec<Block>> {
	Ok(res!(assemble_full(sources)).0)
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

/// No (flattened) block leaks raw markup beginning with `#`.
fn assert_no_hash_leak(blocks: &[Block]) -> Outcome<()> {
	for b in flatten(blocks) {
		let t = block_text(b);
		if t.trim_start().starts_with('#') {
			return Err(err!("a block leaked raw markup beginning with `#`: {:?}", t; Test, Mismatch));
		}
	}
	Ok(())
}

/// Content bindings evaluate and imports resolve on both paths. Fixture 1 is daimond-a's repro (a doc root
/// with an `#include`, importing a content function); fixture 2 is the lone path (a bare `#one` against an
/// imported value binding) -- the two things the lone path could not do before. Without this lane's change
/// fixture 1 yields only "Doc".
#[test]
fn content_bindings_evaluate_and_imports_resolve_on_every_path() -> Outcome<()> {
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
	res!(assert_no_hash_leak(&blocks1));

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
	res!(assert_no_hash_leak(&blocks2));
	Ok(())
}

/// A self- or mutually-referential binding must be refused at the expansion-depth cap, not recurse until the
/// stack overflows: reaching this assertion at all proves it terminated. (Risk 1 -- the blocker.)
#[test]
fn content_binding_cycle_is_refused_and_terminates() -> Outcome<()> {
	// Self-cycle: `#let a = [#a]` referenced as a bare `#a`.
	let (blocks, names) = res!(assemble_full(&[
		("/__vfs__/main.typ",	"#import \"a.typ\": a\n\n#a\n"),
		("/__vfs__/a.typ",		"#let a = [#a]\n"),
	]));
	assert!(names.iter().any(|n| n.contains("cycle")),
		"a self-referential binding must record a cycle refusal, got refusals: {:?}", names);
	res!(assert_no_hash_leak(&blocks));

	// Mutual cycle: `#let a = [#b]` and `#let b = [#a]`.
	let (blocks, names) = res!(assemble_full(&[
		("/__vfs__/main.typ",	"#import \"m.typ\": a\n\n#a\n"),
		("/__vfs__/m.typ",		"#let a = [#b]\n#let b = [#a]\n"),
	]));
	assert!(names.iter().any(|n| n.contains("cycle")),
		"a mutually-referential pair must record a cycle refusal, got refusals: {:?}", names);
	res!(assert_no_hash_leak(&blocks));
	Ok(())
}

/// An inline-shaped content function used mid-line -- `#em[Note] the rest.` -- must not discard the trailing
/// prose: the only-whitespace-after rule keeps the reference from being taken as a standalone splice, so the
/// sentence's tail survives in the paragraph. (Risk 3 -- silent word loss.)
#[test]
fn inline_shaped_content_fn_keeps_trailing_prose() -> Outcome<()> {
	let blocks = res!(assemble_blocks(&[
		("/__vfs__/main.typ",
			"#import \"e.typ\": em\n\n#em[Note] the rest of this sentence survives.\n"),
		("/__vfs__/e.typ",
			"#let em(x) = [_#x_]\n"),
	]));
	let joined: String = flatten(&blocks).into_iter().map(block_text).collect::<Vec<_>>().join(" ");
	assert!(joined.contains("the rest of this sentence survives."),
		"the trailing prose after `#em[Note]` must survive, got: {:?}", blocks);
	res!(assert_no_hash_leak(&blocks));
	Ok(())
}

/// A code-mode form surfacing in a re-read content-binding body -- `#if`/`#for` -- must be refused with a
/// span, not leaked onto the page with its leading `#`. (Risk 4 -- code-mode leak.)
#[test]
fn code_mode_form_in_expanded_body_is_refused_not_leaked() -> Outcome<()> {
	let (blocks, names) = res!(assemble_full(&[
		("/__vfs__/main.typ",	"#import \"c.typ\": cond\n\n#cond\n"),
		("/__vfs__/c.typ",		"#let cond = [#if x [yes] else [no]]\n"),
	]));
	assert!(names.iter().any(|n| n.starts_with("#if")),
		"an expanded body's `#if` must be recorded as a refusal, got refusals: {:?}", names);
	res!(assert_no_hash_leak(&blocks));

	// A `#for` loop likewise.
	let (blocks, names) = res!(assemble_full(&[
		("/__vfs__/main.typ",	"#import \"f.typ\": loop\n\n#loop\n"),
		("/__vfs__/f.typ",		"#let loop = [#for i in range(3) [item]]\n"),
	]));
	assert!(names.iter().any(|n| n.starts_with("#for")),
		"an expanded body's `#for` must be recorded as a refusal, got refusals: {:?}", names);
	res!(assert_no_hash_leak(&blocks));
	Ok(())
}
