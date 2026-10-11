//! Ported from hayagriva 0.10.1 (`src/types/page.rs`) and citationberg 0.7.0
//! (`PageRangeFormat::format`), Copyright (c) Martin Haug and the hayagriva contributors,
//! MIT OR Apache-2.0.
use crate::csl::{
	item::Numeric,
	style::PageRange,
};

use std::fmt::Write;


#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PagePart {
	Ampersand,
	Comma,
	Escaped(Numeric, Numeric),	// `12\-15`, always written expanded
	Single(Numeric),
	Range(Numeric, Numeric),
}

/// A `page` value that parsed as pages, ranges and the `,` and `&` between them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageRanges(pub Vec<PagePart>);

/// The `page` variable: typed when it parsed, otherwise the string as given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageVal {
	Typed(PageRanges),
	Str(String),
}

impl PageVal {
	/// Reads a page value as hayagriva's `MaybeTyped::infallible_from_str` does.
	pub fn from_str(s: &str) -> Self {
		match PageRanges::parse(s) {
			Some(p)	=> Self::Typed(p),
			None	=> Self::Str(s.to_string()),
		}
	}

	/// Is it plural for a label? A string never is (hayagriva).
	pub fn is_plural(&self) -> bool {
		match self {
			Self::Typed(p)	=> p.is_plural(),
			Self::Str(_)	=> false,
		}
	}
}

impl PageRanges {

	/// Parses runs split at `,` and `&` (each its own part), then each run as a page, `a-b`,
	/// `a–b` or the escaped `a\-b`. `None` when any run fails.
	pub fn parse(s: &str) -> Option<Self> {
		let mut parts = Vec::new();
		let mut run = String::new();
		let flush = |run: &mut String, parts: &mut Vec<PagePart>| -> bool {
			if run.is_empty() {
				return true;
			}
			let r = PagePart::parse(run);
			run.clear();
			match r {
				Some(p)	=> { parts.push(p); true }
				None	=> false,
			}
		};
		for c in s.chars() {
			if c == ',' || c == '&' {
				if !flush(&mut run, &mut parts) {
					return None;
				}
				parts.push(if c == ',' { PagePart::Comma } else { PagePart::Ampersand });
			} else {
				run.push(c);
			}
		}
		if !flush(&mut run, &mut parts) {
			return None;
		}
		Some(Self(parts))
	}

	/// Is there more than one page? A range of distinct ends counts at once.
	pub fn is_plural(&self) -> bool {
		let mut count = 0;
		for p in &self.0 {
			match p {
				PagePart::Single(_) => count += 1,
				PagePart::Range(s, e) | PagePart::Escaped(s, e) => {
					if s != e {
						return true;
					}
					count += 1;
				}
				_ => {}
			}
		}
		count > 1
	}

	/// Writes the ranges in a `page-range-format` (hayagriva's default is expanded), the range
	/// delimiter being the locale's `page-range-delimiter` term or an en dash.
	pub fn write<W: Write>(&self, w: &mut W, fmt: Option<PageRange>, delim: Option<&str>) {
		let fmt = fmt.unwrap_or(PageRange::Expanded);
		for p in &self.0 {
			match p {
				PagePart::Ampersand	=> { let _ = w.write_str(" & "); }
				PagePart::Comma		=> { let _ = w.write_str(", "); }
				PagePart::Escaped(s, e)	=> format_range(w, PageRange::Expanded, &s.to_string(), &e.to_string(), delim),
				PagePart::Single(s)	=> { let _ = w.write_str(&s.to_string()); }
				PagePart::Range(s, e)	=> format_range(w, fmt, &s.to_string(), &e.to_string(), delim),
			}
		}
	}
}

impl PagePart {
	fn parse(s: &str) -> Option<Self> {
		let s = s.trim();
		if s.is_empty() {
			return None;
		}
		if s.contains("\\-") {
			let mut it = s.split("\\-").map(str::trim);
			let a = Numeric::parse(it.next().unwrap_or(""));
			let b = Numeric::parse(it.next().unwrap_or(""));
			if it.next().is_some() {
				return None;
			}
			return match (a, b) {
				(Some(a), Some(b))	=> Some(Self::Escaped(a, b)),
				_			=> None,
			};
		}
		let mut it = s.split(['-', '–']).map(str::trim);
		let a = it.next().and_then(Numeric::parse);
		let r = match (a, it.next()) {
			(Some(a), None)		=> Self::Single(a),
			(Some(a), Some(b))	=> match Numeric::parse(b) {
				Some(b)	=> Self::Range(a, b),
				None	=> return None,
			},
			(None, _)		=> return None,
		};
		if it.next().is_some() {
			return None;
		}
		Some(r)
	}
}

/// citationberg's `PageRangeFormat::format`. `chicago` is `chicago-15`, as in citationberg.
pub fn format_range<W: Write>(w: &mut W, fmt: PageRange, start: &str, end: &str, delim: Option<&str>) {
	let sep = delim.unwrap_or("–");
	let start = start.trim();
	let end = end.trim();
	let (spre, x) = split_digit_suffix(start);
	let (epre, y) = split_digit_suffix(end);
	if spre != epre {
		let _ = write!(w, "{}{}{}", start, sep, end);
		return;
	}
	let (xl, yl) = (x.len(), y.len());
	// A shorter end is shorthand (`101-7`): its missing digits come from the start.
	let y = if xl <= yl { y.to_string() } else { format!("{}{}", &x[..xl - yl], y) };
	let _ = write!(w, "{}{}{}", spre, x, sep);
	let chicago = matches!(fmt, PageRange::Chicago | PageRange::Chicago15 | PageRange::Chicago16);
	let c15 = matches!(fmt, PageRange::Chicago | PageRange::Chicago15);
	match fmt {
		_ if chicago && (xl < 3 || x.ends_with("00"))			=> { let _ = w.write_str(&y); }
		_ if chicago && x[xl - 2..].starts_with('0')			=> minimal(w, 1, x, &y),
		_ if c15 && xl == 4 && changed_digits(x, &y) >= 3		=> { let _ = w.write_str(&y); }
		PageRange::Chicago | PageRange::Chicago15 | PageRange::Chicago16	=> minimal(w, 2, x, &y),
		// Only expanded repeats a shared prefix on the end: `S1–S12`, where the others give `S1–12`.
		PageRange::Expanded	=> { let _ = write!(w, "{}{}", spre, y); }
		PageRange::Minimal	=> minimal(w, 1, x, &y),
		PageRange::MinimalTwo	=> minimal(w, 2, x, &y),
	}
}

fn changed_digits(x: &str, y: &str) -> usize {
	let x = if x.len() < y.len() { format!("{}{}", " ".repeat(y.len() - x.len()), x) } else { x.to_string() };
	for (i, (c, d)) in x.chars().rev().zip(y.chars().rev()).enumerate() {
		if c == d {
			return i;
		}
	}
	x.len()
}

fn minimal<W: Write>(w: &mut W, thresh: usize, x: &str, y: &str) {
	if y.len() > x.len() {
		let _ = w.write_str(y);
		return;
	}
	let ys: String = x.chars().zip(y.chars()).skip_while(|(c, d)| c == d).map(|(_, d)| d).collect();
	if ys.len() < thresh && y.len() >= thresh {
		let _ = w.write_str(&y[y.len() - thresh..]);
	} else {
		let _ = w.write_str(&ys);
	}
}

fn split_digit_suffix(s: &str) -> (&str, &str) {
	let n = s.chars().rev().take_while(|c| c.is_ascii_digit()).count();
	s.split_at(s.len() - n)
}
