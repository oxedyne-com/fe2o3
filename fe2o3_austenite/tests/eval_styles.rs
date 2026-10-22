//! U4: style folding, selectors and realisation, each checked against the `typst` 0.15 oracle run on the
//! synthetic fixtures in `tests/fixtures/eval/styles/`. Until the evaluator (U2) lands, the Austenite side
//! of each case is built directly as content and styles mirroring its fixture. A missing `typst` fails
//! the suite unless `EVAL_ORACLE_SKIP=1` is set.

use oxedyne_fe2o3_austenite::eval::args::Args;
use oxedyne_fe2o3_austenite::eval::content::{
	Content,
	ElemKind,
	FieldDefault,
	FieldId,
	FieldSpec,
	FieldType,
	Fold,
};
use oxedyne_fe2o3_austenite::eval::func::{
	Func,
	NativeFunc,
};
use oxedyne_fe2o3_austenite::eval::lib::calc::CalcFn;
use oxedyne_fe2o3_austenite::eval::realise::{
	realise,
	Pair,
	RealiseMode,
	Tag,
};
use oxedyne_fe2o3_austenite::eval::select::{
	self,
	Selector,
	StyleFn,
};
use oxedyne_fe2o3_austenite::eval::styles::{
	fold_all,
	set_rule,
	Property,
	Recipe,
	RecipeIndex,
	Style,
	StyleChain,
	Styles,
	Transformation,
};
use oxedyne_fe2o3_austenite::eval::value::{
	Color,
	ColorSpace,
	Dash,
	DashItem,
	Dict,
	Label,
	Length,
	Paint,
	RegexValue,
	Stroke,
	Type,
	Value,
};
use oxedyne_fe2o3_austenite::eval::{
	eval_source,
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::syntax::{
	FileId,
	Span,
};

use std::path::{
	Path,
	PathBuf,
};
use std::process::Command;
use std::sync::Arc;

// Oracle plumbing

fn fixture(name: &str) -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval/styles").join(name)
}

fn skip() -> bool { std::env::var("EVAL_ORACLE_SKIP").map(|v| v == "1").unwrap_or(false) }

fn typst(args: &[&str]) -> Option<(String, String)> {
	match Command::new("typst").args(args).output() {
		Ok(o) => Some((String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned())),
		Err(e) => {
			assert!(skip(), "the typst oracle could not be run ({}); set EVAL_ORACLE_SKIP=1 to skip", e);
			None
		}
	}
}

/// The fixture's `<p>` probes, as the JSON array of strings (or numbers) `typst eval` prints.
fn probes(name: &str) -> Option<Vec<String>> {
	let path = fixture(name);
	let p = path.to_string_lossy().into_owned();
	let (out, err) = match typst(&["eval", "query(<p>).map(it => it.value)", "--in", &p]) {
		Some(r)	=> r,
		None	=> return None,
	};
	assert!(out.trim_start().starts_with('['), "oracle failed on {}: {}", name, err);
	Some(json_array(out.trim()))
}

/// A flat JSON array of strings and numbers.
fn json_array(s: &str) -> Vec<String> {
	let mut out = Vec::new();
	let chars: Vec<char> = s.chars().collect();
	let mut i = 1;
	while i < chars.len() {
		match chars[i] {
			'"' => {
				let mut v = String::new();
				i += 1;
				while i < chars.len() && chars[i] != '"' {
					if chars[i] == '\\' && i + 1 < chars.len() {
						i += 1;
						v.push(match chars[i] { 'n' => '\n', 't' => '\t', c => c });
					} else {
						v.push(chars[i]);
					}
					i += 1;
				}
				out.push(v);
				i += 1;
			}
			c if c == '-' || c.is_ascii_digit() => {
				let mut v = String::new();
				while i < chars.len() && (chars[i] == '-' || chars[i] == '.' || chars[i].is_ascii_digit()) {
					v.push(chars[i]);
					i += 1;
				}
				out.push(v);
			}
			_ => i += 1,
		}
	}
	out
}

/// The body text of the fixture's HTML export, tags stripped and `<br>` as a newline.
fn html_text(name: &str) -> Option<String> {
	let path = fixture(name);
	let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(fmt_name(name, "html"));
	let (_, err) = match typst(&["compile", "--features", "html", "--format", "html",
		&path.to_string_lossy(), &out.to_string_lossy()]) {
		Some(r)	=> r,
		None	=> return None,
	};
	let html = std::fs::read_to_string(&out).unwrap_or_else(|_| panic!("oracle failed on {}: {}", name, err));
	let body = html.split("<body>").nth(1).and_then(|b| b.split("</body>").next()).unwrap_or("");
	let body = body.replace("<br>", "\n");
	let mut text = String::new();
	let mut in_tag = false;
	for c in body.chars() {
		match c {
			'<'				=> in_tag = true,
			'>'				=> in_tag = false,
			_ if !in_tag	=> text.push(c),
			_				=> (),
		}
	}
	Some(text)
}

fn fmt_name(name: &str, ext: &str) -> String { format!("{}.{}", name.trim_end_matches(".typ"), ext) }

/// The first error of compiling the fixture, and its hints.
fn oracle_error(name: &str) -> Option<(String, Vec<String>)> {
	let path = fixture(name);
	let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(fmt_name(name, "pdf"));
	let (_, err) = match typst(&["compile", &path.to_string_lossy(), &out.to_string_lossy()]) {
		Some(r)	=> r,
		None	=> return None,
	};
	let mut lines = err.lines();
	let msg = lines.find_map(|l| l.strip_prefix("error: ")).unwrap_or_else(|| panic!("no oracle error for {}", name));
	let hints = err.lines().filter_map(|l| l.trim().strip_prefix("= hint: ")).map(|s| s.to_string()).collect();
	Some((msg.to_string(), hints))
}

// Rendering Austenite values as Typst's `repr` for the kinds the fold cases produce, then normalising
// both sides: whitespace and `0% + ` (Typst's relative wrapper) dropped, dictionary entries sorted.

fn num(x: f64) -> String {
	let r = (x * 100.0).round() / 100.0;
	let s = format!("{:.2}", r);
	s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn render(v: &Value) -> String {
	match v {
		Value::None			=> "none".into(),
		Value::Auto			=> "auto".into(),
		Value::Bool(b)		=> b.to_string(),
		Value::Int(i)		=> i.to_string(),
		Value::Float(f)		=> num(*f),
		Value::Str(s)		=> format!("\"{}\"", s),
		Value::Length(l)	=> match (l.abs, l.em) {
			(a, e) if e == 0.0	=> format!("{}pt", num(a)),
			(a, e) if a == 0.0	=> format!("{}em", num(e)),
			(a, e)				=> format!("{}pt + {}em", num(a), num(e)),
		},
		Value::Color(c)		=> color(c),
		Value::Stroke(s)	=> stroke(s),
		Value::Dict(d)		=> {
			if d.is_empty() {
				return "(:)".into();
			}
			let parts: Vec<String> = d.iter().map(|(k, v)| format!("{}: {}", k, render(v))).collect();
			format!("({})", parts.join(", "))
		}
		other => format!("<{}>", other.ty().name()),
	}
}

fn color(c: &Color) -> String {
	let h = |x: f32| format!("{:02x}", (x * 255.0).round() as u8);
	format!("rgb(\"#{}{}{}\")", h(c.c[0]), h(c.c[1]), h(c.c[2]))
}

fn stroke(s: &Stroke) -> String {
	let paint = s.paint.as_ref().map(|p| match p {
		Paint::Color(c)	=> color(c),
		_				=> "<paint>".to_string(),
	});
	let thick = s.thickness.map(|t| render(&Value::Length(t)));
	if s.cap.is_none() && s.join.is_none() && s.dash.is_none() && s.miter_limit.is_none() {
		match (&thick, &paint) {
			(Some(t), Some(p))	=> return format!("{} + {}", t, p),
			(Some(t), None)		=> return t.clone(),
			(None, Some(p))		=> return p.clone(),
			(None, None)		=> (),
		}
	}
	let mut parts = Vec::new();
	if let Some(p) = paint { parts.push(format!("paint: {}", p)); }
	if let Some(t) = thick { parts.push(format!("thickness: {}", t)); }
	if let Some(c) = s.cap { parts.push(format!("cap: \"{}\"", format!("{:?}", c).to_lowercase())); }
	if let Some(j) = s.join { parts.push(format!("join: \"{}\"", format!("{:?}", j).to_lowercase())); }
	if let Some(d) = &s.dash {
		parts.push(format!("dash: {}", match d {
			None	=> "none".to_string(),
			Some(d)	=> dash(d),
		}));
	}
	if let Some(m) = s.miter_limit { parts.push(format!("miter-limit: {}", num(m))); }
	format!("({})", parts.join(", "))
}

fn dash(d: &Dash) -> String {
	let items: Vec<String> = d.array.iter().map(|i| match i {
		DashItem::Dot		=> "\"dot\"".to_string(),
		DashItem::Len(l)	=> render(&Value::Length(*l)),
	}).collect();
	format!("(array: ({}), phase: {})", items.join(", "), render(&Value::Length(d.phase)))
}

fn normalise(s: &str) -> String {
	let mut out = String::new();
	let mut quoted = false;
	for c in s.chars() {
		if c == '"' {
			quoted = !quoted;
		}
		if quoted || !c.is_whitespace() {
			out.push(c);
		}
	}
	let out = out.replace("0%+", "").replace(",)", ")");
	sort_dict(&out)
}

/// Sorts a dictionary's top-level entries, so key order (which differs between Typst's `Sides` and
/// `Margin` reprs) does not matter.
fn sort_dict(s: &str) -> String {
	if !(s.starts_with('(') && s.ends_with(')')) || s == "(:)" {
		return s.to_string();
	}
	let inner = &s[1..s.len() - 1];
	let mut parts = Vec::new();
	let (mut depth, mut quoted, mut cur) = (0, false, String::new());
	for c in inner.chars() {
		match c {
			'"'						=> quoted = !quoted,
			'(' if !quoted			=> depth += 1,
			')' if !quoted			=> depth -= 1,
			',' if !quoted && depth == 0 => {
				parts.push(std::mem::take(&mut cur));
				continue;
			}
			_ => (),
		}
		cur.push(c);
	}
	parts.push(cur);
	if !parts.iter().all(|p| p.contains(':')) {
		return s.to_string();
	}
	parts.sort();
	format!("({})", parts.join(","))
}

// Values the fixtures use

fn rgb(hex: u32) -> Value {
	let c = |s: u32| ((hex >> s) & 0xff) as f32 / 255.0;
	Value::Color(Color { space: ColorSpace::Rgb, c: [c(16), c(8), c(0), 0.0], alpha: 1.0 })
}

fn red() -> Value { rgb(0xff4136) }
fn green() -> Value { rgb(0x2ecc40) }
fn blue() -> Value { rgb(0x0074d9) }
fn pt(x: f64) -> Value { Value::Length(Length::pt(x)) }
fn em(x: f64) -> Value { Value::Length(Length::em(x)) }
fn cm(x: f64) -> Value { pt(x * 72.0 / 2.54) }

fn dict(entries: &[(&str, Value)]) -> Value {
	let mut d = Dict::new();
	for (k, v) in entries {
		d.insert(k, v.clone());
	}
	Value::dict(d)
}

// Typst's schema for the fields the fold cases set: name, default and fold rule.
const TEXT_SIZE:		FieldSpec = FieldSpec::named("size", FieldType::Of(Type::Length), FieldDefault::Pt(11.0)).fold(Fold::Add);
const TEXT_FEATURES:	FieldSpec = FieldSpec::named("features", FieldType::Any, FieldDefault::Computed).fold(Fold::Add);
const LINE_STROKE:		FieldSpec = FieldSpec::named("stroke", FieldType::Any, FieldDefault::Computed).fold(Fold::Stroke);
const RECT_INSET:		FieldSpec = FieldSpec::named("inset", FieldType::Any, FieldDefault::Pt(5.0)).fold(Fold::Sides);
const RECT_RADIUS:		FieldSpec = FieldSpec::named("radius", FieldType::Any, FieldDefault::Computed).fold(Fold::Corners);
const RECT_STROKE:		FieldSpec = FieldSpec::named("stroke", FieldType::Any, FieldDefault::Computed).fold(Fold::Sides);
const RECT_FILL:		FieldSpec = FieldSpec::named("fill", FieldType::Any, FieldDefault::None);
const BLOCK_STROKE:		FieldSpec = FieldSpec::named("stroke", FieldType::Any, FieldDefault::None).fold(Fold::Sides);
const PAR_INDENT:		FieldSpec = FieldSpec::named("first-line-indent", FieldType::Any, FieldDefault::Computed).fold(Fold::Keyed("amount"));
const PAGE_MARGIN:		FieldSpec = FieldSpec::named("margin", FieldType::Any, FieldDefault::Auto).fold(Fold::Sides);

/// Sets `values` outer to inner and folds them as the chain would.
fn folded(spec: &FieldSpec, outer_to_inner: &[Value]) -> Value {
	let inner_first: Vec<&Value> = outer_to_inner.iter().rev().collect();
	fold_all(spec, &inner_first).unwrap_or(Value::None)
}

#[test]
fn folding_matches_the_oracle() {
	let expected = match probes("fold.typ") {
		Some(p)	=> p,
		None	=> return,
	};
	let dashed = dict(&[("dash", Value::str("dashed"))]);
	let ours = vec![
		folded(&TEXT_SIZE, &[em(2.0), em(1.5)]),
		folded(&TEXT_SIZE, &[Value::Length(Length { abs: 10.0, em: 1.0 })]),
		folded(&LINE_STROKE, &[red(), pt(2.0)]),
		folded(&RECT_INSET, &[pt(3.0), dict(&[("left", pt(1.0))])]),
		folded(&RECT_INSET, &[dict(&[("x", pt(3.0))]), dict(&[("left", pt(1.0))])]),
		folded(&RECT_RADIUS, &[dict(&[("top", pt(3.0))]), dict(&[("left", pt(1.0))])]),
		folded(&RECT_RADIUS, &[pt(2.0), dict(&[("top-left", pt(1.0))])]),
		folded(&RECT_STROKE, &[dict(&[("left", red())]), pt(2.0)]),
		folded(&RECT_STROKE, &[red(), dict(&[("left", pt(2.0))])]),
		folded(&RECT_STROKE, &[dict(&[("left", red()), ("rest", blue())]),
			dict(&[("paint", green()), ("thickness", pt(3.0))])]),
		folded(&BLOCK_STROKE, &[dict(&[("left", red())])]),
		folded(&PAR_INDENT, &[em(1.0), dict(&[("all", Value::Bool(true))])]),
		folded(&PAR_INDENT, &[dict(&[("amount", em(1.0)), ("all", Value::Bool(true))]), em(2.0)]),
		folded(&RECT_INSET, &[dict(&[("rest", pt(3.0))]), dict(&[("left", pt(3.0))])]),
		folded(&LINE_STROKE, &[dict(&[("paint", red()), ("dash", Value::str("dashed"))]), dict(&[("dash", Value::None)])]),
		folded(&LINE_STROKE, &[red(), dashed]),
		folded(&PAGE_MARGIN, &[cm(1.0), dict(&[("left", cm(2.0))])]),
		folded(&RECT_FILL, &[red(), Value::None]),
		folded(&TEXT_FEATURES, &[dict(&[("smcp", Value::Int(1))]), dict(&[("onum", Value::Int(1))])]),
		folded(&LINE_STROKE, &[dict(&[("dash", Value::str("dotted")), ("cap", Value::str("round"))]),
			dict(&[("join", Value::str("bevel")), ("miter-limit", Value::Float(2.5))])]),
	];
	assert_eq!(ours.len(), expected.len(), "one Austenite case per oracle probe");
	for (i, (o, e)) in ours.iter().zip(expected.iter()).enumerate() {
		assert_eq!(normalise(&render(o)), normalise(e), "fold case {} (oracle {:?})", i + 1, e);
	}
}

#[test]
fn chain_get_folds_and_resolve_folds_the_elements_own_value() {
	// `text` has only its string in its schema until U6a, so the chain is exercised through a field
	// with no schema entry (innermost wins) and the fold path through `fold_all` above.
	let id = FieldId(7);
	let a = Styles::from_style(Style::Property(Property::new(ElemKind::Text, id, pt(1.0), Span::detached())));
	let b = Styles::from_style(Style::Property(Property::new(ElemKind::Text, id, pt(2.0), Span::detached())));
	let chain = StyleChain::root().chain(&a).chain(&b);
	assert!(matches!(chain.get(ElemKind::Text, id), Ok(Some(Value::Length(l))) if l.abs == 2.0));
	assert_eq!(chain.values(ElemKind::Text, id).len(), 2);
	assert_eq!(chain.font_size(), 11.0);
}

// The caches each link keeps, against the plain walk over all of a chain's styles

/// The recipes in force by the plain walk, innermost first, the way `recipes` computed them before it was
/// cached per link.
fn walked_recipes(chain: &StyleChain) -> Vec<(RecipeIndex, Arc<Recipe>)> {
	let all: Vec<&Style> = chain.walk().collect();
	let total = all.iter().filter(|s| matches!(s, Style::Recipe(_))).count();
	let (mut revoked, mut out, mut r) = (Vec::new(), Vec::new(), 0);
	for s in all {
		match s {
			Style::Recipe(recipe) => {
				let index = RecipeIndex(total - r);
				r += 1;
				if !revoked.contains(&index) {
					out.push((index, recipe.clone()));
				}
			}
			Style::Revocation(i)	=> revoked.push(*i),
			Style::Property(_)		=> (),
		}
	}
	out
}

fn walked_values(chain: &StyleChain, kind: ElemKind, field: FieldId) -> Vec<&Value> {
	chain.walk().filter_map(|s| match s {
		Style::Property(p) if p.elem == kind && p.field == field	=> Some(&p.value),
		_															=> None,
	}).collect()
}

fn check_caches(chain: &StyleChain, keys: &[(ElemKind, FieldId)]) {
	let want = walked_recipes(chain);
	let got = chain.recipes();
	assert_eq!(got.len(), want.len(), "recipes in force");
	for (g, (index, recipe)) in got.iter().zip(want.iter()) {
		assert_eq!(g.index, *index);
		assert!(Arc::ptr_eq(&g.recipe, recipe), "recipe {:?}", index);
	}
	assert!(std::ptr::eq(chain.recipes().as_ptr(), got.as_ptr()), "the list is built once");
	for (kind, field) in keys {
		let want = walked_values(chain, *kind, *field);
		let got = chain.values(*kind, *field);
		assert_eq!(got.len(), want.len(), "values of {:?}.{:?}", kind, field);
		for (g, w) in got.iter().zip(want.iter()) {
			assert!(std::ptr::eq(*g, *w), "value of {:?}.{:?}", kind, field);
		}
	}
}

#[test]
fn the_per_link_caches_agree_with_the_plain_walk_on_branching_chains() {
	// 192 keys against the filter's 128 slots, so some keys must share a slot.
	let keys: Vec<(ElemKind, FieldId)> = ElemKind::ALL.iter().take(48)
		.flat_map(|k| (0..4u8).map(move |f| (*k, FieldId(f)))).collect();
	let mut seed = 0x2545_F491_4F6C_DD1Du64;
	let mut next = move |n: usize| -> usize {
		seed ^= seed << 13;
		seed ^= seed >> 7;
		seed ^= seed << 17;
		(seed % n as u64) as usize
	};
	let mut chains = vec![StyleChain::root()];
	let mut made = 0;
	for _ in 0..120 {
		// Chains branch from any earlier one, so siblings share a parent whose lists are already built.
		let parent = chains[next(chains.len())].clone();
		check_caches(&parent, &keys);
		let mut styles = Styles::new();
		for _ in 0..next(5) {
			made += 1;
			let style = match next(4) {
				0	=> show_text(&format!("r{}", made), "x"),
				1	=> Style::Revocation(RecipeIndex(1 + next(8))),
				_	=> {
					let (kind, field) = keys[next(keys.len())];
					Style::Property(Property::new(kind, field, pt(made as f64), Span::detached()))
				}
			};
			styles.push(style);
		}
		chains.push(parent.chain(&styles));
	}
	// A descendant made after an ancestor was read must not have changed what the ancestor says.
	for c in &chains {
		check_caches(c, &keys);
	}
}

#[test]
fn a_link_without_recipes_or_revocations_shares_its_parents_recipe_list() {
	let ruled = StyleChain::root().chain(&Styles::from_style(show_text("a", "b")));
	let plain = ruled.chain(&set_prop(ElemKind::Text, 1, pt(3.0)));
	let deeper = plain.chain(&set_prop(ElemKind::Text, 2, pt(4.0)));
	assert_eq!(ruled.recipes().len(), 1);
	assert!(std::ptr::eq(ruled.recipes().as_ptr(), plain.recipes().as_ptr()));
	assert!(std::ptr::eq(ruled.recipes().as_ptr(), deeper.recipes().as_ptr()));
	// A recipe in a branch leaves the shared list alone.
	let branch = plain.chain(&Styles::from_style(show_text("c", "d")));
	assert_eq!(branch.recipes().len(), 2);
	assert_eq!(branch.recipes()[0].index, RecipeIndex(2));
	assert_eq!(deeper.recipes().len(), 1);
}

// Realisation through text and regex rules, in paragraph (inline) mode

fn sp() -> Content { Content::marker(ElemKind::Space, Span::detached()) }
fn t(s: &str) -> Content { Content::text(s) }

fn show_text(sel: &str, out: &str) -> Style {
	Style::Recipe(Arc::new(Recipe {
		selector:	Some(Selector::Text(sel.into())),
		transform:	Transformation::Content(t(out)),
		span:		Span::detached(),
		outside:	false,
	}))
}

fn show_regex(pat: &str, out: &str) -> Style {
	let re = RegexValue::new(pat).unwrap_or_else(|e| panic!("regex {}: {:?}", pat, e));
	Style::Recipe(Arc::new(Recipe {
		selector:	Some(Selector::Regex(Arc::new(re))),
		transform:	Transformation::Content(t(out)),
		span:		Span::detached(),
		outside:	false,
	}))
}

fn engine() -> Engine { Engine::new(World::new(PathBuf::from("/"))) }

/// The realised stream as text, tags skipped.
fn plain(pairs: &[Pair]) -> String {
	pairs.iter().filter(|p| p.tag.is_none()).map(|p| match p.content.kind() {
		Some(ElemKind::Space)		=> " ".to_string(),
		Some(ElemKind::Linebreak)	=> "\n".to_string(),
		_							=> p.content.plain_text(),
	}).collect()
}

/// Realises `body` under `styles` (outermost first, one `show` each) as a paragraph's children.
fn inline(body: Vec<Content>, styles: Vec<Style>) -> String {
	let mut chain = StyleChain::root();
	for s in styles {
		chain = chain.chain(&Styles::from_style(s));
	}
	let mut e = engine();
	let pairs = realise(&mut e, &Content::sequence(body), &chain, RealiseMode::Inline)
		.unwrap_or_else(|err| panic!("realise: {:?} {:?}", err, e.diags));
	plain(&pairs)
}

fn check_html(name: &str, ours: String) {
	if let Some(expected) = html_text(name) {
		assert_eq!(ours, expected, "{}", name);
	}
}

#[test]
fn text_and_regex_rules_match_the_oracle() {
	check_html("text_rules.typ", inline(
		vec![t("ab"), sp(), t("c"), sp(), t("ab"), sp(), t("abd")],
		vec![show_text("ab", "X"), show_regex("b c", "Y")]));
	check_html("text_spaces.typ", inline(
		vec![t("a"), sp(), t("b"), sp(), t("a"), sp(), t("b")],
		vec![show_text("a b", "Z")]));
	check_html("text_inner_wins.typ", inline(vec![t("a")], vec![show_text("a", "1"), show_text("a", "2")]));
	check_html("text_leftmost.typ", inline(vec![t("abc")], vec![show_text("bc", "1"), show_text("ab", "2")]));
	check_html("text_revoked.typ", inline(vec![t("a")], vec![show_text("a", "aa")]));
	check_html("text_linebreak.typ", inline(
		vec![t("a"), sp(), Content::marker(ElemKind::Linebreak, Span::detached()), sp(), t("b")],
		vec![show_text("a\nb", "X")]));
}

#[test]
fn a_style_change_stops_a_text_rule() {
	// `a#text(fill: red)[b]`: the property is a stand-in for `fill`, which U6a's schema will name.
	let fill = Styles::from_style(Style::Property(Property::new(ElemKind::Text, FieldId(1), red(), Span::detached())));
	check_html("text_style_break.typ", inline(vec![t("a"), t("b").styled(fill)], vec![show_text("ab", "X")]));
}

#[test]
fn a_selectorless_show_replaces_the_rest_of_its_scope() {
	let rest = Content::sequence(vec![t("b"), sp(), t("c")]);
	let recipe = Style::Recipe(Arc::new(Recipe { selector: None, transform: Transformation::Content(t("X")), span: Span::detached(), outside: false, }));
	check_html("show_none.typ", inline(vec![t("a"), sp(), rest.styled(Styles::from_style(recipe))], vec![]));
}

#[test]
fn an_inline_only_fragment_is_not_made_a_paragraph() {
	// Typst makes no `par` for `#block[hello world]`: the fixture counts the paragraphs `show par` sees.
	let expected = match probes("fragment_inline.typ") {
		Some(p)	=> p,
		None	=> return,
	};
	let mut e = engine();
	let pairs = realise(&mut e, &Content::sequence(vec![t("hello"), sp(), t("world")]), &StyleChain::root(), RealiseMode::Flow)
		.unwrap_or_else(|err| panic!("{:?}", err));
	let pars = pairs.iter().filter(|p| p.content.is(ElemKind::Par)).count();
	assert_eq!(pars.to_string(), expected[0]);
	assert_eq!(plain(&pairs), "hello world");
}

// Diagnostics, message for message

fn check_error(name: &str, ours: Result<(), String>, diags: &[oxedyne_fe2o3_austenite::diag::Diagnostic]) {
	let (msg, hints) = match oracle_error(name) {
		Some(e)	=> e,
		None	=> return,
	};
	assert!(ours.is_err(), "{}: Austenite accepted what Typst refuses ({})", name, msg);
	let d = diags.iter().find(|d| d.is_error()).unwrap_or_else(|| panic!("{}: no diagnostic recorded", name));
	assert_eq!(d.message, msg, "{}", name);
	assert_eq!(d.hints, hints, "{} hints", name);
}

fn realise_err(content: Content, mode: RealiseMode) -> (Result<(), String>, Engine) {
	let mut e = engine();
	let r = realise(&mut e, &content, &StyleChain::root(), mode).map(|_| ()).map_err(|x| format!("{:?}", x));
	(r, e)
}

#[test]
fn realisation_errors_match_the_oracle() {
	let l = Label::new("l");
	let meta = || Content::marker(ElemKind::Metadata, Span::detached()).labelled(l.clone());
	let recipe = Style::Recipe(Arc::new(Recipe {
		selector:	Some(Selector::Label(l.clone())),
		transform:	Transformation::Content(meta()),
		span:		Span::detached(),
		outside:	false,
	}));
	let (r, e) = realise_err(meta().styled(Styles::from_style(recipe)), RealiseMode::Document);
	check_error("err_depth.typ", r, &e.diags);

	let page = Styles::from_style(Style::Property(Property::new(ElemKind::Page, FieldId(0), pt(85.0), Span::detached())));
	let (r, e) = realise_err(Content::empty().styled(page), RealiseMode::Flow);
	check_error("err_page.typ", r, &e.diags);

	let doc = Styles::from_style(Style::Property(Property::new(ElemKind::Document, FieldId(0), Value::str("x"), Span::detached())));
	let (r, e) = realise_err(Content::empty().styled(doc), RealiseMode::Flow);
	check_error("err_document.typ", r, &e.diags);
}

#[test]
fn set_rule_errors_match_the_oracle() {
	let mut e = engine();
	let mut args = Args::new(Span::detached());
	args.push_named(Span::detached(), "text", Value::str("a"));
	let r = set_rule(&mut e, ElemKind::Text, args).map(|_| ()).map_err(|x| format!("{:?}", x));
	check_error("err_set_text.typ", r, &e.diags);
}

fn native() -> Value { Value::Func(Func::Native(NativeFunc::Calc(CalcFn::Abs))) }

fn stringly<T>(r: oxedyne_fe2o3_core::prelude::Outcome<T>) -> Result<T, String> { r.map_err(|x| format!("{:?}", x)) }

/// Runs one selector case on a fresh engine and compares its diagnostic with the fixture's.
fn selector_case<F: FnOnce(&mut Engine) -> Result<(), String>>(name: &str, f: F) {
	let mut e = engine();
	let r = f(&mut e);
	check_error(name, r, &e.diags);
}

fn call_with(e: &mut Engine, f: StyleFn, pos: Vec<Value>, named: Vec<(&str, Value)>) -> Result<Value, String> {
	let sp = Span::detached();
	let mut a = Args::new(sp);
	for v in pos {
		a.push(sp, v);
	}
	for (n, v) in named {
		a.push_named(sp, n, v);
	}
	stringly(select::call(f, e, a))
}

#[test]
fn selector_casts_match_the_oracle() {
	let sp = Span::detached();
	let heading = || Value::Func(Func::Element(ElemKind::Heading));
	selector_case("err_show_int.typ", |e| stringly(select::cast_showable(e, sp, Value::Int(1))).map(|_| ()));
	selector_case("err_show_empty.typ", |e| stringly(select::cast_showable(e, sp, Value::str(""))).map(|_| ()));
	selector_case("err_func.typ", |e| stringly(select::cast_showable(e, sp, native())).map(|_| ()));
	selector_case("err_show_before.typ", |e| {
		match call_with(e, StyleFn::Before, vec![heading(), Value::Label(Label::new("a"))], vec![]) {
			Ok(v)	=> stringly(select::cast_showable(e, sp, v)).map(|_| ()),
			Err(x)	=> Err(x),
		}
	});
	selector_case("err_or_int.typ", |e| call_with(e, StyleFn::Or, vec![heading(), Value::Int(1)], vec![]).map(|_| ()));
	selector_case("err_where_field.typ", |e| call_with(e, StyleFn::Where, vec![heading()], vec![("foo", Value::Int(1))]).map(|_| ()));
	selector_case("err_where_func.typ", |e| call_with(e, StyleFn::Where, vec![native()], vec![("a", Value::Int(1))]).map(|_| ()));
}

#[test]
fn selector_matching_matches_the_oracle() {
	let expected = match probes("matches.typ") {
		Some(p)	=> p,
		None	=> return,
	};
	let (a, b) = (Label::new("a"), Label::new("b"));
	let m = |l: Option<&Label>| {
		let c = Content::marker(ElemKind::Metadata, Span::detached());
		match l {
			Some(l)	=> c.labelled(l.clone()),
			None	=> c,
		}
	};
	let elems = vec![m(Some(&a)), m(Some(&b)), m(None)];
	let sels = vec![
		Selector::Or(vec![Selector::Label(a.clone()), Selector::Label(b.clone())]),
		Selector::And(vec![Selector::Label(a.clone()), Selector::Elem(ElemKind::Metadata, None)]),
		Selector::And(vec![Selector::Label(a.clone()), Selector::Label(b.clone())]),
	];
	for (i, s) in sels.iter().enumerate() {
		let n = elems.iter().filter(|c| s.matches(c, None).unwrap_or(false)).count();
		assert_eq!(n.to_string(), expected[i], "selector case {}", i + 1);
	}
}

// Mechanisms the oracle shows only through closures and introspection, which need U2 and U8; the
// expected behaviour each asserts is noted from the oracle.

fn recipe(sel: Selector, transform: Transformation) -> Styles {
	Styles::from_style(Style::Recipe(Arc::new(Recipe { selector: Some(sel), transform, span: Span::detached(), outside: false, })))
}

fn set_prop(kind: ElemKind, field: u8, v: Value) -> Styles {
	Styles::from_style(Style::Property(Property::new(kind, FieldId(field), v, Span::detached())))
}

#[test]
fn show_set_rules_collect_whichever_side_of_the_step_they_sit() {
	// Oracle: `show heading: set text(red)` reaches a `show heading: it => ..` body whether it was
	// written before or after it. Here the step is a content replacement, whose output carries the
	// show-set property; the innermost show-set wins.
	let l = Label::new("h");
	let target = Content::marker(ElemKind::Metadata, Span::new(FileId(0), 0, 5)).labelled(l.clone());
	for order in 0..2 {
		let set_red = recipe(Selector::Label(l.clone()), Transformation::Style(set_prop(ElemKind::Text, 9, red())));
		let set_blue = recipe(Selector::Label(l.clone()), Transformation::Style(set_prop(ElemKind::Text, 9, blue())));
		let step = recipe(Selector::Label(l.clone()), Transformation::Content(t("X")));
		let chain = match order {
			0 => StyleChain::root().chain(&set_red).chain(&set_blue).chain(&step),
			_ => StyleChain::root().chain(&step).chain(&set_red).chain(&set_blue),
		};
		let mut e = engine();
		let pairs = realise(&mut e, &target, &chain, RealiseMode::Inline).unwrap_or_else(|x| panic!("{:?}", x));
		let text = pairs.iter().find(|p| p.content.is(ElemKind::Text)).unwrap_or_else(|| panic!("no text"));
		assert_eq!(plain(&pairs), "X");
		assert!(matches!(text.styles.get(ElemKind::Text, FieldId(9)), Ok(Some(Value::Color(c))) if c.c[2] > 0.8),
			"innermost show-set (blue) wins, order {}", order);
	}
}

#[test]
fn a_guarded_element_skips_its_recipe_and_locatable_elements_get_tags() {
	let span = Span::new(FileId(0), 3, 9);
	let l = Label::new("m");
	let step = recipe(Selector::Elem(ElemKind::Metadata, None), Transformation::Content(t("X")));
	let chain = StyleChain::root().chain(&step);
	// Unguarded: replaced, with start and end tags around the output carrying the prepared element.
	let mut e = engine();
	let meta = Content::marker(ElemKind::Metadata, span).labelled(l.clone());
	let pairs = realise(&mut e, &meta, &chain, RealiseMode::Inline).unwrap_or_else(|x| panic!("{:?}", x));
	assert_eq!(plain(&pairs), "X");
	let start = match &pairs[0].tag {
		Some(Tag::Start(c))	=> c.clone(),
		other				=> panic!("expected a start tag, found {:?}", other),
	};
	let loc = start.location().unwrap_or_else(|| panic!("the start tag's element has no location"));
	assert!(matches!(pairs.last().and_then(|p| p.tag.as_ref()), Some(Tag::End(x)) if *x == loc));
	assert_eq!(start.label(), Some(&l));
	// Guarded against the recipe (index 1, the only one): kept as is, a primitive pushed with its tags.
	let mut guarded = meta.clone();
	if let Content::Elem(el) = &mut guarded {
		el.edit().guards.push(oxedyne_fe2o3_austenite::eval::styles::RecipeIndex(1));
	}
	let mut e = engine();
	let pairs = realise(&mut e, &guarded, &chain, RealiseMode::Inline).unwrap_or_else(|x| panic!("{:?}", x));
	// The recipe is skipped, so the element's own show runs: a metadata element shows as nothing, its
	// start tag carrying it.
	assert_eq!(plain(&pairs), "");
	assert_eq!(pairs.iter().filter(|p| matches!(&p.tag, Some(Tag::Start(c)) if c.is(ElemKind::Metadata))).count(), 1);
	// Plain text is neither locatable nor labelled: no location, no tags.
	let mut e = engine();
	let pairs = realise(&mut e, &t("a"), &StyleChain::root(), RealiseMode::Inline).unwrap_or_else(|x| panic!("{:?}", x));
	assert!(pairs.iter().all(|p| p.tag.is_none() && p.content.location().is_none()));
}

#[test]
fn page_styles_in_the_document_break_pages_around_them() {
	let page = set_prop(ElemKind::Page, 0, pt(85.0));
	let body = Content::sequence(vec![
		Content::marker(ElemKind::Metadata, Span::detached()),
		Content::marker(ElemKind::Metadata, Span::detached()).styled(page),
	]);
	let mut e = engine();
	let pairs = realise(&mut e, &body, &StyleChain::root(), RealiseMode::Document).unwrap_or_else(|x| panic!("{:?}", x));
	// A metadata element shows as nothing, leaving its two tags; the break before the styled one is the
	// weak break its page style makes, and the break after is the boundary.
	let seq: Vec<String> = pairs.iter().map(|p| match (&p.tag, p.content.kind()) {
		(Some(Tag::Start(c)), _)	=> format!("start {}", c.kind().map(|k| k.name()).unwrap_or("?")),
		(Some(Tag::End(_)), _)		=> "end".to_string(),
		(None, Some(k))				=> k.name().to_string(),
		(None, None)				=> "?".to_string(),
	}).collect();
	assert_eq!(seq, ["start metadata", "end", "pagebreak", "start metadata", "end", "pagebreak"]);
	// The page property rides on the second metadata and the leading break, not the trailing one.
	let breaks: Vec<&Pair> = pairs.iter().filter(|p| p.content.is(ElemKind::Pagebreak)).collect();
	assert_eq!(breaks[0].styles.values(ElemKind::Page, FieldId(0)).len(), 1);
	assert_eq!(breaks[1].styles.values(ElemKind::Page, FieldId(0)).len(), 0);
}

#[test]
fn selector_functions_build_selectors() {
	let sp = Span::detached();
	let mut e = engine();
	let mut a = Args::new(sp);
	a.push(sp, Value::Label(Label::new("x")));
	let v = select::call(StyleFn::Selector, &mut e, a).unwrap_or_else(|x| panic!("{:?}", x));
	assert!(matches!(&v, Value::Selector(s) if matches!(**s, Selector::Label(_))));
	let mut a = Args::new(sp);
	a.push(sp, v);
	a.push(sp, Value::Func(Func::Element(ElemKind::Metadata)));
	a.push_named(sp, "inclusive", Value::Bool(false));
	let v = select::call(StyleFn::After, &mut e, a).unwrap_or_else(|x| panic!("{:?}", x));
	assert!(matches!(&v, Value::Selector(s) if matches!(**s, Selector::After { inclusive: false, .. })));
	assert_eq!(select::method("where"), Some(StyleFn::Where));
	assert_eq!(select::method("fields"), None);
}

// Labelled sequences, evaluated from source

/// The fixture evaluated by the parser and evaluator, then realised as a paragraph's children.
fn evaluated_inline(name: &str) -> String {
	let path = fixture(name);
	let root = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
	let mut world = World::new(root);
	let id = world.load(&path).unwrap_or_else(|e| panic!("load {}: {:?}", name, e));
	let mut e = Engine::new(world);
	let module = eval_source(&mut e, id).unwrap_or_else(|err| panic!("eval {}: {:?} {:?}", name, err, e.diags));
	let pairs = realise(&mut e, &module.content, &StyleChain::root(), RealiseMode::Inline)
		.unwrap_or_else(|err| panic!("realise {}: {:?} {:?}", name, err, e.diags));
	plain(&pairs).trim().to_string()
}

#[test]
fn a_label_rule_applies_once_to_each_labelled_sequence() {
	// Each labelled sequence is its own target, guarded against the rule's output: the outer one's
	// output does not hide the inner one from the rule, and neither is transformed twice.
	check_html("label_once.typ", evaluated_inline("label_once.typ"));
	check_html("label_nested.typ", evaluated_inline("label_nested.typ"));
}
