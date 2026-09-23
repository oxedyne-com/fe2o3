//! U1's syntax tests. Every expectation comes from outside Austenite: the installed `typst` 0.15.1 (what
//! it evaluates markup and maths to, what values code evaluates to, which syntax errors and warnings it
//! reports and where) and the real package corpus in `~/.cache/typst/packages/preview`. A missing
//! `typst` or corpus fails loudly unless `EVAL_ORACLE_SKIP=1` is set.

use oxedyne_fe2o3_austenite::syntax::ast::{
	self,
	AstNode,
	Expr,
};
use oxedyne_fe2o3_austenite::syntax::parser;
use oxedyne_fe2o3_austenite::syntax::{
	FileId,
	Span,
	SyntaxKind,
	SyntaxNode,
};

use oxedyne_fe2o3_core::prelude::*;

use std::fs;
use std::path::{
	Path,
	PathBuf,
};
use std::process::Command;

const FILE: FileId = FileId(0);
const CASE_SEP: &str = "\n// ---- case ----\n";

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ Harness                                                                                   │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

fn skip_allowed() -> bool {
	std::env::var("EVAL_ORACLE_SKIP").map(|v| v == "1").unwrap_or(false)
}

/// Is the 0.15 oracle on hand? `false` only when its absence is explicitly allowed.
fn typst_available() -> Outcome<bool> {
	match Command::new("typst").arg("--version").output() {
		Ok(out) => {
			let v = String::from_utf8_lossy(&out.stdout).to_string();
			if !v.starts_with("typst 0.15") {
				return Err(err!("The typst oracle is {:?}, not 0.15.x.", v.trim(); Invalid, Input));
			}
			Ok(true)
		},
		Err(e) => {
			if skip_allowed() {
				println!("[syntax] typst oracle absent, skipped by EVAL_ORACLE_SKIP=1: {}", e);
				Ok(false)
			} else {
				Err(err!("The typst oracle is not on PATH ({}); set EVAL_ORACLE_SKIP=1 to skip.", e;
					Missing, Input))
			}
		},
	}
}

fn fixture(name: &str) -> Outcome<Vec<String>> {
	let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval/syntax").join(name);
	let text = res!(fs::read_to_string(&path).map_err(|e| err!("Reading {:?}: {}", path, e; IO, File)));
	let text = text.strip_suffix('\n').unwrap_or(&text).to_string();
	Ok(text.split(CASE_SEP).map(|s| s.to_string()).collect())
}

fn work_dir(name: &str) -> Outcome<PathBuf> {
	let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("syntax_oracle").join(name);
	let _ = fs::remove_dir_all(&dir);
	res!(fs::create_dir_all(&dir).map_err(|e| err!("Creating {:?}: {}", dir, e; IO, File)));
	Ok(dir)
}

fn write(path: &Path, text: &str) -> Outcome<()> {
	res!(fs::write(path, text).map_err(|e| err!("Writing {:?}: {}", path, e; IO, File)));
	Ok(())
}

/// `typst eval 'query(<p>).map(it => it.value)'` over `main.typ` in `dir`, as JSON.
fn typst_probe_values(dir: &Path) -> Outcome<Json> {
	let out = res!(Command::new("typst")
		.args(["eval", "query(<p>).map(it => it.value)", "--in", "main.typ", "--root"])
		.arg(dir)
		.current_dir(dir)
		.output()
		.map_err(|e| err!("Running typst eval: {}", e; IO)));
	if !out.status.success() {
		return Err(err!("typst eval failed: {}", String::from_utf8_lossy(&out.stderr); Invalid, Input));
	}
	let text = String::from_utf8_lossy(&out.stdout).to_string();
	Json::parse(&text)
}

/// A diagnostic as `typst compile` prints it: severity, 1-based line, 0-based column, message and the
/// hints that carry no span of their own.
#[derive(Clone, Debug, PartialEq)]
struct Diag {
	error:		bool,
	line:		usize,
	col:		usize,
	message:	String,
	hints:		Vec<String>,
}

fn typst_diagnostics(dir: &Path, file: &str) -> Outcome<Vec<Diag>> {
	let out = res!(Command::new("typst")
		.args(["compile", "--root"])
		.arg(dir)
		.arg(file)
		.arg(format!("{}.pdf", file))
		.current_dir(dir)
		.output()
		.map_err(|e| err!("Running typst compile: {}", e; IO)));
	let text = String::from_utf8_lossy(&out.stderr).to_string();
	let mut diags: Vec<Diag> = Vec::new();
	for line in text.lines() {
		let t = line.trim_start();
		if let Some(m) = line.strip_prefix("error: ") {
			diags.push(Diag { error: true, line: 0, col: 0, message: m.to_string(), hints: Vec::new() });
		} else if let Some(m) = line.strip_prefix("warning: ") {
			diags.push(Diag { error: false, line: 0, col: 0, message: m.to_string(), hints: Vec::new() });
		} else if let Some(pos) = t.strip_prefix("┌─ ") {
			if let Some(d) = diags.last_mut() {
				if d.line == 0 {
					let mut parts = pos.rsplit(':');
					let col = parts.next().and_then(|c| c.trim().parse().ok()).unwrap_or(0);
					let ln = parts.next().and_then(|l| l.trim().parse().ok()).unwrap_or(0);
					d.line = ln;
					d.col = col;
				}
			}
		} else if let Some(h) = t.strip_prefix("= hint: ") {
			if let Some(d) = diags.last_mut() {
				d.hints.push(h.to_string());
			}
		}
	}
	Ok(diags)
}

fn line_col(text: &str, offset: u32) -> (usize, usize) {
	let offset = (offset as usize).min(text.len());
	let before = text.get(..offset).unwrap_or("");
	let line = before.matches('\n').count() + 1;
	let col = match before.rfind('\n') {
		Some(i)	=> before.get(i + 1..).unwrap_or("").chars().count(),
		None	=> before.chars().count(),
	};
	(line, col)
}

fn our_diagnostics(text: &str, root: &SyntaxNode) -> Vec<Diag> {
	let mut out = Vec::new();
	for (span, e) in root.errors() {
		let (line, col) = line_col(text, span.start);
		out.push(Diag { error: true, line, col, message: e.message.clone(), hints: e.hints.clone() });
	}
	for (span, w) in root.warnings() {
		let (line, col) = line_col(text, span.start);
		out.push(Diag { error: false, line, col, message: w.message.clone(), hints: w.hints.clone() });
	}
	out
}

// Every node's span is its exact slice of the source, and the leaves rebuild the source.
fn assert_lossless(text: &str, root: &SyntaxNode, what: &str) {
	assert_eq!(root.full_text(), text, "{}: the tree does not rebuild its source", what);
	assert_eq!(root.span(), Span::new(FILE, 0, text.len() as u32), "{}: the root span", what);
	fn walk(text: &str, n: &SyntaxNode, what: &str) {
		let slice = text.get(n.span().start as usize..n.span().end as usize);
		assert_eq!(slice, Some(n.full_text().as_str()), "{}: span of {:?} is not its text", what, n.kind());
		for c in n.children() {
			walk(text, c, what);
		}
	}
	walk(text, root, what);
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ A small JSON reader, for the oracle's output                                              │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

#[derive(Clone, Debug)]
enum Json {
	Null,
	Bool(bool),
	Int(i64),
	Float(f64),
	Str(String),
	Arr(Vec<Json>),
	Obj(Vec<(String, Json)>),
}

impl PartialEq for Json {
	fn eq(&self, other: &Self) -> bool {
		match (self, other) {
			(Json::Null, Json::Null)			=> true,
			(Json::Bool(a), Json::Bool(b))		=> a == b,
			(Json::Int(a), Json::Int(b))		=> a == b,
			(Json::Float(a), Json::Float(b))	=> (a - b).abs() <= 1e-9 * a.abs().max(1.0),
			(Json::Str(a), Json::Str(b))		=> a == b,
			(Json::Arr(a), Json::Arr(b))		=> a == b,
			// Element fields compare as sets; dictionary order is checked by `Json::keys_in_order`.
			(Json::Obj(a), Json::Obj(b))		=> a.len() == b.len()
				&& a.iter().all(|(k, v)| b.iter().any(|(k2, v2)| k == k2 && v == v2)),
			_									=> false,
		}
	}
}

impl Json {
	fn parse(s: &str) -> Outcome<Json> {
		let mut p = JsonParser { s: s.as_bytes(), i: 0 };
		let v = res!(p.value());
		p.ws();
		if p.i != p.s.len() {
			return Err(err!("Trailing JSON at byte {}.", p.i; Invalid, Input));
		}
		Ok(v)
	}

	fn obj(pairs: Vec<(&str, Json)>) -> Json {
		Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
	}

	fn elem(func: &str, mut fields: Vec<(&str, Json)>) -> Json {
		fields.insert(0, ("func", Json::Str(func.to_string())));
		Json::obj(fields)
	}

	fn str(s: &str) -> Json { Json::Str(s.to_string()) }

	fn get(&self, key: &str) -> Option<&Json> {
		match self {
			Json::Obj(pairs)	=> pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
			_					=> None,
		}
	}

	fn keys_in_order(&self, other: &Json) -> bool {
		match (self, other) {
			(Json::Obj(a), Json::Obj(b))	=> a.iter().map(|(k, _)| k).eq(b.iter().map(|(k, _)| k))
				&& a.iter().zip(b.iter()).all(|((_, x), (_, y))| x.keys_in_order(y)),
			(Json::Arr(a), Json::Arr(b))	=> a.iter().zip(b.iter()).all(|(x, y)| x.keys_in_order(y)),
			_								=> true,
		}
	}
}

struct JsonParser<'a> {
	s:	&'a [u8],
	i:	usize,
}

impl JsonParser<'_> {
	fn ws(&mut self) {
		while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\n' | b'\r' | b'\t') {
			self.i += 1;
		}
	}

	fn value(&mut self) -> Outcome<Json> {
		self.ws();
		match self.s.get(self.i) {
			Some(b'n')	=> { self.i += 4; Ok(Json::Null) },
			Some(b't')	=> { self.i += 4; Ok(Json::Bool(true)) },
			Some(b'f')	=> { self.i += 5; Ok(Json::Bool(false)) },
			Some(b'"')	=> Ok(Json::Str(res!(self.string()))),
			Some(b'[')	=> {
				self.i += 1;
				let mut v = Vec::new();
				loop {
					self.ws();
					if self.s.get(self.i) == Some(&b']') {
						self.i += 1;
						break;
					}
					v.push(res!(self.value()));
					self.ws();
					if self.s.get(self.i) == Some(&b',') {
						self.i += 1;
					}
				}
				Ok(Json::Arr(v))
			},
			Some(b'{')	=> {
				self.i += 1;
				let mut v = Vec::new();
				loop {
					self.ws();
					if self.s.get(self.i) == Some(&b'}') {
						self.i += 1;
						break;
					}
					let k = res!(self.string());
					self.ws();
					self.i += 1; // colon
					let val = res!(self.value());
					v.push((k, val));
					self.ws();
					if self.s.get(self.i) == Some(&b',') {
						self.i += 1;
					}
				}
				Ok(Json::Obj(v))
			},
			Some(_)		=> {
				let start = self.i;
				while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
					self.i += 1;
				}
				let t = String::from_utf8_lossy(&self.s[start..self.i]).to_string();
				if t.contains(['.', 'e', 'E']) {
					let f = res!(t.parse::<f64>().map_err(|e| err!("Bad float {:?}: {}", t, e; Invalid, Input)));
					Ok(Json::Float(f))
				} else {
					let n = res!(t.parse::<i64>().map_err(|e| err!("Bad int {:?}: {}", t, e; Invalid, Input)));
					Ok(Json::Int(n))
				}
			},
			None		=> Err(err!("Unexpected end of JSON."; Invalid, Input)),
		}
	}

	fn string(&mut self) -> Outcome<String> {
		self.i += 1;
		let mut out: Vec<u8> = Vec::new();
		while let Some(&b) = self.s.get(self.i) {
			self.i += 1;
			match b {
				b'"'	=> return Ok(String::from_utf8_lossy(&out).to_string()),
				b'\\'	=> {
					let e = self.s.get(self.i).copied().unwrap_or(b'\\');
					self.i += 1;
					match e {
						b'n'	=> out.push(b'\n'),
						b't'	=> out.push(b'\t'),
						b'r'	=> out.push(b'\r'),
						b'u'	=> {
							let hex = String::from_utf8_lossy(&self.s[self.i..self.i + 4]).to_string();
							self.i += 4;
							let mut cp = u32::from_str_radix(&hex, 16).unwrap_or(0xFFFD);
							// A surrogate pair.
							if (0xD800..0xDC00).contains(&cp) && self.s.get(self.i) == Some(&b'\\') {
								let lo = String::from_utf8_lossy(&self.s[self.i + 2..self.i + 6]).to_string();
								self.i += 6;
								let lo = u32::from_str_radix(&lo, 16).unwrap_or(0xDC00);
								cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
							}
							let c = char::from_u32(cp).unwrap_or('\u{FFFD}');
							let mut buf = [0u8; 4];
							out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
						},
						other	=> out.push(other),
					}
				},
				_		=> out.push(b),
			}
		}
		Err(err!("Unclosed JSON string."; Invalid, Input))
	}
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ Projection: the tree read the way typst-eval reads markup and maths                      │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

// Only what syntax alone decides: the constructs the fixtures use. Anything else is a fixture error.
fn project_markup(m: ast::Markup) -> Outcome<Json> {
	let mut seq: Vec<Json> = Vec::new();
	for e in m.exprs() {
		match e {
			Expr::Label(l)	=> {
				let target = seq.iter_mut().rev().find(|j| {
					!matches!(j.get("func"), Some(Json::Str(f)) if f == "space" || f == "parbreak")
				});
				if let Some(Json::Obj(pairs)) = target {
					pairs.retain(|(k, _)| k != "label");
					pairs.push(("label".to_string(), Json::Str(format!("<{}>", l.get()))));
				}
			},
			other			=> seq.push(res!(project_expr(other))),
		}
	}
	Ok(sequence(seq))
}

fn sequence(mut seq: Vec<Json>) -> Json {
	if seq.len() == 1 {
		return seq.remove(0);
	}
	Json::elem("sequence", vec![("children", Json::Arr(seq))])
}

// `a + b` on content: sequences are spliced, one level.
fn concat(parts: Vec<Json>) -> Json {
	let mut out = Vec::new();
	for p in parts {
		let is_seq = matches!(p.get("func"), Some(Json::Str(f)) if f == "sequence");
		match (is_seq, p.get("children")) {
			(true, Some(Json::Arr(c)))	=> out.extend(c.iter().cloned()),
			_							=> out.push(p),
		}
	}
	Json::elem("sequence", vec![("children", Json::Arr(out))])
}

fn text(t: &str) -> Json { Json::elem("text", vec![("text", Json::str(t))]) }
fn symbol(t: &str) -> Json { Json::elem("symbol", vec![("text", Json::str(t))]) }

fn project_expr(e: Expr) -> Outcome<Json> {
	Ok(match e {
		Expr::Text(t)			=> text(t.get()),
		Expr::Space(_)			=> Json::elem("space", vec![]),
		Expr::Linebreak(_)		=> Json::elem("linebreak", vec![]),
		Expr::Parbreak(_)		=> Json::elem("parbreak", vec![]),
		Expr::Escape(x)			=> symbol(&x.get().to_string()),
		Expr::Shorthand(x)		=> symbol(&x.get().to_string()),
		Expr::SmartQuote(q)		=> Json::elem("smartquote", vec![("double", Json::Bool(q.double()))]),
		Expr::Strong(s)			=> Json::elem("strong", vec![("body", res!(project_markup(s.body())))]),
		Expr::Emph(s)			=> Json::elem("emph", vec![("body", res!(project_markup(s.body())))]),
		Expr::Raw(r)			=> {
			let lines: Vec<&str> = r.lines().map(|l| l.get()).collect();
			let mut f = vec![("text", Json::Str(lines.join("\n"))), ("block", Json::Bool(r.block()))];
			if let Some(lang) = r.lang() {
				f.push(("lang", Json::str(lang.get())));
			}
			Json::elem("raw", f)
		},
		Expr::Link(l)			=> Json::elem("link", vec![("dest", Json::str(l.get())), ("body", text(l.get()))]),
		Expr::Ref(r)			=> {
			let mut f = vec![("target", Json::Str(format!("<{}>", r.target())))];
			if let Some(s) = r.supplement() {
				f.push(("supplement", res!(project_markup(s.body()))));
			}
			Json::elem("ref", f)
		},
		Expr::Heading(h)		=> Json::elem("heading", vec![
			("depth", Json::Int(h.depth() as i64)),
			("body", res!(project_markup(h.body()))),
		]),
		Expr::ListItem(i)		=> Json::elem("item", vec![("body", res!(project_markup(i.body())))]),
		Expr::EnumItem(i)		=> {
			let mut f = Vec::new();
			if let Some(n) = i.number() {
				f.push(("number", Json::Int(n as i64)));
			}
			f.push(("body", res!(project_markup(i.body()))));
			Json::elem("item", f)
		},
		Expr::TermItem(i)		=> Json::elem("item", vec![
			("term", res!(project_markup(i.term()))),
			("description", res!(project_markup(i.description()))),
		]),
		Expr::Equation(q)		=> Json::elem("equation", vec![
			("block", Json::Bool(q.block())),
			("body", res!(project_math(q.body()))),
		]),
		Expr::ContentBlock(b)	=> res!(project_markup(b.body())),
		Expr::Str(s)			=> text(&s.get()),
		Expr::Int(i)			=> text(&i.get().to_string()),
		Expr::None(_)			=> Json::elem("sequence", vec![("children", Json::Arr(vec![]))]),

		// Maths.
		Expr::Math(m)			=> res!(project_math(m)),
		Expr::MathText(t)		=> match t.get() {
			ast::MathTextKind::Number(n)	=> text(n),
			ast::MathTextKind::Grapheme(g)	=> symbol(g),
		},
		Expr::MathShorthand(s)	=> symbol(&s.get().to_string()),
		Expr::MathAlignPoint(_)	=> Json::elem("align-point", vec![]),
		Expr::MathDelimited(d)	=> Json::elem("lr", vec![("body", concat(vec![
			res!(project_expr(d.open())),
			res!(project_math(d.body())),
			res!(project_expr(d.close())),
		]))]),
		Expr::MathAttach(a)		=> {
			let mut f = vec![("base", res!(project_expr(a.base())))];
			if let Some(t) = a.top() {
				f.push(("t", res!(project_expr(t))));
			}
			if let Some(p) = a.primes() {
				f.push(("tr", Json::elem("primes", vec![("count", Json::Int(p.count() as i64))])));
			}
			if let Some(b) = a.bottom() {
				f.push(("b", res!(project_expr(b))));
			}
			Json::elem("attach", f)
		},
		Expr::MathCall(c)		=> res!(project_call(c)),
		Expr::MathPrimes(p)		=> Json::elem("primes", vec![("count", Json::Int(p.count() as i64))]),
		Expr::MathFrac(f)		=> Json::elem("frac", vec![
			("num", res!(project_expr(f.num()))),
			("denom", res!(project_expr(f.denom()))),
		]),
		Expr::MathRoot(r)		=> {
			let mut f = Vec::new();
			match r.index() {
				Some(i)	=> f.push(("index", text(&i.to_string()))),
				None	=> f.push(("index", Json::Null)),
			}
			f.push(("radicand", res!(project_expr(r.radicand()))));
			Json::elem("root", f)
		},
		other					=> return Err(err!(
			"The fixture uses {:?}, which the projection does not read.", other.to_untyped().kind();
			Unimplemented)),
	})
}

// The two maths functions the fixtures call, `sqrt` and `frac`, to check how arguments split.
fn project_call(c: ast::MathCall) -> Outcome<Json> {
	let name = match c.callee() {
		ast::MathAccess::MathIdent(i)	=> i.get().to_string(),
		other							=> return Err(err!("Callee {:?}.", other; Unimplemented)),
	};
	let mut args = Vec::new();
	for a in c.args().arg_items() {
		match a.arg {
			ast::Arg::Pos(e)	=> args.push(res!(project_expr(e))),
			other				=> return Err(err!("Argument {:?}.", other; Unimplemented)),
		}
	}
	Ok(match (name.as_str(), args.len()) {
		("sqrt", 1)	=> Json::elem("root", vec![("radicand", args.remove(0))]),
		("frac", 2)	=> {
			let denom = args.remove(1);
			Json::elem("frac", vec![("num", args.remove(0)), ("denom", denom)])
		},
		_			=> return Err(err!("Call to {} with {} arguments.", name, args.len(); Unimplemented)),
	})
}

fn project_math(m: ast::Math) -> Outcome<Json> {
	let mut seq = Vec::new();
	for e in m.exprs() {
		seq.push(res!(project_expr(e)));
	}
	Ok(sequence(seq))
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ A value reader for code, over the operators and literals the fixtures use                 │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

fn value(e: Expr) -> Outcome<Json> {
	Ok(match e {
		Expr::Int(i)			=> Json::Int(i.get()),
		Expr::Float(f)			=> Json::Float(f.get()),
		Expr::Bool(b)			=> Json::Bool(b.get()),
		Expr::None(_)			=> Json::Null,
		Expr::Str(s)			=> Json::Str(s.get()),
		Expr::Parenthesized(p)	=> res!(value(p.expr())),
		Expr::Array(a)			=> {
			let mut v = Vec::new();
			for item in a.items() {
				match item {
					ast::ArrayItem::Pos(e)		=> v.push(res!(value(e))),
					ast::ArrayItem::Spread(s)	=> match res!(value(s.expr())) {
						Json::Arr(more)	=> v.extend(more),
						other			=> return Err(err!("Spread of {:?}.", other; Invalid)),
					},
				}
			}
			Json::Arr(v)
		},
		Expr::Dict(d)			=> {
			let mut v: Vec<(String, Json)> = Vec::new();
			let put = |k: String, x: Json, v: &mut Vec<(String, Json)>| {
				v.retain(|(k2, _)| *k2 != k);
				v.push((k, x));
			};
			for item in d.items() {
				match item {
					ast::DictItem::Named(n)		=> put(n.name().get().to_string(), res!(value(n.expr())), &mut v),
					ast::DictItem::Keyed(k)		=> match res!(value(k.key())) {
						Json::Str(key)	=> put(key, res!(value(k.expr())), &mut v),
						other			=> return Err(err!("Key {:?}.", other; Invalid)),
					},
					ast::DictItem::Spread(s)	=> match res!(value(s.expr())) {
						Json::Obj(more)	=> for (k, x) in more { put(k, x, &mut v); },
						other			=> return Err(err!("Spread of {:?}.", other; Invalid)),
					},
				}
			}
			Json::Obj(v)
		},
		Expr::Unary(u)			=> {
			let x = res!(value(u.expr()));
			match (u.op(), x) {
				(ast::UnOp::Neg, Json::Int(n))		=> Json::Int(-n),
				(ast::UnOp::Neg, Json::Float(f))	=> Json::Float(-f),
				(ast::UnOp::Pos, x)					=> x,
				(ast::UnOp::Not, Json::Bool(b))		=> Json::Bool(!b),
				(op, x)								=> return Err(err!("{:?} {:?}.", op, x; Invalid)),
			}
		},
		Expr::Binary(b)			=> res!(binary(b)),
		other					=> return Err(err!("The code fixture uses {:?}.", other.to_untyped().kind();
			Unimplemented)),
	})
}

fn num(j: &Json) -> Option<f64> {
	match j {
		Json::Int(n)	=> Some(*n as f64),
		Json::Float(f)	=> Some(*f),
		_				=> None,
	}
}

fn binary(b: ast::Binary) -> Outcome<Json> {
	use ast::BinOp as B;
	let op = b.op();
	let l = res!(value(b.lhs()));
	// `and` and `or` short-circuit.
	if let (B::And | B::Or, Json::Bool(x)) = (op, &l) {
		if (op == B::And && !*x) || (op == B::Or && *x) {
			return Ok(Json::Bool(*x));
		}
		return value(b.rhs());
	}
	let r = res!(value(b.rhs()));
	Ok(match (op, &l, &r) {
		(B::Add, Json::Int(x), Json::Int(y))	=> Json::Int(x + y),
		(B::Sub, Json::Int(x), Json::Int(y))	=> Json::Int(x - y),
		(B::Mul, Json::Int(x), Json::Int(y))	=> Json::Int(x * y),
		(B::Add, Json::Str(x), Json::Str(y))	=> Json::Str(format!("{}{}", x, y)),
		(B::Add, Json::Arr(x), Json::Arr(y))	=> Json::Arr(x.iter().chain(y.iter()).cloned().collect()),
		(B::Add, Json::Obj(x), Json::Obj(y))	=> {
			let mut v = x.clone();
			for (k, val) in y {
				v.retain(|(k2, _)| k2 != k);
				v.push((k.clone(), val.clone()));
			}
			Json::Obj(v)
		},
		(B::Eq, _, _)							=> Json::Bool(l == r),
		(B::Neq, _, _)							=> Json::Bool(l != r),
		(B::In, _, Json::Arr(a))				=> Json::Bool(a.contains(&l)),
		(B::NotIn, _, Json::Arr(a))				=> Json::Bool(!a.contains(&l)),
		(B::Add | B::Sub | B::Mul | B::Div | B::Lt | B::Leq | B::Gt | B::Geq, _, _) => {
			match (num(&l), num(&r)) {
				(Some(x), Some(y))	=> match op {
					B::Add	=> Json::Float(x + y),
					B::Sub	=> Json::Float(x - y),
					B::Mul	=> Json::Float(x * y),
					B::Div	=> Json::Float(x / y),
					B::Lt	=> Json::Bool(x < y),
					B::Leq	=> Json::Bool(x <= y),
					B::Gt	=> Json::Bool(x > y),
					_		=> Json::Bool(x >= y),
				},
				_					=> return Err(err!("{:?} on {:?} and {:?}.", op, l, r; Invalid)),
			}
		},
		_										=> return Err(err!("{:?} on {:?} and {:?}.", op, l, r; Invalid)),
	})
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ Tests                                                                                     │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

// Markup and maths evaluate, in Typst, to the content the tree's shape implies.
fn content_oracle(fixture_name: &str, dir_name: &str) -> Outcome<()> {
	if !res!(typst_available()) {
		return Ok(());
	}
	let cases = res!(fixture(fixture_name));
	assert!(cases.len() >= 20, "{} holds {} cases", fixture_name, cases.len());
	let dir = res!(work_dir(dir_name));
	let mut main = String::new();
	for (i, c) in cases.iter().enumerate() {
		res!(write(&dir.join(format!("c{}.typ", i)), c));
		main.push_str(&format!("#metadata(include \"c{}.typ\") <p>\n", i));
	}
	res!(write(&dir.join("main.typ"), &main));
	let expected = match res!(typst_probe_values(&dir)) {
		Json::Arr(v)	=> v,
		other			=> return Err(err!("Probe output {:?}.", other; Invalid)),
	};
	assert_eq!(expected.len(), cases.len());

	let mut failures = Vec::new();
	for (i, (case, want)) in cases.iter().zip(expected.iter()).enumerate() {
		let root = parser::parse(case, FILE);
		assert_lossless(case, &root, &format!("{} case {}", fixture_name, i));
		assert!(root.errors().is_empty(), "{} case {} has syntax errors: {:?}", fixture_name, i, root.errors());
		let markup = match ast::Markup::from_untyped(&root) {
			Some(m)	=> m,
			None	=> return Err(err!("The root is not markup."; Invalid)),
		};
		let got = res!(project_markup(markup));
		if &got != want {
			failures.push(format!("case {}: {:?}\n  typst:  {:?}\n  ours:   {:?}", i, case, want, got));
		}
	}
	assert!(failures.is_empty(), "{} of {} {} cases differ from typst:\n{}",
		failures.len(), cases.len(), fixture_name, failures.join("\n"));
	Ok(())
}

#[test]
fn markup_evaluates_as_typst_reads_it() -> Outcome<()> {
	content_oracle("markup.typ", "markup")
}

#[test]
fn maths_evaluates_as_typst_reads_it() -> Outcome<()> {
	content_oracle("math.typ", "math")
}

#[test]
fn code_precedence_and_literals_match_typst() -> Outcome<()> {
	if !res!(typst_available()) {
		return Ok(());
	}
	let cases = res!(fixture("code.typ"));
	assert!(cases.len() >= 30, "code.typ holds {} cases", cases.len());
	let dir = res!(work_dir("code"));
	let mut main = String::new();
	for c in &cases {
		main.push_str(&format!("#metadata({{\n{}\n}}) <p>\n", c));
	}
	res!(write(&dir.join("main.typ"), &main));
	let expected = match res!(typst_probe_values(&dir)) {
		Json::Arr(v)	=> v,
		other			=> return Err(err!("Probe output {:?}.", other; Invalid)),
	};
	assert_eq!(expected.len(), cases.len());

	let mut failures = Vec::new();
	for (i, (case, want)) in cases.iter().zip(expected.iter()).enumerate() {
		let root = parser::parse_code(case, FILE);
		assert_lossless(case, &root, &format!("code case {}", i));
		assert!(root.errors().is_empty(), "code case {} has syntax errors: {:?}", i, root.errors());
		let code = match ast::Code::from_untyped(&root) {
			Some(c)	=> c,
			None	=> return Err(err!("The root is not code."; Invalid)),
		};
		let exprs: Vec<Expr> = code.exprs().collect();
		assert_eq!(exprs.len(), 1, "code case {} parses to {} expressions", i, exprs.len());
		let got = res!(value(exprs[0]));
		if &got != want || !got.keys_in_order(want) {
			failures.push(format!("case {}: {:?}\n  typst:  {:?}\n  ours:   {:?}", i, case, want, got));
		}
	}
	assert!(failures.is_empty(), "{} of {} code cases differ from typst:\n{}",
		failures.len(), cases.len(), failures.join("\n"));
	Ok(())
}

fn diagnostics_oracle(fixture_name: &str, dir_name: &str, errors: bool) -> Outcome<usize> {
	if !res!(typst_available()) {
		return Ok(0);
	}
	let cases = res!(fixture(fixture_name));
	let dir = res!(work_dir(dir_name));
	let mut failures = Vec::new();
	let mut compared = 0;
	for (i, case) in cases.iter().enumerate() {
		let name = format!("e{}.typ", i);
		res!(write(&dir.join(&name), case));
		let want = res!(typst_diagnostics(&dir, &name));
		let root = parser::parse(case, FILE);
		assert_lossless(case, &root, &format!("{} case {}", fixture_name, i));
		let got: Vec<Diag> = our_diagnostics(case, &root).into_iter().filter(|d| d.error == errors).collect();
		let want: Vec<Diag> = want.into_iter().filter(|d| d.error == errors).collect();
		compared += want.len();
		if got != want {
			failures.push(format!("case {}: {:?}\n  typst:  {:?}\n  ours:   {:?}", i, case, want, got));
		}
	}
	assert!(failures.is_empty(), "{} of {} {} cases differ from typst:\n{}",
		failures.len(), cases.len(), fixture_name, failures.join("\n"));
	Ok(compared)
}

#[test]
fn syntax_errors_match_typst_in_message_position_and_hints() -> Outcome<()> {
	let n = res!(diagnostics_oracle("errors.typ", "errors", true));
	assert!(n >= 50 || skip_allowed(), "only {} errors compared", n);
	Ok(())
}

#[test]
fn syntax_warnings_match_typst() -> Outcome<()> {
	let n = res!(diagnostics_oracle("warnings.typ", "warnings", false));
	assert!(n >= 5 || skip_allowed(), "only {} warnings compared", n);
	Ok(())
}

// Random documents over Typst's significant characters: wherever Austenite finds a syntax error, Typst
// reports exactly the same ones; wherever it finds none, Typst reports no syntax error either.
#[test]
fn random_documents_draw_the_same_syntax_errors_as_typst() -> Outcome<()> {
	if !res!(typst_available()) {
		return Ok(());
	}
	const ALPHABET: &[&str] = &[
		"*", "_", "=", "-", "+", "/", ":", "[", "]", "$", "`", "\"", "'", ".", "~", "<", ">", "@", "#",
		"(", ")", "{", "}", ",", ";", " ", " ", "\n", "\n\n", "a", "b", "1", "x", "let", "if", "\\",
		"^", "!", "&", "|", "..", "=>", "in",
	];
	const SYNTAX: &[&str] = &[
		"unclosed", "expected", "unexpected", "invalid", "duplicate", "the character", "label cannot",
		"automatic links", "only one", "maximum parsing depth", "`&&`", "`||`", "`~=`",
	];
	let dir = res!(work_dir("fuzz"));
	// `SYNTAX_FUZZ_N` and `SYNTAX_FUZZ_SEED` widen a local run; the default keeps the suite quick.
	let n: usize = std::env::var("SYNTAX_FUZZ_N").ok().and_then(|v| v.parse().ok()).unwrap_or(150);
	let mut seed: u64 = std::env::var("SYNTAX_FUZZ_SEED").ok().and_then(|v| v.parse::<u64>().ok())
		.unwrap_or(0x2545_F491_4F6C_DD1D).max(1);
	let mut next = move || {
		seed ^= seed << 13;
		seed ^= seed >> 7;
		seed ^= seed << 17;
		seed
	};
	let mut compared = 0;
	let mut failures = Vec::new();
	for i in 0..n {
		let len = 3 + (next() % 24) as usize;
		let mut doc = String::new();
		for _ in 0..len {
			doc.push_str(ALPHABET[(next() % ALPHABET.len() as u64) as usize]);
		}
		let root = parser::parse(&doc, FILE);
		assert_lossless(&doc, &root, &format!("fuzz {}", i));
		let name = format!("f{}.typ", i);
		res!(write(&dir.join(&name), &doc));
		let want: Vec<Diag> = res!(typst_diagnostics(&dir, &name)).into_iter().filter(|d| d.error).collect();
		let got: Vec<Diag> = our_diagnostics(&doc, &root).into_iter().filter(|d| d.error).collect();
		if !got.is_empty() {
			compared += 1;
			if got != want {
				failures.push(format!("{:?}\n  typst:  {:?}\n  ours:   {:?}", doc, want, got));
			}
		} else if let Some(d) = want.iter().find(|d| SYNTAX.iter().any(|s| d.message.starts_with(s))) {
			failures.push(format!("{:?}\n  typst reports {:?}; ours none", doc, d));
		}
	}
	assert!(compared >= 30, "only {} random documents had syntax errors", compared);
	assert!(failures.is_empty(), "{} random documents differ:\n{}", failures.len(), failures.join("\n"));
	Ok(())
}

// The contract gate: the preview packages Typst documents lean on parse without a single error node,
// losslessly, with exact spans.
#[test]
fn package_corpus_parses_without_error_nodes() -> Outcome<()> {
	let home = std::env::var("HOME").unwrap_or_default();
	let base = PathBuf::from(home).join(".cache/typst/packages/preview");
	let mut files = Vec::new();
	for pkg in ["cetz", "cetz-plot", "fletcher", "tablex", "oxifmt"] {
		collect_typ(&base.join(pkg), &mut files);
	}
	if files.is_empty() {
		assert!(skip_allowed(), "no package corpus under {:?}; set EVAL_ORACLE_SKIP=1 to skip", base);
		return Ok(());
	}
	assert!(files.len() >= 150, "the corpus holds only {} files", files.len());
	let mut bad = Vec::new();
	let mut bytes = 0;
	for f in &files {
		let text = res!(fs::read_to_string(f).map_err(|e| err!("Reading {:?}: {}", f, e; IO, File)));
		bytes += text.len();
		let root = parser::parse(&text, FILE);
		assert_lossless(&text, &root, &f.display().to_string());
		let errs = root.errors();
		if let Some((span, e)) = errs.first() {
			let (l, c) = line_col(&text, span.start);
			bad.push(format!("{}:{}:{}: {} ({} errors)", f.display(), l, c, e.message, errs.len()));
		}
	}
	println!("[syntax] {} package files, {} bytes, parsed", files.len(), bytes);
	assert!(bad.is_empty(), "{} package files have error nodes:\n{}", bad.len(), bad.join("\n"));
	Ok(())
}

fn collect_typ(dir: &Path, out: &mut Vec<PathBuf>) {
	if let Ok(rd) = fs::read_dir(dir) {
		let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
		entries.sort();
		for p in entries {
			if p.is_dir() {
				collect_typ(&p, out);
			} else if p.extension().is_some_and(|x| x == "typ") {
				out.push(p);
			}
		}
	}
}

// The three entry points return the root kinds the evaluator dispatches on, and a malformed input still
// yields a whole, lossless tree.
#[test]
fn entry_points_return_whole_trees() {
	for (text, root) in [
		("a *b* $c$ #d", parser::parse("a *b* $c$ #d", FILE)),
		("let x = 1; x + (2, 3)", parser::parse_code("let x = 1; x + (2, 3)", FILE)),
		("a_1^2 / (b)", parser::parse_math("a_1^2 / (b)", FILE)),
		("#{ ((] $ `", parser::parse("#{ ((] $ `", FILE)),
	] {
		assert_lossless(text, &root, text);
	}
	assert_eq!(parser::parse("x", FILE).kind(), SyntaxKind::Markup);
	assert_eq!(parser::parse_code("x", FILE).kind(), SyntaxKind::Code);
	assert_eq!(parser::parse_math("x", FILE).kind(), SyntaxKind::Math);
	assert!(parser::parse("#{ ((] $ `", FILE).erroneous());
	assert!(!parser::parse("a *b* $c$ #d", FILE).erroneous());
}

// Random error-free documents over markup and maths punctuation (no `#`, and letters only as a lone `x`
// so maths never names a variable): Typst evaluates each to the content its tree implies.
#[test]
fn random_documents_evaluate_as_typst_reads_them() -> Outcome<()> {
	if !res!(typst_available()) {
		return Ok(());
	}
	const ALPHABET: &[&str] = &[
		"x ", "x ", "1", "2.5", " ", " ", "\n", "\n\n", "*", "_", "=", "-", "+", "/", ":", "[", "]", "$",
		"$", "`", "\"", "'", ".", "~", "\\", "^", "!", "&", "|", "(", ")", "{", "}", ",", ";", "<", ">",
		"@x ", "--", "...", "√", "⌈", "⌉", "[|", "|]", "->", "é",
	];
	let n: usize = std::env::var("SYNTAX_FUZZ_N").ok().and_then(|v| v.parse().ok()).unwrap_or(600);
	let mut seed: u64 = std::env::var("SYNTAX_FUZZ_SEED").ok().and_then(|v| v.parse::<u64>().ok())
		.unwrap_or(0x9E37_79B9_7F4A_7C15).max(1);
	let mut next = move || {
		seed ^= seed << 13;
		seed ^= seed >> 7;
		seed ^= seed << 17;
		seed
	};
	let dir = res!(work_dir("fuzz_content"));
	let mut docs = Vec::new();
	let mut main = String::new();
	for _ in 0..n {
		let len = 2 + (next() % 20) as usize;
		let mut doc = String::new();
		for _ in 0..len {
			doc.push_str(ALPHABET[(next() % ALPHABET.len() as u64) as usize]);
		}
		let root = parser::parse(&doc, FILE);
		if root.erroneous() {
			continue;
		}
		res!(write(&dir.join(format!("c{}.typ", docs.len())), &doc));
		main.push_str(&format!("#metadata(include \"c{}.typ\") <p>\n", docs.len()));
		docs.push(doc);
	}
	assert!(docs.len() >= n / 5, "only {} of {} random documents were error-free", docs.len(), n);
	res!(write(&dir.join("main.typ"), &main));
	let expected = match res!(typst_probe_values(&dir)) {
		Json::Arr(v)	=> v,
		other			=> return Err(err!("Probe output {:?}.", other; Invalid)),
	};
	assert_eq!(expected.len(), docs.len());
	let mut failures = Vec::new();
	for (doc, want) in docs.iter().zip(expected.iter()) {
		let root = parser::parse(doc, FILE);
		let markup = match ast::Markup::from_untyped(&root) {
			Some(m)	=> m,
			None	=> return Err(err!("The root is not markup."; Invalid)),
		};
		let got = res!(project_markup(markup));
		if &got != want {
			failures.push(format!("{:?}\n  typst:  {:?}\n  ours:   {:?}", doc, want, got));
		}
	}
	assert!(failures.is_empty(), "{} of {} random documents differ:\n{}",
		failures.len(), docs.len(), failures.join("\n"));
	Ok(())
}
