//! Lowering a document's declarative styling onto a [`Theme`].
//!
//! Typst styles a document with `#set` and `#show` -- `#set text(size: 11pt)`, `#show: doc.with(...)`.
//! Austenite does not *execute* these (it has no style-computation layer, by design), but it can *lower*
//! the declarative ones onto the theme: read the arguments and write the fields they name. The reader
//! ([`crate::lang::parse`]) captures these constructs rather than refusing them; this module turns a
//! captured construct's argument text into theme-field writes, and the book assembler
//! ([`crate::book`]), which holds the theme, drives it over a root's own declarations.
//!
//! What is lowered here is deliberately narrow -- the whole-document `#show: <template>.with(...)`
//! application, and a top-level `#set` on an element the theme carries (text, par, heading, list, enum,
//! math.equation, page). A `#set` on a target the theme has no field for (a `#set rect(...)`, say) is
//! **not** lowered: [`lower_set`] returns `false` and the reader's refusal path keeps it a visible
//! refusal rather than a silent no-op. The introspective tail -- a `#show` whose body is a closure that
//! reads `context`, `query`, `counter.at`, `measure`, `layout` or `state` -- is never captured here at
//! all; it stays a refusal, since lowering it would be a lie about what the engine can do.
//!
//! Scope note. This unit lowers a root's *own* top-level declarations. Per-part page geometry, per-level
//! heading faces and the config file's `#let` type scale are a later unit's work; the reserved theme
//! fields those write are populated here only where a `#show: doc.with(...)` names them directly.

use crate::ir::Sp;
use crate::ir::Span;
use crate::lang::lex;
use crate::lang::lex::Place;
use crate::lang::RefusalClass;
use crate::lang::Refusals;
use crate::theme::{
	Theme,
	ThemeHeadingLevelPatch,
	ThemePatch,
};

use oxedyne_fe2o3_core::prelude::*;

/// The `#set` targets the theme carries a field for, so a `#set` on one of these lowers rather than
/// staying a refusal. The single source of truth: [`lower_set`] matches these as its handled arms, and
/// the reader ([`crate::lang::parse::is_lowerable_set`]) tests a `#set` line against this same list to
/// decide whether to capture it for lowering or leave it a visible refusal.
pub const LOWERABLE_SET_TARGETS: &[&str] = &[
	"text",
	"par",
	"page",
	"heading",
	"list",
	"enum",
	"math.equation",
	"columns",
	"document",	// metadata, read by `document_info`; lowers to no theme field
];

/// Lowers a source's own top-level declarations onto `theme`: its `#show: <template>.with(...)`
/// application, then each top-level `#set <target>(...)` the theme carries a field for. Values a
/// construct does not name are left as the theme already holds them, so a document that sets little
/// changes little. This reads only the root's own declarations, not those of its includes, and applies
/// them at the document scope -- the whole theme the driver renders with.
pub fn lower_root_declarations(src: &str, theme: &mut Theme) {
	// The theme's own body size seeds the `em` base: a root that sets no `text(size:)` of its own resolves a
	// `#set par(spacing: <em>)` against the size the theme already carries, not the raw house default.
	theme.apply(&lower_declarations_seeded(src, theme.text.body_size.to_pt()));
}

/// The PDF Info fields a source's own `#set document(...)` rules name: each at its top level or in a bare
/// content block `#[ ... ]`, where Typst applies it. A later `#set` overrides an earlier one field by field,
/// as in Typst. One in a container is refused at its site in `skips`, charged to `file`.
pub fn document_info(src: &str, file: &str, skips: &mut Refusals) -> crate::doc::DocInfo {
	let mut info = crate::doc::DocInfo::default();
	fold_document_info(src, &mut info, file, 0, skips);
	info
}

/// Applies each `#set document(...)` of `src` that Typst applies to `info` in source order, field by field, so a
/// caller walking a document's files in document order -- a root, then each file it includes where the
/// include stands -- builds the dictionary Typst builds. A rule at the top level or in a bare content block
/// applies; one in a list item, a heading, strong or emphasis, which Typst refuses ("document set rules are
/// not allowed inside of containers"), applies nothing and is recorded in `skips` at its `#`, `at` bytes into
/// `file`, since `src` may be a part of it. A rule in any other body is the reader's to refuse where it reads
/// that body.
pub fn fold_document_info(src: &str, info: &mut crate::doc::DocInfo, file: &str, at: u32, skips: &mut Refusals) {
	for (place, hash, args) in document_sets(src) {
		if place == Place::Contained {
			let site = Span::new(at.saturating_add(hash as u32), at.saturating_add(hash as u32));
			skips.record_stand_in_in(file, "#set document", site, RefusalClass::Unsupported,
				"(document set rules are not allowed inside of containers, so none of its fields is applied)");
			continue;
		}
		let list = lex::args(&args);
		// Typst refuses a field named twice and applies nothing of the rule; the reader's site says why.
		if lex::duplicate_key(&list).is_some() {
			continue;
		}
		let fields: [(&str, &mut Option<String>); 4] = [
			("title",		&mut info.title),
			("author",		&mut info.author),
			("description",	&mut info.subject),
			("keywords",	&mut info.keywords),
		];
		for (key, slot) in fields {
			if let Some(value) = document_field(&list, key) {
				*slot = value;
			}
		}
	}
}

/// Is this captured construct a `#set document(...)`?
pub fn sets_document(buf: &str) -> bool {
	top_level_sets(buf).first().map_or(false, |(target, _)| target == "document")
}

/// The [`ThemePatch`] a source's own top-level declarations lower to, without applying it: a `#show:
/// <template>.with(...)` application, then each top-level `#set <target>(...)` the theme carries a field
/// for, folded into one patch in source order so a later `#set` overrides an earlier one. Returned rather
/// than applied so a caller can fold it at the scope it governs -- the document, an included chapter's
/// subtree, or a `#styled-box` body. This reads only the source's own declarations, not those of any
/// file it includes.
pub fn lower_declarations(src: &str) -> ThemePatch {
	// A scoped or reader-side caller (an included chapter, a `#columns`/`#styled-box` body) has no size in
	// hand: the reader is size-agnostic, so an `em` par-field that names no `text(size:)` of its own falls
	// back to the house default. One that DOES set its own size resolves against it, found by the pre-pass.
	lower_declarations_seeded(src, DEFAULT_BODY_PT)
}

/// The [`ThemePatch`] a source's own top-level declarations lower to, resolving each `em` paragraph field
/// against the text size in force AT LAYOUT the way Typst does: the batch's FINAL `#set text(size: <pt>)`
/// wins whether it precedes or follows the `#set par`, and a batch that sets none inherits `scope_body_pt`
/// (the theme or enclosing-scope size). Because the size is resolved lazily over the whole batch, source
/// order between the `text` and `par` sets does not change the result.
fn lower_declarations_seeded(src: &str, scope_body_pt: f64) -> ThemePatch {
	let mut patch = ThemePatch::default();
	if let Some(args) = show_doc_with_args(src) {
		lower_doc_with_into(&lex::args(&args), &mut patch);
	}
	let sets: Vec<(String, Vec<lex::Arg>)> = top_level_sets(src).into_iter()
		.map(|(target, args)| (target, lex::args(&args)))
		.collect();
	// The layout-time `em` base: the last `text(size:)` the batch applies, else the size already in force.
	let em_base_pt = sets.iter().rev()
		.filter(|(target, list)| target == "text" && lex::duplicate_key(list).is_none())
		.find_map(|(_, list)| named_length_pt(list, "size"))
		.unwrap_or(scope_body_pt);
	for (target, list) in &sets {
		// A target the theme has no field for writes nothing; the reader keeps such a `#set` a refusal, so
		// nothing is silently dropped here.
		lower_set_into(target, list, &mut patch, em_base_pt);
	}
	patch
}

/// The [`ThemePatch`] a `#show: <template>.with(...)` application's named arguments lower to. Only the
/// styling-relevant arguments map to theme fields -- `heading-font` names the heading face -- and the
/// rest (title, subtitle, logos, meta-data) are the book's front matter, read on the book's own path.
/// An argument this does not recognise is left alone rather than guessed at.
pub fn lower_doc_with(args: &str) -> ThemePatch {
	let mut patch = ThemePatch::default();
	lower_doc_with_into(&lex::args(args), &mut patch);
	patch
}

fn lower_doc_with_into(list: &[lex::Arg], patch: &mut ThemePatch) {
	if let Some(font) = named_string(list, "heading-font") {
		if !font.is_empty() {
			// The doc template applies the heading font to levels 1 and 2 only, the body family below (its
			// per-level show rule: `font: if it.level <= 2 { heading-font } else { "Libertinus Serif" }`).
			// Lower it into those two levels' `face`, which the renderer resolves and applies, rather than
			// the role-default `heading.face` nothing read.
			while patch.heading.levels.len() < 2 {
				patch.heading.levels.push(ThemeHeadingLevelPatch::default());
			}
			patch.heading.levels[0].face = Some(Some(font.clone()));
			patch.heading.levels[1].face = Some(Some(font));
		}
	}
}

/// The [`ThemePatch`] a top-level `#set <target>(...)` lowers to. A `target` the theme has no field for
/// ([`LOWERABLE_SET_TARGETS`]) lowers to an empty patch, and the reader keeps that `#set` a visible
/// refusal rather than a silent no-op. A named argument the set omits leaves that field unnamed in the
/// patch, so applying it leaves the theme's own value.
pub fn lower_set(target: &str, args: &str) -> ThemePatch {
	let mut patch = ThemePatch::default();
	lower_set_into(target, &lex::args(args), &mut patch, DEFAULT_BODY_PT);
	patch
}

/// The house default body size, in points -- the base an `em` length in a lone `#set` (no earlier
/// `#set text(size:)` to move it) resolves against, matching [`crate::theme::ThemeText`]'s own default.
const DEFAULT_BODY_PT: f64 = 11.0;

/// `em_base_pt` is the text size in force, in points, that a font-relative (`em`) length resolves
/// against; a caller with no size context passes [`DEFAULT_BODY_PT`].
fn lower_set_into(target: &str, list: &[lex::Arg], patch: &mut ThemePatch, em_base_pt: f64) -> Vec<&'static str> {
	// The argument keys this set applied AND the renderer consumes -- the invariant is that every lowered
	// field is either read by the renderer or refused with a diagnostic, never written-and-ignored. A key
	// that lowers into a field nothing reads yet (an equation `numbering`, a `page` dimension) is deliberately NOT pushed here, so the refusal check ([`set_refusal_reason`]) sees it as
	// unapplied and records a visible "not yet supported" rather than a silent no-op. A key present in the
	// source but absent for any other reason (unrecognised for the target, an `em` length, a bare `none`)
	// is likewise not pushed.
	let mut used: Vec<&'static str> = Vec::new();
	// Typst refuses a rule that names a field twice and applies none of it, so nothing is lowered here and
	// the refusal check names the field.
	if lex::duplicate_key(list).is_some() {
		return used;
	}
	match target {
		"text" => {
			if let Some(pt) = named_length_pt(list, "size") {
				patch.text.body_size = Some(Sp::from_pt(pt));
				used.push("size");
			}
			// The body family list: `font: "Name"` or the fallback array `font: ("A", "B")`. Resolved against
			// the document's fonts at assembly, where a family no font declares is a hard error (Typst's
			// missing-family precheck), so nothing named here ever falls back silently.
			if let Some(expr) = lex::named(list, "font") {
				if let Some(families) = font_families(expr) {
					patch.text.faces.body = Some(families);
					used.push("font");
				}
			}
			if let Some(b) = named_bool(list, "hyphenate") {
				patch.text.hyphenate = Some(b);
				used.push("hyphenate");
			}
			// The prose fill colour: `rgb("#…")`, `luma(n)`, a named colour, each optionally `.lighten`/
			// `.darken`. Read with the same grammar as a rule's fill ([`crate::lang::rules::parse_colour`])
			// so the two readers cannot drift. A palette reference (`colours.blue`) resolves to nothing here
			// and is left unmarked, so a `#set text(fill: colours.x)` is refused rather than set wrongly.
			// This lowers a body `#set text(fill:)` only; a heading-selector fill stays refused on the rule
			// engine's own path.
			if let Some(expr) = lex::named(list, "fill") {
				if let Some(rgba) = crate::lang::rules::parse_colour(expr) {
					patch.text.fill = Some(rgba);
					used.push("fill");
				}
			}
		},
		"par" => {
			// `spacing` (the space between paragraphs) and `first-line-indent` are pure lengths: a `pt` value
			// is taken verbatim, and a font-relative `em` -- the book/doc template idiom (`#set par(spacing:
			// 0.75em)`) -- resolves against the text size in force rather than being dropped and left at the
			// theme default.
			//
			// `leading` is deliberately `pt`-only here. Typst's `par(leading:)` is the GAP added between line
			// boxes, whereas the theme's `text.leading` is the baseline-to-baseline distance; converting one
			// to the other needs the calibrated line-box height ([`crate::theme::ThemeCalibration`]), the way
			// the book path's `build_style` does. Lowering an `em` (or even a `pt`) leading straight into the
			// baseline field would set the line grid wrongly, so that conversion is left to a dedicated leading
			// pass -- see this crate's parity notes. A `pt` leading keeps the pre-existing behaviour.
			if let Some(pt) = named_length_pt(list, "leading") {
				patch.text.leading = Some(Sp::from_pt(pt));
				used.push("leading");
			}
			if let Some(pt) = named_length_pt_em(list, "spacing", em_base_pt) {
				patch.par.skip = Some(Sp::from_pt(pt));
				used.push("spacing");
			}
			if let Some(pt) = named_length_pt_em(list, "first-line-indent", em_base_pt) {
				patch.par.indent = Some(Sp::from_pt(pt));
				used.push("first-line-indent");
			}
			if let Some(b) = named_bool(list, "justify") {
				patch.text.justify = Some(b);
				used.push("justify");
			}
		},
		"heading" => {
			// `numbering` applies across the levels, the way Typst's own `set heading(numbering: ...)` does:
			// one group-level leaf the patch folds onto every level, whatever their count.
			if let Some(pattern) = named_string(list, "numbering") {
				let pat = if pattern.is_empty() { None } else { Some(pattern) };
				patch.heading.numbering_all = Some(pat);
				used.push("numbering");
			}
		},
		"list" => {
			if let Some(pt) = named_length_pt(list, "spacing") {
				patch.list.item_skip = Some(Some(Sp::from_pt(pt)));
				used.push("spacing");
			}
			if let Some(pt) = named_length_pt(list, "indent") {
				patch.list.marker_gap = Some(Sp::from_pt(pt));
				used.push("indent");
			}
		},
		"enum" => {
			if let Some(pt) = named_length_pt(list, "spacing") {
				patch.enumeration.item_skip = Some(Some(Sp::from_pt(pt)));
				used.push("spacing");
			}
			if let Some(pt) = named_length_pt(list, "indent") {
				patch.enumeration.marker_gap = Some(Sp::from_pt(pt));
				used.push("indent");
			}
			if let Some(pattern) = named_string(list, "numbering") {
				patch.enumeration.numbering = Some(if pattern.is_empty() { None } else { Some(pattern) });
				used.push("numbering");
			}
		},
		"math.equation" => {
			// The equation renderer numbers displays "(N)" unconditionally and reads no pattern yet, so a
			// `numbering` lowers into the theme but is left unmarked -- refused, not silently ignored.
			if let Some(pattern) = named_string(list, "numbering") {
				patch.equation.numbering = Some(if pattern.is_empty() { None } else { Some(pattern) });
			}
		},
		"page" => {
			// Page geometry lowers onto the body part's reserved override, but no unit consumes it yet (the
			// driver still supplies the document geometry), so `width`/`height` are left unmarked and a lone
			// `#set page(...)` is refused as not-yet-supported rather than silently doing nothing.
			if let Some(pt) = named_length_mm_or_pt(list, "width") {
				patch.page.body.default.width = Some(Some(pt));
			}
			if let Some(pt) = named_length_mm_or_pt(list, "height") {
				patch.page.body.default.height = Some(Some(pt));
			}
			// The column count the body flows in, read by the author and the driver: a whole number of at
			// least one.
			if let Some(expr) = lex::named(list, "columns") {
				if let Ok(n) = expr.trim().parse::<usize>() {
					if n >= 1 {
						patch.page.columns = Some(n);
						used.push("columns");
					}
				}
			}
		},
		"columns" => {
			// The space between two columns: a percentage of the content width, or an absolute length.
			if let Some(expr) = lex::named(list, "gutter") {
				if let Some(len) = crate::lang::parse::parse_length(expr) {
					patch.page.column_gutter = Some(len);
					used.push("gutter");
				}
			}
		},
		"document" => {
			// Metadata, not styling: a field counts as applied only when `document_info` can read its
			// value, so one it cannot is refused rather than dropped from the Info dictionary. No date is
			// ever written, which is what `date: none` asks for.
			for key in ["title", "author", "description", "keywords"] {
				if document_field(list, key).is_some() {
					used.push(key);
				}
			}
			if lex::named(list, "date") == Some("none") {
				used.push("date");
			}
		},
		_ => {},
	}
	used
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ REFUSING A #set THAT LOWERED TO NOTHING (H2)                               │
// └───────────────────────────────────────────────────────────────────────────┘

/// Why a lowerable `#set <target>(...)` should be refused rather than pass silently, as the words that
/// follow the rule's name in its site: it applied none of its arguments, named one twice (which Typst
/// refuses, applying none of the rule), gave one positionally, or named one the lowering does not
/// recognise or could not convert (an `em` length with no context, a `#set text(lang: ...)`, a
/// `heading(numbering: none)`). The keys present and the keys applied are read from the same arguments,
/// with comments as trivia, so a field commented out is no field. `None` when every argument the source
/// named was applied. Only a lowerable target is judged here; a `#set` on any other target is refused by the
/// reader's own skip path.
fn set_refusal_reason(target: &str, args: &str) -> Option<String> {
	if !LOWERABLE_SET_TARGETS.iter().any(|t| *t == target) {
		return None;
	}
	let list = lex::args(args);
	if list.is_empty() {
		return Some("applied no argument".to_string());
	}
	if let Some(key) = lex::duplicate_key(&list) {
		return Some(fmt!("names {} twice, so none of it is applied", key));
	}
	let mut patch	= ThemePatch::default();
	let used		= lower_set_into(target, &list, &mut patch, DEFAULT_BODY_PT);
	let mut leftover: Vec<&str> = list.iter()
		.filter_map(|a| a.key.as_deref())
		.filter(|k| !used.iter().any(|u| u == k))
		.collect();
	if list.iter().any(|a| a.key.is_none()) {
		leftover.push("a positional argument");
	}
	if leftover.is_empty() {
		None
	} else {
		Some(fmt!("left unapplied: {}", leftover.join(", ")))
	}
}

/// If a captured declarative-styling construct is a `#set` on a theme element that lowered to nothing --
/// applying no argument, or hitting an unrecognised or unconvertible one -- the construct name to record
/// as a refusal and why, so a `#set` that silently did nothing becomes a visible refusal (H2). `None` for
/// a `#set` that fully lowered, and for a `#show: <t>.with(...)` (whose non-theme arguments are the book's
/// front matter, not a no-op). The reader calls this as it dispatches a captured `DeclStyle` construct.
pub fn declstyle_refusal(buf: &str) -> Option<(String, String)> {
	let (target, args) = match top_level_sets(buf).into_iter().next() {
		Some(set)	=> set,
		None		=> return None,
	};
	set_refusal_reason(&target, &args).map(|why| (fmt!("#set {}", target), why))
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ EXTRACTING A CONSTRUCT'S ARGUMENTS FROM SOURCE                             │
// └───────────────────────────────────────────────────────────────────────────┘

/// The balanced argument text of the first top-level `#show: <ident>.with(...)` application in `src`,
/// without its enclosing parentheses, and the span of the line it opens on. `None` when the source has no
/// such application at its top level.
pub(crate) fn show_doc_with(src: &str) -> Option<(String, Span)> {
	for (start, raw) in crate::lang::lex::top_level_lines(src) {
		let indent	= raw.len() - raw.trim_start().len();
		let trimmed	= raw.trim_start();
		if !trimmed.starts_with("#show:") {
			continue;
		}
		// The application is `#show: <ident>.with(` -- the `.with(` follows on the same line.
		if let Some(wrel) = trimmed.trim_end().find(".with(") {
			let open = start + indent + wrel + ".with".len();	// the '(' of the argument list
			let end = start + raw.trim_end().len();
			return balanced_parens(&src[open..]).map(|args| (args, Span::new(start as u32, end as u32)));
		}
	}
	None
}

fn show_doc_with_args(src: &str) -> Option<String> {
	show_doc_with(src).map(|(args, _)| args)
}

/// The heading face a source's own `#show: <template>.with(heading-font: ...)` names, with the span of the
/// line the application opens on, so a note about that face is charged to the declaration that named it.
pub fn heading_font_site(src: &str) -> Option<(String, Span)> {
	show_doc_with(src).and_then(|(args, span)|
		named_string(&lex::args(&args), "heading-font").filter(|f| !f.is_empty()).map(|f| (f, span)))
}

/// Every top-level `#set <target>(...)` in `src`, as `(target, args)` pairs with the argument text
/// stripped of its enclosing parentheses: a line of [`crate::lang::lex::top_level_lines`] whose trimmed text opens with
/// `#set `. A malformed set (no balanced parentheses) is skipped.
///
/// The `(`'s position is found by tracking the running byte offset of each line rather than by searching
/// `src` for the line's text: two `#set text(...)` lines with the same target read the same after
/// `#set `, so a search would resolve the second to the first's arguments. The offset is exact, so the
/// balanced scan starts at this line's own `(` and reads its own arguments, even when they run on across
/// several following lines.
fn top_level_sets(src: &str) -> Vec<(String, String)> {
	crate::lang::lex::top_level_lines(src).into_iter()
		.filter_map(|(line_start, raw)| set_on_line(src, line_start, raw))
		.collect()
}

/// The `#set <target>(...)` the line `raw` of `src`, which starts at byte `line_start`, opens, as
/// `(target, args)`. `None` for a line that opens none, and for a malformed one (no balanced parentheses).
fn set_on_line(src: &str, line_start: usize, raw: &str) -> Option<(String, String)> {
	let indent	= raw.len() - raw.trim_start().len();	// leading-whitespace bytes
	let trimmed	= raw.trim_start();
	let Some(after) = trimmed.strip_prefix("#set ") else {
		return None;
	};
	let rest_ws	= after.len() - after.trim_start().len();	// whitespace between `#set ` and the target
	let rest	= after.trim_start();
	let Some(open) = rest.find('(') else {
		return None;
	};
	let target = rest[..open].trim().to_string();
	if target.is_empty() {
		return None;
	}
	// The byte offset of this line's own `(`, so the balanced scan reads this set's arguments -- which
	// may run past the line's end -- rather than an earlier identical prefix's.
	let abs = line_start + indent + "#set ".len() + rest_ws + open;
	balanced_parens(&src[abs..]).map(|args| (target, args))
}

/// Every `#set document(...)` that opens a line of `src` in markup, as the line's place, the byte of its `#` and
/// its argument text, in source order. A line inside code, a string, a comment or an expression opens none.
fn document_sets(src: &str) -> Vec<(Place, usize, String)> {
	let mut out = Vec::new();
	for (line_start, raw, place) in crate::lang::lex::placed_lines(src) {
		if place == Place::Content {
			continue;
		}
		if let Some((target, args)) = set_on_line(src, line_start, raw) {
			if target == "document" {
				out.push((place, line_start + (raw.len() - raw.trim_start().len()), args));
			}
		}
	}
	out
}

/// The text inside a balanced `(...)` at the start of `s` (which must begin with `(`). Delegates to the
/// reader's content-aware group scanner ([`crate::lang::parse::read_group`]), so a `(` an author left
/// unbalanced inside a `[...]` content block or a `"..."` string does not throw off the count -- the
/// naive byte counter this replaced miscounted a `set page(header: [p (1)])`. `None` when the
/// parentheses never close.
fn balanced_parens(s: &str) -> Option<String> {
	let chars: Vec<char> = s.chars().collect();
	if chars.first() != Some(&'(') {
		return None;
	}
	crate::lang::parse::read_group(&chars, 0).map(|(inner, _)| inner)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ READING ONE NAMED ARGUMENT                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// The inner text of `raw` when it is one group, opened by `open` and closed at its very end, as the
/// lexer reads it; `None` for anything else, such as `[a] + [b]`.
fn whole_group(raw: &str, open: char) -> Option<&str> {
	if !raw.starts_with(open) || lex::group_end(raw, 0) != Some(raw.len()) {
		return None;
	}
	let close = raw.len() - raw.chars().next_back().map_or(0, |c| c.len_utf8());
	Some(&raw[open.len_utf8()..close])
}

/// The string a `key: "..."` or `key: [...]` names: the string's value, its escapes resolved, or the
/// content block's markup, trimmed. `None` when the key is absent or its value is neither.
fn named_string(list: &[lex::Arg], key: &str) -> Option<String> {
	let v = match lex::named(list, key) {
		Some(v)	=> v,
		None	=> return None,
	};
	if let Some(s) = string_value(v) {
		return Some(s);
	}
	whole_group(v, '[').map(|inner| inner.trim().to_string())
}

/// The family list a `font:` value names: a lone string, or an array of strings (Typst's fallback list,
/// tried in order). `None` for anything else -- a variable, a dictionary form, an empty name -- so the
/// argument stays unapplied and the `#set` is refused rather than set wrongly.
fn font_families(expr: &str) -> Option<Vec<String>> {
	let items: Vec<lex::Arg> = match whole_group(expr, '(') {
		Some(inner)	=> lex::args(inner),
		None		=> vec![lex::Arg { key: None, value: expr.to_string() }],
	};
	let mut out: Vec<String> = Vec::new();
	for item in items {
		if item.key.is_some() {
			return None;
		}
		match string_value(&item.value) {
			Some(name) if !name.trim().is_empty()	=> out.push(name.trim().to_string()),
			_										=> return None,
		}
	}
	if out.is_empty() { None } else { Some(out) }
}

/// A `#set document(...)` field as its Info entry: `Some(None)` for `none`, else a string, a content
/// block's plain text, or an array of strings joined with ", " as Typst joins an author or keyword list.
/// `None` when the key is absent or its value is one the reader cannot evaluate.
fn document_field(list: &[lex::Arg], key: &str) -> Option<Option<String>> {
	let raw = match lex::named(list, key) {
		Some(r)	=> r,
		None	=> return None,
	};
	if raw == "none" {
		return Some(None);
	}
	if let Some(s) = string_value(raw) {
		return Some(Some(s));
	}
	if let Some(markup) = whole_group(raw, '[') {
		let plain = crate::doc::flatten_segments(&crate::lang::inline_segments(markup.trim()));
		return Some(Some(plain));
	}
	if let Some(inner) = whole_group(raw, '(') {
		let mut items: Vec<String> = Vec::new();
		for item in lex::args(inner) {
			if item.key.is_some() {
				return None;	// a dictionary, not a list of names
			}
			match string_value(&item.value) {
				Some(s)	=> items.push(s),
				None	=> return None,
			}
		}
		return Some(if items.is_empty() { None } else { Some(items.join(", ")) });
	}
	None
}

/// The value of a `"..."` string literal filling the whole of `expr`, its escapes resolved, or `None`
/// when `expr` is not one.
fn string_value(expr: &str) -> Option<String> {
	let inner = match expr.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
		Some(i)	=> i,
		None	=> return None,
	};
	let mut out		= String::with_capacity(inner.len());
	let mut chars	= inner.chars();
	while let Some(c) = chars.next() {
		match c {
			'"'		=> return None,	// an unescaped quote ends the literal early, so this is not one
			'\\'	=> match chars.next() {
				Some('\\')	=> out.push('\\'),
				Some('"')	=> out.push('"'),
				Some('n')	=> out.push('\n'),
				Some('r')	=> out.push('\r'),
				Some('t')	=> out.push('\t'),
				Some('u')	=> {
					let hex: String = chars.by_ref().skip_while(|c| *c == '{').take_while(|c| *c != '}').collect();
					match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
						Some(ch)	=> out.push(ch),
						None		=> return None,
					}
				},
				_			=> return None,
			},
			other	=> out.push(other),
		}
	}
	Some(out)
}

/// The boolean a `key: true`/`key: false` names. `None` when absent or not a boolean literal.
fn named_bool(list: &[lex::Arg], key: &str) -> Option<bool> {
	match lex::named(list, key) {
		Some("true")	=> Some(true),
		Some("false")	=> Some(false),
		_				=> None,
	}
}

/// A `key:`'s value read whole as a number and the unit written directly after it (`pt`, `mm`, `em`, or
/// empty). `None` when the key is absent or its value is anything else, such as `50%` or `12pt * 2`.
fn named_number_unit(list: &[lex::Arg], key: &str) -> Option<(f64, String)> {
	let v = match lex::named(list, key) {
		Some(v)	=> v,
		None	=> return None,
	};
	let mut end			= 0usize;
	let mut seen_dot	= false;
	for (i, c) in v.char_indices() {
		if c.is_ascii_digit() || (c == '-' && i == 0) {
			end = i + c.len_utf8();
		} else if c == '.' && !seen_dot {
			seen_dot = true;
			end = i + c.len_utf8();
		} else {
			break;
		}
	}
	let unit = &v[end..];
	if end == 0 || !unit.chars().all(|c| c.is_ascii_alphabetic()) {
		return None;
	}
	match v[..end].parse::<f64>() {
		Ok(num)	=> Some((num, unit.to_string())),
		Err(_)	=> None,
	}
}

/// The point value of a `key:`'s length, accepting a bare number or one suffixed `pt`. An `em` or `mm`
/// value returns `None` so the field is left unchanged rather than set wrongly; where a field is legally
/// written in ems (a paragraph metric), the caller uses [`named_length_pt_em`] with the body size instead.
fn named_length_pt(list: &[lex::Arg], key: &str) -> Option<f64> {
	match named_number_unit(list, key) {
		Some((num, unit)) if unit.is_empty() || unit == "pt"	=> Some(num),
		_														=> None,
	}
}

/// The point value of a `key:`'s length, accepting a bare number, `pt`, or a font-relative `em` resolved
/// against `em_base_pt` (the text size in force). `mm` is still `None` here -- a paragraph metric is never
/// set in millimetres, and leaving it unconverted keeps such a `#set` a visible refusal rather than a
/// wrong write. Used where a metric may legitimately be written in ems, unlike [`named_length_pt`].
fn named_length_pt_em(list: &[lex::Arg], key: &str, em_base_pt: f64) -> Option<f64> {
	match named_number_unit(list, key) {
		Some((num, unit)) if unit.is_empty() || unit == "pt"	=> Some(num),
		Some((num, unit)) if unit == "em"						=> Some(num * em_base_pt),
		_														=> None,
	}
}

const MM_PER_PT: f64 = 72.0 / 25.4;	// points in one millimetre

/// The scaled-point length of a `key:`'s value, accepting `pt` or `mm` (a page dimension is usually set
/// in millimetres). An `em` value is left to a later unit that knows the body size.
fn named_length_mm_or_pt(list: &[lex::Arg], key: &str) -> Option<Sp> {
	match named_number_unit(list, key) {
		Some((num, unit)) if unit == "pt"	=> Some(Sp::from_pt(num)),
		Some((num, unit)) if unit == "mm"	=> Some(Sp::from_pt(num * MM_PER_PT)),
		_									=> None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The Info fields `src` names, the sites its container rules are refused at left to the caller's tests.
	fn info_of(src: &str) -> crate::doc::DocInfo {
		document_info(src, "t.typ", &mut Refusals::default())
	}

	/// `#show: doc.with(heading-font: "...")` lowers the heading face into levels 1 and 2 (the doc
	/// template's per-level rule), leaving deeper levels and the rest of the theme at their defaults, so a
	/// document that names only a heading font changes only those two levels' face.
	#[test]
	fn doc_with_lowers_the_heading_font() {
		let mut theme = Theme::default();
		let src = "#import \"template.typ\": *\n#show: doc.with(\n  title: [X],\n  heading-font: \"Graystroke\",\n)\n\n= Body\n";
		lower_root_declarations(src, &mut theme);
		assert_eq!(theme.heading.levels[0].face, Some("Graystroke".to_string()));
		assert_eq!(theme.heading.levels[1].face, Some("Graystroke".to_string()));
		// The body family below level 2, and the rest of the theme, are untouched.
		assert_eq!(theme.heading.levels[2].face, None);
		assert_eq!(theme.heading.face, None);
		assert_eq!(theme.text.body_size, Theme::default().text.body_size);
	}

	/// A lowerable `#set text(size: ...)` lowers to a patch that writes the body size and body face; an
	/// omitted argument leaves its field unnamed, so applying the patch leaves the theme's own value.
	#[test]
	fn set_text_lowers_size_and_font() {
		let patch = lower_set("text", "size: 12pt, font: \"Libertinus Serif\"");
		let mut theme = Theme::default();
		theme.apply(&patch);
		assert_eq!(theme.text.body_size, Sp::from_pt(12.0));
		assert_eq!(theme.text.faces.body, vec!["Libertinus Serif".to_string()]);
		// A fallback array lowers to the list in order.
		let listed = lower_set("text", "font: (\"Felipa\", \"Libertinus Serif\",)");
		assert_eq!(listed.text.faces.body, Some(vec!["Felipa".to_string(), "Libertinus Serif".to_string()]));
		// A value that is not a literal family is left unapplied, so the set is refused.
		assert_eq!(lower_set("text", "font: fonts.display").text.faces.body, None);
		// The leading was not named, so it kept its default.
		assert_eq!(theme.text.leading, Theme::default().text.leading);
	}

	/// `#set page(columns: n)` lowers the body's column count and `#set columns(gutter: ..)` the space between
	/// two, both consumed, so neither is refused; a count that is not a whole number is left unapplied.
	#[test]
	fn set_page_columns_and_gutter_lower() {
		assert_eq!(lower_set("page", "columns: 2").page.columns, Some(2));
		assert_eq!(set_refusal_reason("page", "columns: 2"), None);
		assert_eq!(lower_set("columns", "gutter: 12pt").page.column_gutter, Some(crate::ir::Length::Abs(12.0)));
		assert_eq!(lower_set("columns", "gutter: 5%").page.column_gutter, Some(crate::ir::Length::Rel(0.05)));
		assert!(set_refusal_reason("page", "columns: auto").is_some());
	}

	/// A `#set par(...)` lowers its pure-length metrics (`spacing`, `first-line-indent`) in `em` against the
	/// text size IN FORCE AT LAYOUT, the template idiom (`#set par(spacing: 0.75em)`) that was silently
	/// dropped before `em` was convertible. Typst resolves the `em` lazily -- the batch's final `text(size:)`
	/// wins whether it precedes OR follows the `#set par` -- so source order does not change the result.
	/// `leading` is deliberately NOT converted from `em` here: it is a line-box gap, not a baseline distance,
	/// so it stays a visible refusal rather than a wrong write (see the `par` arm and [`named_length_pt_em`]).
	#[test]
	fn set_par_lowers_em_pure_lengths_against_layout_text_size() {
		// A lone `#set par` resolves spacing/indent em against the 11 pt house default; an em leading is left
		// unlowered (its baseline conversion needs the calibrated line box, deferred to a leading pass).
		let patch = lower_set("par", "leading: 0.78em, spacing: 0.75em, first-line-indent: 1.5em");
		assert_eq!(patch.par.skip,     Some(Sp::from_pt(0.75 * 11.0)));
		assert_eq!(patch.par.indent,   Some(Sp::from_pt(1.5  * 11.0)));
		assert_eq!(patch.text.leading, None);

		// `text(size:)` BEFORE the `par` -- the em resolves against 12 pt.
		let mut theme = Theme::default();
		lower_root_declarations("#set text(size: 12pt)\n#set par(spacing: 0.75em)\n\n= Body\n", &mut theme);
		assert_eq!(theme.par.skip, Sp::from_pt(0.75 * 12.0));

		// `text(size:)` AFTER the `par` -- Typst resolves the em lazily against the size in force at layout,
		// so the final 20 pt still wins and the result is order-independent (order.typ/order2.typ are
		// byte-identical under typst 0.15.1).
		let mut theme = Theme::default();
		lower_root_declarations("#set par(spacing: 1em)\n#set text(size: 20pt)\n\n= Body\n", &mut theme);
		assert_eq!(theme.par.skip, Sp::from_pt(20.0));

		// No `text(size:)` in the batch -- the em resolves against the size the theme already carries, not the
		// raw house default (here a 12 pt theme).
		let mut theme = Theme::default();
		theme.text.body_size = Sp::from_pt(12.0);
		lower_root_declarations("#set par(spacing: 1em)\n\n= Body\n", &mut theme);
		assert_eq!(theme.par.skip, Sp::from_pt(12.0));

		// A `pt` spacing is still taken verbatim, and a `pt` leading keeps its pre-existing pass-through.
		assert_eq!(lower_set("par", "spacing: 9pt").par.skip,      Some(Sp::from_pt(9.0)));
		assert_eq!(lower_set("par", "leading: 16pt").text.leading, Some(Sp::from_pt(16.0)));
	}

	/// `#set text(fill: rgb("#ff0000"))` lowers the prose fill, consuming the argument so it is not refused;
	/// the default theme carries black. A palette reference the reader cannot resolve without a palette
	/// (`fill: colours.blue`) leaves the field unset and is flagged for refusal rather than guessed at.
	#[test]
	fn set_text_lowers_fill_colour() {
		use oxedyne_fe2o3_graphics::colour::Rgba;

		// The default is black, so an unset document renders exactly as before text carried a colour.
		assert_eq!(Theme::default().text.fill, Rgba::BLACK);

		let patch = lower_set("text", "fill: rgb(\"#ff0000\")");
		assert_eq!(patch.text.fill, Some(Rgba::opaque(255, 0, 0)));
		let mut theme = Theme::default();
		theme.apply(&patch);
		assert_eq!(theme.text.fill, Rgba::opaque(255, 0, 0));
		// The fill was consumed, so a `#set text(fill: ...)` is not spuriously refused.
		assert_eq!(set_refusal_reason("text", "fill: rgb(\"#ff0000\")"), None);

		// A `luma`, a named colour, and a lightened named colour all read, so the grammar matches a rule's.
		assert_eq!(lower_set("text", "fill: luma(0)").text.fill, Some(Rgba::opaque(0, 0, 0)));
		assert_eq!(lower_set("text", "fill: red").text.fill, Some(Rgba::opaque(255, 65, 54)));
		assert!(lower_set("text", "fill: black.lighten(50%)").text.fill.is_some());

		// A palette reference resolves to nothing here, so the field is left unset and the set is refused
		// rather than set wrongly.
		assert_eq!(lower_set("text", "fill: colours.blue").text.fill, None);
		assert!(set_refusal_reason("text", "fill: colours.blue").is_some());
	}

	/// `set heading(numbering: "1.1")` lowers to a patch that applies the pattern across every level.
	#[test]
	fn set_heading_numbering_applies_to_all_levels() {
		let patch = lower_set("heading", "numbering: \"1.1\"");
		let mut theme = Theme::default();
		theme.apply(&patch);
		for level in &theme.heading.levels {
			assert_eq!(level.numbering, Some("1.1".to_string()));
		}
	}

	/// A `#set` nested inside a `#styled-box[...]` body is that body's own declaration -- lowered onto its
	/// scope when the body is re-parsed -- not captured at the enclosing source's top level. Only a
	/// genuinely top-level `#set` lowers to this source's scope, so the nested 40pt never reaches it and the
	/// top-level 20pt does (the nesting-aware source scan; without it the flat scan would fold both and the
	/// later 40pt would win).
	#[test]
	fn nested_set_inside_a_bracketed_body_is_not_captured() {
		let src = "#set text(size: 20pt)\n#styled-box[\n#set text(size: 40pt)\nInside the box.\n]\n";
		let patch = lower_declarations(src);
		assert_eq!(patch.text.body_size, Some(Sp::from_pt(20.0)),
			"only the top-level #set should lower here; the box body's #set must not leak out");
	}

	/// A `#set` on a target the theme has no field for lowers to an empty patch -- the caller keeps it a
	/// refusal, and applying the empty patch changes nothing.
	#[test]
	fn set_on_unknown_target_is_not_lowered() {
		let patch = lower_set("rect", "stroke: 1pt");
		assert_eq!(patch, ThemePatch::default());
		let mut theme = Theme::default();
		theme.apply(&patch);
		assert_eq!(theme, Theme::default());
	}

	/// Two top-level `#set` lines whose text reads identically after `#set ` are read by their own byte
	/// offset, so the second's arguments are its own -- not the first's, which a naive `src.find` returned.
	#[test]
	fn top_level_sets_reads_each_lines_own_arguments() {
		let src = "#set text(size: 11pt)\n#set text(size: 13pt)\n";
		let sets = top_level_sets(src);
		assert_eq!(sets.len(), 2);
		assert_eq!(sets[0], ("text".to_string(), "size: 11pt".to_string()));
		assert_eq!(sets[1], ("text".to_string(), "size: 13pt".to_string()));
	}

	/// The named-argument reader does not confuse a suffix key: `font:` is not found inside
	/// `heading-font:`.
	#[test]
	fn key_reader_respects_word_boundaries() {
		assert_eq!(named_string(&lex::args("heading-font: \"A\""), "font"), None);
		assert_eq!(named_string(&lex::args("heading-font: \"A\", font: \"B\""), "font"), Some("B".to_string()));
	}

	/// Balanced-paren extraction spans newlines and ignores a parenthesis inside a string.
	#[test]
	fn balanced_parens_spans_lines_and_skips_strings() {
		let s = "(\n  a: 1,\n  b: \"a)b\",\n  c: (1, 2),\n)tail";
		assert_eq!(balanced_parens(s), Some("\n  a: 1,\n  b: \"a)b\",\n  c: (1, 2),\n".to_string()));
	}

	/// The keys a `#set` names and the keys it applied are read from the same arguments, comments as
	/// trivia: a key nested in a value, written in a comment or found in a string is none of the set's own,
	/// and a field named twice, or given positionally, is refused as Typst refuses it.
	#[test]
	fn a_sets_keys_are_its_own_top_level_arguments() {
		assert_eq!(set_refusal_reason("document", "title: \"T\" /* c */, date: auto"),
			Some("left unapplied: date".to_string()));
		assert_eq!(set_refusal_reason("document", "\n  // author: \"Old\",\n  title: \"T\",\n"), None);
		assert_eq!(set_refusal_reason("document", "title: \"T, date: auto\""), None);
		assert_eq!(set_refusal_reason("document", "title: 1 + 1"), Some("left unapplied: title".to_string()));
		assert_eq!(set_refusal_reason("document", "title: \"A\", title: \"B\""),
			Some("names title twice, so none of it is applied".to_string()));
		assert_eq!(set_refusal_reason("text", "\"x\""), Some("left unapplied: a positional argument".to_string()));
		assert_eq!(set_refusal_reason("text", ""), Some("applied no argument".to_string()));
		assert_eq!(set_refusal_reason("text", "size: 12pt, /* font: \"A\", */"), None);
		// A field named twice applies nothing of the rule.
		assert_eq!(lower_set("text", "size: 9pt, size: 10pt"), ThemePatch::default());
		assert_eq!(info_of("#set document(title: \"A\", title: \"B\")\n"), crate::doc::DocInfo::default());
	}

	/// H2: a lowerable `#set` whose named arguments the renderer does not consume -- an unknown key, an
	/// unconvertible `em`, a bare `none`, or a field lowered but not yet read (an equation
	/// `numbering`, a `page` dimension) -- is flagged for refusal, so it is a visible "not yet supported"
	/// rather than a silent no-op; a set every one of whose arguments the renderer consumes is not.
	#[test]
	fn unconsumed_set_is_flagged_for_refusal() {
		// Fully consumed: no refusal.
		assert_eq!(set_refusal_reason("text", "size: 12pt"), None);
		assert_eq!(set_refusal_reason("par", "leading: 14pt, first-line-indent: 12pt, justify: false"), None);
		assert_eq!(set_refusal_reason("text", "hyphenate: false"), None);
		assert_eq!(set_refusal_reason("heading", "numbering: \"1.1\""), None);
		assert_eq!(set_refusal_reason("enum", "numbering: \"(a)\""), None);
		// An unrecognised argument key.
		assert!(set_refusal_reason("text", "lang: \"de\"").is_some());
		// A recognised key whose value does not convert (an `em` needs a context this lowering has not).
		assert!(set_refusal_reason("text", "size: 1em").is_some());
		// A bare `none` is not a string the numbering reader accepts, so nothing is applied.
		assert!(set_refusal_reason("heading", "numbering: none").is_some());
		// A partially-applied set is still flagged, for the argument it dropped.
		assert!(set_refusal_reason("text", "size: 12pt, weight: 700").is_some());
		// A body font is read by the renderer now, so it is consumed; one that is not a literal family is not.
		assert_eq!(set_refusal_reason("text", "font: \"Radley\""), None);
		assert!(set_refusal_reason("text", "font: fonts.display").is_some());
		// Lowered-but-unread fields are flagged, so a `#set` into one alone is refused rather than a no-op.
		assert!(set_refusal_reason("math.equation", "numbering: \"(1)\"").is_some(),
			"equation numbering lowers but the renderer always sets (N), so it must be refused");
		assert!(set_refusal_reason("page", "width: 200mm").is_some(),
			"page geometry lowers but no unit consumes it yet, so it must be refused");

		// declstyle_refusal drives it off a captured construct buffer, naming the set, and never flags a
		// `#show: doc.with(...)`, whose non-theme arguments are front matter rather than a no-op.
		assert_eq!(declstyle_refusal("#set text(size: 12pt)\n"), None);
		assert_eq!(declstyle_refusal("#set text(lang: \"de\")\n"),
			Some(("#set text".to_string(), "left unapplied: lang".to_string())));
		assert_eq!(declstyle_refusal("#show: doc.with(title: [X])\n"), None);
	}

	#[test]
	fn document_info_reads_each_field_as_typst_writes_it() {
		let src = "#set document(\n\ttitle: [A *bold* Title],\n\tauthor: (\"Ann Author\", \"Bob\"),\n\t\
			description: \"A \\\"quoted\\\" line\",\n\tkeywords: (\"one\", \"two\",),\n)\n= Body\n";
		let info = info_of(src);
		assert_eq!(info.title.as_deref(), Some("A bold Title"), "content reads as its plain text");
		assert_eq!(info.author.as_deref(), Some("Ann Author, Bob"));
		assert_eq!(info.subject.as_deref(), Some("A \"quoted\" line"), "escapes resolve");
		assert_eq!(info.keywords.as_deref(), Some("one, two"));

		assert_eq!(info_of("#set document(author: \"Solo\")\n").author.as_deref(), Some("Solo"));
		assert_eq!(info_of("= Body\n#set text(size: 12pt)\n"), crate::doc::DocInfo::default());

		// A later `#set` overrides field by field, and `none` clears.
		let two = info_of("#set document(title: \"One\", author: \"A\")\n#set document(title: none)\n");
		assert_eq!((two.title, two.author.as_deref()), (None, Some("A")));
	}

	/// A `#set document` at the top level or in a bare content block applies; one in a list item, strong or
	/// emphasis, or after an unclosed `*` or `_`, applies nothing and is refused at its `#`.
	#[test]
	fn set_document_in_a_container_is_refused_and_in_a_bare_block_applies() {
		let mut skips = Refusals::default();
		let src = "= R\n\n#[\n#set document(title: \"Block\")\n]\n";
		assert_eq!(document_info(src, "t.typ", &mut skips).title.as_deref(), Some("Block"));
		assert!(skips.is_empty());
		let cases = [
			"*b [\n#set document(title: \"T\")\nb*\n",
			"_e\n#set document(title: \"T\")\ne_\n",
			"- i\n  #set document(title: \"T\")\n",
			"2 * 3\n#set document(title: \"T\")\n",
			"\u{65E5}\u{672C}_\u{8A9E}\n#set document(title: \"T\")\n",
			"- i #[\n#set document(title: \"T\")\n]\n",
		];
		for src in cases {
			let mut skips = Refusals::default();
			let info = document_info(src, "t.typ", &mut skips);
			assert_eq!(info, crate::doc::DocInfo::default(), "{:?}", src);
			assert_eq!(skips.sites().len(), 1, "{:?}", src);
			let site = &skips.sites()[0];
			assert_eq!((site.name.as_str(), site.class, site.file.as_str()), ("#set document", RefusalClass::Unsupported, "t.typ"));
			assert_eq!(&src[site.span.start as usize..][..4], "#set", "{:?}", src);
		}
		// A file's part sits `at` bytes into it, and its site is placed there.
		let mut skips = Refusals::default();
		fold_document_info("- i\n  #set document(title: \"T\")\n", &mut crate::doc::DocInfo::default(), "f.typ", 100, &mut skips);
		assert_eq!(skips.sites()[0].span.start, 100 + 6);
	}

	/// A rule in a list item, strong or emphasis ends with it, so none is lowered for the document.
	#[test]
	fn a_rule_in_an_item_strong_or_emphasis_is_not_lowered_for_the_document() {
		let top = lower_declarations("#set heading(numbering: \"1.\")\n= Next\n");
		assert_eq!(top.heading.numbering_all, Some(Some("1.".to_string())));
		for src in [
			"- item\n  #set heading(numbering: \"1.\")\n= Next\n",
			"*bold\n#set heading(numbering: \"1.\")\nstill*\n\n= Next\n",
			"_emph\n#set heading(numbering: \"1.\")\nstill_\n\n= Next\n",
			"- item\n  #show: doc.with(heading-font: \"Old\")\n",
		] {
			assert_eq!(lower_declarations(src), ThemePatch::default(), "{:?}", src);
		}
	}

	/// A field counts as applied only when its value was read, so one the reader cannot evaluate is refused
	/// rather than silently missing from the Info dictionary.
	#[test]
	fn set_document_refuses_what_it_cannot_read() {
		assert_eq!(set_refusal_reason("document", "title: \"X\", author: (\"Y\", \"Z\")"), None);
		assert_eq!(set_refusal_reason("document", "title: \"X\", date: none"), None);
		assert!(set_refusal_reason("document", "title: my-title").is_some(), "a variable");
		assert!(set_refusal_reason("document", "keywords: (\"a\", b)").is_some(), "a non-string item");
		assert!(set_refusal_reason("document", "date: auto").is_some(), "a date is never written");
		assert!(set_refusal_reason("document", "title: \"X\", lang: \"de\"").is_some());
		assert_eq!(declstyle_refusal("#set document(title: \"X\")\n"), None);
	}

	/// A `#set` shown in a raw block or written in a comment is text, not a rule: neither the theme nor the
	/// Info dictionary reads one, while a declaration after them still applies.
	#[test]
	fn raw_blocks_and_comments_hold_no_declarations() {
		let src = "```typst\n#set document(title: \"Example Title\")\n#set text(size: 30pt)\n```\n\
			/*\n#set document(title: \"Commented Out\")\n#set text(size: 31pt)\n*/\n\
			#set text(size: 12pt)\n= Body\n";
		assert_eq!(info_of(src), crate::doc::DocInfo::default());
		assert_eq!(lower_declarations(src).text.body_size, Some(Sp::from_pt(12.0)));
		// A comment opened after a declaration holds the lines after it, not the one it opens on.
		let src = "#set text(size: 12pt) /* the old size:\n#set text(size: 30pt)\n*/\n";
		assert_eq!(lower_declarations(src).text.body_size, Some(Sp::from_pt(12.0)));
	}

	/// A file's top level is markup: a paren, brace or quotation mark in its prose is a character, so a
	/// `#set` on a later line still applies.
	#[test]
	fn prose_punctuation_leaves_a_later_set_at_the_top_level() {
		let src = "A ruler (and {more, 12\" long\n\n#set text(size: 13pt)\n";
		assert_eq!(lower_declarations(src).text.body_size, Some(Sp::from_pt(13.0)));
	}

	/// The template application is read from the top level alone, so one commented out or shown in a raw
	/// block names no heading face.
	#[test]
	fn a_shown_or_commented_template_application_is_not_applied() {
		let src = "// #show: doc.with(heading-font: \"Old\")\n```\n#show: doc.with(heading-font: \"Shown\")\n```\n\
			#show: doc.with(heading-font: \"Real\")\n";
		let mut theme = Theme::default();
		lower_root_declarations(src, &mut theme);
		assert_eq!(theme.heading.levels[0].face, Some("Real".to_string()));
	}
}
