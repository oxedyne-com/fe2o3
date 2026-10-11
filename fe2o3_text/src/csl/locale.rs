//! CSL locales and the order they are consulted in.
//!
//! The order is hayagriva 0.10.1's (`lookup_locale`), which Typst 0.15.1 shows and which differs from
//! CSL 1.0.2 §Locale Fallback in two places. The in-style step tries the requested code, then its
//! two-letter language, then a `locale` without `xml:lang`. The file step tries the requested code,
//! then the primary dialect of its language, then `en-US`. The primary dialect comes from
//! citationberg's table, not from `locales.json`: the two agree except that the table has no entry
//! for `ar`, `cy`, `gl`, `hi`, `hy`, `id`, `ms`, `tl` and the three-letter codes, and gives `sr` as
//! `sr-RS`, a file hayagriva does not serve. Each of those therefore falls to `en-US` in Typst, and
//! here.

use crate::{
	csl::style::{
		Date,
		Rd,
		Style,
		TermForm,
	},
	xml::{
		Elem as XElem,
		Xml,
	},
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeMap;


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gender { Masculine, Feminine, Neuter }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrdMatch { LastDigit, LastTwoDigits, WholeNumber }

#[derive(Clone, Debug, PartialEq)]
pub struct Term {
	pub name:	String,
	pub form:	TermForm,
	pub gender:	Option<Gender>,	// of a noun term such as `month-01`
	pub gender_form:Option<Gender>,	// of an ordinal suffix
	pub matching:	Option<OrdMatch>,
	pub single:	String,
	pub multiple:	Option<String>,
}

impl Term {
	pub fn text(&self, plural: bool) -> &str {
		match (plural, &self.multiple) {
			(true, Some(m))	=> m,
			_		=> &self.single,
		}
	}
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Locale {
	pub lang:			Option<String>,
	pub terms:			Vec<Term>,
	pub dates:			Vec<Date>,	// each carries its `form`
	pub punctuation_in_quote:	Option<bool>,
	pub limit_ordinals_to_day_1:	Option<bool>,
}

impl Locale {

	/// Reads a standalone locale file.
	pub fn parse(src: &str) -> Outcome<Self> {
		let xml = res!(Xml::parse(src));
		let root = res!(xml.root());
		if root.name.local() != "locale" {
			return Err(err!("CSL locale: the root element is <{}>, not <locale>.", root.name.qname; Invalid, Input));
		}
		Self::read(&xml, root)
	}

	/// Reads a `locale` element, standalone or inside a style.
	pub(crate) fn read(xml: &Xml, e: &XElem) -> Outcome<Self> {
		let rd = Rd { xml };
		let mut out = Self {
			lang: e.attr("xml:lang").map(str::to_string),
			..Default::default()
		};
		for k in e.elems() {
			match k.name.local() {
				"style-options"	=> {
					out.punctuation_in_quote = res!(rd.bool(k, "punctuation-in-quote"));
					out.limit_ordinals_to_day_1 = res!(rd.bool(k, "limit-day-ordinals-to-day-1"));
				}
				"date"		=> out.dates.push(res!(rd.date(k, true))),
				"terms"		=> for t in k.children("term") {
					out.terms.push(res!(Self::term(&rd, t)));
				},
				_		=> {} // info
			}
		}
		Ok(out)
	}

	fn term(rd: &Rd, t: &XElem) -> Outcome<Term> {
		let gender = |attr: &str| -> Outcome<Option<Gender>> {
			Ok(match t.attr(attr) {
				None			=> None,
				Some("masculine")	=> Some(Gender::Masculine),
				Some("feminine")	=> Some(Gender::Feminine),
				Some("neuter")		=> Some(Gender::Neuter),
				Some(v)			=> return Err(rd.bad(t, attr, v)),
			})
		};
		let (single, multiple) = match (t.child("single"), t.child("multiple")) {
			(None, None)	=> (rd.xml.text_of(t).trim().to_string(), None),
			(s, m)		=> (
				s.map(|s| rd.xml.text_of(s).trim().to_string()).unwrap_or_default(),
				m.map(|m| rd.xml.text_of(m).trim().to_string()),
			),
		};
		Ok(Term {
			name:		res!(rd.req(t, "name")).to_string(),
			form:		res!(rd.term_form(t)),
			gender:		res!(gender("gender")),
			gender_form:	res!(gender("gender-form")),
			matching:	match t.attr("match") {
				None			=> None,
				Some("last-digit")	=> Some(OrdMatch::LastDigit),
				Some("last-two-digits")	=> Some(OrdMatch::LastTwoDigits),
				Some("whole-number")	=> Some(OrdMatch::WholeNumber),
				Some(v)			=> return Err(rd.bad(t, "match", v)),
			},
			single,
			multiple,
		})
	}
}

/// The primary dialect of a language, as citationberg 0.7 gives it.
pub fn primary_dialect(code: &str) -> Option<&'static str> {
	let base = res_base(code)?;
	let dialect = PRIMARY.iter().find(|(l, _)| *l == base).map(|(_, d)| *d)?;
	match dialect == code {
		true	=> None,
		false	=> Some(dialect),
	}
}

/// The two-letter base language of a code, which is all citationberg reads a fallback from.
fn res_base(code: &str) -> Option<&str> {
	let first = code.split('-').next()?;
	match first.len() {
		2	=> Some(first),
		_	=> None,
	}
}

const PRIMARY: &[(&str, &str)] = &[
	("af", "af-ZA"), ("bg", "bg-BG"), ("ca", "ca-AD"), ("cs", "cs-CZ"), ("da", "da-DK"), ("de", "de-DE"),
	("el", "el-GR"), ("en", "en-US"), ("es", "es-ES"), ("et", "et-EE"), ("fa", "fa-IR"), ("fi", "fi-FI"),
	("fr", "fr-FR"), ("he", "he-IL"), ("hr", "hr-HR"), ("hu", "hu-HU"), ("is", "is-IS"), ("it", "it-IT"),
	("ja", "ja-JP"), ("km", "km-KH"), ("ko", "ko-KR"), ("lt", "lt-LT"), ("lv", "lv-LV"), ("mn", "mn-MN"),
	("nb", "nb-NO"), ("nl", "nl-NL"), ("nn", "nn-NO"), ("pl", "pl-PL"), ("pt", "pt-PT"), ("ro", "ro-RO"),
	("ru", "ru-RU"), ("sk", "sk-SK"), ("sl", "sl-SI"), ("sr", "sr-RS"), ("sv", "sv-SE"), ("th", "th-TH"),
	("tr", "tr-TR"), ("uk", "uk-UA"), ("vi", "vi-VN"), ("zh", "zh-CN"),
];

/// Locale files by code: the external half of the chain.
#[derive(Clone, Debug, Default)]
pub struct Locales {
	pub files: BTreeMap<String, Locale>,
}

impl Locales {

	pub fn new() -> Self {
		Self::default()
	}

	/// Adds a parsed file under its own `xml:lang`.
	pub fn insert(&mut self, loc: Locale) -> Outcome<()> {
		let code = res!(loc.lang.clone().ok_or_else(|| err!(
			"CSL locale file has no xml:lang on its <locale>."; Invalid, Input, Missing)));
		self.files.insert(code, loc);
		Ok(())
	}

	/// The locale files a style rendered in this code may consult, in order: what a host that
	/// supplies files on demand must fetch.
	pub fn wanted(style: &Style, code: Option<&str>) -> Vec<String> {
		let code = chosen(style, code);
		let mut out = vec![code.clone()];
		if let Some(p) = primary_dialect(&code) {
			out.push(p.to_string());
		}
		if !out.iter().any(|c| c == "en-US") {
			out.push("en-US".to_string());
		}
		out
	}

	/// The chain a style rendered in this code consults. `None` means the style's default locale, or
	/// `en-US` where it names none.
	pub fn chain<'a>(&'a self, style: &'a Style, code: Option<&str>) -> Chain<'a> {
		let code = chosen(style, code);
		let mut locs: Vec<&'a Locale> = Vec::new();
		let push = |l: &'a Locale, locs: &mut Vec<&'a Locale>| {
			if !locs.iter().any(|x| std::ptr::eq(*x, l)) {
				locs.push(l);
			}
		};
		let base = res_base(&code).map(str::to_string);
		// Only the first in-style locale of each code is consulted, as in hayagriva.
		let in_style = |want: Option<&str>| style.locales.iter().find(|l| l.lang.as_deref() == want);
		if let Some(l) = in_style(Some(&code)) { push(l, &mut locs) }
		if let Some(b) = &base {
			if let Some(l) = in_style(Some(b)) { push(l, &mut locs) }
		}
		if let Some(l) = in_style(None) { push(l, &mut locs) }
		if let Some(l) = self.files.get(&code) { push(l, &mut locs) }
		if let Some(p) = primary_dialect(&code) {
			if let Some(l) = self.files.get(p) { push(l, &mut locs) }
		}
		if let Some(l) = self.files.get("en-US") { push(l, &mut locs) }
		Chain { code, locs }
	}
}

fn chosen(style: &Style, code: Option<&str>) -> String {
	match code {
		Some(c)	=> c.to_string(),
		None	=> style.default_locale.clone().unwrap_or_else(|| "en-US".to_string()),
	}
}

/// The locales one rendering consults, most specific first.
#[derive(Clone, Debug)]
pub struct Chain<'a> {
	pub code:	String,
	pub locs:	Vec<&'a Locale>,
}

impl<'a> Chain<'a> {

	/// The first answer any locale in the chain gives.
	pub fn first<R, F: FnMut(&'a Locale) -> Option<R>>(&self, mut f: F) -> Option<R> {
		self.locs.iter().find_map(|l| f(l))
	}

	/// A term by name and form, falling back through the forms (symbol to short to long, verb-short
	/// to verb to long) only once no locale has the asked form.
	pub fn term(&self, name: &str, form: TermForm) -> Option<&'a Term> {
		let mut form = Some(form);
		while let Some(f) = form {
			let hit = self.first(|l| l.terms.iter().find(|t| t.name == name && t.form == f));
			if hit.is_some() {
				return hit;
			}
			form = f.fallback();
		}
		None
	}

	/// The term's text, singular or plural.
	pub fn term_text(&self, name: &str, form: TermForm, plural: bool) -> Option<&'a str> {
		self.term(name, form).map(|t| t.text(plural))
	}

	/// A localised date format.
	pub fn date(&self, form: crate::csl::style::DateForm) -> Option<&'a Date> {
		self.first(|l| l.dates.iter().find(|d| d.form == Some(form)))
	}

	pub fn punctuation_in_quote(&self) -> bool {
		self.first(|l| l.punctuation_in_quote).unwrap_or(false)
	}

	pub fn limit_ordinals_to_day_1(&self) -> bool {
		self.first(|l| l.limit_ordinals_to_day_1).unwrap_or(false)
	}
}
