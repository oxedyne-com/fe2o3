// U3 owns this file. Data loading through `crate::vfs`, paths resolved with `import::resolve_path`, and the
// readers and writers behind `read`, `json`, `csv`, `yaml`, `toml`, `xml` and `cbor` (with `.encode` for
// JSON, YAML, TOML and CBOR). Every format is read here into Typst values directly: JSON keeps object
// order and the integer/float distinction, TOML dates become `datetime`s, and a document a reader cannot
// take is a diagnostic naming the line and column, never an empty value. XML goes through
// `fe2o3_text::xml`. YAML is the block and flow subset real documents use (mappings, sequences, scalars
// of the 1.2 core schema, quoted and block scalars, anchors and aliases); tags and complex keys are
// refused with a diagnostic.

use crate::diag::DiagnosticKind;
use crate::eval::args::Args;
use crate::eval::content::Content;
use crate::eval::func::{
	Func,
	NativeFunc,
};
use crate::eval::import::resolve_path;
use crate::eval::lib::foundations::{
	finish,
	mismatch,
	need,
	repr,
	words,
};
use crate::eval::scope::Scope;
use crate::eval::value::{
	Datetime,
	Dict,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;
use crate::vfs;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

native_fns! {
	pub enum DataFn {
		Read		=> "read",
		Json		=> "json",
		Csv			=> "csv",
		Yaml		=> "yaml",
		Toml		=> "toml",
		Xml			=> "xml",
		Cbor		=> "cbor",
		JsonEncode	=> "encode",
		YamlEncode	=> "encode",
		TomlEncode	=> "encode",
		CborEncode	=> "encode",
	}
}

pub fn define(scope: &mut Scope) {
	for f in [DataFn::Read, DataFn::Json, DataFn::Csv, DataFn::Yaml, DataFn::Toml, DataFn::Xml, DataFn::Cbor] {
		scope.define(f.name(), Value::Func(Func::Native(NativeFunc::Data(f))));
	}
}

/// `json.encode` and its kin: the members a data function carries.
pub fn func_scope(f: DataFn, name: &str) -> Option<Value> {
	let g = match (f, name) {
		(DataFn::Json, "encode")	=> DataFn::JsonEncode,
		(DataFn::Yaml, "encode")	=> DataFn::YamlEncode,
		(DataFn::Toml, "encode")	=> DataFn::TomlEncode,
		(DataFn::Cbor, "encode")	=> DataFn::CborEncode,
		_							=> return None,
	};
	Some(Value::Func(Func::Native(NativeFunc::Data(g))))
}

// The bytes a data function reads: a path through the vfs, or bytes given inline.
fn source(engine: &mut Engine, span: Span, v: Value) -> Outcome<Arc<Vec<u8>>> {
	match v {
		Value::Str(p) => {
			let path = res!(resolve_path(engine, &p, span.file, span));
			match vfs::read(&path) {
				Ok(b)	=> Ok(Arc::new(b)),
				Err(e)	=> Err(engine.error(DiagnosticKind::MissingFile, span, if e.kind() == std::io::ErrorKind::NotFound {
					fmt!("file not found (searched at {})", path.display())
				} else {
					fmt!("failed to load file ({})", e)
				})),
			}
		}
		Value::Bytes(b) => Ok(b),
		other => Err(mismatch(engine, span, "string or bytes", &other)),
	}
}

fn utf8<'a>(engine: &mut Engine, span: Span, b: &'a [u8]) -> Outcome<&'a str> {
	match std::str::from_utf8(b) {
		Ok(s)	=> Ok(s.strip_prefix('\u{feff}').unwrap_or(s)),
		Err(_)	=> Err(engine.error(DiagnosticKind::Encoding, span, "file is not valid utf-8")),
	}
}

// A parse failure at a byte offset, as Typst words it: "failed to parse JSON (message at line L column C
// at L:C)". The column counts the offending character, as serde's does, and at the end of input the
// characters read.
fn parse_error(engine: &mut Engine, span: Span, fmt_name: &str, text: &str, at: usize, msg: &str) -> Error<ErrTag> {
	let (line, col) = line_col(text, at);
	engine.error(DiagnosticKind::Type, span, fmt!("failed to parse {} ({} at line {} column {} at {}:{})",
		fmt_name, msg, line, col, line, col.max(1)))
}

fn line_col(text: &str, at: usize) -> (usize, usize) {
	let end = at >= text.len();
	let at = at.min(text.len());
	let before = &text[..at];
	let line = before.matches('\n').count() + 1;
	let col = before.rsplit('\n').next().map(|l| l.chars().count()).unwrap_or(0);
	(line, if end { col } else { col + 1 })
}

pub fn call(f: DataFn, engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	match f {
		DataFn::Read => {
			let p = res!(need(engine, &mut args, "path"));
			let enc = res!(args.named::<Value>("encoding"));
			res!(finish(engine, args));
			let path = match p {
				Value::Str(s)	=> s,
				other			=> return Err(mismatch(engine, span, "string", &other)),
			};
			let b = res!(source(engine, span, Value::Str(path)));
			match enc {
				None | Some(Value::Str(_)) => {
					if let Some(Value::Str(e)) = &enc {
						if e.as_str() != "utf8" {
							return Err(engine.error(DiagnosticKind::Type, span, "expected \"utf8\" or none"));
						}
					}
					let s = res!(utf8(engine, span, &b));
					Ok(Value::str(s))
				}
				Some(Value::None) => Ok(Value::Bytes(b)),
				Some(other) => Err(mismatch(engine, span, "string or none", &other)),
			}
		}
		DataFn::Json | DataFn::Yaml | DataFn::Toml | DataFn::Xml | DataFn::Cbor | DataFn::Csv => {
			let src = res!(need(engine, &mut args, "source"));
			let (delim, dict_rows) = if f == DataFn::Csv {
				let d = match res!(args.named::<Value>("delimiter")) {
					None					=> ',',
					Some(Value::Str(s))	=> {
						let mut cs = s.chars();
						match (cs.next(), cs.next()) {
							(Some(c), None) if c.is_ascii()	=> c,
							(Some(_), None)	=> return Err(engine.error(DiagnosticKind::Type, span, "delimiter must be an ASCII character")),
							_				=> return Err(engine.error(DiagnosticKind::Type, span, "delimiter must be a single character")),
						}
					}
					Some(other) => return Err(mismatch(engine, span, "string", &other)),
				};
				let rows = match res!(args.named::<Value>("row-type")) {
					None => false,
					Some(Value::Type(t)) if t == crate::eval::value::Type::Array => false,
					Some(Value::Type(t)) if t == crate::eval::value::Type::Dict => true,
					Some(_) => return Err(engine.error(DiagnosticKind::Type, span, "expected `array` or `dictionary`")),
				};
				(d, rows)
			} else {
				(',', false)
			};
			res!(finish(engine, args));
			let b = res!(source(engine, span, src));
			match f {
				DataFn::Cbor => {
					let mut r = CborReader { b: &b, at: 0, depth: 0 };
					match r.value() {
						Ok(v) if r.at == b.len()	=> Ok(v),
						Ok(_)						=> Err(engine.error(DiagnosticKind::Type, span, "failed to parse CBOR (trailing data)")),
						Err(e)						=> Err(engine.error(DiagnosticKind::Type, span, fmt!("failed to parse CBOR ({})", words(&e)))),
					}
				}
				_ => {
					let text = res!(utf8(engine, span, &b)).to_string();
					match f {
						DataFn::Json => {
							let mut p = JsonParser { s: text.as_bytes(), at: 0, depth: 0, fail: None };
							match p.document() {
								Ok(v)	=> Ok(v),
								Err(_)	=> {
									let (at, m) = p.fail.clone().unwrap_or((0, "invalid JSON".to_string()));
									Err(parse_error(engine, span, "JSON", &text, at, &m))
								}
							}
						}
						DataFn::Toml => match toml_parse(&text) {
							Ok(v)			=> Ok(v),
							Err((at, m))	=> Err(parse_error(engine, span, "TOML", &text, at, &m)),
						},
						DataFn::Yaml => match yaml_parse(&text) {
							Ok(v)			=> Ok(v),
							Err((at, m))	=> Err(parse_error(engine, span, "YAML", &text, at, &m)),
						},
						DataFn::Xml => xml_value(engine, span, &text),
						_ => csv_value(engine, span, &text, delim, dict_rows),
					}
				}
			}
		}
		DataFn::JsonEncode | DataFn::YamlEncode | DataFn::TomlEncode | DataFn::CborEncode => {
			let v = res!(need(engine, &mut args, "value"));
			let pretty = if f == DataFn::CborEncode {
				false
			} else {
				match res!(args.named::<Value>("pretty")) {
					None				=> true,
					Some(Value::Bool(b))	=> b,
					Some(other)			=> return Err(mismatch(engine, span, "boolean", &other)),
				}
			};
			res!(finish(engine, args));
			let data = to_data(&v);
			match f {
				DataFn::JsonEncode => {
					let mut out = String::new();
					json_write(&data, pretty, 0, &mut out);
					Ok(Value::str(out))
				}
				DataFn::YamlEncode => {
					let mut out = String::new();
					yaml_write(&data, 0, &mut out, true);
					Ok(Value::str(out))
				}
				DataFn::TomlEncode => match &data {
					Data::Map(m) => {
						let mut out = String::new();
						toml_write_table(m, &[], pretty, &mut out);
						Ok(Value::str(out))
					}
					_ => Err(mismatch(engine, span, "dictionary", &v)),
				},
				_ => {
					let mut out = Vec::new();
					cbor_write(&data, &mut out);
					Ok(Value::Bytes(Arc::new(out)))
				}
			}
		}
	}
}

// Serialisable data: what a Typst value becomes on its way to a text or binary format.

enum Data {
	Null,
	Bool(bool),
	Int(i64),
	Float(f64),
	Str(String),
	Bytes(Vec<u8>),
	Seq(Vec<Data>),
	Map(Vec<(String, Data)>),
}

fn to_data(v: &Value) -> Data {
	match v {
		Value::None			=> Data::Null,
		Value::Bool(b)		=> Data::Bool(*b),
		Value::Int(i)		=> Data::Int(*i),
		Value::Float(f)		=> Data::Float(*f),
		Value::Str(s)		=> Data::Str((**s).clone()),
		Value::Bytes(b)		=> Data::Bytes((**b).clone()),
		Value::Array(a)		=> Data::Seq(a.iter().map(to_data).collect()),
		Value::Dict(d)		=> Data::Map(d.iter().map(|(k, v)| (k.to_string(), to_data(v))).collect()),
		Value::Symbol(s)	=> Data::Str(crate::eval::lib::sym::text(s).to_string()),
		Value::Content(c)	=> content_data(c),
		other				=> Data::Str(repr(other)),
	}
}

// Content serialises as its fields under `func`, as Typst's does.
fn content_data(c: &Content) -> Data {
	match c {
		Content::Sequence(s) => Data::Map(vec![
			("func".to_string(), Data::Str("sequence".to_string())),
			("children".to_string(), Data::Seq(s.children.iter().map(content_data).collect())),
		]),
		Content::Styled(s) => content_data(&s.child),
		Content::Elem(e) => {
			let mut m = vec![("func".to_string(), Data::Str(e.kind.name().to_string()))];
			let specs = e.kind.fields();
			for (i, spec) in specs.iter().enumerate() {
				if let Some((_, v)) = e.fields.iter().find(|(f, _)| f.0 as usize == i) {
					m.push((spec.name.to_string(), to_data(v)));
				}
			}
			if let Some(l) = &e.label {
				m.push(("label".to_string(), Data::Str(fmt!("<{}>", l.as_str()))));
			}
			Data::Map(m)
		}
	}
}

/// A float as `ryu` (and so serde) writes it: shortest round-trip digits, scientific below 1e-5 and from
/// 1e16.
pub fn ryu_format(f: f64) -> String {
	if f == 0.0 {
		return if f.is_sign_negative() { "-0.0".to_string() } else { "0.0".to_string() };
	}
	let sci = fmt!("{:e}", f);
	let (mant, exp) = match sci.split_once('e') {
		Some(p)	=> p,
		None	=> return sci,
	};
	let exp: i32 = exp.parse().unwrap_or(0);
	let neg = mant.starts_with('-');
	let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
	let len = digits.len() as i32;
	let kk = exp + 1;
	let mut out = String::new();
	if neg {
		out.push('-');
	}
	if len <= kk && kk <= 16 {
		out.push_str(&digits);
		out.push_str(&"0".repeat((kk - len) as usize));
		out.push_str(".0");
	} else if 0 < kk && kk <= 16 {
		out.push_str(&digits[..kk as usize]);
		out.push('.');
		out.push_str(&digits[kk as usize..]);
	} else if -5 < kk && kk <= 0 {
		out.push_str("0.");
		out.push_str(&"0".repeat((-kk) as usize));
		out.push_str(&digits);
	} else if len == 1 {
		out.push_str(&digits);
		out.push('e');
		out.push_str(&(kk - 1).to_string());
	} else {
		out.push_str(&digits[..1]);
		out.push('.');
		out.push_str(&digits[1..]);
		out.push('e');
		out.push_str(&(kk - 1).to_string());
	}
	out
}

// JSON

struct JsonParser<'a> {
	s:		&'a [u8],
	at:		usize,
	depth:	usize,
	fail:	Option<(usize, String)>,	// where and why parsing stopped, for the diagnostic
}


impl<'a> JsonParser<'a> {
	fn ws(&mut self) {
		while self.at < self.s.len() && matches!(self.s[self.at], b' ' | b'\t' | b'\n' | b'\r') {
			self.at += 1;
		}
	}

	fn err<T>(&mut self, m: &str) -> Outcome<T> {
		let at = self.at;
		self.fail_at(at, m)
	}

	fn fail_at<T>(&mut self, at: usize, m: &str) -> Outcome<T> {
		self.fail = Some((at, m.to_string()));
		Err(err!("{}", m; Decode, Input))
	}

	fn document(&mut self) -> Outcome<Value> {
		self.ws();
		let v = res!(self.value());
		self.ws();
		if self.at < self.s.len() {
			return self.err("trailing characters");
		}
		Ok(v)
	}

	fn value(&mut self) -> Outcome<Value> {
		self.ws();
		if self.depth > 128 {
			return self.err("recursion limit exceeded");
		}
		match self.s.get(self.at) {
			None		=> self.err("EOF while parsing a value"),
			Some(b'{')	=> {
				self.at += 1;
				self.depth += 1;
				let mut d = Dict::new();
				self.ws();
				if self.s.get(self.at) == Some(&b'}') {
					self.at += 1;
					self.depth -= 1;
					return Ok(Value::dict(d));
				}
				loop {
					self.ws();
					if self.s.get(self.at) != Some(&b'"') {
						return match self.s.get(self.at) {
							Some(b'}')	=> self.err("trailing comma"),
							None		=> self.err("EOF while parsing an object"),
							Some(_)		=> self.err("key must be a string"),
						};
					}
					let k = res!(self.string());
					self.ws();
					if self.s.get(self.at) != Some(&b':') {
						return if self.at >= self.s.len() { self.err("EOF while parsing an object") } else { self.err("expected `:`") };
					}
					self.at += 1;
					let v = res!(self.value());
					d.insert(&k, v);
					self.ws();
					match self.s.get(self.at) {
						Some(b',')	=> self.at += 1,
						Some(b'}')	=> {
							self.at += 1;
							self.depth -= 1;
							return Ok(Value::dict(d));
						}
						None => return self.err("EOF while parsing an object"),
						_ => return self.err("expected `,` or `}`"),
					}
				}
			}
			Some(b'[')	=> {
				self.at += 1;
				self.depth += 1;
				let mut a = Vec::new();
				self.ws();
				if self.s.get(self.at) == Some(&b']') {
					self.at += 1;
					self.depth -= 1;
					return Ok(Value::array(a));
				}
				loop {
					self.ws();
					if self.s.get(self.at) == Some(&b']') {
						return self.err("trailing comma");
					}
					match self.value() {
						Ok(v)	=> a.push(v),
						Err(e)	=> return Err(e),
					}
					self.ws();
					match self.s.get(self.at) {
						Some(b',')	=> self.at += 1,
						Some(b']')	=> {
							self.at += 1;
							self.depth -= 1;
							return Ok(Value::array(a));
						}
						None => return self.err("EOF while parsing a list"),
						_ => return self.err("expected `,` or `]`"),
					}
				}
			}
			Some(b'"')	=> self.string().map(Value::str),
			Some(b't')	=> self.word("true", Value::Bool(true)),
			Some(b'f')	=> self.word("false", Value::Bool(false)),
			Some(b'n')	=> self.word("null", Value::None),
			Some(c) if *c == b'-' || c.is_ascii_digit() => self.number(),
			Some(_)		=> self.err("expected value"),
		}
	}

	fn word(&mut self, w: &str, v: Value) -> Outcome<Value> {
		if self.s[self.at..].starts_with(w.as_bytes()) {
			self.at += w.len();
			Ok(v)
		} else {
			self.err("expected ident")
		}
	}

	fn number(&mut self) -> Outcome<Value> {
		let start = self.at;
		if self.s.get(self.at) == Some(&b'-') {
			self.at += 1;
		}
		let int_start = self.at;
		while self.at < self.s.len() && self.s[self.at].is_ascii_digit() {
			self.at += 1;
		}
		if self.at == int_start {
			return self.err("invalid number");
		}
		if self.at - int_start > 1 && self.s[int_start] == b'0' {
			return self.fail_at(int_start + 1, "invalid number");
		}
		let mut float = false;
		if self.s.get(self.at) == Some(&b'.') {
			float = true;
			self.at += 1;
			let f = self.at;
			while self.at < self.s.len() && self.s[self.at].is_ascii_digit() {
				self.at += 1;
			}
			if self.at == f {
				return self.err("invalid number");
			}
		}
		if matches!(self.s.get(self.at), Some(b'e') | Some(b'E')) {
			float = true;
			self.at += 1;
			if matches!(self.s.get(self.at), Some(b'+') | Some(b'-')) {
				self.at += 1;
			}
			let e = self.at;
			while self.at < self.s.len() && self.s[self.at].is_ascii_digit() {
				self.at += 1;
			}
			if self.at == e {
				return self.err("invalid number");
			}
		}
		let text = std::str::from_utf8(&self.s[start..self.at]).unwrap_or("0");
		// A negative zero has no integer form; serde reads it as the float.
		if !float && text != "-0" {
			if let Ok(i) = text.parse::<i64>() {
				return Ok(Value::Int(i));
			}
		}
		match text.parse::<f64>() {
			Ok(f) if f.is_finite()	=> Ok(if f == 0.0 && text.starts_with('-') { Value::Float(-0.0) } else { Value::Float(f) }),
			_						=> {
				let at = self.at - 1;
				self.fail_at(at, "number out of range")
			}
		}
	}

	fn hex4(&mut self) -> Outcome<u32> {
		let h = self.s.get(self.at..self.at + 4).and_then(|b| std::str::from_utf8(b).ok())
			.and_then(|t| u32::from_str_radix(t, 16).ok());
		match h {
			Some(v)	=> {
				self.at += 4;
				Ok(v)
			}
			None	=> self.err("invalid escape"),
		}
	}

	fn string(&mut self) -> Outcome<String> {
		self.at += 1;
		let mut out = String::new();
		loop {
			let start = self.at;
			while self.at < self.s.len() && self.s[self.at] != b'"' && self.s[self.at] != b'\\' && self.s[self.at] >= 0x20 {
				self.at += 1;
			}
			out.push_str(std::str::from_utf8(&self.s[start..self.at]).unwrap_or(""));
			match self.s.get(self.at) {
				None		=> return self.err("EOF while parsing a string"),
				Some(b'"')	=> {
					self.at += 1;
					return Ok(out);
				}
				Some(b'\\')	=> {
					self.at += 1;
					let c = match self.s.get(self.at) {
						Some(c)	=> *c,
						None	=> return self.err("EOF while parsing a string"),
					};
					self.at += 1;
					match c {
						b'"'	=> out.push('"'),
						b'\\'	=> out.push('\\'),
						b'/'	=> out.push('/'),
						b'b'	=> out.push('\u{8}'),
						b'f'	=> out.push('\u{c}'),
						b'n'	=> out.push('\n'),
						b'r'	=> out.push('\r'),
						b't'	=> out.push('\t'),
						b'u'	=> {
							let hi = res!(self.hex4());
							let cp = if (0xD800..0xDC00).contains(&hi) {
								if !self.s[self.at..].starts_with(b"\\u") {
									return self.err("lone leading surrogate in hex escape");
								}
								self.at += 2;
								let lo = res!(self.hex4());
								if !(0xDC00..0xE000).contains(&lo) {
									return self.err("invalid unicode code point");
								}
								0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
							} else {
								hi
							};
							match char::from_u32(cp) {
								Some(ch)	=> out.push(ch),
								None		=> return self.err("invalid unicode code point"),
							}
						}
						_ => return self.err("invalid escape"),
					}
				}
				Some(_) => return self.err("control character (\\u0000-\\u001F) found while parsing a string"),
			}
		}
	}
}

fn json_str(s: &str, out: &mut String) {
	out.push('"');
	for c in s.chars() {
		match c {
			'"'		=> out.push_str("\\\""),
			'\\'	=> out.push_str("\\\\"),
			'\n'	=> out.push_str("\\n"),
			'\r'	=> out.push_str("\\r"),
			'\t'	=> out.push_str("\\t"),
			'\u{8}'	=> out.push_str("\\b"),
			'\u{c}'	=> out.push_str("\\f"),
			c if (c as u32) < 0x20 => out.push_str(&fmt!("\\u{:04x}", c as u32)),
			c		=> out.push(c),
		}
	}
	out.push('"');
}

fn json_write(d: &Data, pretty: bool, indent: usize, out: &mut String) {
	let nl = |out: &mut String, n: usize| {
		if pretty {
			out.push('\n');
			out.push_str(&"  ".repeat(n));
		}
	};
	match d {
		Data::Null		=> out.push_str("null"),
		Data::Bool(b)	=> out.push_str(if *b { "true" } else { "false" }),
		Data::Int(i)	=> out.push_str(&i.to_string()),
		Data::Float(f)	=> if f.is_finite() { out.push_str(&ryu_format(*f)) } else { out.push_str("null") },
		Data::Str(s)	=> json_str(s, out),
		Data::Bytes(b)	=> json_write(&Data::Seq(b.iter().map(|x| Data::Int(*x as i64)).collect()), pretty, indent, out),
		Data::Seq(a)	=> {
			if a.is_empty() {
				out.push_str("[]");
				return;
			}
			out.push('[');
			for (i, x) in a.iter().enumerate() {
				if i > 0 {
					out.push(',');
				}
				nl(out, indent + 1);
				json_write(x, pretty, indent + 1, out);
			}
			nl(out, indent);
			out.push(']');
		}
		Data::Map(m)	=> {
			if m.is_empty() {
				out.push_str("{}");
				return;
			}
			out.push('{');
			for (i, (k, x)) in m.iter().enumerate() {
				if i > 0 {
					out.push(',');
				}
				nl(out, indent + 1);
				json_str(k, out);
				out.push(':');
				if pretty {
					out.push(' ');
				}
				json_write(x, pretty, indent + 1, out);
			}
			nl(out, indent);
			out.push('}');
		}
	}
}

// CSV

fn csv_value(engine: &mut Engine, span: Span, text: &str, delim: char, dict_rows: bool) -> Outcome<Value> {
	let mut rows: Vec<Vec<String>> = Vec::new();
	let mut row: Vec<String> = Vec::new();
	let mut field = String::new();
	let mut quoted = false;
	let mut in_quotes = false;
	let mut chars = text.chars().peekable();
	let mut line = 1usize;
	let mut expected: Option<usize> = None;
	let mut row_line = 1usize;
	let finish_row = |row: &mut Vec<String>, rows: &mut Vec<Vec<String>>, expected: &mut Option<usize>, line: usize|
		-> std::result::Result<(), String>
	{
		let r = std::mem::take(row);
		match expected {
			None		=> *expected = Some(r.len()),
			Some(n) if *n != r.len() => return Err(fmt!(
				"found {} instead of {} fields in line {} at {}:1", r.len(), n, line, line)),
			_ => (),
		}
		rows.push(r);
		Ok(())
	};
	let mut any = false;
	while let Some(c) = chars.next() {
		any = true;
		if in_quotes {
			if c == '"' {
				if chars.peek() == Some(&'"') {
					chars.next();
					field.push('"');
				} else {
					in_quotes = false;
				}
			} else {
				if c == '\n' {
					line += 1;
				}
				field.push(c);
			}
			continue;
		}
		if c == '"' && field.is_empty() && !quoted {
			in_quotes = true;
			quoted = true;
		} else if c == delim {
			row.push(std::mem::take(&mut field));
			quoted = false;
		} else if c == '\n' || c == '\r' {
			if c == '\r' && chars.peek() == Some(&'\n') {
				chars.next();
			}
			row.push(std::mem::take(&mut field));
			quoted = false;
			if let Err(m) = finish_row(&mut row, &mut rows, &mut expected, row_line) {
				return Err(engine.error(DiagnosticKind::Type, span, fmt!("failed to parse CSV ({})", m)));
			}
			line += 1;
			row_line = line;
			any = false;
		} else {
			field.push(c);
		}
	}
	if any || !field.is_empty() || !row.is_empty() {
		row.push(field);
		if let Err(m) = finish_row(&mut row, &mut rows, &mut expected, row_line) {
			return Err(engine.error(DiagnosticKind::Type, span, fmt!("failed to parse CSV ({})", m)));
		}
	}
	if dict_rows {
		let mut it = rows.into_iter();
		let header = it.next().unwrap_or_default();
		let out = it.map(|r| {
			let mut d = Dict::new();
			for (k, v) in header.iter().zip(r) {
				d.insert(k, Value::str(v));
			}
			Value::dict(d)
		}).collect();
		Ok(Value::array(out))
	} else {
		Ok(Value::array(rows.into_iter().map(|r| Value::array(r.into_iter().map(Value::str).collect())).collect()))
	}
}

// XML

fn xml_value(engine: &mut Engine, span: Span, text: &str) -> Outcome<Value> {
	use oxedyne_fe2o3_text::xml::{
		Node,
		Xml,
	};
	let doc = match Xml::parse(text) {
		Ok(d)	=> d,
		Err(e)	=> return Err(engine.error(DiagnosticKind::Type, span, fmt!("failed to parse XML ({})",
			e.msgs().last().cloned().unwrap_or_default()))),
	};
	// Typst reads XML through roxmltree, whose tree this reproduces: adjacent text and CDATA merge
	// into one string, a comment or a processing instruction is a node with an empty tag, and the XML
	// declaration and the document type are not nodes at all.
	fn empty_node() -> Value {
		let mut d = Dict::new();
		d.insert("namespace", Value::None);
		d.insert("tag", Value::str(""));
		d.insert("attrs", Value::dict(Dict::new()));
		d.insert("children", Value::array(Vec::new()));
		Value::dict(d)
	}
	fn is_decl(doc: &Xml, n: &Node) -> bool {
		match n {
			Node::Pi(s)		=> {
				let raw = doc.raw(s);
				raw.starts_with("<?xml") && raw[5..].starts_with(|c: char| c.is_whitespace() || c == '?')
			}
			Node::DocType(_)	=> true,
			_				=> false,
		}
	}
	fn children(doc: &Xml, nodes: &[Node], top: bool) -> Vec<Value> {
		let mut out = Vec::new();
		let mut text: Option<String> = None;
		for n in nodes {
			let piece = match n {
				Node::Text(s)	=> Some(doc.text(s).to_string()),
				Node::CData(s)	=> {
					let raw = doc.raw(s);
					Some(raw.strip_prefix("<![CDATA[").and_then(|r| r.strip_suffix("]]>")).unwrap_or(raw).to_string())
				}
				_				=> None,
			};
			match piece {
				Some(p) => {
					if !top {
						text.get_or_insert_with(String::new).push_str(&p);
					}
					continue;
				}
				None => if let Some(t) = text.take() {
					out.push(Value::str(t));
				},
			}
			if is_decl(doc, n) {
				continue;
			}
			out.push(match n {
				Node::Elem(e)	=> elem(doc, e),
				_				=> empty_node(),
			});
		}
		if let Some(t) = text.take() {
			out.push(Value::str(t));
		}
		out
	}
	fn elem(doc: &Xml, e: &oxedyne_fe2o3_text::xml::Elem) -> Value {
		let mut d = Dict::new();
		d.insert("namespace", match e.name.ns.and_then(|i| doc.uris().get(i)) {
			Some(u)	=> Value::str(u.as_str()),
			None	=> Value::None,
		});
		d.insert("tag", Value::str(e.name.local()));
		let mut attrs = Dict::new();
		for a in &e.attrs {
			if a.name.qname == "xmlns" || a.name.qname.starts_with("xmlns:") {
				continue;
			}
			attrs.insert(a.name.local(), Value::str(oxedyne_fe2o3_text::xml::write::decode(&a.value)));
		}
		d.insert("attrs", Value::dict(attrs));
		d.insert("children", Value::array(children(doc, &e.kids, false)));
		Value::dict(d)
	}
	Ok(Value::array(children(&doc, &doc.nodes, true)))
}

// CBOR

struct CborReader<'a> {
	b:		&'a [u8],
	at:		usize,
	depth:	usize,
}

impl<'a> CborReader<'a> {
	fn byte(&mut self) -> Outcome<u8> {
		match self.b.get(self.at) {
			Some(x)	=> {
				self.at += 1;
				Ok(*x)
			}
			None	=> Err(err!("unexpected end of input"; Decode, Input)),
		}
	}

	fn take(&mut self, n: usize) -> Outcome<&'a [u8]> {
		match self.b.get(self.at..self.at.saturating_add(n)) {
			Some(s)	=> {
				self.at += n;
				Ok(s)
			}
			None	=> Err(err!("unexpected end of input"; Decode, Input)),
		}
	}

	fn arg(&mut self, info: u8) -> Outcome<Option<u64>> {
		let n = match info {
			0..=23	=> return Ok(Some(info as u64)),
			24		=> 1,
			25		=> 2,
			26		=> 4,
			27		=> 8,
			31		=> return Ok(None),
			_		=> return Err(err!("invalid additional information"; Decode, Input)),
		};
		let s = res!(self.take(n));
		Ok(Some(s.iter().fold(0u64, |a, x| (a << 8) | *x as u64)))
	}

	fn value(&mut self) -> Outcome<Value> {
		if self.depth > 256 {
			return Err(err!("recursion limit exceeded"; Decode, Input));
		}
		let ib = res!(self.byte());
		let (major, info) = (ib >> 5, ib & 0x1f);
		match major {
			0 => {
				let n = res!(res!(self.arg(info)).ok_or_else(|| err!("indefinite integer"; Decode, Input)));
				i64::try_from(n).map(Value::Int).map_err(|_| err!("integer too large"; Decode, Input))
			}
			1 => {
				let n = res!(res!(self.arg(info)).ok_or_else(|| err!("indefinite integer"; Decode, Input)));
				i64::try_from(n).map(|x| Value::Int(-1 - x)).map_err(|_| err!("integer too large"; Decode, Input))
			}
			2 | 3 => {
				let mut buf = Vec::new();
				match res!(self.arg(info)) {
					Some(n)	=> buf.extend_from_slice(res!(self.take(n as usize))),
					None	=> loop {
						if self.b.get(self.at) == Some(&0xff) {
							self.at += 1;
							break;
						}
						let ib2 = res!(self.byte());
						if ib2 >> 5 != major {
							return Err(err!("invalid chunk in indefinite string"; Decode, Input));
						}
						let n = res!(res!(self.arg(ib2 & 0x1f)).ok_or_else(|| err!("nested indefinite string"; Decode, Input)));
						buf.extend_from_slice(res!(self.take(n as usize)));
					},
				}
				if major == 2 {
					Ok(Value::Bytes(Arc::new(buf)))
				} else {
					String::from_utf8(buf).map(Value::str).map_err(|_| err!("invalid utf-8 in text string"; Decode, Input))
				}
			}
			4 => {
				self.depth += 1;
				let mut a = Vec::new();
				match res!(self.arg(info)) {
					Some(n)	=> for _ in 0..n {
						a.push(res!(self.value()));
					},
					None	=> while self.b.get(self.at) != Some(&0xff) {
						a.push(res!(self.value()));
					},
				}
				if info == 31 {
					self.at += 1;
				}
				self.depth -= 1;
				Ok(Value::array(a))
			}
			5 => {
				self.depth += 1;
				let mut d = Dict::new();
				let n = res!(self.arg(info));
				let mut i = 0u64;
				loop {
					match n {
						Some(n) if i >= n => break,
						None if self.b.get(self.at) == Some(&0xff) => {
							self.at += 1;
							break;
						}
						_ => (),
					}
					let k = match res!(self.value()) {
						Value::Str(s)	=> (*s).clone(),
						Value::Int(i)	=> i.to_string(),
						other			=> repr(&other),
					};
					let v = res!(self.value());
					d.insert(&k, v);
					i += 1;
				}
				self.depth -= 1;
				Ok(Value::dict(d))
			}
			6 => {
				res!(self.arg(info));
				self.value()
			}
			_ => match info {
				20	=> Ok(Value::Bool(false)),
				21	=> Ok(Value::Bool(true)),
				22 | 23	=> Ok(Value::None),
				25	=> {
					let s = res!(self.take(2));
					Ok(Value::Float(f16_to_f64(((s[0] as u16) << 8) | s[1] as u16)))
				}
				26	=> {
					let s = res!(self.take(4));
					Ok(Value::Float(f32::from_be_bytes([s[0], s[1], s[2], s[3]]) as f64))
				}
				27	=> {
					let s = res!(self.take(8));
					let mut a = [0u8; 8];
					a.copy_from_slice(s);
					Ok(Value::Float(f64::from_be_bytes(a)))
				}
				_	=> Err(err!("unsupported simple value"; Decode, Input)),
			},
		}
	}
}

fn f16_to_f64(h: u16) -> f64 {
	let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
	let exp = ((h >> 10) & 0x1f) as i32;
	let frac = (h & 0x3ff) as f64;
	let v = match exp {
		0	=> frac * 2f64.powi(-24),
		31	=> if frac == 0.0 { f64::INFINITY } else { f64::NAN },
		e	=> (1.0 + frac / 1024.0) * 2f64.powi(e - 15),
	};
	sign * v
}

// The half-precision bits of `f`, when it converts exactly.
fn f64_to_f16_exact(f: f64) -> Option<u16> {
	if f.is_nan() {
		return Some(0x7e00);
	}
	let sign: u16 = if f.is_sign_negative() { 0x8000 } else { 0 };
	let a = f.abs();
	if a == 0.0 {
		return Some(sign);
	}
	if a.is_infinite() {
		return Some(sign | 0x7c00);
	}
	for exp in 1..31i32 {
		let base = 2f64.powi(exp - 15);
		if a >= base && a < base * 2.0 {
			let frac = (a / base - 1.0) * 1024.0;
			return if frac.fract() == 0.0 { Some(sign | ((exp as u16) << 10) | frac as u16) } else { None };
		}
	}
	let frac = a / 2f64.powi(-24);
	if frac.fract() == 0.0 && frac < 1024.0 { Some(sign | frac as u16) } else { None }
}

fn cbor_head(major: u8, n: u64, out: &mut Vec<u8>) {
	let m = major << 5;
	if n < 24 {
		out.push(m | n as u8);
	} else if n <= 0xff {
		out.push(m | 24);
		out.push(n as u8);
	} else if n <= 0xffff {
		out.push(m | 25);
		out.extend_from_slice(&(n as u16).to_be_bytes());
	} else if n <= 0xffff_ffff {
		out.push(m | 26);
		out.extend_from_slice(&(n as u32).to_be_bytes());
	} else {
		out.push(m | 27);
		out.extend_from_slice(&n.to_be_bytes());
	}
}

fn cbor_write(d: &Data, out: &mut Vec<u8>) {
	match d {
		Data::Null		=> out.push(0xf6),
		Data::Bool(b)	=> out.push(if *b { 0xf5 } else { 0xf4 }),
		Data::Int(i)	=> if *i >= 0 { cbor_head(0, *i as u64, out) } else { cbor_head(1, (-1 - *i) as u64, out) },
		Data::Float(f)	=> {
			if let Some(h) = f64_to_f16_exact(*f) {
				out.push(0xf9);
				out.extend_from_slice(&h.to_be_bytes());
			} else if (*f as f32) as f64 == *f {
				out.push(0xfa);
				out.extend_from_slice(&(*f as f32).to_be_bytes());
			} else {
				out.push(0xfb);
				out.extend_from_slice(&f.to_be_bytes());
			}
		}
		Data::Str(s)	=> {
			cbor_head(3, s.len() as u64, out);
			out.extend_from_slice(s.as_bytes());
		}
		Data::Bytes(b)	=> {
			cbor_head(2, b.len() as u64, out);
			out.extend_from_slice(b);
		}
		Data::Seq(a)	=> {
			cbor_head(4, a.len() as u64, out);
			for x in a {
				cbor_write(x, out);
			}
		}
		Data::Map(m)	=> {
			cbor_head(5, m.len() as u64, out);
			for (k, x) in m {
				cbor_write(&Data::Str(k.clone()), out);
				cbor_write(x, out);
			}
		}
	}
}

// TOML

struct Toml<'a> {
	s:		&'a str,
	b:		&'a [u8],
	at:		usize,
	fail:	Option<(usize, String)>,	// where and why parsing stopped, for the diagnostic
}

// A table under construction: its entries, and which were defined how, for TOML's redefinition rules.
#[derive(Clone, Debug, Default)]
struct TTable {
	entries:	Vec<(String, TVal)>,
	defined:	bool,	// by a `[header]` or as a value
	inline:		bool,	// an inline table, closed to further keys
	dotted:		bool,	// created implicitly by a dotted key
}

#[derive(Clone, Debug)]
enum TVal {
	Val(Value),
	Table(TTable),
	Array(Vec<TVal>, bool),	// the bool: an array of tables (`[[x]]`), still extendable
}

impl TTable {
	fn get_mut(&mut self, k: &str) -> Option<&mut TVal> {
		self.entries.iter_mut().find(|(n, _)| n == k).map(|(_, v)| v)
	}

	fn has(&self, k: &str) -> bool { self.entries.iter().any(|(n, _)| n == k) }

	fn into_value(self) -> Value {
		let mut d = Dict::new();
		for (k, v) in self.entries {
			d.insert(&k, v.into_value());
		}
		Value::dict(d)
	}
}

impl TVal {
	fn into_value(self) -> Value {
		match self {
			TVal::Val(v)		=> v,
			TVal::Table(t)		=> t.into_value(),
			TVal::Array(a, _)	=> Value::array(a.into_iter().map(TVal::into_value).collect()),
		}
	}
}

pub(crate) fn toml_parse(s: &str) -> std::result::Result<Value, (usize, String)> {
	let mut p = Toml { s, b: s.as_bytes(), at: 0, fail: None };
	match toml_document(&mut p) {
		Ok(v)	=> Ok(v),
		Err(_)	=> Err(p.fail.unwrap_or((0, "invalid TOML".to_string()))),
	}
}

fn toml_document(p: &mut Toml) -> Outcome<Value> {
	let mut root = TTable { defined: true, ..TTable::default() };
	let mut current: Vec<String> = Vec::new();
	loop {
		p.skip_ws_comments_newlines();
		if p.at >= p.b.len() {
			break;
		}
		if p.b[p.at] == b'[' {
			let aot = p.b.get(p.at + 1) == Some(&b'[');
			p.at += if aot { 2 } else { 1 };
			p.skip_ws();
			let key = res!(p.key());
			p.skip_ws();
			if aot {
				if !p.s[p.at..].starts_with("]]") {
					return p.err("expected `]]`");
				}
				p.at += 2;
			} else {
				if p.b.get(p.at) != Some(&b']') {
					return p.err("expected `]`");
				}
				p.at += 1;
			}
			res!(p.end_of_line());
			let at = p.at;
			if let Err(m) = toml_open_table(&mut root, &key, aot) {
				return p.fail_at(at, &m);
			}
			current = key;
		} else {
			let key = res!(p.key());
			p.skip_ws();
			if p.b.get(p.at) != Some(&b'=') {
				return p.err("expected `=`");
			}
			p.at += 1;
			p.skip_ws();
			let at = p.at;
			let v = res!(p.value());
			res!(p.end_of_line());
			let table = match toml_current(&mut root, &current) {
				Some(t)	=> t,
				None	=> return p.fail_at(at, "invalid table"),
			};
			if let Err(m) = toml_insert(table, &key, v) {
				return p.fail_at(at, &m);
			}
		}
	}
	Ok(root.into_value())
}

// The table a path names, descending through the last element of arrays of tables.
fn toml_current<'t>(root: &'t mut TTable, path: &[String]) -> Option<&'t mut TTable> {
	let mut t = root;
	for k in path {
		t = match t.get_mut(k) {
			Some(TVal::Table(x))		=> x,
			Some(TVal::Array(a, _))	=> match a.last_mut() {
				Some(TVal::Table(x))	=> x,
				_						=> return None,
			},
			_						=> return None,
		};
	}
	Some(t)
}

fn toml_open_table(root: &mut TTable, key: &[String], aot: bool) -> std::result::Result<(), String> {
	let mut t = root;
	let (last, head) = match key.split_last() {
		Some(x)	=> x,
		None	=> return Err("empty table key".to_string()),
	};
	for k in head {
		if !t.has(k) {
			t.entries.push((k.clone(), TVal::Table(TTable::default())));
		}
		t = match t.get_mut(k) {
			Some(TVal::Table(x)) if !x.inline	=> x,
			Some(TVal::Array(a, true))			=> match a.last_mut() {
				Some(TVal::Table(x))	=> x,
				_						=> return Err(fmt!("duplicate key `{}`", k)),
			},
			_ => return Err(fmt!("duplicate key `{}`", k)),
		};
	}
	if aot {
		match t.get_mut(last) {
			None => t.entries.push((last.clone(),
				TVal::Array(vec![TVal::Table(TTable { defined: true, ..TTable::default() })], true))),
			Some(TVal::Array(a, true)) => a.push(TVal::Table(TTable { defined: true, ..TTable::default() })),
			_ => return Err(fmt!("duplicate key `{}`", last)),
		}
	} else {
		match t.get_mut(last) {
			None => t.entries.push((last.clone(), TVal::Table(TTable { defined: true, ..TTable::default() }))),
			Some(TVal::Table(x)) if !x.defined && !x.inline && !x.dotted => x.defined = true,
			_ => return Err(fmt!("duplicate key `{}`", last)),
		}
	}
	Ok(())
}

fn toml_insert(t: &mut TTable, key: &[String], v: TVal) -> std::result::Result<(), String> {
	let (last, head) = match key.split_last() {
		Some(x)	=> x,
		None	=> return Err("empty key".to_string()),
	};
	let mut t = t;
	for k in head {
		if !t.has(k) {
			t.entries.push((k.clone(), TVal::Table(TTable { dotted: true, ..TTable::default() })));
		}
		t = match t.get_mut(k) {
			Some(TVal::Table(x)) if !x.inline && (x.dotted || !x.defined) => x,
			_ => return Err(fmt!("duplicate key `{}`", k)),
		};
	}
	if t.has(last) {
		return Err(fmt!("duplicate key `{}`", last));
	}
	t.entries.push((last.clone(), v));
	Ok(())
}

impl<'a> Toml<'a> {
	fn err<T>(&mut self, m: &str) -> Outcome<T> {
		let at = self.at;
		self.fail_at(at, m)
	}

	fn fail_at<T>(&mut self, at: usize, m: &str) -> Outcome<T> {
		self.fail = Some((at, m.to_string()));
		Err(err!("{}", m; Decode, Input))
	}

	fn skip_ws(&mut self) {
		while self.at < self.b.len() && matches!(self.b[self.at], b' ' | b'\t') {
			self.at += 1;
		}
	}

	fn skip_comment(&mut self) {
		if self.b.get(self.at) == Some(&b'#') {
			while self.at < self.b.len() && self.b[self.at] != b'\n' {
				self.at += 1;
			}
		}
	}

	fn skip_ws_comments_newlines(&mut self) {
		loop {
			self.skip_ws();
			self.skip_comment();
			match self.b.get(self.at) {
				Some(b'\n')					=> self.at += 1,
				Some(b'\r') if self.b.get(self.at + 1) == Some(&b'\n') => self.at += 2,
				_							=> break,
			}
		}
	}

	fn end_of_line(&mut self) -> Outcome<()> {
		self.skip_ws();
		self.skip_comment();
		match self.b.get(self.at) {
			None | Some(b'\n')	=> Ok(()),
			Some(b'\r') if self.b.get(self.at + 1) == Some(&b'\n') => Ok(()),
			_					=> self.err("expected newline"),
		}
	}

	fn key(&mut self) -> Outcome<Vec<String>> {
		let mut parts = Vec::new();
		loop {
			self.skip_ws();
			let part = match self.b.get(self.at) {
				Some(b'"')	=> res!(self.basic_string()),
				Some(b'\'')	=> res!(self.literal_string()),
				_			=> {
					let start = self.at;
					while self.at < self.b.len() && (self.b[self.at].is_ascii_alphanumeric() || matches!(self.b[self.at], b'_' | b'-')) {
						self.at += 1;
					}
					if self.at == start {
						return self.err("invalid key");
					}
					self.s[start..self.at].to_string()
				}
			};
			parts.push(part);
			self.skip_ws();
			if self.b.get(self.at) == Some(&b'.') {
				self.at += 1;
				continue;
			}
			return Ok(parts);
		}
	}

	fn value(&mut self) -> Outcome<TVal> {
		match self.b.get(self.at) {
			None		=> self.err("expected a value"),
			Some(b'"')	=> {
				if self.s[self.at..].starts_with("\"\"\"") {
					self.ml_basic_string().map(|s| TVal::Val(Value::str(s)))
				} else {
					self.basic_string().map(|s| TVal::Val(Value::str(s)))
				}
			}
			Some(b'\'')	=> {
				if self.s[self.at..].starts_with("'''") {
					self.ml_literal_string().map(|s| TVal::Val(Value::str(s)))
				} else {
					self.literal_string().map(|s| TVal::Val(Value::str(s)))
				}
			}
			Some(b'[')	=> {
				self.at += 1;
				let mut a = Vec::new();
				loop {
					self.skip_ws_comments_newlines();
					if self.b.get(self.at) == Some(&b']') {
						self.at += 1;
						break;
					}
					a.push(res!(self.value()));
					self.skip_ws_comments_newlines();
					match self.b.get(self.at) {
						Some(b',')	=> self.at += 1,
						Some(b']')	=> {
							self.at += 1;
							break;
						}
						_			=> return self.err("expected `,` or `]`"),
					}
				}
				Ok(TVal::Array(a, false))
			}
			Some(b'{')	=> {
				self.at += 1;
				let mut t = TTable::default();
				self.skip_ws();
				if self.b.get(self.at) == Some(&b'}') {
					self.at += 1;
				} else {
					loop {
						self.skip_ws();
						let k = res!(self.key());
						self.skip_ws();
						if self.b.get(self.at) != Some(&b'=') {
							return self.err("expected `=`");
						}
						self.at += 1;
						self.skip_ws();
						let at = self.at;
						let v = res!(self.value());
						if let Err(m) = toml_insert(&mut t, &k, v) {
							return self.fail_at(at, &m);
						}
						self.skip_ws();
						match self.b.get(self.at) {
							Some(b',')	=> self.at += 1,
							Some(b'}')	=> {
								self.at += 1;
								break;
							}
							_			=> return self.err("expected `,` or `}`"),
						}
					}
				}
				t.inline = true;
				t.defined = true;
				Ok(TVal::Table(t))
			}
			Some(b't') if self.s[self.at..].starts_with("true") => {
				self.at += 4;
				Ok(TVal::Val(Value::Bool(true)))
			}
			Some(b'f') if self.s[self.at..].starts_with("false") => {
				self.at += 5;
				Ok(TVal::Val(Value::Bool(false)))
			}
			Some(_)		=> self.scalar(),
		}
	}

	// Numbers and dates: a bare run of characters, classified.
	fn scalar(&mut self) -> Outcome<TVal> {
		let start = self.at;
		while self.at < self.b.len() {
			let c = self.b[self.at];
			let date_space = c == b' ' && self.at - start == 10
				&& self.b.get(self.at + 1).map(|d| d.is_ascii_digit()).unwrap_or(false)
				&& self.s[start..self.at].bytes().filter(|x| *x == b'-').count() == 2;
			if c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'_' | b'.' | b':') || date_space {
				self.at += 1;
			} else {
				break;
			}
		}
		let t = &self.s[start..self.at];
		if t.is_empty() {
			return self.err("expected a value");
		}
		if let Some(dt) = toml_datetime(t) {
			return Ok(TVal::Val(Value::Datetime(dt)));
		}
		match toml_number(t) {
			Some(v)	=> Ok(TVal::Val(v)),
			None	=> self.fail_at(start, "invalid number"),
		}
	}

	fn escape(&mut self, out: &mut String) -> Outcome<()> {
		let c = match self.b.get(self.at) {
			Some(c)	=> *c,
			None	=> return self.err("unterminated string"),
		};
		self.at += 1;
		match c {
			b'b'	=> out.push('\u{8}'),
			b't'	=> out.push('\t'),
			b'n'	=> out.push('\n'),
			b'f'	=> out.push('\u{c}'),
			b'r'	=> out.push('\r'),
			b'e'	=> out.push('\u{1b}'),
			b'"'	=> out.push('"'),
			b'\\'	=> out.push('\\'),
			b'u' | b'U' => {
				let n = if c == b'u' { 4 } else { 8 };
				let cp = self.s.get(self.at..self.at + n).and_then(|h| u32::from_str_radix(h, 16).ok())
					.and_then(char::from_u32);
				match cp {
					Some(ch) => {
						out.push(ch);
						self.at += n;
					}
					None => return self.err("invalid unicode escape"),
				}
			}
			_ => return self.err("invalid escape"),
		}
		Ok(())
	}

	fn basic_string(&mut self) -> Outcome<String> {
		self.at += 1;
		let mut out = String::new();
		loop {
			let rest = &self.s[self.at..];
			let ch = match rest.chars().next() {
				Some(c)	=> c,
				None	=> return self.err("unterminated string"),
			};
			match ch {
				'"'		=> {
					self.at += 1;
					return Ok(out);
				}
				'\\'	=> {
					self.at += 1;
					res!(self.escape(&mut out));
				}
				'\n'	=> return self.err("newline in string"),
				c		=> {
					out.push(c);
					self.at += c.len_utf8();
				}
			}
		}
	}

	fn ml_basic_string(&mut self) -> Outcome<String> {
		self.at += 3;
		if self.s[self.at..].starts_with("\r\n") {
			self.at += 2;
		} else if self.s[self.at..].starts_with('\n') {
			self.at += 1;
		}
		let mut out = String::new();
		loop {
			if self.s[self.at..].starts_with("\"\"\"") {
				// Up to two quotes may close into the content: `""""` ends with one quote inside.
				let mut n = 3;
				while self.b.get(self.at + n) == Some(&b'"') && n < 5 {
					n += 1;
				}
				out.push_str(&"\"".repeat(n - 3));
				self.at += n;
				return Ok(out);
			}
			let ch = match self.s[self.at..].chars().next() {
				Some(c)	=> c,
				None	=> return self.err("unterminated string"),
			};
			if ch == '\\' {
				self.at += 1;
				// A line-ending backslash trims the newline and following whitespace.
				let save = self.at;
				let mut j = self.at;
				while j < self.b.len() && matches!(self.b[j], b' ' | b'\t') {
					j += 1;
				}
				if matches!(self.b.get(j), Some(b'\n') | Some(b'\r')) {
					while j < self.b.len() && matches!(self.b[j], b' ' | b'\t' | b'\n' | b'\r') {
						j += 1;
					}
					self.at = j;
				} else {
					self.at = save;
					res!(self.escape(&mut out));
				}
			} else {
				out.push(ch);
				self.at += ch.len_utf8();
			}
		}
	}

	fn literal_string(&mut self) -> Outcome<String> {
		self.at += 1;
		match self.s[self.at..].find(['\'', '\n']) {
			Some(i) if self.b[self.at + i] == b'\'' => {
				let out = self.s[self.at..self.at + i].to_string();
				self.at += i + 1;
				Ok(out)
			}
			_ => self.err("unterminated string"),
		}
	}

	fn ml_literal_string(&mut self) -> Outcome<String> {
		self.at += 3;
		if self.s[self.at..].starts_with("\r\n") {
			self.at += 2;
		} else if self.s[self.at..].starts_with('\n') {
			self.at += 1;
		}
		match self.s[self.at..].find("'''") {
			Some(i) => {
				let mut end = self.at + i;
				let mut n = 3;
				while self.b.get(end + n) == Some(&b'\'') && n < 5 {
					n += 1;
				}
				end += n - 3;
				let out = self.s[self.at..end].to_string();
				self.at = end + 3;
				Ok(out)
			}
			None => self.err("unterminated string"),
		}
	}
}

fn toml_number(t: &str) -> Option<Value> {
	let (sign, body) = match t.as_bytes().first() {
		Some(b'+')	=> (1i64, &t[1..]),
		Some(b'-')	=> (-1i64, &t[1..]),
		_			=> (1i64, t),
	};
	match body {
		"inf"	=> return Some(Value::Float(sign as f64 * f64::INFINITY)),
		"nan"	=> return Some(Value::Float(f64::NAN)),
		_		=> (),
	}
	let underscores_ok = |s: &str| !s.starts_with('_') && !s.ends_with('_') && !s.contains("__");
	for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
		if let Some(d) = body.strip_prefix(prefix) {
			if sign != 1 || t.starts_with('+') || !underscores_ok(d) {
				return None;
			}
			return i64::from_str_radix(&d.replace('_', ""), radix).ok().map(Value::Int);
		}
	}
	if !underscores_ok(body) || body.is_empty() {
		return None;
	}
	let clean = body.replace('_', "");
	if clean.bytes().all(|b| b.is_ascii_digit()) {
		if clean.len() > 1 && clean.starts_with('0') {
			return None;
		}
		return clean.parse::<i64>().ok().map(|v| Value::Int(sign * v)).or_else(|| {
			if sign < 0 { clean.parse::<i128>().ok().filter(|v| -*v == i64::MIN as i128).map(|_| Value::Int(i64::MIN)) } else { None }
		});
	}
	let ok = clean.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
		&& !clean.starts_with('.') && !clean.ends_with('.') && !clean.contains(".e") && !clean.contains(".E");
	if !ok {
		return None;
	}
	clean.parse::<f64>().ok().map(|v| Value::Float(sign as f64 * v))
}

// Fixed-width decimal digits, or `None`.
fn digits_n(s: Option<&str>, n: usize) -> Option<u32> {
	match s {
		Some(s) if s.len() == n && s.bytes().all(|b| b.is_ascii_digit()) => s.parse().ok(),
		_ => None,
	}
}

// `hh:mm:ss`, a fraction and an offset dropped, as Typst keeps whole seconds.
fn toml_time(s: &str) -> Option<(u8, u8, u8)> {
	let s = s.split(['Z', 'z', '+']).next().unwrap_or("");
	let s = if s.len() > 8 && s.as_bytes()[8] == b'-' { &s[..8] } else { s };
	let s = s.split('.').next().unwrap_or("");
	let mut p = s.split(':');
	match (digits_n(p.next(), 2), digits_n(p.next(), 2), digits_n(p.next(), 2)) {
		(Some(h), Some(m), Some(sec)) if h <= 23 && m <= 59 && sec <= 60 => Some((h as u8, m as u8, sec.min(59) as u8)),
		_ => None,
	}
}

fn toml_date(s: &str) -> Option<(i32, u8, u8)> {
	let mut p = s.split('-');
	match (digits_n(p.next(), 4), digits_n(p.next(), 2), digits_n(p.next(), 2), p.next()) {
		(Some(y), Some(m), Some(d), None) if (1..=12).contains(&m) && (1..=31).contains(&d) => Some((y as i32, m as u8, d as u8)),
		_ => None,
	}
}

fn toml_datetime(t: &str) -> Option<Datetime> {
	if t.len() >= 10 && t.as_bytes().get(4) == Some(&b'-') {
		let (y, m, d) = match toml_date(&t[..10]) {
			Some(x)	=> x,
			None	=> return None,
		};
		let mut dt = Datetime { year: Some(y), month: Some(m), day: Some(d), ..Datetime::default() };
		if t.len() > 10 {
			if !matches!(t.as_bytes()[10], b'T' | b't' | b' ') {
				return None;
			}
			let (h, mi, s) = match toml_time(&t[11..]) {
				Some(x)	=> x,
				None	=> return None,
			};
			dt.hour = Some(h);
			dt.minute = Some(mi);
			dt.second = Some(s);
		}
		return Some(dt);
	}
	if t.len() >= 8 && t.as_bytes().get(2) == Some(&b':') {
		let (h, mi, s) = match toml_time(t) {
			Some(x)	=> x,
			None	=> return None,
		};
		return Some(Datetime { hour: Some(h), minute: Some(mi), second: Some(s), ..Datetime::default() });
	}
	None
}

fn toml_key(k: &str) -> String {
	if !k.is_empty() && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
		k.to_string()
	} else {
		toml_str(k)
	}
}

// A TOML string: literal quotes when the text has a double quote and no single quote or control
// character, as the `toml` crate prefers; basic quotes otherwise.
fn toml_str(s: &str) -> String {
	if s.contains('"') && !s.contains('\'') && !s.chars().any(|c| c.is_control()) {
		return fmt!("'{}'", s);
	}
	let mut out = String::from("\"");
	for c in s.chars() {
		match c {
			'"'		=> out.push_str("\\\""),
			'\\'	=> out.push_str("\\\\"),
			'\n'	=> out.push_str("\\n"),
			'\r'	=> out.push_str("\\r"),
			'\t'	=> out.push_str("\\t"),
			'\u{8}'	=> out.push_str("\\b"),
			'\u{c}'	=> out.push_str("\\f"),
			c if c.is_control() => out.push_str(&fmt!("\\u{:04X}", c as u32)),
			c		=> out.push(c),
		}
	}
	out.push('"');
	out
}

fn toml_inline(d: &Data, pretty: bool, out: &mut String) {
	match d {
		Data::Null		=> (),
		Data::Bool(b)	=> out.push_str(if *b { "true" } else { "false" }),
		Data::Int(i)	=> out.push_str(&i.to_string()),
		Data::Float(f)	=> out.push_str(&if f.is_nan() {
			"nan".to_string()
		} else if f.is_infinite() {
			if *f < 0.0 { "-inf".to_string() } else { "inf".to_string() }
		} else {
			ryu_format(*f)
		}),
		Data::Str(s)	=> out.push_str(&toml_str(s)),
		Data::Bytes(b)	=> toml_inline(&Data::Seq(b.iter().map(|x| Data::Int(*x as i64)).collect()), pretty, out),
		Data::Seq(a)	=> {
			if pretty && !a.is_empty() {
				out.push_str("[\n");
				for x in a {
					out.push_str("    ");
					toml_inline(x, false, out);
					out.push_str(",\n");
				}
				out.push(']');
			} else {
				out.push('[');
				for (i, x) in a.iter().enumerate() {
					if i > 0 {
						out.push_str(", ");
					}
					toml_inline(x, false, out);
				}
				out.push(']');
			}
		}
		Data::Map(m)	=> {
			out.push_str("{ ");
			for (i, (k, x)) in m.iter().filter(|(_, x)| !matches!(x, Data::Null)).enumerate() {
				if i > 0 {
					out.push_str(", ");
				}
				out.push_str(&toml_key(k));
				out.push_str(" = ");
				toml_inline(x, false, out);
			}
			out.push_str(" }");
		}
	}
}

fn is_table_array(d: &Data) -> bool {
	matches!(d, Data::Seq(a) if !a.is_empty() && a.iter().all(|x| matches!(x, Data::Map(_))))
}

fn toml_write_table(m: &[(String, Data)], path: &[String], pretty: bool, out: &mut String) {
	// Plain values first, then subtables and arrays of tables, as the `toml` crate orders them.
	for (k, v) in m {
		if matches!(v, Data::Null | Data::Map(_)) || is_table_array(v) {
			continue;
		}
		out.push_str(&toml_key(k));
		out.push_str(" = ");
		toml_inline(v, pretty, out);
		out.push('\n');
	}
	for (k, v) in m {
		let mut p = path.to_vec();
		p.push(k.clone());
		let header = p.iter().map(|x| toml_key(x)).collect::<Vec<_>>().join(".");
		match v {
			Data::Map(sub) => {
				let has_plain = sub.iter().any(|(_, x)| !matches!(x, Data::Null | Data::Map(_)) && !is_table_array(x));
				if has_plain || sub.is_empty() {
					if !out.is_empty() {
						out.push('\n');
					}
					out.push_str(&fmt!("[{}]\n", header));
				}
				toml_write_table(sub, &p, pretty, out);
			}
			Data::Seq(a) if is_table_array(v) => for x in a {
				if let Data::Map(sub) = x {
					if !out.is_empty() {
						out.push('\n');
					}
					out.push_str(&fmt!("[[{}]]\n", header));
					toml_write_table(sub, &p, pretty, out);
				}
			},
			_ => (),
		}
	}
}

// YAML

#[derive(Clone, Debug)]
struct YLine<'a> {
	indent:	usize,
	text:	&'a str,	// without indentation; comments stripped outside quotes
	at:		usize,		// byte offset of the text in the source
}

struct Yaml<'a> {
	lines:		Vec<YLine<'a>>,
	i:			usize,
	anchors:	Vec<(String, Value)>,
	src:		&'a str,
	fail:		Option<(usize, String)>,	// where and why parsing stopped, for the diagnostic
}

fn strip_comment(t: &str) -> &str {
	let b = t.as_bytes();
	let (mut sq, mut dq) = (false, false);
	for i in 0..b.len() {
		match b[i] {
			b'\'' if !dq	=> sq = !sq,
			b'"' if !sq		=> dq = !dq,
			b'#' if !sq && !dq && (i == 0 || b[i - 1] == b' ' || b[i - 1] == b'\t') => return t[..i].trim_end(),
			_ => (),
		}
	}
	t.trim_end()
}

fn yaml_parse(src: &str) -> std::result::Result<Value, (usize, String)> {
	let mut y = Yaml { lines: Vec::new(), i: 0, anchors: Vec::new(), src, fail: None };
	match yaml_document(&mut y) {
		Ok(v)	=> Ok(v),
		Err(_)	=> Err(y.fail.unwrap_or((0, "invalid YAML".to_string()))),
	}
}

fn yaml_document(y: &mut Yaml) -> Outcome<Value> {
	let src = y.src;
	let mut lines = Vec::new();
	let mut off = 0;
	for raw in src.split('\n') {
		let line = raw.strip_suffix('\r').unwrap_or(raw);
		let indent = line.len() - line.trim_start_matches(' ').len();
		lines.push(YLine { indent, text: &line[indent..], at: off + indent });
		off += raw.len() + 1;
	}
	y.lines = lines;
	y.skip_blank();
	if let Some(l) = y.lines.get(y.i) {
		if l.text.starts_with("%") {
			let at = l.at;
			return y.fail_at(at, "directives are not supported");
		}
		if l.text == "---" || l.text.starts_with("--- ") {
			let rest = strip_comment(l.text[3..].trim_start());
			if rest.is_empty() {
				y.i += 1;
			} else {
				let at = l.at + (l.text.len() - l.text[3..].trim_start().len());
				let v = res!(y.inline_value(rest, at));
				return Ok(v);
			}
		}
	}
	y.skip_blank();
	if y.i >= y.lines.len() {
		return Ok(Value::None);
	}
	let ind = y.lines[y.i].indent;
	let v = res!(y.block(ind));
	y.skip_blank();
	if let Some(l) = y.lines.get(y.i) {
		if l.text != "..." && l.text != "---" {
			let at = l.at;
			return y.fail_at(at, "did not find expected end of document");
		}
	}
	Ok(v)
}

impl<'a> Yaml<'a> {
	fn fail_at<T>(&mut self, at: usize, m: &str) -> Outcome<T> {
		self.fail = Some((at, m.to_string()));
		Err(err!("{}", m; Decode, Input))
	}

	fn skip_blank(&mut self) {
		while let Some(l) = self.lines.get(self.i) {
			if strip_comment(l.text).is_empty() {
				self.i += 1;
			} else {
				break;
			}
		}
	}

	// A block node whose lines are indented by `ind`.
	fn block(&mut self, ind: usize) -> Outcome<Value> {
		self.skip_blank();
		let l = match self.lines.get(self.i) {
			Some(l)	=> l.clone(),
			None	=> return Ok(Value::None),
		};
		let t = strip_comment(l.text);
		if t == "-" || t.starts_with("- ") {
			return self.sequence(ind);
		}
		if yaml_key_split(t).is_some() {
			return self.mapping(ind);
		}
		// A scalar spread over lines, or a flow collection.
		self.i += 1;
		let mut text = t.to_string();
		if t.starts_with('[') || t.starts_with('{') || t.starts_with('"') || t.starts_with('\'') {
			while !yaml_balanced(&text) && self.i < self.lines.len() {
				text.push(' ');
				text.push_str(strip_comment(self.lines[self.i].text).trim());
				self.i += 1;
			}
		} else {
			while let Some(n) = self.lines.get(self.i) {
				let nt = strip_comment(n.text);
				if nt.is_empty() || n.indent < ind || (n.indent == ind && (nt.starts_with("- ") || yaml_key_split(nt).is_some())) {
					break;
				}
				text.push(' ');
				text.push_str(nt.trim());
				self.i += 1;
			}
		}
		self.inline_value(&text, l.at)
	}

	fn sequence(&mut self, ind: usize) -> Outcome<Value> {
		let mut out = Vec::new();
		loop {
			self.skip_blank();
			let l = match self.lines.get(self.i) {
				Some(l) if l.indent == ind => l.clone(),
				_ => break,
			};
			let t = strip_comment(l.text);
			if !(t == "-" || t.starts_with("- ")) {
				break;
			}
			let rest = t[1..].trim_start();
			let rest_at = l.at + (t.len() - rest.len());
			if rest.is_empty() {
				self.i += 1;
				self.skip_blank();
				match self.lines.get(self.i) {
					Some(n) if n.indent > ind => {
						let ni = n.indent;
						out.push(res!(self.block(ni)));
					}
					_ => out.push(Value::None),
				}
			} else {
				// The item's content starts on this line; treat it as a line indented to its column.
				let col = ind + (t.len() - rest.len());
				self.lines[self.i] = YLine { indent: col, text: rest, at: rest_at };
				out.push(res!(self.item(col, ind)));
			}
		}
		Ok(Value::array(out))
	}

	// A node starting at the current line, which sits at column `col`.
	// A sequence item's content starting on the dash's line, at column `col`; a block scalar there is
	// indented relative to the sequence's own indentation `parent`, not to the item's column.
	fn item(&mut self, col: usize, parent: usize) -> Outcome<Value> {
		let l = match self.lines.get(self.i) {
			Some(l)	=> l.clone(),
			None	=> return Ok(Value::None),
		};
		let t = strip_comment(l.text);
		if let Some(v) = res!(self.block_scalar(t, parent)) {
			return Ok(v);
		}
		self.block(col)
	}

	fn mapping(&mut self, ind: usize) -> Outcome<Value> {
		let mut d = Dict::new();
		loop {
			self.skip_blank();
			let l = match self.lines.get(self.i) {
				Some(l) if l.indent == ind => l.clone(),
				_ => break,
			};
			let t = strip_comment(l.text);
			let (k, rest) = match yaml_key_split(t) {
				Some(x)	=> x,
				None	=> break,
			};
			let key = match res!(self.inline_value(k, l.at)) {
				Value::Str(s)	=> (*s).clone(),
				Value::None		=> "null".to_string(),
				other			=> match crate::eval::lib::foundations::display(&other) {
					Some(s)	=> s.replace('\u{2212}', "-"),
					None	=> repr(&other),
				},
			};
			let rest = rest.trim_start();
			let rest_at = l.at + (t.len() - rest.len());
			let (anchor, rest) = yaml_anchor(rest);
			let v = if rest.is_empty() {
				self.i += 1;
				self.skip_blank();
				match self.lines.get(self.i) {
					Some(n) if n.indent > ind => {
						let ni = n.indent;
						res!(self.block(ni))
					}
					Some(n) if n.indent == ind && (strip_comment(n.text).starts_with("- ") || strip_comment(n.text) == "-") => {
						res!(self.sequence(ind))
					}
					_ => Value::None,
				}
			} else if let Some(v) = res!(self.block_scalar(rest, ind)) {
				v
			} else {
				self.i += 1;
				let mut text = rest.to_string();
				if rest.starts_with('[') || rest.starts_with('{') || rest.starts_with('"') || rest.starts_with('\'') {
					while !yaml_balanced(&text) && self.i < self.lines.len() {
						text.push(' ');
						text.push_str(strip_comment(self.lines[self.i].text).trim());
						self.i += 1;
					}
				} else {
					while let Some(n) = self.lines.get(self.i) {
						let nt = strip_comment(n.text);
						if nt.is_empty() || n.indent <= ind {
							break;
						}
						text.push(' ');
						text.push_str(nt.trim());
						self.i += 1;
					}
				}
				res!(self.inline_value(&text, rest_at))
			};
			if let Some(a) = anchor {
				self.anchors.push((a.to_string(), v.clone()));
			}
			d.insert(&key, v);
		}
		Ok(Value::dict(d))
	}

	// `|` and `>` block scalars, with `-`/`+` chomping.
	fn block_scalar(&mut self, t: &str, ind: usize) -> Outcome<Option<Value>> {
		let head = t.trim();
		let (style, chomp) = match head {
			"|"		=> ('|', ' '),
			"|-"	=> ('|', '-'),
			"|+"	=> ('|', '+'),
			">"		=> ('>', ' '),
			">-"	=> ('>', '-'),
			">+"	=> ('>', '+'),
			_		=> return Ok(None),
		};
		self.i += 1;
		let mut body: Vec<&str> = Vec::new();
		let mut block_ind: Option<usize> = None;
		while let Some(n) = self.lines.get(self.i) {
			if n.text.is_empty() {
				body.push("");
				self.i += 1;
				continue;
			}
			let bi = *block_ind.get_or_insert(n.indent);
			if n.indent < bi || n.indent <= ind {
				break;
			}
			// Keep the extra indentation of a more indented line.
			let line_start = n.at - n.indent;
			let full = &self.src[line_start..line_start + n.indent + n.text.len()];
			body.push(&full[bi.min(full.len())..]);
			self.i += 1;
		}
		let mut trailing = 0;
		while body.last() == Some(&"") {
			body.pop();
			trailing += 1;
		}
		let mut text = if style == '|' {
			body.join("\n")
		} else {
			// Folding: a single break between text lines becomes a space, a run of blank lines keeps
			// one break per blank line, and more-indented lines keep their breaks.
			let mut s = String::new();
			let mut blanks = 0usize;
			let mut prev: Option<&str> = None;
			for line in body.iter() {
				if line.is_empty() {
					blanks += 1;
					continue;
				}
				if let Some(p) = prev {
					if blanks > 0 {
						s.push_str(&"\n".repeat(blanks));
					} else if line.starts_with(' ') || p.starts_with(' ') {
						s.push('\n');
					} else {
						s.push(' ');
					}
				} else {
					s.push_str(&"\n".repeat(blanks));
				}
				blanks = 0;
				s.push_str(line);
				prev = Some(line);
			}
			s
		};
		match chomp {
			'-'	=> (),
			'+'	=> {
				text.push('\n');
				text.push_str(&"\n".repeat(trailing));
			}
			_	=> if !body.is_empty() {
				text.push('\n');
			},
		}
		Ok(Some(Value::str(text)))
	}

	// A scalar, a flow collection, or an alias written on one line.
	fn inline_value(&mut self, t: &str, at: usize) -> Outcome<Value> {
		let mut p = YFlow { s: t, b: t.as_bytes(), i: 0, at, fail: None };
		let v = match p.value(self) {
			Ok(v)	=> v,
			Err(e)	=> {
				if self.fail.is_none() {
					self.fail = p.fail.take();
				}
				return Err(e);
			}
		};
		p.ws();
		if p.i < p.b.len() {
			return self.fail_at(at + p.i, "did not find expected end of flow value");
		}
		Ok(v)
	}
}

fn yaml_anchor(rest: &str) -> (Option<&str>, &str) {
	if let Some(r) = rest.strip_prefix('&') {
		let end = r.find(' ').unwrap_or(r.len());
		return (Some(&r[..end]), r[end..].trim_start());
	}
	(None, rest)
}

fn yaml_balanced(t: &str) -> bool {
	let (mut depth, mut sq, mut dq, mut esc) = (0i32, false, false, false);
	for c in t.chars() {
		if esc {
			esc = false;
			continue;
		}
		match c {
			'\\' if dq			=> esc = true,
			'"' if !sq			=> dq = !dq,
			'\'' if !dq			=> sq = !sq,
			'[' | '{' if !sq && !dq	=> depth += 1,
			']' | '}' if !sq && !dq	=> depth -= 1,
			_ => (),
		}
	}
	depth <= 0 && !sq && !dq
}

// `key: value` on a line, outside quotes and brackets.
fn yaml_key_split(t: &str) -> Option<(&str, &str)> {
	let b = t.as_bytes();
	let (mut sq, mut dq, mut depth) = (false, false, 0i32);
	if t.starts_with('[') || t.starts_with('{') || t.starts_with("- ") {
		return None;
	}
	for i in 0..b.len() {
		match b[i] {
			b'\'' if !dq	=> sq = !sq,
			b'"' if !sq		=> dq = !dq,
			b'[' | b'{' if !sq && !dq	=> depth += 1,
			b']' | b'}' if !sq && !dq	=> depth -= 1,
			b':' if !sq && !dq && depth == 0 && (i + 1 == b.len() || b[i + 1] == b' ' || b[i + 1] == b'\t') => {
				return Some((t[..i].trim_end(), &t[i + 1..]));
			}
			_ => (),
		}
	}
	None
}

struct YFlow<'a> {
	s:		&'a str,
	b:		&'a [u8],
	i:		usize,
	at:		usize,
	fail:	Option<(usize, String)>,
}

impl<'a> YFlow<'a> {
	fn ws(&mut self) {
		while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t') {
			self.i += 1;
		}
	}

	fn err<T>(&mut self, m: &str) -> Outcome<T> {
		self.fail = Some((self.at + self.i, m.to_string()));
		Err(err!("{}", m; Decode, Input))
	}

	fn value(&mut self, y: &mut Yaml) -> Outcome<Value> {
		self.ws();
		let (anchor, _) = if self.b.get(self.i) == Some(&b'&') {
			let r = &self.s[self.i + 1..];
			let end = r.find([' ', ',', ']', '}']).unwrap_or(r.len());
			let a = r[..end].to_string();
			self.i += 1 + end;
			self.ws();
			(Some(a), ())
		} else {
			(None, ())
		};
		let v = match self.b.get(self.i) {
			None		=> Value::None,
			Some(b'*')	=> {
				let r = &self.s[self.i + 1..];
				let end = r.find([' ', ',', ']', '}']).unwrap_or(r.len());
				let name = &r[..end];
				self.i += 1 + end;
				match y.anchors.iter().rev().find(|(n, _)| n == name) {
					Some((_, v))	=> v.clone(),
					None			=> return self.err("unknown anchor"),
				}
			}
			Some(b'!')	=> return self.err("tags are not supported"),
			Some(b'[')	=> {
				self.i += 1;
				let mut a = Vec::new();
				loop {
					self.ws();
					if self.b.get(self.i) == Some(&b']') {
						self.i += 1;
						break;
					}
					a.push(res!(self.value(y)));
					self.ws();
					match self.b.get(self.i) {
						Some(b',')	=> self.i += 1,
						Some(b']')	=> {
							self.i += 1;
							break;
						}
						_			=> return self.err("did not find expected ',' or ']'"),
					}
				}
				Value::array(a)
			}
			Some(b'{')	=> {
				self.i += 1;
				let mut d = Dict::new();
				loop {
					self.ws();
					if self.b.get(self.i) == Some(&b'}') {
						self.i += 1;
						break;
					}
					let k = match res!(self.scalar(true)) {
						Value::Str(s)	=> (*s).clone(),
						other			=> crate::eval::lib::foundations::display(&other).unwrap_or_default(),
					};
					self.ws();
					let v = if self.b.get(self.i) == Some(&b':') {
						self.i += 1;
						res!(self.value(y))
					} else {
						Value::None
					};
					d.insert(&k, v);
					self.ws();
					match self.b.get(self.i) {
						Some(b',')	=> self.i += 1,
						Some(b'}')	=> {
							self.i += 1;
							break;
						}
						_			=> return self.err("did not find expected ',' or '}'"),
					}
				}
				Value::dict(d)
			}
			Some(_)		=> res!(self.scalar(false)),
		};
		if let Some(a) = anchor {
			y.anchors.push((a, v.clone()));
		}
		Ok(v)
	}

	// A quoted or plain scalar; in a flow collection it stops at `,`, `]`, `}` and (for keys) `:`.
	fn scalar(&mut self, key: bool) -> Outcome<Value> {
		match self.b.get(self.i) {
			Some(b'"') => {
				self.i += 1;
				let mut out = String::new();
				loop {
					let ch = match self.s[self.i..].chars().next() {
						Some(c)	=> c,
						None	=> return self.err("found unexpected end of stream"),
					};
					self.i += ch.len_utf8();
					match ch {
						'"'	=> break,
						'\\' => {
							let e = match self.s[self.i..].chars().next() {
								Some(c)	=> c,
								None	=> return self.err("found unexpected end of stream"),
							};
							self.i += e.len_utf8();
							match e {
								'n'	=> out.push('\n'),
								't'	=> out.push('\t'),
								'r'	=> out.push('\r'),
								'0'	=> out.push('\0'),
								'"'	=> out.push('"'),
								'\\' => out.push('\\'),
								'/'	=> out.push('/'),
								' '	=> out.push(' '),
								'e'	=> out.push('\u{1b}'),
								'x' | 'u' | 'U' => {
									let n = match e { 'x' => 2, 'u' => 4, _ => 8 };
									let c = self.s.get(self.i..self.i + n).and_then(|h| u32::from_str_radix(h, 16).ok())
										.and_then(char::from_u32);
									match c {
										Some(c)	=> {
											out.push(c);
											self.i += n;
										}
										None	=> return self.err("invalid escape"),
									}
								}
								_ => return self.err("found unknown escape character"),
							}
						}
						c => out.push(c),
					}
				}
				Ok(Value::str(out))
			}
			Some(b'\'') => {
				self.i += 1;
				let mut out = String::new();
				loop {
					let ch = match self.s[self.i..].chars().next() {
						Some(c)	=> c,
						None	=> return self.err("found unexpected end of stream"),
					};
					self.i += ch.len_utf8();
					if ch == '\'' {
						if self.b.get(self.i) == Some(&b'\'') {
							self.i += 1;
							out.push('\'');
						} else {
							break;
						}
					} else {
						out.push(ch);
					}
				}
				Ok(Value::str(out))
			}
			_ => {
				let start = self.i;
				let in_flow = self.s.len() != self.b.len() || true;
				while self.i < self.b.len() {
					let c = self.b[self.i];
					if in_flow && matches!(c, b',' | b']' | b'}') && self.flow_context() {
						break;
					}
					if key && c == b':' {
						break;
					}
					self.i += 1;
				}
				let t = self.s[start..self.i].trim();
				Ok(yaml_plain(t))
			}
		}
	}

	// Whether the scalar being read sits inside a flow collection on this line.
	fn flow_context(&self) -> bool {
		let before = &self.s[..self.i];
		let opens = before.matches(['[', '{']).count();
		let closes = before.matches([']', '}']).count();
		opens > closes
	}
}

// A plain scalar under the YAML 1.2 core schema.
fn yaml_plain(t: &str) -> Value {
	match t {
		"" | "~" | "null" | "Null" | "NULL"	=> return Value::None,
		"true" | "True" | "TRUE"			=> return Value::Bool(true),
		"false" | "False" | "FALSE"			=> return Value::Bool(false),
		".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" => return Value::Float(f64::INFINITY),
		"-.inf" | "-.Inf" | "-.INF"			=> return Value::Float(f64::NEG_INFINITY),
		".nan" | ".NaN" | ".NAN"			=> return Value::Float(f64::NAN),
		_ => (),
	}
	if let Some(h) = t.strip_prefix("0x") {
		if let Ok(i) = i64::from_str_radix(h, 16) {
			return Value::Int(i);
		}
	}
	if let Some(o) = t.strip_prefix("0o") {
		if let Ok(i) = i64::from_str_radix(o, 8) {
			return Value::Int(i);
		}
	}
	let body = t.strip_prefix(['+', '-']).unwrap_or(t);
	if !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit()) {
		if let Ok(i) = t.parse::<i64>() {
			return Value::Int(i);
		}
	}
	let num = !body.is_empty()
		&& body.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
		&& body.bytes().any(|b| b.is_ascii_digit());
	if num {
		if let Ok(f) = t.parse::<f64>() {
			return Value::Float(f);
		}
	}
	Value::str(t)
}

// Whether serde_yaml would quote a string: when it would read back as another type, or starts or
// contains what YAML reads as syntax.
fn yaml_needs_quotes(s: &str) -> bool {
	if s.is_empty() || s == "-" {
		return true;
	}
	if !matches!(yaml_plain(s), Value::Str(_)) {
		return true;
	}
	let first = s.chars().next().unwrap_or(' ');
	if "@!%&*|>[{'\"#`,]}".contains(first) || first == ' ' || s.ends_with(' ') || s.starts_with("- ") {
		return true;
	}
	s.contains(": ") || s.contains(" #") || s.ends_with(':')
}

fn yaml_scalar(s: &str) -> String {
	if s.chars().any(|c| c.is_control() && c != '\n') {
		let mut out = String::from("\"");
		for c in s.chars() {
			match c {
				'"'		=> out.push_str("\\\""),
				'\\'	=> out.push_str("\\\\"),
				'\n'	=> out.push_str("\\n"),
				'\t'	=> out.push_str("\\t"),
				c if c.is_control() => out.push_str(&fmt!("\\x{:02X}", c as u32)),
				c		=> out.push(c),
			}
		}
		out.push('"');
		return out;
	}
	if yaml_needs_quotes(s) {
		fmt!("'{}'", s.replace('\'', "''"))
	} else {
		s.to_string()
	}
}

fn yaml_write(d: &Data, indent: usize, out: &mut String, top: bool) {
	let pad = "  ".repeat(indent);
	match d {
		Data::Map(m) if !m.is_empty() => {
			for (i, (k, v)) in m.iter().enumerate() {
				if i > 0 || !top {
					out.push_str(&pad);
				}
				out.push_str(&yaml_scalar(k));
				out.push(':');
				yaml_child(v, indent, out);
			}
		}
		Data::Seq(a) if !a.is_empty() => {
			for (i, v) in a.iter().enumerate() {
				if i > 0 || !top {
					out.push_str(&pad);
				}
				out.push('-');
				match v {
					Data::Map(m) if !m.is_empty() => {
						out.push(' ');
						yaml_write(v, indent + 1, out, true);
					}
					Data::Seq(s) if !s.is_empty() => {
						out.push(' ');
						yaml_write(v, indent + 1, out, true);
					}
					_ => {
						out.push(' ');
						yaml_leaf(v, indent + 1, out);
						out.push('\n');
					}
				}
			}
		}
		other => {
			yaml_leaf(other, indent, out);
			out.push('\n');
		}
	}
}

fn yaml_child(v: &Data, indent: usize, out: &mut String) {
	match v {
		Data::Map(m) if !m.is_empty() => {
			out.push('\n');
			yaml_write(v, indent + 1, out, false);
		}
		Data::Seq(a) if !a.is_empty() => {
			out.push('\n');
			yaml_write(v, indent, out, false);
		}
		_ => {
			out.push(' ');
			yaml_leaf(v, indent + 1, out);
			out.push('\n');
		}
	}
}

fn yaml_leaf(v: &Data, indent: usize, out: &mut String) {
	match v {
		Data::Null		=> out.push_str("null"),
		Data::Bool(b)	=> out.push_str(if *b { "true" } else { "false" }),
		Data::Int(i)	=> out.push_str(&i.to_string()),
		Data::Float(f)	=> out.push_str(&if f.is_nan() {
			".nan".to_string()
		} else if f.is_infinite() {
			if *f < 0.0 { "-.inf".to_string() } else { ".inf".to_string() }
		} else {
			ryu_format(*f)
		}),
		Data::Str(s) if s.contains('\n') && !s.chars().any(|c| c.is_control() && c != '\n') => {
			let chomp = if s.ends_with('\n') { "" } else { "-" };
			out.push('|');
			out.push_str(chomp);
			let pad = "  ".repeat(indent);
			for line in s.trim_end_matches('\n').split('\n') {
				out.push('\n');
				if !line.is_empty() {
					out.push_str(&pad);
					out.push_str(line);
				}
			}
		}
		Data::Str(s)	=> out.push_str(&yaml_scalar(s)),
		Data::Bytes(b)	=> {
			let mut first = true;
			out.push('[');
			for x in b {
				if !first {
					out.push_str(", ");
				}
				first = false;
				out.push_str(&x.to_string());
			}
			out.push(']');
		}
		Data::Seq(_)	=> out.push_str("[]"),
		Data::Map(_)	=> out.push_str("{}"),
	}
}
