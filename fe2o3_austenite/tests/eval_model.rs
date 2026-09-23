//! U5: the model elements against the `typst` 0.15.1 oracle. Four comparisons, none against Austenite's
//! own output:
//!
//! * construction -- each line of `construct.txt` is evaluated by `typst eval` and by Austenite in code
//!   mode; the element's fields (as JSON, in order) or the first error must agree;
//! * materialised and synthesised fields -- for each `fields_*.typ`, `query(kind).map(e => e.fields())`
//!   against Austenite's elements after preparation (location, unset fields from the style chain and
//!   the schema's defaults, then `model::synthesise`), over two passes so a reference sees its target;
//! * local names -- each language's supplements and titles against what Typst synthesises;
//! * bibliographies -- IEEE and Chicago author-date entries and citations against Typst's HTML export.
//!
//! Preparation is done here the way realisation's `prepare` does it, plus the two family hooks this unit
//! adds (computed defaults, synthesis), because realisation does not call them yet (see the U5 report).
//! A case needing another unit is marked `%%[U3]` and checked from the moment that unit is present.
//! `EVAL_ORACLE_SKIP=1` skips explicitly; a missing `typst` otherwise fails.

#[path = "eval_oracle/json.rs"]
#[allow(dead_code)]
mod json;

use json::J;

use oxedyne_fe2o3_austenite::bib::{
	Bibliography,
	CiteForm,
	CiteStyle,
};
use oxedyne_fe2o3_austenite::eval::content::{
	Content,
	ElemKind,
	Family,
	FieldId,
};
use oxedyne_fe2o3_austenite::eval::eval::{
	eval_string,
	EvalMode,
};
use oxedyne_fe2o3_austenite::eval::intro::{
	CounterKey,
	Introspector,
};
use oxedyne_fe2o3_austenite::eval::lib::foundations::repr;
use oxedyne_fe2o3_austenite::eval::lib::model;
use oxedyne_fe2o3_austenite::eval::lib::model::local::local_name;
use oxedyne_fe2o3_austenite::eval::scope::Scope;
use oxedyne_fe2o3_austenite::eval::select::Selector;
use oxedyne_fe2o3_austenite::eval::styles::StyleChain;
use oxedyne_fe2o3_austenite::eval::value::{
	Alignment,
	HAlign,
	Length,
	VAlign,
	Value,
};
use oxedyne_fe2o3_austenite::eval::{
	eval_source,
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;
use std::path::{
	Path,
	PathBuf,
};
use std::process::Command;
use std::sync::Arc;

// Oracle plumbing

fn dir() -> PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval/model") }

fn skip() -> bool { std::env::var("EVAL_ORACLE_SKIP").map(|v| v == "1").unwrap_or(false) }

/// Runs `typst` under a 3G memory cap when systemd can scope it; environment expansion is off so a `$`
/// in an argument reaches typst.
fn typst(args: &[&str]) -> Outcome<(bool, String, String)> {
	let mut capped = vec!["--user", "--scope", "--quiet", "--expand-environment=no", "-p", "MemoryMax=3G",
		"--slice=claude-rc.slice", "typst"];
	capped.extend_from_slice(args);
	let out = match Command::new("systemd-run").args(&capped).output() {
		Ok(o) if o.status.code() != Some(127) && !String::from_utf8_lossy(&o.stderr).contains("Failed to") => o,
		_ => res!(Command::new("typst").args(args).output().map_err(|e| err!(
			"The typst oracle could not run ({}); set EVAL_ORACLE_SKIP=1 to skip explicitly.", e; IO, Missing))),
	};
	Ok((out.status.success(), String::from_utf8_lossy(&out.stdout).to_string(),
		String::from_utf8_lossy(&out.stderr).to_string()))
}

fn first_error(stderr: &str) -> String {
	stderr.lines().find_map(|l| l.strip_prefix("error: ")).unwrap_or(stderr.trim()).to_string()
}

// Values as `typst eval` serialises them

fn num(f: f64) -> String {
	let r = (f * 100.0).round() / 100.0;
	let s = fmt!("{:.2}", r);
	let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
	if s == "-0" { "0".to_string() } else { s }
}

fn length(l: &Length) -> String {
	match (l.abs != 0.0, l.em != 0.0) {
		(true, true)	=> fmt!("{}pt + {}em", num(l.abs), num(l.em)),
		(false, true)	=> fmt!("{}em", num(l.em)),
		_				=> fmt!("{}pt", num(l.abs)),
	}
}

fn alignment(a: &Alignment) -> String {
	let x = a.x.map(|x| match x {
		HAlign::Start	=> "start",
		HAlign::Left	=> "left",
		HAlign::Center	=> "center",
		HAlign::Right	=> "right",
		HAlign::End		=> "end",
	});
	let y = a.y.map(|y| match y {
		VAlign::Top		=> "top",
		VAlign::Horizon	=> "horizon",
		VAlign::Bottom	=> "bottom",
	});
	match (x, y) {
		(Some(x), Some(y))	=> fmt!("{} + {}", x, y),
		(Some(x), None)		=> x.to_string(),
		(None, Some(y))		=> y.to_string(),
		(None, None)		=> "start".to_string(),
	}
}

fn selector(s: &Selector) -> String {
	match s {
		Selector::Elem(k, None)		=> k.path().to_string(),
		Selector::Elem(k, Some(w))	=> {
			let parts: Vec<String> = w.iter().map(|(id, v)| {
				let name = k.field_spec(*id).map(|s| s.name).unwrap_or("?");
				fmt!("{}: {}", name, leaf(v))
			}).collect();
			fmt!("{}.where({})", k.path(), parts.join(", "))
		}
		Selector::Label(l)			=> fmt!("<{}>", l.as_str()),
		_							=> "selector(..)".to_string(),
	}
}

/// The `repr` of a value JSON cannot hold, for the kinds a model field holds; anything else through
/// U3's `repr`, whose stub makes such a case pend rather than pass.
fn leaf(v: &Value) -> String {
	match v {
		Value::Auto				=> "auto".to_string(),
		Value::None				=> "none".to_string(),
		Value::Int(i)			=> i.to_string(),
		Value::Str(s)			=> fmt!("\"{}\"", s),
		Value::Length(l)		=> length(l),
		Value::Ratio(r)			=> fmt!("{}%", num(r.0 * 100.0)),
		Value::Relative(r)		=> fmt!("{}% + {}", num(r.rel.0 * 100.0), length(&r.abs)),
		Value::Fraction(f)		=> fmt!("{}fr", num(f.0)),
		Value::Alignment(a)		=> alignment(a),
		Value::Label(l)			=> fmt!("<{}>", l.as_str()),
		Value::Func(f)			=> f.name().unwrap_or("(..) => ..").to_string(),
		Value::Selector(s)		=> selector(s),
		Value::Counter(c)		=> match &c.key {
			CounterKey::Page		=> "counter(page)".to_string(),
			CounterKey::Str(s)		=> fmt!("counter(\"{}\")", s),
			CounterKey::Selector(s)	=> fmt!("counter({})", selector(s)),
		},
		other					=> repr(other),
	}
}

fn to_json(v: &Value) -> J {
	match v {
		Value::None			=> J::Null,
		Value::Bool(b)		=> J::Bool(*b),
		Value::Int(i)		=> J::Int(*i),
		Value::Float(f)		=> J::Float(*f),
		Value::Str(s)		=> J::Str(s.to_string()),
		Value::Array(a)		=> J::Arr(a.iter().map(to_json).collect()),
		Value::Dict(d)		=> J::Obj(d.iter().map(|(k, x)| (k.to_string(), to_json(x))).collect()),
		Value::Content(c)	=> content_json(c),
		other				=> J::Str(leaf(other)),
	}
}

fn content_json(c: &Content) -> J {
	let mut kv: Vec<(String, J)> = Vec::new();
	match c {
		Content::Sequence(s) => {
			kv.push(("func".to_string(), J::str("sequence")));
			kv.push(("children".to_string(), J::Arr(s.children.iter().map(content_json).collect())));
		}
		Content::Styled(s) => {
			kv.push(("func".to_string(), J::str("styled")));
			kv.push(("child".to_string(), content_json(&s.child)));
			kv.push(("styles".to_string(), J::str("styles(..)")));
		}
		Content::Elem(e) => {
			kv.push(("func".to_string(), J::str(e.kind.name())));
			let mut fields: Vec<&(FieldId, Value)> = e.fields.iter().collect();
			fields.sort_by_key(|(id, _)| id.0);
			for (id, v) in fields {
				let name = e.kind.field_spec(*id).map(|s| s.name).unwrap_or("?");
				if e.kind.family() == Family::Model && model::is_internal(e.kind, name) {
					continue;
				}
				kv.push((name.to_string(), to_json(v)));
			}
		}
	}
	if let Some(l) = c.label() {
		kv.push(("label".to_string(), J::Str(fmt!("<{}>", l.as_str()))));
	}
	J::Obj(kv)
}

/// A raw line's body is compared by its text: Typst highlights it, which Austenite does not yet.
fn loosen(j: &J) -> J {
	match j {
		J::Obj(kv) => {
			let raw_line = j.get("func").and_then(|f| f.as_str()) == Some("line") && j.get("count").is_some();
			J::Obj(kv.iter().map(|(k, v)| {
				if raw_line && k == "body" {
					(k.clone(), J::Str(plain(v)))
				} else {
					(k.clone(), loosen(v))
				}
			}).collect())
		}
		J::Arr(a)	=> J::Arr(a.iter().map(loosen).collect()),
		other		=> other.clone(),
	}
}

fn plain(j: &J) -> String {
	match j {
		J::Obj(_) => match (j.get("text"), j.get("children"), j.get("child")) {
			(Some(J::Str(t)), _, _)		=> t.clone(),
			(_, Some(J::Arr(a)), _)		=> a.iter().map(plain).collect(),
			(_, _, Some(c))				=> plain(c),
			_							=> String::new(),
		},
		_ => String::new(),
	}
}

/// Is the U3 library present (its `repr` no longer a stub)?
fn have_u3() -> bool { repr(&Value::Int(1)) == "1" }

/// Does the rendering hold a value only U3's `repr` can print? Its stub prints `<type>`.
fn pends_on_u3(j: &J) -> bool {
	let r = j.render();
	!have_u3() && ["<color>", "<stroke>", "<angle>", "<datetime>", "<regex>", "<symbol>", "<bytes>", "<version>",
		"<float>", "<dictionary>", "<array>", "<function>"].iter().any(|t| r.contains(t))
}

// Construction

#[test]
fn construction_matches_the_typst_oracle() -> Outcome<()> {
	if skip() {
		println!("EVAL_ORACLE_SKIP=1: oracle comparison skipped");
		return Ok(());
	}
	let text = res!(std::fs::read_to_string(dir().join("construct.txt")).map_err(|e| err!("{}", e; IO)));
	let mut checked = 0;
	let mut pending = Vec::new();
	let mut failures = Vec::new();
	for line in text.lines() {
		let line = line.trim();
		if line.is_empty() || line.starts_with(";;") {
			continue;
		}
		if let Some(c) = line.strip_prefix("%%[CSL] ") {
			pending.push(c.to_string());
			continue;
		}
		let code = match line.strip_prefix("%%[U3] ") {
			Some(c) if !have_u3() => {
				pending.push(c.to_string());
				continue;
			}
			Some(c)	=> c,
			None	=> line,
		};
		let (ok, out, err) = res!(typst(&["eval", "--", code]));
		let mut engine = Engine::new(World::new(PathBuf::from("/")));
		let got = eval_string(&mut engine, code, EvalMode::Code, Scope::new(), Span::detached());
		checked += 1;
		match (ok, got) {
			(true, Ok(v)) => {
				let want = match json::parse(out.trim()) {
					Ok(j)	=> loosen(&j),
					Err(_)	=> {
						failures.push(fmt!("{}\n  unparsable oracle output {}", code, out.trim()));
						continue;
					}
				};
				let have = loosen(&to_json(&v));
				if pends_on_u3(&have) {
					pending.push(code.to_string());
					continue;
				}
				if want.render() != have.render() {
					failures.push(fmt!("{}\n  typst     {}\n  austenite {}", code, want.render(), have.render()));
				}
			}
			(false, Err(_)) => {
				let want = first_error(&err);
				let have = engine.diags.iter().find(|d| d.is_error()).map(|d| d.message.clone()).unwrap_or_default();
				if want != have {
					failures.push(fmt!("{}\n  typst error     {}\n  austenite error {}", code, want, have));
				}
			}
			(true, Err(e)) => failures.push(fmt!("{}\n  typst accepts it, austenite: {}", code, e)),
			(false, Ok(v)) => failures.push(fmt!("{}\n  typst refuses it ({}), austenite gives {}", code,
				first_error(&err), to_json(&v).render())),
		}
	}
	println!("construction: {} checked, {} pending (U3 or CSL), {} failed", checked, pending.len(), failures.len());
	assert!(checked > 120, "the construction corpus shrank to {}", checked);
	assert!(failures.is_empty(), "{} case(s) differ from typst:\n{}", failures.len(), failures.join("\n"));
	Ok(())
}

// Preparation and synthesis

/// Evaluates a fixture file into its content.
fn load(path: &Path) -> Outcome<(Engine, Content)> {
	let text = res!(std::fs::read_to_string(path).map_err(|e| err!("{}", e; IO)));
	let mut world = World::new(dir());
	let id = res!(world.add_source(path.to_path_buf(), text));
	let mut engine = Engine::new(world);
	let module = res!(eval_source(&mut engine, id));
	Ok((engine, module.content.clone()))
}

/// The model elements Typst 0.15 locates, which `ElemKind::locatable` should report (see the U5 report);
/// until it does, the walk locates them itself so their queries can be compared.
const TYPST_LOCATABLE: &[ElemKind] = &[ElemKind::Par, ElemKind::Strong, ElemKind::Emph, ElemKind::Raw,
	ElemKind::Title, ElemKind::List, ElemKind::Enum, ElemKind::Terms, ElemKind::Link, ElemKind::Ref,
	ElemKind::FootnoteEntry, ElemKind::FigureCaption, ElemKind::OutlineEntry, ElemKind::Quote];

fn located(k: ElemKind) -> bool { k.locatable() || TYPST_LOCATABLE.contains(&k) }

/// Realisation's preparation: a location for a locatable or labelled element, unset settable fields
/// from the chain (or the model's computed default), then the model's synthesis.
fn prepare(engine: &mut Engine, c: &Content, chain: &StyleChain) -> Outcome<Content> {
	let mut out = c.clone();
	if let Content::Elem(e) = &mut out {
		let e = Arc::make_mut(e);
		if e.location.is_none() && (located(e.kind) || e.label.is_some()) {
			e.location = Some(engine.locator.locate(e.kind, e.span));
		}
		for (i, spec) in e.kind.fields().iter().enumerate() {
			let id = FieldId(i as u8);
			if !spec.settable || e.fields.iter().any(|(f, _)| *f == id) {
				continue;
			}
			let v = match res!(chain.get(e.kind, id)) {
				Some(v)	=> Some(v),
				None if e.kind.family() == Family::Model => model::default_value(e.kind, spec.name),
				None	=> None,
			};
			if let Some(v) = v {
				e.fields.push((id, v));
			}
		}
		e.prepared = true;
	}
	if out.kind().map(|k| k.family() == Family::Model).unwrap_or(false) {
		res!(model::synthesise(engine, &mut out, chain));
	}
	Ok(out)
}

/// Walks the content as realisation would reach it, preparing each element and keeping the located
/// ones in document order. Show rules are not applied: the fixtures have none, and preparation happens
/// before any.
fn walk(engine: &mut Engine, c: &Content, chain: &StyleChain, sink: &mut Vec<Content>) -> Outcome<()> {
	match c {
		Content::Sequence(s) => {
			for x in &s.children {
				res!(walk(engine, x, chain, sink));
			}
		}
		Content::Styled(s) => {
			let inner = chain.chain(&s.styles);
			res!(walk(engine, &s.child, &inner, sink));
		}
		Content::Elem(_) => {
			let prepared = res!(prepare(engine, c, chain));
			if prepared.location().is_some() {
				sink.push(prepared.clone());
			}
			if let Content::Elem(e) = c {
				for (_, v) in &e.fields {
					res!(walk_value(engine, v, chain, sink));
				}
			}
		}
	}
	Ok(())
}

fn walk_value(engine: &mut Engine, v: &Value, chain: &StyleChain, sink: &mut Vec<Content>) -> Outcome<()> {
	match v {
		Value::Content(c)	=> walk(engine, c, chain, sink),
		Value::Array(a)		=> {
			for x in a.iter() {
				res!(walk_value(engine, x, chain, sink));
			}
			Ok(())
		}
		_					=> Ok(()),
	}
}

/// Two passes: the second sees the first's elements, as the fixpoint's second pass does.
fn realised(engine: &mut Engine, content: &Content) -> Outcome<Vec<Content>> {
	let mut first = Vec::new();
	res!(walk(engine, content, &StyleChain::root(), &mut first));
	let mut index = HashMap::new();
	for (i, e) in first.iter().enumerate() {
		if let Some(l) = e.location() {
			index.insert(l, i);
		}
	}
	engine.intro = Arc::new(Introspector { elems: first, index, ..Introspector::default() });
	engine.locator.reset();
	let mut second = Vec::new();
	res!(walk(engine, content, &StyleChain::root(), &mut second));
	Ok(second)
}

fn kind_named(name: &str) -> Option<ElemKind> { ElemKind::ALL.iter().copied().find(|k| k.path() == name) }

#[test]
fn materialised_fields_match_the_typst_oracle() -> Outcome<()> {
	if skip() {
		println!("EVAL_ORACLE_SKIP=1: oracle comparison skipped");
		return Ok(());
	}
	let mut files: Vec<PathBuf> = res!(std::fs::read_dir(dir()).map_err(|e| err!("{}", e; IO)))
		.filter_map(|e| e.ok().map(|e| e.path()))
		.filter(|p| p.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with("fields_")).unwrap_or(false))
		.collect();
	files.sort();
	let mut failures = Vec::new();
	let mut compared = 0;
	for f in &files {
		let text = res!(std::fs::read_to_string(f).map_err(|e| err!("{}", e; IO)));
		if text.lines().any(|l| l.trim() == "// needs: U3") && !have_u3() {
			println!("{}: pending on U3", f.display());
			continue;
		}
		let kinds: Vec<String> = text.lines().next().and_then(|l| l.strip_prefix("// query: "))
			.map(|q| q.split_whitespace().map(|s| s.to_string()).collect()).unwrap_or_default();
		let (mut engine, content) = match load(f) {
			Ok(x)	=> x,
			Err(e)	=> {
				failures.push(fmt!("{}: austenite cannot evaluate it: {}", f.display(), e));
				continue;
			}
		};
		let elems = match realised(&mut engine, &content) {
			Ok(x)	=> x,
			Err(e)	=> {
				let d: Vec<String> = engine.diags.iter().map(|d| d.message.clone()).collect();
				failures.push(fmt!("{}: preparation failed: {} {:?}", f.display(), e, d));
				continue;
			}
		};
		for k in &kinds {
			let kind = match kind_named(k) {
				Some(x)	=> x,
				None	=> return Err(err!("unknown kind {} in {}", k, f.display(); Input, Invalid)),
			};
			let fpath = f.to_string_lossy().to_string();
			let (ok, out, err) = res!(typst(&["eval", "--in", &fpath, &fmt!("query({}).map(e => e.fields())", k)]));
			if !ok {
				failures.push(fmt!("{} {}: typst failed: {}", f.display(), k, first_error(&err)));
				continue;
			}
			let want = loosen(&res!(json::parse(out.trim())));
			let have = loosen(&J::Arr(elems.iter().filter(|e| e.is(kind)).map(|e| {
				// `fields()` has no `func` key; it is the element's own dictionary.
				match content_json(e) {
					J::Obj(kv)	=> J::Obj(kv.into_iter().filter(|(k, _)| k != "func").collect()),
					other		=> other,
				}
			}).collect()));
			compared += 1;
			if want.render() != have.render() {
				let mut d = Vec::new();
				json::diff(k, &want, &have, &mut d);
				failures.push(fmt!("{} {}:\n  typst     {}\n  austenite {}\n  {}", f.display(), k,
					json::clip(&want.render()), json::clip(&have.render()), d.join("\n  ")));
			}
		}
	}
	println!("fields: {} queries compared over {} documents, {} failed", compared, files.len(), failures.len());
	assert!(failures.is_empty(), "{} query(ies) differ from typst:\n{}", failures.len(), failures.join("\n"));
	assert!(compared >= 9, "only {} queries were compared", compared);
	Ok(())
}

// Local names

const LANGS: &[(&str, Option<&str>)] = &[
	("en", None), ("de", None), ("fr", None), ("fr", Some("CA")), ("es", None), ("it", None), ("pt", None),
	("pt", Some("PT")), ("nl", None), ("sv", None), ("da", None), ("nb", None), ("nn", None), ("fi", None),
	("pl", None), ("cs", None), ("ru", None), ("uk", None), ("el", None), ("tr", None), ("ar", None),
	("he", None), ("zh", None), ("zh", Some("TW")), ("ja", None), ("ko", None), ("hi", None), ("la", None),
	("ca", None), ("hu", None), ("ro", None), ("xx", None),
];

#[test]
fn local_names_match_the_typst_oracle() -> Outcome<()> {
	if skip() {
		println!("EVAL_ORACLE_SKIP=1: oracle comparison skipped");
		return Ok(());
	}
	// One document: per language a heading, a figure of each named kind and an outline, whose synthesised
	// supplements and default title Typst reports.
	let mut doc = String::new();
	for (l, r) in LANGS {
		let region = r.map(|r| fmt!(", region: \"{}\"", r)).unwrap_or_default();
		doc.push_str(&fmt!("#[#set text(lang: \"{}\"{})\n= H\n#figure(rect())\n#figure(table[a])\n#figure(`x`)\n]\n",
			l, region));
	}
	let work = std::env::temp_dir().join(fmt!("u5-local-{}", std::process::id()));
	res!(std::fs::create_dir_all(&work).map_err(|e| err!("{}", e; IO)));
	let path = work.join("local.typ");
	res!(std::fs::write(&path, &doc).map_err(|e| err!("{}", e; IO)));
	let p = path.to_string_lossy().to_string();
	let (ok, out, err) = res!(typst(&["eval", "--in", &p,
		"(query(heading).map(h => h.supplement.text), query(figure).map(f => f.supplement.text))"]));
	let _ = std::fs::remove_dir_all(&work);
	assert!(ok, "typst failed: {}", first_error(&err));
	let got = res!(json::parse(out.trim()));
	let (heads, figs) = match &got {
		J::Arr(a) if a.len() == 2 => (a[0].clone(), a[1].clone()),
		other => return Err(err!("unexpected oracle output {}", other.render(); Input, Invalid)),
	};
	let mut failures = Vec::new();
	for (i, (l, r)) in LANGS.iter().enumerate() {
		let want_h = match &heads { J::Arr(a) => a.get(i).and_then(|x| x.as_str()).unwrap_or("").to_string(), _ => String::new() };
		let have_h = local_name("heading", l, *r);
		if want_h != have_h {
			failures.push(fmt!("{}-{:?} heading: typst {:?}, austenite {:?}", l, r, want_h, have_h));
		}
		for (j, key) in ["figure", "table", "raw"].iter().enumerate() {
			let want = match &figs { J::Arr(a) => a.get(i * 3 + j).and_then(|x| x.as_str()).unwrap_or("").to_string(), _ => String::new() };
			let have = local_name(key, l, *r);
			if want != have {
				failures.push(fmt!("{}-{:?} {}: typst {:?}, austenite {:?}", l, r, key, want, have));
			}
		}
	}
	println!("local names: {} languages, {} differences", LANGS.len(), failures.len());
	assert!(failures.is_empty(), "{}", failures.join("\n"));
	Ok(())
}

// Bibliographies

/// The text of every `<li>` of the bibliography section and every citation link, entities decoded.
fn html_texts(html: &str) -> (Vec<String>, Vec<String>) {
	fn strip(s: &str) -> String {
		let mut out = String::new();
		let mut in_tag = false;
		for c in s.chars() {
			match c {
				'<'	=> in_tag = true,
				'>'	=> in_tag = false,
				c if !in_tag => out.push(c),
				_	=> (),
			}
		}
		out.replace("&#x20;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">")
			.replace("&quot;", "\"").replace("&#39;", "'")
	}
	let mut entries = Vec::new();
	let section = html.split("role=\"doc-bibliography\"").nth(1).unwrap_or("");
	for li in section.split("<li").skip(1) {
		let body = li.split("</li>").next().unwrap_or("");
		let body = body.splitn(2, '>').nth(1).unwrap_or("");
		entries.push(strip(body).trim().to_string());
	}
	let mut cites = Vec::new();
	let before = html.split("role=\"doc-bibliography\"").next().unwrap_or("");
	for a in before.split("role=\"doc-biblioref\">").skip(1) {
		cites.push(strip(a.split("</a>").next().unwrap_or("")).to_string());
	}
	(entries, cites)
}

const BIB_KEYS: &[&str] = &["smith2020", "brown2018", "lee2019", "gray2015", "web2021", "tech2017", "phd2016",
	"many2022"];

#[test]
fn bibliography_formatting_matches_the_typst_oracle() -> Outcome<()> {
	if skip() {
		println!("EVAL_ORACLE_SKIP=1: oracle comparison skipped");
		return Ok(());
	}
	let bib_text = res!(std::fs::read_to_string(dir().join("refs.bib")).map_err(|e| err!("{}", e; IO)));
	let lib = res!(Bibliography::parse(&bib_text));
	let cited: Vec<String> = BIB_KEYS.iter().map(|k| k.to_string()).collect();
	let mut failures = Vec::new();
	let mut compared = 0;
	for (style, name) in [(CiteStyle::Ieee, "ieee"), (CiteStyle::ChicagoAuthorDate, "chicago-author-date")] {
		let src = dir().join(fmt!("bib_{}.typ", name));
		let work = std::env::temp_dir().join(fmt!("u5-bib-{}-{}", name, std::process::id()));
		res!(std::fs::create_dir_all(&work).map_err(|e| err!("{}", e; IO)));
		let html = work.join("out.html");
		let (s, h) = (src.to_string_lossy().to_string(), html.to_string_lossy().to_string());
		let (ok, _, err) = res!(typst(&["compile", "--features", "html", "--format", "html", &s, &h]));
		assert!(ok, "typst failed on {}: {}", s, first_error(&err));
		let text = res!(std::fs::read_to_string(&html).map_err(|e| err!("{}", e; IO)));
		let _ = std::fs::remove_dir_all(&work);
		let (want_entries, want_cites) = html_texts(&text);
		// The list.
		let have: Vec<String> = lib.render_list(style, &cited, false).iter().map(|e| match &e.prefix {
			Some(p)	=> fmt!("{} {}", p, e.body.plain()),
			None	=> e.body.plain(),
		}).collect();
		if want_entries.len() != have.len() {
			failures.push(fmt!("{}: typst lists {} entries, austenite {}", name, want_entries.len(), have.len()));
		}
		for (w, h) in want_entries.iter().zip(have.iter()) {
			compared += 1;
			if norm(w) != norm(h) {
				failures.push(fmt!("{} entry:\n  typst     {}\n  austenite {}", name, w, h));
			}
		}
		// The citations, in the fixture's order: eight single ones (the second and third cited together),
		// then a supplement, prose, author, year and full form.
		let mut have_cites = Vec::new();
		let one = |k: &str, f: CiteForm, sup: Option<&str>| -> Outcome<String> {
			let runs = res!(lib.render_citation(style, k, f, sup, &cited));
			Ok(runs.iter().map(|r| r.text.as_str()).collect())
		};
		have_cites.push(res!(one("smith2020", CiteForm::Normal, None)));
		let group = res!(lib.render_group(style, &[("brown2018", None), ("lee2019", None)], &cited));
		for g in group {
			have_cites.push(g.iter().map(|r| r.text.as_str()).collect());
		}
		for k in &BIB_KEYS[3..] {
			have_cites.push(res!(one(k, CiteForm::Normal, None)));
		}
		have_cites.push(res!(one("smith2020", CiteForm::Normal, Some("p. 5"))));
		// Typst's HTML links an author-date citation inside its parentheses and group punctuation.
		if style == CiteStyle::ChicagoAuthorDate {
			for c in have_cites.iter_mut() {
				*c = c.trim_start_matches('(').trim_end_matches(|x| x == ')' || x == ';').to_string();
			}
		}
		have_cites.push(res!(one("smith2020", CiteForm::Prose, None)));
		have_cites.push(res!(one("brown2018", CiteForm::Author, None)));
		have_cites.push(res!(one("brown2018", CiteForm::Year, None)));
		let mut full = res!(one("lee2019", CiteForm::Full, None));
		// The full stop closing an author-date reference sits outside the link.
		if style == CiteStyle::ChicagoAuthorDate {
			full = full.trim_end_matches('.').to_string();
		}
		have_cites.push(full);
		if want_cites.len() != have_cites.len() {
			failures.push(fmt!("{}: typst has {} citations {:?}, austenite {} {:?}", name, want_cites.len(),
				want_cites, have_cites.len(), have_cites));
		}
		for (w, h) in want_cites.iter().zip(have_cites.iter()) {
			compared += 1;
			if norm(w) != norm(h) {
				failures.push(fmt!("{} citation:\n  typst     {}\n  austenite {}", name, w, h));
			}
		}
	}
	println!("bibliography: {} strings compared, {} differ", compared, failures.len());
	assert!(compared > 30, "only {} strings were compared", compared);
	assert!(failures.is_empty(), "{}", failures.join("\n"));
	Ok(())
}

fn norm(s: &str) -> String { s.split_whitespace().collect::<Vec<_>>().join(" ") }

#[test]
fn bibliography_lists_match_the_typst_oracle() -> Outcome<()> {
	if skip() {
		println!("EVAL_ORACLE_SKIP=1: oracle comparison skipped");
		return Ok(());
	}
	let bib_text = res!(std::fs::read_to_string(dir().join("refs2.bib")).map_err(|e| err!("{}", e; IO)));
	let lib = res!(Bibliography::parse(&bib_text));
	let mut failures = Vec::new();
	let mut compared = 0;
	for (style, name) in [(CiteStyle::Ieee, "ieee"), (CiteStyle::ChicagoAuthorDate, "chicago-author-date")] {
		let src = dir().join(fmt!("bib2_{}.typ", name));
		let text = res!(std::fs::read_to_string(&src).map_err(|e| err!("{}", e; IO)));
		let cited: Vec<String> = text.lines().next().and_then(|l| l.strip_prefix("// cited: "))
			.map(|q| q.split_whitespace().map(|s| s.to_string()).collect()).unwrap_or_default();
		let work = std::env::temp_dir().join(fmt!("u5-bib2-{}-{}", name, std::process::id()));
		res!(std::fs::create_dir_all(&work).map_err(|e| err!("{}", e; IO)));
		let html = work.join("out.html");
		let (s, h) = (src.to_string_lossy().to_string(), html.to_string_lossy().to_string());
		let (ok, _, err) = res!(typst(&["compile", "--features", "html", "--format", "html", &s, &h]));
		assert!(ok, "typst failed on {}: {}", s, first_error(&err));
		let out = res!(std::fs::read_to_string(&html).map_err(|e| err!("{}", e; IO)));
		let _ = std::fs::remove_dir_all(&work);
		let (want, _) = html_texts(&out);
		let have: Vec<String> = lib.render_list(style, &cited, false).iter().map(|e| match &e.prefix {
			Some(p)	=> fmt!("{} {}", p, e.body.plain()),
			None	=> e.body.plain(),
		}).collect();
		if want.len() != have.len() {
			failures.push(fmt!("{}: typst lists {} entries, austenite {}", name, want.len(), have.len()));
		}
		for (w, h) in want.iter().zip(have.iter()) {
			compared += 1;
			if norm(w) != norm(h) {
				failures.push(fmt!("{} entry:\n  typst     {}\n  austenite {}", name, w, h));
			}
		}
	}
	println!("bibliography lists: {} entries compared, {} differ", compared, failures.len());
	assert!(failures.is_empty(), "{}", failures.join("\n"));
	assert!(compared >= 20, "only {} entries were compared", compared);
	Ok(())
}
