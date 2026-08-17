//! Edits a document or a spreadsheet that already exists, so an external reader can be asked whether
//! the result is still a file it recognises.
//!
//! ```text
//! cargo run -p oxedyne_fe2o3_file --example edit_office -- in.docx out.docx "old=>new"
//! cargo run -p oxedyne_fe2o3_file --example edit_office -- in.ods  out.ods  "Sheet1!B7=3.5"
//! cargo run -p oxedyne_fe2o3_file --example edit_office -- in.xlsx out.xlsx "!D2:=B2*C2"
//! ```
//!
//! A find-and-replace is `find=>replace`. A cell is `[sheet]!ref=value`, or `[sheet]!ref:=formula`,
//! and the sheet may be left empty for the first one.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_file::office::docx;
use oxedyne_fe2o3_file::office::edit::Find;
use oxedyne_fe2o3_file::office::odf;
use oxedyne_fe2o3_file::office::sheet::Ref;
use oxedyne_fe2o3_file::office::xlsx;

fn main() -> Outcome<()> {
	let args: Vec<String> = std::env::args().collect();
	if args.len() < 4 {
		return Err(err!("Usage: edit_office <in> <out> <edit> [edit ...]"; Invalid, Input));
	}
	let src = res!(std::fs::read(&args[1]), IO, File);
	let asks = &args[3..];
	let out = match args[1].rsplit('.').next().unwrap_or("") {
		"docx"	=> {
			let e = res!(docx::edit::edit(&src, &res!(finds(asks))));
			println!("{} runs rewritten", e.runs);
			e.bytes
		}
		"odt"	=> {
			let e = res!(odf::text::edit(&src, &res!(finds(asks))));
			println!("{} runs rewritten", e.runs);
			e.bytes
		}
		"xlsx"	=> {
			let sets = res!(cells(asks));
			let sets: Vec<xlsx::edit::Set> = sets.into_iter()
				.map(|(sheet, at, value, formula)| xlsx::edit::Set { sheet, at, value, formula })
				.collect();
			let e = res!(xlsx::edit::edit(&src, &sets));
			println!("{} cells written to {}", e.cells, e.sheets.join(", "));
			e.bytes
		}
		"ods"	=> {
			let sets = res!(cells(asks));
			let sets: Vec<odf::sheet::Set> = sets.into_iter()
				.map(|(sheet, at, value, formula)| odf::sheet::Set { sheet, at, value, formula })
				.collect();
			let e = res!(odf::sheet::edit(&src, &sets));
			println!("{} cells written to {}", e.cells, e.sheets.join(", "));
			e.bytes
		}
		other	=> return Err(err!("'{}' is not a format this edits.", other; Invalid, Input)),
	};
	res!(std::fs::write(&args[2], &out), IO, File);
	println!("{}: {} bytes", args[2], out.len());
	Ok(())
}

/// The find-and-replace edits a `find=>replace` argument asks for.
fn finds(asks: &[String]) -> Outcome<Vec<Find>> {
	let mut out = Vec::new();
	for a in asks {
		let (find, replace) = res!(a.split_once("=>").ok_or_else(|| err!(
			"'{}' is not an edit. One looks like `old=>new`.", a; Invalid, Input)));
		out.push(Find::every(find, replace));
	}
	Ok(out)
}

/// The cells a `[sheet]!ref=value` argument asks for.
fn cells(asks: &[String]) -> Outcome<Vec<(Option<String>, Ref, Option<String>, Option<String>)>> {
	let mut out = Vec::new();
	for a in asks {
		let (sheet, rest) = res!(a.split_once('!').ok_or_else(|| err!(
			"'{}' is not a cell. One looks like `Sheet1!B2=3.5`.", a; Invalid, Input)));
		let sheet = match sheet.is_empty() {
			true	=> None,
			false	=> Some(sheet.to_string()),
		};
		match rest.split_once(":=") {
			Some((at, f))	=> out.push((sheet, res!(Ref::parse(at)), None, Some(f.to_string()))),
			None		=> {
				let (at, v) = res!(rest.split_once('=').ok_or_else(|| err!(
					"'{}' names no value. One looks like `Sheet1!B2=3.5`.", a; Invalid, Input)));
				out.push((sheet, res!(Ref::parse(at)), Some(v.to_string()), None));
			}
		}
	}
	Ok(out)
}
