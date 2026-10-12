//! Ported from hayagriva 0.10.1, Copyright (c) Martin Haug and the hayagriva contributors,
//! MIT OR Apache-2.0 (`src/csl/mod.rs`, the writing context; `src/csl/rendering/mod.rs` and
//! `rendering/names.rs`, the element renderers; citationberg 0.7.0 `OrdinalLookup`, same
//! licence).
//!
//! Dates are hayagriva's `rendering/mod.rs` 624-905 and names `rendering/names.rs` 190-868. B3
//! adds the special forms, sorting (which reverses names and strips institutional articles), name
//! disambiguation, the implicit year-suffix, the citation layout and second-field-align.
use crate::csl::{
	cite::CiteForm,
	case::{
		is_language_dependent,
		Case,
		CaseFolder,
	},
	item::{
		is_number_of,
		var_kind,
		Chunked,
		DateVal,
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
	page::PageVal,
	person::Person,
	style::{
		And,
		Choose,
		CiteFormat,
		Cond,
		Date,
		DatePart,
		DatePartForm,
		DatePartName,
		DateParts,
		DelimRule,
		Demote,
		Display,
		Elem,
		EtAl,
		Fmt,
		Group,
		Label,
		Layout,
		Look,
		Match,
		Name,
		NameForm,
		NameOpts,
		NamePart,
		Names,
		NumForm,
		Number,
		Plural,
		PartName,
		Position,
		SortOrder,
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

/// Rendering a sort key: by a variable, or through a macro. Names reverse, institutional names
/// lose a leading article, and dates and numbers render as sortable values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sorting { Variable, Macro }

/// What a cite's disambiguation has reached: hayagriva's `DisambiguateState`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Disamb { #[default] None, Names(NameDisamb), Choose, YearSuffix(u8) }

impl Disamb {

	pub fn may_names(&self) -> bool {
		matches!(self, Self::None | Self::Names(_))
	}

	pub fn may_choose(&self) -> bool {
		matches!(self, Self::None | Self::Names(_))
	}

	pub fn may_year_suffix(&self) -> bool {
		matches!(self, Self::None | Self::Names(_) | Self::Choose)
	}

	/// The further of two states: names, then choose, then a year-suffix.
	pub fn max(self, other: Self) -> Self {
		match (self, other) {
			(Self::None, o)				=> o,
			(s, Self::None)				=> s,
			(Self::Names(a), Self::Names(b))	=> Self::Names(a.max(b)),
			(Self::Names(_), o)			=> o,
			(s, Self::Names(_))			=> s,
			(Self::Choose, o)			=> o,
			(s, Self::Choose)			=> s,
			(Self::YearSuffix(a), Self::YearSuffix(b))	=> Self::YearSuffix(a.max(b)),
		}
	}
}

/// hayagriva's `NameDisambiguationProperties`: for each variable of a `names` element, each
/// person's form, `None` while et al. cuts it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameDisamb {
	forms:		Vec<Vec<Option<NForm>>>,
	default:	NForm,
	vars:		Vec<String>,
}

impl NameDisamb {

	/// Shows the first `num` names of the first variable that has as many.
	pub fn add_name(&mut self, num: usize) -> bool {
		let def = self.default;
		match self.forms.iter_mut().find(|l| l.len() >= num) {
			Some(l) => {
				for n in l.iter_mut().take(num) {
					if n.is_none() {
						*n = Some(def);
					}
				}
				true
			}
			None => false,
		}
	}

	/// The more disambiguated: more names shown, then the larger forms.
	pub fn max(self, other: Self) -> Self {
		let shown = |x: &Self| x.forms.iter().flatten().filter(|f| f.is_some()).count();
		let ord = shown(&self).cmp(&shown(&other))
			.then_with(|| self.forms.iter().flatten().cmp(other.forms.iter().flatten()));
		match ord {
			std::cmp::Ordering::Greater	=> self,
			_				=> other,
		}
	}

	/// The `idx`th name's form across the variables, or the default form when it is cut or
	/// absent.
	pub fn form_mut(&mut self, idx: usize) -> &mut NForm {
		match self.forms.iter_mut().flat_map(|l| l.iter_mut()).nth(idx) {
			Some(Some(f))	=> f,
			_		=> &mut self.default,
		}
	}
}

/// hayagriva's special forms: render one variable only, the first date alone (a collapsed
/// year), the year-suffix alone, or everything but the author.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Special { VarOnly(String), OnlyFirstDate, OnlyYearSuffix, SuppressAuthor }

/// What the citation, rather than the entry, supplies: its number, label and locator, and the
/// position tests' answers.
#[derive(Clone, Debug, Default)]
pub struct Cite {
	pub number:		Option<usize>,
	pub label:		Option<String>,
	pub locator:		Option<(String, NumVal)>,	// kind (`page`, `custom`) and value
	pub first_note:		Option<usize>,			// first-reference-note-number
	pub disamb:		Disamb,
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
	first_name:	Option<NameDisamb>,	// the first `names` element's forms
	dates_seen:	bool,	// the first date has rendered
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

	/// A space after what is written, unless it already ends in a space or NBSP. With the buffer
	/// empty it goes on the top level's last text.
	fn ensure_space(&mut self) {
		if !self.buf.is_empty() {
			let s = self.buf.as_str();
			if !s.ends_with(' ') && !s.ends_with('\u{a0}') {
				self.buf.push(' ');
			}
		} else if let Some(l) = node::last_text_mut(self.top_mut()) {
			if !l.ends_with(' ') && !l.ends_with('\u{a0}') {
				l.push(' ');
			}
		}
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

/// Formatting laid over a base: each attribute `top` sets wins.
fn fmt_over(top: &Fmt, base: &Fmt) -> Fmt {
	Fmt {
		style:		top.style.or(base.style),
		variant:	top.variant.or(base.variant),
		weight:		top.weight.or(base.weight),
		deco:		top.deco.or(base.deco),
		valign:		top.valign.or(base.valign),
	}
}

/// A date part's form, defaulting to numeric days, long months and long years.
fn part_form(p: &DatePart) -> DatePartForm {
	match (p.form, p.name) {
		(Some(f), _)			=> f,
		(None, DatePartName::Day)	=> DatePartForm::Numeric,
		(None, _)			=> DatePartForm::Long,
	}
}

// Names
/// hayagriva's `DisambiguatedNameForm`, ordered as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum NForm { Count, ShortInit, ShortFull, LongInit, LongFull }

/// citationberg's `NameOptions`: a `name`'s options over those it inherits, with CSL's defaults.
#[derive(Clone, Debug)]
struct NOpts {
	and:		Option<And>,
	delim:		String,
	prec_et_al:	DelimRule,
	prec_last:	DelimRule,
	min:		Option<u32>,
	use_first:	Option<u32>,
	sub_min:	Option<u32>,
	sub_first:	Option<u32>,
	use_last:	bool,
	form:		NameForm,
	initialize:	bool,
	init_with:	Option<String>,
	sort_order:	Option<SortOrder>,
	sort_sep:	String,
}

impl NOpts {

	fn new(o: NameOpts) -> Self {
		Self {
			and:		o.and,
			delim:		o.delimiter.unwrap_or_else(|| ", ".to_string()),
			prec_et_al:	o.delim_precedes_et_al.unwrap_or(DelimRule::Contextual),
			prec_last:	o.delim_precedes_last.unwrap_or(DelimRule::Contextual),
			min:		o.et_al_min,
			use_first:	o.et_al_use_first,
			sub_min:	o.et_al_subsequent_min,
			sub_first:	o.et_al_subsequent_use_first,
			use_last:	o.et_al_use_last.unwrap_or(false),
			form:		o.form.unwrap_or(NameForm::Long),
			initialize:	o.initialize.unwrap_or(true),
			init_with:	o.initialize_with,
			sort_order:	o.name_as_sort_order,
			sort_sep:	o.sort_separator.unwrap_or_else(|| ", ".to_string()),
		}
	}

	fn form(&self) -> NForm {
		match (self.init_with.is_some() && self.initialize, self.form) {
			(true, NameForm::Short)		=> NForm::ShortInit,
			(true, _)			=> NForm::LongInit,
			(false, NameForm::Short)	=> NForm::ShortFull,
			(false, NameForm::Count)	=> NForm::Count,
			(false, NameForm::Long)		=> NForm::LongFull,
		}
	}

	/// Is the name at `i` of `len` cut by et al.? The last name never is under et-al-use-last.
	fn is_suppressed(&self, i: usize, len: usize, subsequent: bool) -> bool {
		let (min, first) = match subsequent {
			true	=> (self.sub_min.or(self.min), self.sub_first.or(self.use_first)),
			false	=> (self.min, self.use_first),
		};
		if self.use_last && i + 1 == len {
			return false;
		}
		match (min, first) {
			(Some(m), Some(f))	=> len >= m as usize && i + 1 > f as usize,
			_			=> false,
		}
	}
}

/// A `names` element as rendered. Inside a `substitute`, a `names` with no `name`, `et-al` or
/// `substitute` of its own takes its parent's name, et-al, label, delimiter, affixes, display and
/// formatting, and its variables when it names none.
#[derive(Clone, Copy)]
struct NamesView<'a> {
	vars:		&'a [String],
	delimiter:	Option<&'a str>,
	opts:		&'a NameOpts,
	name:		Option<&'a Name>,
	et_al:		Option<&'a EtAl>,
	label:		Option<&'a Label>,
	label_first:	bool,
	look:		&'a Look,
	substitute:	Option<&'a [Elem]>,
}

impl<'a> NamesView<'a> {

	fn of(n: &'a Names) -> Self {
		Self {
			vars:		&n.vars,
			delimiter:	n.delimiter.as_deref(),
			opts:		&n.opts,
			name:		n.name.as_ref(),
			et_al:		n.et_al.as_ref(),
			label:		n.label.as_ref(),
			label_first:	n.label_first,
			look:		&n.look,
			substitute:	n.substitute.as_deref(),
		}
	}

	fn substitute_child(self, c: &'a Names) -> Self {
		if c.name.is_some() || c.et_al.is_some() || c.substitute.is_some() {
			return Self::of(c);
		}
		let (label, label_first) = match &c.label {
			Some(l)	=> (Some(l), c.label_first),
			None	=> (self.label, self.label_first),
		};
		Self {
			vars:		if c.vars.is_empty() { self.vars } else { &c.vars },
			label,
			label_first,
			substitute:	None,
			..self
		}
	}
}

/// A year-suffix as letters: a to z, then aa.
fn letters(n: u8) -> String {
	let mut out = Vec::new();
	let mut c = n as u32;
	loop {
		out.insert(0, (b'a' + (c % 26) as u8) as char);
		if c < 26 {
			break;
		}
		c = (c - 26) / 26;
	}
	out.into_iter().collect()
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
	Page(PageVal),
	Macro(&'a [Elem]),
	Term(&'a str),
	Value(&'a str),
}

enum NumOrPage {
	Num(NumVal),
	Page(PageVal),
}

/// A `names` element's metadata: each variable's people, so that grouping and disambiguation can
/// compare them.
fn meta_people(people: &[(&[Person], &str)]) -> Meta {
	Meta::Names(people.iter().map(|(p, v)| (p.to_vec(), v.to_string())).collect())
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
	sorting:	Option<Sorting>,
	sort_names:	NameOpts,	// a macro sort key's names-min, -use-first and -use-last
	special:	Option<Special>,
	bib_names:	bool,		// the bibliography's name options, outside the bibliography
	ibid:		Option<Ibid>,	// overrides the cite's for the Author form's retry
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

/// One cite rendered through the citation layout, which, as in hayagriva, applies its formatting
/// and delimiter but leaves its affixes to the whole citation.
#[derive(Clone, Debug, Default)]
pub struct CiteOut {
	pub nodes:		Vec<Node>,
	pub checked_disamb:	bool,	// a `choose` asked `disambiguate`
	pub first_name:		Option<NameDisamb>,
}

/// One cite in a citation form: the citation layout for `Normal` and `Year` (whose final text
/// [`year_only`] gives), the bibliography layout for `Full`, the author alone for `Author`, and
/// the author followed by the rest for `Prose`. hayagriva's `do_citation`.
pub fn citation_entry<'a, I: Item>(
	style:	&'a Style,
	chain:	&'a Chain<'a>,
	item:	&'a I,
	cite:	&'a Cite,
	lang:	Option<&'a str>,
	special:	Option<Special>,
	form:	CiteForm,
)
	-> CiteOut
{
	let mut ctx = Ctx::new(style, chain, item, cite, lang, false);
	match (form, style.bibliography.as_ref()) {
		(CiteForm::Author, _) => ctx.cite_author(),
		(CiteForm::Full, Some(bib)) => {
			ctx.bib_names = true;
			ctx.render_layout(&bib.layout, true);
			ctx.bib_names = false;
			if bib.second_field_align.is_some() {
				ctx.w.save();
				let top = &mut ctx.w.base;
				if let Some(mut first) = node::remove_any_meta(top) {
					match &mut first {
						Node::Text(t, _)	=> t.push(' '),
						Node::Elem(e)		=> if let Some(t) = node::last_text_mut(&mut e.children) { t.push(' ') },
						_			=> (),
					}
					top.insert(0, first);
				}
			}
		}
		(CiteForm::Prose, _) => {
			ctx.cite_author();
			let layout = &style.citation.layout;
			let pre = layout.look.prefix.as_deref();
			if !pre.map_or(false, |p| p.chars().next().map_or(false, char::is_whitespace)) {
				ctx.w.ensure_space();
			}
			let label = matches!(style.info.format, Some(CiteFormat::Label | CiteFormat::Numeric));
			let suf = layout.look.suffix.as_deref();
			match label {
				true => {
					if let Some(p) = pre {
						ctx.push_str(p);
					}
					ctx.regular(special);
					if let Some(s) = suf {
						ctx.push_str(s);
					}
				}
				false => {
					ctx.push_str(pre.unwrap_or("("));
					ctx.regular(Some(Special::SuppressAuthor));
					ctx.push_str(suf.unwrap_or(")"));
				}
			}
		}
		_ => ctx.regular(special),
	}
	let checked_disamb = ctx.checked_disambiguate();
	let first_name = ctx.w.first_name.take();
	CiteOut { nodes: ctx.flush(), checked_disamb, first_name }
}

/// The Year form's final text: the year of the first of issued, event, submitted and original
/// dates, else the `no date` term. hayagriva's `date_replacement`.
pub fn year_only<'a, I: Item>(
	chain:	&'a Chain<'a>,
	item:	&'a I,
)
	-> Vec<Node>
{
	let date = ["issued", "event-date", "submitted", "original-date"].iter()
		.find_map(|v| item.date(v));
	let term = |name: &str| chain.term_text(name, TermForm::Long, false);
	let text = match date {
		Some(d) => {
			let y = d.year;
			let mut s = if y > 0 { y.to_string() } else { (y.abs() + 1).to_string() };
			if y < 1000 {
				s.push_str(match y <= 0 {
					true	=> term("bc").unwrap_or(" BC"),
					false	=> term("ad").unwrap_or(" AD"),
				});
			}
			s
		}
		None => term("no date").unwrap_or("n.d.").to_string(),
	};
	vec![Node::Text(text, Formatting::default())]
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
		Self { style, chain, item, cite, lang, bib, sorting: None, sort_names: NameOpts::default(), special: None, bib_names: bib, ibid: None, w: Writer::default() }
	}

	/// This context renders a sort key.
	pub fn sorting(mut self, s: Sorting, names: NameOpts) -> Self {
		self.sorting = Some(s);
		self.sort_names = names;
		self
	}

	/// The special form to render in, or none.
	pub fn special(mut self, s: Option<Special>) -> Self {
		self.special = s;
		self
	}

	/// The citation layout, with its formatting and delimiter but not its affixes, in a special
	/// form or none.
	fn regular(&mut self, special: Option<Special>) {
		let prev = std::mem::replace(&mut self.special, special);
		self.render_layout(&self.style.citation.layout, false);
		self.special = prev;
	}

	/// A layout's elements, with its formatting and delimiter, and its affixes when asked.
	fn render_layout(&mut self, layout: &'a Layout, affixes: bool) {
		let a = match affixes {
			true	=> Some(self.w.apply_prefix(layout.look.prefix.as_deref())),
			false	=> None,
		};
		let fi = self.w.push_format(&layout.look.fmt);
		let di = self.w.push_delim(layout.delimiter.clone());
		self.render_all(&layout.elems);
		self.w.pop_delim(di);
		self.w.pop_format(fi);
		if let Some(a) = a {
			self.apply_suffix(layout.look.suffix.as_deref(), a);
		}
	}

	/// The cite's ibid position, or the override.
	fn ibid(&self) -> Ibid {
		self.ibid.unwrap_or(self.cite.ibid)
	}

	/// The Author form: the author, or the editor when there is no author, as the citation
	/// layout renders it; failing that, with ibid set aside; failing that, as the bibliography
	/// renders it; failing that, a plain `names` element.
	fn cite_author(&mut self) {
		let style: &'a Style = self.style;
		let var = match self.item.names("author").is_empty() && !self.item.names("editor").is_empty() {
			true	=> "editor",
			false	=> "author",
		};
		let layout = &style.citation.layout;
		let only = Some(Special::VarOnly(var.to_string()));
		if self.will_render_all(&layout.elems, var) {
			self.regular(only);
			return;
		}
		if self.ibid() != Ibid::No {
			let prev = self.ibid.replace(Ibid::No);
			let found = self.will_render_all(&layout.elems, var);
			if found {
				self.regular(only.clone());
			}
			self.ibid = prev;
			if found {
				return;
			}
		}
		self.special = only;
		match style.bibliography.as_ref() {
			Some(b) if self.will_render_all(&b.layout.elems, var) => {
				self.bib_names = true;
				self.render_layout(&b.layout, true);
				self.bib_names = false;
			}
			_ => {
				// hayagriva synthesises `<names variable="author"/>` with the citation layout's
				// formatting, the special form still set.
				static SYNTH: std::sync::OnceLock<Elem> = std::sync::OnceLock::new();
				let synth = SYNTH.get_or_init(|| Elem::Names(Names {
					vars:		vec!["author".to_string()],
					delimiter:	None,
					opts:		NameOpts::default(),
					name:		None,
					et_al:		None,
					label:		None,
					label_first:	false,
					substitute:	None,
					look:		Look::default(),
				}));
				let fi = self.w.push_format(&layout.look.fmt);
				self.render(synth);
				self.w.pop_format(fi);
			}
		}
		self.special = None;
	}

	pub fn render_all(&mut self, elems: &'a [Elem]) {
		for e in elems {
			self.render(e);
		}
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
			"year-suffix"		=> match self.cite.disamb {
				Disamb::YearSuffix(n)	=> Some(Chunked::normal(&letters(n))),
				_			=> None,
			},
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

	fn resolve_page(&mut self) -> Option<PageVal> {
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

	fn resolve_date(&mut self, var: &str) -> Option<DateVal> {
		if !self.prepare(var) {
			return None;
		}
		self.item.date(var)
	}

	fn resolve_names(&mut self, var: &str) -> &'a [Person] {
		if !self.prepare(var) {
			return &[];
		}
		let item: &'a I = self.item;
		item.names(var)
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
		// A special form passes macros and only its own variables.
		if let (Some(sp), TextSrc::Var { name, .. }) = (&self.special, &t.src) {
			let pass = match sp {
				Special::VarOnly(v)					=> v == name,
				Special::OnlyFirstDate | Special::OnlyYearSuffix	=> name == "year-suffix" || name == "locator",
				Special::SuppressAuthor					=> true,
			};
			if !pass {
				return None;
			}
		}
		if let (Some(Special::VarOnly(_) | Special::OnlyFirstDate | Special::OnlyYearSuffix), TextSrc::Term { .. } | TextSrc::Value(_)) = (&self.special, &t.src) {
			return None;
		}
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
		// A macro used out of its context, under VarOnly, prints no affixes.
		let out_of_context = matches!(self.special, Some(Special::VarOnly(_))) && matches!(t.src, TextSrc::Macro(_));
		let a = match affix_is_url || out_of_context {
			true	=> None,
			false	=> Some(self.w.apply_prefix(t.look.prefix.as_deref())),
		};
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
			Target::Page(p) => self.push_page(p, true),
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

	/// A page value: typed ranges through `page-range-format` and the locale's range delimiter. An
	/// untyped value has its hyphens made en dashes in `text` but is written as given by `number`
	/// (hayagriva).
	fn push_page(&mut self, p: PageVal, in_text: bool) {
		match p {
			PageVal::Typed(r) => {
				let fmt = self.style.page_range_format;
				let delim = self.term("page-range-delimiter", TermForm::Long, false);
				r.write(self, fmt, delim);
			}
			PageVal::Str(s) if in_text	=> self.push_str(&s.replace('-', "–")),
			PageVal::Str(s)			=> self.push_str(&s),
		}
	}

	/// `number`. The gender for ordinals is that of the variable's own long term.
	fn render_number(&mut self, n: &'a Number) {
		if !self.number_passes(&n.var) {
			return;
		}
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
			Some(NumOrPage::Page(p))			=> self.push_page(p, false),
			None						=> {}
		}
		self.w.pop_case(cidx);
		self.apply_suffix(n.look.suffix.as_deref(), a);
		let meta = if n.var == "citation-number" { Meta::CitationNumber } else { Meta::Number };
		self.w.commit_elem(depth, n.look.display, Some(meta));
	}

	/// Does a special form let a number or label of this variable through? hayagriva's
	/// `Number::will_render`: only the locator, under a date form, and nothing under VarOnly.
	fn number_passes(&self, var: &str) -> bool {
		match &self.special {
			None | Some(Special::SuppressAuthor)				=> true,
			Some(Special::OnlyFirstDate | Special::OnlyYearSuffix)	=> var == "locator",
			Some(Special::VarOnly(_))					=> false,
		}
	}

	/// hayagriva's answer under a special form: a number other than the locator claims to print
	/// and queries nothing, so its group still renders.
	fn special_info(&self, var: &str) -> Option<(bool, Usage)> {
		match &self.special {
			Some(Special::VarOnly(v)) if v != var				=> Some((false, Usage::default())),
			Some(Special::VarOnly(_) | Special::OnlyFirstDate | Special::OnlyYearSuffix)
				if var != "locator"					=> Some((true, Usage::default())),
			_								=> None,
		}
	}

	fn number_info(&mut self, n: &'a Number) -> (bool, Usage) {
		if let Some(r) = self.special_info(&n.var) {
			return r;
		}
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
				Some(p)	=> p.is_plural(),
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
		if let Some(r) = self.special_info(&l.var) {
			return r;
		}
		if l.var == "locator" && self.cite.locator.as_ref().map_or(false, |(k, _)| k == "custom") {
			return (false, Usage::default());
		}
		(self.label_term(l).map_or(false, |t| t.is_some()), Usage::default())
	}

	/// `names`. An empty list with a `substitute` renders the first substitute child that writes
	/// anything, with suppression on so the variable it used is not repeated later.
	fn render_names(&mut self, n: &'a Names) {
		self.render_names_view(NamesView::of(n));
	}

	/// The variables' people. editor and translator merge into editor-translator when both are
	/// asked for, the locale has the term, and the lists are equal.
	fn people(&mut self, v: NamesView<'a>) -> Vec<(&'a [Person], &'a str)> {
		let has = |s: &str| v.vars.iter().any(|x| x == s);
		if v.vars.len() == 2 && has("editor") && has("translator")
			&& self.term("editor-translator", TermForm::Long, false).is_some()
		{
			let eds = self.resolve_names("editor");
			let trs = self.resolve_names("translator");
			let mut out = Vec::new();
			if !eds.is_empty() && eds == trs {
				out.push((eds, "editor-translator"));
			} else {
				if !eds.is_empty() {
					out.push((eds, "editor"));
				}
				if !trs.is_empty() {
					out.push((trs, "translator"));
				}
			}
			return out;
		}
		v.vars.iter().map(|x| (self.resolve_names(x), x.as_str())).collect()
	}

	fn render_names_view(&mut self, v: NamesView<'a>) {
		let people = self.people(v);
		let empty = people.iter().all(|(p, _)| p.is_empty());
		if !self.names_pass(v.vars, v.substitute, empty) {
			return;
		}
		if people.iter().any(|(p, _)| !p.is_empty()) {
			self.render_name_list(v, &people);
			return;
		}
		if let Some(sub) = v.substitute {
			self.w.suppressing = true;
			let depth = self.w.push_elem(&v.look.fmt);
			for c in sub {
				let len = self.w.len();
				match c {
					Elem::Names(cn)	=> self.render_names_view(v.substitute_child(cn)),
					_		=> self.render(c),
				}
				if len < self.w.len() {
					break;
				}
			}
			self.w.commit_elem(depth, v.look.display, Some(meta_people(&people)));
			self.w.suppressing = false;
		}
	}

	/// The name options a `name` inherits: the bibliography's or citation's over the style's, and
	/// the delimiter between variables likewise.
	fn inherited_names(&self) -> (NameOpts, Option<&'a str>) {
		let style: &'a Style = self.style;
		// hayagriva's macro sort context stacks the key's options on the style's alone.
		if self.sorting == Some(Sorting::Macro) {
			return (self.sort_names.over(&style.names), style.names_delimiter.as_deref());
		}
		let (o, d) = match (self.bib_names, style.bibliography.as_ref()) {
			(true, Some(b))	=> (&b.names, b.names_delimiter.as_deref()),
			_		=> (&style.citation.names, style.citation.names_delimiter.as_deref()),
		};
		(o.over(&style.names), d.or(style.names_delimiter.as_deref()))
	}

	/// A non-empty name list, each variable's names after the delimiter between variables, with
	/// the label before or after them. B3 adds name disambiguation, which changes the forms.
	fn render_name_list(&mut self, v: NamesView<'a>, people: &[(&'a [Person], &'a str)]) {
		let depth = self.w.push_elem(&v.look.fmt);
		let a = self.w.apply_prefix(v.look.prefix.as_deref());
		let (base, names_delim) = self.inherited_names();
		let plain = Name::default();
		let name = v.name.unwrap_or(&plain);
		let o = NOpts::new(name.opts.over(&v.opts.over(&base)));
		let form = o.form();
		// hayagriva asks the cite whether it is the entry's first; a bibliography inherits the
		// first citation's answer.
		let subsequent = !self.cite.first;
		if form == NForm::Count {
			// Only the names that would be written are counted.
			let n: usize = people.iter()
				.map(|(p, _)| (0..p.len()).filter(|&i| !o.is_suppressed(i, p.len(), subsequent)).count())
				.sum();
			self.push_str(&n.to_string());
			self.apply_suffix(v.look.suffix.as_deref(), a);
			self.w.commit_elem(depth, v.look.display, Some(meta_people(people)));
			return;
		}
		// A disambiguated cite brings its own forms, when they are for these variables.
		let props = match &self.cite.disamb {
			Disamb::Names(p) if p.vars.iter().map(String::as_str).eq(people.iter().map(|(_, v)| *v)) => p.clone(),
			_ => NameDisamb {
				forms:		people.iter().map(|(p, _)| (0..p.len())
					.map(|j| if o.is_suppressed(j, p.len(), subsequent) { None } else { Some(form) })
					.collect()).collect(),
				default:	form,
				vars:		v.vars.to_vec(),
			},
		};
		let listed = people.iter().zip(props.forms.iter()).filter(|((p, _), _)| !p.is_empty());
		for (i, ((persons, var), forms)) in listed.enumerate() {
			let plural = persons.len() != 1;
			if i > 0 {
				let d = v.delimiter.or(names_delim).unwrap_or("");
				if !d.is_empty() {
					self.push_str(d);
				}
			}
			if let (Some(l), true, None) = (v.label, v.label_first, self.sorting) {
				let t = self.term(var, l.form, plural).unwrap_or("");
				self.render_label_with_var(l, t);
			}
			let fi = self.w.push_format(&name.look.fmt);
			self.add_names(v, persons, name, &o, forms, var);
			self.w.pop_format(fi);
			if let (Some(l), false, None) = (v.label, v.label_first, self.sorting) {
				let t = self.term(var, l.form, plural).unwrap_or("");
				self.render_label_with_var(l, t);
			}
		}
		self.apply_suffix(v.look.suffix.as_deref(), a);
		self.w.commit_elem(depth, v.look.display, Some(meta_people(people)));
		if self.w.first_name.is_none() {
			self.w.first_name = Some(props);
		}
	}

	/// A label inside `names`, its term chosen by the caller.
	fn render_label_with_var(&mut self, l: &Label, content: &str) {
		if content.is_empty() {
			return;
		}
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

	fn and_text(&self, a: And) -> &'a str {
		match a {
			And::Text	=> self.term("and", TermForm::Long, false).unwrap_or(""),
			And::Symbol	=> "&",
		}
	}

	/// One variable's names. The "and" goes in only when there is no et al., and an
	/// et-al-use-last list ends in the delimiter, an ellipsis and the last name.
	fn add_names(
		&mut self,
		v:		NamesView<'a>,
		persons:	&[Person],
		name:		&Name,
		o:		&NOpts,
		forms:		&[Option<NForm>],
		var:		&str,
	) {
		let has_et_al = forms.iter().any(Option::is_none);
		let take = forms.iter().position(Option::is_none).unwrap_or(persons.len());
		let use_last = match o.use_last && has_et_al {
			true	=> forms.last().copied().flatten(),
			false	=> None,
		};
		let demote = match self.style.demote_particle {
			Demote::DisplayAndSort	=> true,
			Demote::SortOnly	=> self.sorting.is_some(),
			Demote::Never		=> false,
		};
		let mut last_inverted = false;
		for (i, (p, f)) in persons.iter().take(take).zip(forms).enumerate() {
			let f = match f {
				Some(f)	=> *f,
				None	=> continue,
			};
			if i > 0 {
				let and = match i + 1 == take && !has_et_al {
					true	=> o.and,
					false	=> None,
				};
				match and {
					None => self.push_str(&o.delim),
					Some(and) => {
						let with_delim = match o.prec_last {
							DelimRule::Contextual		=> i >= 2,
							DelimRule::AfterInvertedName	=> last_inverted,
							DelimRule::Always		=> true,
							DelimRule::Never		=> false,
						};
						if with_delim {
							self.push_str(&o.delim);
						} else {
							self.push_str(" ");
						}
						let t = self.and_text(and);
						self.push_str(t);
						self.w.ensure_space();
					}
				}
			}
			let reverse = match o.sort_order {
				Some(SortOrder::First)	=> i == 0,
				Some(SortOrder::All)	=> true,
				None			=> false,
			};
			self.write_name(p, f, reverse, demote, name, o, var, i);
			last_inverted = reverse;
		}
		if let Some(f) = use_last {
			if let Some(p) = persons.last() {
				self.push_str(&o.delim);
				self.push_str("… ");
				let reverse = o.sort_order == Some(SortOrder::All);
				self.write_name(p, f, reverse, demote, name, o, var, persons.len() - 1);
			}
		} else if has_et_al && self.sorting.is_none() {
			let (tname, fmt) = match v.et_al {
				Some(e)	=> (e.term.as_str(), e.look.fmt),
				None	=> ("et-al", Fmt::default()),
			};
			if let Some(t) = self.term(tname, TermForm::Long, false) {
				let with_delim = match o.prec_et_al {
					DelimRule::Always		=> true,
					DelimRule::Contextual		=> take >= 2,
					DelimRule::AfterInvertedName	=> last_inverted,
					DelimRule::Never		=> false,
				};
				if with_delim {
					self.push_str(&o.delim);
				}
				let fi = self.w.push_format(&fmt);
				self.w.ensure_space();
				self.push_str(t);
				self.w.pop_format(fi);
			}
		}
	}

	/// The given name: initials with `initialize-with` when initialising, else the given names
	/// joined by it.
	fn first_name(&mut self, p: &Person, form: NForm, o: &NOpts) {
		let given = match &p.given {
			Some(g)	=> g,
			None	=> return,
		};
		let hyphen = self.style.initialize_with_hyphen;
		match &o.init_with {
			Some(w) if form == NForm::LongInit	=> { let _ = p.initials(self, Some(w), hyphen); }
			Some(w)					=> { let _ = p.given_with_delimiter(self, Some(w)); }
			None					=> self.push_str(given),
		}
	}

	/// Opens a name part's formatting and case and writes its prefix; `close_part` ends it.
	fn open_part(&mut self, part: Option<&NamePart>, cased: bool) -> (usize, usize) {
		let fi = self.w.push_format(&part.map(|x| x.look.fmt).unwrap_or_default());
		let ci = self.push_case(if cased { part.and_then(|x| x.look.text_case) } else { None });
		if let Some(pre) = part.and_then(|x| x.look.prefix.as_deref()) {
			self.push_str(pre);
		}
		(fi, ci)
	}

	fn close_part(&mut self, (fi, ci): (usize, usize)) {
		self.w.pop_case(ci);
		self.w.pop_format(fi);
	}

	fn part_suffix(&mut self, part: Option<&NamePart>) {
		if let Some(suf) = part.and_then(|x| x.look.suffix.as_deref()) {
			self.push_str(suf);
		}
	}

	/// One name in its own element. Institutional and short names are the family part alone, a
	/// long CJK name is family then given, unspaced, and a reversed name is family first after
	/// which the sort separator.
	#[allow(clippy::too_many_arguments)]
	fn write_name(
		&mut self,
		p:		&Person,
		form:		NForm,
		reverse:	bool,
		demote:		bool,
		name:		&Name,
		o:		&NOpts,
		var:		&str,
		idx:		usize,
	) {
		let giv = name.parts.iter().find(|x| x.part == PartName::Given);
		let fam = name.parts.iter().find(|x| x.part == PartName::Family);
		let loc = self.w.push_elem(&Fmt::default());
		let long = matches!(form, NForm::LongInit | NForm::LongFull);
		if p.is_institutional() && self.sorting.is_some() {
			let s = self.open_part(fam, true);
			self.push_str(p.family_without_article());
			self.close_part(s);
		} else if p.is_institutional() || !long {
			let s = self.open_part(fam, true);
			self.push_str(&p.family);
			self.close_part(s);
			self.part_suffix(fam);
		} else if p.is_cjk() {
			// hayagriva applies neither part's case here.
			let s = self.open_part(fam, false);
			self.push_str(&p.family);
			self.close_part(s);
			self.part_suffix(fam);
			if let Some(g) = &p.given {
				let s = self.open_part(giv, false);
				self.push_str(g);
				self.close_part(s);
				self.part_suffix(giv);
			}
		} else if reverse || self.sorting.is_some() {
			let s = self.open_part(fam, true);
			self.push_str(if demote { p.family_without_particle() } else { p.family.as_str() });
			self.close_part(s);
			self.part_suffix(fam);
			if p.given.is_some() {
				self.push_str(&o.sort_sep);
				let s = self.open_part(giv, true);
				self.first_name(p, form, o);
				if let Some(pt) = &p.particle {
					self.w.ensure_space();
					self.push_str(pt);
				}
				self.close_part(s);
				if demote {
					if let Some(pt) = p.particle_in_family() {
						self.w.ensure_space();
						self.push_str(pt);
					}
				}
				self.part_suffix(giv);
			}
			if let Some(s) = &p.suffix {
				self.push_str(&o.sort_sep);
				self.push_str(s);
			}
		} else {
			let s = self.open_part(giv, true);
			self.first_name(p, form, o);
			if let Some(pt) = &p.particle {
				self.w.ensure_space();
				self.push_str(pt);
			}
			self.close_part(s);
			self.part_suffix(giv);
			self.w.ensure_space();
			let s = self.open_part(fam, true);
			self.push_str(&p.family);
			self.close_part(s);
			if let Some(sx) = &p.suffix {
				if p.comma_suffix {
					self.push_str(&o.sort_sep);
				}
				self.w.ensure_space();
				self.push_str(sx);
			}
			// The family part's suffix follows the name's suffix (hayagriva).
			self.part_suffix(fam);
		}
		self.w.commit_elem(loc, None, Some(Meta::Name(var.to_string(), idx)));
	}

	/// hayagriva's `renders_given_special_form`: VarOnly passes a `names` holding its variable,
	/// directly or through the substitute of an empty one; SuppressAuthor stops one that would
	/// render the author; the date forms stop every `names`.
	fn names_pass(&mut self, vars: &'a [String], sub: Option<&'a [Elem]>, empty: bool) -> bool {
		match self.special.clone() {
			Some(Special::VarOnly(var)) => {
				let has = vars.iter().any(|v| *v == var);
				let in_sub = empty && sub.map_or(false, |s| s.iter().any(|c| match c {
					Elem::Names(n)	=> n.vars.iter().any(|v| *v == var),
					_		=> false,
				}));
				has || in_sub
			}
			Some(Special::OnlyFirstDate | Special::OnlyYearSuffix) => false,
			Some(Special::SuppressAuthor) => !self.names_will_render(vars, sub, "author"),
			None => true,
		}
	}

	fn names_info(&mut self, n: &'a Names) -> (bool, Usage) {
		let s = self.w.suppressing;
		self.w.suppressing = false;
		let counts: Vec<usize> = n.vars.iter().map(|v| self.resolve_names(v).len()).collect();
		if !self.names_pass(&n.vars, n.substitute.as_deref(), counts.iter().all(|&c| c == 0)) {
			self.w.suppressing = s;
			return (false, Usage::default());
		}
		let mut empty = counts.iter().all(|&c| c == 0);
		let mut sub = (false, Usage::default());
		if let Some(elems) = &n.substitute {
			for c in elems {
				let (p, i) = self.info(c);
				sub = (sub.0 || p, sub.1.merge(i));
			}
		}
		// A name element showing none of an et-al list counts as empty. So, as in hayagriva, does
		// a `names` with children but no `name`: AMA's `<names variable="editor"><label/></names>`
		// prints nothing, though CSL would print the names with an implicit `name`.
		match &n.name {
			Some(name) => if let (Some(min), Some(0)) = (name.opts.et_al_min, name.opts.et_al_use_first) {
				if counts.iter().all(|&c| c >= min as usize) {
					empty = true;
				}
			},
			None => if n.et_al.is_some() || n.label.is_some() || n.substitute.is_some() {
				empty = true;
			},
		}
		self.w.suppressing = s;
		let visible = !empty || sub.0;
		(visible, if empty { sub.1 } else { Usage { vars: true, non_empty: true, ..Usage::default() } })
	}

	/// `date`. hayagriva renders no date ranges (a BibTeX range reaches it as its end date) and
	/// nothing for circa, which only answers `is-uncertain-date`. A localised form takes the
	/// locale's parts and delimiter, each part overridden by the style's part of the same name,
	/// and the cs:date's own delimiter is ignored, as CSL 1.0.2 says.
	fn render_date(&mut self, d: &'a Date) {
		if d.var.is_empty() || !self.date_passes(&d.var) {
			return;
		}
		let date = match self.resolve_date(&d.var) {
			Some(x)	=> x,
			None	=> return,
		};
		if let Some(sorting) = self.sorting {
			// citeproc's trick, as hayagriva has it: BC years as N(i32::MAX + y), AD as Py.
			let has = |n: DatePartName| d.parts.iter().any(|p| p.name == n);
			let (y, m, dd) = match (d.form, sorting) {
				(Some(_), _) => match d.parts_sel {
					Some(DateParts::Year)		=> (true, false, false),
					Some(DateParts::YearMonth)	=> (true, true, false),
					_				=> (true, true, true),
				},
				(None, Sorting::Variable)	=> (true, true, true),
				(None, Sorting::Macro)		=> (has(DatePartName::Year), has(DatePartName::Month), has(DatePartName::Day)),
			};
			let mut k = String::new();
			if y {
				let _ = match date.year < 0 {
					true	=> write!(k, "N{:09}", i32::MAX + date.year),
					false	=> write!(k, "P{:09}", date.year),
				};
			}
			if m {
				let _ = write!(k, "{:02}", date.month.unwrap_or(0));
			}
			if dd {
				let _ = write!(k, "{:02}", date.day.unwrap_or(0));
			}
			self.push_str(&k);
			return;
		}
		let base: Option<&'a Date> = match d.form {
			// A locale without the form prints nothing.
			Some(f) => match self.chain.date(f) {
				Some(b)	=> Some(b),
				None	=> return,
			},
			None => None,
		};
		let first = !self.w.dates_seen;
		self.w.dates_seen = true;
		let fmt = match base {
			Some(b)	=> fmt_over(&d.look.fmt, &b.look.fmt),
			None	=> d.look.fmt,
		};
		let depth = self.w.push_elem(&fmt);
		let a = self.w.apply_prefix(d.look.prefix.as_deref());
		let cidx = self.push_case(d.look.text_case.or(base.and_then(|b| b.look.text_case)));
		let sel = d.parts_sel.or(base.and_then(|b| b.parts_sel)).unwrap_or(DateParts::YearMonthDay);
		let delim = match base {
			Some(b)	=> b.delimiter.as_deref(),
			None	=> d.delimiter.as_deref(),
		};
		let mut last_empty = true;
		for part in &base.unwrap_or(d).parts {
			match part.name {
				DatePartName::Month if sel == DateParts::Year && date.season.is_none()	=> continue,
				DatePartName::Day if sel != DateParts::YearMonthDay			=> continue,
				_									=> {}
			}
			let cursor = self.w.len();
			if !last_empty {
				if let Some(dl) = delim {
					self.push_str(dl);
				}
			}
			let over = match base {
				Some(_)	=> d.parts.iter().find(|p| p.name == part.name),
				None	=> None,
			};
			self.render_date_part(part, &date, over, first);
			last_empty = cursor == self.w.len();
		}
		self.w.pop_case(cidx);
		self.apply_suffix(d.look.suffix.as_deref(), a);
		self.w.commit_elem(depth, d.look.display, Some(Meta::Date));
	}

	/// One date part. A style part overriding a localised one replaces its form whole (an unset
	/// form being the default), lays its formatting over it and wins on case; the affixes and
	/// strip-periods stay the locale's.
	fn render_date_part(&mut self, part: &DatePart, date: &DateVal, over: Option<&DatePart>, first: bool) {
		let val = match part.name {
			DatePartName::Day	=> date.day.map(i32::from),
			DatePartName::Month	=> date.month.or(date.season).map(i32::from),
			DatePartName::Year	=> Some(date.year),
		};
		let val = match val {
			Some(v)	=> v,
			None	=> return,
		};
		let form = part_form(over.unwrap_or(part));
		let fmt = match over {
			Some(o)	=> fmt_over(&o.look.fmt, &part.look.fmt),
			None	=> part.look.fmt,
		};
		let fi = self.w.push_format(&fmt);
		// The year-suffix form writes only the suffix, without the part's affixes.
		let only_suffix = self.special == Some(Special::OnlyYearSuffix);
		let a = match only_suffix {
			true	=> None,
			false	=> Some(self.w.apply_prefix(part.look.prefix.as_deref())),
		};
		if part.name == DatePartName::Month {
			self.w.strip_periods = part.look.strip_periods;
		}
		let cidx = self.push_case(over.and_then(|o| o.look.text_case).or(part.look.text_case));
		match (part.name, form) {
			_ if only_suffix => {}
			(DatePartName::Month, _) if date.month.is_none() => {
				// A season. hayagriva returns early when the locale lacks the term, leaving its
				// stacks pushed; here the part is closed as usual.
				if let Some(t) = self.term(&format!("season-{:02}", val), TermForm::Short, false) {
					self.push_str(t);
				}
			}
			(DatePartName::Year, DatePartForm::Short) if val >= 1000 => {
				self.push_str(&format!("{:02}", val % 100));
			}
			(DatePartName::Year, _) => {
				let mut s = if val > 0 { val.to_string() } else { (val.abs() + 1).to_string() };
				if val < 1000 {
					let era = match val <= 0 {
						true	=> self.term("bc", TermForm::Long, false).unwrap_or(" BC"),
						false	=> self.term("ad", TermForm::Long, false).unwrap_or(" AD"),
					};
					s.push_str(era);
				}
				self.push_str(&s);
			}
			(_, DatePartForm::NumericLeadingZeros) => self.push_str(&format!("{:02}", val)),
			(DatePartName::Day, DatePartForm::Ordinal) if val == 1 || !self.chain.limit_ordinals_to_day_1() => {
				let gender = date.month
					.and_then(|m| self.chain.term(&format!("month-{:02}", m), TermForm::Long))
					.and_then(|t| t.gender);
				let ords = Ords::new(self.chain);
				self.push_str(&val.to_string());
				self.push_str(ords.lookup(val, gender).unwrap_or(""));
			}
			(DatePartName::Month, DatePartForm::Long | DatePartForm::Short) => {
				let tf = if form == DatePartForm::Long { TermForm::Long } else { TermForm::Short };
				match self.term(&format!("month-{:02}", val), tf, false) {
					Some(t)	=> self.push_str(t),
					None	=> self.push_str(&val.to_string()),
				}
			}
			_ => self.push_str(&val.to_string()),
		}
		if part.name == DatePartName::Year && first && self.year_suffix_implicit() {
			if let Some(c) = self.resolve_std("year-suffix", VarForm::Long) {
				self.push_chunked(&c);
			}
		}
		if let Some(a) = a {
			self.apply_suffix(part.look.suffix.as_deref(), a);
		}
		self.w.strip_periods = false;
		self.w.pop_case(cidx);
		self.w.pop_format(fi);
	}

	/// hayagriva's `Date::will_render` for its own variable: VarOnly passes only that date, and
	/// the date forms only the first date.
	fn date_passes(&self, var: &str) -> bool {
		match &self.special {
			Some(Special::VarOnly(v))					=> v == var,
			Some(Special::OnlyFirstDate | Special::OnlyYearSuffix)	=> !self.w.dates_seen,
			_								=> true,
		}
	}

	/// Is the year-suffix written after the first date's year, because neither layout writes it?
	fn year_suffix_implicit(&mut self) -> bool {
		let style: &'a Style = self.style;
		if self.will_render_all(&style.citation.layout.elems, "year-suffix") {
			return false;
		}
		!style.bibliography.as_ref().map_or(false, |b| self.will_render_all(&b.layout.elems, "year-suffix"))
	}

	fn will_render_all(&mut self, elems: &'a [Elem], var: &str) -> bool {
		elems.iter().any(|e| self.will_render(e, var))
	}

	/// Would the element render the variable? hayagriva's `will_render`: a text resolves its
	/// target, a group any child, a choose its chosen branch, a label never.
	pub fn will_render(&mut self, e: &'a Elem, var: &str) -> bool {
		match e {
			Elem::Text(t) => match self.target(t) {
				Some(Target::Std(v, _) | Target::Num(v, _))	=> v == var,
				Some(Target::Page(_))				=> var == "page",
				Some(Target::Macro(m))				=> self.will_render_all(m, var),
				_						=> false,
			},
			Elem::Number(n)	=> self.number_passes(&n.var) && n.var == var,
			Elem::Label(_)	=> false,
			Elem::Names(n)	=> self.names_will_render(&n.vars, n.substitute.as_deref(), var),
			Elem::Date(d)	=> match &self.special {
				Some(Special::OnlyFirstDate | Special::OnlyYearSuffix)	=> !self.w.dates_seen,
				_							=> self.date_passes(&d.var) && d.var == var,
			},
			Elem::Group(g)	=> self.will_render_all(&g.elems, var),
			Elem::Choose(c)	=> match self.choose_branch(c) {
				Some(b)	=> self.will_render_all(b, var),
				None	=> false,
			},
		}
	}

	fn names_will_render(&mut self, vars: &'a [String], sub: Option<&'a [Elem]>, var: &str) -> bool {
		if vars.iter().any(|v| v == var) {
			return true;
		}
		match sub {
			Some(s) if vars.iter().all(|v| self.item.names(v).is_empty()) => self.will_render_all(s, var),
			_ => false,
		}
	}

	/// The date is queried after suppression is restored, so it is recorded (hayagriva).
	fn date_info(&mut self, d: &'a Date) -> (bool, Usage) {
		if d.var.is_empty() {
			return (false, Usage { vars: true, ..Usage::default() });
		}
		// A special form that will not render this date leaves no trace of it.
		if !self.date_passes(&d.var) {
			return (false, Usage::default());
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
				*d == (self.cite.disamb == Disamb::Choose)
			}
			Test::IsNumeric(v) => match var_kind(v) {
				VarKind::Standard	=> self.resolve_std(v, VarForm::Long).map_or(false, |c| Numeric::parse(&c.to_string()).is_some()),
				VarKind::Number		=> matches!(self.resolve_num(v), Some(NumVal::Typed(_))),
				VarKind::Page		=> matches!(self.resolve_page(), Some(PageVal::Typed(_))),
				_			=> false,
			},
			Test::IsUncertainDate(v)	=> self.resolve_date(v).map_or(false, |d| d.approximate),
			Test::Locator(l)		=> self.cite.locator.as_ref().map_or(false, |(k, _)| k == l),
			Test::Position(p) => match p {
				Position::First			=> self.cite.first,
				Position::Subsequent		=> !self.cite.first,
				Position::Ibid			=> self.ibid() != Ibid::No,
				Position::IbidWithLocator	=> self.ibid() == Ibid::WithLocator,
				Position::NearNote		=> self.cite.near_note,
			},
			Test::Type(ty)	=> self.item.is_type(ty),
			Test::Variable(v) => match var_kind(v) {
				VarKind::Standard	=> self.resolve_std(v, VarForm::Long).map_or(false, |c| !c.is_blank()),
				VarKind::Number		=> self.resolve_num(v).is_some(),
				VarKind::Date		=> self.resolve_date(v).is_some(),
				VarKind::Name		=> !self.resolve_names(v).is_empty(),
				VarKind::Page		=> self.resolve_page().is_some(),
			},
		}
	}
}
