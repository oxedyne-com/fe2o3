//! Writes a `.xlsx` holding one of everything, so an external reader can be asked what it makes of it.
//!
//! A spreadsheet is the format where reading your own output proves least, because the interesting
//! cases -- a shared string, a date that is really a number, a formula whose cached value is what
//! everyone sees -- are all conventions rather than structure. A file that satisfies this crate's own
//! reader may still open in Excel as five columns of five-digit integers.
//!
//! ```text
//! cargo run -p oxedyne_fe2o3_file --example make_xlsx -- out.xlsx
//! soffice --headless --convert-to csv out.xlsx
//! ```
//!
//! `dev/xlsx_oracle.sh` does that and prints what came back.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_file::office::sheet::{
	Book,
	Cell,
	Sheet,
	Value,
};
use oxedyne_fe2o3_file::office::xlsx;

fn main() -> Outcome<()> {
	let args: Vec<String> = std::env::args().collect();
	if args.len() != 2 {
		return Err(err!("Usage: make_xlsx <xlsx out>"; Invalid, Input));
	}
	let mut sales = Sheet::new("Sales");
	sales.rows.push(vec![
		Cell::text("Region"), Cell::text("Units"), Cell::text("Price"), Cell::text("Total"),
		Cell::text("Booked"), Cell::text("Active"),
	]);
	// A formula and the value the last calculation left beside it. Both are written; a formula with
	// no cached value shows as blank in every reader that does not calculate.
	sales.rows.push(vec![
		Cell::text("North"), Cell::number(120.0), Cell::number(3.4),
		Cell::formula("B2*C2", Value::Number(408.0)),
		Cell { value: Value::Date("2026-03-14".to_string()), formula: None },
		Cell::bool(true),
	]);
	sales.rows.push(vec![
		Cell::text("South"), Cell::number(85.0), Cell::number(11.0),
		Cell::formula("B3*C3", Value::Number(935.0)),
		Cell { value: Value::Date("2026-04-01".to_string()), formula: None },
		Cell::bool(false),
	]);
	// A gap, then a total: the sparse case, which is where a reader that counts positions instead of
	// reading addresses puts everything one column out.
	sales.rows.push(Vec::new());
	sales.rows.push(vec![
		Cell::text("Total"), Cell::empty(), Cell::empty(),
		Cell::formula("SUM(D2:D3)", Value::Number(1343.0)),
	]);
	// A string that is only spaces, and one with markup in it: both are text and neither is markup.
	let mut notes = Sheet::new("Notes");
	notes.rows.push(vec![Cell::text("   "), Cell::text("a < b & c"), Cell::text("3.40")]);

	let book = Book { sheets: vec![sales, notes] };
	let bytes = res!(xlsx::write(&book));
	res!(std::fs::write(&args[1], &bytes), IO, File);
	println!("{}: {} bytes, {} sheets", args[1], bytes.len(), book.sheets.len());
	Ok(())
}
