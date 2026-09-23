// U3 owns this file. Methods on `str` (and `str.to-unicode`/`str.from-unicode`). Indices are byte
// offsets and `first`/`last`/`at`/`rev` work in grapheme clusters, as in Typst. Regex matching walks
// `fe2o3_text::regex`'s iterators once over the whole text, so `^` and `\b` see the true context and a
// long text is not searched afresh from every match.

use crate::eval::args::Args;
use crate::eval::lib::foundations::{
	finish,
	int_of,
	locate_bound,
	mismatch,
	need,
	receiver,
	slice_bounds,
	str_of,
};
use crate::eval::scope::Scope;
use crate::eval::value::{
	Alignment,
	Dict,
	HAlign,
	RegexValue,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::unicode::norm;
use oxedyne_fe2o3_text::unicode::segment;

use std::sync::Arc;

native_fns! {
	pub enum StrFn {
		Len				=> "len",
		First			=> "first",
		Last			=> "last",
		At				=> "at",
		Slice			=> "slice",
		Clusters		=> "clusters",
		Codepoints		=> "codepoints",
		Contains		=> "contains",
		StartsWith		=> "starts-with",
		EndsWith		=> "ends-with",
		Find			=> "find",
		Position		=> "position",
		Match			=> "match",
		Matches			=> "matches",
		Replace			=> "replace",
		Rev				=> "rev",
		Split			=> "split",
		Trim			=> "trim",
		ToUnicode		=> "to-unicode",
		FromUnicode		=> "from-unicode",
		Normalize		=> "normalize",
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn method(name: &str) -> Option<StrFn> {
	StrFn::ALL.iter().copied().find(|f| f.name() == name
		&& !matches!(f, StrFn::ToUnicode | StrFn::FromUnicode))
}

/// `str.to-unicode` and `str.from-unicode`, reached through the type rather than a receiver.
pub fn static_fn(name: &str) -> Option<StrFn> {
	match name {
		"to-unicode"	=> Some(StrFn::ToUnicode),
		"from-unicode"	=> Some(StrFn::FromUnicode),
		_				=> None,
	}
}

// A pattern argument: a string matched literally or a regex.
enum Pattern {
	Str(Arc<String>),
	Regex(Arc<RegexValue>),
}

fn pattern(engine: &mut Engine, span: Span, v: Value) -> Outcome<Pattern> {
	match v {
		Value::Str(s)	=> Ok(Pattern::Str(s)),
		Value::Regex(r)	=> Ok(Pattern::Regex(r)),
		other			=> Err(mismatch(engine, span, "string or regular expression", &other)),
	}
}

/// A match: its byte range and, for a regex, its groups' texts (`none` for a group that took no part).
struct Hit {
	a:		usize,
	b:		usize,
	caps:	Vec<Value>,
}

fn regex_error(engine: &mut Engine, span: Span, e: Error<ErrTag>) -> Error<ErrTag> {
	engine.error(span, fmt!("regex search failed: {}", e.msgs().last().cloned().unwrap_or_default()))
}

fn hit(c: &oxedyne_fe2o3_text::regex::Captures) -> Hit {
	let w = c.whole();
	let caps = (1..c.len()).map(|i| match c.text(i) {
		Some(t)	=> Value::str(t),
		None	=> Value::None,
	}).collect();
	Hit { a: w.start, b: w.end, caps }
}

/// The first `limit` non-overlapping matches, left to right, by the `regex` crate's iteration rule (an
/// empty match directly after the previous match is passed over). One pass over the text.
fn find_all(engine: &mut Engine, span: Span, pat: &Pattern, hay: &str, limit: usize) -> Outcome<Vec<Hit>> {
	let mut out = Vec::new();
	match pat {
		Pattern::Str(p) => for (i, m) in hay.match_indices(p.as_str()).take(limit) {
			out.push(Hit { a: i, b: i + m.len(), caps: Vec::new() });
		},
		Pattern::Regex(re) => for c in re.re.captures_iter(hay).take(limit) {
			match c {
				Ok(c)	=> out.push(hit(&c)),
				Err(e)	=> return Err(regex_error(engine, span, e)),
			}
		},
	}
	Ok(out)
}

fn first_match(engine: &mut Engine, span: Span, pat: &Pattern, hay: &str) -> Outcome<Option<Hit>> {
	match pat {
		Pattern::Str(p)		=> Ok(hay.find(p.as_str()).map(|i| Hit { a: i, b: i + p.len(), caps: Vec::new() })),
		Pattern::Regex(re)	=> match re.re.captures(hay) {
			Ok(c)	=> Ok(c.map(|c| hit(&c))),
			Err(e)	=> Err(regex_error(engine, span, e)),
		},
	}
}

fn match_dict(hay: &str, h: Hit) -> Value {
	let mut d = Dict::new();
	d.insert("start", Value::Int(h.a as i64));
	d.insert("end", Value::Int(h.b as i64));
	d.insert("text", Value::str(hay.get(h.a..h.b).unwrap_or("")));
	d.insert("captures", Value::array(h.caps));
	Value::dict(d)
}

fn text_recv(engine: &mut Engine, args: &mut Args) -> Outcome<Arc<String>> {
	let v = res!(receiver(args));
	str_of(engine, args.span, v)
}

pub fn call(f: StrFn, engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	if let StrFn::FromUnicode = f {
		let v = res!(need(engine, &mut args, "value"));
		let i = res!(int_of(engine, span, v));
		res!(finish(engine, args));
		if i < 0 {
			return Err(engine.error(span, "number must be at least zero"));
		}
		return match u32::try_from(i).ok().and_then(char::from_u32) {
			Some(c)	=> Ok(Value::str(c.to_string())),
			None	=> Err(engine.error(span, fmt!("{:#x} is not a valid codepoint", i))),
		};
	}
	if let StrFn::ToUnicode = f {
		let s = res!(text_recv(engine, &mut args));
		res!(finish(engine, args));
		let mut cs = s.chars();
		return match (cs.next(), cs.next()) {
			(Some(c), None)	=> Ok(Value::Int(c as i64)),
			_				=> Err(engine.error(span, "expected exactly one character")),
		};
	}
	let s = res!(text_recv(engine, &mut args));
	let out = match f {
		StrFn::Len => Value::Int(s.len() as i64),
		StrFn::First | StrFn::Last => {
			let default = res!(args.named::<Value>("default"));
			let gs = segment::graphemes(&s);
			let g = if f == StrFn::First { gs.first() } else { gs.last() };
			match (g, default) {
				(Some(g), _)		=> Value::str(*g),
				(None, Some(d))		=> d,
				(None, None)		=> return Err(engine.error(span, "string is empty")),
			}
		}
		StrFn::At => {
			let i = res!(need(engine, &mut args, "index"));
			let i = res!(int_of(engine, span, i));
			let default = res!(args.named::<Value>("default"));
			let at = res!(locate_bound(engine, span, i, s.len(), &|k| s.is_char_boundary(k)));
			let g = at.and_then(|k| segment::graphemes(&s[k..]).first().map(|g| g.to_string()));
			match (g, default) {
				(Some(g), _)	=> Value::str(g),
				(None, Some(d))	=> d,
				(None, None)	=> return Err(engine.error(span, fmt!(
					"no default value was specified and string index out of bounds (index: {}, len: {})",
					i, s.len()))),
			}
		}
		StrFn::Slice => {
			let (a, b) = res!(slice_bounds(engine, &mut args, s.len(), "string", |k| s.is_char_boundary(k)));
			Value::str(&s[a..b])
		}
		StrFn::Clusters => Value::array(segment::graphemes(&s).into_iter().map(Value::str).collect()),
		StrFn::Codepoints => Value::array(s.chars().map(|c| Value::str(c.to_string())).collect()),
		StrFn::Contains | StrFn::StartsWith | StrFn::EndsWith | StrFn::Find | StrFn::Position | StrFn::Match => {
			let p = res!(need(engine, &mut args, "pattern"));
			let p = res!(pattern(engine, span, p));
			match f {
				StrFn::Contains		=> Value::Bool(res!(first_match(engine, span, &p, &s)).is_some()),
				StrFn::StartsWith	=> Value::Bool(match &p {
					Pattern::Str(q)		=> s.starts_with(q.as_str()),
					Pattern::Regex(_)	=> matches!(res!(first_match(engine, span, &p, &s)), Some(Hit { a: 0, .. })),
				}),
				// A regex ends the string when a search from some position finds first a match ending
				// there, as Typst restarts its search one character past each match's start.
				StrFn::EndsWith		=> Value::Bool(match &p {
					Pattern::Str(q)		=> s.ends_with(q.as_str()),
					Pattern::Regex(re)	=> {
						let mut found = false;
						for m in re.re.overlapping_iter(&s) {
							match m {
								Ok(m) if m.end == s.len()	=> { found = true; break; }
								Ok(_)						=> (),
								Err(e)						=> return Err(regex_error(engine, span, e)),
							}
						}
						found
					}
				}),
				StrFn::Find			=> match res!(first_match(engine, span, &p, &s)) {
					Some(h)	=> Value::str(&s[h.a..h.b]),
					None	=> Value::None,
				},
				StrFn::Position		=> match res!(first_match(engine, span, &p, &s)) {
					Some(h)	=> Value::Int(h.a as i64),
					None	=> Value::None,
				},
				_					=> match res!(first_match(engine, span, &p, &s)) {
					Some(m)	=> match_dict(&s, m),
					None	=> Value::None,
				},
			}
		}
		StrFn::Matches => {
			let p = res!(need(engine, &mut args, "pattern"));
			let p = res!(pattern(engine, span, p));
			let ms = res!(find_all(engine, span, &p, &s, usize::MAX));
			Value::array(ms.into_iter().map(|m| match_dict(&s, m)).collect())
		}
		StrFn::Replace => {
			let p = res!(need(engine, &mut args, "pattern"));
			let p = res!(pattern(engine, span, p));
			let with = res!(need(engine, &mut args, "replacement"));
			let count = match res!(args.named::<Value>("count")) {
				None	=> usize::MAX,
				Some(v)	=> {
					let c = res!(int_of(engine, span, v));
					if c < 0 {
						return Err(engine.error(span, "number must be at least zero"));
					}
					c as usize
				}
			};
			res!(finish(engine, args));
			return replace(engine, span, &s, &p, with, count);
		}
		StrFn::Rev => {
			let mut gs = segment::graphemes(&s);
			gs.reverse();
			Value::str(gs.concat())
		}
		StrFn::Split => {
			let p = res!(args.eat::<Value>());
			match p {
				None | Some(Value::None) => Value::array(s.split_whitespace().map(Value::str).collect()),
				Some(v) => {
					let p = res!(pattern(engine, span, v));
					match &p {
						Pattern::Str(q) => Value::array(s.split(q.as_str()).map(Value::str).collect()),
						Pattern::Regex(_) => {
							let ms = res!(find_all(engine, span, &p, &s, usize::MAX));
							let mut out = Vec::with_capacity(ms.len() + 1);
							let mut last = 0;
							for h in ms {
								out.push(Value::str(&s[last..h.a]));
								last = h.b;
							}
							out.push(Value::str(&s[last..]));
							Value::array(out)
						}
					}
				}
			}
		}
		StrFn::Trim => {
			let p = match res!(args.eat::<Value>()) {
				None | Some(Value::None)	=> None,
				Some(v)						=> Some(res!(pattern(engine, span, v))),
			};
			let at = match res!(args.named::<Value>("at")) {
				None | Some(Value::None) => None,
				Some(Value::Alignment(Alignment { x: Some(HAlign::Start), y: None })) => Some(true),
				Some(Value::Alignment(Alignment { x: Some(HAlign::End), y: None })) => Some(false),
				Some(_) => return Err(engine.error(span, "expected either `start` or `end`")),
			};
			let repeat = match res!(args.named::<Value>("repeat")) {
				None				=> true,
				Some(Value::Bool(b))	=> b,
				Some(other)			=> return Err(mismatch(engine, span, "boolean", &other)),
			};
			Value::str(res!(trim(engine, span, &s, p, at, repeat)))
		}
		StrFn::Normalize => {
			let form = match res!(args.named::<Value>("form")) {
				None => norm::Form::Nfc,
				Some(Value::Str(x)) => match x.as_str() {
					"nfc"	=> norm::Form::Nfc,
					"nfd"	=> norm::Form::Nfd,
					"nfkc"	=> norm::Form::Nfkc,
					"nfkd"	=> norm::Form::Nfkd,
					_		=> return Err(engine.error(span,
						"expected \"nfc\", \"nfd\", \"nfkc\", or \"nfkd\"")),
				},
				Some(other) => return Err(mismatch(engine, span, "string", &other)),
			};
			Value::str(norm::normalise(&s, form))
		}
		StrFn::ToUnicode | StrFn::FromUnicode => Value::None,
	};
	res!(finish(engine, args));
	Ok(out)
}

fn replace(engine: &mut Engine, span: Span, s: &str, p: &Pattern, with: Value, count: usize) -> Outcome<Value> {
	let ms = res!(find_all(engine, span, p, s, count));
	let mut out = String::with_capacity(s.len());
	let mut last = 0;
	for h in ms {
		let (a, b) = (h.a, h.b);
		out.push_str(&s[last..a]);
		match &with {
			Value::Str(r)	=> out.push_str(r),
			Value::Func(_) | Value::Type(_)	=> {
				let f = res!(crate::eval::lib::array::func_of(engine, span, with.clone()));
				let f = &f;
				let mut fa = Args::new(span);
				fa.push(span, match_dict(s, h));
				let r = res!(engine.call_func(f, fa));
				match r {
					Value::Str(r)	=> out.push_str(&r),
					other			=> return Err(mismatch(engine, span, "string", &other)),
				}
			}
			other => return Err(mismatch(engine, span, "string or function", other)),
		}
		last = b;
	}
	out.push_str(&s[last..]);
	Ok(Value::str(out))
}

fn trim(engine: &mut Engine, span: Span, s: &str, p: Option<Pattern>, at: Option<bool>, repeat: bool) -> Outcome<String> {
	let mut start = at != Some(false);
	let end = at != Some(true);
	let r = match p {
		None => match at {
			None		=> s.trim(),
			Some(true)	=> s.trim_start(),
			Some(false)	=> s.trim_end(),
		},
		Some(Pattern::Str(q)) => {
			let q = q.as_str();
			let mut t = s;
			if repeat {
				if start { t = t.trim_start_matches(q); }
				if end { t = t.trim_end_matches(q); }
			} else {
				if start { t = t.strip_prefix(q).unwrap_or(t); }
				if end { t = t.strip_suffix(q).unwrap_or(t); }
			}
			t
		}
		Some(p @ Pattern::Regex(_)) => {
			let ms = res!(find_all(engine, span, &p, s, usize::MAX));
			let mut last: Option<usize> = None;
			let (mut lo, mut hi) = (0, s.len());
			for Hit { a, b, .. } in ms {
				let consecutive = last == Some(a);
				start &= a == 0 || consecutive;
				if start {
					lo = b;
					start &= repeat;
				}
				if end && (!consecutive || !repeat) {
					hi = a;
				}
				last = Some(b);
			}
			if last.map(|l| l < s.len()).unwrap_or(false) {
				hi = s.len();
			}
			&s[lo..lo.max(hi)]
		}
	};
	Ok(r.to_string())
}
