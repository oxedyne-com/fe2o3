//! The source front end: Austenite reads Typst markup, so an existing Typst document sets without
//! being rewritten in a new language. [`parse::document`] reads a source string into the surface tree
//! of [`ast::Item`], and [`lower::blocks`] maps that tree onto [`doc::Block`](crate::doc::Block), the
//! authoring vocabulary the two-pass driver already sets. The two steps are kept apart so the surface
//! can grow -- richer parse, same lowering seam -- without disturbing the block layer beneath it.
//!
//! The markup implemented so far is Typst's: headings (`=`), paragraphs, `*strong*` and `_emph_`,
//! bullet (`-`) and numbered (`+`) lists, and the `@label` cross-reference, with a heading labelled by a
//! trailing `<name>` and `\` escaping the next character. Typst code statements -- `#import`, `#let`,
//! `#set`, `#show` -- and whole-line calls to template functions are skipped for now: the styling and
//! computation layer, and inline `$maths$`, code and `#figure`/`#image`, are later increments.

pub mod ast;
pub mod codefig;
pub mod lower;
pub mod mathparse;
pub mod parse;
pub mod rules;
pub mod set;

use crate::doc::Block;
use crate::doc::Segment;

pub use parse::Refusal;
pub use parse::RefusalClass;
pub use parse::Refusals;

use oxedyne_fe2o3_core::prelude::*;

/// The 1-based line and column a byte offset falls on within `src`, and the full text of that line (its
/// trailing newline trimmed). The column is a byte offset within the line, not a character count, matching
/// [`crate::ir::Span`]'s own byte-based accounting. Shared by the native binary's `--explain` caret and the
/// wasm surface's `file:line:col` diagnostics, so a refused site reads back to the same position on both.
pub fn line_col_of(src: &str, offset: u32) -> (usize, usize, &str) {
	let offset = (offset as usize).min(src.len());
	let mut line_no		= 1usize;
	let mut line_start	= 0usize;
	for (i, b) in src.bytes().enumerate() {
		if i >= offset {
			break;
		}
		if b == b'\n' {
			line_no += 1;
			line_start = i + 1;
		}
	}
	let line_end = src[line_start..].find('\n').map(|p| line_start + p).unwrap_or(src.len());
	let col = offset.saturating_sub(line_start) + 1;
	(line_no, col, &src[line_start..line_end])
}

/// Reads one run of Typst inline markup -- prose with `*strong*`, `_emph_`, a maths span or a glossary
/// term -- into the [`Segment`]s the block layer sets, without a surrounding block. The book layer uses
/// it to turn a `term-defs` definition (Typst content, `[...]`) into the runs of a glossary table cell.
pub fn inline_segments(text: &str) -> Vec<Segment> {
	lower::lower_runs(&parse::parse_inlines(text))
}

/// Parses Typst source and lowers it to the block list the driver authors from, in one step. The usual
/// entry point: a caller that wants the surface tree in between reaches for [`parse::document`] and
/// [`lower::blocks`] directly, and one that wants the report of skipped constructs reaches for
/// [`to_blocks_with_refusals`].
pub fn to_blocks(src: &str) -> Outcome<Vec<Block>> {
	let (blocks, _) = res!(to_blocks_with_refusals(src));
	Ok(blocks)
}

/// Parses and lowers as [`to_blocks`], and alongside the blocks returns the [`Refusals`] naming every
/// construct the reader passed over -- a `#let`/`#set`/`#show`/`#import` line, an unknown standalone or
/// inline `#func` call, a `#columns` wrapper. A caller prints the summary so a dropped construct is a
/// visible report ("skipped 3 unsupported constructs: #show (2), #columns (1)") rather than a silent gap.
pub fn to_blocks_with_refusals(src: &str) -> Outcome<(Vec<Block>, Refusals)> {
	let (items, skips) = res!(parse::document_with_refusals(src));
	Ok((lower::blocks(&items), skips))
}

/// As [`to_blocks_with_refusals`], with a set of bound `#let` furniture functions in scope: a call to one
/// (`#pr-note[ ... ]`, `#aside-box(title: [..])[ ... ]`) expands into a padded box rather than a skip. The
/// book assembler collects the definitions once and threads them into every chapter it reads.
pub fn to_blocks_with_templates(src: &str, tfns: &rules::TemplateFns) -> Outcome<(Vec<Block>, Refusals)> {
	let (items, skips) = res!(parse::document_with_templates(src, tfns));
	Ok((lower::blocks(&items), skips))
}

#[cfg(test)]
mod tests {
	use super::line_col_of;

	/// A pure check of the byte-offset-to-line/column arithmetic the `--explain` caret and the wasm
	/// diagnostics both depend on, with no file involved: the third line, its fifth byte (the `d` of "third").
	#[test]
	fn line_col_of_finds_the_right_line_and_column() {
		let src = "first\nsecond\nthird line\n";
		let offset = src.find("d line").expect("fixture text") as u32;
		let (line_no, col, text) = line_col_of(src, offset);
		assert_eq!(line_no, 3, "wrong line for offset {}", offset);
		assert_eq!(col, 5, "wrong column for offset {}", offset);
		assert_eq!(text, "third line");
	}

	/// The very first byte reports line 1, column 1 -- the boundary a fencepost error would miss.
	#[test]
	fn line_col_of_handles_the_first_byte() {
		let (line_no, col, text) = line_col_of("hello\nworld\n", 0);
		assert_eq!((line_no, col), (1, 1));
		assert_eq!(text, "hello");
	}
}
