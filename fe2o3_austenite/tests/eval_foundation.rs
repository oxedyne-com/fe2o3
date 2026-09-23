//! U3's oracle tests: the foundation library (calc, str, array, dict, numbering, colour, sym, datetime,
//! data, foundations, geometry) against `typst` 0.15.1.
//!
//! Each fixture under `tests/fixtures/eval/foundation/` is a list of Typst expressions, one per line.
//! A plain line must evaluate in both engines to the same `repr`; a line starting `!` must fail in both
//! with the same message, and `!~` must fail in both whatever the message. The expressions are
//! evaluated here by a small expression reader that calls the library natively (literals, field
//! access, calls, method calls, unary minus and `+`/`-` between literals), so the library is tested
//! before the evaluator core lands; the expected values come from `typst eval`, never from Austenite.
//! A missing `typst` fails the suite unless `EVAL_ORACLE_SKIP=1` is set.

use oxedyne_fe2o3_austenite::eval::args::Args;
use oxedyne_fe2o3_austenite::eval::func::{
	Func,
	NativeFunc,
};
use oxedyne_fe2o3_austenite::eval::lib::foundations::{
	field,
	func_scope,
	method,
	repr,
	type_scope,
	constructor,
};
use oxedyne_fe2o3_austenite::eval::lib::{
	array,
	dict,
	library,
	sym,
};
use oxedyne_fe2o3_austenite::eval::scope::Scope;
use oxedyne_fe2o3_austenite::eval::value::{
	Alignment,
	Angle,
	Dict,
	Fraction,
	Length,
	Ratio,
	Relative,
	Value,
};
use oxedyne_fe2o3_austenite::eval::{
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::path::PathBuf;
use std::process::Command;

// The expression reader.

struct Reader<'a> {
	s:		&'a [u8],
	src:	&'a str,
	i:		usize,
}

type R<T> = Outcome<T>;

enum Arg {
	Pos(Value),
	Named(String, Value),
}

fn is_ident_start(c: u8) -> bool { c.is_ascii_alphabetic() || c == b'_' }

fn is_ident_char(c: u8) -> bool { c.is_ascii_alphanumeric() || c == b'_' || c == b'-' }

impl<'a> Reader<'a> {
	fn ws(&mut self) {
		while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
			self.i += 1;
		}
	}

	fn peek(&self) -> Option<u8> { self.s.get(self.i).copied() }

	fn eat(&mut self, c: u8) -> bool {
		self.ws();
		if self.peek() == Some(c) {
			self.i += 1;
			true
		} else {
			false
		}
	}

	fn expr(&mut self, e: &mut Engine) -> R<Value> {
		let mut v = res!(self.unary(e));
		loop {
			self.ws();
			match self.peek() {
				Some(b'+') => {
					self.i += 1;
					let w = res!(self.unary(e));
					v = res!(add(v, w, 1.0));
				}
				Some(b'-') => {
					self.i += 1;
					let w = res!(self.unary(e));
					v = res!(add(v, w, -1.0));
				}
				Some(b'*') => {
					self.i += 1;
					let w = res!(self.unary(e));
					v = match (v, w) {
						(Value::Int(a), Value::Int(b))		=> Value::Int(a * b),
						(Value::Float(a), Value::Int(b))	=> Value::Float(a * b as f64),
						(Value::Int(a), Value::Float(b))	=> Value::Float(a as f64 * b),
						(Value::Float(a), Value::Float(b))	=> Value::Float(a * b),
						(Value::Str(a), Value::Int(n))		=> Value::str(a.repeat(n as usize)),
						(Value::Array(a), Value::Int(n))	=> {
							let mut out = Vec::new();
							for _ in 0..n {
								out.extend(a.iter().cloned());
							}
							Value::array(out)
						}
						_ => return Err(err!("unsupported `*`"; Invalid)),
					};
				}
				Some(b'/') => {
					self.i += 1;
					let w = res!(self.unary(e));
					v = match (v, w) {
						(Value::Int(a), Value::Int(b))		=> Value::Float(a as f64 / b as f64),
						(Value::Float(a), Value::Int(b))	=> Value::Float(a / b as f64),
						(Value::Int(a), Value::Float(b))	=> Value::Float(a as f64 / b),
						(Value::Float(a), Value::Float(b))	=> Value::Float(a / b),
						(Value::Length(a), Value::Int(b))	=> Value::Length(Length { abs: a.abs / b as f64, em: a.em / b as f64 }),
						_ => return Err(err!("unsupported `/`"; Invalid)),
					};
				}
				_ => return Ok(v),
			}
		}
	}

	fn unary(&mut self, e: &mut Engine) -> R<Value> {
		self.ws();
		if self.peek() == Some(b'-') {
			self.i += 1;
			let v = res!(self.unary(e));
			return neg(v);
		}
		self.postfix(e)
	}

	fn postfix(&mut self, e: &mut Engine) -> R<Value> {
		let mut v = res!(self.atom(e));
		loop {
			// No whitespace before `.` or `(`: that is how Typst reads a chain.
			match self.peek() {
				Some(b'.') if self.s.get(self.i + 1).map(|c| is_ident_start(*c)).unwrap_or(false) => {
					self.i += 1;
					let name = self.ident();
					if self.peek() == Some(b'(') {
						let args = res!(self.args(e));
						v = res!(call_method(e, v, &name, args));
					} else {
						v = res!(access(v, &name));
					}
				}
				Some(b'(') => {
					let args = res!(self.args(e));
					v = res!(call(e, &v, args, None));
				}
				_ => return Ok(v),
			}
		}
	}

	fn ident(&mut self) -> String {
		let start = self.i;
		while self.i < self.s.len() && is_ident_char(self.s[self.i]) {
			self.i += 1;
		}
		self.src[start..self.i].to_string()
	}

	fn args(&mut self, e: &mut Engine) -> R<Vec<Arg>> {
		self.i += 1;
		let mut out = Vec::new();
		loop {
			self.ws();
			if self.eat(b')') {
				return Ok(out);
			}
			// A named argument is an identifier followed by a colon.
			let save = self.i;
			if self.peek().map(is_ident_start).unwrap_or(false) {
				let name = self.ident();
				self.ws();
				if self.peek() == Some(b':') {
					self.i += 1;
					let v = res!(self.expr(e));
					out.push(Arg::Named(name, v));
					self.ws();
					self.eat(b',');
					continue;
				}
				self.i = save;
			}
			let v = res!(self.expr(e));
			out.push(Arg::Pos(v));
			self.ws();
			if !self.eat(b',') {
				self.ws();
				if self.eat(b')') {
					return Ok(out);
				}
				return Err(err!("expected `,` or `)` at {}", self.i; Invalid));
			}
		}
	}

	fn atom(&mut self, e: &mut Engine) -> R<Value> {
		self.ws();
		match self.peek() {
			None => Err(err!("unexpected end"; Invalid)),
			Some(b'"') => self.string().map(Value::str),
			Some(b'<') => {
				let end = res!(self.src[self.i..].find('>').ok_or_else(|| err!("unclosed label"; Invalid)));
				let name = self.src[self.i + 1..self.i + end].to_string();
				self.i += end + 1;
				Ok(Value::Label(oxedyne_fe2o3_austenite::eval::value::Label::new(&name)))
			}
			Some(b'(') => self.group(e),
			Some(c) if c.is_ascii_digit() || c == b'.' => self.number(),
			Some(c) if is_ident_start(c) => {
				let name = self.ident();
				match name.as_str() {
					"none"	=> Ok(Value::None),
					"auto"	=> Ok(Value::Auto),
					"true"	=> Ok(Value::Bool(true)),
					"false"	=> Ok(Value::Bool(false)),
					_		=> library().get(&name).cloned().ok_or_else(|| err!("unknown variable: {}", name; Invalid)),
				}
			}
			Some(c) => Err(err!("unexpected `{}`", c as char; Invalid)),
		}
	}

	fn group(&mut self, e: &mut Engine) -> R<Value> {
		self.i += 1;
		self.ws();
		if self.eat(b')') {
			return Ok(Value::array(Vec::new()));
		}
		if self.peek() == Some(b':') {
			self.i += 1;
			self.ws();
			if !self.eat(b')') {
				return Err(err!("expected `)` after `(:`"; Invalid));
			}
			return Ok(Value::dict(Dict::new()));
		}
		let mut items = Vec::new();
		let mut dict = Dict::new();
		let mut is_dict = false;
		let mut trailing = false;
		loop {
			self.ws();
			if self.eat(b')') {
				break;
			}
			let save = self.i;
			let mut key: Option<String> = None;
			if self.peek().map(is_ident_start).unwrap_or(false) {
				let k = self.ident();
				self.ws();
				if self.peek() == Some(b':') {
					key = Some(k);
					self.i += 1;
				} else {
					self.i = save;
				}
			} else if self.peek() == Some(b'"') {
				let k = res!(self.string());
				self.ws();
				if self.peek() == Some(b':') {
					key = Some(k);
					self.i += 1;
				} else {
					self.i = save;
				}
			}
			let v = res!(self.expr(e));
			match key {
				Some(k) => {
					is_dict = true;
					dict.insert(&k, v);
				}
				None => items.push(v),
			}
			self.ws();
			trailing = self.eat(b',');
			if !trailing {
				self.ws();
				if !self.eat(b')') {
					return Err(err!("expected `)` at {}", self.i; Invalid));
				}
				break;
			}
		}
		if is_dict {
			return Ok(Value::dict(dict));
		}
		if items.len() == 1 && !trailing {
			return Ok(items.remove(0));
		}
		Ok(Value::array(items))
	}

	fn string(&mut self) -> R<String> {
		self.i += 1;
		let mut out = String::new();
		loop {
			let rest = &self.src[self.i..];
			let c = res!(rest.chars().next().ok_or_else(|| err!("unterminated string"; Invalid)));
			self.i += c.len_utf8();
			match c {
				'"'		=> return Ok(out),
				'\\'	=> {
					let e = res!(self.src[self.i..].chars().next().ok_or_else(|| err!("bad escape"; Invalid)));
					self.i += e.len_utf8();
					match e {
						'n'		=> out.push('\n'),
						't'		=> out.push('\t'),
						'r'		=> out.push('\r'),
						'\\'	=> out.push('\\'),
						'"'		=> out.push('"'),
						'u'		=> {
							let end = res!(self.src[self.i..].find('}').ok_or_else(|| err!("bad unicode escape"; Invalid)));
							let hex = &self.src[self.i + 1..self.i + end];
							let cp = res!(u32::from_str_radix(hex, 16).map_err(|e| err!("{}", e; Invalid)));
							out.push(res!(char::from_u32(cp).ok_or_else(|| err!("bad codepoint"; Invalid))));
							self.i += end + 1;
						}
						other	=> return Err(err!("unknown escape {}", other; Invalid)),
					}
				}
				c => out.push(c),
			}
		}
	}

	fn number(&mut self) -> R<Value> {
		let start = self.i;
		if self.s[self.i..].starts_with(b"0x") || self.s[self.i..].starts_with(b"0b") || self.s[self.i..].starts_with(b"0o") {
			let radix = match self.s[self.i + 1] { b'x' => 16, b'b' => 2, _ => 8 };
			self.i += 2;
			let d = self.i;
			while self.i < self.s.len() && self.s[self.i].is_ascii_alphanumeric() {
				self.i += 1;
			}
			return i64::from_str_radix(&self.src[d..self.i], radix).map(Value::Int).map_err(|e| err!("{}", e; Invalid));
		}
		let mut float = false;
		while self.i < self.s.len() {
			let c = self.s[self.i];
			if c.is_ascii_digit() {
				self.i += 1;
			} else if c == b'.' && self.s.get(self.i + 1).map(|d| d.is_ascii_digit()).unwrap_or(false) {
				float = true;
				self.i += 1;
			} else if (c == b'e' || c == b'E') && self.s.get(self.i + 1).map(|d| d.is_ascii_digit() || *d == b'-' || *d == b'+').unwrap_or(false) {
				float = true;
				self.i += 2;
			} else {
				break;
			}
		}
		let num = &self.src[start..self.i];
		let x: f64 = res!(num.parse().map_err(|e: std::num::ParseFloatError| err!("{}", e; Invalid)));
		let unit_start = self.i;
		while self.i < self.s.len() && (self.s[self.i].is_ascii_alphabetic() || self.s[self.i] == b'%') {
			self.i += 1;
		}
		let unit = &self.src[unit_start..self.i];
		Ok(match unit {
			""		=> if float { Value::Float(x) } else {
				num.parse::<i64>().map(Value::Int).unwrap_or(Value::Float(x))
			},
			"pt"	=> Value::Length(Length::pt(x)),
			"mm"	=> Value::Length(Length::pt(x * 72.0 / 25.4)),
			"cm"	=> Value::Length(Length::pt(x * 720.0 / 25.4)),
			"in"	=> Value::Length(Length::pt(x * 72.0)),
			"em"	=> Value::Length(Length::em(x)),
			"%"		=> Value::Ratio(Ratio(x / 100.0)),
			"deg"	=> Value::Angle(Angle(x.to_radians())),
			"rad"	=> Value::Angle(Angle(x)),
			"fr"	=> Value::Fraction(Fraction(x)),
			other	=> return Err(err!("unknown unit {}", other; Invalid)),
		})
	}
}

fn neg(v: Value) -> R<Value> {
	Ok(match v {
		Value::Int(i)		=> Value::Int(-i),
		Value::Float(f)		=> Value::Float(-f),
		Value::Length(l)	=> Value::Length(Length { abs: -l.abs, em: -l.em }),
		Value::Ratio(r)		=> Value::Ratio(Ratio(-r.0)),
		Value::Angle(a)		=> Value::Angle(Angle(-a.0)),
		Value::Fraction(f)	=> Value::Fraction(Fraction(-f.0)),
		_					=> return Err(err!("unsupported unary minus"; Invalid)),
	})
}

fn add(a: Value, b: Value, sign: f64) -> R<Value> {
	let b = if sign < 0.0 { res!(neg(b)) } else { b };
	Ok(match (a, b) {
		(Value::Int(x), Value::Int(y))			=> Value::Int(x + y),
		(Value::Float(x), Value::Float(y))		=> Value::Float(x + y),
		(Value::Int(x), Value::Float(y))		=> Value::Float(x as f64 + y),
		(Value::Float(x), Value::Int(y))		=> Value::Float(x + y as f64),
		(Value::Str(x), Value::Str(y))			=> Value::str(format!("{}{}", x, y)),
		(Value::Length(x), Value::Length(y))	=> Value::Length(Length { abs: x.abs + y.abs, em: x.em + y.em }),
		(Value::Ratio(r), Value::Length(l)) | (Value::Length(l), Value::Ratio(r)) => Value::Relative(Relative { rel: r, abs: l }),
		(Value::Ratio(x), Value::Ratio(y))		=> Value::Ratio(Ratio(x.0 + y.0)),
		(Value::Angle(x), Value::Angle(y))		=> Value::Angle(Angle(x.0 + y.0)),
		(Value::Alignment(x), Value::Alignment(y)) => Value::Alignment(Alignment { x: x.x.or(y.x), y: x.y.or(y.y) }),
		_ => return Err(err!("unsupported `+`"; Invalid)),
	})
}

fn access(v: Value, name: &str) -> R<Value> {
	match &v {
		Value::Module(m)	=> m.scope.get(name).cloned().ok_or_else(|| err!("module does not contain `{}`", name; Invalid)),
		Value::Type(t)		=> type_scope(*t, name).ok_or_else(|| err!("type does not contain `{}`", name; Invalid)),
		Value::Symbol(s)	=> sym::modify(s, name).map(Value::Symbol).ok_or_else(|| err!("unknown symbol modifier"; Invalid)),
		Value::Dict(d)		=> d.get(name).cloned().ok_or_else(|| err!("dictionary does not contain key {}", name; Invalid)),
		Value::Func(f)		=> func_scope(f, name).ok_or_else(|| err!("function does not contain `{}`", name; Invalid)),
		_					=> field(&v, name).ok_or_else(|| err!("no field {}", name; Invalid)),
	}
}

fn build(args: Vec<Arg>) -> Args {
	let mut a = Args::new(Span::detached());
	for x in args {
		match x {
			Arg::Pos(v)			=> a.push(Span::detached(), v),
			Arg::Named(n, v)	=> a.push_named(Span::detached(), n, v),
		}
	}
	a
}

fn call(e: &mut Engine, f: &Value, args: Vec<Arg>, recv: Option<Value>) -> R<Value> {
	let mut a = build(args);
	if let Some(r) = recv {
		a.prepend(Span::detached(), r);
	}
	let native = match f {
		Value::Func(Func::Native(n))	=> *n,
		Value::Type(t)					=> res!(constructor(*t).ok_or_else(|| err!("type has no constructor"; Invalid))),
		_								=> return Err(err!("not callable here"; Invalid)),
	};
	native.call(e, a)
}

// A method call. Mutating methods need a place, so they run on a copy and return what the call returned.
fn call_method(e: &mut Engine, recv: Value, name: &str, args: Vec<Arg>) -> R<Value> {
	// A module, type or function holds members rather than methods: `calc.pow(..)`, `str.to-unicode(..)`.
	if matches!(recv, Value::Module(_) | Value::Type(_) | Value::Func(_)) {
		let f = res!(access(recv, name));
		return call(e, &f, args, None);
	}
	// A literal receiver is a temporary, so a mutating method fails here as it does in Typst; places are
	// tested by `mutating_methods_agree_with_typst`.
	let ty = recv.ty();
	if let Some(m) = method(ty, name) {
		return call(e, &Value::Func(Func::Native(m)), args, Some(recv));
	}
	Err(err!("type {} has no method `{}`", ty.name(), name; Invalid))
}

/// Evaluates one expression; the error is the last diagnostic's message when there is one.
fn eval_line(line: &str) -> std::result::Result<Value, String> {
	let mut e = Engine::new(World::new(PathBuf::from("/")));
	let mut r = Reader { s: line.as_bytes(), src: line, i: 0 };
	let out = r.expr(&mut e);
	r.ws();
	match out {
		Ok(_) if r.i < r.s.len()	=> Err(format!("trailing input at {}", r.i)),
		Ok(v)						=> Ok(v),
		Err(err)					=> Err(match e.diags.last() {
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

// Typst's reprs for many expressions at once, through a document that exposes them as metadata.
fn oracle_reprs(bin: &str, name: &str, exprs: &[&str]) -> Outcome<Vec<String>> {
	let file = res!(scratch()).join(format!("{}.typ", name));
	let mut src = String::from("#metadata((\n");
	for e in exprs {
		src.push_str(&format!("  repr({}),\n", e));
	}
	src.push_str(")) <reprs>\n");
	res!(std::fs::write(&file, src));
	let out = res!(Command::new(bin)
		.args(["eval", "query(<reprs>).first().value", "--in"])
		.arg(&file)
		.args(["--format", "json", "--root", "/"])
		.output());
	if !out.status.success() {
		return Err(err!("typst failed on {}: {}", name, String::from_utf8_lossy(&out.stderr); Test));
	}
	parse_json_strings(&String::from_utf8_lossy(&out.stdout))
}

// The oracle's first error line, or `None` when the expression evaluates.
fn oracle_error(bin: &str, expr: &str) -> Outcome<Option<String>> {
	let file = res!(scratch()).join("err.typ");
	res!(std::fs::write(&file, ""));
	let out = res!(Command::new(bin)
		.args(["eval", &format!("repr({})", expr), "--in"])
		.arg(&file)
		.output());
	if out.status.success() {
		return Ok(None);
	}
	let err = String::from_utf8_lossy(&out.stderr);
	Ok(err.lines().find_map(|l| l.strip_prefix("error: ").map(|s| s.to_string())))
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
	let expected = res!(oracle_reprs(&bin, name.trim_end_matches(".txt"), &exprs));
	if expected.len() != exprs.len() {
		return Err(err!("oracle returned {} values for {} expressions", expected.len(), exprs.len(); Test));
	}
	let mut bad = Vec::new();
	for (e, want) in exprs.iter().zip(expected.iter()) {
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
				(Some(w), Err(m)) => if *strict && w != m {
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

/// Functions passed as arguments (`map`, `sorted(key:)`) and the operator-backed methods (`sum`, `join`,
/// `calc.max`) call `Engine::call_func` and `ops`, which the evaluator core (U2) provides.
#[test]
#[ignore = "needs U2: Engine::call_func and ops::{add, mul, join, compare}"]
fn higher_order_agrees_with_typst() -> Outcome<()> { run("higher_order.txt") }

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

/// `push`, `pop`, `insert` and `remove` on a place: the receiver and the call's result after the call,
/// against `{let a = ..; let r = a.m(..); (r, a)}` in Typst.
#[test]
fn mutating_methods_agree_with_typst() -> Outcome<()> {
	let bin = match typst() {
		Some(b)					=> b,
		None if skip_allowed()	=> return Ok(()),
		None					=> return Err(err!("typst is not on PATH"; Test)),
	};
	let cases: [(&str, &str, &str); 12] = [
		("(1, 2, 3)", "push", "4"),
		("(1, 2, 3)", "pop", ""),
		("(1, 2, 3)", "insert", "1, 9"),
		("(1, 2, 3)", "insert", "3, 9"),
		("(1, 2, 3)", "insert", "-1, 9"),
		("(1, 2, 3)", "remove", "0"),
		("(1, 2, 3)", "remove", "-1"),
		("(1, 2, 3)", "remove", "5, default: 9"),
		("(a: 1)", "insert", "\"b\", 2"),
		("(a: 1, b: 2)", "insert", "\"a\", 3"),
		("(a: 1, b: 2)", "remove", "\"a\""),
		("(a: 1)", "remove", "\"z\", default: 0"),
	];
	let exprs: Vec<String> = cases.iter()
		.map(|(r, m, a)| format!("{{let a = {}; let r = a.{}({}); (r, a)}}", r, m, a))
		.collect();
	let refs: Vec<&str> = exprs.iter().map(|s| s.as_str()).collect();
	let want = res!(oracle_reprs(&bin, "mutate", &refs));
	let mut bad = Vec::new();
	for ((r, m, a), (e, w)) in cases.iter().zip(refs.iter().zip(want.iter())) {
		let mut place = match eval_line(r) {
			Ok(v)	=> v,
			Err(msg) => return Err(err!("receiver {}: {}", r, msg; Test)),
		};
		let mut eng = Engine::new(World::new(PathBuf::from("/")));
		let src = format!("f({})", a);
		let mut rd = Reader { s: src.as_bytes(), src: &src, i: 1 };
		let args = res!(rd.args(&mut eng));
		let result = match (place.ty(), method(place.ty(), m)) {
			(_, Some(NativeFunc::Array(f)))	=> array::call_mut(f, &mut eng, &mut place, build(args)),
			(_, Some(NativeFunc::Dict(f)))	=> dict::call_mut(f, &mut eng, &mut place, build(args)),
			(t, _)							=> return Err(err!("{} has no mutating {}", t.name(), m; Test)),
		};
		let got = match result {
			Ok(v)	=> repr(&Value::array(vec![v, place])),
			Err(x)	=> format!("error: {}", x),
		};
		if !same_repr(&got, w) {
			bad.push(format!("{}: typst {} austenite {}", e, w, got));
		}
	}
	if bad.is_empty() {
		Ok(())
	} else {
		Err(err!("{} mutation(s) differ:\n{}", bad.len(), bad.join("\n"); Test))
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
