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
use crate::ir::Span;
use crate::theme::{
	Theme,
	ThemeHeadingLevelPatch,
	ThemePatch,
};

use super::parse::Refusals;
use super::set;

use oxedyne_fe2o3_core::prelude::*;

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

/// What a rule does to a matched element. This part builds only a set-fields transform; a page-reading or
/// otherwise unsupported transform is a [`Transform::Refused`], recorded and never run.
#[derive(Clone, Debug)]
pub enum Transform {
	SetFields(ThemePatch),
	Refused(String),	// the reason, recorded as a refusal rather than applied
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
		let transform	= lower_transform(&selector, tr_text.trim());
		if let Transform::Refused(reason) = &transform {
			refusals.record(&fmt!("{} ({})", source, reason), span);
		}
		let rule_id = base_id + rules.len();
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

	// The one form this part lowers: `set <target>(<args>)`.
	let after = match body.strip_prefix("set ") {
		Some(a)	=> a.trim(),
		None	=> return Transform::Refused(fmt!("unsupported transform: {}", short(body))),
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
pub fn apply_rules(blocks: &mut Vec<Block>, rules: &[Rule]) {
	// Descend into existing nesting subtrees first, so their own elements are matched too.
	for b in blocks.iter_mut() {
		if let Block::Scoped { blocks: inner, .. } | Block::Box { blocks: inner, .. } = b {
			apply_rules(inner, rules);
		}
	}
	// Then wrap each block this slice holds that a rule matches.
	let taken = std::mem::take(blocks);
	let mut out = Vec::with_capacity(taken.len());
	for block in taken {
		out.push(wrap_matching(block, rules));
	}
	*blocks = out;
}

/// Wraps `block` in one [`Block::Scoped`] per rule that matches it, the last matching rule innermost so it
/// overrides the earlier ones; a rule whose patch is empty adds no scope. A block no rule matches is
/// returned unchanged.
fn wrap_matching(block: Block, rules: &[Rule]) -> Block {
	// The matching rules' patches in order, so the fold wraps them last-innermost below.
	let mut patches: Vec<&ThemePatch> = Vec::new();
	for rule in rules {
		if let Transform::SetFields(p) = &rule.transform {
			if *p != ThemePatch::default() && matches(&rule.selector, &block) {
				patches.push(p);
			}
		}
	}
	let mut wrapped = block;
	for p in patches.into_iter().rev() {
		wrapped = Block::Scoped { patch: p.clone(), blocks: vec![wrapped] };
	}
	wrapped
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
			Transform::Refused(r)	=> panic!("a heading size rule was refused: {}", r),
		}
		// Under a body selector, the same set stays a body-size patch.
		match lower_transform(&par, "set text(size: 30pt)") {
			Transform::SetFields(p)	=> assert_eq!(p.text.body_size, Some(crate::ir::Sp::from_pt(30.0))),
			Transform::Refused(r)	=> panic!("a body size rule was refused: {}", r),
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
			apply_rules(&mut bs, &rules);
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
		apply_rules(&mut blocks, &rules);
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
			apply_rules(&mut bs, &rules);
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
}
