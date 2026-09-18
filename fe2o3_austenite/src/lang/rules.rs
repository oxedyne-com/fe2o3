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
/// into). `frame`, when set, seats the element in a washed [`Block::Box`] of that fill -- a `block.with(fill:
/// ...)` callout. `rule_id` and the hole's index (always `pre.len()`) are recorded so a later pass can
/// address the moved element by the rule that placed it -- the block-identity hook the design note calls for.
#[derive(Clone, Debug)]
pub struct Template {
	pub pre:		Vec<Block>,
	pub hole:		ThemePatch,
	pub post:		Vec<Block>,
	pub frame:		Option<Rgba>,
	pub rule_id:	RuleId,
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
		match heading_size_patch(selector, args) {
			Some(p)	=> p,
			None	=> return Transform::Refused(fmt!(
				"set text on {} named no size the renderer can consume", selector_label(selector))),
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

/// The heading-group patch a `set text(size: ...)` on a heading selector lowers to: the size the renderer
/// reads for a heading is its own level size ([`Theme::heading_size`]), so the text size is redirected
/// there rather than into `text.body_size`, which a heading never reads. A `level: N` predicate targets
/// that one level; an unpredicated heading selector sizes every level alike (`size_all`). `None` when the
/// set names no size, or a size that does not convert to points (an `em`, which needs a running size this
/// lowering has not) -- the caller then refuses it rather than wrapping to no effect.
fn heading_size_patch(selector: &Selector, args: &str) -> Option<ThemePatch> {
	// Reuse the body-size reader: `set text(size: 30pt)` lowers its size into `text.body_size`, and that
	// point value is exactly the size to redirect into the heading level.
	let size = set::lower_set("text", args).text.body_size?;
	let mut patch = ThemePatch::default();
	match level_predicate(selector) {
		Some(n)	=> {
			let idx = (n.max(1) as usize) - 1;	// level 0/1 both index 0, as the theme maps them
			let mut levels: Vec<ThemeHeadingLevelPatch> = Vec::with_capacity(idx + 1);
			levels.resize_with(idx + 1, Default::default);
			levels[idx].size = Some(size);
			patch.heading.levels = levels;
		},
		None	=> patch.heading.size_all = Some(size),
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
/// into the heading group by [`heading_size_patch`]).
fn unread_field_reason(selector: &Selector, target: &str, args: &str) -> Option<String> {
	let has = |key: &str| names_arg(args, key);
	// A heading's only renderable `text` field is its size; the rest style running body text a heading
	// never sets, so they are refused here, naming the field and the selector.
	if selector.kind == ElementKind::Heading && target == "text" {
		if has("tracking")	{ return Some(fmt!("text.tracking is not read for {}", selector_label(selector))); }
		if has("ligatures")	{ return Some(fmt!("text.ligatures is not read for {}", selector_label(selector))); }
		if has("font")		{ return Some(fmt!("a heading font is resolved elsewhere, not from {}", selector_label(selector))); }
		if has("hyphenate")	{ return Some(fmt!("text.hyphenate is not read for {}", selector_label(selector))); }
		return None;
	}
	match target {
		"text" => {
			if has("tracking")	{ return Some("text.tracking is not read by the renderer".to_string()); }
			if has("ligatures")	{ return Some("text.ligatures is not read by the renderer".to_string()); }
			if has("font")		{ return Some("text.faces.* (a body font) is not read by the renderer".to_string()); }
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
	let mut frame:	Option<Rgba>	= None;
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
/// or `block(fill: ...)[#it]` sets the frame's wash; a `#set text(...)` inside the wrap's content overlays
/// the element; a bare `it` leaves both untouched. A wrap that is neither a `.with` partial nor a content
/// wrap of `it` is refused, so a body this reader cannot place is a visible refusal, not a silent no-op.
fn read_element(s: &str, hole: &mut ThemePatch, frame: &mut Option<Rgba>) -> Outcome<()> {
	let is_wrap = s.starts_with("block") || s.starts_with("box");
	if !is_wrap {
		// A bare `it` / `it.body` -- the element passes through untouched.
		return Ok(());
	}
	// The wrap's argument list -- `block.with(<args>)` or `block(<args>)[...]`.
	let head = s.strip_prefix("block").or_else(|| s.strip_prefix("box")).unwrap_or(s);
	let head = head.trim_start_matches(".with").trim_start();
	let args = call_group(head).unwrap_or_default();
	// A `fill:` washes the element in a box.
	if let Some(fv) = named_value(&args, "fill") {
		match parse_colour(&fv) {
			Some(rgba)	=> *frame = Some(rgba),
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

/// A colour expression lowered to an [`Rgba`]. The forms a template fill takes that resolve without a
/// palette: `luma(<n>)`, `rgb("#rrggbb")`, `rgb(<r>, <g>, <b>)` and a small set of named colours, each
/// optionally lightened or darkened (`.lighten(<p>%)` / `.darken(<p>%)`). A palette reference (`colours.blue`)
/// resolves to no value here and the caller refuses it rather than guessing.
fn parse_colour(expr: &str) -> Option<Rgba> {
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
	} else {
		named_colour(head)?
	};
	Some(apply_colour_mods(base, mods))
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
		if let Block::Scoped { blocks: inner, .. } | Block::Box { blocks: inner, .. } = b {
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
		Some(fill)	=> {
			// The element seated in a box washed the template's fill -- the wash the renderer reads from
			// `callout.fill` for a `Block::Box`.
			let mut patch = ThemePatch::default();
			patch.callout.fill = Some(fill);
			Block::Box { blocks: vec![it], patch }
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
	/// named fill, under a transparent hole scope. The frame colour is the one the template named.
	#[test]
	fn template_on_raw_frames_the_code() {
		let sel	= raw_selector();
		let tr	= lower_transform(&sel, "it => block.with(fill: luma(240), inset: 8pt, radius: 4pt)");
		let t	= match tr {
			Transform::Template(t)	=> t,
			other					=> panic!("expected a template, got {:?}", other),
		};
		assert_eq!(t.frame, Some(Rgba::opaque(240, 240, 240)), "the block.with fill becomes the frame");
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
					Block::Box { blocks: bb, patch }	=> {
						assert_eq!(bb.len(), 1);
						assert!(matches!(bb[0], Block::Code { .. }), "the code is moved into the box");
						assert_eq!(patch.callout.fill, Some(Rgba::opaque(240, 240, 240)),
							"the box wash is the template's fill");
					},
					other	=> panic!("expected the code framed in a Box, got {:?}", other),
				}
			},
			other	=> panic!("expected a Scoped group, got {:?}", other),
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
}
