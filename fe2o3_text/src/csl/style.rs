use crate::{
	csl::locale::Locale,
	xml::{
		Elem as XElem,
		Xml,
	},
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeMap;


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
	InText,
	Note,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
	pub id:		String,
	pub title:	String,
	pub updated:	String,
	pub rights:	Option<String>,
	pub parent:	Option<String>,	// `independent-parent` link of a dependent style
}

// Formatting
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontStyle { Normal, Italic, Oblique }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontVariant { Normal, SmallCaps }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontWeight { Normal, Bold, Light }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decoration { None, Underline }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VAlign { Baseline, Sup, Sub }

/// The CSL formatting attributes of one element. `None` leaves the surrounding formatting as it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fmt {
	pub style:	Option<FontStyle>,
	pub variant:	Option<FontVariant>,
	pub weight:	Option<FontWeight>,
	pub deco:	Option<Decoration>,
	pub valign:	Option<VAlign>,
}

impl Fmt {
	pub fn is_plain(&self) -> bool {
		*self == Self::default()
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Display { Block, LeftMargin, RightInline, Indent }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextCase { Lowercase, Uppercase, CapitaliseFirst, CapitaliseAll, Sentence, Title }

/// What every rendering element may carry besides its own attributes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
	pub fmt:		Fmt,
	pub prefix:		Option<String>,
	pub suffix:		Option<String>,
	pub display:		Option<Display>,
	pub quotes:		bool,
	pub strip_periods:	bool,
	pub text_case:		Option<TextCase>,
}

// Elements
#[derive(Clone, Debug, PartialEq)]
pub enum Elem {
	Text(Text),
	Number(Number),
	Label(Label),
	Names(Names),
	Date(Date),
	Group(Group),
	Choose(Choose),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarForm { Long, Short }

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TermForm { Long, Short, Verb, VerbShort, Symbol }

impl TermForm {
	/// The next form to try where a locale lacks this one: symbol and short fall to long by way of
	/// short, verb-short to verb, verb to long.
	pub fn fallback(self) -> Option<Self> {
		match self {
			Self::Long	=> None,
			Self::Short	=> Some(Self::Long),
			Self::Verb	=> Some(Self::Long),
			Self::VerbShort	=> Some(Self::Verb),
			Self::Symbol	=> Some(Self::Short),
		}
	}
}

#[derive(Clone, Debug, PartialEq)]
pub enum TextSrc {
	Var { name: String, form: VarForm },
	Macro(String),
	Term { name: String, form: TermForm, plural: bool },
	Value(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Text {
	pub src:	TextSrc,
	pub look:	Look,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumForm { Numeric, Ordinal, LongOrdinal, Roman }

#[derive(Clone, Debug, PartialEq)]
pub struct Number {
	pub var:	String,
	pub form:	NumForm,
	pub look:	Look,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plural { Contextual, Always, Never }

#[derive(Clone, Debug, PartialEq)]
pub struct Label {
	pub var:	String,	// empty inside `names`, where the names' variable is meant
	pub form:	TermForm,
	pub plural:	Plural,
	pub look:	Look,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum And { Text, Symbol }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DelimRule { Contextual, AfterInvertedName, Always, Never }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameForm { Long, Short, Count }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortOrder { First, All }

/// The inheritable name options. `None` defers to the level above: the `name` element, then the
/// citation or bibliography, then the style.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NameOpts {
	pub and:			Option<And>,
	pub delimiter:			Option<String>,
	pub delim_precedes_et_al:	Option<DelimRule>,
	pub delim_precedes_last:	Option<DelimRule>,
	pub et_al_min:			Option<u32>,
	pub et_al_use_first:		Option<u32>,
	pub et_al_subsequent_min:	Option<u32>,
	pub et_al_subsequent_use_first:	Option<u32>,
	pub et_al_use_last:		Option<bool>,
	pub form:			Option<NameForm>,
	pub initialize:			Option<bool>,
	pub initialize_with:		Option<String>,
	pub name_as_sort_order:		Option<SortOrder>,
	pub sort_separator:		Option<String>,
}

macro_rules! over {
	($me:ident, $base:ident; $($f:ident),*) => {
		NameOpts { $($f: match &$me.$f { Some(v) => Some(v.clone()), None => $base.$f.clone() },)* }
	};
}

impl NameOpts {
	/// These options laid over a base: each set field wins.
	pub fn over(&self, base: &NameOpts) -> NameOpts {
		over!(self, base; and, delimiter, delim_precedes_et_al, delim_precedes_last, et_al_min,
			et_al_use_first, et_al_subsequent_min, et_al_subsequent_use_first, et_al_use_last, form,
			initialize, initialize_with, name_as_sort_order, sort_separator)
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartName { Given, Family }

#[derive(Clone, Debug, PartialEq)]
pub struct NamePart {
	pub part:	PartName,
	pub look:	Look,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Name {
	pub opts:	NameOpts,
	pub parts:	Vec<NamePart>,
	pub look:	Look,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EtAl {
	pub term:	String,	// `et-al` or `and others`
	pub look:	Look,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Names {
	pub vars:		Vec<String>,
	pub delimiter:		Option<String>,
	pub name:		Option<Name>,
	pub et_al:		Option<EtAl>,
	pub label:		Option<Label>,
	pub label_first:	bool,	// the label was written before the name
	pub substitute:		Option<Vec<Elem>>,
	pub look:		Look,
}

impl Names {
	/// Does this element give no `name`, `et-al` or `label` of its own, so that inside a
	/// `substitute` it takes its parent's?
	pub fn is_bare(&self) -> bool {
		self.name.is_none() && self.et_al.is_none() && self.label.is_none()
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateForm { Text, Numeric }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateParts { YearMonthDay, YearMonth, Year }
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DatePartName { Day, Month, Year }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatePartForm { Numeric, NumericLeadingZeros, Ordinal, Long, Short }

#[derive(Clone, Debug, PartialEq)]
pub struct DatePart {
	pub name:		DatePartName,
	pub form:		Option<DatePartForm>,
	pub range_delimiter:	Option<String>,
	pub look:		Look,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Date {
	pub var:	String,
	pub form:	Option<DateForm>,	// localised where set
	pub parts_sel:	Option<DateParts>,	// `None` takes the localised form's, then all three
	pub parts:	Vec<DatePart>,
	pub delimiter:	Option<String>,
	pub look:	Look,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
	pub delimiter:	Option<String>,
	pub elems:	Vec<Elem>,
	pub look:	Look,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Match { All, Any, None }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position { First, Subsequent, IbidWithLocator, Ibid, NearNote }

#[derive(Clone, Debug, PartialEq)]
pub enum Test {
	Disambiguate(bool),
	IsNumeric(String),
	IsUncertainDate(String),
	Locator(String),
	Position(Position),
	Type(String),
	Variable(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cond {
	pub mode:	Match,
	pub tests:	Vec<Test>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Branch {
	pub cond:	Option<Cond>,	// `None` for `else`
	pub elems:	Vec<Elem>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Choose {
	pub branches:	Vec<Branch>,
}

// Sorting
#[derive(Clone, Debug, PartialEq)]
pub enum SortSrc {
	Var(String),
	Macro(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SortKey {
	pub src:		SortSrc,
	pub ascending:		bool,
	pub names_min:		Option<u32>,
	pub names_use_first:	Option<u32>,
	pub names_use_last:	Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sort {
	pub keys:	Vec<SortKey>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
	pub delimiter:	Option<String>,
	pub elems:	Vec<Elem>,
	pub look:	Look,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Collapse { CitationNumber, Year, YearSuffix, YearSuffixRanged }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GivenRule { AllNames, AllNamesWithInitials, PrimaryName, PrimaryNameWithInitials, ByCite }

#[derive(Clone, Debug, PartialEq)]
pub struct Citation {
	pub names:			NameOpts,
	pub names_delimiter:		Option<String>,
	pub sort:			Option<Sort>,
	pub layout:			Layout,
	pub add_names:			bool,
	pub add_givenname:		bool,
	pub givenname_rule:		GivenRule,
	pub add_year_suffix:		bool,
	pub cite_group_delimiter:	Option<String>,
	pub collapse:			Option<Collapse>,
	pub year_suffix_delimiter:	Option<String>,
	pub after_collapse_delimiter:	Option<String>,
	pub near_note_distance:		u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecondField { Flush, Margin }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubRule { CompleteAll, CompleteEach, PartialEach, PartialFirst }

#[derive(Clone, Debug, PartialEq)]
pub struct Bibliography {
	pub names:		NameOpts,
	pub names_delimiter:	Option<String>,
	pub sort:		Option<Sort>,
	pub layout:		Layout,
	pub hanging_indent:	bool,
	pub second_field_align:	Option<SecondField>,
	pub line_spacing:	u32,
	pub entry_spacing:	u32,
	pub sub_author:		Option<String>,	// subsequent-author-substitute
	pub sub_rule:		SubRule,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageRange { Chicago, Chicago15, Chicago16, Expanded, Minimal, MinimalTwo }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Demote { Never, SortOnly, DisplayAndSort }

/// A CSL 1.0.2 independent style.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
	pub class:			Class,
	pub version:			String,
	pub info:			Info,
	pub default_locale:		Option<String>,
	pub initialize_with_hyphen:	bool,
	pub page_range_format:		Option<PageRange>,
	pub demote_particle:		Demote,
	pub names:			NameOpts,
	pub names_delimiter:		Option<String>,
	pub macros:			BTreeMap<String, Vec<Elem>>,
	pub citation:			Citation,
	pub bibliography:		Option<Bibliography>,
	pub locales:			Vec<Locale>,	// in-style overrides, in the order written
}

impl Style {

	/// Reads a style from its CSL XML.
	///
	/// A dependent style, which names a parent and has no layout of its own, is refused by name so
	/// that the caller can load the parent instead.
	pub fn parse(src: &str) -> Outcome<Self> {
		let xml = res!(Xml::parse(src));
		let root = res!(xml.root());
		if root.name.local() != "style" {
			return Err(err!("CSL: the root element is <{}>, not <style>.", root.name.qname; Invalid, Input));
		}
		let rd = Rd { xml: &xml };
		let info = match root.child("info") {
			Some(e)	=> rd.info(e),
			None	=> Info::default(),
		};
		let citation_el = match root.child("citation") {
			Some(e)	=> e,
			None	=> return Err(match &info.parent {
				Some(p)	=> err!("CSL style '{}' is a dependent style of '{}'; load its parent.", info.id, p;
					Invalid, Input, Unimplemented),
				None	=> err!("CSL style '{}' has no <citation>.", info.id; Invalid, Input, Missing),
			}),
		};
		let class = match root.attr("class") {
			Some("in-text")	=> Class::InText,
			Some("note")	=> Class::Note,
			other		=> return Err(err!(
				"CSL style '{}': class is {:?}, not 'in-text' or 'note'.", info.id, other; Invalid, Input)),
		};
		let mut macros = BTreeMap::new();
		for m in root.children("macro") {
			let name = res!(rd.req(m, "name"));
			let elems = res!(rd.elems(m, &info.id));
			macros.insert(name.to_string(), elems);
		}
		let mut locales = Vec::new();
		for l in root.children("locale") {
			locales.push(res!(Locale::read(&xml, l)));
		}
		let citation = res!(rd.citation(citation_el, &info.id));
		let bibliography = match root.child("bibliography") {
			Some(b)	=> Some(res!(rd.bibliography(b, &info.id))),
			None	=> None,
		};
		Ok(Self {
			class,
			version:		root.attr("version").unwrap_or("1.0").to_string(),
			default_locale:		root.attr("default-locale").map(str::to_string),
			initialize_with_hyphen:	res!(rd.bool(root, "initialize-with-hyphen")).unwrap_or(true),
			page_range_format:	res!(rd.page_range(root)),
			demote_particle:	match root.attr("demote-non-dropping-particle") {
				None | Some("display-and-sort")	=> Demote::DisplayAndSort,
				Some("sort-only")		=> Demote::SortOnly,
				Some("never")			=> Demote::Never,
				Some(v)				=> return Err(rd.bad(root, "demote-non-dropping-particle", v)),
			},
			names:			res!(rd.name_opts(root)),
			names_delimiter:	root.attr("names-delimiter").map(str::to_string),
			info,
			macros,
			citation,
			bibliography,
			locales,
		})
	}

	pub fn get_macro(&self, name: &str) -> Option<&[Elem]> {
		self.macros.get(name).map(|v| v.as_slice())
	}
}

/// Walks every element of a list and its descendants, in document order.
pub fn walk<'a, F: FnMut(&'a Elem)>(elems: &'a [Elem], f: &mut F) {
	for e in elems {
		f(e);
		match e {
			Elem::Names(n)	=> if let Some(s) = &n.substitute { walk(s, f) },
			Elem::Group(g)	=> walk(&g.elems, f),
			Elem::Choose(c)	=> for b in &c.branches { walk(&b.elems, f) },
			_		=> {}
		}
	}
}

/// The reader: attribute parsing over the XML tree, with errors naming the style.
pub(crate) struct Rd<'a> {
	pub xml: &'a Xml,
}

impl<'a> Rd<'a> {

	pub fn bad(&self, e: &XElem, attr: &str, val: &str) -> Error<ErrTag> {
		err!("CSL: <{}> at byte {} has {}=\"{}\", which is not a value CSL 1.0.2 allows.",
			e.name.qname, e.span.start, attr, val; Invalid, Input)
	}

	pub fn req<'b>(&self, e: &'b XElem, attr: &str) -> Outcome<&'b str> {
		match e.attr(attr) {
			Some(v)	=> Ok(v),
			None	=> Err(err!("CSL: <{}> at byte {} lacks its required '{}' attribute.",
				e.name.qname, e.span.start, attr; Invalid, Input, Missing)),
		}
	}

	pub fn bool(&self, e: &XElem, attr: &str) -> Outcome<Option<bool>> {
		match e.attr(attr) {
			None		=> Ok(None),
			Some("true")	=> Ok(Some(true)),
			Some("false")	=> Ok(Some(false)),
			Some(v)		=> Err(self.bad(e, attr, v)),
		}
	}

	pub fn uint(&self, e: &XElem, attr: &str) -> Outcome<Option<u32>> {
		match e.attr(attr) {
			None	=> Ok(None),
			Some(v)	=> match v.trim().parse::<u32>() {
				Ok(n)	=> Ok(Some(n)),
				Err(_)	=> Err(self.bad(e, attr, v)),
			},
		}
	}

	fn page_range(&self, e: &XElem) -> Outcome<Option<PageRange>> {
		Ok(match e.attr("page-range-format") {
			None			=> None,
			Some("chicago")		=> Some(PageRange::Chicago),
			Some("chicago-15")	=> Some(PageRange::Chicago15),
			Some("chicago-16")	=> Some(PageRange::Chicago16),
			Some("expanded")	=> Some(PageRange::Expanded),
			Some("minimal")		=> Some(PageRange::Minimal),
			Some("minimal-two")	=> Some(PageRange::MinimalTwo),
			Some(v)			=> return Err(self.bad(e, "page-range-format", v)),
		})
	}

	fn info(&self, e: &XElem) -> Info {
		let txt = |name: &str| e.child(name).map(|c| self.xml.text_of(c).trim().to_string());
		let parent = e.children("link").into_iter()
			.find(|l| l.attr("rel") == Some("independent-parent"))
			.and_then(|l| l.attr("href").map(str::to_string));
		Info {
			id:		txt("id").unwrap_or_default(),
			title:		txt("title").unwrap_or_default(),
			updated:	txt("updated").unwrap_or_default(),
			rights:		txt("rights"),
			parent,
		}
	}

	pub fn look(&self, e: &XElem) -> Outcome<Look> {
		let fmt = Fmt {
			style:		match e.attr("font-style") {
				None		=> None,
				Some("normal")	=> Some(FontStyle::Normal),
				Some("italic")	=> Some(FontStyle::Italic),
				Some("oblique")	=> Some(FontStyle::Oblique),
				Some(v)		=> return Err(self.bad(e, "font-style", v)),
			},
			variant:	match e.attr("font-variant") {
				None			=> None,
				Some("normal")		=> Some(FontVariant::Normal),
				Some("small-caps")	=> Some(FontVariant::SmallCaps),
				Some(v)			=> return Err(self.bad(e, "font-variant", v)),
			},
			weight:		match e.attr("font-weight") {
				None		=> None,
				Some("normal")	=> Some(FontWeight::Normal),
				Some("bold")	=> Some(FontWeight::Bold),
				Some("light")	=> Some(FontWeight::Light),
				Some(v)		=> return Err(self.bad(e, "font-weight", v)),
			},
			deco:		match e.attr("text-decoration") {
				None			=> None,
				Some("none")		=> Some(Decoration::None),
				Some("underline")	=> Some(Decoration::Underline),
				Some(v)			=> return Err(self.bad(e, "text-decoration", v)),
			},
			valign:		match e.attr("vertical-align") {
				None			=> None,
				Some("baseline")	=> Some(VAlign::Baseline),
				Some("sup")		=> Some(VAlign::Sup),
				Some("sub")		=> Some(VAlign::Sub),
				Some(v)			=> return Err(self.bad(e, "vertical-align", v)),
			},
		};
		Ok(Look {
			fmt,
			prefix:		e.attr("prefix").map(str::to_string),
			suffix:		e.attr("suffix").map(str::to_string),
			display:	match e.attr("display") {
				None			=> None,
				Some("block")		=> Some(Display::Block),
				Some("left-margin")	=> Some(Display::LeftMargin),
				Some("right-inline")	=> Some(Display::RightInline),
				Some("indent")		=> Some(Display::Indent),
				Some(v)			=> return Err(self.bad(e, "display", v)),
			},
			quotes:		res!(self.bool(e, "quotes")).unwrap_or(false),
			strip_periods:	res!(self.bool(e, "strip-periods")).unwrap_or(false),
			text_case:	match e.attr("text-case") {
				None			=> None,
				Some("lowercase")	=> Some(TextCase::Lowercase),
				Some("uppercase")	=> Some(TextCase::Uppercase),
				Some("capitalize-first")=> Some(TextCase::CapitaliseFirst),
				Some("capitalize-all")	=> Some(TextCase::CapitaliseAll),
				Some("sentence")	=> Some(TextCase::Sentence),
				Some("title")		=> Some(TextCase::Title),
				Some(v)			=> return Err(self.bad(e, "text-case", v)),
			},
		})
	}

	pub fn term_form(&self, e: &XElem) -> Outcome<TermForm> {
		Ok(match e.attr("form") {
			None | Some("long")	=> TermForm::Long,
			Some("short")		=> TermForm::Short,
			Some("verb")		=> TermForm::Verb,
			Some("verb-short")	=> TermForm::VerbShort,
			Some("symbol")		=> TermForm::Symbol,
			Some(v)			=> return Err(self.bad(e, "form", v)),
		})
	}

	pub fn name_opts(&self, e: &XElem) -> Outcome<NameOpts> {
		let rule = |attr: &str| -> Outcome<Option<DelimRule>> {
			Ok(match e.attr(attr) {
				None				=> None,
				Some("contextual")		=> Some(DelimRule::Contextual),
				Some("after-inverted-name")	=> Some(DelimRule::AfterInvertedName),
				Some("always")			=> Some(DelimRule::Always),
				Some("never")			=> Some(DelimRule::Never),
				Some(v)				=> return Err(self.bad(e, attr, v)),
			})
		};
		Ok(NameOpts {
			and:				match e.attr("and") {
				None		=> None,
				Some("text")	=> Some(And::Text),
				Some("symbol")	=> Some(And::Symbol),
				Some(v)		=> return Err(self.bad(e, "and", v)),
			},
			// On `style`, `citation` and `bibliography` the attribute is `name-delimiter`.
			delimiter:			match e.name.local() {
				"name"	=> e.attr("delimiter").map(str::to_string),
				_	=> e.attr("name-delimiter").map(str::to_string),
			},
			delim_precedes_et_al:		res!(rule("delimiter-precedes-et-al")),
			delim_precedes_last:		res!(rule("delimiter-precedes-last")),
			et_al_min:			res!(self.uint(e, "et-al-min")),
			et_al_use_first:		res!(self.uint(e, "et-al-use-first")),
			et_al_subsequent_min:		res!(self.uint(e, "et-al-subsequent-min")),
			et_al_subsequent_use_first:	res!(self.uint(e, "et-al-subsequent-use-first")),
			et_al_use_last:			res!(self.bool(e, "et-al-use-last")),
			form:				match e.attr(match e.name.local() {
				"name"	=> "form",
				_	=> "name-form",
			}) {
				None		=> None,
				Some("long")	=> Some(NameForm::Long),
				Some("short")	=> Some(NameForm::Short),
				Some("count")	=> Some(NameForm::Count),
				Some(v)		=> return Err(self.bad(e, "form", v)),
			},
			initialize:			res!(self.bool(e, "initialize")),
			initialize_with:		e.attr("initialize-with").map(str::to_string),
			name_as_sort_order:		match e.attr("name-as-sort-order") {
				None		=> None,
				Some("first")	=> Some(SortOrder::First),
				Some("all")	=> Some(SortOrder::All),
				Some(v)		=> return Err(self.bad(e, "name-as-sort-order", v)),
			},
			sort_separator:			e.attr("sort-separator").map(str::to_string),
		})
	}

	fn sort(&self, e: &XElem) -> Outcome<Sort> {
		let mut keys = Vec::new();
		for k in e.children("key") {
			let src = match (k.attr("variable"), k.attr("macro")) {
				(Some(v), _)	=> SortSrc::Var(v.to_string()),
				(None, Some(m))	=> SortSrc::Macro(m.to_string()),
				(None, None)	=> return Err(err!(
					"CSL: <key> at byte {} names neither a variable nor a macro.", k.span.start;
					Invalid, Input, Missing)),
			};
			keys.push(SortKey {
				src,
				ascending:	match k.attr("sort") {
					None | Some("ascending")	=> true,
					Some("descending")		=> false,
					Some(v)				=> return Err(self.bad(k, "sort", v)),
				},
				names_min:		res!(self.uint(k, "names-min")),
				names_use_first:	res!(self.uint(k, "names-use-first")),
				names_use_last:		res!(self.bool(k, "names-use-last")),
			});
		}
		Ok(Sort { keys })
	}

	fn layout(&self, e: &XElem, id: &str) -> Outcome<Layout> {
		Ok(Layout {
			delimiter:	e.attr("delimiter").map(str::to_string),
			elems:		res!(self.elems(e, id)),
			look:		res!(self.look(e)),
		})
	}

	fn citation(&self, e: &XElem, id: &str) -> Outcome<Citation> {
		let layout = match e.child("layout") {
			Some(l)	=> res!(self.layout(l, id)),
			None	=> return Err(err!("CSL style '{}': <citation> has no <layout>.", id; Invalid, Input, Missing)),
		};
		Ok(Citation {
			names:			res!(self.name_opts(e)),
			names_delimiter:	e.attr("names-delimiter").map(str::to_string),
			sort:			match e.child("sort") { Some(s) => Some(res!(self.sort(s))), None => None },
			layout,
			add_names:		res!(self.bool(e, "disambiguate-add-names")).unwrap_or(false),
			add_givenname:		res!(self.bool(e, "disambiguate-add-givenname")).unwrap_or(false),
			givenname_rule:		match e.attr("givenname-disambiguation-rule") {
				None | Some("by-cite")			=> GivenRule::ByCite,
				Some("all-names")			=> GivenRule::AllNames,
				Some("all-names-with-initials")		=> GivenRule::AllNamesWithInitials,
				Some("primary-name")			=> GivenRule::PrimaryName,
				Some("primary-name-with-initials")	=> GivenRule::PrimaryNameWithInitials,
				Some(v)	=> return Err(self.bad(e, "givenname-disambiguation-rule", v)),
			},
			add_year_suffix:	res!(self.bool(e, "disambiguate-add-year-suffix")).unwrap_or(false),
			cite_group_delimiter:	e.attr("cite-group-delimiter").map(str::to_string),
			collapse:		match e.attr("collapse") {
				None				=> None,
				Some("citation-number")		=> Some(Collapse::CitationNumber),
				Some("year")			=> Some(Collapse::Year),
				Some("year-suffix")		=> Some(Collapse::YearSuffix),
				Some("year-suffix-ranged")	=> Some(Collapse::YearSuffixRanged),
				Some(v)				=> return Err(self.bad(e, "collapse", v)),
			},
			year_suffix_delimiter:		e.attr("year-suffix-delimiter").map(str::to_string),
			after_collapse_delimiter:	e.attr("after-collapse-delimiter").map(str::to_string),
			near_note_distance:		res!(self.uint(e, "near-note-distance")).unwrap_or(5),
		})
	}

	fn bibliography(&self, e: &XElem, id: &str) -> Outcome<Bibliography> {
		let layout = match e.child("layout") {
			Some(l)	=> res!(self.layout(l, id)),
			None	=> return Err(err!("CSL style '{}': <bibliography> has no <layout>.", id; Invalid, Input, Missing)),
		};
		Ok(Bibliography {
			names:			res!(self.name_opts(e)),
			names_delimiter:	e.attr("names-delimiter").map(str::to_string),
			sort:			match e.child("sort") { Some(s) => Some(res!(self.sort(s))), None => None },
			layout,
			hanging_indent:		res!(self.bool(e, "hanging-indent")).unwrap_or(false),
			second_field_align:	match e.attr("second-field-align") {
				None		=> None,
				Some("flush")	=> Some(SecondField::Flush),
				Some("margin")	=> Some(SecondField::Margin),
				Some(v)		=> return Err(self.bad(e, "second-field-align", v)),
			},
			line_spacing:		res!(self.uint(e, "line-spacing")).unwrap_or(1),
			entry_spacing:		res!(self.uint(e, "entry-spacing")).unwrap_or(1),
			sub_author:		e.attr("subsequent-author-substitute").map(str::to_string),
			sub_rule:		match e.attr("subsequent-author-substitute-rule") {
				None | Some("complete-all")	=> SubRule::CompleteAll,
				Some("complete-each")		=> SubRule::CompleteEach,
				Some("partial-each")		=> SubRule::PartialEach,
				Some("partial-first")		=> SubRule::PartialFirst,
				Some(v)	=> return Err(self.bad(e, "subsequent-author-substitute-rule", v)),
			},
		})
	}

	pub fn elems(&self, parent: &XElem, id: &str) -> Outcome<Vec<Elem>> {
		let mut out = Vec::new();
		for e in parent.elems() {
			out.push(res!(self.elem(e, id)));
		}
		Ok(out)
	}

	fn elem(&self, e: &XElem, id: &str) -> Outcome<Elem> {
		Ok(match e.name.local() {
			"text"		=> Elem::Text(res!(self.text(e))),
			"number"	=> Elem::Number(Number {
				var:	res!(self.req(e, "variable")).to_string(),
				form:	match e.attr("form") {
					None | Some("numeric")	=> NumForm::Numeric,
					Some("ordinal")		=> NumForm::Ordinal,
					Some("long-ordinal")	=> NumForm::LongOrdinal,
					Some("roman")		=> NumForm::Roman,
					Some(v)			=> return Err(self.bad(e, "form", v)),
				},
				look:	res!(self.look(e)),
			}),
			"label"		=> Elem::Label(res!(self.label(e))),
			"names"		=> Elem::Names(res!(self.names(e, id))),
			"date"		=> Elem::Date(res!(self.date(e, false))),
			"group"		=> Elem::Group(Group {
				delimiter:	e.attr("delimiter").map(str::to_string),
				elems:		res!(self.elems(e, id)),
				look:		res!(self.look(e)),
			}),
			"choose"	=> Elem::Choose(res!(self.choose(e, id))),
			other		=> return Err(err!(
				"CSL style '{}': <{}> at byte {} is not a rendering element.", id, other, e.span.start;
				Invalid, Input, Unknown)),
		})
	}

	fn text(&self, e: &XElem) -> Outcome<Text> {
		let src = if let Some(v) = e.attr("variable") {
			TextSrc::Var {
				name:	v.to_string(),
				form:	match e.attr("form") {
					None | Some("long")	=> VarForm::Long,
					Some("short")		=> VarForm::Short,
					Some(f)			=> return Err(self.bad(e, "form", f)),
				},
			}
		} else if let Some(m) = e.attr("macro") {
			TextSrc::Macro(m.to_string())
		} else if let Some(t) = e.attr("term") {
			TextSrc::Term {
				name:	t.to_string(),
				form:	res!(self.term_form(e)),
				plural:	res!(self.bool(e, "plural")).unwrap_or(false),
			}
		} else if let Some(v) = e.attr("value") {
			TextSrc::Value(v.to_string())
		} else {
			return Err(err!("CSL: <text> at byte {} names no variable, macro, term or value.",
				e.span.start; Invalid, Input, Missing));
		};
		Ok(Text { src, look: res!(self.look(e)) })
	}

	fn label(&self, e: &XElem) -> Outcome<Label> {
		Ok(Label {
			var:	e.attr("variable").unwrap_or("").to_string(),
			form:	res!(self.term_form(e)),
			plural:	match e.attr("plural") {
				None | Some("contextual")	=> Plural::Contextual,
				Some("always")			=> Plural::Always,
				Some("never")			=> Plural::Never,
				Some(v)				=> return Err(self.bad(e, "plural", v)),
			},
			look:	res!(self.look(e)),
		})
	}

	fn names(&self, e: &XElem, id: &str) -> Outcome<Names> {
		let mut out = Names {
			vars:		res!(self.req(e, "variable")).split_whitespace().map(str::to_string).collect(),
			delimiter:	e.attr("delimiter").map(str::to_string),
			name:		None,
			et_al:		None,
			label:		None,
			label_first:	false,
			substitute:	None,
			look:		res!(self.look(e)),
		};
		for k in e.elems() {
			match k.name.local() {
				"name"		=> {
					let mut parts = Vec::new();
					for p in k.children("name-part") {
						parts.push(NamePart {
							part:	match res!(self.req(p, "name")) {
								"given"		=> PartName::Given,
								"family"	=> PartName::Family,
								v		=> return Err(self.bad(p, "name", v)),
							},
							look:	res!(self.look(p)),
						});
					}
					out.name = Some(Name { opts: res!(self.name_opts(k)), parts, look: res!(self.look(k)) });
				}
				"et-al"		=> out.et_al = Some(EtAl {
					term:	k.attr("term").unwrap_or("et-al").to_string(),
					look:	res!(self.look(k)),
				}),
				"label"		=> {
					out.label_first = out.name.is_none();
					out.label = Some(res!(self.label(k)));
				}
				"substitute"	=> out.substitute = Some(res!(self.elems(k, id))),
				other		=> return Err(err!(
					"CSL style '{}': <{}> at byte {} cannot sit inside <names>.", id, other, k.span.start;
					Invalid, Input, Unknown)),
			}
		}
		Ok(out)
	}

	/// Reads a `date`; in a locale `var` is absent and `form` names the format being defined.
	pub fn date(&self, e: &XElem, in_locale: bool) -> Outcome<Date> {
		let mut parts = Vec::new();
		for p in e.children("date-part") {
			parts.push(DatePart {
				name:			match res!(self.req(p, "name")) {
					"day"	=> DatePartName::Day,
					"month"	=> DatePartName::Month,
					"year"	=> DatePartName::Year,
					v	=> return Err(self.bad(p, "name", v)),
				},
				form:			match p.attr("form") {
					None				=> None,
					Some("numeric")			=> Some(DatePartForm::Numeric),
					Some("numeric-leading-zeros")	=> Some(DatePartForm::NumericLeadingZeros),
					Some("ordinal")			=> Some(DatePartForm::Ordinal),
					Some("long")			=> Some(DatePartForm::Long),
					Some("short")			=> Some(DatePartForm::Short),
					Some(v)				=> return Err(self.bad(p, "form", v)),
				},
				range_delimiter:	p.attr("range-delimiter").map(str::to_string),
				look:			res!(self.look(p)),
			});
		}
		Ok(Date {
			var:		match in_locale {
				true	=> String::new(),
				false	=> res!(self.req(e, "variable")).to_string(),
			},
			form:		match e.attr("form") {
				None		=> None,
				Some("text")	=> Some(DateForm::Text),
				Some("numeric")	=> Some(DateForm::Numeric),
				Some(v)		=> return Err(self.bad(e, "form", v)),
			},
			parts_sel:	match e.attr("date-parts") {
				None				=> None,
				Some("year-month-day")		=> Some(DateParts::YearMonthDay),
				Some("year-month")		=> Some(DateParts::YearMonth),
				Some("year")			=> Some(DateParts::Year),
				Some(v)				=> return Err(self.bad(e, "date-parts", v)),
			},
			parts,
			delimiter:	e.attr("delimiter").map(str::to_string),
			look:		res!(self.look(e)),
		})
	}

	fn choose(&self, e: &XElem, id: &str) -> Outcome<Choose> {
		let mut branches = Vec::new();
		for b in e.elems() {
			let cond = match b.name.local() {
				"if" | "else-if"	=> Some(res!(self.cond(b))),
				"else"			=> None,
				other			=> return Err(err!(
					"CSL style '{}': <{}> at byte {} cannot sit inside <choose>.", id, other, b.span.start;
					Invalid, Input, Unknown)),
			};
			branches.push(Branch { cond, elems: res!(self.elems(b, id)) });
		}
		Ok(Choose { branches })
	}

	fn cond(&self, e: &XElem) -> Outcome<Cond> {
		let mut tests = Vec::new();
		for a in &e.attrs {
			let words = a.value.split_whitespace();
			match a.name.qname.as_str() {
				"disambiguate"		=> tests.push(Test::Disambiguate(match res!(self.bool(e, "disambiguate")) {
					Some(b)	=> b,
					None	=> false,
				})),
				"is-numeric"		=> for w in words { tests.push(Test::IsNumeric(w.to_string())) },
				"is-uncertain-date"	=> for w in words { tests.push(Test::IsUncertainDate(w.to_string())) },
				"locator"		=> for w in words { tests.push(Test::Locator(w.to_string())) },
				"type"			=> for w in words { tests.push(Test::Type(w.to_string())) },
				"variable"		=> for w in words { tests.push(Test::Variable(w.to_string())) },
				"position"		=> for w in words {
					tests.push(Test::Position(match w {
						"first"			=> Position::First,
						"subsequent"		=> Position::Subsequent,
						"ibid-with-locator"	=> Position::IbidWithLocator,
						"ibid"			=> Position::Ibid,
						"near-note"		=> Position::NearNote,
						v			=> return Err(self.bad(e, "position", v)),
					}));
				},
				_			=> {}
			}
		}
		Ok(Cond {
			mode:	match e.attr("match") {
				None | Some("all")	=> Match::All,
				Some("any")		=> Match::Any,
				Some("none")		=> Match::None,
				Some(v)			=> return Err(self.bad(e, "match", v)),
			},
			tests,
		})
	}
}
