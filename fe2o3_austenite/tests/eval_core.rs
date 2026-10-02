//! U2's evaluator against the `typst` 0.15 oracle. Until U1's parser lands, each fixture pairs Typst code
//! with the syntax tree the parser yields for it, written as an s-expression; the code goes to
//! `typst eval`, the tree to Austenite's evaluator, and the two results are compared as JSON (values) or
//! as the first error message (errors). Nothing expected is checked in: the oracle decides.
//!
//! Each case also runs through U1's parser, as `typst eval` itself does -- the code as a string to
//! `eval_string` in code mode -- so the parser and the evaluator are checked together against the same
//! oracle, and a hand tree that drifts from what the parser yields cannot hide a fault.
//!
//! `EVAL_ORACLE_SKIP=1` skips explicitly; a missing `typst` binary otherwise fails, so absence never
//! reads as green.

use oxedyne_fe2o3_austenite::diag::DiagnosticKind;
use oxedyne_fe2o3_austenite::eval::content::{
	Content,
	ElemKind,
};
use oxedyne_fe2o3_austenite::eval::lib::foundations::repr;
use oxedyne_fe2o3_austenite::eval::func::Func;
use oxedyne_fe2o3_austenite::eval::value::{
	Length,
	Value,
};
use oxedyne_fe2o3_austenite::eval::eval::{
	eval_string,
	EvalMode,
};
use oxedyne_fe2o3_austenite::eval::scope::Scope;
use oxedyne_fe2o3_austenite::eval::{
	eval_source,
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::syntax::{
	FileId,
	Source,
	Span,
	SyntaxKind,
	SyntaxNode,
};

use oxedyne_fe2o3_core::prelude::*;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

// S-expression trees

enum Tok {
	Open,
	Close,
	Quoted(String),		// 'leaf text'
	StrLit(String),		// "a string literal", quotes included
	Atom(String),
}

fn lex(src: &str) -> Outcome<Vec<Tok>> {
	let mut out = Vec::new();
	let cs: Vec<char> = src.chars().collect();
	let mut i = 0;
	while i < cs.len() {
		let c = cs[i];
		match c {
			'(' => { out.push(Tok::Open); i += 1; }
			')' => { out.push(Tok::Close); i += 1; }
			'\'' => {
				let mut j = i + 1;
				while j < cs.len() && cs[j] != '\'' {
					j += 1;
				}
				if j >= cs.len() {
					return Err(err!("Unterminated quoted leaf in {}", src; Input, Invalid));
				}
				out.push(Tok::Quoted(cs[i + 1..j].iter().collect()));
				i = j + 1;
			}
			'"' => {
				let mut j = i + 1;
				while j < cs.len() && cs[j] != '"' {
					if cs[j] == '\\' {
						j += 1;
					}
					j += 1;
				}
				if j >= cs.len() {
					return Err(err!("Unterminated string literal in {}", src; Input, Invalid));
				}
				out.push(Tok::StrLit(cs[i..=j].iter().collect()));
				i = j + 1;
			}
			c if c.is_whitespace() => i += 1,
			_ => {
				let mut j = i;
				while j < cs.len() && !cs[j].is_whitespace() && cs[j] != '(' && cs[j] != ')' {
					j += 1;
				}
				out.push(Tok::Atom(cs[i..j].iter().collect()));
				i = j;
			}
		}
	}
	Ok(out)
}

fn kind_of(name: &str) -> Option<(SyntaxKind, &'static str)> {
	use SyntaxKind as K;
	Some(match name {
		"Markup"		=> (K::Markup, ""),
		"Text"			=> (K::Text, ""),
		"Space"			=> (K::Space, " "),
		"Linebreak"		=> (K::Linebreak, "\\"),
		"Parbreak"		=> (K::Parbreak, "\n\n"),
		"Escape"		=> (K::Escape, ""),
		"Shorthand"		=> (K::Shorthand, ""),
		"Label"			=> (K::Label, ""),
		"Hash"			=> (K::Hash, "#"),
		"LeftBrace"		=> (K::LeftBrace, "{"),
		"RightBrace"	=> (K::RightBrace, "}"),
		"LeftBracket"	=> (K::LeftBracket, "["),
		"RightBracket"	=> (K::RightBracket, "]"),
		"LeftParen"		=> (K::LeftParen, "("),
		"RightParen"	=> (K::RightParen, ")"),
		"Comma"			=> (K::Comma, ","),
		"Semicolon"		=> (K::Semicolon, ";"),
		"Colon"			=> (K::Colon, ":"),
		"Underscore"	=> (K::Underscore, "_"),
		"Plus"			=> (K::Plus, "+"),
		"Minus"			=> (K::Minus, "-"),
		"Star"			=> (K::Star, "*"),
		"Slash"			=> (K::Slash, "/"),
		"Dot"			=> (K::Dot, "."),
		"Eq"			=> (K::Eq, "="),
		"EqEq"			=> (K::EqEq, "=="),
		"ExclEq"		=> (K::ExclEq, "!="),
		"Lt"			=> (K::Lt, "<"),
		"LtEq"			=> (K::LtEq, "<="),
		"Gt"			=> (K::Gt, ">"),
		"GtEq"			=> (K::GtEq, ">="),
		"PlusEq"		=> (K::PlusEq, "+="),
		"HyphEq"		=> (K::HyphEq, "-="),
		"StarEq"		=> (K::StarEq, "*="),
		"SlashEq"		=> (K::SlashEq, "/="),
		"Dots"			=> (K::Dots, ".."),
		"Arrow"			=> (K::Arrow, "=>"),
		"Not"			=> (K::Not, "not"),
		"And"			=> (K::And, "and"),
		"Or"			=> (K::Or, "or"),
		"Let"			=> (K::Let, "let"),
		"Set"			=> (K::Set, "set"),
		"Show"			=> (K::Show, "show"),
		"If"			=> (K::If, "if"),
		"Else"			=> (K::Else, "else"),
		"For"			=> (K::For, "for"),
		"In"			=> (K::In, "in"),
		"While"			=> (K::While, "while"),
		"Break"			=> (K::Break, "break"),
		"Continue"		=> (K::Continue, "continue"),
		"Return"		=> (K::Return, "return"),
		"Code"			=> (K::Code, ""),
		"Strong"		=> (K::Strong, ""),
		"Emph"			=> (K::Emph, ""),
		"Raw"			=> (K::Raw, ""),
		"RawLang"		=> (K::RawLang, ""),
		"RawDelim"		=> (K::RawDelim, ""),
		"RawTrimmed"	=> (K::RawTrimmed, ""),
		"Link"			=> (K::Link, ""),
		"Ref"			=> (K::Ref, ""),
		"RefMarker"		=> (K::RefMarker, ""),
		"Heading"		=> (K::Heading, ""),
		"HeadingMarker"	=> (K::HeadingMarker, ""),
		"ListItem"		=> (K::ListItem, ""),
		"ListMarker"	=> (K::ListMarker, "-"),
		"EnumItem"		=> (K::EnumItem, ""),
		"EnumMarker"	=> (K::EnumMarker, ""),
		"TermItem"		=> (K::TermItem, ""),
		"TermMarker"	=> (K::TermMarker, "/"),
		"SmartQuote"	=> (K::SmartQuote, ""),
		"Equation"		=> (K::Equation, ""),
		"Math"			=> (K::Math, ""),
		"MathText"		=> (K::MathText, ""),
		"MathIdent"		=> (K::MathIdent, ""),
		"MathAttach"	=> (K::MathAttach, ""),
		"MathPrimes"	=> (K::MathPrimes, "'"),	// the parser lexes a run of primes as one leaf
		"MathFrac"		=> (K::MathFrac, ""),
		"MathDelimited"	=> (K::MathDelimited, ""),
		"Dollar"		=> (K::Dollar, "$"),
		"Hat"			=> (K::Hat, "^"),
		"Prime"			=> (K::Prime, "'"),
		"Contextual"	=> (K::Contextual, ""),
		"ModuleImport"	=> (K::ModuleImport, ""),
		"ImportItems"	=> (K::ImportItems, ""),
		"ImportItemPath"	=> (K::ImportItemPath, ""),
		"RenamedImportItem"	=> (K::RenamedImportItem, ""),
		"Import"		=> (K::Import, "import"),
		"As"			=> (K::As, "as"),
		"Context"		=> (K::Context, "context"),
		"CodeBlock"		=> (K::CodeBlock, ""),
		"ContentBlock"	=> (K::ContentBlock, ""),
		"Parenthesized"	=> (K::Parenthesized, ""),
		"Array"			=> (K::Array, ""),
		"Dict"			=> (K::Dict, ""),
		"Named"			=> (K::Named, ""),
		"Keyed"			=> (K::Keyed, ""),
		"Unary"			=> (K::Unary, ""),
		"Binary"		=> (K::Binary, ""),
		"FieldAccess"	=> (K::FieldAccess, ""),
		"FuncCall"		=> (K::FuncCall, ""),
		"Args"			=> (K::Args, ""),
		"Spread"		=> (K::Spread, ""),
		"Closure"		=> (K::Closure, ""),
		"Params"		=> (K::Params, ""),
		"LetBinding"	=> (K::LetBinding, ""),
		"SetRule"		=> (K::SetRule, ""),
		"ShowRule"		=> (K::ShowRule, ""),
		"Conditional"	=> (K::Conditional, ""),
		"WhileLoop"		=> (K::WhileLoop, ""),
		"ForLoop"		=> (K::ForLoop, ""),
		"LoopBreak"		=> (K::LoopBreak, ""),
		"LoopContinue"	=> (K::LoopContinue, ""),
		"FuncReturn"	=> (K::FuncReturn, ""),
		"Destructuring"	=> (K::Destructuring, ""),
		"DestructAssignment"	=> (K::DestructAssignment, ""),
		_				=> return None,
	})
}

const UNITS: &[&str] = &["pt", "mm", "cm", "in", "deg", "rad", "em", "fr", "%"];

struct Builder {
	offset:	u32,
}

impl Builder {
	fn leaf(&mut self, kind: SyntaxKind, text: &str) -> SyntaxNode {
		let start = self.offset;
		self.offset += text.len() as u32;
		SyntaxNode::leaf(kind, text, Span::new(FileId(0), start, self.offset))
	}

	fn atom(&mut self, a: &str) -> Outcome<SyntaxNode> {
		if let Some((k, text)) = kind_of(a) {
			return Ok(self.leaf(k, text));
		}
		let first = a.chars().next().unwrap_or(' ');
		let kind = if first.is_ascii_digit() {
			if a.starts_with("0x") || a.starts_with("0o") || a.starts_with("0b") {
				SyntaxKind::Int
			} else if UNITS.iter().any(|u| a.ends_with(u)) {
				SyntaxKind::Numeric
			} else if a.contains('.') || a.contains('e') {
				SyntaxKind::Float
			} else {
				SyntaxKind::Int
			}
		} else {
			match a {
				"true" | "false"	=> SyntaxKind::Bool,
				"none"				=> SyntaxKind::None,
				"auto"				=> SyntaxKind::Auto,
				_ if first.is_ascii_uppercase() => return Err(err!("Unknown kind {}", a; Input, Invalid)),
				_					=> SyntaxKind::Ident,
			}
		};
		Ok(self.leaf(kind, a))
	}

	fn node(&mut self, toks: &[Tok], i: &mut usize) -> Outcome<SyntaxNode> {
		let t = match toks.get(*i) {
			Some(t)	=> t,
			None	=> return Err(err!("Unexpected end of tree"; Input, Invalid)),
		};
		*i += 1;
		match t {
			Tok::Atom(a)	=> self.atom(a),
			Tok::StrLit(s)	=> Ok(self.leaf(SyntaxKind::Str, s)),
			Tok::Quoted(_)	=> Err(err!("A quoted leaf must follow a kind"; Input, Invalid)),
			Tok::Close		=> Err(err!("Unbalanced )"; Input, Invalid)),
			Tok::Open		=> {
				let name = match toks.get(*i) {
					Some(Tok::Atom(a))	=> a.clone(),
					_					=> return Err(err!("Expected a kind after ("; Input, Invalid)),
				};
				*i += 1;
				let (kind, _) = res!(kind_of(&name).ok_or_else(|| err!("Unknown kind {}", name; Input, Invalid)));
				if let (Some(Tok::Quoted(text)), Some(Tok::Close)) = (toks.get(*i), toks.get(*i + 1)) {
					*i += 2;
					return Ok(self.leaf(kind, text));
				}
				let mut kids = Vec::new();
				loop {
					match toks.get(*i) {
						Some(Tok::Close)	=> {
							*i += 1;
							break;
						}
						Some(_)				=> kids.push(res!(self.node(toks, i))),
						None				=> return Err(err!("Unclosed ( in {}", name; Input, Invalid)),
					}
				}
				Ok(SyntaxNode::inner(kind, kids))
			}
		}
	}
}

// The module `#let result = <expr>`, so the evaluated module's scope holds the value.
fn module_for(sexpr: &str) -> Outcome<SyntaxNode> {
	let toks = res!(lex(sexpr));
	let mut b = Builder { offset: 0 };
	let hash = b.leaf(SyntaxKind::Hash, "#");
	let kw = b.leaf(SyntaxKind::Let, "let");
	let name = b.leaf(SyntaxKind::Ident, "result");
	let eq = b.leaf(SyntaxKind::Eq, "=");
	let mut i = 0;
	let expr = res!(b.node(&toks, &mut i));
	if i != toks.len() {
		return Err(err!("Trailing tokens after the tree {}", sexpr; Input, Invalid));
	}
	let bind = SyntaxNode::inner(SyntaxKind::LetBinding, vec![kw, name, eq, expr]);
	Ok(SyntaxNode::inner(SyntaxKind::Markup, vec![hash, bind]))
}

fn evaluate(sexpr: &str) -> Outcome<(Engine, Outcome<Value>)> {
	let root = res!(module_for(sexpr));
	let mut world = World::new(PathBuf::from("/"));
	world.sources.push(Source {
		id:		FileId(0),
		path:	PathBuf::from("/case.typ"),
		text:	Arc::new(root.full_text()),
		root,
	});
	let mut engine = Engine::new(world);
	let out = eval_source(&mut engine, FileId(0)).map(|m| m.scope.get("result").cloned().unwrap_or(Value::None));
	Ok((engine, out))
}

// The case's code through U1's parser, as `typst eval` takes it: a string evaluated in code mode.
fn evaluate_parsed(code: &str) -> (Engine, Outcome<Value>) {
	let mut engine = Engine::new(World::new(PathBuf::from("/")));
	let out = eval_string(&mut engine, code, EvalMode::Code, Scope::new(), Span::detached());
	(engine, out)
}

// Values as `typst eval` serialises them

fn num(f: f64) -> String {
	// Typst's repr rounds unit values to two decimals and drops trailing zeros.
	let r = (f * 100.0).round() / 100.0;
	let s = fmt!("{:.2}", r);
	let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
	if s == "-0" { "0".to_string() } else { s }
}

fn length_repr(l: &Length) -> String {
	match (l.abs != 0.0, l.em != 0.0) {
		(true, true)	=> fmt!("{}pt + {}em", num(l.abs), num(l.em)),
		(false, true)	=> fmt!("{}em", num(l.em)),
		_				=> fmt!("{}pt", num(l.abs)),
	}
}

fn json_str(s: &str) -> String {
	let mut o = String::from("\"");
	for c in s.chars() {
		match c {
			'"'		=> o.push_str("\\\""),
			'\\'	=> o.push_str("\\\\"),
			'\n'	=> o.push_str("\\n"),
			'\r'	=> o.push_str("\\r"),
			'\t'	=> o.push_str("\\t"),
			c if (c as u32) < 0x20	=> o.push_str(&fmt!("\\u{:04x}", c as u32)),
			c		=> o.push(c),
		}
	}
	o.push('"');
	o
}

fn float(f: f64) -> String {
	if f.fract() == 0.0 && f.abs() < 1e16 { fmt!("{:.1}", f) } else { fmt!("{}", f) }
}

fn short_repr(v: &Value) -> String {
	match v {
		Value::Int(i)	=> fmt!("{}", i),
		Value::Str(s)	=> fmt!("\"{}\"", s),
		other			=> to_json(other),
	}
}

fn content_json(c: &Content) -> String {
	let mut parts = Vec::new();
	match c {
		Content::Elem(e) => {
			parts.push(fmt!("\"func\":{}", json_str(e.kind.name())));
			let mut fields: Vec<_> = e.fields.iter().collect();
			fields.sort_by_key(|(id, _)| *id);
			for (id, v) in fields {
				let name = e.kind.field_spec(*id).map(|s| s.name).unwrap_or("?");
				// `context`'s closure is internal; Typst's serialisation leaves it out.
				if name == "func" {
					continue;
				}
				parts.push(fmt!("{}:{}", json_str(name), to_json(v)));
			}
		}
		Content::Sequence(s) => {
			parts.push("\"func\":\"sequence\"".to_string());
			let kids: Vec<String> = s.children.iter().map(content_json).collect();
			parts.push(fmt!("\"children\":[{}]", kids.join(",")));
		}
		Content::Styled(s) => {
			parts.push("\"func\":\"styled\"".to_string());
			parts.push(fmt!("\"child\":{}", content_json(&s.child)));
			parts.push("\"styles\":\"styles(..)\"".to_string());
		}
	}
	if let Some(l) = c.label() {
		parts.push(fmt!("\"label\":\"<{}>\"", l.as_str()));
	}
	fmt!("{{{}}}", parts.join(","))
}

fn to_json(v: &Value) -> String {
	match v {
		Value::None			=> "null".to_string(),
		Value::Auto			=> "\"auto\"".to_string(),
		Value::Bool(b)		=> fmt!("{}", b),
		Value::Int(i)		=> fmt!("{}", i),
		Value::Float(f)		=> float(*f),
		Value::Str(s)		=> json_str(s),
		Value::Length(l)	=> json_str(&length_repr(l)),
		Value::Ratio(r)		=> json_str(&fmt!("{}%", num(r.0 * 100.0))),
		Value::Angle(a)		=> json_str(&fmt!("{}deg", num(a.0.to_degrees()))),
		Value::Fraction(f)	=> json_str(&fmt!("{}fr", num(f.0))),
		Value::Relative(r)	=> {
			let mut s = fmt!("{}%", num(r.rel.0 * 100.0));
			if r.abs.abs != 0.0 {
				s.push_str(&fmt!(" + {}pt", num(r.abs.abs)));
			}
			if r.abs.em != 0.0 {
				s.push_str(&fmt!(" + {}em", num(r.abs.em)));
			}
			json_str(&s)
		}
		Value::Label(l)		=> json_str(&fmt!("<{}>", l.as_str())),
		Value::Array(a)		=> fmt!("[{}]", a.iter().map(to_json).collect::<Vec<_>>().join(",")),
		Value::Dict(d)		=> fmt!("{{{}}}", d.iter().map(|(k, v)| fmt!("{}:{}", json_str(k), to_json(v)))
			.collect::<Vec<_>>().join(",")),
		Value::Content(c)	=> content_json(c),
		Value::Func(f)		=> json_str(match f {
			Func::Closure(c) if c.name.is_none()	=> "(..) => ..",
			other									=> other.name().unwrap_or("?"),
		}),
		Value::Args(a)		=> {
			let items: Vec<String> = a.items.iter().map(|i| match &i.name {
				Some(n)	=> fmt!("{}: {}", n, short_repr(&i.value)),
				None	=> short_repr(&i.value),
			}).collect();
			json_str(&fmt!("arguments({})", items.join(", ")))
		}
		other				=> json_str(&fmt!("<unserialised {}>", other.ty().name())),
	}
}

// The oracle

struct Case {
	code:	String,
	tree:	String,
	dep:	Option<String>,	// another unit the case needs, from `%%[dep]`
}

// Is the unit a case depends on present yet? Probed through its own public surface, so a case is
// enforced from the moment its dependency lands.
fn available(deps: &str) -> bool {
	deps.split(',').all(|dep| match dep.trim() {
		"U3"		=> repr(&Value::Int(1)) == "1",
		"U5"		=> ElemKind::Strong.field_id("body").is_some() && ElemKind::Heading.field_id("depth").is_some(),
		"U6a"		=> ElemKind::SmartQuote.field_id("double").is_some(),
		"U7"		=> ElemKind::MathFrac.field_id("num").is_some() && ElemKind::MathAttach.field_id("base").is_some(),
		"U8"		=> ElemKind::Context.field_id("func").is_some(),
		"symbol"	=> ElemKind::Symbol.field_id("text").is_some(),
		_			=> false,
	})
}

// JSON, only as far as comparing `typst eval` output needs: an element's fields compare in any order
// (the schema's order is the owning unit's), a dictionary's in insertion order (Typst's semantics).
#[derive(Debug, PartialEq)]
enum Json {
	Null,
	Bool(bool),
	Num(String),
	Str(String),
	Arr(Vec<Json>),
	Obj(Vec<(String, Json)>),
}

fn parse_json(s: &str) -> Outcome<Json> {
	let cs: Vec<char> = s.chars().collect();
	let mut i = 0;
	let j = res!(json_value(&cs, &mut i));
	Ok(canon(j))
}

fn json_ws(cs: &[char], i: &mut usize) {
	while *i < cs.len() && cs[*i].is_whitespace() {
		*i += 1;
	}
}

fn json_string(cs: &[char], i: &mut usize) -> Outcome<String> {
	*i += 1;
	let mut out = String::new();
	while *i < cs.len() && cs[*i] != '"' {
		if cs[*i] == '\\' {
			*i += 1;
			match cs.get(*i) {
				Some('n')	=> out.push('\n'),
				Some('t')	=> out.push('\t'),
				Some('r')	=> out.push('\r'),
				Some('u')	=> {
					let hex: String = cs[*i + 1..(*i + 5).min(cs.len())].iter().collect();
					if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
						out.push(c);
					}
					*i += 4;
				}
				Some(c)		=> out.push(*c),
				None		=> (),
			}
		} else {
			out.push(cs[*i]);
		}
		*i += 1;
	}
	*i += 1;
	Ok(out)
}

fn json_value(cs: &[char], i: &mut usize) -> Outcome<Json> {
	json_ws(cs, i);
	let c = match cs.get(*i) {
		Some(c)	=> *c,
		None	=> return Err(err!("Truncated JSON"; Input, Invalid)),
	};
	match c {
		'"' => Ok(Json::Str(res!(json_string(cs, i)))),
		'[' => {
			*i += 1;
			let mut v = Vec::new();
			loop {
				json_ws(cs, i);
				if cs.get(*i) == Some(&']') {
					*i += 1;
					break;
				}
				v.push(res!(json_value(cs, i)));
				json_ws(cs, i);
				if cs.get(*i) == Some(&',') {
					*i += 1;
				}
			}
			Ok(Json::Arr(v))
		}
		'{' => {
			*i += 1;
			let mut v = Vec::new();
			loop {
				json_ws(cs, i);
				if cs.get(*i) == Some(&'}') {
					*i += 1;
					break;
				}
				let k = res!(json_string(cs, i));
				json_ws(cs, i);
				*i += 1;
				let x = res!(json_value(cs, i));
				v.push((k, x));
				json_ws(cs, i);
				if cs.get(*i) == Some(&',') {
					*i += 1;
				}
			}
			Ok(Json::Obj(v))
		}
		_ => {
			let start = *i;
			while *i < cs.len() && !matches!(cs[*i], ',' | ']' | '}') && !cs[*i].is_whitespace() {
				*i += 1;
			}
			let t: String = cs[start..*i].iter().collect();
			Ok(match t.as_str() {
				"null"	=> Json::Null,
				"true"	=> Json::Bool(true),
				"false"	=> Json::Bool(false),
				_		=> Json::Num(t),
			})
		}
	}
}

fn canon(j: Json) -> Json {
	match j {
		Json::Arr(v)	=> Json::Arr(v.into_iter().map(canon).collect()),
		Json::Obj(v)	=> {
			let elem = v.iter().any(|(k, _)| k == "func");
			let mut v: Vec<(String, Json)> = v.into_iter().map(|(k, x)| (k, canon(x))).collect();
			if elem {
				v.sort_by(|a, b| a.0.cmp(&b.0));
			}
			Json::Obj(v)
		}
		other			=> other,
	}
}

fn cases(file: &str) -> Outcome<Vec<Case>> {
	let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval/core").join(file);
	let text = res!(std::fs::read_to_string(&path).map_err(|e| err!(
		"Could not read {}: {}", path.display(), e; IO, File, Read)));
	let mut out: Vec<Case> = Vec::new();
	for line in text.lines() {
		if line.starts_with(";;") {
			continue;
		}
		if let Some(code) = line.strip_prefix("%% ") {
			out.push(Case { code: code.to_string(), tree: String::new(), dep: None });
		} else if let Some(tagged) = line.strip_prefix("%%[") {
			let (dep, code) = match tagged.split_once("] ") {
				Some(p)	=> p,
				None	=> return Err(err!("Malformed case header {}", line; Input, Invalid)),
			};
			out.push(Case { code: code.to_string(), tree: String::new(), dep: Some(dep.to_string()) });
		} else if let Some(c) = out.last_mut() {
			c.tree.push_str(line);
			c.tree.push('\n');
		}
	}
	Ok(out)
}

fn skip() -> bool { std::env::var("EVAL_ORACLE_SKIP").map(|v| v == "1").unwrap_or(false) }

// `typst eval`, memory-capped when systemd can scope it. Environment expansion is off, or systemd reads
// the `$` of an equation as a variable.
fn oracle(code: &str) -> Outcome<(bool, String)> {
	let capped = Command::new("systemd-run")
		.args(["--user", "--scope", "--quiet", "--expand-environment=no", "-p", "MemoryMax=3G",
			"--slice=claude-rc.slice", "typst", "eval", "--", code])
		.output();
	let out = match capped {
		Ok(o) if o.status.code() != Some(127) && !String::from_utf8_lossy(&o.stderr).contains("Failed to") => o,
		_ => res!(Command::new("typst").args(["eval", "--", code]).output().map_err(|e| err!(
			"The typst oracle could not run ({}); set EVAL_ORACLE_SKIP=1 to skip explicitly.", e; IO, Missing))),
	};
	if out.status.success() {
		return Ok((true, String::from_utf8_lossy(&out.stdout).trim().to_string()));
	}
	let stderr = String::from_utf8_lossy(&out.stderr).to_string();
	let first = stderr.lines().find_map(|l| l.strip_prefix("error: ")).unwrap_or(stderr.trim()).to_string();
	Ok((false, first))
}

fn first_error(engine: &Engine) -> String {
	engine.diags.iter().find(|d| d.is_error()).map(|d| d.message.clone()).unwrap_or_default()
}

#[test]
fn values_match_the_typst_oracle() -> Outcome<()> {
	if skip() {
		println!("EVAL_ORACLE_SKIP=1: oracle comparison skipped");
		return Ok(());
	}
	let all = res!(cases("values.txt"));
	assert!(all.len() > 100, "the value corpus shrank to {}", all.len());
	let mut failures = Vec::new();
	let mut pending = Vec::new();
	for c in &all {
		if let Some(d) = &c.dep {
			if !available(d) {
				pending.push(fmt!("[{}] {}", d, c.code));
				continue;
			}
		}
		let (ok, expected) = res!(oracle(&c.code));
		if !ok {
			failures.push(fmt!("{}\n  oracle failed: {}", c.code, expected));
			continue;
		}
		let mut runs = vec![("parsed", evaluate_parsed(&c.code))];
		if !c.tree.trim().is_empty() {
			runs.insert(0, ("tree", res!(evaluate(&c.tree))));
		}
		for (how, (engine, got)) in runs {
			match got {
				Ok(v) => {
					let got = to_json(&v);
					let same = match (parse_json(&got), parse_json(&expected)) {
						(Ok(a), Ok(b))	=> a == b,
						_				=> got == expected,
					};
					if !same {
						failures.push(fmt!("{} ({})\n  typst:     {}\n  austenite: {}", c.code, how, expected, got));
					}
				}
				Err(_) => failures.push(fmt!("{} ({})\n  typst:     {}\n  austenite error: {}",
					c.code, how, expected, first_error(&engine))),
			}
		}
	}
	println!("{} value cases pending on other units:\n{}", pending.len(), pending.join("\n"));
	assert!(failures.is_empty(), "{} of {} value cases differ:\n{}", failures.len(), all.len(), failures.join("\n"));
	Ok(())
}

#[test]
fn errors_match_the_typst_oracle() -> Outcome<()> {
	if skip() {
		println!("EVAL_ORACLE_SKIP=1: oracle comparison skipped");
		return Ok(());
	}
	let all = res!(cases("errors.txt"));
	assert!(all.len() > 60, "the error corpus shrank to {}", all.len());
	let mut failures = Vec::new();
	let mut pending = Vec::new();
	for c in &all {
		if let Some(d) = &c.dep {
			if !available(d) {
				pending.push(fmt!("[{}] {}", d, c.code));
				continue;
			}
		}
		let (ok, expected) = res!(oracle(&c.code));
		if ok {
			failures.push(fmt!("{}\n  oracle succeeded with {}; not an error case", c.code, expected));
			continue;
		}
		let mut runs = vec![("parsed", evaluate_parsed(&c.code))];
		if !c.tree.trim().is_empty() {
			runs.insert(0, ("tree", res!(evaluate(&c.tree))));
		}
		for (how, (engine, got)) in runs {
			match got {
				Ok(v)	=> failures.push(fmt!("{} ({})\n  typst:     error: {}\n  austenite: {}",
					c.code, how, expected, to_json(&v))),
				Err(_)	=> {
					let msg = first_error(&engine);
					if msg != expected {
						failures.push(fmt!("{} ({})\n  typst:     error: {}\n  austenite: error: {}",
							c.code, how, expected, msg));
					}
				}
			}
		}
	}
	println!("{} error cases pending on other units:\n{}", pending.len(), pending.join("\n"));
	assert!(failures.is_empty(), "{} of {} error cases differ:\n{}", failures.len(), all.len(), failures.join("\n"));
	Ok(())
}

#[test]
fn every_error_is_positioned_in_the_source() -> Outcome<()> {
	for c in res!(cases("errors.txt")) {
		if c.dep.as_deref().map(|d| !available(d)).unwrap_or(false) || c.tree.trim().is_empty() {
			continue;
		}
		let (engine, got) = res!(evaluate(&c.tree));
		assert!(got.is_err(), "{} evaluated without error", c.code);
		let d = engine.diags.iter().find(|d| d.is_error());
		assert!(d.map(|d| !d.span.is_detached()).unwrap_or(false), "{}: error without a position", c.code);
	}
	Ok(())
}

// Warnings

/// One warning as both sides report it: message, 1-based line and column, hints.
type Warning = (String, usize, usize, Vec<String>);

fn warning_cases() -> Outcome<Vec<String>> {
	let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval/core/warnings.txt");
	let text = res!(std::fs::read_to_string(&path).map_err(|e| err!(
		"Could not read {}: {}", path.display(), e; IO, File, Read)));
	let mut out: Vec<String> = Vec::new();
	for line in text.lines() {
		if line.starts_with(";;") {
			continue;
		}
		if line == "%%" {
			out.push(String::new());
		} else if let Some(doc) = out.last_mut() {
			doc.push_str(line);
			doc.push('\n');
		}
	}
	Ok(out)
}

fn work_dir() -> Outcome<PathBuf> {
	let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("eval_core_warnings");
	res!(std::fs::create_dir_all(&d).map_err(|e| err!("Cannot create {}: {}", d.display(), e; IO, File, Write)));
	Ok(d)
}

// `typst compile`'s warnings. Its location line gives a 0-based character column.
fn oracle_warnings(doc: &str, n: usize) -> Outcome<Vec<Warning>> {
	let dir = res!(work_dir());
	let src = dir.join(fmt!("w{}.typ", n));
	res!(std::fs::write(&src, doc).map_err(|e| err!("Cannot write {}: {}", src.display(), e; IO, File, Write)));
	let pdf = dir.join(fmt!("w{}.pdf", n));
	let out = res!(Command::new("systemd-run")
		.args(["--user", "--scope", "--quiet", "-p", "MemoryMax=3G", "--slice=claude-rc.slice", "typst", "compile"])
		.arg(&src).arg(&pdf).output()
		.map_err(|e| err!("The typst oracle could not run: {}", e; IO, Missing)));
	let stderr = String::from_utf8_lossy(&out.stderr).to_string();
	if !out.status.success() {
		return Err(err!("typst rejects warning case {}: {}", n, stderr; Input, Invalid));
	}
	let mut ws: Vec<Warning> = Vec::new();
	let mut lines = stderr.lines().peekable();
	while let Some(l) = lines.next() {
		if let Some(msg) = l.strip_prefix("warning: ") {
			let mut w = (msg.to_string(), 0, 0, Vec::new());
			if let Some(loc) = lines.peek().and_then(|n| n.trim().strip_prefix("┌─ ")) {
				let mut parts = loc.rsplitn(3, ':');
				let col = parts.next().and_then(|c| c.parse::<usize>().ok()).unwrap_or(0);
				let line = parts.next().and_then(|c| c.parse::<usize>().ok()).unwrap_or(0);
				w.1 = line;
				w.2 = col + 1;
			}
			ws.push(w);
		} else if let Some(h) = l.trim().strip_prefix("= hint: ") {
			if let Some(w) = ws.last_mut() {
				w.3.push(h.to_string());
			}
		}
	}
	Ok(ws)
}

fn austenite_warnings(doc: &str) -> Outcome<Vec<Warning>> {
	let mut world = World::new(PathBuf::from("/"));
	let id = res!(world.add_source(PathBuf::from("/case.typ"), doc.to_string()));
	let mut engine = Engine::new(world);
	if let Err(e) = eval_source(&mut engine, id) {
		return Err(err!("evaluation failed: {} ({:?})", first_error(&engine), e; Invalid));
	}
	let src = match engine.world.source(id) {
		Some(s)	=> s,
		None	=> return Err(err!("the case's source vanished"; Bug)),
	};
	Ok(engine.diags.iter().filter(|d| !d.is_error()).map(|d| {
		let (l, c) = src.line_col(d.span.start);
		(d.message.clone(), l, c, d.hints.clone())
	}).collect())
}

#[test]
fn warnings_match_the_typst_oracle() -> Outcome<()> {
	if skip() {
		println!("EVAL_ORACLE_SKIP=1: oracle comparison skipped");
		return Ok(());
	}
	let docs = res!(warning_cases());
	assert!(docs.len() >= 10, "the warning corpus shrank to {}", docs.len());
	let mut warned = 0;
	let mut failures = Vec::new();
	for (n, doc) in docs.iter().enumerate() {
		let want = res!(oracle_warnings(doc, n));
		warned += want.len();
		match austenite_warnings(doc) {
			Ok(got) if got == want	=> (),
			Ok(got)					=> failures.push(fmt!("{}\n  typst:     {:?}\n  austenite: {:?}", doc, want, got)),
			Err(e)					=> failures.push(fmt!("{}\n  austenite: {}", doc, e)),
		}
	}
	// The corpus must hold warnings to compare, or agreement means nothing.
	assert!(warned >= 5, "only {} warnings in the corpus", warned);
	assert!(failures.is_empty(), "{} of {} warning cases differ:\n{}", failures.len(), docs.len(), failures.join("\n"));
	Ok(())
}

// Every diagnostic carries the kind of what it reports, set where it is raised, so a caller switches on
// the kind and never on the wording. Each row is code, the kind of its first diagnostic, and whether that
// diagnostic is an error.
#[test]
fn diagnostics_carry_the_kind_of_what_they_report() -> Outcome<()> {
	use DiagnosticKind as K;
	let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("eval_core_kinds");
	res!(std::fs::create_dir_all(&dir).map_err(|e| err!("Cannot create {}: {}", dir.display(), e; IO, File, Write)));
	// A file at a path needs a file to resolve it from, so each case is evaluated from a source in `dir`, as
	// the string handed to `eval` is in Typst. 0xff is not UTF-8 in any position.
	res!(std::fs::write(dir.join("bad.txt"), [255u8, 254, 65]).map_err(|e| err!(
		"Cannot write bad.txt in {}: {}", dir.display(), e; IO, File, Write)));
	let cases: [(&str, K, bool); 18] = [
		("missing",							K::UnknownVariable,	true),
		("(1 +",							K::Syntax,			true),
		("break",							K::Syntax,			true),
		("1 + \"a\"",						K::Type,			true),
		("int(\"x\")",					K::Type,			true),
		("{ let f() = f(); f() }",			K::Limit,			true),
		("plugin(\"x.wasm\")",			K::Unsupported,		true),
		("json(bytes((255, 254, 65)))",	K::Encoding,		true),
		("read(\"/absent.txt\")",		K::MissingFile,		true),
		("read(\"/bad.txt\")",			K::Encoding,		true),
		("{ let f() = { [a]; return 1 }; f() }",	K::Lint,		false),
		("import heading as heading",		K::Lint,		false),
		("import list: item as item",		K::Lint,		false),
		("[<x>]",							K::Lint,		false),
		("[a <x> <y>]",					K::Lint,		false),
		("numbering(\"a\", 0)",			K::Lint,		false),
		("enum(([1], [a]))",				K::Lint,		false),
		("terms(([a], [b]))",				K::Lint,		false),
	];
	let mut failures = Vec::new();
	for (code, kind, error) in cases {
		let mut engine = Engine::new(World::new(dir.clone()));
		let id = res!(engine.world.add_source(dir.join("case.typ"), String::new()));
		let _ = eval_string(&mut engine, code, EvalMode::Code, Scope::new(), Span::new(id, 0, 0));
		match engine.diags.iter().find(|d| d.is_error() == error) {
			Some(d) if d.kind == kind	=> (),
			Some(d)						=> failures.push(fmt!("{}: kind {} ({}), wanted {}", code, d.kind, d.message, kind)),
			None						=> failures.push(fmt!("{}: no {} raised", code, if error { "error" } else { "warning" })),
		}
	}
	assert!(failures.is_empty(), "{} case(s) carry the wrong kind:\n{}", failures.len(), failures.join("\n"));
	Ok(())
}
