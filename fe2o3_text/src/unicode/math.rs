//! The mathematical character classes of Unicode Technical Report #25, revision 15, which decide
//! how a character spaces and stretches in a formula: `+` is Vary (binary between operands, unary
//! before one), `(` is Opening, `∑` is Large.
//!
//! The classes are not part of the UCD proper, so the table in [`tables::math`](super::tables::math)
//! comes from `MathClass-15.txt`, fetched by the same generator as the rest.
//!
//! ```
//! use oxedyne_fe2o3_text::unicode::math::{class, MathClass};
//!
//! assert_eq!(class('+'), Some(MathClass::Vary));
//! assert_eq!(class('('), Some(MathClass::Opening));
//! assert_eq!(class('😃'), None);
//! ```

use crate::unicode::lookup;
use crate::unicode::tables::math::{
	MATH_STARTS,
	MATH_VALS,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum MathClass {
	Normal,
	Alphabetic,
	Binary,
	Closing,
	Diacritic,
	Fence,
	GlyphPart,
	Large,
	Opening,
	Punctuation,
	Relation,
	Space,
	Unary,
	Vary,
	Special,
}

// In the order of the generator's class letters, N A B C D F G L O P R S U V X.
const CLASSES: [MathClass; 15] = [
	MathClass::Normal,
	MathClass::Alphabetic,
	MathClass::Binary,
	MathClass::Closing,
	MathClass::Diacritic,
	MathClass::Fence,
	MathClass::GlyphPart,
	MathClass::Large,
	MathClass::Opening,
	MathClass::Punctuation,
	MathClass::Relation,
	MathClass::Space,
	MathClass::Unary,
	MathClass::Vary,
	MathClass::Special,
];

/// The class of `c`, or `None` for a character the report does not classify.
pub fn class(c: char) -> Option<MathClass> {
	match lookup::flags(&MATH_STARTS, &MATH_VALS, c) {
		0	=> None,
		n	=> CLASSES.get(n as usize - 1).copied(),
	}
}
