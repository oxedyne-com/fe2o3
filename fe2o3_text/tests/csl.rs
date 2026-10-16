use oxedyne_fe2o3_text::{
	csl::{
		Locale,
		Locales,
		Style,
		style::{
			Elem,
			TermForm,
			walk,
		},
	},
	xml::{
		Elem as XElem,
		Xml,
	},
};

use oxedyne_fe2o3_core::prelude::*;

use std::path::Path;


const RENDERING: &[&str] = &["text", "number", "label", "names", "date", "group", "choose"];

/// Counts the rendering elements under an XML element, by name, descending through everything
/// except an in-style `locale` and `info`.
fn xml_count(e: &XElem, name: &str) -> usize {
	let mut n = 0;
	for k in e.elems() {
		let l = k.name.local();
		if l == "locale" || l == "info" {
			continue;
		}
		if l == name {
			n += 1;
		}
		n += xml_count(k, name);
	}
	n
}

fn model_count(style: &Style, name: &str) -> usize {
	let mut lists: Vec<&[Elem]> = style.macros.values().map(|v| v.as_slice()).collect();
	lists.push(&style.citation.layout.elems);
	if let Some(b) = &style.bibliography {
		lists.push(&b.layout.elems);
	}
	let mut n = 0;
	for l in lists {
		walk(l, &mut |e| {
			let hit = match e {
				Elem::Text(_)	=> "text",
				Elem::Number(_)	=> "number",
				Elem::Label(_)	=> "label",
				Elem::Names(_)	=> "names",
				Elem::Date(_)	=> "date",
				Elem::Group(_)	=> "group",
				Elem::Choose(_)	=> "choose",
			};
			if hit == name {
				n += 1;
			}
			// A label inside names is held by the names, not as an element of a list.
			if name == "label" {
				if let Elem::Names(ns) = e {
					if ns.label.is_some() {
						n += 1;
					}
				}
			}
		});
	}
	n
}

/// Parses a style and checks that the model holds every macro and every rendering element the XML
/// does.
fn round_trip(path: &Path) -> Outcome<Option<Style>> {
	let src = res!(std::fs::read_to_string(path));
	if src.contains("rel=\"independent-parent\"") {
		// A dependent style is refused by name; the caller loads its parent.
		return match Style::parse(&src) {
			Err(e) if format!("{}", e).contains("dependent style") => Ok(None),
			_ => Err(err!("{}: a dependent style was not refused as one.", path.display(); Test, Mismatch)),
		};
	}
	let style = match Style::parse(&src) {
		Ok(s)	=> s,
		Err(e)	=> return Err(err!("{}: {}", path.display(), e; Test, Invalid)),
	};
	let xml = res!(Xml::parse(&src));
	let root = res!(xml.root());
	let macros = root.children("macro").len();
	if style.macros.len() != macros {
		return Err(err!("{}: {} macros in the XML, {} in the model.", path.display(), macros, style.macros.len();
			Test, Mismatch));
	}
	for name in RENDERING {
		let (x, m) = (xml_count(root, name), model_count(&style, name));
		if x != m {
			return Err(err!("{}: {} <{}> in the XML, {} in the model.", path.display(), x, name, m;
				Test, Mismatch));
		}
	}
	Ok(Some(style))
}

fn csl_files(dir: &str) -> Outcome<Vec<std::path::PathBuf>> {
	let mut out = Vec::new();
	for e in res!(std::fs::read_dir(dir)) {
		let p = res!(e).path();
		if p.extension().map(|x| x == "csl").unwrap_or(false) {
			out.push(p);
		}
	}
	out.sort();
	Ok(out)
}

#[test]
fn csl_corpus_styles_parse_and_round_trip() -> Outcome<()> {
	let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/csl/corpus");
	let files = res!(csl_files(dir));
	if files.len() < 60 {
		return Err(err!("Only {} corpus styles found in {}.", files.len(), dir; Test, Missing));
	}
	let mut bad = Vec::new();
	for f in &files {
		if let Err(e) = round_trip(f) {
			bad.push(format!("{}", e));
		}
	}
	match bad.is_empty() {
		true	=> Ok(()),
		false	=> Err(err!("{} of {} styles fail:\n{}", bad.len(), files.len(), bad.join("\n"); Test, Mismatch)),
	}
}

const EN_US: &str = r#"<?xml version="1.0"?>
<locale xmlns="http://purl.org/net/xbiblio/csl" version="1.0" xml:lang="en-US">
  <style-options punctuation-in-quote="true"/>
  <date form="numeric"><date-part name="month" form="numeric" suffix="/"/><date-part name="year"/></date>
  <terms>
    <term name="and">and</term>
    <term name="editor"><single>editor</single><multiple>editors</multiple></term>
    <term name="editor" form="short"><single>ed.</single><multiple>eds.</multiple></term>
    <term name="page" form="short"><single>p.</single><multiple>pp.</multiple></term>
    <term name="accessed">accessed</term>
  </terms>
</locale>"#;

const DE_DE: &str = r#"<?xml version="1.0"?>
<locale xmlns="http://purl.org/net/xbiblio/csl" version="1.0" xml:lang="de-DE">
  <style-options punctuation-in-quote="false"/>
  <terms>
    <term name="and">und</term>
    <term name="page" form="short"><single>S.</single><multiple>S.</multiple></term>
  </terms>
</locale>"#;

const STYLE: &str = r#"<?xml version="1.0"?>
<style xmlns="http://purl.org/net/xbiblio/csl" class="in-text" version="1.0">
  <info><id>urn:test</id><title>Test</title><updated>2026-10-12T00:00:00+00:00</updated></info>
  <locale xml:lang="de"><terms><term name="accessed">abgerufen</term></terms></locale>
  <locale xml:lang="de-AT"><terms><term name="and">sowie</term></terms></locale>
  <locale><terms><term name="page" form="short">Seite</term></terms></locale>
  <citation><layout><text variable="title"/></layout></citation>
</style>"#;

fn locales() -> Outcome<Locales> {
	let mut ls = Locales::new();
	res!(ls.insert(res!(Locale::parse(EN_US))));
	res!(ls.insert(res!(Locale::parse(DE_DE))));
	Ok(ls)
}

#[test]
fn csl_locale_language_alone_reaches_its_primary_dialect() -> Outcome<()> {
	let ls = res!(locales());
	let style = res!(Style::parse(STYLE));
	let chain = ls.chain(&style, Some("de"));
	// In-style `de`, then in-style without a language, then de-DE by its primary dialect, then en-US.
	assert_eq!(chain.term_text("and", TermForm::Long, false), Some("und"));
	assert_eq!(chain.term_text("accessed", TermForm::Long, false), Some("abgerufen"));
	assert_eq!(chain.term_text("page", TermForm::Short, false), Some("Seite"));
	assert_eq!(chain.term_text("editor", TermForm::Long, true), Some("editors"));
	assert!(!chain.punctuation_in_quote());
	assert_eq!(Locales::wanted(&style, Some("de")), vec!["de".to_string(), "de-DE".into(), "en-US".into()]);
	Ok(())
}

#[test]
fn csl_locale_in_style_dialect_comes_first() -> Outcome<()> {
	let ls = res!(locales());
	let style = res!(Style::parse(STYLE));
	let chain = ls.chain(&style, Some("de-AT"));
	assert_eq!(chain.term_text("and", TermForm::Long, false), Some("sowie"));
	assert_eq!(chain.term_text("accessed", TermForm::Long, false), Some("abgerufen"));
	// en-US has the date format the German files lack.
	assert!(chain.date(oxedyne_fe2o3_text::csl::style::DateForm::Numeric).is_some());
	let en = ls.chain(&style, None);
	assert_eq!(en.code, "en-US");
	// Only the language-free in-style locale applies to English.
	assert_eq!(en.term_text("page", TermForm::Short, false), Some("Seite"));
	assert_eq!(en.term_text("accessed", TermForm::Long, false), Some("accessed"));
	assert!(en.punctuation_in_quote());
	Ok(())
}

#[test]
fn csl_term_forms_fall_back_after_the_whole_chain() -> Outcome<()> {
	let ls = res!(locales());
	let style = res!(Style::parse(STYLE));
	let chain = ls.chain(&style, Some("en-US"));
	assert_eq!(chain.term_text("editor", TermForm::VerbShort, false), Some("editor"));
	assert_eq!(chain.term_text("editor", TermForm::Symbol, true), Some("eds."));
	assert_eq!(chain.term_text("nothing", TermForm::Long, false), None);
	Ok(())
}

#[test]
fn csl_refuses_a_dependent_style_by_name() -> Outcome<()> {
	let dep = r#"<style xmlns="http://purl.org/net/xbiblio/csl" class="in-text" version="1.0">
  <info><id>urn:dep</id><link href="http://www.zotero.org/styles/apa" rel="independent-parent"/></info>
</style>"#;
	match Style::parse(dep) {
		Ok(_)	=> Err(err!("A dependent style parsed."; Test, Unexpected)),
		Err(e)	=> match format!("{}", e).contains("http://www.zotero.org/styles/apa") {
			true	=> Ok(()),
			false	=> Err(err!("The error does not name the parent: {}", e; Test, Mismatch)),
		},
	}
}
