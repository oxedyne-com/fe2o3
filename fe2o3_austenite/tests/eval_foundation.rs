//! U3's oracle tests: the foundation library (calc, str, array, dict, numbering, colour, sym, datetime,
//! data, foundations, geometry) against `typst` 0.15.1.
//!
//! Each fixture under `tests/fixtures/eval/foundation/` is a list of Typst code expressions, one per line.
//! A plain line must evaluate in both engines to the same `repr`; a line starting `!` must fail in both
//! with the same first error message, and `!~` must fail in both whatever the message. Austenite
//! evaluates each line through the evaluator proper (`eval_string` in code mode); the expected values
//! come from `typst eval`, never from Austenite. A missing `typst` fails the suite unless
//! `EVAL_ORACLE_SKIP=1` is set. Oracle answers are cached under the target directory, keyed on the
//! exact text sent, so a changed fixture line is always asked afresh.

use oxedyne_fe2o3_austenite::eval::eval::{
	eval_string,
	EvalMode,
};
use oxedyne_fe2o3_austenite::eval::lib::foundations::repr;
use oxedyne_fe2o3_austenite::eval::lib::library;
use oxedyne_fe2o3_austenite::eval::scope::Scope;
use oxedyne_fe2o3_austenite::eval::value::Value;
use oxedyne_fe2o3_austenite::eval::{
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::hash_map::DefaultHasher;
use std::hash::{
	Hash,
	Hasher,
};
use std::path::PathBuf;
use std::process::Command;

fn fixtures_dir() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval/foundation")
}

/// Evaluates one expression; the error is the first error diagnostic's message, as Typst reports the
/// first error first.
fn eval_line(line: &str) -> std::result::Result<Value, String> {
	let mut e = Engine::new(World::new(fixtures_dir()));
	let out = eval_string(&mut e, line, EvalMode::Code, Scope::new(), Span::detached());
	match out {
		Ok(v)	=> Ok(v),
		Err(err)	=> Err(match e.diags.iter().find(|d| d.is_error()) {
			Some(d)	=> d.message.clone(),
			None	=> err.msgs().last().cloned().unwrap_or_default(),
		}),
	}
}

// The oracle.

fn typst() -> Option<String> {
	let bin = std::env::var("TYPST").unwrap_or_else(|_| "typst".to_string());
	match Command::new(&bin).arg("--version").output() {
		Ok(o) if o.status.success() => Some(bin),
		_ => None,
	}
}

// Whether a missing oracle may pass silently: only when the skip is asked for by name.
fn skip_allowed() -> bool { std::env::var("EVAL_ORACLE_SKIP").as_deref() == Ok("1") }

fn scratch() -> Outcome<PathBuf> {
	let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("eval_foundation");
	res!(std::fs::create_dir_all(&d));
	Ok(d)
}

// The oracle runs capped, so a runaway document cannot take the build slot's memory.
fn capped(bin: &str) -> Command {
	let mut c = Command::new("systemd-run");
	c.args(["--user", "--scope", "--quiet", "-p", "MemoryMax=3G", "--slice=claude-rc.slice", bin]);
	c
}

fn key(parts: &[&str]) -> String {
	let mut h = DefaultHasher::new();
	for p in parts {
		p.hash(&mut h);
	}
	format!("{:016x}", h.finish())
}

// Runs the oracle once per distinct input: the answer is kept beside the input it answers.
fn cached(tag: &str, input: &str, run: impl Fn() -> Outcome<String>) -> Outcome<String> {
	let dir = res!(scratch()).join("cache");
	res!(std::fs::create_dir_all(&dir));
	let file = dir.join(format!("{}-{}", tag, key(&[tag, input])));
	if let Ok(s) = std::fs::read_to_string(&file) {
		if let Some(rest) = s.strip_prefix(input) {
			if let Some(ans) = rest.strip_prefix("\n\u{0}\n") {
				return Ok(ans.to_string());
			}
		}
	}
	let ans = res!(run());
	res!(std::fs::write(&file, format!("{}\n\u{0}\n{}", input, ans)));
	Ok(ans)
}

// Typst's reprs for many expressions at once, through a document that exposes them as metadata.
fn oracle_reprs(bin: &str, name: &str, exprs: &[&str]) -> Outcome<Vec<String>> {
	let mut src = String::from("#metadata((\n");
	for e in exprs {
		src.push_str(&format!("  repr({}),\n", e));
	}
	src.push_str(")) <reprs>\n");
	let file = fixtures_dir().join(format!("_oracle_{}.typ", name));
	let json = res!(cached("reprs", &src, || {
		res!(std::fs::write(&file, &src));
		let out = res!(capped(bin)
			.args(["eval", "query(<reprs>).first().value", "--in"])
			.arg(&file)
			.args(["--format", "json", "--root"])
			.arg(fixtures_dir())
			.output());
		let _ = std::fs::remove_file(&file);
		if !out.status.success() {
			return Err(err!("typst failed on {}: {}", name, String::from_utf8_lossy(&out.stderr); Test));
		}
		Ok(String::from_utf8_lossy(&out.stdout).to_string())
	}));
	parse_json_strings(&json)
}

// One expression's repr, or the oracle's first error line, from `typst eval` alone.
fn oracle_one(bin: &str, expr: &str) -> Outcome<std::result::Result<String, String>> {
	let ans = res!(cached("one", expr, || {
		let file = fixtures_dir().join(format!("_oracle_one_{}.typ", key(&[expr])));
		res!(std::fs::write(&file, ""));
		let out = res!(capped(bin)
			.args(["eval", &format!("repr({})", expr), "--in"])
			.arg(&file)
			.args(["--format", "json", "--root"])
			.arg(fixtures_dir())
			.output());
		let _ = std::fs::remove_file(&file);
		if out.status.success() {
			return Ok(format!("V[{}]", String::from_utf8_lossy(&out.stdout).trim()));
		}
		let err = String::from_utf8_lossy(&out.stderr);
		Ok(err.lines().find_map(|l| l.strip_prefix("error: ").map(|s| format!("E{}", s)))
			.unwrap_or_else(|| "E".to_string()))
	}));
	match ans.strip_prefix('V') {
		Some(json)	=> match res!(parse_json_strings(json)).into_iter().next() {
			Some(r)	=> Ok(Ok(r)),
			None	=> Err(err!("no oracle value for {}", expr; Test)),
		},
		None		=> Ok(Err(ans.strip_prefix('E').unwrap_or("").to_string())),
	}
}

// The oracle's first error line, or `None` when the expression evaluates.
fn oracle_error(bin: &str, expr: &str) -> Outcome<Option<String>> {
	let ans = res!(cached("error", expr, || {
		// One file per expression: the tests run in parallel.
		let file = fixtures_dir().join(format!("_oracle_err_{}.typ", key(&[expr])));
		res!(std::fs::write(&file, ""));
		let out = res!(capped(bin)
			.args(["eval", &format!("repr({})", expr), "--in"])
			.arg(&file)
			.arg("--root")
			.arg(fixtures_dir())
			.output());
		let _ = std::fs::remove_file(&file);
		if out.status.success() {
			return Ok(String::new());
		}
		let err = String::from_utf8_lossy(&out.stderr);
		Ok(err.lines().find_map(|l| l.strip_prefix("error: ").map(|s| format!("E{}", s)))
			.unwrap_or_else(|| "E".to_string()))
	}));
	Ok(ans.strip_prefix('E').map(|s| s.to_string()))
}

// A JSON array of strings, as `typst eval --format json` prints one.
fn parse_json_strings(s: &str) -> Outcome<Vec<String>> {
	let b: Vec<char> = s.trim().chars().collect();
	let hex = |from: usize| -> Outcome<u32> {
		let h: String = b.get(from..from + 4).map(|x| x.iter().collect()).unwrap_or_default();
		u32::from_str_radix(&h, 16).map_err(|e| err!("bad escape {}: {}", h, e; Decode))
	};
	let mut out = Vec::new();
	let mut i = 0;
	while i < b.len() {
		if b[i] == '"' {
			i += 1;
			let mut cur = String::new();
			while i < b.len() && b[i] != '"' {
				if b[i] == '\\' {
					i += 1;
					match b.get(i).copied().unwrap_or(' ') {
						'n'	=> cur.push('\n'),
						't'	=> cur.push('\t'),
						'r'	=> cur.push('\r'),
						'u'	=> {
							let mut cp = res!(hex(i + 1));
							i += 4;
							if (0xD800..0xDC00).contains(&cp) {
								let lo = res!(hex(i + 3));
								cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
								i += 6;
							}
							cur.push(res!(char::from_u32(cp).ok_or_else(|| err!("bad codepoint {}", cp; Decode))));
						}
						c	=> cur.push(c),
					}
				} else {
					cur.push(b[i]);
				}
				i += 1;
			}
			out.push(cur);
		}
		i += 1;
	}
	Ok(out)
}

/// Equal reprs, allowing floats to differ by 1e-12 relative: the platform `libm` may round an `exp` or
/// `ln` one unit differently from the one the oracle binary links (musl), which is not a library fault.
/// Everything that is not a decimal float must match exactly.
fn same_repr(got: &str, want: &str) -> bool {
	if got == want {
		return true;
	}
	let (g, w) = (tokens(got), tokens(want));
	g.len() == w.len() && g.iter().zip(w.iter()).all(|(a, b)| match (a, b) {
		(Tok::Float(x), Tok::Float(y))	=> x == y || ((x - y).abs() <= 1e-12 * x.abs().max(y.abs())),
		(Tok::Text(x), Tok::Text(y))	=> x == y,
		_								=> false,
	})
}

enum Tok {
	Float(f64),
	Text(String),
}

// Splits a repr into floats (a digit run with a `.` or an exponent) and everything else.
fn tokens(s: &str) -> Vec<Tok> {
	let b: Vec<char> = s.chars().collect();
	let mut out = Vec::new();
	let mut text = String::new();
	let mut i = 0;
	while i < b.len() {
		let starts = b[i].is_ascii_digit() && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == '#'));
		if starts {
			let mut j = i;
			while j < b.len() && (b[j].is_ascii_digit() || b[j] == '.' || b[j] == 'e'
				|| (b[j] == '-' && j > 0 && b[j - 1] == 'e')) {
				j += 1;
			}
			let num: String = b[i..j].iter().collect();
			let is_float = num.contains('.') || num.contains('e');
			let alpha_after = j < b.len() && b[j].is_ascii_alphabetic() && !num.contains('.');
			if is_float && !alpha_after {
				if let Ok(x) = num.parse::<f64>() {
					if !text.is_empty() {
						out.push(Tok::Text(std::mem::take(&mut text)));
					}
					out.push(Tok::Float(x));
					i = j;
					continue;
				}
			}
			text.push_str(&num);
			i = j;
			continue;
		}
		text.push(b[i]);
		i += 1;
	}
	if !text.is_empty() {
		out.push(Tok::Text(text));
	}
	out
}

enum Line {
	Value(String),
	Error(String, bool),	// the expression, and whether the message must match
}

fn fixture(name: &str) -> Outcome<Vec<Line>> {
	let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval/foundation").join(name);
	let text = res!(std::fs::read_to_string(&path));
	Ok(text.lines()
		.map(|l| l.trim())
		.filter(|l| !l.is_empty() && !l.starts_with("//"))
		.map(|l| {
			if let Some(r) = l.strip_prefix("!~") {
				Line::Error(r.trim().to_string(), false)
			} else if let Some(r) = l.strip_prefix('!') {
				Line::Error(r.trim().to_string(), true)
			} else {
				Line::Value(l.to_string())
			}
		})
		.collect())
}

/// Runs a fixture against the oracle and returns every disagreement.
fn check(name: &str) -> Outcome<Vec<String>> {
	let bin = match typst() {
		Some(b)	=> b,
		None	=> {
			if skip_allowed() {
				eprintln!("typst missing: skipping {} because EVAL_ORACLE_SKIP=1", name);
				return Ok(Vec::new());
			}
			return Err(err!("typst is not on PATH (set TYPST, or EVAL_ORACLE_SKIP=1 to skip deliberately)"; Test));
		}
	};
	let lines = res!(fixture(name));
	let exprs: Vec<&str> = lines.iter().filter_map(|l| match l {
		Line::Value(e)	=> Some(e.as_str()),
		_				=> None,
	}).collect();
	let mut bad = Vec::new();
	// One document for the whole fixture; when Typst rejects it, each line is asked alone, and a line
	// Typst rejects without a `!` mark is itself a fixture fault.
	let expected: Vec<String> = match oracle_reprs(&bin, name.trim_end_matches(".txt"), &exprs) {
		Ok(v) if v.len() == exprs.len()	=> v,
		_ => {
			let mut v = Vec::with_capacity(exprs.len());
			for e in &exprs {
				match res!(oracle_one(&bin, e)) {
					Ok(r)	=> v.push(r),
					Err(m)	=> {
						bad.push(format!("{}\n    typst rejects an unmarked line: {}", e, m));
						v.push(String::from("\u{0}"));
					}
				}
			}
			v
		}
	};
	for (e, want) in exprs.iter().zip(expected.iter()) {
		if want == "\u{0}" {
			continue;
		}
		match eval_line(e) {
			Ok(v) => {
				let got = repr(&v);
				if !same_repr(&got, want) {
					bad.push(format!("{}\n    typst:     {}\n    austenite: {}", e, want, got));
				}
			}
			Err(m) => bad.push(format!("{}\n    typst:     {}\n    austenite: error: {}", e, want, m)),
		}
	}
	for l in &lines {
		if let Line::Error(e, strict) = l {
			let want = res!(oracle_error(&bin, e));
			let got = eval_line(e);
			match (want, got) {
				(None, _) => bad.push(format!("{}\n    typst accepted an expression marked as an error", e)),
				(Some(w), Ok(v)) => bad.push(format!("{}\n    typst:     error: {}\n    austenite: {}", e, w, repr(&v))),
				(Some(w), Err(m)) => if *strict && w.lines().next() != m.lines().next() {
					bad.push(format!("{}\n    typst:     error: {}\n    austenite: error: {}", e, w, m));
				},
			}
		}
	}
	eprintln!("{}: {} expressions, {} disagreements", name, lines.len(), bad.len());
	Ok(bad)
}

fn run(name: &str) -> Outcome<()> {
	let bad = res!(check(name));
	if bad.is_empty() {
		Ok(())
	} else {
		Err(err!("{} disagreement(s) with typst:\n{}", bad.len(), bad.join("\n"); Test))
	}
}

#[test] fn foundations_agree_with_typst()	-> Outcome<()> { run("foundations.txt") }
#[test] fn repr_agrees_with_typst()			-> Outcome<()> { run("repr.txt") }
#[test] fn calc_agrees_with_typst()			-> Outcome<()> { run("calc.txt") }
#[test] fn strings_agree_with_typst()		-> Outcome<()> { run("string.txt") }
#[test] fn arrays_agree_with_typst()		-> Outcome<()> { run("array.txt") }
#[test] fn dicts_agree_with_typst()			-> Outcome<()> { run("dict.txt") }
#[test] fn numbering_agrees_with_typst()	-> Outcome<()> { run("numbering.txt") }
#[test] fn colours_agree_with_typst()		-> Outcome<()> { run("color.txt") }
#[test] fn datetimes_agree_with_typst()		-> Outcome<()> { run("datetime.txt") }
#[test] fn data_agrees_with_typst()			-> Outcome<()> { run("data.txt") }
#[test] fn geometry_agrees_with_typst()		-> Outcome<()> { run("geom.txt") }
#[test] fn symbols_agree_with_typst()		-> Outcome<()> { run("sym.txt") }
#[test] fn the_math_module_agrees_with_typst()	-> Outcome<()> { run("math.txt") }

#[test] fn coverage_agrees_with_typst()		-> Outcome<()> { run("coverage.txt") }
#[test] fn decimals_agree_with_typst()		-> Outcome<()> { run("decimal.txt") }
#[test] fn higher_order_agrees_with_typst()	-> Outcome<()> { run("higher_order.txt") }

/// Every symbol and emoji Typst has, with every variant: the generated tables against the oracle.
#[test]
fn every_symbol_agrees_with_typst() -> Outcome<()> {
	let bin = match typst() {
		Some(b)			=> b,
		None if skip_allowed()	=> return Ok(()),
		None			=> return Err(err!("typst is not on PATH"; Test)),
	};
	let lib = library();
	let mut exprs = Vec::new();
	let mut got = Vec::new();
	for m in ["sym", "emoji"] {
		collect_symbols(&lib, m, m, &mut exprs, &mut got);
	}
	if exprs.len() < 1000 {
		return Err(err!("only {} symbols collected", exprs.len(); Test));
	}
	let refs: Vec<&str> = exprs.iter().map(|s| s.as_str()).collect();
	let want = res!(oracle_reprs(&bin, "symbols", &refs));
	let bad: Vec<String> = refs.iter().zip(want.iter().zip(got.iter()))
		.filter(|(_, (w, g))| w != g)
		.map(|(e, (w, g))| format!("{}: typst {} austenite {}", e, w, g))
		.collect();
	eprintln!("symbols: {} checked, {} differ", refs.len(), bad.len());
	if bad.is_empty() {
		Ok(())
	} else {
		Err(err!("{} symbol(s) differ:\n{}", bad.len(), bad.join("\n"); Test))
	}
}

fn collect_symbols(lib: &Scope, root: &str, path: &str, exprs: &mut Vec<String>, got: &mut Vec<String>) {
	let mut v = lib.get(root).cloned();
	for p in path.split('.').skip(1) {
		v = v.and_then(|m| match m {
			Value::Module(m)	=> m.scope.get(p).cloned(),
			_					=> None,
		});
	}
	if let Some(Value::Module(m)) = v {
		let mut names: Vec<&String> = m.scope.map.keys().collect();
		names.sort();
		for n in names {
			let p = format!("{}.{}", path, n);
			match m.scope.get(n) {
				Some(Value::Module(_))	=> collect_symbols(lib, root, &p, exprs, got),
				Some(s)					=> {
					exprs.push(p);
					got.push(repr(s));
				}
				None					=> (),
			}
		}
	}
}

/// The harness can fail: the comparison sees a value and an error message, not just "something ran".
#[test]
fn harness_reads_values_and_errors() -> Outcome<()> {
	let v = match eval_line("calc.pow(2, 10)") {
		Ok(v)	=> v,
		Err(m)	=> return Err(err!("calc.pow failed: {}", m; Test)),
	};
	if repr(&v) != "1024" {
		return Err(err!("calc.pow(2, 10) gave {}", repr(&v); Test));
	}
	match eval_line("calc.pow(0, 0)") {
		Err(m) if m == "zero to the power of zero is undefined" => Ok(()),
		other => Err(err!("calc.pow(0, 0) gave {:?}", other.map(|v| repr(&v)); Test)),
	}
}

/// CMYK to RGB is an ICC transform in Typst, reproduced here by interpolating a grid sampled from the
/// oracle: exact on the grid, and within two eight-bit steps per channel between its points. The sample is
/// fixed pseudo-random ink values, so the bound is tested off the grid, where it can fail.
#[test]
fn cmyk_conversion_stays_within_two_steps() -> Outcome<()> {
	let bin = match typst() {
		Some(b)					=> b,
		None if skip_allowed()	=> return Ok(()),
		None					=> return Err(err!("typst is not on PATH"; Test)),
	};
	let mut seed: u32 = 0x2545_f491;
	let mut exprs = Vec::new();
	for _ in 0..60 {
		let mut ink = [0u32; 4];
		for x in ink.iter_mut() {
			seed ^= seed << 13;
			seed ^= seed >> 17;
			seed ^= seed << 5;
			*x = seed % 101;
		}
		exprs.push(format!("rgb(cmyk({}%, {}%, {}%, {}%)).to-hex()", ink[0], ink[1], ink[2], ink[3]));
	}
	let refs: Vec<&str> = exprs.iter().map(|s| s.as_str()).collect();
	let want = res!(oracle_reprs(&bin, "cmyk", &refs));
	let (mut exact, mut bad) = (0, Vec::new());
	for (e, w) in refs.iter().zip(want.iter()) {
		let got = match eval_line(e) {
			Ok(v)	=> repr(&v),
			Err(m)	=> return Err(err!("{}: {}", e, m; Test)),
		};
		let chan = |s: &str, i: usize| i64::from_str_radix(s.get(2 + 2 * i..4 + 2 * i).unwrap_or("0"), 16).unwrap_or(-99);
		let worst = (0..3).map(|i| (chan(&got, i) - chan(w, i)).abs()).max().unwrap_or(0);
		if worst == 0 {
			exact += 1;
		}
		if worst > 2 {
			bad.push(format!("{}: typst {} austenite {}", e, w, got));
		}
	}
	eprintln!("cmyk: {} of {} exact", exact, refs.len());
	if bad.is_empty() {
		Ok(())
	} else {
		Err(err!("{} conversion(s) off by more than two steps:\n{}", bad.len(), bad.join("\n"); Test))
	}
}

/// `Color::to_rgba`, the conversion every fill and stroke takes to the back end, gives the eight-bit
/// sRGB Typst's `to-hex` gives, in every colour space and with alpha.
#[test]
fn to_rgba_agrees_with_typst_hex() -> Outcome<()> {
	let bin = match typst() {
		Some(b)					=> b,
		None if skip_allowed()	=> return Ok(()),
		None					=> return Err(err!("typst is not on PATH"; Test)),
	};
	let colours = [
		"red", "blue", "eastern", "gray", "silver", "black", "white", "lime", "rgb(10%, 20%, 30%)",
		"rgb(\"#12345678\")", "rgb(1, 2, 3, 40%)", "luma(40%)", "luma(40%, 50%)", "oklab(60%, 0.1, -0.1)",
		"oklch(70%, 0.1, 200deg)", "color.linear-rgb(10%, 50%, 90%)", "color.hsl(200deg, 40%, 60%)",
		"color.hsv(20deg, 60%, 70%, 30%)", "cmyk(0%, 0%, 0%, 0%)", "cmyk(100%, 0%, 0%, 0%)",
		"red.lighten(20%)", "blue.transparentize(50%)", "color.mix(red, blue)",
	];
	let exprs: Vec<String> = colours.iter().map(|c| format!("({}).to-hex()", c)).collect();
	let refs: Vec<&str> = exprs.iter().map(|s| s.as_str()).collect();
	let want = res!(oracle_reprs(&bin, "to_rgba", &refs));
	let mut bad = Vec::new();
	for (c, w) in colours.iter().zip(want.iter()) {
		let v = match eval_line(c) {
			Ok(Value::Color(v))	=> v,
			Ok(other)			=> return Err(err!("{} is not a colour: {}", c, repr(&other); Test)),
			Err(m)				=> return Err(err!("{}: {}", c, m; Test)),
		};
		let p = res!(v.to_rgba());
		let hex = if p.a == 255 {
			format!("\"#{:02x}{:02x}{:02x}\"", p.r, p.g, p.b)
		} else {
			format!("\"#{:02x}{:02x}{:02x}{:02x}\"", p.r, p.g, p.b, p.a)
		};
		if &hex != w {
			bad.push(format!("{}: typst {} to_rgba {}", c, w, hex));
		}
	}
	if bad.is_empty() {
		Ok(())
	} else {
		Err(err!("{} colour(s) differ:\n{}", bad.len(), bad.join("\n"); Test))
	}
}

/// `numbering::apply_trimmed` is what a reference shows of a heading's number, and `apply_kth` what an
/// enumeration item shows at its depth. Typst's rendered text, through `pdftotext`, is the oracle.
#[test]
fn numbering_trimmed_and_kth_agree_with_typst() -> Outcome<()> {
	use oxedyne_fe2o3_austenite::eval::lib::numbering;
	let bin = match typst() {
		Some(b)					=> b,
		None if skip_allowed()	=> return Ok(()),
		None					=> return Err(err!("typst is not on PATH"; Test)),
	};
	let dir = res!(scratch());
	let (src, pdf) = (dir.join("numbering.typ"), dir.join("numbering.pdf"));
	res!(std::fs::write(&src, concat!(
		"#set page(width: 12cm, height: auto)\n#set heading(numbering: \"(I.a)\")\n= Alpha\n",
		"== Beta <b>\nRef: @b\n#set enum(numbering: \"1.a.i)\")\n+ one\n  + two\n    + three\n      + four\n")));
	let out = res!(capped(&bin).arg("compile").arg(&src).arg(&pdf).output());
	if !out.status.success() {
		return Err(err!("typst compile failed: {}", String::from_utf8_lossy(&out.stderr); Test));
	}
	let text = res!(Command::new("pdftotext").args(["-layout"]).arg(&pdf).arg("-").output());
	let lines: Vec<String> = String::from_utf8_lossy(&text.stdout).lines()
		.map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|l| !l.is_empty()).collect();
	let mut e = Engine::new(World::new(fixtures_dir()));
	let pat = Value::str("(I.a)");
	let mut want = vec![
		format!("({}) Alpha", "I"),
		format!("{} Beta", repr(&res!(numbering::apply(&mut e, &pat, &[1, 1])))),
		format!("Ref: Section {}", repr(&res!(numbering::apply_trimmed(&mut e, &pat, &[1, 1])))),
	];
	let items = Value::str("1.a.i)");
	for (k, word) in ["one", "two", "three", "four"].iter().enumerate() {
		want.push(format!("{} {}", repr(&res!(numbering::apply_kth(&mut e, &items, k, 1))), word));
	}
	// The reprs carry quotes; the rendered text does not.
	let want: Vec<String> = want.iter().map(|w| w.replace('"', "")).collect();
	if lines != want {
		return Err(err!("typst rendered {:?}, austenite gives {:?}", lines, want; Test));
	}
	Ok(())
}
