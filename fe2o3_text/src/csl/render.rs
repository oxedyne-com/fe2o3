//! Ported from hayagriva 0.10.1, Copyright (c) Martin Haug and the hayagriva contributors,
//! MIT OR Apache-2.0 (`src/csl/mod.rs`, the writing context; `src/csl/rendering/mod.rs` and
//! `rendering/names.rs`, the element renderers; citationberg 0.7.0 `OrdinalLookup`, same
//! licence).
//!
//! Hooks B2b fills: non-empty names (`render_name_list`), dates (`render_date`), typed page ranges
//! with `page-range-format` and their label plural (`page_plural`, `is_numeric_page`). B3 adds the
//! special forms, sorting, the citation layout and second-field-align.
use crate::csl::{
	case::{
		is_language_dependent,
		Case,
		CaseFolder,
	},
	item::{
		is_number_of,
		var_kind,
		Chunked,
		Item,
		NumDelim,
		NumVal,
		Numeric,
		VarKind,
	},
	locale::{
		Chain,
		Gender,
		OrdMatch,
		Term,
	},
	node::{
		self,
		Elem as NodeElem,
		Formatting,
		Meta,
		Node,
	},
	style::{
		Choose,
		Cond,
		Date,
		Display,
		Elem,
		Fmt,
		Group,
		Label,
		Match,
		Names,
		NumForm,
		Number,
		Plural,
		Position,
		Style,
		Test,
		Text,
		TextCase,
		TextSrc,
		TermForm,
		VarForm,
	},
};

use std::fmt::Write;


#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ibid { #[default] No, Ibid, WithLocator }

/// What the citation, rather than the entry, supplies: its number, label and locator, and the
/// position tests' answers.
#[derive(Clone, Debug, Default)]
pub struct Cite {
	pub number:		Option<usize>,
	pub label:		Option<String>,
	pub locator:		Option<(String, NumVal)>,	// kind (`page`, `custom`) and value
	pub first_note:		Option<usize>,			// first-reference-note-number
	pub disambiguate:	bool,
	pub first:		bool,
	pub ibid:		Ibid,
	pub near_note:		bool,
}

// Usage
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Usage {
	vars:		bool,
	non_empty:	bool,
	macros:		bool,
	group:		bool,
}

impl Usage {
	fn merge(self, c: Self) -> Self {
		Self {
			vars:		self.vars || c.vars,
			non_empty:	self.non_empty || c.non_empty,
			macros:		self.macros || c.macros,
			group:		self.group || c.group,
		}
	}

	/// Does a group with this usage print? Only when it queried no variable, or one of them was
	/// non-empty (directly, through a macro, or through a nested group).
	fn should_render_group(&self) -> bool {
		!self.vars || self.non_empty || self.macros || self.group
	}
}

// Writer
#[derive(Clone, Copy, Debug)]
struct Loc {
	elem:	usize,
	fmt:	usize,
}

#[derive(Clone, Copy, Debug)]
struct Affix {
	loc:	Loc,
	plen:	usize,	// prefix length in bytes
}

/// hayagriva's `WritingContext`: stacks of formatting, case and delimiter over a case-folding
/// buffer, and a stack of element levels. Each stack's base is held apart so it is never popped.
#[derive(Debug, Default)]
struct Writer {
	pull:		bool,
	inner_quotes:	bool,
	strip_periods:	bool,
	suppressing:	bool,
	suppressed:	Vec<String>,
	checked_disamb:	bool,
	fmts:		Vec<Formatting>,
	cases:		Vec<Option<TextCase>>,
	delims:		Vec<Option<String>>,
	buf:		CaseFolder,
	base:		Vec<Node>,
	stack:		Vec<Vec<Node>>,
}

impl Writer {

	fn fmt(&self) -> Formatting {
		self.fmts.last().copied().unwrap_or_default()
	}

	fn top(&self) -> &Vec<Node> {
		match self.stack.last() {
			Some(l)	=> l,
			None	=> &self.base,
		}
	}

	fn top_mut(&mut self) -> &mut Vec<Node> {
		match self.stack.last_mut() {
			Some(l)	=> l,
			None	=> &mut self.base,
		}
	}

	/// The topmost level that is not empty, as hayagriva's `last_mut_predicate`.
	fn top_non_empty_mut(&mut self) -> Option<&mut Vec<Node>> {
		match self.stack.iter().rposition(|l| !node::is_empty(l)) {
			Some(i)				=> self.stack.get_mut(i),
			None if !node::is_empty(&self.base)	=> Some(&mut self.base),
			None				=> None,
		}
	}

	fn push_format(&mut self, f: &Fmt) -> usize {
		if f.is_plain() {
			return self.fmts.len();
		}
		self.save();
		let pos = self.fmts.len();
		let top = self.fmt().apply(f);
		self.fmts.push(top);
		pos
	}

	fn pop_format(&mut self, pos: usize) {
		if pos == self.fmts.len() {
			return;
		}
		self.save();
		self.fmts.truncate(pos);
	}

	/// Moves the buffer into the top level, merging with a last text run of equal formatting. The
	/// buffer is replaced by a fresh folder with no case, which `reconfigure` later corrects.
	fn save(&mut self) {
		if self.buf.is_empty() {
			return;
		}
		let fmt = self.fmt();
		let text = std::mem::take(&mut self.buf).finish();
		let top = self.top_mut();
		if let Some(Node::Text(t, f)) = top.last_mut() {
			if *f == fmt {
				t.push_str(&text);
				return;
			}
		}
		top.push(Node::Text(text, fmt));
	}

	fn push_elem(&mut self, f: &Fmt) -> Loc {
		self.save();
		let elem = self.stack.len();
		self.stack.push(Vec::new());
		Loc { elem, fmt: self.push_format(f) }
	}

	fn discard_elem(&mut self, loc: Loc) {
		self.pop_format(loc.fmt);
		self.save();
		self.stack.truncate(loc.elem);
	}

	/// Closes a level, wrapping it in an element only when it carries display or meta, and
	/// otherwise splicing its children into the level below.
	fn commit_elem(&mut self, loc: Loc, display: Option<Display>, meta: Option<Meta>) {
		self.pop_format(loc.fmt);
		self.save();
		let children: Vec<Node> = self.stack.split_off(loc.elem).into_iter().flatten().collect();
		if display.is_some() || meta.is_some() {
			self.top_mut().push(Node::Elem(NodeElem { children, display, meta }));
		} else {
			self.top_mut().extend(children);
		}
	}

	/// Pushes a plain level and writes the prefix straight into the buffer, uncased and
	/// unchecked.
	fn apply_prefix(&mut self, prefix: Option<&str>) -> Affix {
		let loc = self.push_elem(&Fmt::default());
		if let Some(p) = prefix {
			self.buf.push_str(p);
		}
		Affix { loc, plen: prefix.map_or(0, str::len) }
	}

	fn has_content_since(&mut self, a: &Affix) -> bool {
		self.save();
		let top = self.top();
		let first = match top.first() {
			Some(Node::Text(t, _))	=> t.get(a.plen..).map_or(false, |r| r.chars().any(|c| !c.is_whitespace())),
			Some(Node::Elem(e))	=> node::has_content(&e.children),
			Some(_)			=> true,
			None			=> false,
		};
		first || top.iter().skip(1).any(Node::has_content)
	}

	fn reconfigure(&mut self) {
		let case = self.cases.last().copied().flatten().map(Case::from).unwrap_or_default();
		self.buf.reconfigure(case);
	}

	fn push_delim(&mut self, d: Option<String>) -> usize {
		let idx = self.delims.len();
		self.delims.push(d);
		idx
	}

	fn pop_delim(&mut self, idx: usize) {
		self.delims.truncate(idx);
	}

	fn delim(&self) -> Option<String> {
		self.delims.last().cloned().flatten()
	}

	fn pop_case(&mut self, idx: usize) {
		self.cases.truncate(idx);
	}

	/// The buffer if it holds text, else the last text of the topmost non-empty level.
	fn last_buffer(&mut self) -> Option<&mut String> {
		if !self.buf.is_empty() {
			return Some(self.buf.as_string_mut());
		}
		self.top_non_empty_mut().and_then(|l| node::last_text_mut(l))
	}

	fn len(&self) -> usize {
		self.buf.len()
			+ self.base.iter().map(Node::str_len).sum::<usize>()
			+ self.stack.iter().flat_map(|l| l.iter().map(Node::str_len)).sum::<usize>()
	}

	fn flush(mut self) -> Vec<Node> {
		self.save();
		let mut base = self.base;
		for l in self.stack {
			base.extend(l);
		}
		base
	}
}

/// The last text of a node, descending only through elements with a single child. Pulling
/// punctuation into a quote uses this rather than `last_text_mut`, so that `“On $x^2$ and other
/// things”,` keeps its comma outside: the quote sits in an element with several children.
fn get_last_text(n: &mut Node) -> Option<&mut String> {
	match n {
		Node::Text(t, _)				=> Some(t),
		Node::Elem(e) if e.children.len() == 1		=> e.children.first_mut().and_then(get_last_text),
		_						=> None,
	}
}

fn is_english_code(code: &str) -> bool {
	code.starts_with("en") && (code.len() == 2 || code.get(2..3) == Some("-"))
}

fn url_prefix(var: &str) -> Option<&'static str> {
	match var {
		"DOI"	=> Some("https://doi.org/"),
		"PMID"	=> Some("https://www.ncbi.nlm.nih.gov/pubmed/"),
		"PMCID"	=> Some("https://www.ncbi.nlm.nih.gov/pmc/articles/"),
		_	=> None,
	}
}

/// Is a page value plural for its label? Hook for B2b, which types page ranges; until then a
/// string that reads as several numbers counts as plural.
fn page_plural(s: &str) -> bool {
	Numeric::parse(s).map_or(false, |n| n.nums.len() > 1)
}

/// Does a page value parse as page ranges? Hook for B2b, approximated by `Numeric`.
fn is_numeric_page(s: &str) -> bool {
	Numeric::parse(s).is_some()
}

fn roman(n: i32) -> Option<String> {
	if !(1..=3999).contains(&n) {
		return None;
	}
	const R: [(i32, &str); 13] = [
		(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"),
		(50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i"),
	];
	let mut n = n;
	let mut s = String::new();
	for (v, r) in R {
		while n >= v {
			s.push_str(r);
			n -= v;
		}
	}
	Some(s)
}

// Ordinals
enum OrdName { Ordinal, N(u8), Long(u8), Other }

fn ord_name(name: &str) -> OrdName {
	if name == "ordinal" {
		OrdName::Ordinal
	} else if let Some(n) = name.strip_prefix("long-ordinal-").and_then(|n| n.parse().ok()) {
		OrdName::Long(n)
	} else if let Some(n) = name.strip_prefix("ordinal-").and_then(|n| n.parse().ok()) {
		OrdName::N(n)
	} else {
		OrdName::Other
	}
}

/// citationberg's `OrdinalLookup`: the ordinal terms of the first locale in the chain that has
/// any.
struct Ords<'a> {
	terms:	Vec<&'a Term>,
	legacy:	bool,	// no `ordinal` term but `ordinal-01` to `-04`: CSL 1.0's scheme
}

impl<'a> Ords<'a> {

	fn new(chain: &Chain<'a>) -> Self {
		let terms = chain.first(|l| {
			let ts: Vec<&'a Term> = l.terms.iter()
				.filter(|t| !matches!(ord_name(&t.name), OrdName::Other))
				.collect();
			if ts.is_empty() { None } else { Some(ts) }
		}).unwrap_or_default();
		let defines = terms.iter().any(|t| matches!(ord_name(&t.name), OrdName::Ordinal));
		let legacy = !defines && (1..=4u8).all(|n| terms.iter().any(|t| matches!(ord_name(&t.name), OrdName::N(o) if o == n)));
		Self { terms, legacy }
	}

	/// The best ordinal suffix: a two-digit term over a one-digit one, then one of the asked
	/// gender, then the nearest number. citationberg compares the term's `gender`, not its
	/// `gender-form`, so `gender-form` never steers the choice.
	fn lookup(&self, n: i32, gender: Option<Gender>) -> Option<&'a str> {
		let mut best: Option<&'a Term> = None;
		for &t in &self.terms {
			let hit = match ord_name(&t.name) {
				OrdName::Ordinal => true,
				OrdName::N(o) if self.legacy => {
					let class = match (n, n % 10) {
						(11..=13, _)		=> 4,
						(_, v @ 1..=3)		=> v as u8,
						_			=> 4,
					};
					o == class
				}
				OrdName::N(o) if o <= 9 => match t.matching {
					Some(OrdMatch::LastDigit) | None	=> n % 10 == o as i32,
					Some(OrdMatch::LastTwoDigits)		=> n % 100 == o as i32,
					Some(OrdMatch::WholeNumber)		=> n == o as i32,
				},
				OrdName::N(o) if o <= 99 => match t.matching {
					Some(OrdMatch::LastTwoDigits) | None	=> n % 100 == o as i32,
					Some(OrdMatch::WholeNumber)		=> n == o as i32,
					Some(OrdMatch::LastDigit)		=> false,
				},
				_ => false,
			};
			if !hit {
				continue;
			}
			let cur = match best {
				None => { best = Some(t); continue; }
				Some(c) => c,
			};
			let OrdName::N(other_n) = ord_name(&t.name) else { continue };
			let OrdName::N(cur_n) = ord_name(&cur.name) else { best = Some(t); continue };
			best = Some(if other_n >= 10 && cur_n < 10 {
				t
			} else if other_n < 10 && cur_n >= 10 {
				cur
			} else if gender == cur.gender && gender != t.gender {
				cur
			} else if gender != cur.gender && gender == t.gender {
				t
			} else if (n - other_n as i32).abs() <= (n - cur_n as i32).abs() {
				t
			} else {
				cur
			});
		}
		best.map(|t| t.single.as_str())
	}

	/// A long ordinal; 1 to 10 need an exact match.
	fn lookup_long(&self, n: i32) -> Option<&'a str> {
		self.terms.iter().find(|t| {
			let OrdName::Long(o) = ord_name(&t.name) else { return false };
			if n > 0 && n <= 10 {
				n == o as i32
			} else {
				match t.matching {
					Some(OrdMatch::LastTwoDigits) | None	=> n % 100 == o as i32,
					Some(OrdMatch::WholeNumber)		=> n == o as i32,
					Some(OrdMatch::LastDigit)		=> false,
				}
			}
		}).map(|t| t.single.as_str())
	}
}

// Context
enum Target<'a> {
	Std(&'a str, Chunked),
	Num(&'a str, NumVal),
	Page(String),
	Macro(&'a [Elem]),
	Term(&'a str),
	Value(&'a str),
}

enum NumOrPage {
	Num(NumVal),
	Page(String),
}

/// One rendering of one entry: the style, the locale chain, the item and the citation, over a
/// writer.
pub struct Ctx<'a, I: Item> {
	style:	&'a Style,
	chain:	&'a Chain<'a>,
	item:	&'a I,
	cite:	&'a Cite,
	lang:	Option<&'a str>,	// the locale the document asked for, if any
	bib:	bool,
	w:	Writer,
}

impl<'a, I: Item> Write for Ctx<'a, I> {
	fn write_str(&mut self, s: &str) -> std::fmt::Result {
		self.push_str(s);
		Ok(())
	}
}

/// Renders one bibliography entry through the style's bibliography layout, wrapped in the
/// layout's prefix and suffix. `None` when the style has no bibliography.
pub fn bibliography_entry<'a, I: Item>(
	style:	&'a Style,
	chain:	&'a Chain<'a>,
	item:	&'a I,
	cite:	&'a Cite,
	lang:	Option<&'a str>,
)
	-> Option<Vec<Node>>
{
	let bib = match style.bibliography.as_ref() {
		Some(b)	=> b,
		None	=> return None,
	};
	let mut ctx = Ctx::new(style, chain, item, cite, lang, true);
	let layout = &bib.layout;
	let a = ctx.w.apply_prefix(layout.look.prefix.as_deref());
	let fi = ctx.w.push_format(&layout.look.fmt);
	let di = ctx.w.push_delim(layout.delimiter.clone());
	for e in &layout.elems {
		ctx.render(e);
	}
	ctx.w.pop_delim(di);
	ctx.w.pop_format(fi);
	ctx.apply_suffix(layout.look.suffix.as_deref(), a);
	Some(ctx.flush())
}

impl<'a, I: Item> Ctx<'a, I> {

	pub fn new(
		style:	&'a Style,
		chain:	&'a Chain<'a>,
		item:	&'a I,
		cite:	&'a Cite,
		lang:	Option<&'a str>,
		bib:	bool,
	)
		-> Self
	{
		Self { style, chain, item, cite, lang, bib, w: Writer::default() }
	}

	pub fn flush(self) -> Vec<Node> {
		self.w.flush()
	}

	/// Did a `choose` ask the `disambiguate` condition? B3 reads this to know whether
	/// disambiguation could change the output.
	pub fn checked_disambiguate(&self) -> bool {
		self.w.checked_disamb
	}

	// Writing

	/// Pushes text through the quirks hayagriva applies, in its order: pull punctuation into a
	/// closing quote, reconfigure the case, let punctuation eat a trailing space, drop an affix
	/// that duplicates the end of what is written, and strip periods.
	fn push_str(&mut self, s: &str) {
		let pulled = self.pull_punctuation(s);
		let s = pulled.as_str();
		self.w.reconfigure();

		let ends_ws = self.w.last_buffer()
			.map_or(false, |b| b.chars().next_back().map_or(false, char::is_whitespace));
		// Punctuation eats spaces.
		if ends_ws && s.chars().next().map_or(false, |c| c.is_whitespace() || matches!(c, '.' | ',' | ']' | ')')) {
			if let Some(b) = self.w.last_buffer() {
				let n = b.trim_end().len();
				b.truncate(n);
			}
		}

		// A string with no letters is not written twice in a row, so `.` after `.` is one.
		if s.chars().all(|c| !c.is_alphabetic()) {
			let tr = s.trim_end();
			if let Some(last) = self.w.last_buffer() {
				if last.ends_with(tr) {
					let n = last.len() - tr.len();
					last.truncate(n);
				}
			}
		}

		if self.w.strip_periods {
			// citeproc.js's rule: a period goes unless the character before it was a period.
			let mut last_period = false;
			for c in s.chars() {
				let is_period = c == '.';
				if !is_period || last_period {
					self.w.buf.push(c);
				}
				last_period = is_period;
			}
		} else {
			self.w.buf.push_str(s);
		}
		self.w.pull = false;
	}

	/// Moves a leading `.`, `,`, `;`, `!` or `?` inside a closing quote just written, when the
	/// locale puts punctuation in quotes.
	fn pull_punctuation(&mut self, s: &str) -> String {
		let first = match s.chars().next() {
			Some(c) if self.w.pull && matches!(c, '.' | ',' | ';' | '!' | '?') => c,
			_ => return s.to_string(),
		};
		let cq = self.term("close-quote", TermForm::Long, false);
		let ciq = self.term("close-inner-quote", TermForm::Long, false);
		let in_tree = self.w.buf.is_empty() && self.w.top_non_empty_mut()
			.and_then(|l| l.last_mut())
			.and_then(get_last_text)
			.is_some();
		let used_buf = !in_tree;
		let buf: &mut String = if in_tree {
			match self.w.top_non_empty_mut().and_then(|l| l.last_mut()).and_then(get_last_text) {
				Some(b)	=> b,
				None	=> return s.to_string(),
			}
		} else {
			self.w.buf.as_string_mut()
		};
		let mut rest = s;
		for q in [cq, ciq].into_iter().flatten() {
			if buf.ends_with(q) {
				let n = buf.len() - q.len();
				buf.truncate(n);
				rest = &s[first.len_utf8()..];
				buf.push(first);
				buf.push_str(q);
				if used_buf {
					self.w.buf.mark_changed();
				}
				break;
			}
		}
		rest.to_string()
	}

	/// Pushes a chunked variable: protected chunks bypass the case, and mathematical chunks
	/// become `Math` nodes the case folder never trims across.
	fn push_chunked(&mut self, c: &Chunked) {
		use crate::csl::item::ChunkKind;
		for chunk in &c.0 {
			match chunk.kind {
				ChunkKind::Normal => self.push_str(&chunk.value),
				ChunkKind::Verbatim => {
					self.w.buf.push_verbatim(&chunk.value);
					self.w.pull = false;
				}
				ChunkKind::Math => {
					self.w.buf.prevent_trimming();
					self.w.save();
					self.w.top_mut().push(Node::Math(chunk.value.clone()));
					self.w.reconfigure();
					self.w.buf.prevent_trimming();
				}
			}
		}
	}

	fn push_link(&mut self, text: String, url: String) {
		let fmt = self.w.fmt();
		self.w.save();
		self.w.top_mut().push(Node::Link { text, fmt, url });
	}

	fn push_transparent(&mut self, idx: usize) {
		let fmt = self.w.fmt();
		self.w.save();
		self.w.top_mut().push(Node::Cite { idx, fmt });
	}

	fn push_quotes(&mut self) {
		let name = if self.w.inner_quotes { "open-inner-quote" } else { "open-quote" };
		if let Some(m) = self.term(name, TermForm::Long, false) {
			self.push_str(m);
		}
		self.w.inner_quotes = !self.w.inner_quotes;
	}

	fn pop_quotes(&mut self) {
		self.w.inner_quotes = !self.w.inner_quotes;
		let name = if self.w.inner_quotes { "close-inner-quote" } else { "close-quote" };
		if let Some(m) = self.term(name, TermForm::Long, false) {
			self.push_str(m);
		}
	}

	fn may_pull_punctuation(&mut self) {
		self.w.pull |= self.chain.punctuation_in_quote();
	}

	/// Pushes a text case. hayagriva applies title and sentence case only to English: the
	/// entry's language decides, then the requested locale, then the style's default, and an
	/// unknown language counts as English. This departs from CSL, which leaves sentence case
	/// to the style.
	fn push_case(&mut self, case: Option<TextCase>) -> usize {
		let idx = self.w.cases.len();
		let english = self.item.is_english()
			.or_else(|| self.lang.map(is_english_code))
			.or_else(|| self.style.default_locale.as_deref().map(is_english_code))
			.unwrap_or(true);
		let apply = case.map_or(true, |c| !is_language_dependent(c)) || english;
		self.w.cases.push(if apply { case } else { None });
		idx
	}

	/// A term, the `locator` term being replaced by the cited locator's kind.
	fn term(&self, name: &str, form: TermForm, plural: bool) -> Option<&'a str> {
		let name = match (&self.cite.locator, name) {
			(Some((kind, _)), "locator")	=> kind.as_str(),
			_				=> name,
		};
		self.chain.term_text(name, form, plural)
	}

	fn apply_suffix(&mut self, suffix: Option<&str>, a: Affix) {
		if !self.w.has_content_since(&a) {
			self.w.discard_elem(a.loc);
			return;
		}
		if let Some(sfx) = suffix {
			// A suffix the text already ends with is not repeated.
			let present = if !self.w.buf.is_empty() {
				self.w.buf.as_str().ends_with(sfx)
			} else {
				node::last_text(self.w.top()).map_or(false, |l| l.ends_with(sfx))
			};
			if !present {
				self.push_str(sfx);
			}
		}
		self.w.commit_elem(a.loc, None, None);
	}

	// Variables

	/// Is the variable still available? Once a substituted variable has rendered it is
	/// suppressed for the rest of the entry; while suppression is on, each variable asked for
	/// is recorded.
	fn prepare(&mut self, var: &str) -> bool {
		if self.w.suppressed.iter().any(|v| v == var) {
			return false;
		}
		if self.w.suppressing {
			self.w.suppressed.push(var.to_string());
		}
		true
	}

	fn resolve_std(&mut self, var: &str, form: VarForm) -> Option<Chunked> {
		if !self.prepare(var) {
			return None;
		}
		match var {
			"citation-label"	=> self.cite.label.as_deref().map(Chunked::normal),
			_			=> self.item.text(var, form),
		}
	}

	fn resolve_num(&mut self, var: &str) -> Option<NumVal> {
		if !self.prepare(var) {
			return None;
		}
		match var {
			"citation-number"		=> self.cite.number.map(|n| NumVal::Typed(Numeric::new(n as i32))),
			"locator"			=> self.cite.locator.as_ref().map(|(_, v)| v.clone()),
			"first-reference-note-number"	=> self.cite.first_note.map(|n| NumVal::Typed(Numeric::new(n as i32))),
			_				=> self.item.number(var).map(|s| NumVal::from_str(&s)),
		}
	}

	fn resolve_page(&mut self) -> Option<String> {
		if !self.prepare("page") {
			return None;
		}
		self.item.page()
	}

	fn resolve_num_or_page(&mut self, var: &str) -> Option<NumOrPage> {
		match var_kind(var) {
			VarKind::Page	=> self.resolve_page().map(NumOrPage::Page),
			_		=> self.resolve_num(var).map(NumOrPage::Num),
		}
	}

	fn resolve_date(&mut self, var: &str) -> Option<bool> {
		if !self.prepare(var) {
			return None;
		}
		self.item.date_state(var)
	}

	fn resolve_names(&mut self, var: &str) -> usize {
		if !self.prepare(var) {
			return 0;
		}
		self.item.name_count(var)
	}

	// Rendering

	pub fn render(&mut self, e: &'a Elem) {
		match e {
			Elem::Text(t)	=> self.render_text(t),
			Elem::Number(n)	=> self.render_number(n),
			Elem::Label(l)	=> self.render_label(l),
			Elem::Names(n)	=> self.render_names(n),
			Elem::Date(d)	=> self.render_date(d),
			Elem::Group(g)	=> self.render_group(g),
			Elem::Choose(c)	=> self.render_choose(c),
		}
	}

	/// Will the element print, and what did it query? hayagriva's `will_have_info`.
	fn info(&mut self, e: &'a Elem) -> (bool, Usage) {
		match e {
			Elem::Text(t)	=> self.text_info(t),
			Elem::Number(n)	=> self.number_info(n),
			Elem::Label(l)	=> self.label_info(l),
			Elem::Names(n)	=> self.names_info(n),
			Elem::Date(d)	=> self.date_info(d),
			Elem::Group(g)	=> self.group_info(g),
			Elem::Choose(c)	=> self.choose_info(c),
		}
	}

	/// Runs `f` with variable suppression off, restoring it after.
	fn unsuppressed<R, F: FnOnce(&mut Self) -> R>(&mut self, f: F) -> R {
		let s = self.w.suppressing;
		self.w.suppressing = false;
		let r = f(self);
		self.w.suppressing = s;
		r
	}

	fn target(&mut self, t: &'a Text) -> Option<Target<'a>> {
		match &t.src {
			TextSrc::Var { name, form } => match var_kind(name) {
				VarKind::Standard	=> self.resolve_std(name, *form).map(|v| Target::Std(name, v)),
				VarKind::Number		=> self.resolve_num(name).map(|n| Target::Num(name, n)),
				VarKind::Page		=> self.resolve_page().map(Target::Page),
				_			=> None,
			},
			TextSrc::Macro(m)			=> self.style.get_macro(m).map(Target::Macro),
			TextSrc::Term { name, form, plural }	=> self.term(name, *form, *plural).map(Target::Term),
			TextSrc::Value(v)			=> Some(Target::Value(v)),
		}
	}

	/// `text`. The prefix goes inside the element's formatting, so an italic title's prefix is
	/// italic too (hayagriva departs from CSL here). DOI, PMID and PMCID become links, and a
	/// prefix equal to the resolver URL is dropped in favour of showing the full URL.
	fn render_text(&mut self, t: &'a Text) {
		let target = match self.target(t) {
			Some(x)	=> x,
			None	=> return,
		};
		let depth = self.w.push_elem(&t.look.fmt);
		let url = match &target {
			Target::Std(v, _)	=> url_prefix(v),
			_			=> None,
		};
		let affix_is_url = url.map_or(false, |p| t.look.prefix.as_deref() == Some(p));
		let a = if affix_is_url { None } else { Some(self.w.apply_prefix(t.look.prefix.as_deref())) };
		if t.look.quotes {
			self.push_quotes();
		}
		self.w.strip_periods = t.look.strip_periods;
		let cidx = self.push_case(t.look.text_case);
		let mut should_render = true;
		match target {
			Target::Std(var, val) => match url {
				Some(p) => {
					let full = format!("{}{}", p, val);
					let shown = if affix_is_url { full.clone() } else { val.to_string() };
					self.push_link(shown, full);
				}
				None if var == "URL"	=> self.push_link(val.to_string(), val.to_string()),
				None			=> self.push_chunked(&val),
			},
			Target::Num(_, n) => match n {
				NumVal::Typed(num) if num.will_transform()	=> self.push_num(&num, NumForm::Numeric, None),
				NumVal::Typed(num)				=> self.push_str(&num.to_string()),
				NumVal::Str(s)					=> self.push_str(&s),
				NumVal::Transparent(i)				=> self.push_transparent(i),
			},
			// B2b: a typed page range goes through `page-range-format`.
			Target::Page(s) => self.push_str(&s.replace('-', "–")),
			Target::Macro(m) => {
				should_render = self.text_info(t).1.should_render_group();
				if should_render {
					let di = self.w.push_delim(None);
					for c in m {
						self.render(c);
					}
					self.w.pop_delim(di);
				}
			}
			Target::Term(s)		=> self.push_str(s),
			Target::Value(v)	=> self.push_str(v),
		}
		self.w.pop_case(cidx);
		self.w.strip_periods = false;
		if t.look.quotes {
			self.pop_quotes();
			self.may_pull_punctuation();
		}
		if let Some(a) = a {
			self.apply_suffix(t.look.suffix.as_deref(), a);
		}
		if should_render {
			let meta = match &t.src {
				TextSrc::Var { name, .. } if name == "citation-number"	=> Some(Meta::CitationNumber),
				TextSrc::Var { name, .. } if name == "citation-label"	=> Some(Meta::CitationLabel),
				TextSrc::Var { .. }					=> Some(Meta::Text),
				_							=> None,
			};
			self.w.commit_elem(depth, t.look.display, meta);
		} else {
			self.w.discard_elem(depth);
		}
	}

	fn text_info(&mut self, t: &'a Text) -> (bool, Usage) {
		let found = self.unsuppressed(|c| c.target(t));
		let targets_var = matches!(&t.src, TextSrc::Var { name, .. } if name != "year-suffix");
		let vars = Usage { vars: true, ..Usage::default() };
		match found {
			None					=> (false, Usage { vars: targets_var, ..Usage::default() }),
			Some(Target::Std(_, s)) if s.is_empty()	=> (false, vars),
			Some(Target::Std(..) | Target::Num(..) | Target::Page(_)) => (true, Usage { non_empty: true, ..vars }),
			Some(Target::Macro(m)) => {
				let (print, info) = self.children_info(m);
				let print = print && info.should_render_group();
				(print, Usage { macros: print, ..info })
			}
			Some(Target::Term(_))	=> (true, Usage::default()),
			Some(Target::Value(v))	=> (!v.is_empty(), Usage::default()),
		}
	}

	fn children_info(&mut self, elems: &'a [Elem]) -> (bool, Usage) {
		let mut print = false;
		let mut info = Usage::default();
		for c in elems {
			let (p, i) = self.info(c);
			print |= p;
			info = info.merge(i);
		}
		(print, info)
	}

	/// Writes a number in a form, each through `push_str` as hayagriva's `write!` does.
	fn push_num(&mut self, num: &Numeric, form: NumForm, gender: Option<Gender>) {
		let ords = Ords::new(self.chain);
		for (n, d) in &num.nums {
			let n = *n;
			match form {
				NumForm::Numeric => self.push_str(&n.to_string()),
				NumForm::Ordinal => {
					self.push_str(&n.to_string());
					self.push_str(ords.lookup(n, gender).unwrap_or(""));
				}
				NumForm::LongOrdinal => match ords.lookup_long(n) {
					Some(s) => self.push_str(s),
					None => {
						self.push_str(&n.to_string());
						self.push_str(ords.lookup(n, gender).unwrap_or(""));
					}
				},
				NumForm::Roman => match roman(n) {
					Some(r)	=> self.push_str(&r),
					None	=> self.push_str(&n.to_string()),
				},
			}
			if let Some(d) = d {
				self.push_str(NumDelim::as_str(*d));
			}
		}
	}

	/// `number`. The gender for ordinals is that of the variable's own long term.
	fn render_number(&mut self, n: &'a Number) {
		let value = self.resolve_num_or_page(&n.var);
		let depth = self.w.push_elem(&n.look.fmt);
		let a = self.w.apply_prefix(n.look.prefix.as_deref());
		let cidx = self.push_case(n.look.text_case);
		let gender = self.chain.term(&n.var, TermForm::Long).and_then(|t| t.gender);
		match value {
			Some(NumOrPage::Num(NumVal::Typed(num))) if num.will_transform() => self.push_num(&num, n.form, gender),
			Some(NumOrPage::Num(NumVal::Typed(num)))	=> { let _ = write!(self, "{}", num); }
			Some(NumOrPage::Num(NumVal::Str(s)))		=> self.push_str(&s),
			Some(NumOrPage::Num(NumVal::Transparent(i)))	=> self.push_transparent(i),
			// B2b: a typed page range goes through `page-range-format`.
			Some(NumOrPage::Page(s))			=> self.push_str(&s),
			None						=> {}
		}
		self.w.pop_case(cidx);
		self.apply_suffix(n.look.suffix.as_deref(), a);
		let meta = if n.var == "citation-number" { Meta::CitationNumber } else { Meta::Number };
		self.w.commit_elem(depth, n.look.display, Some(meta));
	}

	fn number_info(&mut self, n: &'a Number) -> (bool, Usage) {
		let found = self.unsuppressed(|c| c.resolve_num_or_page(&n.var).is_some());
		(found, Usage { vars: true, non_empty: found, ..Usage::default() })
	}

	fn label_plural(l: &Label, v: &NumVal) -> bool {
		match l.plural {
			Plural::Always		=> true,
			Plural::Never		=> false,
			Plural::Contextual	=> match v {
				NumVal::Typed(n)	=> n.is_plural(is_number_of(&l.var)),
				_			=> false,
			},
		}
	}

	/// The label's term and plural, or `None` when its variable is absent.
	fn label_term(&mut self, l: &'a Label) -> Option<Option<&'a str>> {
		let plural = match var_kind(&l.var) {
			// hayagriva ignores the label's `plural` attribute for pages.
			VarKind::Page => match self.resolve_page() {
				Some(p)	=> page_plural(&p),
				None	=> return None,
			},
			_ => match self.resolve_num(&l.var) {
				Some(v)	=> Self::label_plural(l, &v),
				None	=> return None,
			},
		};
		Some(self.term(&l.var, l.form, plural))
	}

	/// `label`. Inside `names` (an empty variable) it is B2b's.
	fn render_label(&mut self, l: &'a Label) {
		if l.var.is_empty() || !self.label_info(l).0 {
			return;
		}
		let content = match self.label_term(l) {
			Some(c)	=> c.unwrap_or(""),
			None	=> return,
		};
		let depth = self.w.push_elem(&Fmt::default());
		if !content.is_empty() {
			let fi = self.w.push_format(&l.look.fmt);
			let a = self.w.apply_prefix(l.look.prefix.as_deref());
			self.w.strip_periods = l.look.strip_periods;
			let cidx = self.push_case(l.look.text_case);
			self.push_str(content);
			self.w.pop_case(cidx);
			self.w.strip_periods = false;
			self.apply_suffix(l.look.suffix.as_deref(), a);
			self.w.pop_format(fi);
		}
		self.w.commit_elem(depth, None, Some(Meta::Label));
	}

	/// A label of a `custom` locator never prints. Unlike most queries this one runs with
	/// suppression as it stands (hayagriva).
	fn label_info(&mut self, l: &'a Label) -> (bool, Usage) {
		if l.var == "locator" && self.cite.locator.as_ref().map_or(false, |(k, _)| k == "custom") {
			return (false, Usage::default());
		}
		(self.label_term(l).map_or(false, |t| t.is_some()), Usage::default())
	}

	/// `names`. An empty list with a `substitute` renders the first substitute child that writes
	/// anything, with suppression on so the variable it used is not repeated later. A non-empty
	/// list is B2b's (`render_name_list`).
	fn render_names(&mut self, n: &'a Names) {
		// B2b: editor and translator merge into editor-translator when equal.
		let mut empty = true;
		for v in &n.vars {
			if self.resolve_names(v) > 0 {
				empty = false;
			}
		}
		if !empty {
			self.render_name_list(n);
			return;
		}
		if let Some(sub) = &n.substitute {
			self.w.suppressing = true;
			let depth = self.w.push_elem(&n.look.fmt);
			for c in sub {
				let len = self.w.len();
				// B2b: a `names` child inherits this element's options.
				self.render(c);
				if len < self.w.len() {
					break;
				}
			}
			self.w.commit_elem(depth, n.look.display, Some(Meta::Names));
			self.w.suppressing = false;
		}
	}

	/// Hook for B2b: renders a non-empty name list.
	fn render_name_list(&mut self, _n: &'a Names) {}

	fn names_info(&mut self, n: &'a Names) -> (bool, Usage) {
		let s = self.w.suppressing;
		self.w.suppressing = false;
		let counts: Vec<usize> = n.vars.iter().map(|v| self.resolve_names(v)).collect();
		let mut empty = counts.iter().all(|&c| c == 0);
		let mut sub = (false, Usage::default());
		if let Some(elems) = &n.substitute {
			for c in elems {
				let (p, i) = self.info(c);
				sub = (sub.0 || p, sub.1.merge(i));
			}
		}
		// A name element showing none of an et-al list counts as empty.
		if let Some(name) = &n.name {
			if let (Some(min), Some(0)) = (name.opts.et_al_min, name.opts.et_al_use_first) {
				if counts.iter().all(|&c| c >= min as usize) {
					empty = true;
				}
			}
		}
		self.w.suppressing = s;
		let visible = !empty || sub.0;
		(visible, if empty { sub.1 } else { Usage { vars: true, non_empty: true, ..Usage::default() } })
	}

	/// `date`. An absent date writes nothing; a present one is B2b's.
	fn render_date(&mut self, d: &'a Date) {
		if d.var.is_empty() || self.resolve_date(&d.var).is_none() {
			return;
		}
		// B2b: render the date parts.
	}

	/// The date is queried after suppression is restored, so it is recorded (hayagriva).
	fn date_info(&mut self, d: &'a Date) -> (bool, Usage) {
		if d.var.is_empty() {
			return (false, Usage { vars: true, ..Usage::default() });
		}
		let found = self.resolve_date(&d.var).is_some();
		(found, Usage { vars: true, non_empty: found, ..Usage::default() })
	}

	/// `group`: rendered and then kept only if it queried no variable or one was non-empty.
	fn render_group(&mut self, g: &'a Group) {
		let depth = self.w.push_elem(&g.look.fmt);
		let a = self.w.apply_prefix(g.look.prefix.as_deref());
		let info = self.group_info(g).1;
		let di = self.w.push_delim(g.delimiter.clone());
		self.render_with_delimiter(&g.elems);
		self.w.pop_delim(di);
		self.apply_suffix(g.look.suffix.as_deref(), a);
		if info.should_render_group() {
			self.w.commit_elem(depth, g.look.display, None);
		} else {
			self.w.discard_elem(depth);
		}
	}

	fn group_info(&mut self, g: &'a Group) -> (bool, Usage) {
		let mut print = false;
		let mut info = Usage::default();
		for c in &g.elems {
			let (p, i) = self.info(c);
			print |= p;
			info = info.merge(i);
			if p && matches!(c, Elem::Group(_)) {
				info.group = true;
			}
		}
		(print && info.should_render_group(), info)
	}

	/// Renders children that will print, the current delimiter between them. A delimiter opens a
	/// level that also holds the child after it.
	fn render_with_delimiter(&mut self, elems: &'a [Elem]) {
		let delim = self.w.delim();
		let mut first = true;
		let mut loc: Option<Loc> = None;
		for c in elems {
			if !self.info(c).0 {
				continue;
			}
			if !first {
				if let Some(d) = &delim {
					if let Some(prev) = loc.take() {
						self.w.commit_elem(prev, None, None);
					}
					loc = Some(self.w.push_elem(&Fmt::default()));
					self.push_str(d);
				}
			}
			first = false;
			let pos = self.w.push_elem(&Fmt::default());
			self.render(c);
			self.w.commit_elem(pos, None, None);
		}
		if let Some(l) = loc {
			self.w.commit_elem(l, None, None);
		}
	}

	fn render_choose(&mut self, c: &'a Choose) {
		if let Some(elems) = self.choose_branch(c) {
			self.render_with_delimiter(elems);
		}
	}

	fn choose_info(&mut self, c: &'a Choose) -> (bool, Usage) {
		match self.choose_branch(c) {
			Some(elems)	=> self.children_info(elems),
			None		=> (false, Usage::default()),
		}
	}

	fn choose_branch(&mut self, c: &'a Choose) -> Option<&'a [Elem]> {
		self.unsuppressed(|ctx| {
			for b in &c.branches {
				match &b.cond {
					Some(cond) if ctx.test(cond)	=> return Some(b.elems.as_slice()),
					Some(_)				=> {}
					None				=> return Some(b.elems.as_slice()),
				}
			}
			None
		})
	}

	/// Evaluates a condition's tests lazily, in hayagriva's order whatever the attribute order:
	/// disambiguate, is-numeric, is-uncertain-date, locator, position, type, variable. Position is
	/// skipped in a bibliography.
	fn test(&mut self, cond: &'a Cond) -> bool {
		fn rank(t: &Test) -> u8 {
			match t {
				Test::Disambiguate(_)		=> 0,
				Test::IsNumeric(_)		=> 1,
				Test::IsUncertainDate(_)	=> 2,
				Test::Locator(_)		=> 3,
				Test::Position(_)		=> 4,
				Test::Type(_)			=> 5,
				Test::Variable(_)		=> 6,
			}
		}
		let mut tests: Vec<&'a Test> = cond.tests.iter()
			.filter(|t| !(self.bib && matches!(t, Test::Position(_))))
			.collect();
		tests.sort_by_key(|t| rank(t));
		let mut results = tests.into_iter().map(|t| self.eval(t));
		match cond.mode {
			Match::All	=> results.all(|b| b),
			Match::Any	=> results.any(|b| b),
			Match::None	=> !results.any(|b| b),
		}
	}

	fn eval(&mut self, t: &'a Test) -> bool {
		match t {
			Test::Disambiguate(d) => {
				self.w.checked_disamb = true;
				*d == self.cite.disambiguate
			}
			Test::IsNumeric(v) => match var_kind(v) {
				VarKind::Standard	=> self.resolve_std(v, VarForm::Long).map_or(false, |c| Numeric::parse(&c.to_string()).is_some()),
				VarKind::Number		=> matches!(self.resolve_num(v), Some(NumVal::Typed(_))),
				VarKind::Page		=> self.resolve_page().map_or(false, |p| is_numeric_page(&p)),
				_			=> false,
			},
			Test::IsUncertainDate(v)	=> self.resolve_date(v) == Some(true),
			Test::Locator(l)		=> self.cite.locator.as_ref().map_or(false, |(k, _)| k == l),
			Test::Position(p) => match p {
				Position::First			=> self.cite.first,
				Position::Subsequent		=> !self.cite.first,
				Position::Ibid			=> self.cite.ibid != Ibid::No,
				Position::IbidWithLocator	=> self.cite.ibid == Ibid::WithLocator,
				Position::NearNote		=> self.cite.near_note,
			},
			Test::Type(ty)	=> self.item.is_type(ty),
			Test::Variable(v) => match var_kind(v) {
				VarKind::Standard	=> self.resolve_std(v, VarForm::Long).map_or(false, |c| !c.is_blank()),
				VarKind::Number		=> self.resolve_num(v).is_some(),
				VarKind::Date		=> self.resolve_date(v).is_some(),
				VarKind::Name		=> self.resolve_names(v) > 0,
				VarKind::Page		=> self.resolve_page().is_some(),
			},
		}
	}
}
