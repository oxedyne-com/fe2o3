use oxedyne_fe2o3_text::csl::{
	archive,
	style::TermForm,
};

use oxedyne_fe2o3_core::prelude::*;


#[test]
fn csl_archive_every_style_resolves_parses_and_matches_its_updated() -> Outcome<()> {
	let mut n = 0;
	for a in archive::styles() {
		for name in a.names {
			let b = res!(archive::by_name(name).ok_or_else(|| err!(
				"Style name '{}' does not resolve.", name; Missing)));
			assert_eq!(b.stem, a.stem);
		}
		let b = res!(archive::by_id(a.id).ok_or_else(|| err!(
			"Style id '{}' does not resolve.", a.id; Missing)));
		assert_eq!(b.stem, a.stem);
		let style = res!(a.style());
		assert_eq!(style.info.id.as_str(), a.id);
		assert_eq!(style.info.updated.as_str(), a.updated);
		assert_eq!(a.path(), format!("styles/{}.csl", a.stem));
		n += 1;
	}
	assert_eq!(n, 80);
	assert!(archive::by_name("no-such-style").is_none());
	Ok(())
}

#[test]
fn csl_archive_every_locale_parses_under_its_own_code() -> Outcome<()> {
	assert_eq!(archive::locale_codes().len(), 63);
	for code in archive::locale_codes() {
		let loc = res!(res!(archive::locale(code)).ok_or_else(|| err!(
			"Locale '{}' is not embedded.", code; Missing)));
		assert_eq!(loc.lang.as_deref(), Some(*code));
	}
	assert!(res!(archive::locale("xx-XX")).is_none());
	assert!(archive::locales_json().contains("primary-dialects"));
	Ok(())
}

#[test]
fn csl_archive_index_tsv_agrees_with_the_embedded_index() -> Outcome<()> {
	let tsv = include_str!("../data/csl/index.tsv");
	let mut styles = 0;
	let mut locales = 0;
	for line in tsv.lines().filter(|l| !l.starts_with('#')) {
		let cols: Vec<&str> = line.split('\t').collect();
		match cols.first() {
			Some(&"style")	=> {
				let a = res!(archive::by_id(cols[2]).ok_or_else(|| err!(
					"index.tsv style '{}' is not in the embedded index.", cols[2]; Missing)));
				assert_eq!(cols[4], a.updated);
				assert_eq!(cols[1], a.path());
				styles += 1;
			}
			Some(&"locale")	=> {
				assert!(archive::has_locale(cols[2]));
				let src = res!(archive::locale_src(cols[2]).ok_or_else(|| err!(
					"Locale '{}' is not embedded.", cols[2]; Missing)));
				assert!(src.contains(&format!("<updated>{}</updated>", cols[4])));
				locales += 1;
			}
			_ => (),
		}
	}
	assert_eq!((styles, locales), (80, 63));
	Ok(())
}

#[test]
fn csl_archive_language_alone_reaches_the_primary_dialect() -> Outcome<()> {
	let a = res!(archive::by_name("apa").ok_or_else(|| err!("apa missing"; Missing)));
	let style = res!(a.style());
	assert_eq!(archive::wanted_paths(&style, Some("de")),
		vec!["locales/de-DE.xml".to_string(), "locales/en-US.xml".to_string()]);
	let files = res!(archive::locales_for(&style, Some("de")));
	let chain = files.chain(&style, Some("de"));
	assert_eq!(chain.term_text("and", TermForm::Long, false), Some("und"));
	// `sr` maps to sr-RS, which hayagriva does not serve, so Typst falls to en-US.
	let files = res!(archive::locales_for(&style, Some("sr")));
	assert_eq!(files.chain(&style, Some("sr")).term_text("and", TermForm::Long, false), Some("and"));
	Ok(())
}
