//! Writes the three OpenDocument formats from one Markdown file, so an external reader can be asked
//! what it makes of each.
//!
//! ```text
//! cargo run -p oxedyne_fe2o3_file --example make_odf -- in.md out_prefix
//! soffice --headless --convert-to txt out_prefix.odt
//! ```
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_file::office::deck::Deck;
use oxedyne_fe2o3_file::office::odf;
use oxedyne_fe2o3_file::office::sheet::{Book, Cell, Sheet, Value};
use oxedyne_fe2o3_text::doc::markdown;

fn main() -> Outcome<()> {
	let args: Vec<String> = std::env::args().collect();
	if args.len() != 3 {
		return Err(err!("Usage: make_odf <markdown in> <output prefix>"; Invalid, Input));
	}
	let src = res!(std::fs::read_to_string(&args[1]), IO, File);
	let doc = res!(markdown::parse(&src));

	let (odt, left) = res!(odf::text::write(&doc));
	res!(std::fs::write(fmt!("{}.odt", args[2]), &odt), IO, File);
	println!("{}.odt: {} bytes", args[2], odt.len());
	for s in &left.images {
		println!("  not carried: the image at {}", s);
	}

	let (odp, left) = res!(odf::slides::write(&Deck::from_doc(&doc)));
	res!(std::fs::write(fmt!("{}.odp", args[2]), &odp), IO, File);
	println!("{}.odp: {} bytes", args[2], odp.len());
	if left.notes > 0 {
		println!("  not carried: speaker's notes on {} slide(s)", left.notes);
	}

	// A spreadsheet has no Markdown to come from, so it is built here: one of everything that
	// separates the format from a table.
	let mut s = Sheet::new("Sales");
	s.rows.push(vec![Cell::text("Region"), Cell::text("Units"), Cell::text("Price"),
		Cell::text("Total"), Cell::text("Booked"), Cell::text("Active")]);
	s.rows.push(vec![Cell::text("North"), Cell::number(120.0), Cell::number(3.4),
		Cell::formula("B2*C2", Value::Number(408.0)),
		Cell { value: Value::Date("2026-03-14".to_string()), formula: None },
		Cell::bool(true)]);
	s.rows.push(vec![Cell::text("South"), Cell::number(85.0), Cell::number(11.0),
		Cell::formula("B3*C3", Value::Number(935.0)),
		Cell { value: Value::Date("2026-04-01".to_string()), formula: None },
		Cell::bool(false)]);
	s.rows.push(Vec::new());
	s.rows.push(vec![Cell::text("Total"), Cell::empty(), Cell::empty(),
		Cell::formula("SUM(D2:D3)", Value::Number(1343.0))]);
	let ods = res!(odf::sheet::write(&Book { sheets: vec![s] }));
	res!(std::fs::write(fmt!("{}.ods", args[2]), &ods), IO, File);
	println!("{}.ods: {} bytes", args[2], ods.len());
	Ok(())
}
