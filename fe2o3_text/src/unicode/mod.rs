//! Unicode text algorithms, implemented over tables generated from the Unicode Character Database.
//!
//! The crate carries an implementation of four of the Unicode annexes, each of which a document
//! renderer needs:
//!
//! - [`norm`], the normalisation forms of UAX #15. NFC and NFD, and their compatibility variants
//!   NFKC and NFKD.
//! - [`linebreak`], the line breaking algorithm of UAX #14, which yields the byte offsets at which
//!   a line may or must be broken.
//! - [`segment`], the grapheme cluster and word boundaries of UAX #29, which give a cursor
//!   somewhere to land.
//! - [`bidi`], the bidirectional algorithm of UAX #9, which resolves embedding levels and the
//!   visual order of a paragraph.
//!
//! The tables in [`tables`] are generated and committed, never fetched at build or run time. The
//! Unicode version they come from is [`UCD_VERSION`]; the generator is
//! `fe2o3_text/src/bin/gen_unicode.rs`, and it also vendors the Unicode Consortium conformance
//! files that `tests/unicode.rs` runs against.
//!
//! ```
//! use oxedyne_fe2o3_text::unicode::norm;
//!
//! assert_eq!(norm::nfc("e\u{0301}"), "é");
//! assert_eq!(norm::nfd("é"), "e\u{0301}");
//! ```

pub mod bidi;
pub mod linebreak;
pub mod lookup;
pub mod norm;
pub mod segment;
pub mod tables;

pub use tables::prop;
pub use tables::UCD_VERSION;
