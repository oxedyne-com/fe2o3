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
pub mod set;

use crate::doc::Block;
use crate::doc::Segment;

pub use parse::Refusal;
pub use parse::RefusalClass;
pub use parse::Refusals;

use oxedyne_fe2o3_core::prelude::*;

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
