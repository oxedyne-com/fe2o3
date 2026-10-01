// U5 owns this file: bibliography and the realisation of citations, over `crate::bib`.
//
// Sources are read when the element is constructed and kept on it (Typst keeps the parsed library the
// same way), so a citation anywhere in the document can be formatted from the bibliography the previous
// pass found. BibTeX and BibLaTeX sources are read; Hayagriva YAML is refused with a diagnostic. Two
// styles are set, IEEE (Typst's default) and Chicago author-date; another style name is an error, since
// no CSL processor exists yet. Only the document's first bibliography is consulted: `target` and `group`,
// which split citations across several bibliographies, are accepted but not applied. Adjacent citations
// are grouped by realisation (`cite-group`, which awaits its element kind), and [`show_cite_group`]
// renders such a group.

use crate::bib::{
	Bibliography,
	CiteForm,
	CiteStyle,
	ListEntry,
	RefRun,
	RefStyle,
};
use crate::diag::DiagnosticKind;
use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	ElemKind,
	FieldDefault,
	FieldSpec,
	FieldType,
};
use crate::eval::lib::model::common::{
	self,
	expect,
	to_content,
	CastErr,
	K,
};
use crate::eval::lib::model::link;
use crate::eval::styles::{
	StyleChain,
	Styles,
};
use crate::eval::value::{
	Label,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;
use crate::vfs;

use oxedyne_fe2o3_core::prelude::*;

const ANY: FieldType = FieldType::Any;

static BIBLIOGRAPHY: [FieldSpec; 8] = [
	FieldSpec::required("sources",		ANY),
	FieldSpec::named("title",			ANY,	FieldDefault::Auto),
	FieldSpec::named("full",			FieldType::Of(crate::eval::value::Type::Bool),	FieldDefault::Bool(false)),
	FieldSpec::named("style",			ANY,	FieldDefault::Str("ieee")),
	FieldSpec::named("target",			ANY,	FieldDefault::Auto),
	FieldSpec::named("group",			ANY,	FieldDefault::Auto),
	FieldSpec::named("keys",			ANY,	FieldDefault::EmptyArray).synthesised().internal(),
	FieldSpec::named("data",			ANY,	FieldDefault::EmptyArray).synthesised().internal(),
];

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::Bibliography	=> &BIBLIOGRAPHY,
		ElemKind::CiteGroup		=> &CITE_GROUP,
		_						=> &[],
	}
}

pub fn cast(_kind: ElemKind, name: &str, v: Value) -> Result<Value, CastErr> {
	match name {
		"sources"	=> match &v {
			Value::Array(a) => {
				for x in a.iter() {
					ok!(expect(x, &[K::Path, K::Str, K::Bytes]));
				}
				Ok(v)
			}
			_ => expect(&v, &[K::Path, K::Str, K::Bytes, K::Array]).map(|_| v),
		},
		"title"		=> match v {
			Value::Auto | Value::None	=> Ok(v),
			other	=> expect(&other, &[K::Content, K::None, K::Auto]).map(|_| to_content(other)),
		},
		"full"		=> expect(&v, &[K::Bool]).map(|_| v),
		"style"		=> match &v {
			Value::Str(s)	=> style_name(s).map(|_| style_value(s)),
			other			=> Err(CastErr::Type(common::mismatch("string", other))),
		},
		"target"	=> expect(&v, &[K::Label, K::Func, K::Location, K::Selector, K::Auto]).map(|_| v),
		"group"		=> expect(&v, &[K::Str, K::None, K::Auto]).map(|_| v),
		_			=> Ok(v),
	}
}

/// `cite-group`'s one field: the citations realisation gathered.
pub fn cast_group(name: &str, v: Value) -> Result<Value, CastErr> {
	match name {
		"children"	=> expect(&v, &[K::Array]).map(|_| v),
		_			=> Ok(v),
	}
}

static CITE_GROUP: [FieldSpec; 1] = [
	FieldSpec::required("children",	ANY),
];

/// Checks a style name: one Austenite sets; one of Typst's other built-in styles, refused as not yet
/// supported; anything else unknown, as Typst says. A path to a `.csl` file is refused too.
pub fn style_name(s: &str) -> Result<CiteStyle, CastErr> {
	if let Some(st) = CiteStyle::by_name(s) {
		return Ok(st);
	}
	if s.ends_with(".csl") || TYPST_STYLES.contains(&s) {
		return Err(CastErr::Value(fmt!("the bibliography style `{}` is not supported yet \
			(the styles set are ieee and chicago-author-date)", s)));
	}
	Err(CastErr::Value(fmt!("unknown style: {}", s)))
}

/// A style name as Typst stores it: IEEE's long name is `ieee`.
pub fn style_value(s: &str) -> Value {
	match CiteStyle::by_name(s) {
		Some(CiteStyle::Ieee)	=> Value::str("ieee"),
		_						=> Value::str(s),
	}
}

// Typst 0.15.1's built-in style names, for telling a style not yet set from a misspelt one.
const TYPST_STYLES: &[&str] = &[
	"alphanumeric", "american-anthropological-association", "american-chemical-society",
	"american-geophysical-union", "american-institute-of-aeronautics-and-astronautics",
	"american-institute-of-physics", "american-medical-association", "american-meteorological-society",
	"american-physics-society", "american-physiological-society", "american-political-science-association",
	"american-psychological-association", "apa", "american-society-for-microbiology",
	"american-society-of-civil-engineers", "american-society-of-mechanical-engineers",
	"american-sociological-association", "angewandte-chemie", "annual-reviews", "annual-reviews-author-date",
	"associacao-brasileira-de-normas-tecnicas", "association-for-computing-machinery", "biomed-central",
	"bristol-university-press", "british-medical-journal", "bmj", "chicago-notes", "chicago-fullnotes",
	"chicago-shortened-notes", "copernicus", "cse-citation-sequence-brackets-8th-edition",
	"council-of-science-editors", "cse-name-year", "council-of-science-editors-author-date",
	"current-opinion", "deutsche-gesellschaft-f\u{fc}r-psychologie", "deutsche-sprache", "elsevier-harvard",
	"elsevier-vancouver", "elsevier-with-titles", "frontiers", "future-medicine", "future-science",
	"gb-7714-2005-numeric", "gb-7714-2015-author-date", "gb-7714-2015-note", "gb-7714-2015-numeric",
	"gost-r-705-2008-numeric", "harvard-cite-them-right", "institute-of-physics-numeric",
	"iso-690-author-date", "iso-690-numeric", "karger", "mary-ann-liebert-vancouver",
	"modern-humanities-research-association-notes", "modern-humanities-research-association",
	"modern-language-association", "mla", "modern-language-association-8", "mla-8",
	"multidisciplinary-digital-publishing-institute", "nature", "nlm-citation-sequence", "vancouver",
	"nlm-citation-sequence-superscript", "vancouver-superscript", "pensoft", "public-library-of-science",
	"royal-society-of-chemistry", "sage-vancouver", "sist02", "springer-basic", "springer-basic-author-date",
	"springer-fachzeitschriften-medizin-psychologie", "springer-humanities-author-date",
	"springer-lecture-notes-in-computer-science", "springer-mathphys", "springer-socpsych-author-date",
	"springer-vancouver", "taylor-and-francis-chicago-author-date",
	"taylor-and-francis-national-library-of-medicine", "the-institution-of-engineering-and-technology",
	"the-lancet", "thieme", "trends", "turabian-author-date", "turabian-fullnote-8",
];

/// `bibliography(sources, ..)`: the schema walk, then the sources read and their keys checked.
pub fn construct(engine: &mut Engine, args: &mut Args) -> Outcome<Content> {
	let span = args.span;
	let mut elem = res!(super::construct_schema(engine, ElemKind::Bibliography, args));
	let sources = match elem.field("sources") {
		Some(Value::Array(a))	=> (**a).clone(),
		Some(v)					=> vec![v.clone()],
		None					=> Vec::new(),
	};
	let mut texts = Vec::new();
	for s in sources {
		let (text, name) = match s {
			Value::Str(p) => {
				let path = res!(crate::eval::import::resolve_path(engine, &p, span.file, span));
				let text = match vfs::read_to_string(&path) {
					Ok(t)	=> t,
					Err(e)	=> return Err(engine.error(DiagnosticKind::MissingFile, span, fmt!("failed to load file ({})", e))),
				};
				(text, p.to_string())
			}
			Value::Bytes(b) => match String::from_utf8((*b).clone()) {
				Ok(t)	=> (t, String::new()),
				Err(_)	=> return Err(engine.error(DiagnosticKind::Encoding, span, "file is not valid utf-8")),
			},
			_ => continue,
		};
		let lower = name.to_ascii_lowercase();
		if lower.ends_with(".yml") || lower.ends_with(".yaml") || (!lower.ends_with(".bib") && !text.contains('@')) {
			return Err(engine.error(DiagnosticKind::Unsupported, span, "Hayagriva YAML bibliographies are not supported yet; use a BibTeX (.bib) file"));
		}
		if !name.is_empty() && !lower.ends_with(".bib") {
			return Err(engine.error(DiagnosticKind::Type, span, "unknown bibliography format (must be .yaml/.yml or .bib)"));
		}
		texts.push(text);
	}
	let mut seen: Vec<String> = Vec::new();
	let mut dups: Vec<String> = Vec::new();
	for t in &texts {
		let one = res!(Bibliography::parse(t));
		for k in one.keys() {
			if seen.iter().any(|s| s == k) {
				if !dups.iter().any(|d| d == k) {
					dups.push(k.to_string());
				}
			} else {
				seen.push(k.to_string());
			}
		}
	}
	if !dups.is_empty() {
		return Err(engine.error(DiagnosticKind::Type, span, fmt!("duplicate bibliography keys: {}", dups.join(", "))));
	}
	let keys: Vec<Value> = seen.into_iter().map(Value::str).collect();
	elem.set(res!(common::fid(ElemKind::Bibliography, "keys")), Value::array(keys));
	elem.set(res!(common::fid(ElemKind::Bibliography, "data")),
		Value::array(texts.into_iter().map(Value::str).collect()));
	Ok(elem)
}

/// The sources as one library.
fn parse(texts: &[String]) -> Outcome<Bibliography> {
	let joined = texts.join("\n");
	Bibliography::parse(&joined)
}

/// Does this bibliography element hold `key`?
pub fn has_key(bib: &Content, key: &Label) -> bool {
	match bib.field("keys") {
		Some(Value::Array(a)) => a.iter().any(|k| matches!(k, Value::Str(s) if s.as_str() == key.as_str())),
		_ => false,
	}
}

/// Headings in a bibliography are not numbered; the list is indented by 1em.
pub fn show_set() -> Outcome<Styles> {
	common::props(vec![
		(ElemKind::Heading,	"numbering",	Value::None),
		(ElemKind::Pad,		"left",			common::em(1.0)),
	])
}

/// What formatting a citation needs: the bibliography, its library, its style and the cited keys in
/// first-citation order.
struct Works {
	bib:	Content,
	lib:	Bibliography,
	style:	CiteStyle,
	cited:	Vec<String>,
}

fn works(engine: &mut Engine, span: Span) -> Outcome<Works> {
	let bib = match engine.intro.elems.iter().find(|e| e.is(ElemKind::Bibliography)) {
		Some(b)	=> b.clone(),
		None	=> return Err(engine.error(DiagnosticKind::Type, span, "the document does not contain a bibliography")),
	};
	let texts: Vec<String> = match bib.field("data") {
		Some(Value::Array(a)) => a.iter().filter_map(|v| match v {
			Value::Str(s)	=> Some(s.to_string()),
			_				=> None,
		}).collect(),
		_ => Vec::new(),
	};
	let lib = res!(parse(&texts));
	let style = match bib.field("style") {
		Some(Value::Str(s))	=> CiteStyle::by_name(s).unwrap_or(CiteStyle::Ieee),
		_					=> CiteStyle::Ieee,
	};
	let mut cited: Vec<String> = Vec::new();
	for e in engine.intro.elems.iter() {
		let key = match e.kind() {
			Some(ElemKind::Cite)	=> e.field("key"),
			Some(ElemKind::Ref)		=> e.field("target"),
			_						=> None,
		};
		if let Some(Value::Label(l)) = key {
			if lib.has(l.as_str()) && !cited.iter().any(|c| c == l.as_str()) {
				cited.push(l.as_str().to_string());
			}
		}
	}
	Ok(Works { bib, lib, style, cited })
}

/// Runs as content: italics as `emph`, a URL or a DOI as a link.
fn runs_to_content(runs: &[RefRun], span: Span) -> Outcome<Content> {
	let mut seq = Vec::with_capacity(runs.len());
	for r in runs {
		let text = common::text(&r.text).with_span(span);
		seq.push(match r.style {
			RefStyle::Normal	=> text,
			RefStyle::Italic	=> res!(common::raw_elem(ElemKind::Emph, span, vec![("body", Value::Content(text))])),
			RefStyle::Link		=> res!(common::raw_elem(ElemKind::Link, span, vec![
				("dest", Value::str(r.text.as_str())),
				("body", Value::Content(text)),
			])),
			RefStyle::Doi		=> res!(common::raw_elem(ElemKind::Link, span, vec![
				("dest", Value::str(fmt!("https://doi.org/{}", r.text))),
				("body", Value::Content(text)),
			])),
		});
	}
	Ok(Content::sequence(seq))
}

fn citation_parts(engine: &mut Engine, elem: &Content, styles: &StyleChain, works: &Works)
	-> Outcome<(String, Option<CiteForm>, Option<String>, CiteStyle)>
{
	let span = elem.span();
	let key = match elem.field("key") {
		Some(Value::Label(l))	=> l.as_str().to_string(),
		_						=> return Err(engine.error(DiagnosticKind::Type, span, "missing argument: key")),
	};
	if !works.lib.has(&key) {
		return Err(engine.error(DiagnosticKind::Type, span, fmt!("key `{}` does not exist in the bibliography", key)));
	}
	let form = match res!(common::get(elem, styles, "form")) {
		Value::Str(s)	=> CiteForm::by_name(&s),
		_				=> None,
	};
	let supplement = match res!(common::get(elem, styles, "supplement")) {
		Value::Content(c) if !c.is_empty()	=> Some(common::plain_text(&c)),
		_									=> None,
	};
	let style = match res!(common::get(elem, styles, "style")) {
		Value::Str(s)	=> CiteStyle::by_name(&s).unwrap_or(works.style),
		_				=> works.style,
	};
	Ok((key, form, supplement, style))
}

/// A citation: its form in the bibliography's style, linked to the bibliography. A citation of form
/// `none` shows nothing and only lists the work.
pub fn show_cite(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Content> {
	let span = elem.span();
	let works = res!(works(engine, span));
	let (key, form, supplement, style) = res!(citation_parts(engine, elem, styles, &works));
	let form = match form {
		Some(f)	=> f,
		None	=> return Ok(Content::empty()),
	};
	let runs = res!(works.lib.render_citation(style, &key, form, supplement.as_deref(), &works.cited));
	let body = res!(runs_to_content(&runs, span));
	match works.bib.location() {
		Some(loc)	=> link::to_location(body, loc),
		None		=> Ok(body),
	}
}

/// Citations cited together: IEEE's `[1], [2]`, author-date's `(A 2001; B 2002)`.
pub fn show_cite_group(engine: &mut Engine, cites: &[Content], styles: &StyleChain) -> Outcome<Content> {
	let span = cites.first().map(|c| c.span()).unwrap_or(Span::detached());
	let works = res!(works(engine, span));
	let mut items: Vec<(String, Option<String>)> = Vec::new();
	let mut style = works.style;
	for c in cites {
		let (key, _, supplement, s) = res!(citation_parts(engine, c, styles, &works));
		style = s;
		items.push((key, supplement));
	}
	let borrowed: Vec<(&str, Option<&str>)> = items.iter().map(|(k, s)| (k.as_str(), s.as_deref())).collect();
	let parts = res!(works.lib.render_group(style, &borrowed, &works.cited));
	let sep = crate::bib::group_separator(style);
	let mut seq = Vec::new();
	for (i, runs) in parts.iter().enumerate() {
		if i > 0 {
			seq.push(common::text(sep));
		}
		let body = res!(runs_to_content(runs, span));
		seq.push(match works.bib.location() {
			Some(loc)	=> res!(link::to_location(body, loc)),
			None		=> body,
		});
	}
	Ok(Content::sequence(seq))
}

const COLUMN_GUTTER:	f64 = 0.65;	// em, between a numbered entry's prefix and its body
const HANGING_INDENT:	f64 = 1.5;	// em, of an unnumbered entry's second and later lines

pub fn show(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Option<Content>> {
	if elem.is(ElemKind::CiteGroup) {
		let cites: Vec<Content> = match elem.field("children") {
			Some(Value::Array(a)) => a.iter().filter_map(|v| match v {
				Value::Content(c)	=> Some(c.clone()),
				_					=> None,
			}).collect(),
			_ => Vec::new(),
		};
		return show_cite_group(engine, &cites, styles).map(Some);
	}
	let span = elem.span();
	let mut seq = Vec::new();
	let title = match res!(common::get(elem, styles, "title")) {
		Value::Auto	=> Some(common::text(common::local(styles, "bibliography"))),
		Value::None	=> None,
		v			=> Some(common::display(v)),
	};
	if let Some(t) = title {
		seq.push(res!(common::raw_elem(ElemKind::Heading, span, vec![
			("depth",	Value::Int(1)),
			("body",	Value::Content(common::spanned(t, span))),
		])));
	}
	let works = res!(works(engine, span));
	let full = matches!(res!(common::get(elem, styles, "full")), Value::Bool(true));
	let style = match res!(common::get(elem, styles, "style")) {
		Value::Str(s)	=> CiteStyle::by_name(&s).unwrap_or(works.style),
		_				=> works.style,
	};
	let entries: Vec<ListEntry> = works.lib.render_list(style, &works.cited, full);
	if entries.iter().any(|e| e.prefix.is_some()) {
		let mut cells = Vec::with_capacity(entries.len() * 2);
		for e in &entries {
			cells.push(Value::Content(common::text(e.prefix.as_deref().unwrap_or(""))));
			cells.push(Value::Content(res!(runs_to_content(&e.body.runs, span))));
		}
		let row_gutter = res!(common::style(styles, ElemKind::Par, "spacing"));
		seq.push(res!(common::build(engine, ElemKind::Grid, span, cells, vec![
			("columns",			Value::array(vec![Value::Auto, Value::Auto])),
			("column-gutter",	Value::array(vec![common::em(COLUMN_GUTTER)])),
			("row-gutter",		Value::array(vec![row_gutter])),
		])));
	} else {
		for e in &entries {
			let body = res!(runs_to_content(&e.body.runs, span));
			let block = if style.hangs() {
				let pull = res!(common::h(engine, common::em(-HANGING_INDENT), false));
				let inset = common::start_side(styles, common::em(HANGING_INDENT));
				res!(common::block(engine, common::seq(vec![pull, body]), span, vec![("inset", inset)]))
			} else {
				res!(common::block(engine, body, span, Vec::new()))
			};
			seq.push(block);
		}
	}
	Ok(Some(common::seq(seq)))
}
