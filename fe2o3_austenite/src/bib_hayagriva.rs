// SPDX-License-Identifier: Apache-2.0
// Hayagriva YAML read into `bib::Entry`, the model the BibTeX reader fills (B6, 2026-10-12). The YAML is
// parsed by the evaluator's own reader (`eval::lib::data::yaml_parse`, the one behind Typst's `yaml()`);
// this file maps its value. Each field lands under the BibTeX name the formatters read, and a `parent` is
// flattened into the container fields BibLaTeX would hold: a periodical's title becomes `journal`, a book's
// or proceedings' `booktitle`, and the parent's volume, issue, publisher, location, editors and date fill
// whatever the child leaves unset. Types follow hayagriva 0.10's BibLaTeX interop read in reverse.

use crate::bib::{
	Entry,
	EntryKind,
	Name,
};
use crate::eval::value::{
	Dict,
	Value,
};

use oxedyne_fe2o3_core::prelude::*;

/// Reads a parsed Hayagriva document (a dictionary of entries keyed by citation key) into entries, in
/// file order. A fault is returned as the diagnostic's text.
pub fn entries(doc: &Value) -> std::result::Result<Vec<Entry>, String> {
	let top = match doc {
		Value::Dict(d)	=> d,
		Value::None		=> return Ok(Vec::new()),
		other			=> return Err(fmt!(
			"a Hayagriva bibliography is a dictionary of entries, found {}", other.ty().long_name())),
	};
	let mut out = Vec::with_capacity(top.len());
	for (key, v) in top.iter() {
		match v {
			Value::Dict(d)	=> out.push(entry(key, d)),
			other			=> return Err(fmt!(
				"the Hayagriva entry `{}` must be a dictionary, found {}", key, other.ty().long_name())),
		}
	}
	Ok(out)
}

fn entry(key: &str, d: &Dict) -> Entry {
	let ty = text(d.get("type")).map(|t| t.to_ascii_lowercase()).unwrap_or_default();
	let parent = match d.get("parent") {
		Some(Value::Dict(p))	=> Some(&**p),
		Some(Value::Array(a))	=> a.iter().find_map(|v| match v {
			Value::Dict(p)	=> Some(&**p),
			_				=> None,
		}),
		_						=> None,
	};
	let pty = parent.and_then(|p| text(p.get("type"))).map(|t| t.to_ascii_lowercase()).unwrap_or_default();
	let in_event = matches!(pty.as_str(), "proceedings" | "conference");
	let kind = match ty.as_str() {
		"article" if in_event						=> EntryKind::InProceedings,
		"article"									=> EntryKind::Article,
		"chapter" | "anthos" if in_event			=> EntryKind::InProceedings,
		"chapter" | "anthos"						=> EntryKind::InCollection,
		"book" | "anthology" | "proceedings"
			| "reference"							=> EntryKind::Book,
		"thesis"									=> EntryKind::Thesis,
		"report"									=> EntryKind::Report,
		"web" | "blog" | "repository" | "misc"		=> EntryKind::Online,	// as BibTeX's `misc` reads
		"manuscript"								=> EntryKind::Unpublished,
		_											=> EntryKind::Misc,
	};

	let mut f = Fields::default();
	// The child's own fields, then what the parent supplies and the child left unset.
	f.own(d, kind);
	if let Some(p) = parent {
		if let Some(t) = text(p.get("title")) {
			let slot = match kind {
				EntryKind::Article							=> "journal",
				EntryKind::InCollection
					| EntryKind::InProceedings				=> "booktitle",
				_											=> "series",
			};
			f.set(slot, t);
		}
		f.inherit(p);
	}
	let authors = names(d.get("author"));
	let mut editors = names(d.get("editor"));
	if editors.is_empty() {
		if let Some(p) = parent {
			if matches!(kind, EntryKind::InCollection | EntryKind::InProceedings) {
				editors = names(p.get("editor"));
			}
		}
	}
	Entry::new(key.to_string(), kind, f.0, authors, editors)
}

// The fields gathered under their BibTeX names, the first value set for a name kept.
#[derive(Default)]
struct Fields(Vec<(String, String)>);

impl Fields {
	fn set(&mut self, name: &str, value: String) {
		if !value.is_empty() && !self.0.iter().any(|(k, _)| k == name) {
			self.0.push((name.to_string(), value));
		}
	}

	fn own(&mut self, d: &Dict, kind: EntryKind) {
		if let Some(t) = title(d) {
			self.set("title", t);
		}
		self.shared(d);
		if let Some(v) = text(d.get("page-range")).or_else(|| text(d.get("pages"))) {
			self.set("pages", v);
		}
		if let Some(v) = text(d.get("page-total")) {
			self.set("pagetotal", v);
		}
		match d.get("url") {
			Some(Value::Dict(u))	=> {
				if let Some(v) = text(u.get("value")) {
					self.set("url", v);
				}
				if let Some(v) = text(u.get("date")) {
					self.set("urldate", v);
				}
			}
			other					=> if let Some(v) = text(other) {
				self.set("url", v);
			},
		}
		match d.get("serial-number") {
			Some(Value::Dict(s))	=> {
				for (k, v) in s.iter() {
					if let Some(v) = text(Some(v)) {
						let k = k.to_ascii_lowercase();
						match k.as_str() {
							"doi" | "isbn" | "issn" | "pmid" | "pmcid" | "arxiv"	=> self.set(&k, v),
							_														=> self.set("number", v),
						}
					}
				}
			}
			other					=> if let Some(v) = text(other) {
				self.set("number", v);
			},
		}
		if let Some(v) = text(d.get("organization")) {
			let slot = match kind {
				EntryKind::PhdThesis | EntryKind::MastersThesis	=> "school",
				EntryKind::Report | EntryKind::TechReport		=> "institution",
				_												=> "organization",
			};
			self.set(slot, v);
		}
		if let Some(v) = text(d.get("genre")) {
			self.set("type", v);
		}
		for (from, to) in [
			("note",		"note"),
			("language",	"language"),
			("doi",			"doi"),
			("isbn",		"isbn"),
			("issn",		"issn"),
			("abstract",	"abstract"),
			("version",		"version"),
		] {
			if let Some(v) = text(d.get(from)) {
				self.set(to, v);
			}
		}
	}

	// Fields a child and its container share; the child's are read first, so the parent only fills gaps.
	fn shared(&mut self, d: &Dict) {
		if let Some((y, m)) = date(d.get("date")) {
			self.set("year", y);
			if let Some(m) = m {
				self.set("month", m);
			}
		}
		for (from, to) in [
			("volume",		"volume"),
			("issue",		"number"),
			("edition",		"edition"),
			("location",	"address"),
		] {
			if let Some(v) = text(d.get(from)) {
				self.set(to, v);
			}
		}
		match d.get("publisher") {
			Some(Value::Dict(p))	=> {
				if let Some(v) = text(p.get("name")) {
					self.set("publisher", v);
				}
				if let Some(v) = text(p.get("location")) {
					self.set("address", v);
				}
			}
			other					=> if let Some(v) = text(other) {
				self.set("publisher", v);
			},
		}
	}

	fn inherit(&mut self, p: &Dict) {
		self.shared(p);
		if let Some(v) = text(p.get("organization")) {
			self.set("organization", v);
		}
		match p.get("serial-number") {
			Some(Value::Dict(s))	=> {
				for (k, v) in s.iter() {
					let k = k.to_ascii_lowercase();
					if k == "issn" || k == "isbn" {
						if let Some(v) = text(Some(v)) {
							self.set(&k, v);
						}
					}
				}
			}
			_						=> (),
		}
	}
}

// A title: a string, or a dictionary whose `value` is the title (beside `short` and `verbatim`).
fn title(d: &Dict) -> Option<String> {
	match d.get("title") {
		Some(Value::Dict(t))	=> text(t.get("value")),
		other					=> text(other),
	}
}

// A scalar as text: strings trimmed, numbers as written. A dictionary with a `value` reads as that value.
fn text(v: Option<&Value>) -> Option<String> {
	let s = match v {
		Some(Value::Str(s))		=> s.trim().to_string(),
		Some(Value::Int(i))		=> i.to_string(),
		Some(Value::Float(x))	=> x.to_string(),
		Some(Value::Bool(b))	=> b.to_string(),
		Some(Value::Dict(d))	=> return text(d.get("value")),
		_						=> return None,
	};
	if s.is_empty() { None } else { Some(s) }
}

// A Hayagriva date, `YYYY`, `YYYY-MM` or `YYYY-MM-DD` (a year may be signed): the year and the month.
fn date(v: Option<&Value>) -> Option<(String, Option<String>)> {
	let s = match text(v) {
		Some(s)	=> s,
		None	=> return None,
	};
	let (sign, body) = match s.strip_prefix('-') {
		Some(rest)	=> ("-", rest),
		None		=> ("", s.as_str()),
	};
	let mut parts = body.split('-');
	let year = match parts.next() {
		Some(y) if !y.is_empty() && y.chars().all(|c| c.is_ascii_digit())	=> fmt!("{}{}", sign, y),
		_																	=> return Some((s.clone(), None)),
	};
	let month = parts.next()
		.filter(|m| !m.is_empty() && m.chars().all(|c| c.is_ascii_digit()))
		.map(|m| m.trim_start_matches('0').to_string());
	Some((year, month))
}

// Names: one or a list, each a `Family, Given[, Suffix]` string (one with no comma kept whole, as hayagriva
// keeps it) or a dictionary of `name`, `given-name`, `prefix` and `suffix`.
fn names(v: Option<&Value>) -> Vec<Name> {
	match v {
		Some(Value::Array(a))	=> a.iter().filter_map(name).collect(),
		Some(one)				=> name(one).into_iter().collect(),
		None					=> Vec::new(),
	}
}

fn name(v: &Value) -> Option<Name> {
	match v {
		Value::Str(s)	=> {
			let parts: Vec<String> = s.split(',')
				.map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
				.collect();
			match parts.as_slice() {
				[]						=> None,
				[one] if one.is_empty()	=> None,
				[one]					=> Some(Name::literal(one.clone())),
				[family, given, ..]		=> Some(Name::personal(family.clone(), given.clone())),
			}
		}
		Value::Dict(d)	=> {
			let family = match text(d.get("name")) {
				Some(n)	=> n,
				None	=> return None,
			};
			let family = match text(d.get("prefix")) {
				Some(p)	=> fmt!("{} {}", p, family),
				None	=> family,
			};
			match text(d.get("given-name")).or_else(|| text(d.get("given-names"))) {
				Some(g)	=> Some(Name::personal(family, g)),
				None	=> Some(Name::literal(family)),
			}
		}
		_				=> None,
	}
}
