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
		let transform	= lower_transform(tr_text.trim());
		if let Transform::Refused(reason) = &transform {
			refusals.record(&fmt!("{} ({})", source, reason), span);
		}
		let rule_id = base_id + rules.len();
		rules.push(Rule { selector, transform, rule_id, source, span });
	}
	rules
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
fn lower_transform(body: &str) -> Transform {
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

	// A field the renderer does not read (or reads only cross-group) is refused rather than applied, so a
	// rule that would silently no-op is a visible "not yet supported".
	if let Some(reason) = unread_field_reason(target, args) {
		return Transform::Refused(reason);
	}

	let patch = set::lower_set(target, args);
	if patch == ThemePatch::default() {
		// The set named a target or argument the theme carries no read field for: refuse rather than wrap an
		// element in an empty scope that changes nothing.
		return Transform::Refused(fmt!("set {} lowered to nothing", target));
	}
	Transform::SetFields(patch)
}

/// Why a `set <target>(<args>)` transform patches a field the renderer does not read, or reads only across
/// a group boundary the readiness audit named -- so the rule is refused rather than wrapped to no effect.
/// `None` when every field it names is one the renderer reads.
fn unread_field_reason(target: &str, args: &str) -> Option<String> {
	let has = |key: &str| names_arg(args, key);
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

	/// A `set text(size: 30pt)` transform lowers to a set-fields patch that names the body size; a
	/// page-reading transform and an unread-field transform are both refused instead.
	#[test]
	fn transform_lowers_or_refuses() {
		match lower_transform("set text(size: 30pt)") {
			Transform::SetFields(p)	=> assert!(p.text.body_size.is_some()),
			Transform::Refused(r)	=> panic!("a supported set was refused: {}", r),
		}
		assert!(matches!(lower_transform("it => context measure(it)"), Transform::Refused(_)),
			"a page-reading transform must be refused");
		assert!(matches!(lower_transform("set text(tracking: 0.1em)"), Transform::Refused(_)),
			"an unread field must be refused");
		assert!(matches!(lower_transform("it => underline(it)"), Transform::Refused(_)),
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

	/// Applying an authored 30pt rule wraps the level-1 heading in a `Block::Scoped` carrying the size
	/// patch and leaves a level-2 heading unwrapped -- the rule styles only its target.
	#[test]
	fn apply_wraps_only_the_matched_heading() {
		let mut refusals = Refusals::default();
		let rules = collect_from_source(
			"#show heading.where(level: 1): set text(size: 30pt)\n", 0, &mut refusals);
		let mut blocks = vec![heading(1), heading(2)];
		apply_rules(&mut blocks, &rules);
		// The level-1 heading is now a scope carrying the size patch; the level-2 heading is untouched.
		match &blocks[0] {
			Block::Scoped { patch, blocks } => {
				assert_eq!(patch.text.body_size, Some(crate::ir::Sp::from_pt(30.0)));
				assert!(matches!(blocks[0], Block::Heading { level: 1, .. }));
			},
			other => panic!("the level-1 heading should be wrapped, found {:?}", other),
		}
		assert!(matches!(blocks[1], Block::Heading { level: 2, .. }),
			"the level-2 heading must be left unwrapped");
	}

	/// An authored rule changes only the element it targets, at render time: a level-1 heading numbering
	/// rule renumbers the level-1 heading while a level-2 heading, outside the rule's scope, keeps the
	/// default dotted number. Numbering is used as the visibly-read supported field because a heading's
	/// glyph size is read from `heading_size(level)`, not `text.body_size` -- a `set text(size: ...)` rule
	/// wraps the heading identically (see `apply_wraps_only_the_matched_heading`) but is inert on its glyphs,
	/// a finding for the wrap-transform part.
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
}
