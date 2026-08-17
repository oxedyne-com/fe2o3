//! Writing cells into a `.xlsx` that already exists, one cell at a time.
//!
//! # A sheet is sparse, so a write is an insertion and not an assignment
//!
//! There is no array to index. A row may be absent, and within a row `<c r="D3">` may follow
//! `<c r="A3">` with nothing between, so writing `B3` means finding where `B3` WOULD go and putting it
//! there -- among the cells of row 3 if that row exists, and as a new row in the right place among the
//! rows if it does not. A writer that appended would put the value in the wrong column, or the wrong
//! row, and the file would still look like a file.
//!
//! # Two things are rewritten besides the cell, and both of them have to be
//!
//! `<dimension>` says the rectangle the sheet occupies, and a cell written outside it is a cell some
//! readers do not draw. It is widened.
//!
//! `xl/calcChain.xml` is Excel's record of what order to recalculate in, and it names cells by
//! position. A formula written into a cell the chain does not know about makes the chain wrong, and
//! Excel's response to a wrong chain is a repair prompt -- which "fixes" the file and never says why.
//! So the part is DELETED, along with its content-type override and its relationship, and Excel
//! rebuilds it. LibreOffice never reads it at all.
//!
//! **The chain goes wrong in BOTH directions and both of them drop it.** A formula WRITTEN leaves the
//! chain short of a cell. A formula DESTROYED -- a plain value put over a cell that held an `<f>`,
//! which is one call to this module and no formula anywhere in it -- leaves the chain naming a cell
//! that has none, and ECMA-376 Part 1 §18.6.1 says a `c` in the chain is "a single cell, which shall
//! contain a formula". The second case is not a lesser version of the first; it is a spec violation
//! where the first is only a stale hint, and it went unnoticed until the ECMA-376 schemas were pointed
//! at the output, because no reader on this machine reads the part. An EMPTY `<calcChain/>` would be no
//! better -- `CT_CalcChain` requires at least one `c` -- so the part is removed rather than emptied.
//!
//! # A written formula carries no cached value unless the caller supplies one
//!
//! Nothing here calculates -- see [`crate::office::sheet`] on why that is the correct answer -- so a
//! formula goes in with no `<v>` beside it and the reader computes it on open. The alternative would be
//! to leave the value that was there, which is the value of the OLD formula, and a cell showing a
//! number that does not follow from the expression above it is worse than one showing nothing.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::office::opc;
use crate::office::sheet::{
	Ref,
	Value,
	stored,
	typed,
};
use crate::zip::{
	Method,
	Zip,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::xml::{
	Elem,
	Span,
	Xml,
};
use oxedyne_fe2o3_text::xml::write::{
	escape,
	escape_attr,
};

/// One cell to write.
#[derive(Clone, Debug)]
pub struct Set {
	pub sheet:	Option<String>,	// by the name on the tab; None means the first
	pub at:	Ref,
	// The value as a person would type it. `sheet::typed` decides what it is; an empty string
	// empties the cell.
	pub value:	Option<String>,
	pub formula:	Option<String>,	// without its leading `=`, which is stripped if present
}

/// What an edit of a `.xlsx` produced.
#[derive(Clone, Debug, Default)]
pub struct Edited {
	pub bytes:	Vec<u8>,
	pub cells:	usize,
	pub sheets:	Vec<String>,	// the tabs that were touched, in the order they were
}

/// Writes cells into a `.xlsx`, leaving every part of the archive it did not have to change.
pub fn edit(bytes: &[u8], sets: &[Set]) -> Outcome<Edited> {
	if sets.is_empty() {
		return Err(err!("A write to a spreadsheet was asked for with no cells in it."; Invalid, Input));
	}
	let cap = super::read::MAX_PART;
	let mut zip = res!(Zip::read(bytes.to_vec()));
	let wb_part = res!(opc::main_part(&zip, cap));
	let dir = opc::dir_of(&wb_part);
	let rels = res!(opc::rels_of(&zip, &wb_part, cap));
	let wb_src = res!(String::from_utf8(res!(zip.content_capped(&wb_part, cap))), Decode, String);
	let wb = res!(Xml::parse(&wb_src));

	// The tabs, in tab order, each with the part that holds it. A sheet the workbook names and whose
	// part is missing is not writable, and saying so by name beats writing into the wrong one.
	let mut tabs: Vec<(String, String)> = Vec::new();
	if let Some(list) = res!(wb.root()).child("sheets") {
		for s in list.children("sheet") {
			let name = s.attr("name").unwrap_or("Sheet").to_string();
			let target = s.attr("r:id")
				.and_then(|id| rels.get(id))
				.filter(|(kind, _)| kind == opc::REL_SHEET)
				.map(|(_, t)| t.clone())
				.filter(|t| zip.has(t));
			if let Some(t) = target {
				tabs.push((name, t));
			}
		}
	}
	if tabs.is_empty() {
		return Err(err!(
			"This workbook has no sheet whose part is in the package, so there is nowhere to write.";
			Invalid, Input, Missing));
	}
	let names: Vec<&str> = tabs.iter().map(|(n, _)| n.as_str()).collect();

	// Grouped by part, because one part is parsed, spliced and written once however many cells land
	// in it -- and because two sheets may share neither a row nor a dimension.
	let mut jobs: Vec<(String, String, Vec<&Set>)> = Vec::new();
	for set in sets {
		let (name, part) = match &set.sheet {
			None		=> tabs[0].clone(),
			Some(want)	=> {
				let found = tabs.iter()
					.find(|(n, _)| n == want)
					.or_else(|| tabs.iter().find(|(n, _)| n.eq_ignore_ascii_case(want)));
				res!(found.cloned().ok_or_else(|| err!(
					"This workbook has no sheet named '{}'. It has: {}. Nothing has been written.",
					want, names.join(", "); Invalid, Input, Missing)))
			}
		};
		match jobs.iter_mut().find(|(_, p, _)| *p == part) {
			Some(j)	=> j.2.push(set),
			None		=> jobs.push((name, part, vec![set])),
		}
	}
	// TWO WRITES TO ONE CELL ARE REFUSED, and it is worth saying why rather than picking one. There
	// is no order for them to be applied IN: nothing here reads a cell it has written, so "the last
	// one wins" would be a rule about the order a caller happened to build a list in. A caller that
	// means to overwrite its own value has changed its mind, and can say so in one write.
	for (name, _, sets) in &jobs {
		for (i, one) in sets.iter().enumerate() {
			if sets[..i].iter().any(|s| s.at == one.at) {
				return Err(err!(
					"{} of sheet '{}' is written twice in one call, and there is no order in which \
					to apply the two. Nothing has been written.", one.at.name(), name;
					Invalid, Input, Conflict));
			}
		}
	}

	let mut formulas = false;	// a formula was written
	let mut razed = false;	// a formula that was there is not any more
	let mut touched = Vec::new();
	let mut cells = 0usize;
	for (name, part, sets) in &jobs {
		let src = res!(String::from_utf8(res!(zip.content_capped(part, cap))), Decode, String);
		let mut xml = res!(Xml::parse(&src));
		razed = res!(write_cells(&mut xml, sets)) || razed;
		zip.set(part, xml.render().into_bytes(), Method::Deflate);
		touched.push(name.clone());
		cells += sets.len();
		formulas = formulas || sets.iter().any(|s| s.formula.is_some());
	}
	if formulas || razed {
		res!(drop_calc_chain(&mut zip, &dir, &wb_part, cap));
	}
	Ok(Edited { bytes: res!(zip.write()), cells, sheets: touched })
}

/// Puts every cell of one sheet where it belongs, as splices into the part's own bytes.
///
/// The answer says whether a cell that held an `<f>` was replaced by one that does not, which the
/// caller needs because it is the other half of what makes `xl/calcChain.xml` wrong.
fn write_cells(xml: &mut Xml, sets: &[&Set]) -> Outcome<bool> {
	let root = res!(xml.root()).clone();
	let data = res!(root.child("sheetData").cloned().ok_or_else(|| err!(
		"This sheet has no <sheetData>, so it is not a worksheet."; Invalid, Input, Missing)));
	let rows: Vec<Elem> = data.children("row").into_iter().cloned().collect();

	// One pass per ROW rather than per cell, because a row is what gets built or rebuilt: two cells
	// written into a `<row r="3"/>` that has nothing inside it are one replacement of that element,
	// and done a cell at a time they would be two splices over the same bytes.
	let mut by_row: Vec<(u32, Vec<&Set>)> = Vec::new();
	let mut edge = Ref { col: 0, row: 0 };
	for set in sets {
		edge = Ref { col: edge.col.max(set.at.col), row: edge.row.max(set.at.row) };
		match by_row.iter_mut().find(|(r, _)| *r == set.at.row) {
			Some(g)	=> g.1.push(set),
			None		=> by_row.push((set.at.row, vec![set])),
		}
	}
	by_row.sort_by_key(|(r, _)| *r);
	for (_, group) in by_row.iter_mut() {
		group.sort_by_key(|s| s.at.col);
	}

	let mut splices: Vec<(Span, String)> = Vec::new();
	let mut razed = false;
	for (at_row, group) in &by_row {
		let row = rows.iter().find(|r| {
			r.attr("r").and_then(|v| v.parse::<u32>().ok()) == Some(at_row + 1)
		});
		let (row, inner) = match row {
			// A row the sheet has not got is built whole and put in among the rows it has.
			None		=> {
				let body: String = group.iter()
					.map(|s| cell_markup(&s.at, None, s))
					.collect();
				let text = fmt!("<row r=\"{}\">{}</row>", at_row + 1, body);
				let after = rows.iter().find(|r| {
					r.attr("r").and_then(|v| v.parse::<u32>().ok())
						.map(|v| v > at_row + 1)
						.unwrap_or(false)
				});
				let at = match after {
					Some(r)	=> r.span.start..r.span.start,
					// An empty `<sheetData/>` has no inside; it is replaced by one that has.
					None		=> match &data.inner {
						Some(i)	=> i.end..i.end,
						None		=> {
							splices.push((data.span.clone(),
								fmt!("<sheetData>{}</sheetData>", text)));
							continue;
						}
					},
				};
				splices.push((at, text));
				continue;
			}
			// A `<row r="3"/>` is rebuilt from its own open tag, for the same reason.
			Some(r) if r.inner.is_none()	=> {
				let body: String = group.iter()
					.map(|s| cell_markup(&s.at, None, s))
					.collect();
				splices.push((r.span.clone(), fmt!("{}{}</row>", open_of(xml, r), body)));
				continue;
			}
			Some(r)	=> (r, res!(r.inner.clone().ok_or_else(|| err!(
				"A row was found to have an inside and then not to.";	Bug)))),
		};
		let cells = row.children("c");
		for set in group {
			let existing = cells.iter().find(|c| {
				c.attr("r").and_then(|a| Ref::parse(a).ok()).map(|p| p.col) == Some(set.at.col)
			});
			match existing {
				// The style stays on the cell. It is the user's formatting, and a value written over
				// a number with a currency format is still money.
				Some(c)	=> {
					// The whole `<c>` is replaced, so a formula on the old one is gone unless the
					// new one carries its own. This is the only place a formula can be destroyed.
					razed = razed || (c.child("f").is_some() && set.formula.is_none());
					splices.push((c.span.clone(), cell_markup(&set.at, c.attr("s"), set)));
				}
				None		=> {
					let after = cells.iter().find(|c| {
						c.attr("r").and_then(|a| Ref::parse(a).ok())
							.map(|p| p.col > set.at.col)
							.unwrap_or(false)
					});
					let at = match after {
						Some(c)	=> c.span.start,
						None		=> inner.end,
					};
					splices.push((at..at, cell_markup(&set.at, None, set)));
				}
			}
		}
	}

	if let Some((span, text)) = widened(&root, &edge) {
		splices.push((span, text));
	}
	// Ascending, because `Xml::splice` refuses one that overlaps an earlier one and two inserts at the
	// same offset are ordered by the order they are made.
	splices.sort_by_key(|(s, _)| (s.start, s.end));
	for (span, text) in splices {
		res!(xml.splice(span, text));
	}
	Ok(razed)
}

/// The `<dimension>`'s new `ref` value, where a cell was written outside the rectangle it claims.
///
/// The bottom right only. A sheet whose dimension starts at `B2` and which gains a cell at `A1` is
/// rare enough, and shrinking or moving the top left is how a dimension stops describing the sheet.
fn widened(root: &Elem, edge: &Ref) -> Option<(Span, String)> {
	let dim = root.child("dimension")?;
	let attr = dim.attrs.iter().find(|a| a.name.qname == "ref")?;
	let (from, to) = match attr.value.split_once(':') {
		Some((a, b))	=> (a, b),
		None		=> (attr.value.as_str(), attr.value.as_str()),
	};
	let from = Ref::parse(from).ok()?;
	let to = Ref::parse(to).ok()?;
	if to.col >= edge.col && to.row >= edge.row {
		return None;
	}
	let grown = Ref { col: to.col.max(edge.col), row: to.row.max(edge.row) };
	Some((attr.val_span.clone(), fmt!("{}:{}", from.name(), grown.name())))
}

/// One `<c>` element holding what the caller asked for.
fn cell_markup(at: &Ref, style: Option<&str>, set: &Set) -> String {
	let value = set.value.as_deref().map(typed);
	let mut out = fmt!("<c r=\"{}\"", at.name());
	if let Some(s) = style {
		out.push_str(&fmt!(" s=\"{}\"", escape_attr(s)));
	}
	// The type attribute describes the `<v>`, so a formula cell with no cached value carries none.
	let kind = match (&set.formula, &value) {
		(Some(_), None)		=> None,
		(_, Some(Value::Text(_)))	=> Some("inlineStr"),
		(_, Some(Value::Bool(_)))	=> Some("b"),
		(_, Some(Value::Error(_)))	=> Some("e"),
		_			=> None,
	};
	// A formula cell whose cached value is text says `str` rather than `inlineStr`: the latter puts the
	// text in an `<is>`, and a formula's result goes in the `<v>` like any other.
	let kind = match (&set.formula, kind) {
		(Some(_), Some("inlineStr"))	=> Some("str"),
		(_, k)			=> k,
	};
	if let Some(k) = kind {
		out.push_str(&fmt!(" t=\"{}\"", k));
	}
	let body = body_of(set, &value);
	match body.is_empty() {
		true	=> out.push_str("/>"),
		false	=> out.push_str(&fmt!(">{}</c>", body)),
	}
	out
}

/// What goes between a cell's tags: the formula, and the value where there is one.
fn body_of(set: &Set, value: &Option<Value>) -> String {
	let mut out = String::new();
	if let Some(f) = &set.formula {
		let f = f.trim_start_matches('=');
		if !f.is_empty() {
			out.push_str(&fmt!("<f>{}</f>", escape(f)));
		}
	}
	match value {
		None | Some(Value::Empty)	=> {}
		Some(Value::Number(n))	=> out.push_str(&fmt!("<v>{}</v>", stored(*n))),
		Some(Value::Bool(b))	=> out.push_str(match b {
			true	=> "<v>1</v>",
			false	=> "<v>0</v>",
		}),
		Some(Value::Error(e))	=> out.push_str(&fmt!("<v>{}</v>", escape(e))),
		Some(Value::Date(d))	=> out.push_str(&fmt!("<v>{}</v>", escape(d))),
		Some(Value::Text(t))	=> match set.formula.is_some() {
			true	=> out.push_str(&fmt!("<v>{}</v>", escape(t))),
			// `xml:space` is not a SpreadsheetML thing; `<t>` in an `<is>` takes it the same way
			// `<w:t>` does, and without it a value of " " is written and read back as "".
			false	=> out.push_str(&fmt!(
				"<is><t xml:space=\"preserve\">{}</t></is>", escape(t))),
		},
	}
	out
}

/// A row's open tag exactly as it was written, so rebuilding the element keeps its attributes.
fn open_of(xml: &Xml, row: &Elem) -> String {
	let raw = xml.raw(&row.open);
	// `<row r="3"/>` opens and closes at once; the rebuilt element needs the open tag alone.
	match raw.strip_suffix("/>") {
		Some(head)	=> fmt!("{}>", head),
		None		=> raw.to_string(),
	}
}

/// Takes `xl/calcChain.xml` out of the package, along with the two declarations that name it.
///
/// A part removed while `[Content_Types].xml` still declares it is a package Excel refuses, so the
/// override and the relationship go with it. See this module's own note on why it goes at all.
fn drop_calc_chain(zip: &mut Zip, dir: &str, wb_part: &str, cap: u64) -> Outcome<()> {
	let chain = fmt!("{}calcChain.xml", dir);
	if !zip.remove(&chain) {
		return Ok(());
	}
	// The override names the part with a leading slash, which is how a package addresses one.
	let types = "[Content_Types].xml";
	if zip.has(types) {
		let src = res!(String::from_utf8(res!(zip.content_capped(types, cap))), Decode, String);
		let mut xml = res!(Xml::parse(&src));
		let want = fmt!("/{}", chain);
		let gone: Vec<Span> = res!(xml.root()).children("Override").iter()
			.filter(|o| o.attr("PartName") == Some(want.as_str()))
			.map(|o| o.span.clone())
			.collect();
		if !gone.is_empty() {
			for span in gone {
				res!(xml.splice(span, String::new()));
			}
			zip.set(types, xml.render().into_bytes(), Method::Deflate);
		}
	}
	let rels_part = fmt!("{}_rels/{}.rels", dir, &wb_part[dir.len()..]);
	if zip.has(&rels_part) {
		let src = res!(String::from_utf8(res!(zip.content_capped(&rels_part, cap))), Decode, String);
		let mut xml = res!(Xml::parse(&src));
		let gone: Vec<Span> = res!(xml.root()).children("Relationship").iter()
			.filter(|r| r.attr("Type") == Some(REL_CALC_CHAIN))
			.map(|r| r.span.clone())
			.collect();
		if !gone.is_empty() {
			for span in gone {
				res!(xml.splice(span, String::new()));
			}
			zip.set(&rels_part, xml.render().into_bytes(), Method::Deflate);
		}
	}
	Ok(())
}

/// The relationship type of the calculation chain.
const REL_CALC_CHAIN: &str =
	"http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain";
