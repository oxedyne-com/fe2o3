//! SpreadsheetML: the `.xlsx` half of the Office formats.
//!
//! # What a `.xlsx` is, in one paragraph
//!
//! A ZIP holding `xl/workbook.xml`, which names the sheets and points at them; one
//! `xl/worksheets/sheetN.xml` per sheet, which holds the cells; `xl/sharedStrings.xml`, which holds
//! every distinct string in the book once; and `xl/styles.xml`, which is where a number turns out to
//! be a date. Plus the two parts every Office document has, `[Content_Types].xml` and `_rels/.rels`.
//!
//! Inside a sheet the nesting is `worksheet > sheetData > row > c > v`: rows of cells, each carrying a
//! value and, where there is one, an `<f>` holding the formula that produced it.
//!
//! # Three things about this format are traps and all three are handled here
//!
//! **A string is not in the cell.** `<c r="A1" t="s"><v>4</v></c>` does not hold the number four; it
//! holds shared string number four. A reader that took the `<v>` at face value returns a column of
//! small integers where the names were.
//!
//! **A date is not a date.** It is a number, and the only thing that makes it a date is the number
//! format its style points at. Reading a sheet without reading `xl/styles.xml` turns every date in it
//! into a five-digit integer.
//!
//! **A cell is not where its position says.** Rows and cells are sparse and carry their own
//! addresses: a row may be missing entirely, and `<c r="D3">` may follow `<c r="A3">` with nothing
//! between. A reader that pushed cells onto the end of a row puts every value after a gap in the wrong
//! column, which is the kind of wrong that looks right.
//!
//! # The stored value is the value
//!
//! Nothing here recalculates a formula. See [`crate::office::sheet`] for why that is the correct
//! answer rather than a shortcut.

pub mod read;
pub mod write;

pub use read::read;
pub use write::write;

/// The SpreadsheetML namespace, which nearly every element of a workbook is in.
pub const NS_S: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
