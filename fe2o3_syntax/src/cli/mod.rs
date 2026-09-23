//! Command-line helpers that a `fe2o3_syntax` caller reaches for once a message has been
//! parsed: launching an external editor, and asking an interactive yes/no question. Parsing
//! stays in the crate's other modules; this is where a CLI *runs* something.

pub mod editor;
pub mod interactive;
