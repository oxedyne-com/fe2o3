//! The built-in styles and locales: the CSL XML of exactly the 80 styles and 63 locales hayagriva
//! 0.10.1 serves, at the revisions it archived (`data/csl/index.tsv` records each source commit).
//!
//! The index of names, ids and paths is always present under `csl`. The XML itself is embedded
//! only under `csl-archive`, about 4.0 MB; a host without it (the browser build) reads each file
//! as `<path>` inside the virtual package [`PACKAGE`] and supplies it on demand.
//!
//! The data is CC BY-SA 3.0 (the CSL project), except `alphanumeric`, which is MIT/Apache-2.0
//! (the hayagriva authors); `data/csl/LICENSE.txt` holds the notices and travels with the crate.
#[cfg(feature = "csl-archive")]
use crate::csl::{
	Locale,
	Locales,
	Style,
};

#[cfg(feature = "csl-archive")]
use oxedyne_fe2o3_core::prelude::*;

include!("archive_index.rs");

/// The virtual package a host that does not embed the archive serves its files from.
pub const PACKAGE: &str = "@austenite/csl:0.10.1";

/// One built-in style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Archived {
	pub stem:		&'static str,	// file stem under `styles/`
	pub id:			&'static str,	// CSL `<id>`
	pub names:		&'static [&'static str],	// what Typst's `style:` accepts
	pub updated:	&'static str,	// `<updated>` as archived, empty for alphanumeric
}

impl Archived {

	fn at(i: usize) -> Self {
		let (stem, id, names, updated) = STYLES[i];
		Self { stem, id, names, updated }
	}

	/// The style's path inside the archive, and inside [`PACKAGE`].
	pub fn path(&self) -> String {
		format!("styles/{}.csl", self.stem)
	}

	#[cfg(feature = "csl-archive")]
	pub fn xml(&self) -> Outcome<&'static str> {
		Ok(res!(style_xml(self.stem).ok_or_else(|| err!(
			"CSL archive index names style '{}' but its file is not embedded.", self.stem;
			Bug, Missing))))
	}

	#[cfg(feature = "csl-archive")]
	pub fn style(&self) -> Outcome<Style> {
		Style::parse(res!(self.xml()))
	}
}

/// Every built-in style, in hayagriva's order.
pub fn styles() -> impl Iterator<Item = Archived> {
	(0..STYLES.len()).map(Archived::at)
}

/// A built-in style by a name Typst accepts for `style:`, such as `apa` or `ieee`.
pub fn by_name(name: &str) -> Option<Archived> {
	STYLES.iter().position(|(_, _, names, _)| names.contains(&name)).map(Archived::at)
}

/// A built-in style by its CSL id, such as `http://www.zotero.org/styles/apa`.
pub fn by_id(id: &str) -> Option<Archived> {
	STYLES.iter().position(|(_, i, _, _)| *i == id).map(Archived::at)
}

/// Every built-in locale code.
pub fn locale_codes() -> &'static [&'static str] {
	LOCALES
}

/// Is there a built-in locale file of this exact code?
pub fn has_locale(code: &str) -> bool {
	LOCALES.contains(&code)
}

/// A locale's path inside the archive, and inside [`PACKAGE`].
pub fn locale_path(code: &str) -> String {
	format!("locales/{}.xml", code)
}

/// The built-in locale files a style rendered in this code consults, as paths: what a host
/// without the archive must supply. Codes the archive lacks are left out, as hayagriva's
/// lookup finds nothing for them either.
pub fn wanted_paths(style: &crate::csl::Style, code: Option<&str>) -> Vec<String> {
	crate::csl::Locales::wanted(style, code).iter()
		.filter(|c| has_locale(c))
		.map(|c| locale_path(c))
		.collect()
}

#[cfg(feature = "csl-archive")]
pub fn locale_src(code: &str) -> Option<&'static str> {
	locale_xml(code)
}

/// `locales.json` as the CSL locales repository stood at the newest archived locale.
#[cfg(feature = "csl-archive")]
pub fn locales_json() -> &'static str {
	LOCALES_JSON
}

#[cfg(feature = "csl-archive")]
pub fn locale(code: &str) -> Outcome<Option<Locale>> {
	match locale_xml(code) {
		Some(src)	=> Ok(Some(res!(Locale::parse(src)))),
		None		=> Ok(None),
	}
}

/// The built-in locale files a style rendered in this code consults, parsed, ready for
/// [`Locales::chain`].
///
/// The fallback is citationberg's primary-dialect table, the one Typst reaches through
/// hayagriva; `data/csl/locales.json` is carried for hosts but not read, since Typst ignores it.
#[cfg(feature = "csl-archive")]
pub fn locales_for(style: &Style, code: Option<&str>) -> Outcome<Locales> {
	let mut out = Locales::new();
	for c in Locales::wanted(style, code) {
		if let Some(loc) = res!(locale(&c)) {
			res!(out.insert(loc));
		}
	}
	Ok(out)
}
