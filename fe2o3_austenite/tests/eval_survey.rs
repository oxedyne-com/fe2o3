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

use oxedyne_fe2o3_austenite::door::{
	Failure,
	Instance,
	Project,
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
	let main = res!(write_main(dir, name, src));
	let want = main.with_extension("pdf");
	let (ok, stderr) = res!(typst(&main, &want));
	assert!(ok, "typst refuses {}: {}", name, stderr);
	let mut inst = Instance::new();
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

// Loops: Typst bounds one `while`, not the document

// Eleven million empty iterations and a body that ends at once: more than the ten million the evaluator
// once allowed across a whole compile, as the plot of a technical document spends them.
const MANY: &str = "#{ for i in range(3200) { for j in range(3400) { } }; [done] }\n";

#[test]
fn a_document_may_run_more_loop_iterations_than_ten_million_as_typst_lets_it() -> Outcome<()> {
	let dir = res!(work_dir("fuel"));
	let text = res!(builds_alike(&dir, "many", MANY));
	assert_eq!(text, "done");
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
