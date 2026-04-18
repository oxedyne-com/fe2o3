//! Anneal -- the Oxedyne code formatter.
//!
//! A language-aware code formatter built on Wadler-style layout
//! algebra. The architecture separates three concerns:
//!
//! - **Lexing**: source text to token stream (per-language, data-driven).
//! - **Parsing**: token stream to concrete syntax tree (structural, keyword-aware).
//! - **Formatting**: CST to layout document to formatted text (universal algebra).
//!
//! The layout algebra (the `Doc` type) is a small set of combinators
//! that can express every formatting pattern across every language.
//! The renderer walks the document and makes optimal line-breaking
//! decisions within a given width.
//!
//! # Usage
//!
//! ```ignore
//! use oxedyne_fe2o3_text::fmt::{format_rust, spec::FormatSpec};
//!
//! let source = "fn main ( ) { println!(\"hello\") ; }";
//! let spec = FormatSpec::fe2o3();
//! let formatted = res!(format_rust(source, &spec));
//! ```
//!
pub mod doc;
pub mod cst;
pub mod lex;
pub mod parse;
pub mod spec;
pub mod render;
pub mod format;

use crate::fmt::spec::FormatSpec;

use oxedyne_fe2o3_core::prelude::*;


/// Format Rust source code according to the given specification.
pub fn format_rust(source: &str, spec: &FormatSpec) -> Outcome<String> {
    format::format_rust(source, spec)
}
