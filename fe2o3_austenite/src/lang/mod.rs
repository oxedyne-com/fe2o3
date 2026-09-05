//! The Ingot front end: the human-written source language above the block layer.
//!
//! An Ingot file is line-oriented markup. [`parse::document`] reads it into the surface tree of
//! [`ast::Item`], and [`lower::blocks`] maps that tree onto [`doc::Block`](crate::doc::Block), the
//! authoring vocabulary the two-pass driver already sets. The two steps are kept apart so a later
//! increment can grow the surface language -- richer parse, same lowering seam -- without disturbing
//! the block layer beneath it.
//!
//! Increment 1 is the markup spine: headings and paragraphs only. The `#` code mode, inline emphasis
//! (`*strong*`, `/emph/`), and the declared-query references of `sec_language` are later increments,
//! so this front end treats `#`, `*` and `/` as ordinary literal characters.

pub mod ast;
pub mod lower;
pub mod parse;

use crate::doc::Block;

use oxedyne_fe2o3_core::prelude::*;

/// Parses Ingot source and lowers it to the block list the driver authors from, in one step. The
/// usual entry point: a caller that wants the surface tree in between reaches for [`parse::document`]
/// and [`lower::blocks`] directly.
pub fn to_blocks(src: &str) -> Outcome<Vec<Block>> {
	let items = res!(parse::document(src));
	Ok(lower::blocks(&items))
}
