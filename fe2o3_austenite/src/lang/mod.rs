//! The Ingot front end: the human-written source language above the block layer.
//!
//! An Ingot file is line-oriented markup. [`parse::document`] reads it into the surface tree of
//! [`ast::Item`], and [`lower::blocks`] maps that tree onto [`doc::Block`](crate::doc::Block), the
//! authoring vocabulary the two-pass driver already sets. The two steps are kept apart so a later
//! increment can grow the surface language -- richer parse, same lowering seam -- without disturbing
//! the block layer beneath it.
//!
//! The markup spine is headings, paragraphs, inline emphasis (`*strong*`, `/emph/`), bullet and
//! numbered lists, and the declared cross-references `#ref(<label>).page` and `#total-pages()` -- a
//! heading is labelled with a trailing `<name>`. The general `#` code mode of `sec_language` is a later
//! increment, so a `#` that opens neither reference form is an ordinary literal character.

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
