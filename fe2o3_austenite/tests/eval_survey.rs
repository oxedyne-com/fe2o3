//! Survey fixes (DEV, D-20261004-08): constructs a survey of the document trees found the evaluator
//! refusing while `typst` 0.15.1 builds them. Typst is asked first and must compile each source; the door
//! must then compile what Typst compiled, to the same page count and the same text, or, where Typst
//! refuses, refuse at Typst's site with Typst's message. Every expectation comes from the oracle at run
//! time, never from Austenite's own output.
//!
//! The door's columns are 1-based UTF-16 code units, Typst's are 0-based characters.

#[allow(dead_code)]
#[path = "eval_oracle/mod.rs"]
mod harness;

use harness::pdf::{
	self,
	tool,
	TYPST,
};

use oxedyne_fe2o3_austenite::compile::{
	supply_typst_package_cache,
	DiagnosticKind,
};
use oxedyne_fe2o3_austenite::door::{
	Failure,
	Instance,
	Project,
	LOOP_BUDGET,
};
use oxedyne_fe2o3_austenite::eval::content::{
	ElemKind,
	FieldType,
};

use oxedyne_fe2o3_core::prelude::*;

use std::path::{
	Path,
	PathBuf,
};
use std::process::Command;

fn work_dir(name: &str) -> Outcome<PathBuf> { pdf::work_dir(&fmt!("eval-survey-{}", name)) }

/// Typst's run with the cap every oracle run is given: whether it compiled, and its standard error.
fn typst(main: &Path, pdf: &Path) -> Outcome<(bool, String)> {
	let out = res!(Command::new("systemd-run")
		.args(["--user", "--scope", "--quiet", "-p", "MemoryMax=3G", "--slice=claude-rc.slice", TYPST])
		.args(["compile", "--diagnostic-format", "short"])
		.arg(main)
		.arg(pdf)
		.output());
	Ok((out.status.success(), String::from_utf8_lossy(&out.stderr).to_string()))
}

fn pages_of(pdf: &Path) -> Outcome<usize> {
	let info = res!(tool("pdfinfo", &[], pdf, &[]));
	let line = res!(info.lines().find(|l| l.starts_with("Pages:")).ok_or_else(|| err!("pdfinfo gave no page count"; Test)));
	Ok(res!(line.trim_start_matches("Pages:").trim().parse::<usize>().map_err(|e| err!("{}", e; Test))))
}

/// The PDF's text with every space gone, so a thin space between two glyphs is not a difference.
fn glyphs_of(pdf: &Path) -> Outcome<String> {
	Ok(res!(pdf::text_of(pdf)).chars().filter(|c| !c.is_whitespace()).collect())
}

/// Writes `src` as `main.typ` under a case directory and returns its path.
fn write_main(dir: &Path, name: &str, src: &str) -> Outcome<PathBuf> {
	let case = dir.join(name);
	res!(std::fs::create_dir_all(&case));
	let main = case.join("main.typ");
	res!(std::fs::write(&main, src));
	Ok(main)
}

/// Compiles `src` under Typst and through the door, not strict, and asserts they agree on the pages and the
/// text. Returns the text.
fn builds_alike(dir: &Path, name: &str, src: &str) -> Outcome<String> {
	builds_alike_with(dir, name, src, Instance::new())
}

/// As [`builds_alike`], through the door instance given, whose host options the caller has set.
fn builds_alike_with(dir: &Path, name: &str, src: &str, mut inst: Instance) -> Outcome<String> {
	let main = res!(write_main(dir, name, src));
	let want = main.with_extension("pdf");
	let (ok, stderr) = res!(typst(&main, &want));
	assert!(ok, "typst refuses {}: {}", name, stderr);
	let made = match inst.compile_pdf(&Project::single(src)) {
		Ok(m)	=> m,
		Err(f)	=> return Err(err!("The door refused {}, which typst builds: {} {:?}", name, f.head, f.head.kind; Test)),
	};
	let ours = main.with_file_name("ours.pdf");
	res!(std::fs::write(&ours, made.product.to_vec()));
	assert_eq!(res!(pages_of(&ours)), res!(pages_of(&want)), "{}: pages", name);
	let text = res!(glyphs_of(&want));
	assert_eq!(res!(glyphs_of(&ours)), text, "{}: text", name);
	Ok(text)
}

/// Typst's first error: its message, and its line and 0-based column.
fn typst_error(dir: &Path, name: &str, src: &str) -> Outcome<(String, usize, usize)> {
	let main = res!(write_main(dir, name, src));
	let (ok, stderr) = res!(typst(&main, &main.with_extension("pdf")));
	assert!(!ok, "typst builds {}, which should be refused", name);
	for l in stderr.lines() {
		if let Some(at) = l.find(": error: ") {
			let message = l[at + ": error: ".len()..].to_string();
			let mut parts = l[..at].rsplitn(3, ':');
			let col = parts.next().and_then(|c| c.parse::<usize>().ok());
			let line = parts.next().and_then(|c| c.parse::<usize>().ok());
			if let (Some(line), Some(col)) = (line, col) {
				return Ok((message, line, col));
			}
		}
	}
	Err(err!("Typst gave no error site for {}: {}", name, stderr; Test))
}

/// The column the door gives for Typst's 0-based character column `col` on `line` of `text`.
fn door_col(text: &str, line: usize, col: usize) -> usize {
	let row = text.split('\n').nth(line - 1).unwrap_or("");
	let before: usize = row.chars().take(col).map(|c| c.len_utf16()).sum();
	before + 1
}

/// Asserts the door refuses `src` at Typst's site with Typst's message, and returns the failure.
fn refuses_alike(dir: &Path, name: &str, src: &str) -> Outcome<Failure> {
	let (message, line, col) = res!(typst_error(dir, name, src));
	let mut inst = Instance::new();
	let f = match inst.compile_pdf(&Project::single(src)) {
		Ok(_)	=> return Err(err!("The door builds {}, which typst refuses with: {}", name, message; Test)),
		Err(f)	=> f,
	};
	assert_eq!(f.head.message, message, "{}: the message", name);
	assert_eq!((f.head.line, f.head.col), (line, door_col(src, line, col)), "{}: the site", name);
	Ok(f)
}

/// As [`refuses_alike`] for the message alone: Typst reports a method's wrong argument at the argument, which
/// the door has no span for, so it names the call.
fn refuses_message_alike(dir: &Path, name: &str, src: &str) -> Outcome<Failure> {
	let (message, _, _) = res!(typst_error(dir, name, src));
	let mut inst = Instance::new();
	let f = match inst.compile_pdf(&Project::single(src)) {
		Ok(_)	=> return Err(err!("The door builds {}, which typst refuses with: {}", name, message; Test)),
		Err(f)	=> f,
	};
	assert_eq!(f.head.message, message, "{}: the message", name);
	Ok(f)
}

// Maths: `floor` and `ceil` are symbols

// The sources are the shapes a document writes: the delimiter as a variant of the symbol, called, set
// beside its partner, and the symbol alone.
const FLOORS: [(&str, &str); 6] = [
	("variant-called",		"$ T = floor.l(2^256 dot n slash N) $\n"),
	("both-sides",			"$ floor.l a floor.r + ceil.l b ceil.r $\n"),
	("right-called",		"$ floor.r(x) + ceil.r(y) $\n"),
	("alone",				"$floor$ and $ceil$ and $floor.l$ and $ceil.r$\n"),
	("mixed-with-calls",	"$ floor(x) + floor.l(y) + ceil(z) + ceil.l(w) + round(v) $\n"),
	("in-code",				"#math.floor.l($x$) #math.ceil.l($y$) #sym.floor.r #sym.ceil.l\n"),
];

#[test]
fn floor_and_ceil_are_symbols_with_left_and_right_variants_in_maths_as_typst_has_them() -> Outcome<()> {
	let dir = res!(work_dir("floor"));
	for (name, src) in FLOORS {
		res!(builds_alike(&dir, name, src));
	}
	// A symbol, not a function: its type and its variants are what `typst` reports.
	let ty = res!(builds_alike(&dir, "type-of", "#type(math.floor) #type(math.ceil) #repr(math.floor) #repr(math.ceil.r)\n"));
	assert!(ty.starts_with("symbolsymbol"), "typst's own answer: {}", ty);
	Ok(())
}

#[test]
fn a_function_of_maths_that_has_no_variants_still_refuses_one() -> Outcome<()> {
	let dir = res!(work_dir("variants"));
	// `round` and `abs` are functions in Typst, so the field is not found there.
	for (name, src) in [
		("round-l",	"$round.l$\n"),
		("abs-l",	"$abs.l$\n"),
		("modifier", "$floor.l.double$\n"),
	] {
		let f = res!(refuses_alike(&dir, name, src));
		assert!(!f.head.message.is_empty(), "{}", name);
	}
	Ok(())
}

// Code: a symbol is callable where it is an accent or an opening delimiter

#[test]
fn a_symbol_is_called_in_code_where_it_is_an_accent_or_an_opening_delimiter() -> Outcome<()> {
	let dir = res!(work_dir("call"));
	for (name, src) in [
		("floor",		"#sym.floor($x$) #sym.floor.l($y$) #math.ceil($z$)\n"),
		("paren",		"#sym.paren.l($x$) #sym.bracket.l($y$)\n"),
		("accent",		"#sym.acute($x$) #sym.arrow.r($y$)\n"),
		("bound",		"#let f = sym.floor\n#f($x$)\n"),
	] {
		res!(builds_alike(&dir, name, src));
	}
	for (name, src) in [
		("letter",		"#sym.alpha($x$)\n"),
		("closing",		"#sym.floor.r($x$)\n"),
		("plain",		"#sym.plus($x$)\n"),
	] {
		let f = res!(refuses_alike(&dir, name, src));
		assert!(f.head.message.ends_with("is not callable"), "{}: {}", name, f.head.message);
	}
	Ok(())
}

// Functions: a callable symbol is a function wherever a function is expected

// A symbol that is an accent or an opening delimiter casts to the function it calls as, so a document hands
// `math.floor` or `sym.paren.l` to `map` as it hands any function.
const SYMBOL_FUNCS: [(&str, &str); 8] = [
	("floor-mapped",	"#([a], [b]).map(math.floor).join()\n"),
	("ceil-mapped",		"#([a], [b]).map(math.ceil).join()\n"),
	("variant-mapped",	"#([a], [b]).map(math.floor.l).join() #([a], [b]).map(sym.ceil.l).join()\n"),
	("delimiter-mapped", "#([a], [b]).map(sym.paren.l).join() #([a], [b]).map(sym.bracket.l).join()\n"),
	("accent-mapped",	"#([a], [b]).map(sym.hat).join()\n"),
	("key-of-dedup",	"#([a], [b], [a]).dedup(key: sym.paren.l).len()\n"),
	("in-maths",		"$#([a], [b]).map(math.ceil).join()$ $#([a], [b]).map(sym.paren.l).join()$\n"),
	("bound-first",		"#let g = math.floor\n#([a], [b]).map(g).join()\n"),
];

#[test]
fn a_callable_symbol_is_a_function_where_a_function_is_expected_as_typst_casts_it() -> Outcome<()> {
	let dir = res!(work_dir("symbol-funcs"));
	for (name, src) in SYMBOL_FUNCS {
		res!(builds_alike(&dir, name, src));
	}
	Ok(())
}

#[test]
fn a_symbol_that_is_not_callable_is_refused_as_a_function_with_typsts_message() -> Outcome<()> {
	let dir = res!(work_dir("symbol-not-callable"));
	// Typst reports each at the argument, which the door has no span for, so it names the call.
	for (name, src) in [
		("map",			"#(1, 2).map(sym.alpha)\n"),
		("filter",		"#(1, 2).filter(sym.alpha)\n"),
		("any",			"#(1, 2).any(sym.alpha)\n"),
		("fold",		"#(1, 2).fold(0, sym.alpha)\n"),
		("reduce",		"#(1, 2).reduce(sym.alpha)\n"),
		("sorted-key",	"#(1, 2).sorted(key: sym.alpha)\n"),
		("dedup-key",	"#(1, 2).dedup(key: sym.alpha)\n"),
		("closing",		"#(1, 2).map(sym.floor.r)\n"),
		("plain",		"#(1, 2).map(sym.plus)\n"),
	] {
		let f = res!(refuses_message_alike(&dir, name, src));
		assert!(f.head.message.starts_with("symbol "), "{}: {}", name, f.head.message);
		assert!(f.head.message.ends_with(" is not callable"), "{}: {}", name, f.head.message);
	}
	Ok(())
}

// Keywords: a parameter that takes a closed set of strings takes a string alone

// Each is a parameter Typst types as a keyword: `@` is the value given. A symbol is never cast to the
// string it stands for here, as it is where a parameter is a `str`, so Typst refuses it by naming what it
// found, as it names any other type; a string outside the set is named without what was found.
const KEYWORDS: [(&str, &str); 28] = [
	("text-style",			"#text(style: @)[x]\n"),
	("text-weight",			"#text(weight: @)[x]\n"),
	("text-top-edge",		"#text(top-edge: @)[x]\n"),
	("text-bottom-edge",	"#text(bottom-edge: @)[x]\n"),
	("text-number-type",	"#text(number-type: @)[x]\n"),
	("text-number-width",	"#text(number-width: @)[x]\n"),
	("set-text-weight",		"#set text(weight: @)\nx\n"),
	("set-text-style",		"#set text(style: @)\nx\n"),
	("highlight-top-edge",	"#highlight(top-edge: @)[x]\n"),
	("highlight-bottom-edge", "#highlight(bottom-edge: @)[x]\n"),
	("frac-style",			"#math.frac([a], [b], style: @)\n"),
	("math-class",			"#math.class(@, [x])\n"),
	("place-scope",			"#place(scope: @)[x]\n"),
	("figure-scope",		"#figure([a], scope: @)\n"),
	("page-paper",			"#set page(paper: @)\n"),
	("pagebreak-to",		"#pagebreak(to: @)\n"),
	("curve-close-mode",	"#curve(curve.move((0pt, 0pt)), curve.close(mode: @))\n"),
	("curve-fill-rule",		"#curve(fill-rule: @, curve.line((1pt, 1pt)))\n"),
	("polygon-fill-rule",	"#polygon(fill-rule: @, (0pt, 0pt), (1pt, 1pt), (0pt, 2pt))\n"),
	("stroke-cap",			"#line(stroke: (cap: @))\n"),
	("stroke-join",			"#line(stroke: (join: @))\n"),
	("gradient-relative",	"#gradient.linear(red, blue, relative: @)\n"),
	("par-linebreaks",		"#set par(linebreaks: @)\n"),
	("par-line-scope",		"#set par.line(numbering-scope: @)\n"),
	("ref-form",			"#ref(<a>, form: @)\n"),
	("image-format",		"#image(\"x.svg\", format: @)\n"),
	("image-fit",			"#image(\"x.svg\", fit: @)\n"),
	("image-scaling",		"#image(\"x.svg\", scaling: @)\n"),
];

#[test]
fn a_keyword_parameter_refuses_a_symbol_and_any_other_type_with_typsts_message() -> Outcome<()> {
	// One SVG, `x.svg`, for the `image` cases: written beside Typst's copy and supplied as the project's own.
	const SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"4\"><rect width=\"4\" height=\"4\"/></svg>";
	let dir = res!(work_dir("keywords"));
	let mut bad: Vec<String> = Vec::new();
	let mut cases = 0;
	for (name, template) in KEYWORDS {
		for (given, found) in [("sym.alpha", "symbol"), ("true", "boolean")] {
			cases += 1;
			let case = fmt!("{}-{}", name, found);
			let src = template.replace('@', given);
			let main = res!(write_main(&dir, &case, &src));
			res!(std::fs::write(main.with_file_name("x.svg"), SVG));
			let (want, _, _) = res!(typst_error(&dir, &case, &src));
			let mut project = Project::single(&src);
			project.assets.push(("/x.svg".to_string(), SVG.as_bytes().to_vec()));
			let mut inst = Instance::new();
			let got = match inst.compile_pdf(&project) {
				Ok(_)	=> "it builds".to_string(),
				Err(f)	=> f.head.message,
			};
			if got != want || !want.ends_with(&fmt!(", found {}", found)) {
				bad.push(fmt!("{}: typst says `{}`, the door `{}`", case, want, got));
			}
		}
	}
	assert!(bad.is_empty(), "{} of {} keyword cases differ:\n{}", bad.len(), cases, bad.join("\n"));
	Ok(())
}

// The fields the schema types as keywords, each one in `KEYWORDS` above: a field typed so and left out
// is not held to Typst, and one listed here that is no longer a keyword is a stale line.
const KEYWORD_FIELDS: [&str; 11] = [
	"text.style", "text.weight", "text.top-edge", "text.bottom-edge", "text.number-type", "text.number-width",
	"highlight.top-edge", "highlight.bottom-edge", "math.frac.style", "math.class.class", "place.scope",
];

#[test]
fn every_field_the_schema_types_as_a_keyword_is_held_to_typst_above() -> Outcome<()> {
	let mut have: Vec<String> = Vec::new();
	for kind in ElemKind::ALL {
		for spec in kind.fields() {
			if matches!(spec.ty, FieldType::Keyword(..)) {
				have.push(fmt!("{}.{}", kind.path(), spec.name));
			}
		}
	}
	have.sort();
	have.dedup();
	let mut want: Vec<String> = KEYWORD_FIELDS.iter().map(|s| s.to_string()).collect();
	want.sort();
	want.dedup();
	assert_eq!(have, want, "the schema's keyword fields and the test's list differ");
	Ok(())
}

// Loops: Typst bounds one `while`, not the document

// Eleven million empty iterations and a body that ends at once: more than the ten million the evaluator
// once allowed across a whole compile, as the plot of a technical document spends them.
const MANY: &str = "#{ for i in range(3200) { for j in range(3400) { } }; [done] }\n";

// A host that can stop a compile itself declares so by setting no budget.
#[test]
fn a_document_may_run_more_loop_iterations_than_ten_million_as_typst_lets_it() -> Outcome<()> {
	let dir = res!(work_dir("fuel"));
	let mut inst = Instance::new();
	inst.set_loop_budget(None);
	let text = res!(builds_alike_with(&dir, "many", MANY, inst));
	assert_eq!(text, "done");
	Ok(())
}

// A host cannot stop a compile that has begun (Daimond's runs on the page's own thread), so the door bounds
// the loops of one unless the host says it can stop it. The command line has no bound, as Typst has none.
#[test]
fn the_door_refuses_a_document_that_spends_more_loop_iterations_than_its_default_budget() -> Outcome<()> {
	let mut inst = Instance::new();
	let f = match inst.compile_pdf(&Project::single(MANY)) {
		Ok(_)	=> return Err(err!("The door builds eleven million loop iterations, past its default budget"; Test)),
		Err(f)	=> f,
	};
	assert_eq!(f.head.kind, DiagnosticKind::Limit);
	assert_eq!(f.head.message, "loop seems to be infinite");
	Ok(())
}

#[test]
fn a_host_sets_the_loop_budget_to_a_number_or_to_none_and_a_small_one_refuses_earlier() -> Outcome<()> {
	assert_eq!(LOOP_BUDGET, 10_000_000);
	assert_eq!(Instance::new().loop_budget(), Some(LOOP_BUDGET));
	let dir = res!(work_dir("budget-host"));
	// Five thousand iterations of an empty loop, and the range it walks, spend ten thousand of the budget.
	let few = "#{ for i in range(5000) { }; [done] }\n";
	let mut small = Instance::new();
	small.set_loop_budget(Some(1_000));
	assert_eq!(small.loop_budget(), Some(1_000));
	let f = match small.compile_pdf(&Project::single(few)) {
		Ok(_)	=> return Err(err!("A budget of a thousand builds a document of five thousand iterations"; Test)),
		Err(f)	=> f,
	};
	assert_eq!(f.head.kind, DiagnosticKind::Limit);
	assert_eq!(f.head.message, "loop seems to be infinite");
	// The default allows it, as does a budget the host raises or removes.
	for (name, budget) in [("default", Some(LOOP_BUDGET)), ("raised", Some(100_000_000)), ("none", None)] {
		let mut inst = Instance::new();
		inst.set_loop_budget(budget);
		assert_eq!(inst.loop_budget(), budget);
		assert_eq!(res!(builds_alike_with(&dir, name, few, inst)), "done");
	}
	// The host's budget is read again by the next compile of an instance that has refused one.
	small.set_loop_budget(None);
	assert!(small.compile_pdf(&Project::single(few)).is_ok());
	Ok(())
}

#[test]
fn a_project_is_document_data_and_carries_no_loop_budget() -> Outcome<()> {
	// Every field of a project, named: one added for a budget stops this compiling, so the choice to let a
	// project that arrives by sync set its own bound is made here, deliberately, or not at all.
	let Project { main, sources, assets, fonts, strict, known } = Project::single("#[a]\n");
	assert!(main.is_empty() && assets.is_empty() && fonts.is_empty() && known.is_empty() && !strict);
	assert_eq!(sources.len(), 1);
	// A source that spells a budget is only source: the door's bound is the same with it.
	let mut inst = Instance::new();
	let src = "// loopBudget: null\n#let loop-budget = none\n#{ for i in range(3200) { for j in range(3400) { } }; [done] }\n";
	let f = match inst.compile_pdf(&Project::single(src)) {
		Ok(_)	=> return Err(err!("A project that spells a budget lifted the door's bound"; Test)),
		Err(f)	=> f,
	};
	assert_eq!(f.head.kind, DiagnosticKind::Limit);
	assert_eq!(inst.loop_budget(), Some(LOOP_BUDGET));
	Ok(())
}

#[test]
fn the_command_line_has_no_loop_budget_and_builds_the_document_the_door_refuses() -> Outcome<()> {
	let dir = res!(work_dir("budget-cli"));
	res!(write_main(&dir, "many", MANY));
	let out_dir = dir.join("many").join("out");
	let out = res!(Command::new(env!("CARGO_BIN_EXE_austenite"))
		.current_dir(dir.join("many"))
		.args(["--eval", "--root", ".", "main.typ"])
		.arg(&out_dir)
		.output());
	assert!(out.status.success(), "the command line builds it: {}", String::from_utf8_lossy(&out.stderr));
	let pdf = out_dir.join("document.pdf");
	assert!(pdf.is_file(), "{} is written", pdf.display());
	assert_eq!(res!(glyphs_of(&pdf)), "done");
	Ok(())
}

#[test]
fn a_while_loop_still_ends_at_ten_thousand_iterations_where_typst_ends_it() -> Outcome<()> {
	let dir = res!(work_dir("while"));
	let text = res!(builds_alike(&dir, "edge", "#{ let n = 0; while n < 10000 { n += 1 }; [#n] }\n"));
	assert_eq!(text, "10000");
	let f = res!(refuses_alike(&dir, "over", "#{ let n = 0; while n < 10001 { n += 1 }; [#n] }\n"));
	assert_eq!(f.head.message, "loop seems to be infinite");
	Ok(())
}

// Strings: a symbol is cast to the string it stands for wherever Typst expects a string

// A computed key of a dictionary is cast to a string, so a symbol is the key its text spells, the
// variant its modifiers pick: the shape of a table of arrows that maps each to a mark's name.
const KEYS: [(&str, &str); 4] = [
	("arrows",		"#let d = ((sym.arrow.r): \"x\", (sym.arrow.l): \"y\", (sym.arrow.r.l): \"z\", (sym.arrow.double.long.r): \"w\")\n#repr(d) #d.keys()\n"),
	("later-wins",	"#let d = ((sym.alpha): 1, (sym.zws): 2, (math.floor): 3, (\"a\".first()): 4, (\"a\" + \"b\"): 5, (sym.arrow): 6, (sym.arrow.r): 7)\n#repr(d)\n"),
	("looked-up",	"#let d = ((sym.arrow.r): 1)\n#d.at(\"→\") #d.at(sym.arrow.r) #d.at(sym.arrow.l, default: 5)\n"),
	("a-string",	"#let d = ((\"a\" + \"b\"): 1, (str(1)): 2)\n#repr(d)\n"),
];

#[test]
fn a_symbol_is_a_dictionary_key_as_the_text_it_stands_for_and_any_other_non_string_is_refused() -> Outcome<()> {
	let dir = res!(work_dir("keys"));
	for (name, src) in KEYS {
		res!(builds_alike(&dir, name, src));
	}
	// The key's own expression is where Typst reports a value that is not a string.
	for (name, src, found) in [
		("integer",	"#let d = ((1): 1)\n",			"integer"),
		("none",	"#let d = ((none): 1)\n",		"none"),
		("float",	"#let d = ((1.5): 1)\n",		"float"),
		("content",	"#let d = (([a]): 1)\n",		"content"),
		("label",	"#let d = ((<lbl>): 1)\n",		"label"),
	] {
		let f = res!(refuses_alike(&dir, name, src));
		assert_eq!(f.head.message, fmt!("expected string, found {}", found), "{}", name);
	}
	Ok(())
}

// Each is a place Typst's parameter is a string, so a symbol is accepted and read as its text.
const CASTS: [(&str, &str); 14] = [
	("dict-at",			"#let d = (\"α\": 1, \"→\": 2)\n#d.at(sym.alpha) #d.at(sym.arrow.r) #d.at(sym.beta, default: 7)\n"),
	("dict-insert",		"#let d = (a: 1)\n#{ d.insert(sym.alpha, 2) } #d.remove(sym.alpha) #d.remove(sym.beta, default: 3)\n#repr(d)\n"),
	("dict-assign",		"#let d = (\"α\": 1)\n#{ d.at(sym.alpha) = 9 }\n#{ d.at(sym.alpha) += 1 }\n#repr(d)\n"),
	("dict-nested",		"#let d = (a: (:))\n#{ d.a.insert(sym.alpha, 1) }\n#repr(d)\n"),
	("str-patterns",	"#\"xαy\".contains(sym.alpha) #\"xαy\".starts-with(sym.alpha) #\"xα\".ends-with(sym.alpha) #\"xαy\".find(sym.alpha) #\"xαy\".position(sym.alpha) #\"xαy\".match(sym.alpha).text #\"xαyα\".matches(sym.alpha).len() #\"xαy\".split(sym.alpha) #\"αxα\".trim(sym.alpha)\n"),
	("str-replace",		"#\"xαy\".replace(sym.alpha, sym.beta) #\"xαy\".replace(\"x\", sym.beta) #\"a→b\".replace(regex(\"→\"), m => sym.beta)\n"),
	("constructors",	"#regex(sym.alpha) #bytes(sym.alpha).len() #str.to-unicode(sym.alpha) #repr(label(sym.alpha))\n"),
	("keys",			"#let c = counter(sym.alpha)\n#context c.get()\n#let s = state(sym.alpha, 5)\n#context s.get()\n#numbering(sym.alpha, 1)\n"),
	("case",			"#upper(sym.alpha) #lower(sym.Alpha)\n"),
	("eval",			"#eval(sym.alpha, mode: \"markup\")\n"),
	("fields",			"#raw(sym.alpha) #raw(sym.alpha, lang: sym.beta)\n"),
	("font",			"#text(font: sym.alpha)[x]\n"),
	("args-at",			"#let f(..a) = a\n#f(α: 1).at(sym.alpha) #f(α: 1).at(sym.beta, default: 0)\n"),
	("content-at",		"#[a].has(sym.alpha) #[a].at(sym.alpha, default: 1)\n"),
];

#[test]
fn a_symbol_is_read_as_its_text_wherever_typst_expects_a_string() -> Outcome<()> {
	let dir = res!(work_dir("casts"));
	for (name, src) in CASTS {
		res!(builds_alike(&dir, name, src));
	}
	Ok(())
}

#[test]
fn a_value_that_is_neither_a_string_nor_a_symbol_is_refused_where_a_string_is_expected_as_typst_does() -> Outcome<()> {
	let dir = res!(work_dir("string-refusals"));
	for (name, src) in [
		("at-label",		"#let d = (a: 1)\n#d.at(<lbl>)\n"),
		("at-none",			"#let d = (a: 1)\n#d.at(none)\n"),
		("at-float",		"#let d = (a: 1)\n#d.at(1.5)\n"),
		("remove-integer",	"#let d = (a: 1)\n#{ d.remove(1) }\n"),
		("insert-integer",	"#let d = (a: 1)\n#{ d.insert(1, 2) }\n"),
	] {
		let f = res!(refuses_message_alike(&dir, name, src));
		assert!(f.head.message.starts_with("expected string, found "), "{}: {}", name, f.head.message);
	}
	for (name, src) in [
		("assign-integer",	"#let d = (a: 1)\n#{ d.at(2) = 3 }\n"),
		// A symbol is a key; an absent one is refused at the assignment as any absent key is.
		("assign-absent",	"#let d = (a: 1)\n#{ d.at(sym.alpha) = 3 }\n"),
		("assert-message",	"#assert(false, message: sym.alpha)\n"),
	] {
		let f = res!(refuses_alike(&dir, name, src));
		assert!(!f.head.message.is_empty(), "{}", name);
	}
	Ok(())
}

// Packages: fletcher draws its edges' marks from a table keyed by symbols

// Synthetic documents that use two packages from the cache Typst keeps its own in: a commutative square
// and a second diagram whose edges are marked by strings and by symbols, a page of marks, shapes and
// loops, a page of nodes named and joined by paths and a figure, and a canvas of cetz with a plot.
const FLETCHER: [(&str, &str); 3] = [
	("square",	include_str!("fixtures/survey/fletcher_square.typ")),
	("marks",	include_str!("fixtures/survey/fletcher_marks.typ")),
	("shapes",	include_str!("fixtures/survey/fletcher_shapes.typ")),
];

/// Is the package in Typst's cache? Without it the test is skipped only on the owner's word.
fn package_cached(spec: &str) -> bool {
	let home = std::env::var("HOME").unwrap_or_default();
	if Path::new(&home).join(".cache/typst/packages/preview").join(spec).is_dir() {
		return true;
	}
	let skip = std::env::var("EVAL_ORACLE_SKIP").map(|v| v == "1").unwrap_or(false);
	assert!(skip, "no {} in Typst's package cache (set EVAL_ORACLE_SKIP=1 to skip explicitly)", spec);
	false
}

#[test]
fn a_fletcher_diagram_marked_by_symbols_and_strings_builds_to_typsts_pages_and_text() -> Outcome<()> {
	if !package_cached("fletcher/0.5.7") {
		return Ok(());
	}
	supply_typst_package_cache();
	let dir = res!(work_dir("fletcher"));
	for (name, src) in FLETCHER {
		res!(builds_alike(&dir, name, src));
	}
	Ok(())
}

// Not a fault this unit found: the next construct a package of that kind is known for, held to the same
// account.
#[test]
fn a_cetz_canvas_with_shapes_arcs_and_a_plot_builds_to_typsts_pages_and_text() -> Outcome<()> {
	if !package_cached("cetz-plot/0.1.1") {
		return Ok(());
	}
	supply_typst_package_cache();
	let dir = res!(work_dir("cetz"));
	res!(builds_alike(&dir, "canvas", include_str!("fixtures/survey/cetz_canvas.typ")));
	Ok(())
}
