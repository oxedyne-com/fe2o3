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
];

/// Lowers a source's own top-level declarations onto `theme`: its `#show: <template>.with(...)`
/// application, then each top-level `#set <target>(...)` the theme carries a field for. Values a
/// construct does not name are left as the theme already holds them, so a document that sets little
/// changes little. This reads only the root's own declarations, not those of its includes, and applies
/// them at the document scope -- the whole theme the driver renders with.
pub fn lower_root_declarations(src: &str, theme: &mut Theme) {
	theme.apply(&lower_declarations(src));
}

/// The [`ThemePatch`] a source's own top-level declarations lower to, without applying it: a `#show:
/// <template>.with(...)` application, then each top-level `#set <target>(...)` the theme carries a field
/// for, folded into one patch in source order so a later `#set` overrides an earlier one. Returned rather
/// than applied so a caller can fold it at the scope it governs -- the document, an included chapter's
/// subtree, or a `#styled-box` body. This reads only the source's own declarations, not those of any
/// file it includes.
pub fn lower_declarations(src: &str) -> ThemePatch {
	let mut patch = ThemePatch::default();
	if let Some(args) = show_doc_with_args(src) {
		lower_doc_with_into(&args, &mut patch);
	}
	for (target, args) in top_level_sets(src) {
		// A target the theme has no field for writes nothing; the reader keeps such a `#set` a refusal, so
		// nothing is silently dropped here.
		lower_set_into(&target, &args, &mut patch);
	}
	patch
}

/// The [`ThemePatch`] a `#show: <template>.with(...)` application's named arguments lower to. Only the
/// styling-relevant arguments map to theme fields -- `heading-font` names the heading face -- and the
/// rest (title, subtitle, logos, meta-data) are the book's front matter, read on the book's own path.
/// An argument this does not recognise is left alone rather than guessed at.
pub fn lower_doc_with(args: &str) -> ThemePatch {
	let mut patch = ThemePatch::default();
	lower_doc_with_into(args, &mut patch);
	patch
}

fn lower_doc_with_into(args: &str, patch: &mut ThemePatch) {
	if let Some(font) = named_string(args, "heading-font") {
		if !font.is_empty() {
			// The doc template applies the heading font to levels 1 and 2 only, the body family below (its
			// per-level show rule: `font: if it.level <= 2 { heading-font } else { "Libertinus Serif" }`).
			// Lower it into those two levels' `face`, which the renderer resolves and applies, rather than
			// the role-default `text.faces.heading` nothing read.
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
	lower_set_into(target, args, &mut patch);
	patch
}

fn lower_set_into(target: &str, args: &str, patch: &mut ThemePatch) -> Vec<&'static str> {
	// The argument keys this set applied AND the renderer consumes -- the invariant is that every lowered
	// field is either read by the renderer or refused with a diagnostic, never written-and-ignored. A key
	// that lowers into a field nothing reads yet (a body `font`, an equation `numbering`, a `page`
	// dimension) is deliberately NOT pushed here, so the refusal check ([`set_refusal_reason`]) sees it as
	// unapplied and records a visible "not yet supported" rather than a silent no-op. A key present in the
	// source but absent for any other reason (unrecognised for the target, an `em` length, a bare `none`)
	// is likewise not pushed.
	let mut used: Vec<&'static str> = Vec::new();
	match target {
		"text" => {
			if let Some(pt) = named_length_pt(args, "size") {
				patch.text.body_size = Some(Sp::from_pt(pt));
				used.push("size");
			}
			// The body family lowers into the theme, but no renderer reads `text.faces.body` yet (the face
			// resolver reaches heading faces only), so `font` is left unmarked and a lone `#set text(font:)`
			// is refused rather than silently ignored.
			if let Some(font) = named_string(args, "font") {
				if !font.is_empty() {
					patch.text.faces.body = Some(Some(font));
				}
			}
			if let Some(b) = named_bool(args, "hyphenate") {
				patch.text.hyphenate = Some(b);
				used.push("hyphenate");
			}
		},
		"par" => {
			if let Some(pt) = named_length_pt(args, "leading") {
				patch.text.leading = Some(Sp::from_pt(pt));
				used.push("leading");
			}
			if let Some(pt) = named_length_pt(args, "spacing") {
				patch.par.skip = Some(Sp::from_pt(pt));
				used.push("spacing");
			}
			if let Some(pt) = named_length_pt(args, "first-line-indent") {
				patch.par.indent = Some(Sp::from_pt(pt));
				used.push("first-line-indent");
			}
			if let Some(b) = named_bool(args, "justify") {
				patch.text.justify = Some(b);
				used.push("justify");
			}
		},
		"heading" => {
			// `numbering` applies across the levels, the way Typst's own `set heading(numbering: ...)` does:
			// one group-level leaf the patch folds onto every level, whatever their count.
			if let Some(pattern) = named_string(args, "numbering") {
				let pat = if pattern.is_empty() { None } else { Some(pattern) };
				patch.heading.numbering_all = Some(pat);
				used.push("numbering");
			}
		},
		"list" => {
			if let Some(pt) = named_length_pt(args, "spacing") {
				patch.list.item_skip = Some(Sp::from_pt(pt));
				used.push("spacing");
			}
			if let Some(pt) = named_length_pt(args, "indent") {
				patch.list.marker_gap = Some(Sp::from_pt(pt));
				used.push("indent");
			}
		},
		"enum" => {
			if let Some(pt) = named_length_pt(args, "spacing") {
				patch.enumeration.item_skip = Some(Sp::from_pt(pt));
				used.push("spacing");
			}
			if let Some(pt) = named_length_pt(args, "indent") {
				patch.enumeration.marker_gap = Some(Sp::from_pt(pt));
				used.push("indent");
			}
			if let Some(pattern) = named_string(args, "numbering") {
				patch.enumeration.numbering = Some(if pattern.is_empty() { None } else { Some(pattern) });
				used.push("numbering");
			}
		},
		"math.equation" => {
			// The equation renderer numbers displays "(N)" unconditionally and reads no pattern yet, so a
			// `numbering` lowers into the theme but is left unmarked -- refused, not silently ignored.
			if let Some(pattern) = named_string(args, "numbering") {
				patch.equation.numbering = Some(if pattern.is_empty() { None } else { Some(pattern) });
			}
		},
		"page" => {
			// Page geometry lowers onto the body part's reserved override, but no unit consumes it yet (the
			// driver still supplies the document geometry), so `width`/`height` are left unmarked and a lone
			// `#set page(...)` is refused as not-yet-supported rather than silently doing nothing.
			if let Some(pt) = named_length_mm_or_pt(args, "width") {
				patch.page.body.default.width = Some(Some(pt));
			}
			if let Some(pt) = named_length_mm_or_pt(args, "height") {
				patch.page.body.default.height = Some(Some(pt));
			}
		},
		_ => {},
	}
	used
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ REFUSING A #set THAT LOWERED TO NOTHING (H2)                               │
// └───────────────────────────────────────────────────────────────────────────┘

/// The top-level argument keys `args` names, in source order: an identifier at depth zero immediately
/// before a `:`. A key nested inside a `(...)`, `[...]` or `"..."` is not top-level, so `header: [x: y]`
/// names only `header`. Used to tell which of a `#set`'s arguments the lowering left unapplied.
fn arg_keys(args: &str) -> Vec<String> {
	let chars:	Vec<char>			= args.chars().collect();
	let mut keys					= Vec::new();
	let mut depth					= 0i32;
	let mut in_str					= false;
	let mut esc						= false;
	// The start of the current top-level token, or `None` once its `:` has been passed, so only the first
	// `:` of a `key: value` names a key and a `:` inside the value is ignored.
	let mut token_start:	Option<usize>	= Some(0);
	let mut i						= 0usize;
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
			'(' | '[' | '{'		=> depth += 1,
			')' | ']' | '}'		=> depth -= 1,
			',' if depth == 0	=> token_start = Some(i + 1),
			':' if depth == 0	=> {
				if let Some(start) = token_start.take() {
					let key: String = chars[start..i].iter().collect();
					let key = key.trim().to_string();
					if !key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.') {
						keys.push(key);
					}
				}
			},
			_					=> {},
		}
		i += 1;
	}
	keys
}

/// Why a lowerable `#set <target>(...)` should be refused rather than pass silently: it applied none of
/// its arguments, or named one the lowering does not recognise or could not convert (an `em` length with
/// no context, a `#set text(lang: ...)`, a `heading(numbering: none)`). `None` when every argument the
/// source named was applied. Only a lowerable target is judged here; a `#set` on any other target is
/// refused by the reader's own skip path.
fn set_refusal_reason(target: &str, args: &str) -> Option<String> {
	if !LOWERABLE_SET_TARGETS.iter().any(|t| *t == target) {
		return None;
	}
	let present			= arg_keys(args);
	let mut patch		= ThemePatch::default();
	let used			= lower_set_into(target, args, &mut patch);
	if present.is_empty() {
		return Some(fmt!("#set {} applied no argument", target));
	}
	let leftover: Vec<String> = present.into_iter()
		.filter(|k| !used.iter().any(|u| *u == k.as_str()))
		.collect();
	if leftover.is_empty() {
		None
	} else {
		Some(fmt!("#set {} left unapplied: {}", target, leftover.join(", ")))
	}
}

/// If a captured declarative-styling construct is a `#set` on a theme element that lowered to nothing --
/// applying no argument, or hitting an unrecognised or unconvertible one -- the construct name to record
/// as a refusal, so a `#set` that silently did nothing becomes a visible refusal (H2). `None` for a `#set`
/// that fully lowered, and for a `#show: <t>.with(...)` (whose non-theme arguments are the book's front
/// matter, not a no-op). The reader calls this as it dispatches a captured `DeclStyle` construct.
pub fn declstyle_refusal(buf: &str) -> Option<String> {
	let (target, args) = top_level_sets(buf).into_iter().next()?;
	set_refusal_reason(&target, &args).map(|_| fmt!("#set {}", target))
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ EXTRACTING A CONSTRUCT'S ARGUMENTS FROM SOURCE                             │
// └───────────────────────────────────────────────────────────────────────────┘

/// The balanced argument text of the first `#show: <ident>.with(...)` application in `src`, without its
/// enclosing parentheses. `None` when the source has no such application.
fn show_doc_with_args(src: &str) -> Option<String> {
	let mut from = 0usize;
	while let Some(rel) = src[from..].find("#show:") {
		let at		= from + rel;
		let rest	= &src[at..];
		// The application is `#show: <ident>.with(` -- find the `.with(` that follows, on the same line.
		let line_end	= rest.find('\n').map(|n| at + n).unwrap_or(src.len());
		if let Some(wrel) = src[at..line_end].find(".with(") {
			let open = at + wrel + ".with".len();	// the '(' of the argument list
			return balanced_parens(&src[open..]);
		}
		from = line_end.max(at + 1);
	}
	None
}

/// Every top-level `#set <target>(...)` in `src`, as `(target, args)` pairs with the argument text
/// stripped of its enclosing parentheses. "Top-level" is by line: a line whose trimmed text opens with
/// `#set `. A malformed set (no balanced parentheses) is skipped.
///
/// The `(`'s position is found by tracking the running byte offset of each line rather than by searching
/// `src` for the line's text: two `#set text(...)` lines with the same target read the same after
/// `#set `, so a search would resolve the second to the first's arguments. The offset is exact, so the
/// balanced scan starts at this line's own `(` and reads its own arguments, even when they run on across
/// several following lines.
fn top_level_sets(src: &str) -> Vec<(String, String)> {
	let mut out		= Vec::new();
	let mut offset	= 0usize;	// running byte offset of the current line's start within `src`
	// The running bracket balance across lines, folded through the reader's own content-aware scanner. A
	// `#set` on a line that opens inside a `#styled-box[...]`/`#columns[...]` body -- a bracket still open at
	// the line's start -- is that body's own declaration, lowered onto its scope when the body is re-parsed;
	// capturing it here too would apply it to the enclosing scope as well. Only a `#set` at true top level
	// (no open bracket) lowers to this source's scope.
	let mut state	= crate::lang::parse::SkipState::new();
	for raw in src.split_inclusive('\n') {
		let line_start	= offset;
		offset			= offset.saturating_add(raw.len());

		// The depth in force at this line's start, before its own delimiters are folded in.
		let nested = state.has_open_bracket();
		crate::lang::parse::scan_brackets(raw, &mut state);
		if nested {
			continue;
		}

		let indent	= raw.len() - raw.trim_start().len();	// leading-whitespace bytes
		let trimmed	= raw.trim_start();
		let after	= match trimmed.strip_prefix("#set ") {
			Some(a)	=> a,
			None	=> continue,
		};
		let rest_ws	= after.len() - after.trim_start().len();	// whitespace between `#set ` and the target
		let rest	= after.trim_start();
		let open	= match rest.find('(') {
			Some(i)	=> i,
			None	=> continue,
		};
		let target = rest[..open].trim().to_string();
		if target.is_empty() {
			continue;
		}
		// The byte offset of this line's own `(`, so the balanced scan reads this set's arguments -- which
		// may run past the line's end -- rather than an earlier identical prefix's.
		let abs = line_start + indent + "#set ".len() + rest_ws + open;
		if let Some(args) = balanced_parens(&src[abs..]) {
			out.push((target, args));
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

/// The byte offset just past a `key:` binding in `args`, matched only where `key` is preceded by a
/// non-identifier character (or the start), so `font:` is not found inside `heading-font:`. `None` when
/// the key is not present.
fn key_value_start(args: &str, key: &str) -> Option<usize> {
	let bytes	= args.as_bytes();
	let mut from	= 0usize;
	while let Some(rel) = args[from..].find(key) {
		let at	= from + rel;
		let before_ok = at == 0 || {
			let p = bytes[at - 1];
			!(p.is_ascii_alphanumeric() || p == b'-' || p == b'_')
		};
		// After the key: optional spaces, then a colon.
		let mut j = at + key.len();
		while j < bytes.len() && bytes[j] == b' ' {
			j += 1;
		}
		if before_ok && j < bytes.len() && bytes[j] == b':' {
			return Some(j + 1);
		}
		from = at + key.len();
	}
	None
}

/// The string a `key: "..."` or `key: [...]` names, without its quotes or brackets, trimmed. `None`
/// when the key is absent or its value is neither a string nor a content block.
fn named_string(args: &str, key: &str) -> Option<String> {
	let start	= key_value_start(args, key)?;
	let rest	= args[start..].trim_start();
	if let Some(inner) = rest.strip_prefix('"') {
		let end = inner.find('"')?;
		return Some(inner[..end].to_string());
	}
	if rest.starts_with('[') {
		let mut depth = 0i32;
		for (i, c) in rest.char_indices() {
			match c {
				'['	=> depth += 1,
				']'	=> {
					depth -= 1;
					if depth == 0 {
						return Some(rest[1..i].trim().to_string());
					}
				},
				_	=> {},
			}
		}
	}
	None
}

/// The boolean a `key: true`/`key: false` names. `None` when absent or not a boolean literal.
fn named_bool(args: &str, key: &str) -> Option<bool> {
	let start	= key_value_start(args, key)?;
	let rest	= args[start..].trim_start();
	if rest.starts_with("true") {
		Some(true)
	} else if rest.starts_with("false") {
		Some(false)
	} else {
		None
	}
}

/// The leading real number of a `key:`'s value, and the unit token immediately after it (`pt`, `mm`,
/// `em`, or empty). `None` when the key is absent or its value does not begin with a number.
fn named_number_unit(args: &str, key: &str) -> Option<(f64, String)> {
	let start	= key_value_start(args, key)?;
	let rest	= args[start..].trim_start();
	let mut end	= 0usize;
	let mut seen_dot = false;
	for (i, c) in rest.char_indices() {
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
	let num: f64 = rest[..end].parse().ok()?;
	let unit: String = rest[end..].chars().take_while(|c| c.is_ascii_alphabetic()).collect();
	Some((num, unit))
}

/// The point value of a `key:`'s length, accepting a bare number or one suffixed `pt`. An `em` or `mm`
/// value is not converted here (a later unit that knows the body size and the millimetre-per-point ratio
/// does that); this returns `None` for those so the field is left unchanged rather than set wrongly.
fn named_length_pt(args: &str, key: &str) -> Option<f64> {
	let (num, unit) = named_number_unit(args, key)?;
	match unit.as_str() {
		"" | "pt"	=> Some(num),
		_			=> None,
	}
}

const MM_PER_PT: f64 = 72.0 / 25.4;	// points in one millimetre

/// The scaled-point length of a `key:`'s value, accepting `pt` or `mm` (a page dimension is usually set
/// in millimetres). An `em` value is left to a later unit that knows the body size.
fn named_length_mm_or_pt(args: &str, key: &str) -> Option<Sp> {
	let (num, unit) = named_number_unit(args, key)?;
	match unit.as_str() {
		"pt"		=> Some(Sp::from_pt(num)),
		"mm"		=> Some(Sp::from_pt(num * MM_PER_PT)),
		_			=> None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

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
		assert_eq!(theme.text.faces.heading, None);
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
		assert_eq!(theme.text.faces.body, Some("Libertinus Serif".to_string()));
		// The leading was not named, so it kept its default.
		assert_eq!(theme.text.leading, Theme::default().text.leading);
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
		assert_eq!(named_string("heading-font: \"A\"", "font"), None);
		assert_eq!(named_string("heading-font: \"A\", font: \"B\"", "font"), Some("B".to_string()));
	}

	/// Balanced-paren extraction spans newlines and ignores a parenthesis inside a string.
	#[test]
	fn balanced_parens_spans_lines_and_skips_strings() {
		let s = "(\n  a: 1,\n  b: \"a)b\",\n  c: (1, 2),\n)tail";
		assert_eq!(balanced_parens(s), Some("\n  a: 1,\n  b: \"a)b\",\n  c: (1, 2),\n".to_string()));
	}

	/// The top-level argument keys are read at depth zero, so a nested `key:` inside a `[...]` value is not
	/// mistaken for one of the set's own arguments.
	#[test]
	fn arg_keys_reads_top_level_keys_only() {
		assert_eq!(arg_keys("size: 12pt, font: \"A\""), vec!["size".to_string(), "font".to_string()]);
		assert_eq!(arg_keys("header: [page: 1]"), vec!["header".to_string()]);
		assert_eq!(arg_keys(""), Vec::<String>::new());
	}

	/// H2: a lowerable `#set` whose named arguments the renderer does not consume -- an unknown key, an
	/// unconvertible `em`, a bare `none`, or a field lowered but not yet read (a body `font`, an equation
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
		// Lowered-but-unread fields are flagged, so a `#set` into one alone is refused rather than a no-op.
		assert!(set_refusal_reason("text", "font: \"Radley\"").is_some(),
			"a body font lowers but no renderer reads it yet, so it must be refused");
		assert!(set_refusal_reason("math.equation", "numbering: \"(1)\"").is_some(),
			"equation numbering lowers but the renderer always sets (N), so it must be refused");
		assert!(set_refusal_reason("page", "width: 200mm").is_some(),
			"page geometry lowers but no unit consumes it yet, so it must be refused");

		// declstyle_refusal drives it off a captured construct buffer, naming the set, and never flags a
		// `#show: doc.with(...)`, whose non-theme arguments are front matter rather than a no-op.
		assert_eq!(declstyle_refusal("#set text(size: 12pt)\n"), None);
		assert_eq!(declstyle_refusal("#set text(lang: \"de\")\n"), Some("#set text".to_string()));
		assert_eq!(declstyle_refusal("#show: doc.with(title: [X])\n"), None);
	}
}
