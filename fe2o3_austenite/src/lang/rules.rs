//! The styling rule engine: a `#show <selector>: <transform>` as data over the block tree.
//!
//! Typst styles individual elements with a show rule -- `#show heading.where(level: 1): set text(size:
//! 30pt)`. Austenite does not *execute* one (it has no style-computation layer, by design), but it can
//! *lower* a set-fields show rule the same way [`crate::lang::set`] lowers a top-level `#set`: read the
//! selector and the transform, and, where the transform is a plain field write the theme carries, apply
//! it to the matched element's subtree by wrapping the element in a [`Block::Scoped`] carrying the patch.
//! The scope machinery ([`Block::Scoped`], made scope-aware in every document-order pass) then confines
//! the styling to that one element and lifts it again after, so a rule on a level-1 heading styles only
//! the level-1 headings and no further.
//!
//! What is lowered here is deliberately narrow: a **set-fields** transform (`set <target>(...)`), on a
//! field the renderer actually reads. Two families are refused rather than run, so a rule never silently
//! misleads:
//!
//! - A transform whose body *reads the page* -- `context`, `query`, `counter.at`, `state`, `measure`,
//!   `layout` -- has no lowering (Austenite settles numbers in a document-order pre-pass, not a layout
//!   round-trip), so it is recorded as a refusal and never run.
//! - A transform patching a field the renderer does not read, or reads only cross-group -- `text.tracking`,
//!   `text.ligatures`, a body/heading `font` face, `heading` `smallcaps`, `figure.skip`, `code.background`,
//!   `equation.numbering`, any `page` dimension -- is refused too, so a rule that would quietly no-op is a
//!   visible "not yet supported" instead.
//!
//! A wrap-transform (a template with holes, `it => underline(it)`, `block.with(...)`) is a later part's
//! work; here such a transform is refused, not guessed at.
//!
//! Consuming is selector-aware. The one `text` field a heading renders is its size, so a
//! `#show heading.where(level: N): set text(size: ...)` is *not* refused: it is redirected into the matched
//! level's own `heading` size -- the field the renderer reads for a heading (`Theme::heading_size`) -- so
//! the rule resizes that level's headings and no others (an unpredicated `#show heading:` sizes every level
//! alike). A `text` field a heading never renders -- tracking, ligatures, a font face, a body hyphenation
//! switch -- is still refused under a heading selector, naming the field and the selector, so the invariant
//! holds: every lowered field is consumed by the renderer or refused, never written-and-ignored.
//!
//! Block identity (design note). A set-fields transform *preserves* the source element's identity -- the
//! [`Block::Scoped`] it produces is a derived wrap of the very block matched, not a new element -- so a
//! later part can address the styled element by the source element's own identity and the [`RuleId`] that
//! wrapped it. This part records those two hooks (the rule's index, and that the wrap is derived) and
//! computes no address or hash from them yet.

use crate::doc::Block;
use crate::ir::{
	FloatPlacement,
	Length,
	Sp,
	Span,
};
use crate::theme::{
	Theme,
	ThemeHeadingLevelPatch,
	ThemePatch,
};

use super::parse::Refusals;
use super::set;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::colour::Rgba;

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE RULE                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// The element kind a selector's base names. A `figure.caption` refines the figure kind to its caption;
/// the rest name a block the reader sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElementKind {
	Heading,
	Figure,
	FigureCaption,
	Raw,		// a verbatim code block (`raw`)
	Equation,
	Link,
	Paragraph,
	List,
	Table,
}

/// A field predicate a `.where(...)` narrows a selector by.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldPredicate {
	Level(u8),		// heading.where(level: N)
	Block(bool),	// raw.where(block: true/false)
}

/// A `#show` selector: an element kind and the field predicates that narrow it. An empty predicate list
/// matches every element of the kind.
#[derive(Clone, Debug, PartialEq)]
pub struct Selector {
	pub kind:		ElementKind,
	pub predicates:	Vec<FieldPredicate>,
}

/// What a rule does to a matched element: patch its subtree ([`Transform::SetFields`], Part 1), or place
/// sibling blocks around it and overlay a theme on the group ([`Transform::Template`], this part). A
/// page-reading or otherwise unsupported transform is a [`Transform::Refused`], recorded and never run.
#[derive(Clone, Debug)]
pub enum Transform {
	SetFields(ThemePatch),
	Template(Template),
	Refused(String),	// the reason, recorded as a refusal rather than applied
}

/// A template transform: the matched element is *moved* (not cloned) into a hole between sibling blocks the
/// template placed around it, and a theme overlay wraps the whole group. `pre`/`post` are the blocks a
/// `v(<len>)`/`line(...)` in the template lowers to, before and after the element. `hole` is the overlay a
/// `#set` inside the wrap contributes (and, under a heading selector, the level spacing a `v(...)` folds
/// into). `frame`, when set, seats the element in a washed [`Block::Box`] of that fill and, where the rule
/// names them, that inset and radius too -- a `block.with(fill:, inset:, radius:)` callout. `rule_id` and
/// the hole's index (always `pre.len()`) are recorded so a later pass can address the moved element by the
/// rule that placed it -- the block-identity hook the design note calls for.
#[derive(Clone, Debug)]
pub struct Template {
	pub pre:		Vec<Block>,
	pub hole:		ThemePatch,
	pub post:		Vec<Block>,
	pub frame:		Option<TemplateFrame>,
	pub rule_id:	RuleId,
}

/// The wash a `block.with(fill:, inset:, radius:)` template names -- the fill always, `inset_x`/
/// `inset_top`/`inset_bot`/`radius` only where the rule sets them. `None` on any of the four leaves the
/// renderer's own default (the `#styled-box` template's one body em, 1.2 body em and 4pt) untouched, so a
/// rule that names only `fill:` frames the element without moving its geometry at all.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TemplateFrame {
	pub fill:		Rgba,
	pub inset_x:	Option<Sp>,
	pub inset_top:	Option<Sp>,
	pub inset_bot:	Option<Sp>,
	pub radius:		Option<Sp>,
}

/// A rule's index in the set that produced it -- the identity hook a set-fields wrap carries so a later
/// part can address the styled element. No address or hash is computed from it here.
pub type RuleId = usize;

/// One styling rule: a selector, its transform, and the [`RuleId`] a derived wrap records. `source`
/// is the rule's own source name (with its leading `#`), for a refusal diagnostic.
#[derive(Clone, Debug)]
pub struct Rule {
	pub selector:	Selector,
	pub transform:	Transform,
	pub rule_id:	RuleId,
	pub source:		String,
	pub span:		Span,
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE DEFAULT RULE SET                                                       │
// └───────────────────────────────────────────────────────────────────────────┘

/// The default rule set applied ahead of a document's own rules: the styling the block layer would set
/// anyway, expressed as rules over the same block tree. This part migrates the heading styling that a
/// set-fields transform can carry -- one rule per heading level re-asserting that level's own size from
/// the theme -- so every heading flows through the rule engine and the corpus renders byte-identically
/// (the re-asserted value equals the theme's, and the scope wrap is transparent).
///
/// Caption sizing and the inline-equation digit shrink are *not* migrated here: the caption reads
/// `text.body_size` (not a `figure` field) and the inline digit shrink reads the running maths size, both
/// cross-group reads a per-element `figure`/`equation` set-fields rule cannot target without the renderer
/// change the readiness audit flagged. Their migration waits on that change (a wrap-transform part), so
/// they are left in the block layer and named here rather than expressed as a rule that would silently
/// no-op.
pub fn default_rule_set(theme: &Theme) -> Vec<Rule> {
	let mut rules = Vec::new();
	for (i, level) in theme.heading.levels.iter().enumerate() {
		let lvl = (i + 1) as u8;	// level index 0 is level 1
		// Re-assert this level's own size: a set-fields patch that names the size the theme already holds,
		// so applying it (by wrapping the heading in a scope) leaves the rendered size exactly as it was.
		let mut patch	= ThemePatch::default();
		let mut levels: Vec<ThemeHeadingLevelPatch> = Vec::with_capacity(i + 1);
		levels.resize_with(i + 1, Default::default);
		levels[i].size	= Some(level.size);
		patch.heading.levels = levels;
		rules.push(Rule {
			selector:	Selector { kind: ElementKind::Heading, predicates: vec![FieldPredicate::Level(lvl)] },
			transform:	Transform::SetFields(patch),
			rule_id:	rules.len(),
			source:		fmt!("#show heading.where(level: {})", lvl),
			span:		Span::new(0, 0),
		});
	}
	rules
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ COLLECTING A DOCUMENT'S OWN RULES FROM SOURCE                              │
// └───────────────────────────────────────────────────────────────────────────┘

/// Every `#show <selector>: <transform>` a source declares at top level, lowered to a [`Rule`]. A rule
/// whose transform reads the page, or patches a field the renderer does not read, is captured as a
/// [`Transform::Refused`] and its reason recorded in `refusals`, so it is a visible "not supported" rather
/// than a silent drop. A `#show:` with no selector (the whole-document `doc.with` application) is not a
/// per-element rule and is left for [`crate::lang::set`]. Rules are numbered from `base_id`, so a caller
/// appending them after the default set keeps every [`RuleId`] distinct.
pub fn collect_from_source(src: &str, base_id: RuleId, refusals: &mut Refusals) -> Vec<Rule> {
	let mut rules	= Vec::new();
	let mut offset	= 0usize;	// running byte offset of the current line's start
	for raw in src.split_inclusive('\n') {
		let line_start	= offset;
		offset			= offset.saturating_add(raw.len());
		let trimmed		= raw.trim_start();
		// A per-element show rule opens `#show <selector>:` -- a selector between `#show ` and the colon.
		// `#show:` (no selector) is the whole-document application, not ours.
		let after = match trimmed.strip_prefix("#show ") {
			Some(a)	=> a,
			None	=> continue,
		};
		let (sel_text, tr_text) = match split_at_top_level_colon(after) {
			Some(pair)	=> pair,
			None		=> continue,
		};
		let selector = match parse_selector(sel_text.trim()) {
			Some(s)	=> s,
			None	=> continue,	// an unrecognised selector is left to the reader's own refusal path
		};
		let span		= Span::new(line_start as u32, offset as u32);
		let source		= fmt!("#show {}", sel_text.trim());
		let mut transform	= lower_transform(&selector, tr_text.trim());
		if let Transform::Refused(reason) = &transform {
			refusals.record(&fmt!("{} ({})", source, reason), span);
		}
		let rule_id = base_id + rules.len();
		// A template records the rule that placed it, so the moved element keeps an addressable identity.
		if let Transform::Template(t) = &mut transform {
			t.rule_id = rule_id;
		}
		rules.push(Rule { selector, transform, rule_id, source, span });
	}
	rules
}

/// Does this already-left-trimmed line declare a per-element `#show <selector>: <transform>` rule the rule
/// engine collects and applies (or refuses in its own diagnostic)? True only when a selector between
/// `#show ` and a top-level colon parses to a known element kind -- the same recognition
/// [`collect_from_source`] uses -- so the reader can stop tallying such a line as a skipped construct and
/// leave it to the engine. A `#show:` doc application (no selector) and a `#show ...` whose selector no
/// element answers to are not rule lines.
pub fn is_rule_line(trimmed: &str) -> bool {
	let after = match trimmed.strip_prefix("#show ") {
		Some(a)	=> a,
		None	=> return false,
	};
	match split_at_top_level_colon(after) {
		Some((sel_text, _))	=> parse_selector(sel_text.trim()).is_some(),
		None				=> false,
	}
}

/// The `(selector, transform)` split of a `#show <selector>: <transform>` body at the first colon that is
/// not inside a `(...)`/`[...]`/`"..."` -- so the colon inside `where(level: 1)` is passed over and the
/// real separator found. `None` when there is no top-level colon.
fn split_at_top_level_colon(s: &str) -> Option<(&str, &str)> {
	let bytes		= s.as_bytes();
	let mut depth	= 0i32;
	let mut in_str	= false;
	let mut esc		= false;
	let mut i		= 0usize;
	while i < bytes.len() {
		let c = bytes[i];
		if in_str {
			if esc				{ esc = false; }
			else if c == b'\\'	{ esc = true; }
			else if c == b'"'	{ in_str = false; }
			i += 1;
			continue;
		}
		match c {
			b'"'					=> in_str = true,
			b'(' | b'[' | b'{'		=> depth += 1,
			b')' | b']' | b'}'		=> depth -= 1,
			b':' if depth == 0		=> return Some((&s[..i], &s[i + 1..])),
			_						=> {},
		}
		i += 1;
	}
	None
}

/// Parses a selector -- `heading.where(level: 1)`, `figure.caption`, `raw.where(block: true)`, `link` --
/// into an [`ElementKind`] and its field predicates. The base identifier before the first `.` names the
/// kind; a `.caption` refines a figure to its caption; a `.where(...)` reads the predicates. `None` for a
/// base identifier no element kind answers to.
fn parse_selector(s: &str) -> Option<Selector> {
	// The base runs up to the first `.` or `(`; the rest is a `.caption` refinement or a `.where(...)`.
	let base_end	= s.find(['.', '(']).unwrap_or(s.len());
	let base		= s[..base_end].trim();
	let rest		= s[base_end..].trim();

	let mut kind = match base {
		"heading"					=> ElementKind::Heading,
		"figure"					=> ElementKind::Figure,
		"raw"						=> ElementKind::Raw,
		"math.equation" | "equation"	=> ElementKind::Equation,
		"link"						=> ElementKind::Link,
		"par" | "parbreak"			=> ElementKind::Paragraph,
		"list" | "enum"				=> ElementKind::List,
		"table"						=> ElementKind::Table,
		_							=> return None,
	};

	let mut predicates = Vec::new();
	if let Some(after) = rest.strip_prefix(".caption") {
		if kind == ElementKind::Figure {
			kind = ElementKind::FigureCaption;
		}
		predicates.extend(parse_where(after.trim()));
	} else {
		predicates.extend(parse_where(rest));
	}
	Some(Selector { kind, predicates })
}

/// The predicates of a `.where(k: v, ...)` selector tail, or an empty list when there is none. Only the
/// two forms this part reads -- `level: N` and `block: bool` -- are lifted; an argument it does not know
/// is passed over rather than failing the whole selector.
fn parse_where(rest: &str) -> Vec<FieldPredicate> {
	let inner = match rest.strip_prefix(".where") {
		Some(a)	=> a.trim(),
		None	=> return Vec::new(),
	};
	let args = match (inner.strip_prefix('('), inner.strip_suffix(')')) {
		(Some(a), _)	=> a.trim_end_matches(')').trim(),
		_				=> return Vec::new(),
	};
	let mut out = Vec::new();
	for part in args.split(',') {
		let mut kv = part.splitn(2, ':');
		let key = kv.next().unwrap_or("").trim();
		let val = kv.next().unwrap_or("").trim();
		match key {
			"level"	=> if let Ok(n) = val.parse::<u8>() { out.push(FieldPredicate::Level(n)); },
			"block"	=> match val {
				"true"	=> out.push(FieldPredicate::Block(true)),
				"false"	=> out.push(FieldPredicate::Block(false)),
				_		=> {},
			},
			_		=> {},
		}
	}
	out
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ LOWERING A TRANSFORM                                                       │
// └───────────────────────────────────────────────────────────────────────────┘

/// The page-reading primitives whose presence in a transform body means it settles a number from the laid
/// page, which Austenite has no layer to run -- so such a transform is refused, never lowered.
const PAGE_READING_TOKENS: &[&str] = &["context", "query", "counter.at", "state", "measure", "layout"];

/// Lowers a transform body to a [`Transform`]. A `set <target>(...)` on a field the renderer reads lowers
/// to a [`Transform::SetFields`]; a body that reads the page, patches an unread or cross-group field, or is
/// a wrap-transform this part does not build, is a [`Transform::Refused`] carrying its reason.
fn lower_transform(selector: &Selector, body: &str) -> Transform {
	// A closure head (`it =>`, `x =>`) is stripped; the transform is what it evaluates to.
	let body = match body.split_once("=>") {
		Some((_head, tail))	=> tail.trim(),
		None				=> body,
	};

	// A page-reading body has no lowering at all.
	for tok in PAGE_READING_TOKENS {
		if body.contains(tok) {
			return Transform::Refused(fmt!("reads the page: {}", tok));
		}
	}

	// A `set <target>(<args>)` lowers to a set-fields patch below; a body that is not a bare `set` -- a
	// template with holes, a wrap -- is read by the template lowerer, which builds a [`Transform::Template`]
	// or refuses the body it cannot place.
	let after = match body.strip_prefix("set ") {
		Some(a)	=> a.trim(),
		None	=> return lower_template(selector, body),
	};
	let open = match after.find('(') {
		Some(i)	=> i,
		None	=> return Transform::Refused(fmt!("unsupported transform: {}", short(body))),
	};
	let target	= after[..open].trim();
	let args	= match after[open..].strip_prefix('(').and_then(|a| a.strip_suffix(')')) {
		Some(a)	=> a,
		None	=> return Transform::Refused(fmt!("unbalanced set(...) in transform: {}", short(body))),
	};

	// A field this selector's element does not read (or reads only cross-group) is refused rather than
	// applied, so a rule that would silently no-op is a visible "not yet supported". The judgement is
	// selector-aware: a `text` field a heading does not render is refused under a heading selector even
	// though the same field is read for body text.
	if let Some(reason) = unread_field_reason(selector, target, args) {
		return Transform::Refused(reason);
	}

	// A heading reads its glyph size from the `heading` group, not from `text.body_size` (which no heading
	// renders), so a `set text(size: ...)` on a heading selector is redirected into the matched level's own
	// size -- the field the renderer consumes -- affecting only that level (or every level, for an
	// unpredicated rule). Every other case lowers straight through `set::lower_set`.
	let patch = if selector.kind == ElementKind::Heading && target == "text" {
		match heading_text_patch(selector, args) {
			Some(p)	=> p,
			None	=> return Transform::Refused(fmt!(
				"set text on {} named no size or font the renderer can consume", selector_label(selector))),
		}
	} else {
		set::lower_set(target, args)
	};
	if patch == ThemePatch::default() {
		// The set named a target or argument the theme carries no read field for: refuse rather than wrap an
		// element in an empty scope that changes nothing.
		return Transform::Refused(fmt!("set {} lowered to nothing", target));
	}
	Transform::SetFields(patch)
}

/// The heading-group patch a `set text(size: ..., font: ...)` on a heading selector lowers to: the size the
/// renderer
/// reads for a heading is its own level size ([`Theme::heading_size`]), so the text size is redirected
/// there rather than into `text.body_size`, which a heading never reads. A `level: N` predicate targets
/// that one level; an unpredicated heading selector sizes every level alike (`size_all`). `None` when the
/// set names no size, or a size that does not convert to points (an `em`, which needs a running size this
/// lowering has not) -- the caller then refuses it rather than wrapping to no effect.
fn heading_text_patch(selector: &Selector, args: &str) -> Option<ThemePatch> {
	// Reuse the body readers: `set text(size: 30pt, font: "Felipa")` lowers its size into `text.body_size`
	// and its family list into `text.faces.body`, and those are exactly what to redirect into the heading
	// level. A heading draws in one named face, so the list's first family is its face; the fall-back
	// families after it are not carried (a heading's uncovered glyphs fall to the body role instead).
	let text	= set::lower_set("text", args).text;
	let face	= text.faces.body.as_ref().and_then(|l| l.first().cloned());
	if text.body_size.is_none() && face.is_none() {
		return None;
	}
	let mut patch = ThemePatch::default();
	match level_predicate(selector) {
		Some(n)	=> {
			let idx = (n.max(1) as usize) - 1;	// level 0/1 both index 0, as the theme maps them
			let mut levels: Vec<ThemeHeadingLevelPatch> = Vec::with_capacity(idx + 1);
			levels.resize_with(idx + 1, Default::default);
			levels[idx].size = text.body_size;
			levels[idx].face = face.map(Some);
			patch.heading.levels = levels;
		},
		None	=> {
			patch.heading.size_all = text.body_size;
			patch.heading.face_all = face.map(Some);
		},
	}
	Some(patch)
}

/// The single `level: N` a selector narrows to, or `None` for an unpredicated selector (or one narrowed by
/// some other predicate). Used to target a heading size rule at the one level it names.
fn level_predicate(selector: &Selector) -> Option<u8> {
	selector.predicates.iter().find_map(|p| match p {
		FieldPredicate::Level(n)	=> Some(*n),
		_							=> None,
	})
}

/// A selector rendered back to its source form -- `heading`, `heading.where(level: 1)` -- for a refusal
/// diagnostic that names which selector left a field unconsumed.
fn selector_label(selector: &Selector) -> String {
	let kind = match selector.kind {
		ElementKind::Heading		=> "heading",
		ElementKind::Figure			=> "figure",
		ElementKind::FigureCaption	=> "figure.caption",
		ElementKind::Raw			=> "raw",
		ElementKind::Equation		=> "equation",
		ElementKind::Link			=> "link",
		ElementKind::Paragraph		=> "par",
		ElementKind::List			=> "list",
		ElementKind::Table			=> "table",
	};
	if selector.predicates.is_empty() {
		kind.to_string()
	} else {
		let preds: Vec<String> = selector.predicates.iter().map(|p| match p {
			FieldPredicate::Level(n)	=> fmt!("level: {}", n),
			FieldPredicate::Block(b)	=> fmt!("block: {}", b),
		}).collect();
		fmt!("{}.where({})", kind, preds.join(", "))
	}
}

/// Why a `set <target>(<args>)` transform patches a field the selector's element does not read, or reads
/// only across a group boundary the readiness audit named -- so the rule is refused rather than wrapped to
/// no effect. `None` when every field it names is one the renderer reads for that element. Selector-aware:
/// a heading renders one shaped line from the `heading` group, so a `text` field that only styles running
/// body text is refused under a heading selector, whereas the heading's own size passes (it is redirected
/// into the heading group by [`heading_text_patch`]).
fn unread_field_reason(selector: &Selector, target: &str, args: &str) -> Option<String> {
	let has = |key: &str| names_arg(args, key);
	// A heading's only renderable `text` field is its size; the rest style running body text a heading
	// never sets, so they are refused here, naming the field and the selector.
	if selector.kind == ElementKind::Heading && target == "text" {
		if has("tracking")	{ return Some(fmt!("text.tracking is not read for {}", selector_label(selector))); }
		if has("ligatures")	{ return Some(fmt!("text.ligatures is not read for {}", selector_label(selector))); }
		if has("hyphenate")	{ return Some(fmt!("text.hyphenate is not read for {}", selector_label(selector))); }
		return None;
	}
	match target {
		"text" => {
			if has("tracking")	{ return Some("text.tracking is not read by the renderer".to_string()); }
			if has("ligatures")	{ return Some("text.ligatures is not read by the renderer".to_string()); }
			None
		},
		"heading" => {
			if has("smallcaps")	{ return Some("heading smallcaps is not read per level by the renderer".to_string()); }
			if has("font")		{ return Some("a heading font face is resolved elsewhere, not read from a rule".to_string()); }
			None
		},
		"figure"			=> Some("figure.skip / figure sizing is not read from a rule".to_string()),
		"raw" | "code"		=> Some("code.background / code sizing is not read from a rule".to_string()),
		"math.equation" | "equation"	=> Some("equation.numbering is inert; the renderer always sets (N)".to_string()),
		"page"				=> Some("page.* geometry is not consumed from a rule".to_string()),
		_					=> None,
	}
}

/// Does `args` name the top-level argument `key` (an identifier immediately before a `:`, at depth zero)?
/// Reuses the same word-boundary care as the `#set` reader so `font` is not found inside `heading-font`.
fn names_arg(args: &str, key: &str) -> bool {
	let bytes	= args.as_bytes();
	let mut from	= 0usize;
	while let Some(rel) = args[from..].find(key) {
		let at = from + rel;
		let before_ok = at == 0 || {
			let p = bytes[at - 1];
			!(p.is_ascii_alphanumeric() || p == b'-' || p == b'_')
		};
		let mut j = at + key.len();
		while j < bytes.len() && bytes[j] == b' ' {
			j += 1;
		}
		if before_ok && j < bytes.len() && bytes[j] == b':' {
			return true;
		}
		from = at + key.len();
	}
	false
}

/// A short, single-line echo of a transform body for a refusal message.
fn short(body: &str) -> String {
	let one: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
	if one.chars().count() > 40 {
		let mut s: String = one.chars().take(40).collect();
		s.push('…');
		s
	} else {
		one
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ LOWERING A TEMPLATE (a #show whose body wraps the element)                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// Lowers a `#show <selector>: <body>` whose body is not a bare `set` to a [`Transform::Template`], or
/// refuses it. The recognised shapes are the corpus's shared template forms: a `block.with(fill:, inset:,
/// radius:)` (or `block(fill: ...)[#it]`) wash around the element -- a callout frame; a `block(...)[ #set
/// text(...) #it.body ]` whose inner `#set` overlays the element (the hole patch); and `v(<len>)` / `line(...)`
/// statements set as siblings before or after the element (`pre` / `post`). An inline wrap (`underline`) and a
/// text rewrite (`regex`) have no block lowering and are refused; a page-reading body is already refused
/// upstream. Under a heading selector a `v(<len>)` folds into the matched level's `space_above` / `space_below`
/// rather than a sibling -- a sibling after a heading would break its keep-with-next -- and any other sibling
/// there is refused for the same reason.
fn lower_template(selector: &Selector, body: &str) -> Transform {
	let body = strip_code_block(body.trim());
	// An inline dress or a text rewrite is not a block-level template.
	if mentions_call(body, "underline") {
		return Transform::Refused(fmt!("underline wraps inline content, not a block: {}", short(body)));
	}
	if mentions_call(body, "regex") {
		return Transform::Refused(fmt!("a regex show rewrites matched text, not an element: {}", short(body)));
	}

	let heading			= selector.kind == ElementKind::Heading;
	let mut pre:	Vec<Block>		= Vec::new();
	let mut post:	Vec<Block>		= Vec::new();
	let mut hole					= ThemePatch::default();
	let mut frame:	Option<TemplateFrame>	= None;
	let mut seen_hole				= false;

	for stmt in split_statements(body) {
		let s = stmt.trim().trim_start_matches('#').trim();
		if s.is_empty() {
			continue;
		}
		if let Some(kind) = spacer_kind(s) {
			match make_spacer(selector, kind, s, seen_hole, heading, &mut hole) {
				Ok(None)		=> {},	// folded into the level's spacing (a heading v)
				Ok(Some(b))		=> if seen_hole { post.push(b); } else { pre.push(b); },
				Err(e)			=> return Transform::Refused(fmt!("{}", e)),
			}
		} else if is_element_stmt(s) {
			if seen_hole {
				return Transform::Refused(fmt!("a template names the element `it` more than once: {}", short(body)));
			}
			if let Err(e) = read_element(s, &mut hole, &mut frame) {
				return Transform::Refused(fmt!("{}", e));
			}
			seen_hole = true;
		} else {
			return Transform::Refused(fmt!("unsupported template statement: {}", short(s)));
		}
	}

	if !seen_hole {
		return Transform::Refused(fmt!("a template body names no element `it`: {}", short(body)));
	}
	// The rule_id is stamped by `collect_from_source` once the rule's index is known.
	Transform::Template(Template { pre, hole, post, frame, rule_id: 0 })
}

/// A code-block wrapper `{ ... }` stripped to its contents, so the statements inside can be split; a body
/// that is a single expression (a `block.with(...)`) is returned unchanged.
fn strip_code_block(body: &str) -> &str {
	let b = body.trim();
	match (b.strip_prefix('{'), b.strip_suffix('}')) {
		(Some(inner), _) if b.ends_with('}')	=> inner.trim(),
		_										=> b,
	}
}

/// Does `body` call `name` -- the identifier `name` immediately before a `(`, at a word boundary -- so a
/// template mentioning `underline(` or `regex(` is caught without matching it inside a longer word?
fn mentions_call(body: &str, name: &str) -> bool {
	let bytes	= body.as_bytes();
	let mut from	= 0usize;
	while let Some(rel) = body[from..].find(name) {
		let at = from + rel;
		let before_ok = at == 0 || {
			let p = bytes[at - 1];
			!(p.is_ascii_alphanumeric() || p == b'-' || p == b'_')
		};
		let after = at + name.len();
		if before_ok && after < bytes.len() && bytes[after] == b'(' {
			return true;
		}
		from = at + name.len();
	}
	false
}

/// Splits a template body into its top-level statements, at a newline or `;` outside any `(...)`, `[...]`,
/// `{...}` or `"..."` -- the statement separators of a Typst code block.
fn split_statements(body: &str) -> Vec<String> {
	let mut out		= Vec::new();
	let mut depth	= 0i32;
	let mut in_str	= false;
	let mut esc		= false;
	let mut cur		= String::new();
	for c in body.chars() {
		if in_str {
			cur.push(c);
			if esc				{ esc = false; }
			else if c == '\\'	{ esc = true; }
			else if c == '"'	{ in_str = false; }
			continue;
		}
		match c {
			'"'					=> { in_str = true; cur.push(c); },
			'(' | '[' | '{'		=> { depth += 1; cur.push(c); },
			')' | ']' | '}'		=> { depth -= 1; cur.push(c); },
			'\n' | ';' if depth == 0	=> { out.push(std::mem::take(&mut cur)); },
			_						=> cur.push(c),
		}
	}
	if !cur.trim().is_empty() {
		out.push(cur);
	}
	out
}

/// A `v(...)` or `line(...)` spacer statement, or `None` for a statement that is neither.
#[derive(Clone, Copy, PartialEq)]
enum SpacerKind {
	Vertical,	// v(<len>)
	Line,		// line(...) -- a horizontal divider
}

impl SpacerKind {
	fn label(self) -> &'static str {
		match self {
			SpacerKind::Vertical	=> "v()",
			SpacerKind::Line		=> "line()",
		}
	}
}

/// Which spacer, if any, this already-`#`-stripped statement opens with.
fn spacer_kind(s: &str) -> Option<SpacerKind> {
	if s.starts_with("v(")		{ Some(SpacerKind::Vertical) }
	else if s.starts_with("line(")	{ Some(SpacerKind::Line) }
	else						{ None }
}

/// Does this statement carry the element `it` -- a wrap (`block`/`box`) or a bare `it` reference? Used to
/// tell the hole statement from the spacers around it.
fn is_element_stmt(s: &str) -> bool {
	s.starts_with("block") || s.starts_with("box") || mentions_word(s, "it")
}

/// Does `s` contain the bare identifier `word` at a word boundary (so `it` is not found inside `with`)?
fn mentions_word(s: &str, word: &str) -> bool {
	let bytes	= s.as_bytes();
	let mut from	= 0usize;
	while let Some(rel) = s[from..].find(word) {
		let at		= from + rel;
		let before	= at == 0 || !is_ident_byte(bytes[at - 1]);
		let after_i	= at + word.len();
		let after	= after_i >= bytes.len() || !is_ident_byte(bytes[after_i]);
		if before && after {
			return true;
		}
		from = at + word.len();
	}
	false
}

fn is_ident_byte(b: u8) -> bool {
	b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

/// Builds the sibling block a spacer lowers to, or folds a heading's `v(...)` into the matched level's
/// spacing (returning `Ok(None)`). Under a heading a non-`v` sibling is refused -- a divider after a heading
/// strands it from the content it must keep with.
fn make_spacer(
	selector:	&Selector,
	kind:		SpacerKind,
	s:			&str,
	seen_hole:	bool,
	heading:	bool,
	hole:		&mut ThemePatch,
)
	-> Outcome<Option<Block>>
{
	if heading {
		if kind != SpacerKind::Vertical {
			// The keep-with-next guard: a heading must stay adjacent to the block it keeps with.
			if seen_hole {
				return Err(err!("post-content after a heading breaks keep-with-next"; Invalid, Input));
			}
			return Err(err!(
				"a heading template supports only v() spacing, not a leading {}", kind.label(); Invalid, Input));
		}
		let sp	= res!(spacer_length(s));
		let n	= res!(level_predicate(selector).ok_or_else(||
			err!("a heading spacing template needs a level: predicate"; Invalid, Input)));
		let idx	= (n.max(1) as usize) - 1;
		while hole.heading.levels.len() <= idx {
			hole.heading.levels.push(ThemeHeadingLevelPatch::default());
		}
		// A v() before the element lifts the level's space above; one after sets its space below.
		if seen_hole {
			hole.heading.levels[idx].space_below = Some(sp);
		} else {
			hole.heading.levels[idx].space_above = Some(sp);
		}
		return Ok(None);
	}
	match kind {
		SpacerKind::Vertical	=> Ok(Some(Block::Space(res!(spacer_length(s))))),
		SpacerKind::Line		=> Ok(Some(res!(read_line(s)))),
	}
}

/// The scaled-point length of a `v(<len>)` statement's argument. An `em` (or `%`) value has no running
/// size at lowering time, so it is refused rather than set wrongly.
fn spacer_length(s: &str) -> Outcome<Sp> {
	let inside = res!(call_args(s, "v").ok_or_else(|| err!("malformed v() spacer: {}", short(s); Invalid, Input)));
	match length_pt(inside.trim()) {
		Some(pt)	=> Ok(Sp::from_pt(pt)),
		None		=> Err(err!("a v() length needs an absolute unit (pt/mm/cm), not {}", short(&inside); Invalid, Input)),
	}
}

/// A `line(length: <len>, stroke: <n>pt)` lowered to a horizontal rule. A `length` given as a percentage is
/// a fraction of the placement measure ([`Length::Rel`]); an absolute length is [`Length::Abs`]. The stroke
/// thickness and grey take template-like defaults when the source names none.
fn read_line(s: &str) -> Outcome<Block> {
	let inside = res!(call_args(s, "line").ok_or_else(|| err!("malformed line() divider: {}", short(s); Invalid, Input)));
	let width = match named_value(&inside, "length") {
		Some(v) if v.trim_end().ends_with('%')	=> {
			let f = res!(v.trim_end().trim_end_matches('%').trim().parse::<f64>()
				.map_err(|_| err!("line length percentage not a number: {}", short(&v); Invalid, Input)));
			Length::Rel(f / 100.0)
		},
		Some(v)	=> match length_pt(v.trim()) {
			Some(pt)	=> Length::Abs(pt),
			None		=> return Err(err!("a line length needs pt/mm/cm or %, not {}", short(&v); Invalid, Input)),
		},
		None	=> Length::Rel(1.0),	// a bare divider runs the full measure
	};
	// The stroke, if any, is `<n>pt` optionally `+ luma(<g>)`; default a thin black rule.
	let (thickness, grey) = match named_value(&inside, "stroke") {
		Some(v)	=> (length_pt(&v).unwrap_or(0.6), stroke_grey(&v).unwrap_or(0)),
		None	=> (0.6, 0),
	};
	Ok(Block::Rule { width, thickness, grey })
}

/// The grey level a `stroke: ... + luma(<g>)` names, or `None` when the stroke carries no `luma`.
fn stroke_grey(v: &str) -> Option<u8> {
	let at	= v.find("luma(")? + "luma(".len();
	let end	= v[at..].find(')')?;
	v[at..at + end].trim().parse::<f64>().ok().map(|n| n.round().clamp(0.0, 255.0) as u8)
}

/// Reads the element (hole) statement of a template into the hole patch and frame. A `block.with(fill: ...)`
/// or `block(fill: ...)[#it]` sets the frame's wash, plus its `inset:`/`radius:` when the same call names
/// them; a `#set text(...)` inside the wrap's content overlays the element; a bare `it` leaves both
/// untouched. A wrap that is neither a `.with` partial nor a content wrap of `it` is refused, so a body
/// this reader cannot place is a visible refusal, not a silent no-op -- and so is an `inset`/`radius` this
/// reader cannot resolve to a length (an `em` or `%` value, or a dict form naming something other than
/// `x`/`y`/`bottom`), rather than the fill being framed while its geometry is quietly dropped.
fn read_element(s: &str, hole: &mut ThemePatch, frame: &mut Option<TemplateFrame>) -> Outcome<()> {
	let is_wrap = s.starts_with("block") || s.starts_with("box");
	if !is_wrap {
		// A bare `it` / `it.body` -- the element passes through untouched.
		return Ok(());
	}
	// The wrap's argument list -- `block.with(<args>)` or `block(<args>)[...]`.
	let head = s.strip_prefix("block").or_else(|| s.strip_prefix("box")).unwrap_or(s);
	let head = head.trim_start_matches(".with").trim_start();
	let args = call_group(head).unwrap_or_default();
	// A `fill:` washes the element in a box; `inset:`/`radius:` in the same call ride along on it, since
	// neither means anything without a box to draw them on.
	if let Some(fv) = named_value(&args, "fill") {
		match parse_colour(&fv) {
			Some(rgba)	=> {
				let mut tf = TemplateFrame { fill: rgba, inset_x: None, inset_top: None, inset_bot: None, radius: None };
				if let Some(iv) = named_value(&args, "inset") {
					res!(read_inset(&iv, &mut tf));
				}
				if let Some(rv) = named_value(&args, "radius") {
					tf.radius = Some(Sp::from_pt(res!(length_pt_or_refuse("radius", &rv))));
				}
				*frame = Some(tf);
			},
			None		=> return Err(err!(
				"a template fill colour could not be resolved: {}", short(&fv); Invalid, Input)),
		}
	}
	// A content block `[ ... ]` may carry `#set` overlays and must reference `it` when the wrap is not a
	// `.with` partial application.
	let has_partial	= s.contains(".with");
	if let Some(content) = bracket_content(s) {
		for (target, cargs) in inner_sets(&content) {
			merge_patch(hole, &set::lower_set(&target, &cargs));
		}
		if !mentions_word(&content, "it") {
			return Err(err!("a template wrap's content does not place the element `it`: {}", short(s); Invalid, Input));
		}
	} else if !has_partial {
		return Err(err!("a template wrap places no element `it`: {}", short(s); Invalid, Input));
	}
	Ok(())
}

/// Reads a `block.with(inset: ...)` argument into `tf`: a scalar length (`inset: 8pt`) pads every side
/// alike, and the dict form (`inset: (x: 8pt, y: 6pt, bottom: 8pt)`) reuses the corpus's own shape -- `x`
/// the horizontal pad, `y` the top pad (and the foot pad too, unless `bottom` overrides it), `bottom` the
/// foot pad alone. A dict key this reader does not recognise, or a length it cannot resolve to points (an
/// `em` or `%` value), is refused rather than silently left at the renderer's default.
fn read_inset(raw: &str, tf: &mut TemplateFrame) -> Outcome<()> {
	let raw = raw.trim();
	if raw.starts_with('(') {
		let inner = res!(call_group(raw).ok_or_else(||
			err!("a template inset dict is not a closed (...) group: {}", short(raw); Invalid, Input)));
		let mut named = false;
		if let Some(xv) = named_value(&inner, "x") {
			tf.inset_x = Some(Sp::from_pt(res!(length_pt_or_refuse("inset x", &xv))));
			named = true;
		}
		if let Some(yv) = named_value(&inner, "y") {
			let pt = Sp::from_pt(res!(length_pt_or_refuse("inset y", &yv)));
			tf.inset_top = Some(pt);
			tf.inset_bot = Some(pt);	// `y` sets top and bottom alike, unless `bottom` overrides it below
			named = true;
		}
		if let Some(bv) = named_value(&inner, "bottom") {
			tf.inset_bot = Some(Sp::from_pt(res!(length_pt_or_refuse("inset bottom", &bv))));
			named = true;
		}
		if !named {
			return Err(err!("a template inset dict names none of x/y/bottom: {}", short(raw); Invalid, Input));
		}
		Ok(())
	} else {
		let pt = Sp::from_pt(res!(length_pt_or_refuse("inset", raw)));
		tf.inset_x		= Some(pt);
		tf.inset_top	= Some(pt);
		tf.inset_bot	= Some(pt);
		Ok(())
	}
}

/// A length token to points, refusing rather than silently dropping a value [`length_pt`] cannot resolve
/// (an `em` or `%`, which has no absolute size at lowering time) -- named by `field` for the diagnostic.
fn length_pt_or_refuse(field: &str, v: &str) -> Outcome<f64> {
	match length_pt(v) {
		Some(pt)	=> Ok(pt),
		None		=> Err(err!(
			"a template {} length could not be resolved to points (em/% are not supported here): {}",
			field, short(v); Invalid, Input)),
	}
}

/// Folds the non-default leaves of `src` onto `dst` -- the overlay a wrap's inner `#set` contributes to the
/// hole. Each group `lower_set` writes is folded, the heading group per level so a heading `v(...)` spacing
/// already folded in stands beside a `#set heading(...)` the same wrap might carry.
fn merge_patch(dst: &mut ThemePatch, src: &ThemePatch) {
	let d = ThemePatch::default();
	if src.text != d.text				{ dst.text = src.text.clone(); }
	if src.par != d.par					{ dst.par = src.par.clone(); }
	if src.list != d.list				{ dst.list = src.list.clone(); }
	if src.enumeration != d.enumeration	{ dst.enumeration = src.enumeration.clone(); }
	if src.equation != d.equation		{ dst.equation = src.equation.clone(); }
	if src.page != d.page				{ dst.page = src.page.clone(); }
	if src.code != d.code				{ dst.code = src.code.clone(); }
	// The heading group, folded leaf by leaf so a level's spacing set elsewhere survives.
	if src.heading.numbering_all.is_some()	{ dst.heading.numbering_all = src.heading.numbering_all.clone(); }
	if src.heading.size_all.is_some()		{ dst.heading.size_all = src.heading.size_all; }
	if src.heading.face_all.is_some()		{ dst.heading.face_all = src.heading.face_all.clone(); }
	if src.heading.face.is_some()			{ dst.heading.face = src.heading.face.clone(); }
	if src.heading.kind.is_some()			{ dst.heading.kind = src.heading.kind; }
	for (i, lvl) in src.heading.levels.iter().enumerate() {
		while dst.heading.levels.len() <= i {
			dst.heading.levels.push(ThemeHeadingLevelPatch::default());
		}
		let into = &mut dst.heading.levels[i];
		if lvl.size.is_some()			{ into.size = lvl.size; }
		if lvl.space_above.is_some()	{ into.space_above = lvl.space_above; }
		if lvl.space_below.is_some()	{ into.space_below = lvl.space_below; }
		if lvl.face.is_some()			{ into.face = lvl.face.clone(); }
		if lvl.weight.is_some()			{ into.weight = lvl.weight; }
		if lvl.italic.is_some()			{ into.italic = lvl.italic; }
		if lvl.smallcaps.is_some()		{ into.smallcaps = lvl.smallcaps; }
		if lvl.numbering.is_some()		{ into.numbering = lvl.numbering.clone(); }
	}
}

/// The top-level `#set <target>(<args>)` declarations inside a wrap's content block, as `(target, args)`
/// pairs -- the overlay a `block(...)[ #set text(size: 9pt) #it ]` contributes to the hole.
fn inner_sets(content: &str) -> Vec<(String, String)> {
	let mut out = Vec::new();
	for stmt in split_statements(content) {
		let s = stmt.trim().trim_start_matches('#').trim();
		let after = match s.strip_prefix("set ") {
			Some(a)	=> a.trim(),
			None	=> continue,
		};
		let open = match after.find('(') {
			Some(i)	=> i,
			None	=> continue,
		};
		let target = after[..open].trim().to_string();
		if let Some(args) = call_group(&after[open..]) {
			out.push((target, args));
		}
	}
	out
}

/// The text inside the first balanced `(...)` of `call(...)` when `s` opens with `name`, or `None`.
fn call_args(s: &str, name: &str) -> Option<String> {
	let rest = s.strip_prefix(name)?.trim_start();
	call_group(rest)
}

/// The text inside a balanced `(...)` at the start of `s` (which must open with `(`), spanning nested
/// brackets and strings. `None` when the parentheses never close.
fn call_group(s: &str) -> Option<String> {
	let s = s.trim_start();
	let bytes = s.as_bytes();
	if bytes.first() != Some(&b'(') {
		return None;
	}
	let mut depth	= 0i32;
	let mut in_str	= false;
	let mut esc		= false;
	for (i, c) in s.char_indices() {
		if in_str {
			if esc				{ esc = false; }
			else if c == '\\'	{ esc = true; }
			else if c == '"'	{ in_str = false; }
			continue;
		}
		match c {
			'"'			=> in_str = true,
			'(' | '[' | '{'	=> depth += 1,
			')' | ']' | '}'	=> {
				depth -= 1;
				if depth == 0 {
					return Some(s[1..i].to_string());
				}
			},
			_			=> {},
		}
	}
	None
}

/// The text inside the first balanced `[...]` content block of `s`, or `None` when there is none.
fn bracket_content(s: &str) -> Option<String> {
	let start	= s.find('[')?;
	let bytes	= s.as_bytes();
	let mut depth	= 0i32;
	for (i, c) in s[start..].char_indices() {
		match c {
			'['	=> depth += 1,
			']'	=> {
				depth -= 1;
				if depth == 0 {
					let _ = bytes;
					return Some(s[start + 1..start + i].to_string());
				}
			},
			_	=> {},
		}
	}
	None
}

/// The raw value text a `key:` names inside an argument list, up to the next top-level comma. `None` when
/// the key is absent.
fn named_value(args: &str, key: &str) -> Option<String> {
	let bytes	= args.as_bytes();
	let mut from	= 0usize;
	let start = loop {
		let rel	= args[from..].find(key)?;
		let at	= from + rel;
		let before_ok = at == 0 || !is_ident_byte(bytes[at - 1]);
		let mut j = at + key.len();
		while j < bytes.len() && bytes[j] == b' ' {
			j += 1;
		}
		if before_ok && j < bytes.len() && bytes[j] == b':' {
			break j + 1;
		}
		from = at + key.len();
	};
	// Read to the next comma outside any nested group.
	let tail	= &args[start..];
	let mut depth	= 0i32;
	let mut end		= tail.len();
	for (i, c) in tail.char_indices() {
		match c {
			'(' | '[' | '{'	=> depth += 1,
			')' | ']' | '}'	=> depth -= 1,
			',' if depth == 0	=> { end = i; break; },
			_			=> {},
		}
	}
	Some(tail[..end].trim().to_string())
}

/// A template's named colour palette (`#let colours = (yellow: rgb("#f0f600"), ...)`), by name. Empty
/// unless a book's palette is collected from its template chain; a `colours.<name>` reference resolves
/// against it.
pub type Palette = std::collections::HashMap<String, Rgba>;

/// A colour expression lowered to an [`Rgba`]. The forms a template fill takes that resolve without a
/// palette: `luma(<n>)`, `rgb("#rrggbb")`, `rgb(<r>, <g>, <b>)` and a small set of named colours, each
/// optionally lightened or darkened (`.lighten(<p>%)` / `.darken(<p>%)`). A palette reference (`colours.blue`)
/// resolves only through [`parse_colour_pal`], which is given the book's palette.
///
/// Shared with the `#set text(fill:)` lowering ([`crate::lang::set`]), which reads a body-text colour
/// with the same grammar, so the two readers cannot drift.
pub(crate) fn parse_colour(expr: &str) -> Option<Rgba> {
	parse_colour_pal(expr, &Palette::new())
}

/// As [`parse_colour`], resolving a `colours.<name>` reference against `palette` (and applying any trailing
/// `.lighten`/`.darken` to the looked-up colour). With an empty palette this is exactly [`parse_colour`].
pub(crate) fn parse_colour_pal(expr: &str, palette: &Palette) -> Option<Rgba> {
	let e = expr.trim();
	// The base runs up to the first `.lighten`/`.darken` modifier (a `luma(...)`/`rgb(...)` call keeps its
	// own parentheses); the rest is the modifier chain.
	let split_at = [".lighten", ".darken"].iter().filter_map(|m| e.find(m)).min();
	let (head, mods) = match split_at {
		Some(i)	=> (e[..i].trim(), &e[i..]),
		None	=> (e, ""),
	};
	let base = if let Some(rest) = head.strip_prefix("luma(") {
		let n = rest.trim_end_matches(')').trim().parse::<f64>().ok()?;
		let v = n.round().clamp(0.0, 255.0) as u8;
		Rgba::opaque(v, v, v)
	} else if let Some(rest) = head.strip_prefix("rgb(") {
		res_rgb(rest.trim_end_matches(')').trim())?
	} else if let Some(name) = head.strip_prefix("colours.").or_else(|| head.strip_prefix("colors.")) {
		*palette.get(name.trim())?
	} else {
		named_colour(head)?
	};
	Some(apply_colour_mods(base, mods))
}

/// Collects a template's `#let colours = ( name: <colour>, ... )` palette from `src` into `palette`, so a
/// `colours.<name>` reference in a furniture fill or stroke resolves. Each entry's value is read with the
/// same colour grammar as a fill (`rgb("#...")`, `luma(...)`, a named colour). An entry this reader cannot
/// resolve is passed over; a source with no such binding adds nothing.
pub fn collect_palette(src: &str, palette: &mut Palette) {
	let chars:	Vec<char>	= src.chars().collect();
	let mut i	= 0usize;
	while i < chars.len() {
		// The literal must name `colours` exactly, not merely start with it -- `#let colours_x = (...)`
		// is a different binding and must not be read as the palette.
		if at_line_start(&chars, i) && starts_with_at(&chars, i, "#let colours")
			&& !chars.get(i + "#let colours".chars().count()).is_some_and(|&c| is_ident_char(c))
		{
			// The dict opens at the first `(` after the `=`.
			let mut j = i;
			while j < chars.len() && chars[j] != '(' && chars[j] != '\n' {
				j += 1;
			}
			if chars.get(j) == Some(&'(') {
				if let Some((inner, next)) = read_delim_group(&chars, j) {
					for entry in split_top_commas_str(&inner) {
						if let Some((name_part, val_part)) = entry.split_once(':') {
							// The name is the last line of the key part, so a `//` comment line preceding the
							// entry is dropped; the value is taken up to any trailing `//` line comment.
							let name = name_part.rsplit('\n').next().unwrap_or(name_part).trim();
							let val = val_part.split("//").next().unwrap_or(val_part).trim();
							if !name.is_empty() && name.chars().all(is_ident_char) {
								if let Some(rgba) = parse_colour(val) {
									palette.insert(name.to_string(), rgba);
								}
							}
						}
					}
					i = next;
					continue;
				}
			}
		}
		i += 1;
	}
}

/// `rgb("#rrggbb")` or `rgb(<r>, <g>, <b>)` to an [`Rgba`].
fn res_rgb(inner: &str) -> Option<Rgba> {
	let inner = inner.trim();
	if let Some(hex) = inner.strip_prefix('"').and_then(|h| h.strip_suffix('"')) {
		let hex = hex.trim_start_matches('#');
		if hex.len() == 6 {
			let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
			let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
			let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
			return Some(Rgba::opaque(r, g, b));
		}
		return None;
	}
	let parts: Vec<&str> = inner.split(',').map(|p| p.trim()).collect();
	if parts.len() == 3 {
		let r = parts[0].parse::<f64>().ok()?.round().clamp(0.0, 255.0) as u8;
		let g = parts[1].parse::<f64>().ok()?.round().clamp(0.0, 255.0) as u8;
		let b = parts[2].parse::<f64>().ok()?.round().clamp(0.0, 255.0) as u8;
		return Some(Rgba::opaque(r, g, b));
	}
	None
}

/// A named colour to its [`Rgba`], for the handful Typst's own defaults carry.
fn named_colour(name: &str) -> Option<Rgba> {
	Some(match name {
		"black"				=> Rgba::opaque(0, 0, 0),
		"white"				=> Rgba::opaque(255, 255, 255),
		"gray" | "grey"		=> Rgba::opaque(170, 170, 170),
		"silver"			=> Rgba::opaque(221, 221, 221),
		"red"				=> Rgba::opaque(255, 65, 54),
		"green"				=> Rgba::opaque(46, 204, 64),
		"blue"				=> Rgba::opaque(0, 116, 217),
		"yellow"			=> Rgba::opaque(255, 220, 0),
		"orange"			=> Rgba::opaque(255, 133, 27),
		"purple"			=> Rgba::opaque(177, 13, 201),
		_					=> return None,
	})
}

/// Applies the trailing `.lighten(<p>%)` / `.darken(<p>%)` modifiers of a colour expression, each mixing the
/// colour that fraction toward white or black the way Typst's own `.lighten`/`.darken` do.
fn apply_colour_mods(base: Rgba, mods: &str) -> Rgba {
	let mut c = base;
	let mut rest = mods;
	loop {
		let dot = match rest.find('.') {
			Some(i)	=> i,
			None	=> break,
		};
		let after = &rest[dot + 1..];
		let open = match after.find('(') {
			Some(i)	=> i,
			None	=> break,
		};
		let name = after[..open].trim();
		let inner = match call_group(&after[open..]) {
			Some(g)	=> g,
			None	=> break,
		};
		let pct = inner.trim().trim_end_matches('%').trim().parse::<f64>().unwrap_or(0.0) / 100.0;
		c = match name {
			"lighten"	=> mix(c, 255, pct),
			"darken"	=> mix(c, 0, pct),
			_			=> c,
		};
		// Advance past this `.name(inner)` modifier: the dot, the name, and the balanced `(inner)`.
		let consumed = dot + 1 + open + 1 + inner.len() + 1;
		if consumed >= rest.len() {
			break;
		}
		rest = &rest[consumed..];
	}
	c
}

/// Mixes each channel of `c` a fraction `t` toward `target` (0 or 255) -- the arithmetic behind lighten/darken.
fn mix(c: Rgba, target: i32, t: f64) -> Rgba {
	let f = |v: u8| -> u8 {
		let nv = v as f64 + (target as f64 - v as f64) * t;
		nv.round().clamp(0.0, 255.0) as u8
	};
	Rgba::new(f(c.r), f(c.g), f(c.b), c.a)
}

/// A length token to points, accepting `pt`, `mm`, `cm`, `in` or a bare number. An `em` or `%` value has no
/// absolute size at lowering time, so it returns `None` and the caller refuses it.
fn length_pt(s: &str) -> Option<f64> {
	let s = s.trim();
	let mut end = 0usize;
	let mut seen_dot = false;
	for (i, c) in s.char_indices() {
		if c.is_ascii_digit() || (c == '-' && i == 0) {
			end = i + c.len_utf8();
		} else if c == '.' && !seen_dot {
			seen_dot = true;
			end = i + c.len_utf8();
		} else {
			break;
		}
	}
	if end == 0 {
		return None;
	}
	let num: f64 = s[..end].parse().ok()?;
	let unit = s[end..].trim();
	match unit {
		"" | "pt"	=> Some(num),
		"mm"		=> Some(num * 72.0 / 25.4),
		"cm"		=> Some(num * 72.0 / 2.54),
		"in"		=> Some(num * 72.0),
		_			=> None,
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ #let TEMPLATE FUNCTIONS (a `#let name(params) = block/box(...)` furniture)  │
// └───────────────────────────────────────────────────────────────────────────┘

/// A `#let name(params) = <expr>` furniture function the reader expands at each call site. The two the
/// Lucronics corpus defines -- `#pr-note(body)` (a plain indented, tightened block) and
/// `#aside-box(title: ..., body)` (a washed, left-stroked callout) -- both wrap their body parameter in a
/// `block(...)`/`box(...)`, so a call `#name[ ... ]` sets that body inside the furniture's frame rather
/// than being tallied as a skipped construct.
///
/// The definition is lowered once (at book-assembly time, against the document's body size, so every
/// `em` length resolves to an absolute at that size) into a serialisable [`ThemePatch`] carrying the
/// frame's geometry (`callout.*`) and the inner `#set text`/`#set par` overlay (`text.*`/`par.*`). A call
/// then re-parses its `[ ... ]` body and wraps it in a `Block::Box` under that patch -- the same shape a
/// `#styled-box[...]` produces, so the callout renderer sets it with no new path.
#[derive(Clone, Debug, PartialEq)]
pub struct TemplateFn {
	pub body_param:		String,			// the content parameter the call's `[ ... ]` body fills
	pub has_title:		bool,			// a `title:` parameter -> a leading bold title paragraph in the body
	pub title_size:		Option<Sp>,		// the title run's text size, em-resolved (a bold paragraph is set at it)
	pub patch:			ThemePatch,		// the frame geometry and the inner-set overlay, merged
	pub float:			Option<FloatPlacement>,	// Some when the body is re-wrapped in `figure(placement: ...)` -- a float
}

/// The template functions in scope for a source, by name. Empty until a book's definitions are collected;
/// a source with none reads exactly as before.
pub type TemplateFns = std::collections::HashMap<String, TemplateFn>;

/// A `#let name = [ ... ]` (value) or `#let name(p, ...) = [ ... ]` (function) content binding: markup
/// captured verbatim and, at each reference, expanded by substituting the call's positional arguments for
/// each `#param` in the body and re-reading the result as document markup. Unlike a [`TemplateFn`], whose
/// body is a furniture wrap the reader lowers to a padded box, a content binding's body is arbitrary block
/// markup -- headings, paragraphs, nested calls -- so its blocks are spliced into the stream, not boxed.
#[derive(Clone, Debug, PartialEq)]
pub struct ContentFn {
	pub params:		Vec<String>,		// the positional parameter names, empty for a value binding
	pub body:		String,				// the bracketed markup, its `[` `]` delimiters stripped
	pub wrapper:	Option<String>,		// a `box`/`rect`/`block` styling wrap the body was lifted out of
}

/// The content bindings in scope for a source, by name. Empty until a document's definitions are collected;
/// a source with none reads exactly as before.
pub type ContentFns = std::collections::HashMap<String, ContentFn>;

/// A `#let name = <literal>` scalar value binding: a bare string, integer, float or length literal, held as
/// its own display text so a later `#name` reference substitutes it verbatim. Unlike a [`ContentFn`], whose
/// body is markup re-read through the reader, a scalar's value needs no re-parse -- Typst renders a bare
/// literal exactly as it was written (`3`, `1.5`, `12pt`), so the source text a scalar was declared with IS
/// its display text, with a string's quotes stripped.
#[derive(Clone, Debug, PartialEq)]
pub enum ScalarValue {
	Str(String),	// a `"..."` string, its quotes stripped
	Number(String),	// an int, float or length literal, kept as its own source text (unit included)
}

impl ScalarValue {
	/// The text a `#name` reference substitutes: the string's contents, or the number/length literal's own
	/// source text.
	pub fn display_text(&self) -> &str {
		match self {
			Self::Str(s)	=> s,
			Self::Number(s)	=> s,
		}
	}
}

/// The scalar value bindings in scope for a source, by name. Empty until a document's definitions are
/// collected; a source with none reads exactly as before.
pub type ScalarFns = std::collections::HashMap<String, ScalarValue>;

/// The `#let` bindings a parse resolves a call against: the furniture functions ([`TemplateFns`], expanded
/// into a padded box) and the content bindings ([`ContentFns`], spliced as markup). Threaded as one through
/// the reader so a caller passes both together and a nested body carries the same scope. Borrowed, so it is
/// [`Copy`] and travels without a clone.
///
/// `active` is the stack of content-binding names currently being expanded, innermost last. A reference to a
/// name already on it is a cycle (`#let a = [#a]`, or the mutual `#let a = [#b]`/`#let b = [#a]`) and is
/// refused at once -- so a cycle recurses only to its own length, never until the native stack or the wasm
/// shadow stack overflows. Its length also caps a pathological non-cyclic chain (see the reader's own cap).
#[derive(Clone, Copy)]
pub struct Bindings<'a, 'b> {
	pub tfns:	&'a TemplateFns,
	pub cfns:	&'a ContentFns,
	pub sfns:	&'a ScalarFns,
	pub active:	&'b [String],
}

impl<'a> Bindings<'a, 'static> {
	/// No scalar scope to hand: borrows the empty [`ScalarFns`] map, so a caller with only furniture and
	/// content bindings in scope reads exactly as before.
	pub fn new(tfns: &'a TemplateFns, cfns: &'a ContentFns) -> Self {
		Self { tfns, cfns, sfns: empty_scalar_fns(), active: &[] }
	}

	/// As [`Self::new`], with the scalar `#let` value bindings a full `#let` scope also carries -- see
	/// [`crate::book::Scope::bindings`], which is how a book or lone-file compile builds one.
	pub fn with_scalars(tfns: &'a TemplateFns, cfns: &'a ContentFns, sfns: &'a ScalarFns) -> Self {
		Self { tfns, cfns, sfns, active: &[] }
	}
}

impl<'a, 'b> Bindings<'a, 'b> {
	/// Is `name` already being expanded -- a content-binding cycle?
	pub fn expanding(&self, name: &str) -> bool {
		self.active.iter().any(|n| n == name)
	}

	/// The number of content-binding expansions currently open.
	pub fn depth(&self) -> usize {
		self.active.len()
	}

	/// The same bindings with `active` as the stack of names in expansion, for re-reading an expanded body.
	pub fn with_active<'c>(self, active: &'c [String]) -> Bindings<'a, 'c> {
		Bindings { tfns: self.tfns, cfns: self.cfns, sfns: self.sfns, active }
	}
}

/// The empty [`ScalarFns`] map [`Bindings::new`] borrows when a caller has no scalar scope to hand -- a
/// `'static` empty map costs nothing to share and needs no per-call allocation.
fn empty_scalar_fns() -> &'static ScalarFns {
	static EMPTY: std::sync::OnceLock<ScalarFns> = std::sync::OnceLock::new();
	EMPTY.get_or_init(ScalarFns::new)
}

/// Collects every `#let name(params) = block/box(...)` furniture definition in `src` into `tfns`, lowering
/// each against `body_size` so its `em` lengths resolve to absolutes. A definition whose body this reader
/// cannot lower (not a `block`/`box` wrap, or naming a length it cannot resolve) is passed over silently --
/// the call then stays a tallied skip, exactly as before, rather than expanding wrongly. A byte-identical
/// definition seen twice (the corpus repeats `#let pr-note` verbatim atop three chapters) re-inserts the
/// same value, so the map is definition-order-independent.
pub fn collect_template_fns(src: &str, body_size: Sp, palette: &Palette, tfns: &mut TemplateFns) {
	let chars:	Vec<char>	= src.chars().collect();
	let mut i	= 0usize;
	while i < chars.len() {
		// A definition opens at a line-leading `#let <ident>(` -- a function `#let`, whose name is followed by
		// a parameter list. (`#let name = (...)` -- a value binding -- has no `(` right after the name and is
		// left to the data-array reader.)
		if at_line_start(&chars, i) && starts_with_at(&chars, i, "#let ") {
			if let Some((name, params, expr, next)) = read_let_fn(&chars, i) {
				// A name the reader already handles as a built-in construct (`styled-box`, `padded-image`,
				// `part-page`, ...) is NOT overridden by a collected definition, so the built-in path stays
				// authoritative and a corpus that defines its own `styled-box` renders exactly as before.
				if !is_reserved_construct(&name) {
					if let Some(tf) = lower_template_fn(&params, &expr, body_size, palette) {
						tfns.insert(name, tf);
					}
				}
				i = next;
				continue;
			}
		}
		i += 1;
	}
}

/// Collects every `#let name = [ ... ]` and `#let name(params) = [ ... ]` content binding in `src` into
/// `cfns`. The body is a bracket-balanced `[ ... ]`, captured verbatim with its parameter names, so a
/// reference expands into re-read markup. A `#let` whose body is a data array (`= (...)`) or a scalar is
/// passed over here -- the array and scalar readers keep those -- and a name the reader already handles as
/// a built-in construct is not overridden. A binding seen twice re-inserts the same value, so the map is
/// definition-order-independent.
///
/// A body that is a `box(...)[ ... ]`, `rect(...)[ ... ]` or `block(...)[ ... ]` styling wrap -- a content
/// function whose text is set inside a styled box, `#let stamp(s) = box(fill: ..)[*v: #s*]` -- is captured
/// as a content binding of its INNER `[ ... ]` content, with the wrapper name held so the styling this
/// reader cannot draw is recorded as a visible skip when the binding expands. The inner text is kept and
/// set, never silently dropped. This is distinct from a furniture wrap (`#pr-note`, whose content block
/// sits INSIDE the call's parens and the [`collect_template_fns`] reader draws as a styled block): a
/// furniture definition carries no `[ ... ]` group TRAILING the wrap's closing `)`, so the two shapes do
/// not collide, and where a name were somehow read by both, the furniture map wins at every call site.
pub fn collect_content_fns(src: &str, cfns: &mut ContentFns) {
	let chars:	Vec<char>	= src.chars().collect();
	let mut i	= 0usize;
	while i < chars.len() {
		if at_line_start(&chars, i) && starts_with_at(&chars, i, "#let ") {
			if let Some((name, params, body, wrapper, next)) = read_let_content(&chars, i) {
				if !is_reserved_construct(&name) {
					cfns.insert(name, ContentFn { params, body, wrapper });
				}
				i = next;
				continue;
			}
		}
		i += 1;
	}
}

/// Reads a `#let name = [ ... ]` or `#let name(params) = [ ... ]` content binding beginning at `at` (the
/// `#`), returning the name, its positional parameter names (empty for a value binding), the bracketed body
/// with its delimiters stripped, the styling wrapper the body was lifted out of (`Some("box")` and kin, or
/// `None` for a plain bracket body), and the index just past it. `None` when the line is not a
/// content-binding `#let`: a data array `= (...)` and a scalar fail the body check below, so this reader
/// leaves them to the array and scalar readers.
///
/// The recognised body is either a bare `[ ... ]`, or a `box(...)[ ... ]`, `rect(...)[ ... ]` or
/// `block(...)[ ... ]` styling wrap whose content group TRAILS the wrap's closing `)` -- a content function
/// styled by a box. The trailing group tells this shape apart from a furniture definition, whose content
/// block sits inside the wrap's parens; a furniture `= block(...)` with no trailing `[ ... ]` fails the
/// check here and is left to [`collect_template_fns`].
fn read_let_content(chars: &[char], at: usize) -> Option<(String, Vec<String>, String, Option<String>, usize)> {
	let mut j = at + "#let ".chars().count();
	let name_start = j;
	while j < chars.len() && is_ident_char(chars[j]) {
		j += 1;
	}
	let name: String = chars[name_start..j].iter().collect();
	if name.is_empty() {
		return None;
	}
	// An optional parameter list `( ... )` for a function binding. Only a bare positional identifier is a
	// substitutable parameter; a keyword default (`title: ...`) or a spread is not, so it is passed over.
	let mut params = Vec::new();
	if chars.get(j) == Some(&'(') {
		let (plist, after) = read_paren_group(chars, j)?;
		params = split_top_commas_str(&plist).into_iter().filter_map(|p| {
			let p = p.trim();
			if !p.is_empty() && p.chars().all(is_ident_char) { Some(p.to_string()) } else { None }
		}).collect();
		j = after;
	}
	// The `=` separating the signature from the body.
	while j < chars.len() && chars[j].is_whitespace() {
		j += 1;
	}
	if chars.get(j) != Some(&'=') {
		return None;
	}
	j += 1;
	while j < chars.len() && chars[j].is_whitespace() {
		j += 1;
	}
	// A bare bracket body `[ ... ]` -- the plain content binding.
	if chars.get(j) == Some(&'[') {
		let (body, next) = read_delim_group(chars, j)?;
		return Some((name, params, body, None, next));
	}
	// A styling wrap `box(...)[ ... ]` / `rect(...)[ ... ]` / `block(...)[ ... ]`: a content function whose
	// text is set inside a styled box. The inner `[ ... ]` content is the binding's body; the wrapper name is
	// carried so the styling this reader cannot draw records a visible skip when the binding expands. The
	// content group must TRAIL the wrap's closing `)` -- a furniture definition (`#pr-note`) carries its
	// content block inside the parens and has no trailing group, so it fails here and stays with the
	// furniture reader.
	for wrap in ["box", "rect", "block"] {
		if starts_with_at(chars, j, wrap) {
			let after_name = j + wrap.chars().count();
			// A genuine wrap call: the name is followed immediately by `(`, not part of a longer identifier.
			if chars.get(after_name) != Some(&'(') {
				continue;
			}
			if let Some((_, after_args)) = read_delim_group(chars, after_name) {
				if chars.get(after_args) == Some(&'[') {
					let (body, next) = read_delim_group(chars, after_args)?;
					return Some((name, params, body, Some(wrap.to_string()), next));
				}
			}
		}
	}
	None
}

/// Collects every `#let name = <literal>` scalar value binding in `src` into `sfns`: a bare `"..."` string,
/// or an integer, float or length literal, with nothing else on the right of the `=`. A `#let` whose body
/// is furniture (`= block/box(...)`), content (`= [ ... ]`), a data array (`= (...)`), a function signature
/// (`name(params) = ...`) or any other expression this reader does not evaluate (a call, a concatenation, an
/// identifier) is passed over here -- it is left as a visible `#let` skip, exactly as before -- and a name
/// the reader already handles as a built-in construct is not overridden. A binding seen twice re-inserts the
/// same value, so the map is definition-order-independent.
pub fn collect_scalar_fns(src: &str, sfns: &mut ScalarFns) {
	let chars:	Vec<char>	= src.chars().collect();
	let mut i	= 0usize;
	while i < chars.len() {
		if at_line_start(&chars, i) && starts_with_at(&chars, i, "#let ") {
			if let Some((name, value, next)) = read_let_scalar(&chars, i) {
				if !is_reserved_construct(&name) {
					sfns.insert(name, value);
				}
				i = next;
				continue;
			}
		}
		i += 1;
	}
}

/// Reads a `#let name = <literal>` scalar binding beginning at `at` (the `#`), returning the name, its
/// value, and the index just past the line it stands on. `None` when the line is not a scalar `#let`: a
/// name immediately followed by `(` is a function signature, left to [`read_let_content`]'s params check and
/// [`crate::lang::rules::collect_template_fns`]; a value that does not read as a bare string, integer, float
/// or length literal -- a `[...]` content body, a `(...)` array, a call, an `if`, an identifier or any other
/// expression -- is left as it stands, for the same visible `#let` skip a scalar binding got before this
/// reader existed.
fn read_let_scalar(chars: &[char], at: usize) -> Option<(String, ScalarValue, usize)> {
	let mut j = at + "#let ".chars().count();
	let name_start = j;
	while j < chars.len() && is_ident_char(chars[j]) {
		j += 1;
	}
	let name: String = chars[name_start..j].iter().collect();
	if name.is_empty() {
		return None;
	}
	// A scalar binding takes no parameter list; a `(` here (with no space, as a signature is written) is a
	// function, not a value.
	while j < chars.len() && chars[j].is_whitespace() {
		j += 1;
	}
	if chars.get(j) != Some(&'=') {
		return None;
	}
	j += 1;
	while j < chars.len() && chars[j].is_whitespace() {
		j += 1;
	}
	let line_end = chars[j..].iter().position(|&c| c == '\n').map_or(chars.len(), |p| j + p);
	let rest: String = chars[j..line_end].iter().collect();
	let value_text = strip_trailing_line_comment(rest.trim());
	let value = res_scalar_literal(value_text)?;
	Some((name, value, line_end))
}

/// Reads `text` (the right-hand side of a `#let`, comment-stripped and trimmed) as a scalar literal: a
/// `"..."` string, its quotes stripped, or an integer, float or length (`pt`/`mm`/`cm`/`in`) literal kept as
/// its own source text -- Typst renders a bare number or length exactly as written, so no reformatting is
/// needed. `None` for anything else, so an expression this reader cannot evaluate is left for the ordinary
/// `#let` skip rather than misread.
fn res_scalar_literal(text: &str) -> Option<ScalarValue> {
	if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
		return Some(ScalarValue::Str(text[1..text.len() - 1].to_string()));
	}
	for unit in ["pt", "mm", "cm", "in"] {
		if let Some(num) = text.strip_suffix(unit) {
			if !num.is_empty() && num.trim().parse::<f64>().is_ok() {
				return Some(ScalarValue::Number(text.to_string()));
			}
		}
	}
	if text.parse::<f64>().is_ok() {
		return Some(ScalarValue::Number(text.to_string()));
	}
	None
}

/// Strips a trailing `//` line comment from a scalar `#let`'s right-hand side (`#let n = 3 // words/min`),
/// so the literal reads correctly. A `//` inside the value's own `"..."` quotes is not a comment and is kept.
fn strip_trailing_line_comment(s: &str) -> &str {
	let mut in_str = false;
	let mut chars = s.char_indices().peekable();
	while let Some((idx, c)) = chars.next() {
		match c {
			'"'					=> in_str = !in_str,
			'/' if !in_str		=> if let Some(&(_, '/')) = chars.peek() {
				return s[..idx].trim_end();
			},
			_					=> {},
		}
	}
	s
}

/// Is `name` a construct the reader already captures specially, so a `#let` of that name must not shadow
/// the built-in path? These are exactly the names [`crate::lang::parse::capture_opener`] and the document
/// loop match on before the furniture arm.
fn is_reserved_construct(name: &str) -> bool {
	matches!(name,
		"styled-box" | "figure" | "table" | "columns" | "image" | "padded-image"
		| "section-banner" | "print-glossary" | "line" | "part-page"
		// Common Typst built-ins a corpus must not be able to redefine into a wrap the reader would expand.
		| "v" | "h" | "pagebreak" | "colbreak" | "place" | "lorem" | "outline" | "box" | "block" | "text" | "align"
		| "grid" | "stack")
}

/// Is `at` the start of a line (position 0, or just after a newline)?
fn at_line_start(chars: &[char], at: usize) -> bool {
	at == 0 || chars.get(at - 1) == Some(&'\n')
}

/// Does `chars` hold the literal `pat` starting at `at`?
fn starts_with_at(chars: &[char], at: usize, pat: &str) -> bool {
	let p: Vec<char> = pat.chars().collect();
	if at + p.len() > chars.len() {
		return false;
	}
	chars[at..at + p.len()] == p[..]
}

/// Reads a `#let name(params) = <expr>` beginning at `at` (the `#`), returning the name, the parameter
/// list text, the definition expression, and the index just past the expression. The expression runs to
/// the end of the top-level balanced group it opens (`block( ... )`, `box( ... )`, or a `{ ... }` body),
/// so a multi-line definition is read whole. `None` when the line is not a function `#let`.
fn read_let_fn(chars: &[char], at: usize) -> Option<(String, String, String, usize)> {
	let mut j = at + "#let ".chars().count();
	// The name: identifier characters up to the `(`.
	let name_start = j;
	while j < chars.len() && is_ident_char(chars[j]) {
		j += 1;
	}
	let name: String = chars[name_start..j].iter().collect();
	if name.is_empty() || chars.get(j) != Some(&'(') {
		return None;
	}
	// The parameter list `( ... )`.
	let (params, after_params) = read_paren_group(chars, j)?;
	// The `=` separating the signature from the body.
	let mut k = after_params;
	while k < chars.len() && chars[k].is_whitespace() {
		k += 1;
	}
	if chars.get(k) != Some(&'=') {
		return None;
	}
	k += 1;
	while k < chars.len() && chars[k].is_whitespace() {
		k += 1;
	}
	// The expression is the balanced group the body opens: a `block(`/`box(` call, or a `{ ... }` block.
	let (expr, next) = read_balanced_from(chars, k)?;
	Some((name, params, expr, next))
}

/// Reads the balanced `( ... )` group whose `(` sits at `open`, returning the inner text (without the
/// parentheses) and the index just past the `)`.
fn read_paren_group(chars: &[char], open: usize) -> Option<(String, usize)> {
	if chars.get(open) != Some(&'(') {
		return None;
	}
	read_delim_group(chars, open)
}

/// Reads the balanced group whose opener (`(`, `[` or `{`) sits at `open`, returning the inner text
/// (without the delimiters) and the index just past its close. Nesting and string literals are honoured.
fn read_delim_group(chars: &[char], open: usize) -> Option<(String, usize)> {
	if !matches!(chars.get(open), Some('(') | Some('[') | Some('{')) {
		return None;
	}
	let mut depth	= 0i32;
	let mut in_str	= false;
	let mut esc		= false;
	for i in open..chars.len() {
		let c = chars[i];
		if in_str {
			if esc				{ esc = false; }
			else if c == '\\'	{ esc = true; }
			else if c == '"'	{ in_str = false; }
			continue;
		}
		match c {
			'"'				=> in_str = true,
			'(' | '[' | '{'	=> depth += 1,
			')' | ']' | '}'	=> {
				depth -= 1;
				if depth == 0 {
					let inner: String = chars[open + 1..i].iter().collect();
					return Some((inner, i + 1));
				}
			},
			_				=> {},
		}
	}
	None
}

/// Reads the balanced group beginning at `from` -- a `name( ... )` call, a `( ... )`, a `[ ... ]` or a
/// `{ ... }` -- returning the whole group's text (delimiters included) and the index just past its close.
/// The group starts at the first `(`/`[`/`{` at or after `from` on the definition; leading identifier
/// characters (a call name like `block`) are kept in the returned text.
fn read_balanced_from(chars: &[char], from: usize) -> Option<(String, usize)> {
	// Skip a leading call name to its opening bracket, keeping the name in the span.
	let mut open = from;
	while open < chars.len() && (is_ident_char(chars[open]) || chars[open] == '.') {
		open += 1;
	}
	let opener = *chars.get(open)?;
	if !matches!(opener, '(' | '[' | '{') {
		return None;
	}
	let mut depth	= 0i32;
	let mut in_str	= false;
	let mut esc		= false;
	for i in open..chars.len() {
		let c = chars[i];
		if in_str {
			if esc				{ esc = false; }
			else if c == '\\'	{ esc = true; }
			else if c == '"'	{ in_str = false; }
			continue;
		}
		match c {
			'"'				=> in_str = true,
			'(' | '[' | '{'	=> depth += 1,
			')' | ']' | '}'	=> {
				depth -= 1;
				if depth == 0 {
					let span: String = chars[from..=i].iter().collect();
					return Some((span, i + 1));
				}
			},
			_				=> {},
		}
	}
	None
}

fn is_ident_char(c: char) -> bool {
	c.is_alphanumeric() || c == '-' || c == '_'
}

/// Lowers a furniture definition's parameter list and body expression to a [`TemplateFn`], resolving every
/// `em` length against `body_size`. The recognised body is a single `block(...)`/`box(...)` wrap (`pr-note`)
/// or a `{ ... }` block whose first `box(...)`/`block(...)` is that wrap (`aside-box`, which then re-wraps
/// it in a `figure(placement: auto)` this reader lowers in-flow). The named arguments read are `inset`
/// (scalar or a `(left:, right:, x:, y:, top:, bottom:)` dict), `above`/`below` (block margins folded into
/// the top/bottom pads), `fill`, `radius` and `stroke: (left: <w> + <colour>)`; the positional content
/// block's inner `set text(size:)` / `set par(spacing:, first-line-indent:)` become the body overlay.
/// `None` when the body is neither wrap, or a length will not resolve -- the call then stays a tallied skip.
fn lower_template_fn(params: &str, expr: &str, body_size: Sp, palette: &Palette) -> Option<TemplateFn> {
	let body_param	= body_param_name(params)?;
	let has_title	= param_names(params).iter().any(|p| p == "title");

	// The float wrapper: a body re-wrapped in `figure(placement: ...)` (the `#aside-box(float: true)` idiom,
	// `if float { figure(placement: auto, inner) } else { inner }`) is a float the driver defers, not a keep
	// box set in the flow. The placement is read from the `figure(...)` call; a body that never wraps in a
	// figure is not a float.
	let float = figure_placement_in(expr);

	// The wrap call: the definition's own `block(...)`/`box(...)`, taken directly when the body is that call,
	// or found as the first such call inside a `{ ... }` body (the `let inner = box(...)` idiom). A `box` and
	// a `block` lower alike -- both wrap the body in a padded frame.
	let wrap = wrap_call(expr)?;
	let args	= wrap_args(&wrap)?;

	let mut patch = ThemePatch::default();

	// The fill: a resolved colour washes the frame; an absent (or unresolved) fill leaves it transparent, so
	// a plain indented block draws no panel. `box` and `block` alike carry a fill only when the source names one.
	let fill = match named_value(&args, "fill") {
		Some(fv)	=> parse_colour_pal(&fv, palette).unwrap_or(Rgba::TRANSPARENT),
		None		=> Rgba::TRANSPARENT,
	};
	patch.callout.fill = Some(fill);

	// A `stroke: (left: <w> + <colour>)` -- the aside-box left rule. The width and colour are read from the
	// dict's `left:` entry; a stroke this reader cannot resolve leaves both unset (no rule drawn).
	if let Some(sv) = named_value(&args, "stroke") {
		if let Some((w, col)) = read_left_stroke(&sv, body_size, palette) {
			patch.callout.stroke_left_w		= Some(w);
			patch.callout.stroke_left_col	= Some(col);
		}
	}

	// The inset: a scalar pads every side, a dict names `left`/`right`/`x`/`y`/`top`/`bottom`. `x` sets both
	// horizontal pads, `y` both vertical; a side-specific key overrides.
	if let Some(iv) = named_value(&args, "inset") {
		let pads = read_inset_pads(&iv, body_size)?;
		patch.callout.inset_left	= pads.left;
		patch.callout.inset_right	= pads.right;
		patch.callout.inset_top		= pads.top;
		patch.callout.inset_bot		= pads.bottom;
	}
	// The block margins `above`/`below` fold into the top/bottom pads: with a transparent wash they read as
	// the block's own leading/trailing space, an honest first cut of Typst's block spacing model.
	if let Some(av) = named_value(&args, "above") {
		patch.callout.inset_top = Some(resolve_len(&av, body_size)?);
	}
	if let Some(bv) = named_value(&args, "below") {
		patch.callout.inset_bot = Some(resolve_len(&bv, body_size)?);
	}
	if let Some(rv) = named_value(&args, "radius") {
		patch.callout.radius = Some(resolve_len(&rv, body_size)?);
	}

	// The positional content block: its inner `set text`/`set par` overlay the body, and it must reference
	// the body parameter (the hole). A definition whose content never names the body is not a furniture wrap.
	let (_delim, content) = positional_content(&args)?;
	if !mentions_word(&content, &body_param) {
		return None;
	}
	// The body's own text size may be set two ways: an inner `set text(size:)` (pr-note's `{ ... }` block) or
	// a `text(size: <x>)[#body]` wrapper around the body (aside-box). Both are read, resolving `em` in the
	// inner-set chain against the size a preceding `set text` already fixed (so a `par(spacing: 0.55em)` after
	// `text(size: 0.88em)` tracks Typst, which resolves the em against the reduced size, not the outer one).
	read_inner_sets_em(&content, body_size, &mut patch);
	if patch.text.body_size.is_none() {
		if let Some(sz) = body_wrapper_size(&content, &body_param, body_size) {
			patch.text.body_size = Some(sz);
		}
	}

	// The title run's size (`text(size: 0.85em)[#title]`), read from the content so a title paragraph is set
	// at the same size the body is.
	let title_size = if has_title {
		title_text_size(&content, body_size)
	} else {
		None
	};

	Some(TemplateFn { body_param, has_title, title_size, patch, float })
}

/// Reads the placement of a `figure(placement: <p>, ...)` wrapper in a furniture definition's body, or
/// `None` when the body wraps its content in no figure. `auto`/`top` float to the top, `bottom` to the
/// foot -- the same mapping the `#figure` reader uses.
fn figure_placement_in(expr: &str) -> Option<FloatPlacement> {
	let at		= expr.find("figure(")?;
	let rest	= &expr[at + "figure(".len()..];
	let key		= rest.find("placement:")?;
	let after	= rest[key + "placement:".len()..].trim_start();
	// The value runs to the next comma or the close of the call.
	let end		= after.find(|c| c == ',' || c == ')').unwrap_or(after.len());
	match after[..end].trim() {
		"auto"		=> Some(FloatPlacement::Auto),
		"top"		=> Some(FloatPlacement::Top),
		"bottom"	=> Some(FloatPlacement::Bottom),
		_			=> None,
	}
}

/// Reads a `stroke: (left: <w> + <colour>)` dict into a width and colour, resolving `em` against `body_size`
/// and a `colours.<name>` against `palette`. Typst's `2pt + colours.yellow.darken(20%)` is a stroke whose
/// thickness is the length term and whose paint is the colour term. `None` when there is no `left:` entry or
/// its width/colour will not resolve, so no rule is drawn rather than a wrong one.
fn read_left_stroke(raw: &str, body_size: Sp, palette: &Palette) -> Option<(Sp, Rgba)> {
	let raw = raw.trim();
	// A dict `(left: ...)`, or a bare stroke applied to every side -- take the `left:` entry, else the whole.
	let spec = if raw.starts_with('(') {
		let inner = call_group(raw)?;
		named_value(&inner, "left")?
	} else {
		raw.to_string()
	};
	// The spec is `<length> + <colour>` (either order): the term that resolves to a length is the width, the
	// term that resolves to a colour is the paint.
	let mut width:	Option<Sp>		= None;
	let mut colour:	Option<Rgba>	= None;
	for term in spec.split('+') {
		let t = term.trim();
		if let Some(sp) = resolve_len(t, body_size) {
			width = Some(sp);
		} else if let Some(c) = parse_colour_pal(t, palette) {
			colour = Some(c);
		}
	}
	match (width, colour) {
		(Some(w), Some(c))	=> Some((w, c)),
		_					=> None,
	}
}

/// The size of a `text(size: <len>)[ ... body ... ]` wrapper around the body parameter -- the body text size
/// when it is set by wrapping rather than by an inner `#set text` (aside-box's `text(size: 0.85em)[#body]`).
/// `None` when no `text(...)` call whose content names the body carries a size.
fn body_wrapper_size(content: &str, body_param: &str, body_size: Sp) -> Option<Sp> {
	let chars: Vec<char> = content.chars().collect();
	let mut from = 0usize;
	while let Some(at) = find_call(&chars[from..], "text").map(|i| from + i) {
		let (call, next) = match read_balanced_from(&chars, at) {
			Some(r)	=> r,
			None	=> break,
		};
		// The `[ ... ]` content block follows the `( ... )` args (Typst's `text(...)[...]`): read it and test
		// whether it places the body parameter.
		let mut k = next;
		while k < chars.len() && chars[k].is_whitespace() {
			k += 1;
		}
		let places_body = chars.get(k) == Some(&'[')
			&& read_delim_group(&chars, k)
				.map(|(inner, _)| mentions_word(&inner, body_param))
				.unwrap_or(false);
		if places_body {
			if let Some(a) = wrap_args(&call) {
				if let Some(v) = named_value(&a, "size") {
					if let Some(sp) = resolve_len(&v, body_size) {
						return Some(sp);
					}
				}
			}
		}
		from = next;
	}
	None
}

/// The body parameter's name: the last positional (unnamed) parameter in the list, which is the content
/// the call supplies. A named parameter (`title: none`, `float: true`) is a keyword the call may set, not
/// the content hole. `None` when the list names no positional parameter.
fn body_param_name(params: &str) -> Option<String> {
	split_top_commas_str(params).into_iter().rev().find_map(|p| {
		let p = p.trim();
		if p.is_empty() || p.contains(':') {
			None
		} else if p.chars().all(is_ident_char) {
			Some(p.to_string())
		} else {
			None
		}
	})
}

/// Every parameter's name (the identifier before any `:` default), for detecting a `title:` keyword.
fn param_names(params: &str) -> Vec<String> {
	split_top_commas_str(params).into_iter().filter_map(|p| {
		let name = p.split(':').next().unwrap_or("").trim();
		if !name.is_empty() && name.chars().all(is_ident_char) {
			Some(name.to_string())
		} else {
			None
		}
	}).collect()
}

/// The `block(...)`/`box(...)` wrap call of a furniture body: the whole expression when it is that call, or
/// the first such call inside a `{ ... }` body. `None` when neither is present.
fn wrap_call(expr: &str) -> Option<String> {
	let e = expr.trim();
	if e.starts_with("block") || e.starts_with("box") {
		return Some(e.to_string());
	}
	// A `{ ... }` body (the `let inner = box(...)` idiom): find the first `block(`/`box(` call within.
	let chars: Vec<char> = e.chars().collect();
	for name in ["box", "block"] {
		if let Some(at) = find_call(&chars, name) {
			if let Some((span, _)) = read_balanced_from(&chars, at) {
				return Some(span);
			}
		}
	}
	None
}

/// The argument text of a `name( ... )` wrap call, without the enclosing parentheses.
fn wrap_args(wrap: &str) -> Option<String> {
	let chars:	Vec<char>	= wrap.chars().collect();
	let open = chars.iter().position(|&c| c == '(')?;
	read_paren_group(&chars, open).map(|(inner, _)| inner)
}

/// The index of a `name(` call in `chars`, at a word boundary so `box` is not found inside a longer word.
fn find_call(chars: &[char], name: &str) -> Option<usize> {
	let pat: Vec<char> = name.chars().collect();
	let n = pat.len();
	let mut i = 0usize;
	while i + n < chars.len() {
		if chars[i..i + n] == pat[..]
			&& (i == 0 || !is_ident_char(chars[i - 1]))
			&& chars.get(i + n) == Some(&'(')
		{
			return Some(i);
		}
		i += 1;
	}
	None
}

/// The four inset pads a furniture block names.
struct InsetPads {
	left:	Option<Sp>,
	right:	Option<Sp>,
	top:	Option<Sp>,
	bottom:	Option<Sp>,
}

/// Reads a furniture `inset:` value into its four pads, resolving `em` against `body_size`. A scalar
/// (`inset: 8pt`) pads every side; a dict (`inset: (x: 1em, y: 1em, bottom: 1.2em)` or
/// `(left: 1.2em, right: 0.6em)`) names sides -- `x` both horizontal, `y` both vertical, then a
/// side-specific key overrides. `None` when a named length will not resolve, so the call stays a skip.
fn read_inset_pads(raw: &str, body_size: Sp) -> Option<InsetPads> {
	let raw = raw.trim();
	let mut pads = InsetPads { left: None, right: None, top: None, bottom: None };
	if raw.starts_with('(') {
		let inner = call_group(raw)?;
		if let Some(v) = named_value(&inner, "x") {
			let sp = resolve_len(&v, body_size)?;
			pads.left = Some(sp);
			pads.right = Some(sp);
		}
		if let Some(v) = named_value(&inner, "y") {
			let sp = resolve_len(&v, body_size)?;
			pads.top = Some(sp);
			pads.bottom = Some(sp);
		}
		if let Some(v) = named_value(&inner, "left") {
			pads.left = Some(resolve_len(&v, body_size)?);
		}
		if let Some(v) = named_value(&inner, "right") {
			pads.right = Some(resolve_len(&v, body_size)?);
		}
		if let Some(v) = named_value(&inner, "top") {
			pads.top = Some(resolve_len(&v, body_size)?);
		}
		if let Some(v) = named_value(&inner, "bottom") {
			pads.bottom = Some(resolve_len(&v, body_size)?);
		}
		Some(pads)
	} else {
		let sp = resolve_len(raw, body_size)?;
		Some(InsetPads { left: Some(sp), right: Some(sp), top: Some(sp), bottom: Some(sp) })
	}
}

/// The first top-level `{ ... }` or `[ ... ]` content group in a wrap's argument list, as its delimiter and
/// inner text -- the block the body parameter sits in. `None` when the call carries no positional content.
fn positional_content(args: &str) -> Option<(char, String)> {
	let chars:	Vec<char>	= args.chars().collect();
	let mut depth	= 0i32;
	let mut in_str	= false;
	let mut esc		= false;
	let mut i		= 0usize;
	while i < chars.len() {
		let c = chars[i];
		if in_str {
			if esc				{ esc = false; }
			else if c == '\\'	{ esc = true; }
			else if c == '"'	{ in_str = false; }
			i += 1;
			continue;
		}
		match c {
			'"'					=> in_str = true,
			'(' 				=> depth += 1,
			')'					=> depth -= 1,
			'{' | '[' if depth == 0	=> {
				if let Some((span, _)) = read_delim_group(&chars, i) {
					return Some((c, span));
				}
				return None;
			},
			'{' | '['			=> depth += 1,
			'}' | ']'			=> depth -= 1,
			_					=> {},
		}
		i += 1;
	}
	None
}

/// Reads a content block's inner `set text(size:)` and `set par(spacing:, first-line-indent:)` into the
/// hole overlay, resolving `em` against `body_size`. A `#`-prefixed `#set` (an `[ ... ]` content block) and
/// a bare `set` (a `{ ... }` code block) are both read.
fn read_inner_sets_em(content: &str, body_size: Sp, patch: &mut ThemePatch) {
	// The text size in force as the statements are read in order. Typst resolves an `em` against the running
	// font size, so a `set par(spacing: 0.55em)` AFTER a `set text(size: 0.88em)` resolves its em against the
	// reduced 0.88em size, not the outer body -- tracking that is what keeps the inter-paragraph spacing tight.
	let mut cur_size = body_size;
	for stmt in split_statements(content) {
		let s = stmt.trim().trim_start_matches('#').trim();
		let after = match s.strip_prefix("set ") {
			Some(a)	=> a.trim(),
			None	=> continue,
		};
		let open = match after.find('(') {
			Some(i)	=> i,
			None	=> continue,
		};
		let target	= after[..open].trim();
		let cargs	= match call_group(&after[open..]) {
			Some(a)	=> a,
			None	=> continue,
		};
		match target {
			"text" => {
				// `set text(size: 0.88em)`: the em resolves against the size before this set (the running
				// `cur_size`), and the result becomes the running size for every em that follows.
				if let Some(v) = named_value(&cargs, "size") {
					if let Some(sp) = resolve_len(&v, cur_size) {
						patch.text.body_size = Some(sp);
						cur_size = sp;
					}
				}
			},
			"par" => {
				if let Some(v) = named_value(&cargs, "spacing") {
					if let Some(sp) = resolve_len(&v, cur_size) {
						patch.par.skip = Some(sp);
					}
				}
				if let Some(v) = named_value(&cargs, "first-line-indent") {
					if let Some(sp) = resolve_len(&v, cur_size) {
						patch.par.indent = Some(sp);
					}
				}
				if let Some(v) = named_value(&cargs, "leading") {
					if let Some(sp) = resolve_len(&v, cur_size) {
						patch.text.leading = Some(sp);
					}
				}
			},
			_ => {},
		}
	}
}

/// The size of a `text(size: <len>)[#title]` run inside a furniture's content -- the size a leading title
/// paragraph is set at. `None` when no such run names a size.
fn title_text_size(content: &str, body_size: Sp) -> Option<Sp> {
	// The title run carries `weight: "bold"`, so match the first `text(...)` naming a bold weight and a size.
	let chars: Vec<char> = content.chars().collect();
	let mut from = 0usize;
	while let Some(at) = find_call(&chars[from..], "text").map(|i| from + i) {
		if let Some((_, next)) = read_balanced_from(&chars, at) {
			let call: String = chars[at..next].iter().collect();
			if let Some(a) = wrap_args(&call) {
				if a.contains("bold") {
					if let Some(v) = named_value(&a, "size") {
						if let Some(sp) = resolve_len(&v, body_size) {
							return Some(sp);
						}
					}
				}
			}
			from = next;
		} else {
			break;
		}
	}
	None
}

/// A length token to scaled points, resolving `em` against `body_size` (an em is that fraction of the
/// running text size). Accepts `em`, and every absolute unit [`length_pt`] reads (`pt`/`mm`/`cm`/`in`/bare).
/// `None` for a `%` or an unrecognised unit, so a caller refuses rather than sizing wrongly.
fn resolve_len(v: &str, body_size: Sp) -> Option<Sp> {
	let v = v.trim();
	if let Some(num) = v.strip_suffix("em") {
		let n: f64 = num.trim().parse().ok()?;
		return Some(Sp::from_pt(n * body_size.to_pt()));
	}
	length_pt(v).map(Sp::from_pt)
}

/// Splits a parameter or dict text on top-level commas (outside any `(...)`/`[...]`/`{...}`/`"..."`).
fn split_top_commas_str(s: &str) -> Vec<String> {
	let mut out		= Vec::new();
	let mut depth	= 0i32;
	let mut in_str	= false;
	let mut esc		= false;
	let mut cur		= String::new();
	for c in s.chars() {
		if in_str {
			cur.push(c);
			if esc				{ esc = false; }
			else if c == '\\'	{ esc = true; }
			else if c == '"'	{ in_str = false; }
			continue;
		}
		match c {
			'"'					=> { in_str = true; cur.push(c); },
			'(' | '[' | '{'		=> { depth += 1; cur.push(c); },
			')' | ']' | '}'		=> { depth -= 1; cur.push(c); },
			',' if depth == 0	=> out.push(std::mem::take(&mut cur)),
			_					=> cur.push(c),
		}
	}
	if !cur.trim().is_empty() {
		out.push(cur);
	}
	out
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ MATCHING                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// Does `selector` match `block`? The kind must answer to the block's variant and every predicate must
/// hold. A predicate a block has no field for (a `block:` on a heading) never matches, so a mis-targeted
/// rule styles nothing rather than everything.
pub fn matches(selector: &Selector, block: &Block) -> bool {
	let kind_ok = match (selector.kind, block) {
		(ElementKind::Heading, Block::Heading { .. })			=> true,
		(ElementKind::Figure, Block::Figure { .. })
		| (ElementKind::Figure, Block::TableFigure { .. })
		| (ElementKind::Figure, Block::ImageFigure { .. })
		| (ElementKind::Figure, Block::CodeFigure { .. })		=> true,
		(ElementKind::FigureCaption, Block::Figure { .. })
		| (ElementKind::FigureCaption, Block::TableFigure { .. })
		| (ElementKind::FigureCaption, Block::ImageFigure { .. })
		| (ElementKind::FigureCaption, Block::CodeFigure { .. })	=> true,
		(ElementKind::Raw, Block::Code { .. })					=> true,
		(ElementKind::Equation, Block::Equation { .. })			=> true,
		(ElementKind::Paragraph, Block::Paragraph { .. })
		| (ElementKind::Paragraph, Block::RichParagraph { .. })	=> true,
		(ElementKind::List, Block::List { .. })					=> true,
		(ElementKind::Table, Block::Table(_))					=> true,
		// A `link` selector is an inline element, carried in no block of its own, so it matches no block
		// here and its rule styles nothing this part -- an inline-rule part's work.
		_													=> false,
	};
	if !kind_ok {
		return false;
	}
	selector.predicates.iter().all(|p| predicate_holds(p, block))
}

/// Does one field predicate hold for `block`?
fn predicate_holds(p: &FieldPredicate, block: &Block) -> bool {
	match (p, block) {
		(FieldPredicate::Level(n), Block::Heading { level, .. })	=> level == n,
		// A verbatim code block is always block-level in Austenite (no inline `raw` reaches the block layer),
		// so `raw.where(block: true)` matches it and `block: false` never does.
		(FieldPredicate::Block(b), Block::Code { .. })			=> *b,
		_													=> false,
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ APPLYING RULES                                                             │
// └───────────────────────────────────────────────────────────────────────────┘

/// Applies `rules` over the block tree, wrapping each block a rule matches in a [`Block::Scoped`] carrying
/// the rule's set-fields patch -- the general mechanism a scoped `#set` already uses, so the styling is
/// confined to the matched element and every document-order pass scopes it for free. Rules are tried in
/// order and a block matched by several is wrapped in nested scopes, the last rule innermost so it wins;
/// a [`Transform::Refused`] never wraps (it was recorded at collection). An empty patch never wraps, so a
/// rule that changes nothing leaves the block untouched and the render byte-identical.
///
/// Recurses into existing [`Block::Scoped`] and [`Block::Box`] subtrees first, so a rule reaches an
/// element already inside a scope (an included chapter's own) as well as one at top level.
///
/// `avail` is the content width in force at placement (`geom.content_width()`), threaded so a template's
/// relative divider (a `line(length: 100%)`) resolves to an absolute width against the measure it will set
/// at, rather than being left to guess one.
pub fn apply_rules(blocks: &mut Vec<Block>, rules: &[Rule], avail: Sp) {
	// Descend into existing nesting subtrees first, so their own elements are matched too.
	for b in blocks.iter_mut() {
		if let Block::Scoped { blocks: inner, .. } | Block::Box { blocks: inner, .. } | Block::Place { blocks: inner, .. } = b {
			apply_rules(inner, rules, avail);
		}
	}
	// Then wrap each block this slice holds that a rule matches.
	let taken = std::mem::take(blocks);
	let mut out = Vec::with_capacity(taken.len());
	for block in taken {
		out.push(wrap_matching(block, rules, avail));
	}
	*blocks = out;
}

/// Wraps `block` in one [`Block::Scoped`] per set-fields rule that matches it, the last matching rule
/// innermost so it overrides the earlier ones; a rule whose patch is empty adds no scope. A matching
/// template then restructures the (possibly already-scoped) block into its `Scoped{hole, [pre.., it, post..]}`
/// shape, the last matching template winning. A block no rule matches is returned unchanged.
fn wrap_matching(block: Block, rules: &[Rule], avail: Sp) -> Block {
	// The matching set-fields patches in order, so the fold wraps them last-innermost below.
	let mut patches:	Vec<&ThemePatch>	= Vec::new();
	// The last matching template, applied outermost of the set-fields scopes since it restructures the block.
	let mut template:	Option<&Template>	= None;
	for rule in rules {
		match &rule.transform {
			Transform::SetFields(p)	=> {
				if *p != ThemePatch::default() && matches(&rule.selector, &block) {
					patches.push(p);
				}
			},
			Transform::Template(t)	=> {
				if matches(&rule.selector, &block) {
					template = Some(t);
				}
			},
			Transform::Refused(_)	=> {},
		}
	}
	let mut wrapped = block;
	for p in patches.into_iter().rev() {
		wrapped = Block::Scoped { patch: p.clone(), blocks: vec![wrapped] };
	}
	if let Some(t) = template {
		wrapped = materialise_template(t, wrapped, avail);
	}
	wrapped
}

/// Materialises a template around the matched element: the element is *moved* into the hole between the
/// template's `pre` and `post` siblings (wrapped in a washed [`Block::Box`] when the template frames it), and
/// the whole sequence is overlaid with the hole patch through a [`Block::Scoped`]. A relative divider width
/// in a sibling resolves to an absolute against `avail` here, at the placement measure.
fn materialise_template(t: &Template, it: Block, avail: Sp) -> Block {
	let hole_block = match t.frame {
		Some(tf)	=> {
			// The element seated in a box washed the template's fill, plus whichever of inset/radius the
			// rule named -- the renderer reads all five from `callout.*` on the box's own scoped theme,
			// falling back to its own constants for whatever the rule left `None`.
			let mut patch = ThemePatch::default();
			patch.callout.fill			= Some(tf.fill);
			patch.callout.inset_x		= tf.inset_x;
			patch.callout.inset_top		= tf.inset_top;
			patch.callout.inset_bot		= tf.inset_bot;
			patch.callout.radius		= tf.radius;
			Block::Box { blocks: vec![it], patch, placement: None }
		},
		None		=> it,
	};
	let mut seq = Vec::with_capacity(t.pre.len() + 1 + t.post.len());
	for b in &t.pre {
		seq.push(resolve_avail(b.clone(), avail));
	}
	seq.push(hole_block);
	for b in &t.post {
		seq.push(resolve_avail(b.clone(), avail));
	}
	Block::Scoped { patch: t.hole.clone(), blocks: seq }
}

/// Resolves a template sibling's relative width against the placement `avail`: a `Block::Rule` whose width is
/// a fraction ([`Length::Rel`], from a `line(length: 100%)`) becomes an absolute ([`Length::Abs`]) at that
/// measure, so its extent is fixed where it will set rather than left relative. Every other block is unchanged.
fn resolve_avail(block: Block, avail: Sp) -> Block {
	match block {
		Block::Rule { width: Length::Rel(f), thickness, grey }	=>
			Block::Rule { width: Length::Abs(avail.to_pt() * f), thickness, grey },
		other	=> other,
	}
}

/// The default rule set followed by a source's own rules, ready to apply. The default set uses `theme`
/// (the document theme in force) to re-assert each heading level's own size; the source's rules are
/// appended after, so an authored rule overrides the default for the elements it matches.
pub fn rule_set_for(theme: &Theme, src: &str, refusals: &mut Refusals) -> Vec<Rule> {
	let mut rules = default_rule_set(theme);
	let own = collect_from_source(src, rules.len(), refusals);
	rules.extend(own);
	rules
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::doc::Segment;

	fn heading(level: u8) -> Block {
		Block::Heading { level, segments: vec![Segment::text("H")], label: None }
	}

	/// A `heading.where(level: 1)` selector parses to the heading kind with a single level predicate, and
	/// matches a level-1 heading but not a level-2 one.
	#[test]
	fn heading_where_level_parses_and_matches() {
		let sel = parse_selector("heading.where(level: 1)").expect("selector parses");
		assert_eq!(sel.kind, ElementKind::Heading);
		assert_eq!(sel.predicates, vec![FieldPredicate::Level(1)]);
		assert!(matches(&sel, &heading(1)));
		assert!(!matches(&sel, &heading(2)));
	}

	/// The other selector forms the grammar reads parse to their kinds: a figure caption, a block raw, a
	/// bare link, and a math equation.
	#[test]
	fn selector_forms_parse() {
		assert_eq!(parse_selector("figure.caption").unwrap().kind, ElementKind::FigureCaption);
		assert_eq!(parse_selector("link").unwrap().kind, ElementKind::Link);
		let raw = parse_selector("raw.where(block: true)").unwrap();
		assert_eq!(raw.kind, ElementKind::Raw);
		assert_eq!(raw.predicates, vec![FieldPredicate::Block(true)]);
		assert!(parse_selector("nonesuch").is_none());
	}

	/// A `set text(size: 30pt)` transform lowers to a set-fields patch. Under a heading selector the size is
	/// redirected into the matched level's own heading size -- the field the renderer reads for a heading --
	/// and does not touch `text.body_size`; under a body selector it stays `text.body_size`. A page-reading
	/// transform, a `text` field a heading never renders, and a wrap transform are all refused.
	#[test]
	fn transform_lowers_or_refuses() {
		let h1	= Selector { kind: ElementKind::Heading, predicates: vec![FieldPredicate::Level(1)] };
		let par	= Selector { kind: ElementKind::Paragraph, predicates: vec![] };
		// Under a heading selector, the text size redirects into the matched level's heading size.
		match lower_transform(&h1, "set text(size: 30pt)") {
			Transform::SetFields(p)	=> {
				assert_eq!(p.heading.levels.first().and_then(|l| l.size), Some(crate::ir::Sp::from_pt(30.0)));
				assert!(p.text.body_size.is_none(), "a heading size rule must not write text.body_size");
			},
			other					=> panic!("a heading size rule should lower to set-fields, got {:?}", other),
		}
		// Under a body selector, the same set stays a body-size patch.
		match lower_transform(&par, "set text(size: 30pt)") {
			Transform::SetFields(p)	=> assert_eq!(p.text.body_size, Some(crate::ir::Sp::from_pt(30.0))),
			other					=> panic!("a body size rule should lower to set-fields, got {:?}", other),
		}
		assert!(matches!(lower_transform(&h1, "it => context measure(it)"), Transform::Refused(_)),
			"a page-reading transform must be refused");
		// A `text` field a heading never renders is refused under a heading selector.
		assert!(matches!(lower_transform(&h1, "set text(tracking: 0.1em)"), Transform::Refused(_)),
			"an unread field must be refused");
		assert!(matches!(lower_transform(&h1, "it => underline(it)"), Transform::Refused(_)),
			"a wrap transform is a later part, refused here");
	}

	/// `collect_from_source` reads a per-element `#show` rule and leaves the whole-document `#show:`
	/// application alone.
	#[test]
	fn collect_reads_show_rules_only() {
		let src = "#show: doc.with(title: [X])\n#show heading.where(level: 1): set text(size: 30pt)\n";
		let mut refusals = Refusals::default();
		let rules = collect_from_source(src, 0, &mut refusals);
		assert_eq!(rules.len(), 1, "only the per-element rule is a rule; #show: is the doc application");
		assert_eq!(rules[0].selector.kind, ElementKind::Heading);
		assert!(matches!(rules[0].transform, Transform::SetFields(_)));
	}

	/// A page-reading rule is collected as a refusal, not applied, and its reason recorded.
	#[test]
	fn page_reading_rule_is_refused() {
		let src = "#show heading: it => context counter.at(here())\n";
		let mut refusals = Refusals::default();
		let rules = collect_from_source(src, 0, &mut refusals);
		assert_eq!(rules.len(), 1);
		assert!(matches!(rules[0].transform, Transform::Refused(_)));
		assert!(!refusals.is_empty(), "the refusal is recorded for the report");
	}

	/// A `#show heading.where(level: 1): set text(size: 30pt)` rule actually resizes the level-1 heading's
	/// rendered glyphs and nothing else: the rendered level-1 heading grows to exactly the size a theme that
	/// set level-1 to 30pt directly produces (positive), while the level-2 and level-3 headings and the body
	/// text set beside the resized heading keep their sizes (negative). Rendered through `author`, so the
	/// assertion is on the shaped output, not on the patch -- without the heading-size redirect this fails,
	/// since a `text.body_size` patch never reaches a heading's `heading_size(level)` glyph size.
	#[test]
	fn heading_size_rule_resizes_only_the_matched_level() -> Outcome<()> {
		use std::sync::Arc;
		use crate::doc::{author, HeadingStyle};
		use crate::ir::{Node, Sp};

		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		let geom	= crate::page::PageGeometry::a4();
		let faces	= crate::fonts::FaceResolver::default();

		// DocInline sets every level as an inline sub-heading (no chapter-opener page), so each heading's
		// glyph size reads `heading_size(level)` through `subheading_hbox` -- the arm the rule must reach.
		let mut theme = Theme::default();
		theme.heading.kind = HeadingStyle::DocInline;

		let blocks = || vec![
			heading(1),
			Block::Paragraph { text: "Body after one.".to_string() },
			heading(2),
			Block::Paragraph { text: "Body after two.".to_string() },
			heading(3),
			Block::Paragraph { text: "Body after three.".to_string() },
		];

		// One render's heading keep boxes, as `(heading-line height, joined body-line height)` pairs in
		// document order: the first HBox inside each heading VBox is the shaped heading line (its height
		// scaling with the glyph size), the second is the following paragraph's first line, pulled into the
		// heading's keep box -- so a body line set right beside the resized heading is measured too.
		let render = |base: &Theme, rules_src: &str| -> Outcome<Vec<(i32, Option<i32>)>> {
			let mut refusals	= Refusals::default();
			let rules			= rule_set_for(base, rules_src, &mut refusals);
			let mut bs			= blocks();
			apply_rules(&mut bs, &rules, geom.content_width());
			let (doc, _)		= res!(author(fonts.clone(), geom, base, &faces, &bs, None, None));
			let mut out = Vec::new();
			for n in &doc.nodes {
				if let Node::VBox(b) = n {
					let hs: Vec<i32> = b.list.iter().filter_map(|c| match c {
						Node::HBox(h)	=> Some(h.dims.height.raw()),
						_				=> None,
					}).collect();
					if let Some(&first) = hs.first() {
						out.push((first, hs.get(1).copied()));
					}
				}
			}
			Ok(out)
		};

		let base	= res!(render(&theme, ""));
		let ruled	= res!(render(&theme, "#show heading.where(level: 1): set text(size: 30pt)\n"));
		// The independent oracle: a theme that sets level-1's size to 30pt directly, no authored rule.
		let mut theme30	= theme.clone();
		theme30.heading.levels[0].size = Sp::from_pt(30.0);
		let direct	= res!(render(&theme30, ""));

		assert_eq!(base.len(), 3, "three headings render");
		assert_eq!(ruled.len(), 3);
		assert_eq!(direct.len(), 3);

		// Positive: the rule enlarges the level-1 heading, to exactly the size a direct 30pt theme sets.
		assert!(ruled[0].0 > base[0].0, "the level-1 heading must grow under the 30pt rule");
		assert_eq!(ruled[0].0, direct[0].0,
			"the rule must resize the level-1 heading to the same glyphs as a direct 30pt theme");

		// Negative: levels 2 and 3 keep their sizes; a level-1 rule touches no other level.
		assert_eq!(ruled[1].0, base[1].0, "the level-2 heading must be unchanged");
		assert_eq!(ruled[2].0, base[2].0, "the level-3 heading must be unchanged");

		// Negative: the body lines -- including the one pulled into the resized level-1 heading's keep box --
		// keep the body size; the heading rule must not bleed into running text.
		assert_eq!(ruled.iter().map(|p| p.1).collect::<Vec<_>>(),
			base.iter().map(|p| p.1).collect::<Vec<_>>(),
			"the body text beside every heading must keep its size");
		Ok(())
	}

	/// An authored rule changes only the element it targets, at render time: a level-1 heading numbering
	/// rule renumbers the level-1 heading while a level-2 heading, outside the rule's scope, keeps the
	/// default dotted number. (A heading *size* rule likewise reaches only its target's glyphs, tested
	/// through the rendered output in `heading_size_rule_resizes_only_the_matched_level`.)
	#[test]
	fn authored_rule_renumbers_only_its_target() -> Outcome<()> {
		use std::sync::Arc;
		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		let geom	= crate::page::PageGeometry::a4();
		let style	= Theme::default();	// BookOpener: a heading carries a rendered number
		let mut refusals = Refusals::default();
		let rules = collect_from_source(
			"#show heading.where(level: 1): set heading(numbering: \"A\")\n", 0, &mut refusals);
		let mut blocks = vec![heading(1), heading(2)];
		apply_rules(&mut blocks, &rules, geom.content_width());
		let (_, heads) = res!(crate::doc::author(
			fonts, geom, &style, &crate::fonts::FaceResolver::default(), &blocks, None, None));
		assert_eq!(heads[0].number, "A", "the level-1 rule renumbers only the level-1 heading");
		assert_eq!(heads[1].number, "1.1", "the level-2 heading, outside the rule, keeps the default number");
		Ok(())
	}

	/// A `#show par: set text(size: 9pt)` rule wraps the paragraph the heading keeps with in a single-block
	/// `Scoped`, and `walk`'s lookahead descends that scope: the heading still keeps its following line (the
	/// keep box holds two HBoxes, not a stranded one), and the kept line is set at the scoped 9pt -- exactly
	/// the height a theme whose body size is 9pt directly produces (positive). Without the rule the keep box
	/// is unchanged (negative). Guards the scope-transparent keep-with-next fix (L1-c1); before it, the
	/// `Scoped`-wrapped paragraph was invisible to the lookahead and the heading stranded.
	#[test]
	fn par_rule_keeps_with_heading_through_its_scope() -> Outcome<()> {
		use std::sync::Arc;
		use crate::doc::{author, HeadingStyle};
		use crate::ir::{Node, Sp};

		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		let geom	= crate::page::PageGeometry::a4();
		let faces	= crate::fonts::FaceResolver::default();

		// DocInline sets the heading as an inline sub-heading that keeps with its following paragraph.
		let mut theme = Theme::default();
		theme.heading.kind = HeadingStyle::DocInline;

		let blocks = || vec![
			heading(2),
			Block::Paragraph { text: "Body after the heading, long enough to keep on its own line.".to_string() },
		];

		// The heading's keep VBox as its list of HBox heights: HBox[0] is the shaped heading line, HBox[1] is
		// the following paragraph's first line, pulled into the keep box.
		let keep_hboxes = |base: &Theme, rules_src: &str| -> Outcome<Vec<i32>> {
			let mut refusals	= Refusals::default();
			let rules			= rule_set_for(base, rules_src, &mut refusals);
			let mut bs			= blocks();
			apply_rules(&mut bs, &rules, geom.content_width());
			let (doc, _)		= res!(author(fonts.clone(), geom, base, &faces, &bs, None, None));
			for n in &doc.nodes {
				if let Node::VBox(b) = n {
					return Ok(b.list.iter().filter_map(|c| match c {
						Node::HBox(h)	=> Some(h.dims.height.raw()),
						_				=> None,
					}).collect());
				}
			}
			Ok(Vec::new())
		};

		let base	= res!(keep_hboxes(&theme, ""));
		let ruled	= res!(keep_hboxes(&theme, "#show par: set text(size: 9pt)\n"));
		// The independent oracle: a theme whose body size is 9pt directly, no authored rule.
		let mut theme9	= theme.clone();
		theme9.text.body_size = Sp::from_pt(9.0);
		let direct	= res!(keep_hboxes(&theme9, ""));

		// The keep box holds two HBoxes in every case: the heading line and the body line it kept with.
		assert_eq!(base.len(), 2, "the heading must keep its following body line: {:?}", base);
		assert_eq!(ruled.len(), 2, "the par rule must not strand the heading -- the scope is seen through: {:?}", ruled);
		assert_eq!(direct.len(), 2, "the direct 9pt oracle keeps likewise: {:?}", direct);

		// Positive: the kept body line takes the rule's 9pt, matching a direct 9pt body theme exactly.
		assert_eq!(ruled[1], direct[1],
			"the kept body line must take the scoped 9pt, the same height a direct 9pt theme sets");
		assert!(ruled[1] < base[1], "the 9pt rule must shrink the kept body line below the default");
		// Negative: the heading line itself is untouched by a par rule.
		assert_eq!(ruled[0], base[0], "a par rule must not change the heading line");
		Ok(())
	}

	fn raw_selector() -> Selector { Selector { kind: ElementKind::Raw, predicates: vec![] } }

	/// A `#show raw: block.with(fill: ..., inset: ..., radius: ...)` lowers to a template that frames the
	/// element, and applying it seats the code block -- moved, not cloned -- inside a `Block::Box` washed the
	/// named fill, under a transparent hole scope. The fill, inset and radius are each the value the
	/// template named -- the consume half of the invariant this rule form used to fail (fill was kept,
	/// inset/radius silently dropped).
	#[test]
	fn template_on_raw_frames_the_code() {
		let sel	= raw_selector();
		let tr	= lower_transform(&sel, "it => block.with(fill: luma(240), inset: 8pt, radius: 10pt)");
		let t	= match tr {
			Transform::Template(t)	=> t,
			other					=> panic!("expected a template, got {:?}", other),
		};
		assert_eq!(t.frame, Some(TemplateFrame {
			fill:		Rgba::opaque(240, 240, 240),
			inset_x:	Some(Sp::from_pt(8.0)),
			inset_top:	Some(Sp::from_pt(8.0)),
			inset_bot:	Some(Sp::from_pt(8.0)),
			radius:		Some(Sp::from_pt(10.0)),
		}), "the block.with fill, inset and radius all become the frame");
		assert!(t.pre.is_empty() && t.post.is_empty(), "a bare frame has no siblings");
		assert_eq!(t.hole, ThemePatch::default(), "no #set inside, so the hole is transparent");

		let rule = Rule {
			selector:	sel,
			transform:	Transform::Template(t),
			rule_id:	7,
			source:		"#show raw".to_string(),
			span:		Span::new(0, 0),
		};
		let mut blocks = vec![Block::Code { lines: vec!["let x = 1;".to_string()] }];
		apply_rules(&mut blocks, std::slice::from_ref(&rule), Sp::from_pt(400.0));
		match &blocks[0] {
			Block::Scoped { patch, blocks: inner } => {
				assert_eq!(patch, &ThemePatch::default(), "the hole scope is transparent");
				match &inner[0] {
					Block::Box { blocks: bb, patch, .. }	=> {
						assert_eq!(bb.len(), 1);
						assert!(matches!(bb[0], Block::Code { .. }), "the code is moved into the box");
						assert_eq!(patch.callout.fill, Some(Rgba::opaque(240, 240, 240)),
							"the box wash is the template's fill");
						assert_eq!(patch.callout.inset_x, Some(Sp::from_pt(8.0)), "the box carries the named inset x");
						assert_eq!(patch.callout.inset_top, Some(Sp::from_pt(8.0)), "the box carries the named inset y");
						assert_eq!(patch.callout.inset_bot, Some(Sp::from_pt(8.0)), "the box carries the named inset bottom");
						assert_eq!(patch.callout.radius, Some(Sp::from_pt(10.0)), "the box carries the named radius");
					},
					other	=> panic!("expected the code framed in a Box, got {:?}", other),
				}
			},
			other	=> panic!("expected a Scoped group, got {:?}", other),
		}
	}

	/// A `block.with(fill: ...)` with no `inset`/`radius` frames the element but names no geometry override
	/// -- the byte-identical path a bare `#styled-box[...]` (and this same rule form before it named any
	/// geometry) must keep, with the renderer's own constants left to apply downstream.
	#[test]
	fn template_frame_with_no_geometry_overrides_nothing() {
		let sel	= raw_selector();
		let t	= match lower_transform(&sel, "it => block.with(fill: luma(240))") {
			Transform::Template(t)	=> t,
			other					=> panic!("expected a template, got {:?}", other),
		};
		assert_eq!(t.frame, Some(TemplateFrame {
			fill:		Rgba::opaque(240, 240, 240),
			inset_x:	None,
			inset_top:	None,
			inset_bot:	None,
			radius:		None,
		}), "no inset/radius named, so the frame carries no geometry override");

		let rule = Rule {
			selector:	sel,
			transform:	Transform::Template(t),
			rule_id:	0,
			source:		"#show raw".to_string(),
			span:		Span::new(0, 0),
		};
		let mut blocks = vec![Block::Code { lines: vec!["let x = 1;".to_string()] }];
		apply_rules(&mut blocks, std::slice::from_ref(&rule), Sp::from_pt(400.0));
		match &blocks[0] {
			Block::Scoped { blocks: inner, .. } => match &inner[0] {
				Block::Box { patch, .. }	=> {
					assert_eq!(patch.callout.inset_x, None, "no inset override -- the renderer's own default applies");
					assert_eq!(patch.callout.inset_top, None);
					assert_eq!(patch.callout.inset_bot, None);
					assert_eq!(patch.callout.radius, None, "no radius override -- the renderer's own default applies");
				},
				other	=> panic!("expected the code framed in a Box, got {:?}", other),
			},
			other	=> panic!("expected a Scoped group, got {:?}", other),
		}
	}

	/// The `inset: (x:, y:, bottom:)` dict form the corpus uses: `x` and `y` set the horizontal and top pad,
	/// `y` also sets the bottom pad by default, and a following `bottom` overrides just that one side.
	#[test]
	fn template_inset_dict_form() {
		let sel	= raw_selector();
		let t	= match lower_transform(&sel, "it => block.with(fill: luma(240), inset: (x: 8pt, y: 6pt, bottom: 12pt))") {
			Transform::Template(t)	=> t,
			other					=> panic!("expected a template, got {:?}", other),
		};
		let tf = t.frame.expect("a fill names a frame");
		assert_eq!(tf.inset_x, Some(Sp::from_pt(8.0)), "the dict's x becomes inset_x");
		assert_eq!(tf.inset_top, Some(Sp::from_pt(6.0)), "the dict's y becomes inset_top");
		assert_eq!(tf.inset_bot, Some(Sp::from_pt(12.0)), "a following bottom overrides y for the foot pad");
	}

	/// A rule's `inset`/`radius` that this reader cannot resolve to points -- an `em` value, which has no
	/// absolute size at lowering time -- is refused, naming the field, rather than silently framing the
	/// element with its geometry dropped.
	#[test]
	fn template_frame_geometry_refuses_unresolvable_lengths() {
		let sel	= raw_selector();
		match lower_transform(&sel, "it => block.with(fill: luma(240), inset: 1em)") {
			Transform::Refused(reason)	=> assert!(reason.contains("inset"),
				"the refusal must name the field it could not resolve: {}", reason),
			other						=> panic!("expected a refusal, got {:?}", other),
		}
		match lower_transform(&sel, "it => block.with(fill: luma(240), radius: 50%)") {
			Transform::Refused(reason)	=> assert!(reason.contains("radius"),
				"the refusal must name the field it could not resolve: {}", reason),
			other						=> panic!("expected a refusal, got {:?}", other),
		}
	}

	/// A `#show heading.where(level: N): it => {{ v(a); it; v(b) }}` redirects the `v(...)` spacers into the
	/// matched level's own `space_above`/`space_below` rather than sibling blocks -- a sibling after a heading
	/// would break its keep-with-next -- so the template carries no `pre`/`post` and the hole patch names the
	/// level's spacing.
	#[test]
	fn heading_v_template_redirects_into_level_spacing() {
		let sel	= Selector { kind: ElementKind::Heading, predicates: vec![FieldPredicate::Level(2)] };
		let t	= match lower_transform(&sel, "it => { v(12pt); it; v(6pt) }") {
			Transform::Template(t)	=> t,
			other					=> panic!("expected a template, got {:?}", other),
		};
		assert!(t.pre.is_empty(), "a heading v() must not become a leading sibling block");
		assert!(t.post.is_empty(), "a heading v() must not become a trailing sibling block");
		assert!(t.frame.is_none());
		let lvl = t.hole.heading.levels.get(1).expect("level 2 -> index 1 is present");
		assert_eq!(lvl.space_above, Some(Sp::from_pt(12.0)), "the leading v() lifts space above the level");
		assert_eq!(lvl.space_below, Some(Sp::from_pt(6.0)), "the trailing v() sets space below the level");
	}

	/// The template reader refuses the bodies it cannot place as blocks: an inline `underline` wrap, a `regex`
	/// text rewrite, a page-reading `context` body, and -- under a heading selector -- any post sibling other
	/// than a `v()`, which would strand the heading from the content it keeps with.
	#[test]
	fn template_refusals() {
		let raw	= raw_selector();
		let h1	= Selector { kind: ElementKind::Heading, predicates: vec![FieldPredicate::Level(1)] };
		assert!(matches!(lower_transform(&raw, "it => underline(it)"), Transform::Refused(_)),
			"underline is an inline wrap, not a block template");
		assert!(matches!(lower_transform(&raw, "it => it.text.replace(regex(\"x\"), \"y\")"), Transform::Refused(_)),
			"a regex show rewrites text, not an element");
		assert!(matches!(lower_transform(&raw, "it => context { it }"), Transform::Refused(_)),
			"a page-reading context body has no lowering");
		match lower_transform(&h1, "it => { it; line(length: 100%) }") {
			Transform::Refused(reason)	=> assert!(reason.contains("keep-with-next"),
				"a post block after a heading must be refused for keep-with-next, got: {}", reason),
			other						=> panic!("expected a refusal, got {:?}", other),
		}
	}

	/// A `#show raw: it => {{ v(6pt); block.with(fill: luma(240)); v(6pt) }}` places the `v(...)` spacers as
	/// sibling `Block::Space` blocks around the framed element (a non-heading selector has no keep-with-next
	/// guard), and the divider width of a `line(length: 100%)` resolves to an absolute against the placement
	/// avail when the template is applied.
	#[test]
	fn non_heading_template_places_spacer_siblings() {
		let sel	= raw_selector();
		let t	= match lower_transform(&sel, "it => { v(6pt); block.with(fill: luma(240)); line(length: 50%) }") {
			Transform::Template(t)	=> t,
			other					=> panic!("expected a template, got {:?}", other),
		};
		assert_eq!(t.pre.len(), 1, "the leading v() is a sibling before the element");
		assert!(matches!(t.pre[0], Block::Space(sp) if sp == Sp::from_pt(6.0)));
		assert_eq!(t.post.len(), 1, "the trailing line() is a sibling after the element");
		assert!(matches!(t.post[0], Block::Rule { width: Length::Rel(f), .. } if (f - 0.5).abs() < 1e-9),
			"the divider keeps its relative width until placement");
		assert!(t.frame.is_some());

		let rule = Rule {
			selector:	sel,
			transform:	Transform::Template(t),
			rule_id:	0,
			source:		"#show raw".to_string(),
			span:		Span::new(0, 0),
		};
		let mut blocks = vec![Block::Code { lines: vec!["code".to_string()] }];
		apply_rules(&mut blocks, std::slice::from_ref(&rule), Sp::from_pt(400.0));
		// The produced sequence is [Space, Box{code}, Rule]; the rule's width resolved against avail (400 pt).
		match &blocks[0] {
			Block::Scoped { blocks: seq, .. } => {
				assert_eq!(seq.len(), 3, "pre, hole, post: {:?}", seq);
				assert!(matches!(seq[0], Block::Space(_)));
				assert!(matches!(seq[1], Block::Box { .. }));
				match &seq[2] {
					Block::Rule { width: Length::Abs(pt), .. }	=> assert!((pt - 200.0).abs() < 1e-6,
						"50% of a 400pt avail resolves to 200pt, got {}", pt),
					other	=> panic!("expected an absolute divider width, got {:?}", other),
				}
			},
			other	=> panic!("expected a Scoped group, got {:?}", other),
		}
	}

	/// A `block(...)[ #set text(size: 9pt) #it ]` wrap with no fill lowers to a template whose hole patch
	/// overlays the inner `#set` on the element, with no frame.
	#[test]
	fn template_inner_set_becomes_the_hole() {
		let sel	= raw_selector();
		let t	= match lower_transform(&sel, "it => block(inset: 6pt)[#set text(size: 9pt)\n#it]") {
			Transform::Template(t)	=> t,
			other					=> panic!("expected a template, got {:?}", other),
		};
		assert!(t.frame.is_none(), "a wrap with no fill does not frame");
		assert_eq!(t.hole.text.body_size, Some(Sp::from_pt(9.0)), "the inner #set text overlays the element");
	}

	// -- #let furniture template functions ---------------------------------------------------------

	/// `resolve_len` reads an `em` as that fraction of the running body size, and every absolute unit
	/// straight through, so a furniture length lowers to a fixed point value at the document's own size.
	#[test]
	fn resolve_len_reads_em_against_body_size() {
		let body = Sp::from_pt(10.0);
		assert_eq!(resolve_len("0.88em", body), Some(Sp::from_pt(8.8)));
		assert_eq!(resolve_len("1.2em", body), Some(Sp::from_pt(12.0)));
		assert_eq!(resolve_len("0em", body), Some(Sp::from_pt(0.0)));
		assert_eq!(resolve_len("6pt", body), Some(Sp::from_pt(6.0)));
		assert_eq!(resolve_len("50%", body), None, "a percentage has no absolute size here");
	}

	/// The corpus `#pr-note(body)` definition lowers to a transparent-wash box with the asymmetric left/right
	/// inset it names, the `above`/`below` margins folded into the top/bottom pads, and its inner `#set
	/// text`/`#set par` as the body overlay -- every `em` resolved against the document body size.
	#[test]
	fn collect_lowers_pr_note() {
		let src = "\
#let pr-note(body) = block(
	inset: (left: 1.2em, right: 0.6em),
	above: 0.9em,
	below: 1.1em,
	{
		set text(size: 0.88em)
		set par(spacing: 0.55em, first-line-indent: 0em)
		body
	},
)
";
		let body = Sp::from_pt(10.0);
		let mut tfns = TemplateFns::new();
		collect_template_fns(src, body, &Palette::new(), &mut tfns);
		let tf = tfns.get("pr-note").expect("pr-note is collected");
		assert_eq!(tf.body_param, "body");
		assert!(!tf.has_title, "pr-note takes no title");
		assert_eq!(tf.patch.callout.fill, Some(Rgba::TRANSPARENT), "no fill -- a plain indented block, no wash");
		// The block parameters (inset/above/below) resolve their em against the outer body size, since they are
		// evaluated in the outer context before the inner `set text` takes effect.
		assert_eq!(tf.patch.callout.inset_left, Some(Sp::from_pt(12.0)), "left: 1.2em at a 10pt body");
		assert_eq!(tf.patch.callout.inset_right, Some(Sp::from_pt(6.0)), "right: 0.6em");
		assert_eq!(tf.patch.callout.inset_top, Some(Sp::from_pt(9.0)), "above: 0.9em folds into the top pad");
		assert_eq!(tf.patch.callout.inset_bot, Some(Sp::from_pt(11.0)), "below: 1.1em folds into the bottom pad");
		assert_eq!(tf.patch.text.body_size, Some(Sp::from_pt(8.8)), "set text(size: 0.88em) against the 10pt body");
		// The inner `set par(spacing: 0.55em)` follows `set text(size: 0.88em)`, so its em resolves against the
		// reduced 8.8pt size (0.55 * 8.8 = 4.84pt), tracking Typst -- not against the outer body (which gave 5.5).
		assert_eq!(tf.patch.par.skip, Some(Sp::from_pt(4.84)), "0.55em against the reduced 8.8pt size");
		assert_eq!(tf.patch.par.indent, Some(Sp::from_pt(0.0)), "set par(first-line-indent: 0em)");
	}

	/// A `#let name(s) = box(fill: ..)[content]` styled-box content function collects as a CONTENT binding of
	/// its inner `[content]`, with the wrapper name carried, so the text is set and the box styling records a
	/// visible skip -- rather than being lost to neither the furniture nor the content reader (the silent
	/// content-loss bug). `rect` and `block` trailing-bracket forms collect the same way; the furniture
	/// `#pr-note` (content INSIDE the parens, no trailing bracket) is NOT stolen into the content map.
	#[test]
	fn collect_captures_a_styled_box_content_fn_but_not_furniture() {
		let src = "\
#let stamp(s) = box(fill: luma(240), outset: 2pt, radius: 3pt)[*v: #s*]
#let tag(s) = rect(stroke: 1pt)[tag #s]
#let panel(s) = block(inset: 6pt)[panel #s]
#let pr-note(body) = block(inset: (left: 1.2em), { set text(size: 0.9em); body })
";
		let mut cfns = ContentFns::new();
		collect_content_fns(src, &mut cfns);

		let stamp = cfns.get("stamp").expect("a box-wrapped content fn collects as a content binding");
		assert_eq!(stamp.params, vec!["s".to_string()]);
		assert_eq!(stamp.body, "*v: #s*", "the INNER content is the body, not the box call");
		assert_eq!(stamp.wrapper.as_deref(), Some("box"), "the wrapper name is carried for the styling skip");

		assert_eq!(cfns.get("tag").and_then(|c| c.wrapper.as_deref()), Some("rect"), "rect wraps collect too");
		assert_eq!(cfns.get("tag").map(|c| c.body.as_str()), Some("tag #s"));
		assert_eq!(cfns.get("panel").and_then(|c| c.wrapper.as_deref()), Some("block"), "block wraps collect too");
		assert_eq!(cfns.get("panel").map(|c| c.body.as_str()), Some("panel #s"));

		// The furniture pr-note (content block inside the parens, no trailing bracket) is NOT a content binding.
		assert!(cfns.get("pr-note").is_none(),
			"a furniture definition must stay with the template reader, not be stolen into the content map");

		// And it DOES still lower as furniture, so the styled block is drawn as before -- byte-identity held.
		let mut tfns = TemplateFns::new();
		collect_template_fns(src, Sp::from_pt(10.0), &Palette::new(), &mut tfns);
		assert!(tfns.get("pr-note").is_some(), "pr-note still lowers as furniture");
		assert!(tfns.get("stamp").is_none(), "the styled-box content fn is not a furniture wrap");
	}

	/// A furniture body that never names its content parameter is not a wrap -- it lowers to nothing, so a
	/// call to it stays a tallied skip rather than expanding wrongly.
	#[test]
	fn a_definition_that_ignores_its_body_is_not_lowered() {
		let src = "#let bogus(body) = block(inset: 6pt, { set text(size: 0.9em) })\n";
		let mut tfns = TemplateFns::new();
		collect_template_fns(src, Sp::from_pt(10.0), &Palette::new(), &mut tfns);
		assert!(tfns.get("bogus").is_none(), "a body that never places `body` is not a furniture wrap");
	}

	/// A bare `#let name = <literal>` collects a scalar for a string, an integer and a length alike, keeping
	/// a string's contents unquoted and a number or length as its own written text (Typst's own display form
	/// for a plain literal).
	#[test]
	fn collect_scalar_fns_reads_string_int_and_length_literals() {
		let src = "\
#let title = \"Field Guide\"
#let edition = 3
#let gap = 12pt
";
		let mut sfns = ScalarFns::new();
		collect_scalar_fns(src, &mut sfns);
		assert_eq!(sfns.get("title"), Some(&ScalarValue::Str("Field Guide".to_string())));
		assert_eq!(sfns.get("edition"), Some(&ScalarValue::Number("3".to_string())));
		assert_eq!(sfns.get("gap"), Some(&ScalarValue::Number("12pt".to_string())));
	}

	/// A `#let` whose right-hand side is not a bare literal -- a furniture wrap, a content binding, a data
	/// array, a function signature, or an expression this reader does not evaluate -- collects no scalar, so
	/// it is left exactly as before (a visible `#let` skip, or the furniture/content/array reader's own).
	#[test]
	fn collect_scalar_fns_passes_over_non_literal_lets() {
		let src = "\
#let pr-note(body) = block(inset: 6pt, body)
#let greeting = [Hello]
#let data = (1, 2, 3)
#let doubled(n) = n * 2
#let total = count + 1
";
		let mut sfns = ScalarFns::new();
		collect_scalar_fns(src, &mut sfns);
		assert!(sfns.is_empty(), "no line here is a bare literal binding: {:?}", sfns);
	}

	/// A trailing `//` comment on a scalar `#let`'s line does not leak into its value -- `#let n = 3 //
	/// words/min` reads the plain integer, and a string's own `//`-shaped contents (inside its quotes) are
	/// kept rather than truncated.
	#[test]
	fn collect_scalar_fns_strips_a_trailing_comment_but_keeps_a_quoted_one() {
		let src = "\
#let speed = 230 // words/min
#let url-ish = \"see https://example.com\" // not a real link here
";
		let mut sfns = ScalarFns::new();
		collect_scalar_fns(src, &mut sfns);
		assert_eq!(sfns.get("speed"), Some(&ScalarValue::Number("230".to_string())));
		assert_eq!(sfns.get("url-ish"), Some(&ScalarValue::Str("see https://example.com".to_string())));
	}

	/// An `#aside-box(title: none, float: true, body)` definition (the `let inner = box(...)` idiom, re-wrapped
	/// in a floating figure) lowers: the body parameter is found past the two keyword parameters, the title
	/// keyword is recognised, and the `box`'s fill/inset/radius resolve. A `luma(...)` fill stands in for the
	/// corpus's `colours.yellow.lighten(92%)` here -- palette-name resolution is the aside-box milestone's
	/// own gap. The `figure(placement: auto)` float wrapper is recognised: `float` is `Some(Auto)`, so the
	/// driver sets the callout at the top or foot of a page by the midpoint rule rather than in the flow.
	#[test]
	fn collect_lowers_aside_box_shape() {
		let src = "\
#let aside-box(title: none, float: true, body) = {
	let inner = box(
		width: 100%,
		inset: (x: 1em, y: 1em, bottom: 1.2em),
		fill: luma(240),
		radius: 4pt,
		stroke: (left: 2pt + luma(50)),
		[
			#if title != none [
				#text(weight: \"bold\", size: 0.85em)[#title] #v(0.4em)
			]
			#text(size: 0.85em)[#body]
		]
	)
	if float { figure(placement: auto, inner) } else { inner }
}
";
		let body = Sp::from_pt(10.0);
		let mut tfns = TemplateFns::new();
		collect_template_fns(src, body, &Palette::new(), &mut tfns);
		let tf = tfns.get("aside-box").expect("aside-box is collected");
		assert_eq!(tf.body_param, "body", "the body is the last positional parameter, past title: and float:");
		assert!(tf.has_title, "a title: keyword is recognised");
		assert_eq!(tf.patch.callout.fill, Some(Rgba::opaque(240, 240, 240)), "the box fill resolves");
		assert_eq!(tf.patch.callout.inset_left, Some(Sp::from_pt(10.0)), "inset.x -> left, 1em at 10pt");
		assert_eq!(tf.patch.callout.inset_right, Some(Sp::from_pt(10.0)), "inset.x -> right");
		assert_eq!(tf.patch.callout.inset_top, Some(Sp::from_pt(10.0)), "inset.y -> top");
		assert_eq!(tf.patch.callout.inset_bot, Some(Sp::from_pt(12.0)), "bottom overrides y for the foot pad");
		assert_eq!(tf.patch.callout.radius, Some(Sp::from_pt(4.0)), "radius: 4pt");
		// The left stroke `2pt + luma(50)` -- the length term is the width, the colour term the paint.
		assert_eq!(tf.patch.callout.stroke_left_w, Some(Sp::from_pt(2.0)), "the left rule is 2pt wide");
		assert_eq!(tf.patch.callout.stroke_left_col, Some(Rgba::opaque(50, 50, 50)), "the left rule's colour");
		// The body size comes from the `text(size: 0.85em)[#body]` wrapper, not a `#set`.
		assert_eq!(tf.patch.text.body_size, Some(Sp::from_pt(8.5)), "body wrapped in text(size: 0.85em)");
		assert_eq!(tf.title_size, Some(Sp::from_pt(8.5)), "the bold title run is set at 0.85em");
		assert_eq!(tf.float, Some(FloatPlacement::Auto), "the figure(placement: auto) wrapper makes it an auto float");
	}

	/// A `#let colours = (...)` palette is collected, and a furniture fill/stroke naming `colours.<name>`
	/// resolves through it (with any `.lighten`/`.darken` applied to the looked-up colour). Without the
	/// palette the same reference resolves to nothing and the fill falls back to transparent.
	#[test]
	fn palette_resolves_a_named_colour_reference() {
		let src = "#let colours = (\n  yellow:   rgb(\"#f0f600\"),\n  purple:   rgb(\"#4c1a57\"),\n)\n";
		let mut palette = Palette::new();
		collect_palette(src, &mut palette);
		assert_eq!(palette.get("yellow"), Some(&Rgba::opaque(0xf0, 0xf6, 0x00)));
		// A reference resolves, and a modifier lightens the looked-up colour toward white.
		assert_eq!(parse_colour_pal("colours.yellow", &palette), Some(Rgba::opaque(0xf0, 0xf6, 0x00)));
		assert!(parse_colour_pal("colours.yellow.lighten(92%)", &palette).is_some());
		// Without the palette, the reference cannot resolve.
		assert_eq!(parse_colour_pal("colours.yellow", &Palette::new()), None);
	}

	/// `#let colours` must match exactly -- a differently named dict such as `#let colours_x` is a
	/// separate binding, not the palette, and must not be prefix-matched into it.
	#[test]
	fn collect_palette_does_not_prefix_match_a_longer_name() {
		let src = "#let colours_x = (\n  yellow: rgb(\"#f0f600\"),\n)\n";
		let mut palette = Palette::new();
		collect_palette(src, &mut palette);
		assert!(palette.get("yellow").is_none(), "colours_x must not be read as the colours palette");
	}
}
