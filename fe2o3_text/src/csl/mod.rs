//! Citation Style Language 1.0.2: styles, locales and the processor that renders citations and
//! bibliographies with them.
//!
//! The target is Typst 0.15.1's behaviour, which is hayagriva 0.10.1's, and where hayagriva departs
//! from the CSL specification this follows hayagriva. Each departure is noted where it is made.
pub mod archive;
pub mod locale;
pub mod style;

pub use locale::{
	Chain,
	Locale,
	Locales,
};
pub use style::Style;
