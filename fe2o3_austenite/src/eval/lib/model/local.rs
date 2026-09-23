// U5 owns this file: the words Typst puts in a document for the reader's language -- the default
// supplements (`Figure`, `Section`), titles (`Contents`, `Bibliography`) and link descriptions.
//
// The `translations/*.txt` tables are Typst 0.15.1's own (`typst-library/translations`, Apache-2.0,
// copyright the Typst project contributors), copied unchanged. Lookup follows `localized_str`: the
// language with its region, then the language alone, then English.

use oxedyne_fe2o3_core::prelude::*;

// Each bundle's name (`de`, `fr-CA`) and its `key = value` lines.
static BUNDLES: &[(&str, &str)] = &[
	("af",	include_str!("translations/af.txt")),
	("alt",	include_str!("translations/alt.txt")),
	("am",	include_str!("translations/am.txt")),
	("ar",	include_str!("translations/ar.txt")),
	("ast",	include_str!("translations/ast.txt")),
	("as",	include_str!("translations/as.txt")),
	("az",	include_str!("translations/az.txt")),
	("be",	include_str!("translations/be.txt")),
	("bg",	include_str!("translations/bg.txt")),
	("bn",	include_str!("translations/bn.txt")),
	("bo",	include_str!("translations/bo.txt")),
	("br",	include_str!("translations/br.txt")),
	("bs",	include_str!("translations/bs.txt")),
	("bua",	include_str!("translations/bua.txt")),
	("ca",	include_str!("translations/ca.txt")),
	("ckb",	include_str!("translations/ckb.txt")),
	("cs",	include_str!("translations/cs.txt")),
	("cu",	include_str!("translations/cu.txt")),
	("cy",	include_str!("translations/cy.txt")),
	("da",	include_str!("translations/da.txt")),
	("de",	include_str!("translations/de.txt")),
	("dsb",	include_str!("translations/dsb.txt")),
	("el",	include_str!("translations/el.txt")),
	("en",	include_str!("translations/en.txt")),
	("eo",	include_str!("translations/eo.txt")),
	("es",	include_str!("translations/es.txt")),
	("et",	include_str!("translations/et.txt")),
	("eu",	include_str!("translations/eu.txt")),
	("fa",	include_str!("translations/fa.txt")),
	("fil",	include_str!("translations/fil.txt")),
	("fi",	include_str!("translations/fi.txt")),
	("fr-CA",	include_str!("translations/fr-CA.txt")),
	("fr",	include_str!("translations/fr.txt")),
	("fur",	include_str!("translations/fur.txt")),
	("ga",	include_str!("translations/ga.txt")),
	("gd",	include_str!("translations/gd.txt")),
	("gl",	include_str!("translations/gl.txt")),
	("grc",	include_str!("translations/grc.txt")),
	("gu",	include_str!("translations/gu.txt")),
	("ha",	include_str!("translations/ha.txt")),
	("he",	include_str!("translations/he.txt")),
	("hi",	include_str!("translations/hi.txt")),
	("hr",	include_str!("translations/hr.txt")),
	("hsb",	include_str!("translations/hsb.txt")),
	("hu",	include_str!("translations/hu.txt")),
	("hy",	include_str!("translations/hy.txt")),
	("ia",	include_str!("translations/ia.txt")),
	("id",	include_str!("translations/id.txt")),
	("is",	include_str!("translations/is.txt")),
	("isv",	include_str!("translations/isv.txt")),
	("it",	include_str!("translations/it.txt")),
	("ja",	include_str!("translations/ja.txt")),
	("ka",	include_str!("translations/ka.txt")),
	("kmr",	include_str!("translations/kmr.txt")),
	("km",	include_str!("translations/km.txt")),
	("kn",	include_str!("translations/kn.txt")),
	("ko",	include_str!("translations/ko.txt")),
	("ku",	include_str!("translations/ku.txt")),
	("la",	include_str!("translations/la.txt")),
	("lb",	include_str!("translations/lb.txt")),
	("lo",	include_str!("translations/lo.txt")),
	("lt",	include_str!("translations/lt.txt")),
	("lv",	include_str!("translations/lv.txt")),
	("mk",	include_str!("translations/mk.txt")),
	("ml",	include_str!("translations/ml.txt")),
	("mr",	include_str!("translations/mr.txt")),
	("ms",	include_str!("translations/ms.txt")),
	("nb",	include_str!("translations/nb.txt")),
	("nl",	include_str!("translations/nl.txt")),
	("nn",	include_str!("translations/nn.txt")),
	("no",	include_str!("translations/no.txt")),
	("oc",	include_str!("translations/oc.txt")),
	("or",	include_str!("translations/or.txt")),
	("pa",	include_str!("translations/pa.txt")),
	("pl",	include_str!("translations/pl.txt")),
	("pms",	include_str!("translations/pms.txt")),
	("pt-PT",	include_str!("translations/pt-PT.txt")),
	("pt",	include_str!("translations/pt.txt")),
	("rm",	include_str!("translations/rm.txt")),
	("ro",	include_str!("translations/ro.txt")),
	("ru",	include_str!("translations/ru.txt")),
	("se",	include_str!("translations/se.txt")),
	("si",	include_str!("translations/si.txt")),
	("sk",	include_str!("translations/sk.txt")),
	("sl",	include_str!("translations/sl.txt")),
	("sq",	include_str!("translations/sq.txt")),
	("sr",	include_str!("translations/sr.txt")),
	("sv",	include_str!("translations/sv.txt")),
	("ta",	include_str!("translations/ta.txt")),
	("te",	include_str!("translations/te.txt")),
	("th",	include_str!("translations/th.txt")),
	("tk",	include_str!("translations/tk.txt")),
	("tl",	include_str!("translations/tl.txt")),
	("tr",	include_str!("translations/tr.txt")),
	("ug",	include_str!("translations/ug.txt")),
	("uk",	include_str!("translations/uk.txt")),
	("ur",	include_str!("translations/ur.txt")),
	("vi",	include_str!("translations/vi.txt")),
	("zh-TW",	include_str!("translations/zh-TW.txt")),
	("zh",	include_str!("translations/zh.txt")),
];

/// The word for `key` (`figure`, `heading`, `page`, ...) in `lang`, optionally narrowed by `region`.
pub fn local_name(key: &str, lang: &str, region: Option<&str>) -> &'static str {
	if let Some(r) = region {
		let name = fmt!("{}-{}", lang, r);
		if let Some(v) = lookup(&name, key) {
			return v;
		}
	}
	if let Some(v) = lookup(lang, key) {
		return v;
	}
	lookup("en", key).unwrap_or("")
}

fn lookup(bundle: &str, key: &str) -> Option<&'static str> {
	let text = BUNDLES.iter().find(|(n, _)| n.eq_ignore_ascii_case(bundle)).map(|(_, t)| *t);
	let text = match text {
		Some(t)	=> t,
		None	=> return None,
	};
	for line in text.lines() {
		let line = line.trim();
		if line.is_empty() || line.starts_with('#') {
			continue;
		}
		if let Some((k, v)) = line.split_once('=') {
			if k.trim() == key {
				return Some(v.trim());
			}
		}
	}
	None
}
