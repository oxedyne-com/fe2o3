// The one value shape both sides of the comparison are brought to. The oracle's JSON is parsed into
// it and Austenite's values are serialised into it, so a comparison never goes through text.
//
// A reader of its own rather than `fe2o3_jdat`'s: the comparison depends on two facts JSON text
// carries and a general decoder is free to normalise away -- whether a number was written as an
// integer or a float (`1` is an `int`, `1.0` a `float`, and Typst tells them apart), and the order
// of an object's keys (a Typst dictionary iterates in insertion order).

use oxedyne_fe2o3_core::prelude::*;

#[derive(Clone, Debug)]
pub enum J {
	Null,
	Bool(bool),
	Int(i64),
	Float(f64),
	Str(String),
	Arr(Vec<J>),
	Obj(Vec<(String, J)>),	// in source order
}

impl J {
	pub fn str<S: Into<String>>(s: S) -> Self { J::Str(s.into()) }

	pub fn get(&self, key: &str) -> Option<&J> {
		match self {
			J::Obj(kv)	=> kv.iter().find(|(k, _)| k == key).map(|(_, v)| v),
			_			=> None,
		}
	}

	pub fn as_str(&self) -> Option<&str> {
		match self {
			J::Str(s)	=> Some(s),
			_			=> None,
		}
	}

	pub fn as_i64(&self) -> Option<i64> {
		match self {
			J::Int(i)	=> Some(*i),
			_			=> None,
		}
	}

	/// Is this a Typst content object (`{"func": ..., ...}`), whose keys are fields rather than a
	/// dictionary's ordered entries?
	pub fn is_content(&self) -> bool {
		matches!(self.get("func"), Some(J::Str(_)))
	}

	/// Compact JSON, for reports.
	pub fn render(&self) -> String {
		let mut s = String::new();
		self.push(&mut s);
		s
	}

	fn push(&self, s: &mut String) {
		match self {
			J::Null		=> s.push_str("null"),
			J::Bool(b)	=> s.push_str(if *b { "true" } else { "false" }),
			J::Int(i)	=> s.push_str(&i.to_string()),
			J::Float(f)	=> {
				let t = fmt!("{:?}", f);
				s.push_str(&t);
			}
			J::Str(t)	=> push_str_lit(s, t),
			J::Arr(a)	=> {
				s.push('[');
				for (i, v) in a.iter().enumerate() {
					if i > 0 {
						s.push(',');
					}
					v.push(s);
				}
				s.push(']');
			}
			J::Obj(kv)	=> {
				s.push('{');
				for (i, (k, v)) in kv.iter().enumerate() {
					if i > 0 {
						s.push(',');
					}
					push_str_lit(s, k);
					s.push(':');
					v.push(s);
				}
				s.push('}');
			}
		}
	}
}

fn push_str_lit(s: &mut String, t: &str) {
	s.push('"');
	for c in t.chars() {
		match c {
			'"'					=> s.push_str("\\\""),
			'\\'				=> s.push_str("\\\\"),
			'\n'				=> s.push_str("\\n"),
			'\t'				=> s.push_str("\\t"),
			c if (c as u32) < 0x20	=> s.push_str(&fmt!("\\u{:04x}", c as u32)),
			c					=> s.push(c),
		}
	}
	s.push('"');
}

/// Parses one JSON document; trailing non-space text is an error.
pub fn parse(text: &str) -> Outcome<J> {
	let mut p = Parser { b: text.as_bytes(), i: 0 };
	let v = res!(p.value(0));
	p.ws();
	if p.i != p.b.len() {
		return Err(err!("JSON: trailing text at byte {}", p.i; Decode, Invalid));
	}
	Ok(v)
}

struct Parser<'a> {
	b:	&'a [u8],
	i:	usize,
}

impl<'a> Parser<'a> {
	fn ws(&mut self) {
		while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\n' | b'\r' | b'\t') {
			self.i += 1;
		}
	}

	fn peek(&self) -> Option<u8> { self.b.get(self.i).copied() }

	fn expect(&mut self, c: u8) -> Outcome<()> {
		self.ws();
		if self.peek() == Some(c) {
			self.i += 1;
			return Ok(());
		}
		Err(err!("JSON: expected '{}' at byte {}", c as char, self.i; Decode, Invalid))
	}

	fn lit(&mut self, word: &str, v: J) -> Outcome<J> {
		if self.b[self.i..].starts_with(word.as_bytes()) {
			self.i += word.len();
			return Ok(v);
		}
		Err(err!("JSON: unexpected text at byte {}", self.i; Decode, Invalid))
	}

	fn value(&mut self, depth: usize) -> Outcome<J> {
		if depth > 512 {
			return Err(err!("JSON: nesting deeper than 512"; Decode, Excessive));
		}
		self.ws();
		match self.peek() {
			Some(b'n')	=> self.lit("null", J::Null),
			Some(b't')	=> self.lit("true", J::Bool(true)),
			Some(b'f')	=> self.lit("false", J::Bool(false)),
			Some(b'"')	=> Ok(J::Str(res!(self.string()))),
			Some(b'[')	=> {
				self.i += 1;
				let mut out = Vec::new();
				self.ws();
				if self.peek() == Some(b']') {
					self.i += 1;
					return Ok(J::Arr(out));
				}
				loop {
					out.push(res!(self.value(depth + 1)));
					self.ws();
					match self.peek() {
						Some(b',')	=> self.i += 1,
						Some(b']')	=> {
							self.i += 1;
							return Ok(J::Arr(out));
						}
						_ => return Err(err!("JSON: expected ',' or ']' at byte {}", self.i; Decode, Invalid)),
					}
				}
			}
			Some(b'{')	=> {
				self.i += 1;
				let mut out = Vec::new();
				self.ws();
				if self.peek() == Some(b'}') {
					self.i += 1;
					return Ok(J::Obj(out));
				}
				loop {
					self.ws();
					let k = res!(self.string());
					res!(self.expect(b':'));
					let v = res!(self.value(depth + 1));
					out.push((k, v));
					self.ws();
					match self.peek() {
						Some(b',')	=> self.i += 1,
						Some(b'}')	=> {
							self.i += 1;
							return Ok(J::Obj(out));
						}
						_ => return Err(err!("JSON: expected ',' or '}}' at byte {}", self.i; Decode, Invalid)),
					}
				}
			}
			Some(c) if c == b'-' || c.is_ascii_digit() => self.number(),
			_ => Err(err!("JSON: unexpected byte at {}", self.i; Decode, Invalid)),
		}
	}

	fn number(&mut self) -> Outcome<J> {
		let start = self.i;
		let mut float = false;
		while let Some(c) = self.peek() {
			match c {
				b'0'..=b'9' | b'-' | b'+'	=> (),
				b'.' | b'e' | b'E'			=> float = true,
				_							=> break,
			}
			self.i += 1;
		}
		let t = match std::str::from_utf8(&self.b[start..self.i]) {
			Ok(t)	=> t,
			Err(_)	=> return Err(err!("JSON: bad number at byte {}", start; Decode, Invalid)),
		};
		if !float {
			if let Ok(i) = t.parse::<i64>() {
				return Ok(J::Int(i));
			}
		}
		match t.parse::<f64>() {
			Ok(f)	=> Ok(J::Float(f)),
			Err(_)	=> Err(err!("JSON: bad number '{}' at byte {}", t, start; Decode, Invalid)),
		}
	}

	fn string(&mut self) -> Outcome<String> {
		if self.peek() != Some(b'"') {
			return Err(err!("JSON: expected a string at byte {}", self.i; Decode, Invalid));
		}
		self.i += 1;
		let mut out: Vec<u8> = Vec::new();
		loop {
			let c = match self.peek() {
				Some(c)	=> c,
				None	=> return Err(err!("JSON: unterminated string"; Decode, Invalid)),
			};
			self.i += 1;
			match c {
				b'"'	=> break,
				b'\\'	=> {
					let e = match self.peek() {
						Some(e)	=> e,
						None	=> return Err(err!("JSON: unterminated escape"; Decode, Invalid)),
					};
					self.i += 1;
					match e {
						b'n'	=> out.push(b'\n'),
						b't'	=> out.push(b'\t'),
						b'r'	=> out.push(b'\r'),
						b'b'	=> out.push(0x08),
						b'f'	=> out.push(0x0c),
						b'u'	=> {
							let mut cp = res!(self.hex4());
							if (0xd800..0xdc00).contains(&cp) && self.b[self.i..].starts_with(b"\\u") {
								self.i += 2;
								let lo = res!(self.hex4());
								cp = 0x10000 + ((cp - 0xd800) << 10) + (lo.wrapping_sub(0xdc00) & 0x3ff);
							}
							let ch = char::from_u32(cp).unwrap_or('\u{fffd}');
							let mut buf = [0u8; 4];
							out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
						}
						other	=> out.push(other),
					}
				}
				c		=> out.push(c),
			}
		}
		match String::from_utf8(out) {
			Ok(s)	=> Ok(s),
			Err(_)	=> Err(err!("JSON: a string is not UTF-8"; Decode, Invalid)),
		}
	}

	fn hex4(&mut self) -> Outcome<u32> {
		let end = self.i + 4;
		if end > self.b.len() {
			return Err(err!("JSON: short \\u escape"; Decode, Invalid));
		}
		let t = match std::str::from_utf8(&self.b[self.i..end]) {
			Ok(t)	=> t,
			Err(_)	=> return Err(err!("JSON: bad \\u escape"; Decode, Invalid)),
		};
		self.i = end;
		match u32::from_str_radix(t, 16) {
			Ok(v)	=> Ok(v),
			Err(_)	=> Err(err!("JSON: bad \\u escape '{}'", t; Decode, Invalid)),
		}
	}
}

/// Differences between an oracle value and Austenite's, each as `path: what`. Floats agree within
/// 1e-9, relative for magnitudes above one; a content object's keys are compared as a set, a
/// dictionary's in order.
pub fn diff(path: &str, want: &J, got: &J, out: &mut Vec<String>) {
	if out.len() >= 20 {
		return;
	}
	match (want, got) {
		(J::Null, J::Null)			=> (),
		(J::Bool(a), J::Bool(b)) if a == b	=> (),
		(J::Int(a), J::Int(b)) if a == b	=> (),
		(J::Float(a), J::Float(b)) if close(*a, *b)	=> (),
		(J::Str(a), J::Str(b)) if a == b	=> (),
		(J::Arr(a), J::Arr(b))		=> {
			if a.len() != b.len() {
				out.push(fmt!("{}: typst has {} item(s), austenite {}", path, a.len(), b.len()));
			}
			for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
				diff(&fmt!("{}[{}]", path, i), x, y, out);
			}
		}
		(J::Obj(a), J::Obj(b))		=> {
			let unordered = want.is_content() && got.is_content();
			let ka: Vec<&str> = a.iter().map(|(k, _)| k.as_str()).collect();
			let kb: Vec<&str> = b.iter().map(|(k, _)| k.as_str()).collect();
			if unordered {
				let mut sa = ka.clone();
				let mut sb = kb.clone();
				sa.sort();
				sb.sort();
				if sa != sb {
					out.push(fmt!("{}: typst fields {:?}, austenite {:?}", path, ka, kb));
				}
			} else if ka != kb {
				out.push(fmt!("{}: typst keys {:?}, austenite {:?}", path, ka, kb));
			}
			for (k, x) in a {
				if let Some(y) = got.get(k) {
					diff(&fmt!("{}.{}", path, k), x, y, out);
				}
			}
		}
		_ => out.push(fmt!("{}: typst {}, austenite {}", path, clip(&want.render()), clip(&got.render()))),
	}
}

fn close(a: f64, b: f64) -> bool {
	if a == b || (a.is_nan() && b.is_nan()) {
		return true;
	}
	(a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

pub fn clip(s: &str) -> String {
	if s.chars().count() <= 160 {
		return s.to_string();
	}
	let mut t: String = s.chars().take(157).collect();
	t.push_str("...");
	t
}
