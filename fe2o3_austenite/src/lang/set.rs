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
use crate::theme::Theme;

use oxedyne_fe2o3_core::prelude::*;

/// Lowers a source's own top-level declarations onto `theme`: its `#show: <template>.with(...)`
/// application, then each top-level `#set <target>(...)` the theme carries a field for. Values a
/// construct does not name are left as the theme already holds them, so a document that sets little
/// changes little. This reads only the root's own declarations, not those of its includes.
pub fn lower_root_declarations(src: &str, theme: &mut Theme) {
	if let Some(args) = show_doc_with_args(src) {
		lower_doc_with(&args, theme);
	}
	for (target, args) in top_level_sets(src) {
		// A target the theme has no field for returns false; the reader keeps it a refusal, so nothing is
		// silently dropped here.
		let _ = lower_set(&target, &args, theme);
	}
}

/// Lowers a `#show: <template>.with(...)` application's named arguments onto `theme`. Only the
/// styling-relevant arguments map to theme fields -- `heading-font` names the heading face -- and the
/// rest (title, subtitle, logos, meta-data) are the book's front matter, read on the book's own path.
/// An argument this does not recognise is left alone rather than guessed at.
pub fn lower_doc_with(args: &str, theme: &mut Theme) {
	if let Some(font) = named_string(args, "heading-font") {
		if !font.is_empty() {
			theme.text.faces.heading = Some(font);
		}
	}
}

/// Lowers a top-level `#set <target>(...)` onto `theme`. Returns `true` when `target` is an element the
/// theme carries and the set was applied, `false` when it has no theme field -- the caller then leaves
/// that `#set` a refusal rather than silently dropping it. A named argument the set omits leaves that
/// field unchanged.
pub fn lower_set(target: &str, args: &str, theme: &mut Theme) -> bool {
	match target {
		"text" => {
			if let Some(pt) = named_length_pt(args, "size") {
				theme.text.body_size = Sp::from_pt(pt);
			}
			if let Some(font) = named_string(args, "font") {
				if !font.is_empty() {
					theme.text.faces.body = Some(font);
				}
			}
			if let Some(b) = named_bool(args, "hyphenate") {
				theme.text.hyphenate = b;
			}
			true
		},
		"par" => {
			if let Some(pt) = named_length_pt(args, "leading") {
				theme.text.leading = Sp::from_pt(pt);
			}
			if let Some(pt) = named_length_pt(args, "spacing") {
				theme.par.skip = Sp::from_pt(pt);
			}
			if let Some(pt) = named_length_pt(args, "first-line-indent") {
				theme.par.indent = Sp::from_pt(pt);
			}
			if let Some(b) = named_bool(args, "justify") {
				theme.text.justify = b;
			}
			true
		},
		"heading" => {
			// `numbering` applies across the levels, the way Typst's own `set heading(numbering: ...)` does.
			if let Some(pattern) = named_string(args, "numbering") {
				let pat = if pattern.is_empty() { None } else { Some(pattern) };
				for level in &mut theme.heading.levels {
					level.numbering = pat.clone();
				}
			}
			true
		},
		"list" => {
			if let Some(pt) = named_length_pt(args, "spacing") {
				theme.list.item_skip = Sp::from_pt(pt);
			}
			if let Some(pt) = named_length_pt(args, "indent") {
				theme.list.marker_gap = Sp::from_pt(pt);
			}
			true
		},
		"enum" => {
			if let Some(pt) = named_length_pt(args, "spacing") {
				theme.enumeration.item_skip = Sp::from_pt(pt);
			}
			if let Some(pt) = named_length_pt(args, "indent") {
				theme.enumeration.marker_gap = Sp::from_pt(pt);
			}
			if let Some(pattern) = named_string(args, "numbering") {
				theme.enumeration.numbering = if pattern.is_empty() { None } else { Some(pattern) };
			}
			true
		},
		"math.equation" => {
			if let Some(pattern) = named_string(args, "numbering") {
				theme.equation.numbering = if pattern.is_empty() { None } else { Some(pattern) };
			}
			true
		},
		"page" => {
			// Page geometry lowers onto the body part's reserved override; a later unit consumes it and
			// splits front/body/back. Only the fields a `set page` names are written.
			if let Some(pt) = named_length_mm_or_pt(args, "width") {
				theme.page.body.width = Some(pt);
			}
			if let Some(pt) = named_length_mm_or_pt(args, "height") {
				theme.page.body.height = Some(pt);
			}
			true
		},
		_ => false,
	}
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
fn top_level_sets(src: &str) -> Vec<(String, String)> {
	let mut out = Vec::new();
	for line in src.lines() {
		let trimmed = line.trim_start();
		let rest = match trimmed.strip_prefix("#set ") {
			Some(r)	=> r.trim_start(),
			None	=> continue,
		};
		let open = match rest.find('(') {
			Some(i)	=> i,
			None	=> continue,
		};
		let target = rest[..open].trim().to_string();
		if target.is_empty() {
			continue;
		}
		// The argument text may run past this line; scan from the '(' across the whole source tail.
		let abs = match src.find(rest) {
			Some(i)	=> i + open,
			None	=> continue,
		};
		if let Some(args) = balanced_parens(&src[abs..]) {
			out.push((target, args));
		}
	}
	out
}

/// The text inside a balanced `(...)` at the start of `s` (which must begin with `(`), skipping over
/// double-quoted strings so a parenthesis inside a string does not unbalance the count. `None` when the
/// parentheses never close.
fn balanced_parens(s: &str) -> Option<String> {
	let bytes		= s.as_bytes();
	if bytes.first() != Some(&b'(') {
		return None;
	}
	let mut depth	= 0i32;
	let mut in_str	= false;
	let mut escaped	= false;
	let mut i		= 0usize;
	while i < bytes.len() {
		let c = bytes[i];
		if in_str {
			if escaped {
				escaped = false;
			} else if c == b'\\' {
				escaped = true;
			} else if c == b'"' {
				in_str = false;
			}
		} else {
			match c {
				b'"'	=> in_str = true,
				b'('	=> depth += 1,
				b')'	=> {
					depth -= 1;
					if depth == 0 {
						return Some(s[1..i].to_string());
					}
				},
				_		=> {},
			}
		}
		i += 1;
	}
	None
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

	/// `#show: doc.with(heading-font: "...")` lowers the heading face and leaves the rest of the theme at
	/// its defaults, so a document that names only a heading font changes only that.
	#[test]
	fn doc_with_lowers_the_heading_font() {
		let mut theme = Theme::default();
		let src = "#import \"template.typ\": *\n#show: doc.with(\n  title: [X],\n  heading-font: \"Graystroke\",\n)\n\n= Body\n";
		lower_root_declarations(src, &mut theme);
		assert_eq!(theme.text.faces.heading, Some("Graystroke".to_string()));
		// A default field the application did not name is untouched.
		assert_eq!(theme.text.body_size, Theme::default().text.body_size);
	}

	/// A lowerable `#set text(size: ...)` writes the body size; an omitted argument leaves its field.
	#[test]
	fn set_text_lowers_size_and_font() {
		let mut theme = Theme::default();
		assert!(lower_set("text", "size: 12pt, font: \"Libertinus Serif\"", &mut theme));
		assert_eq!(theme.text.body_size, Sp::from_pt(12.0));
		assert_eq!(theme.text.faces.body, Some("Libertinus Serif".to_string()));
	}

	/// `set heading(numbering: "1.1")` applies the pattern across every level.
	#[test]
	fn set_heading_numbering_applies_to_all_levels() {
		let mut theme = Theme::default();
		assert!(lower_set("heading", "numbering: \"1.1\"", &mut theme));
		for level in &theme.heading.levels {
			assert_eq!(level.numbering, Some("1.1".to_string()));
		}
	}

	/// A `#set` on a target the theme has no field for is not lowered -- the caller keeps it a refusal.
	#[test]
	fn set_on_unknown_target_is_not_lowered() {
		let mut theme = Theme::default();
		assert!(!lower_set("rect", "stroke: 1pt", &mut theme));
		assert_eq!(theme, Theme::default());
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
}
