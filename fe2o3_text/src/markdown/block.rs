//! Block structure: the pass that divides Markdown text into headings, paragraphs, lists, code,
//! quotations and breaks.
//!
//! Markdown is read in two passes, because its two levels are decided by different things. A block is
//! decided by how a line *starts* and by the blank lines around it; an inline run is decided by
//! delimiters within a line. This module does the first pass and hands each block's text to
//! [`crate::markdown::inline`] for the second.

use crate::markdown::{
	Block,
	inline,
};

use oxedyne_fe2o3_core::prelude::*;

/// How deep a document may nest its blocks before the parser refuses it.
///
/// A quotation inside a list inside a quotation is legitimate; a thousand of them is a document built
/// to exhaust the stack of whatever reads it. The limit is generous beside anything an author writes
/// and far below what would trouble the machine.
pub const DEPTH_LIMIT: usize = 32;

/// The columns a tab advances by, counting from a tab stop.
const TAB: usize = 4;

/// The column at which a line is code by its indentation alone.
const CODE_COL: usize = 4;

/// Divides Markdown text into its blocks.
pub fn parse(src: &str) -> Outcome<Vec<Block>> {
	let lines: Vec<&str> = src.lines().collect();
	blocks(&lines, 0)
}

/// What a line begins, judged by how it starts.
///
/// A line's opening is nearly all a block parser needs: the two things it does not settle are a
/// setext underline, which only means anything under a paragraph (see [`under_of`]), and lazy
/// continuation, which the reader of each container decides.
enum Start {
	/// Nothing but whitespace.
	Blank,
	/// An ATX heading: its level, and its text.
	Atx(u8, String),
	/// A code fence.
	Fence {
		/// The character the fence is made of.
		ch:	u8,
		/// How many characters the fence runs to.
		len:	usize,
		/// The column the fence starts at, which its content is stripped back to.
		ind:	usize,
		/// The language the info string named, if it named one.
		lang:	Option<String>,
	},
	/// A thematic break.
	Rule,
	/// A block quotation marker.
	Quote,
	/// A list item marker.
	Item {
		/// Whether the marker is a number.
		ord:	bool,
		/// The bullet, or the delimiter that follows a number.
		mark:	u8,
		/// The column the item's content starts at.
		col:	usize,
		/// The number an ordered marker gave, if it gave one.
		num:	Option<u64>,
		/// Whether the marker line carries no content of its own.
		bare:	bool,
		/// The item's first line, with the marker taken off.
		head:	String,
	},
	/// Code marked by its indentation.
	Code,
	/// Anything else, which is paragraph text.
	Text,
}

/// Reads a run of lines into the blocks they are, at the given nesting depth.
fn blocks(lines: &[&str], depth: usize) -> Outcome<Vec<Block>> {
	if depth > DEPTH_LIMIT {
		return Err(err!(
			"Markdown blocks nest more than {} deep, which no document written to be read \
			does.", DEPTH_LIMIT;
			Excessive, Input));
	}
	let mut out = Vec::new();
	let mut i = 0;
	while i < lines.len() {
		match classify(lines[i]) {
			Start::Blank		=> i += 1,
			Start::Rule		=> {
				out.push(Block::Rule);
				i += 1;
			}
			Start::Atx(level, text)	=> {
				out.push(Block::Heading {
					level,
					content:	res!(inline::parse(&text)),
				});
				i += 1;
			}
			Start::Fence { ch, len, ind, lang } => {
				let (b, n) = code_fenced(&lines[i..], ch, len, ind, lang);
				out.push(b);
				i += n;
			}
			Start::Code		=> {
				let (b, n) = code_indented(&lines[i..]);
				out.push(b);
				i += n;
			}
			Start::Quote		=> {
				let (b, n) = res!(quote(&lines[i..], depth));
				out.push(b);
				i += n;
			}
			Start::Item { .. }	=> {
				let (b, n) = res!(list(&lines[i..], depth));
				out.push(b);
				i += n;
			}
			Start::Text		=> {
				let (b, n) = res!(para(&lines[i..]));
				out.push(b);
				i += n;
			}
		}
	}
	Ok(out)
}

// ── Classification ───────────────────────────────────────────────

/// Judges what a line begins.
fn classify(line: &str) -> Start {
	if is_blank(line) {
		return Start::Blank;
	}
	let ind = indent_of(line);
	if ind >= CODE_COL {
		return Start::Code;
	}
	let off = ws_end(line);
	let rest = &line[off..];

	// A thematic break is judged first, so that `***` is a break and not a list of nothing.
	if is_rule(rest) {
		return Start::Rule;
	}

	// An ATX heading: one to six hashes, set apart from the text by a space.
	if rest.starts_with('#') {
		let n = rest.bytes().take_while(|c| *c == b'#').count();
		let after = &rest[n..];
		if n <= 6 && (after.is_empty() || after.starts_with(' ') || after.starts_with('\t')) {
			return Start::Atx(n as u8, atx_text(after));
		}
	}

	// A code fence: three or more backticks or tildes, and what they say about the code.
	let b = rest.as_bytes();
	if b[0] == b'`' || b[0] == b'~' {
		let ch = b[0];
		let len = rest.bytes().take_while(|c| *c == ch).count();
		let info = rest[len..].trim();
		// A backtick fence's info string may hold no backtick, or `a ` b` would open one.
		if len >= 3 && (ch != b'`' || !info.contains('`')) {
			let lang = match info.split_whitespace().next() {
				Some(w)	=> Some(w.to_string()),
				None	=> None,
			};
			return Start::Fence { ch, len, ind, lang };
		}
	}

	if b[0] == b'>' {
		return Start::Quote;
	}

	match item_start(line, off, ind) {
		Some(s)	=> s,
		None	=> Start::Text,
	}
}

/// Recognises a list item marker, and what it says about the item it begins.
fn item_start(line: &str, off: usize, ind: usize) -> Option<Start> {
	let b = line.as_bytes();
	let mut p = off;	// Byte offset.
	let mut c = ind;	// Column.
	let mut ord = false;
	let mut num = None;
	let mark;
	match b[p] {
		b'-' | b'*' | b'+'	=> {
			mark = b[p];
			p += 1;
			c += 1;
		}
		d if d.is_ascii_digit()	=> {
			let s = p;
			// Nine digits is as long a number as a list may count to.
			while p < b.len() && b[p].is_ascii_digit() && p - s < 9 {
				p += 1;
			}
			if p >= b.len() || (b[p] != b'.' && b[p] != b')') {
				return None;
			}
			num = line[s..p].parse::<u64>().ok();
			mark = b[p];
			ord = true;
			c += p - s + 1;
			p += 1;
		}
		_			=> return None,
	}
	let rest = &line[p..];
	// A marker with nothing after it is an empty item.
	if rest.is_empty() || is_blank(rest) {
		return Some(Start::Item {
			ord,
			mark,
			col:	c + 1,
			num,
			bare:	true,
			head:	String::new(),
		});
	}
	let sp = indent_of(rest);	// Columns of whitespace between marker and content.
	if sp == 0 {
		return None;	// `-foo` is a word beginning with a dash, not a list.
	}
	// Whitespace past the fourth column is code within the item, not part of the marker.
	let take = if sp > CODE_COL { 1 } else { sp };
	Some(Start::Item {
		ord,
		mark,
		col:	c + take,
		num,
		bare:	false,
		head:	strip_cols(rest, take),
	})
}

/// Whether the line, past its indent, is a thematic break.
fn is_rule(rest: &str) -> bool {
	let b = rest.as_bytes();
	if b.is_empty() {
		return false;
	}
	let ch = b[0];
	if ch != b'-' && ch != b'*' && ch != b'_' {
		return false;
	}
	let mut n = 0;
	for &c in b {
		if c == ch {
			n += 1;
		} else if c != b' ' && c != b'\t' {
			return false;
		}
	}
	n >= 3
}

/// The heading level a setext underline gives, if the line is one.
fn under_of(line: &str) -> Option<u8> {
	if indent_of(line) >= CODE_COL {
		return None;
	}
	let t = line.trim_matches(|c| c == ' ' || c == '\t');
	if t.is_empty() {
		return None;
	}
	let ch = t.as_bytes()[0];
	if (ch != b'=' && ch != b'-') || !t.bytes().all(|c| c == ch) {
		return None;
	}
	Some(if ch == b'=' { 1 } else { 2 })
}

/// A heading's text, with the closing run of hashes an author may have balanced it with taken off.
fn atx_text(after: &str) -> String {
	let t = after.trim_matches(|c| c == ' ' || c == '\t');
	let n = t.bytes().rev().take_while(|c| *c == b'#').count();
	if n == 0 {
		return t.to_string();
	}
	let head = &t[..t.len() - n];
	// The closing run counts only when a space sets it apart, or when it is all there is.
	if head.is_empty() || head.ends_with(' ') || head.ends_with('\t') {
		head.trim_end_matches(|c| c == ' ' || c == '\t').to_string()
	} else {
		t.to_string()
	}
}

/// Whether a line begins something that ends the paragraph above it.
fn interrupts(line: &str) -> bool {
	match classify(line) {
		Start::Blank
		| Start::Atx(..)
		| Start::Fence { .. }
		| Start::Rule
		| Start::Quote				=> true,
		// A list ends a paragraph only where it plainly begins one, so that a year opening a
		// sentence, or a full stop wrapping onto its own line, is prose and not a list.
		Start::Item { ord, num, bare, .. }	=> !bare && (!ord || num == Some(1)),
		Start::Code
		| Start::Text				=> false,
	}
}

// ── The blocks themselves ────────────────────────────────────────

/// Reads a paragraph, or the heading a setext underline turns it into.
fn para(lines: &[&str]) -> Outcome<(Block, usize)> {
	let mut acc = vec![lines[0].trim_start_matches(|c| c == ' ' || c == '\t')];
	let mut i = 1;
	while i < lines.len() {
		// An underline under a paragraph is a heading, which is how `---` means a heading here
		// and a thematic break anywhere else.
		if let Some(level) = under_of(lines[i]) {
			return Ok((Block::Heading {
				level,
				content:	res!(inline::parse(acc.join("\n").trim_end())),
			}, i + 1));
		}
		if interrupts(lines[i]) {
			break;
		}
		acc.push(lines[i].trim_start_matches(|c| c == ' ' || c == '\t'));
		i += 1;
	}
	Ok((Block::Para(res!(inline::parse(acc.join("\n").trim_end()))), i))
}

/// Reads a fenced code block, which runs to its closing fence or to the end of the input.
fn code_fenced(lines: &[&str], ch: u8, len: usize, ind: usize, lang: Option<String>)
	-> (Block, usize)
{
	let mut text = String::new();
	let mut i = 1;
	while i < lines.len() {
		if fence_closes(lines[i], ch, len) {
			return (Block::Code { lang, text }, i + 1);
		}
		text.push_str(&strip_cols(lines[i], ind));
		text.push('\n');
		i += 1;
	}
	(Block::Code { lang, text }, i)
}

/// Whether the line closes a fence of the given character and length.
fn fence_closes(line: &str, ch: u8, len: usize) -> bool {
	if indent_of(line) >= CODE_COL {
		return false;
	}
	let rest = &line[ws_end(line)..];
	let n = rest.bytes().take_while(|c| *c == ch).count();
	// A closing fence is at least as long as the one it closes, and says nothing else.
	n >= len && is_blank(&rest[n..])
}

/// Reads a run of code marked by its indentation.
fn code_indented(lines: &[&str]) -> (Block, usize) {
	let mut out: Vec<String> = Vec::new();
	let mut pend: Vec<String> = Vec::new();	// Blank lines held back, in case the code ends here.
	let mut i = 0;
	while i < lines.len() {
		if is_blank(lines[i]) {
			pend.push(strip_cols(lines[i], CODE_COL));
			i += 1;
			continue;
		}
		if indent_of(lines[i]) < CODE_COL {
			break;
		}
		out.append(&mut pend);
		out.push(strip_cols(lines[i], CODE_COL));
		i += 1;
	}
	let mut text = String::new();
	for l in &out {
		text.push_str(l);
		text.push('\n');
	}
	// The blank lines that trail the code belong to whatever follows it.
	(Block::Code { lang: None, text }, i - pend.len())
}

/// Reads a block quotation and the blocks within it.
fn quote(lines: &[&str], depth: usize) -> Outcome<(Block, usize)> {
	let mut body: Vec<String> = Vec::new();
	let mut i = 0;
	while i < lines.len() {
		if let Some(rest) = quote_strip(lines[i]) {
			body.push(rest);
			i += 1;
			continue;
		}
		if is_blank(lines[i]) || !lazy(&body, lines[i]) {
			break;
		}
		body.push(lines[i].trim_start_matches(|c| c == ' ' || c == '\t').to_string());
		i += 1;
	}
	let refs: Vec<&str> = body.iter().map(|s| s.as_str()).collect();
	Ok((Block::Quote(res!(blocks(&refs, depth + 1))), i))
}

/// The line with its quotation marker taken off, if it carries one.
fn quote_strip(line: &str) -> Option<String> {
	if indent_of(line) >= CODE_COL {
		return None;
	}
	match line[ws_end(line)..].strip_prefix('>') {
		// One space after the marker is the marker's own, and no more.
		Some(rest)	=> Some(strip_cols(rest, 1)),
		None		=> None,
	}
}

/// Reads a list: its first item, and every item that follows it at the same level with the same
/// marker.
fn list(lines: &[&str], depth: usize) -> Outcome<(Block, usize)> {
	let (ord, mark) = match classify(lines[0]) {
		Start::Item { ord, mark, .. }	=> (ord, mark),
		_				=> return Err(err!(
			"A list was read from a line that does not begin one."; Bug)),
	};
	let mut items = Vec::new();
	let mut i = 0;
	loop {
		// Blank lines may sit between items, but they are the list's only if an item follows: a
		// blank line before a paragraph ends the list and belongs to the document.
		let mut j = i;
		while j < lines.len() && is_blank(lines[j]) {
			j += 1;
		}
		if j >= lines.len() {
			break;
		}
		// A different marker begins a different list, which is how an author separates two.
		let (col, head) = match classify(lines[j]) {
			Start::Item { ord: o, mark: m, col, head, .. } if o == ord && m == mark
				=> (col, head),
			_	=> break,
		};
		let (body, n) = item_body(&lines[j..], head, col);
		let refs: Vec<&str> = body.iter().map(|s| s.as_str()).collect();
		items.push(res!(blocks(&refs, depth + 1)));
		i = j + n;
	}
	Ok((Block::List { ordered: ord, items }, i))
}

/// Reads one list item's lines, with the marker and the indentation that stands for it taken off.
fn item_body(lines: &[&str], head: String, col: usize) -> (Vec<String>, usize) {
	let mut body = vec![head];
	let mut i = 1;
	while i < lines.len() {
		if is_blank(lines[i]) {
			// A blank line is the item's only if indented content follows it.
			let mut k = i;
			while k < lines.len() && is_blank(lines[k]) {
				k += 1;
			}
			if k >= lines.len() || indent_of(lines[k]) < col {
				break;
			}
			for _ in i..k {
				body.push(String::new());
			}
			i = k;
			continue;
		}
		if indent_of(lines[i]) >= col {
			body.push(strip_cols(lines[i], col));
			i += 1;
			continue;
		}
		if !lazy(&body, lines[i]) {
			break;
		}
		body.push(lines[i].trim_start_matches(|c| c == ' ' || c == '\t').to_string());
		i += 1;
	}
	(body, i)
}

/// Whether an under-indented line carries on the paragraph a container was in the middle of.
///
/// This is Markdown's laziness: an author who wraps a quoted or listed paragraph need not mark every
/// line of it, so a line that is only text goes on belonging to the paragraph above.
fn lazy(body: &[String], line: &str) -> bool {
	match body.last() {
		Some(l) if !is_blank(l)	=> matches!(classify(line), Start::Text | Start::Code),
		_			=> false,
	}
}

// ── Whitespace ───────────────────────────────────────────────────

/// Whether the line holds nothing but whitespace.
fn is_blank(line: &str) -> bool {
	line.bytes().all(|b| b == b' ' || b == b'\t')
}

/// The column the line's first non-whitespace character sits at, a tab advancing to the next tab
/// stop.
fn indent_of(line: &str) -> usize {
	let mut c = 0;
	for b in line.bytes() {
		match b {
			b' '	=> c += 1,
			b'\t'	=> c += TAB - (c % TAB),
			_	=> break,
		}
	}
	c
}

/// The byte offset of the line's first non-whitespace character.
fn ws_end(line: &str) -> usize {
	let mut i = 0;
	for b in line.bytes() {
		if b != b' ' && b != b'\t' {
			break;
		}
		i += 1;
	}
	i
}

/// The line with `n` columns of leading whitespace taken off.
///
/// A tab that straddles the cut gives up what it spans past it as spaces, which is the only way to
/// take a fixed number of columns from a line a tab has indented.
fn strip_cols(line: &str, n: usize) -> String {
	let mut c = 0;	// Column.
	let mut i = 0;	// Byte offset.
	for b in line.bytes() {
		if c >= n {
			break;
		}
		match b {
			b' '	=> {
				c += 1;
				i += 1;
			}
			b'\t'	=> {
				let next = c + TAB - (c % TAB);
				if next > n {
					let mut s = " ".repeat(next - n);
					s.push_str(&line[i + 1..]);
					return s;
				}
				c = next;
				i += 1;
			}
			_	=> break,
		}
	}
	line[i..].to_string()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::markdown::Inline;

	use oxedyne_fe2o3_core::prelude::*;

	/// The text of a block's inlines, for tests that care what a block says and not how.
	fn said(blocks: &[Block]) -> Vec<String> {
		blocks.iter().map(|b| match b {
			Block::Para(c)			=> crate::markdown::text_of(c),
			Block::Heading { content, .. }	=> crate::markdown::text_of(content),
			Block::Code { text, .. }	=> text.clone(),
			_				=> String::new(),
		}).collect()
	}

	/// A hash and a space open a heading, and the hashes count its level.
	#[test]
	fn test_a_run_of_hashes_opens_a_heading_00() -> Outcome<()> {
		let b = res!(parse("# One\n\n### Three\n"));
		assert_eq!(b.len(), 2);
		assert_eq!(b[0], Block::Heading { level: 1, content: vec![Inline::Text("One".into())] });
		assert_eq!(b[1], Block::Heading { level: 3, content: vec![Inline::Text("Three".into())] });
		Ok(())
	}

	/// Seven hashes is not a heading, because there is no seventh level to give it.
	#[test]
	fn test_more_hashes_than_levels_is_not_a_heading_01() -> Outcome<()> {
		let b = res!(parse("####### Seven\n"));
		assert_eq!(b, vec![Block::Para(vec![Inline::Text("####### Seven".into())])]);
		Ok(())
	}

	/// A run of hashes closing a heading is decoration, and is not part of what it says.
	#[test]
	fn test_a_closing_run_of_hashes_is_stripped_02() -> Outcome<()> {
		assert_eq!(said(&res!(parse("## Two ##\n"))), vec!["Two"]);
		assert_eq!(said(&res!(parse("## Two #########\n"))), vec!["Two"]);
		// Without a space, the hash is part of the word and stays.
		assert_eq!(said(&res!(parse("## Two#\n"))), vec!["Two#"]);
		// A heading of nothing but its closing run says nothing.
		assert_eq!(said(&res!(parse("# #\n"))), vec![""]);
		assert_eq!(said(&res!(parse("#\n"))), vec![""]);
		Ok(())
	}

	/// A hash with no space after it is a word, not a heading.
	#[test]
	fn test_a_hash_without_a_space_is_text_03() -> Outcome<()> {
		let b = res!(parse("#hashtag\n"));
		assert_eq!(b, vec![Block::Para(vec![Inline::Text("#hashtag".into())])]);
		Ok(())
	}

	/// Blank lines divide paragraphs, and the lines between them are one paragraph.
	#[test]
	fn test_blank_lines_divide_paragraphs_04() -> Outcome<()> {
		let b = res!(parse("One line\nand its second.\n\nA second paragraph.\n"));
		assert_eq!(b.len(), 2);
		assert_eq!(said(&b), vec!["One line and its second.", "A second paragraph."]);
		Ok(())
	}

	/// A fence holds code exactly as written, and its info string names the language.
	#[test]
	fn test_a_fence_holds_code_and_names_its_language_05() -> Outcome<()> {
		let b = res!(parse("```rust\nlet x = *y;\n```\n"));
		assert_eq!(b, vec![Block::Code {
			lang:	Some("rust".into()),
			text:	"let x = *y;\n".into(),
		}]);
		// Tildes fence as well as backticks, and a fence may name nothing.
		let b = res!(parse("~~~\nplain\n~~~\n"));
		assert_eq!(b, vec![Block::Code { lang: None, text: "plain\n".into() }]);
		Ok(())
	}

	/// A fence nobody closed runs to the end of the input rather than failing.
	#[test]
	fn test_an_unclosed_fence_runs_to_the_end_06() -> Outcome<()> {
		let b = res!(parse("```\nstill code\nand more\n"));
		assert_eq!(b, vec![Block::Code { lang: None, text: "still code\nand more\n".into() }]);
		Ok(())
	}

	/// A fence swallows what would otherwise be markup, which is what a fence is for.
	#[test]
	fn test_a_fence_swallows_markup_07() -> Outcome<()> {
		let b = res!(parse("```\n# not a heading\n- not a list\n```\n\nAfter.\n"));
		assert_eq!(b.len(), 2);
		assert_eq!(b[0], Block::Code {
			lang:	None,
			text:	"# not a heading\n- not a list\n".into(),
		});
		Ok(())
	}

	/// Four spaces, or one tab, makes code of a line.
	#[test]
	fn test_indentation_makes_code_08() -> Outcome<()> {
		let b = res!(parse("    let x = 1;\n    let y = 2;\n"));
		assert_eq!(b, vec![Block::Code { lang: None, text: "let x = 1;\nlet y = 2;\n".into() }]);
		let b = res!(parse("\tby a tab\n"));
		assert_eq!(b, vec![Block::Code { lang: None, text: "by a tab\n".into() }]);
		Ok(())
	}

	/// Blank lines within indented code are kept, and those trailing it are not.
	#[test]
	fn test_indented_code_keeps_its_inner_blank_lines_09() -> Outcome<()> {
		let b = res!(parse("    one\n\n    two\n\nProse.\n"));
		assert_eq!(b.len(), 2);
		assert_eq!(b[0], Block::Code { lang: None, text: "one\n\ntwo\n".into() });
		assert_eq!(said(&b[1..]), vec!["Prose."]);
		Ok(())
	}

	/// Indentation cannot make code of a line that carries a paragraph on.
	#[test]
	fn test_indentation_does_not_interrupt_a_paragraph_10() -> Outcome<()> {
		let b = res!(parse("A paragraph\n    and its wrapped line.\n"));
		assert_eq!(b.len(), 1);
		assert_eq!(said(&b), vec!["A paragraph and its wrapped line."]);
		Ok(())
	}

	/// A quotation holds blocks, and reads them as a document of its own.
	#[test]
	fn test_a_quotation_holds_blocks_11() -> Outcome<()> {
		let b = res!(parse("> # Heading\n>\n> A paragraph.\n"));
		assert_eq!(b.len(), 1);
		match &b[0] {
			Block::Quote(inner)	=> {
				assert_eq!(inner.len(), 2);
				assert_eq!(said(inner), vec!["Heading", "A paragraph."]);
			}
			other			=> panic!("expected a quotation, got {:?}", other),
		}
		Ok(())
	}

	/// Quotations nest, and each `>` is a level.
	#[test]
	fn test_quotations_nest_12() -> Outcome<()> {
		let b = res!(parse("> outer\n>> inner\n"));
		match &b[0] {
			Block::Quote(a)	=> match &a[1] {
				Block::Quote(c)	=> assert_eq!(said(c), vec!["inner"]),
				other		=> panic!("expected a nested quotation, got {:?}", other),
			},
			other		=> panic!("expected a quotation, got {:?}", other),
		}
		Ok(())
	}

	/// An author who wraps a quoted paragraph need not mark every line of it.
	#[test]
	fn test_a_quotation_carries_on_lazily_13() -> Outcome<()> {
		let b = res!(parse("> one\ntwo\n\nOut.\n"));
		assert_eq!(b.len(), 2);
		match &b[0] {
			Block::Quote(inner)	=> assert_eq!(said(inner), vec!["one two"]),
			other			=> panic!("expected a quotation, got {:?}", other),
		}
		Ok(())
	}

	/// Every bullet makes an unordered list.
	#[test]
	fn test_bullets_make_an_unordered_list_14() -> Outcome<()> {
		for src in ["- a\n- b\n", "* a\n* b\n", "+ a\n+ b\n"] {
			let b = res!(parse(src));
			match &b[0] {
				Block::List { ordered, items }	=> {
					assert!(!ordered);
					assert_eq!(items.len(), 2, "for {:?}", src);
					assert_eq!(said(&items[0]), vec!["a"]);
					assert_eq!(said(&items[1]), vec!["b"]);
				}
				other				=> panic!("expected a list, got {:?}", other),
			}
		}
		Ok(())
	}

	/// A number and a delimiter make an ordered list, whichever delimiter it is.
	#[test]
	fn test_numbers_make_an_ordered_list_15() -> Outcome<()> {
		for src in ["1. a\n2. b\n", "1) a\n2) b\n"] {
			let b = res!(parse(src));
			match &b[0] {
				Block::List { ordered, items }	=> {
					assert!(ordered, "for {:?}", src);
					assert_eq!(items.len(), 2);
				}
				other				=> panic!("expected a list, got {:?}", other),
			}
		}
		Ok(())
	}

	/// A change of marker begins a new list, which is how an author sets two lists apart.
	#[test]
	fn test_a_change_of_marker_begins_a_new_list_16() -> Outcome<()> {
		let b = res!(parse("- a\n- b\n\n* c\n"));
		assert_eq!(b.len(), 2);
		match (&b[0], &b[1]) {
			(Block::List { items: x, .. }, Block::List { items: y, .. })	=> {
				assert_eq!(x.len(), 2);
				assert_eq!(y.len(), 1);
			}
			other								=> panic!("expected two lists, got {:?}", other),
		}
		Ok(())
	}

	/// Indentation nests one list inside another, within the item it is indented under.
	#[test]
	fn test_a_list_nests_within_a_list_17() -> Outcome<()> {
		let b = res!(parse("- a\n  - inner\n- b\n"));
		match &b[0] {
			Block::List { items, .. }	=> {
				assert_eq!(items.len(), 2);
				assert_eq!(items[0].len(), 2);	// The paragraph, then the nested list.
				assert_eq!(said(&items[0][..1]), vec!["a"]);
				match &items[0][1] {
					Block::List { items: inner, .. }	=> {
						assert_eq!(said(&inner[0]), vec!["inner"]);
					}
					other					=> panic!("expected a nested list, got {:?}", other),
				}
			}
			other				=> panic!("expected a list, got {:?}", other),
		}
		Ok(())
	}

	/// A quotation nests within a list item, which is nesting of one kind inside another.
	#[test]
	fn test_a_quotation_nests_within_a_list_item_18() -> Outcome<()> {
		let b = res!(parse("- an item\n\n  > quoted\n"));
		match &b[0] {
			Block::List { items, .. }	=> match &items[0][1] {
				Block::Quote(inner)	=> assert_eq!(said(inner), vec!["quoted"]),
				other			=> panic!("expected a quotation, got {:?}", other),
			},
			other				=> panic!("expected a list, got {:?}", other),
		}
		Ok(())
	}

	/// A blank line between items leaves them items of the one list.
	#[test]
	fn test_a_blank_line_between_items_keeps_one_list_19() -> Outcome<()> {
		let b = res!(parse("- a\n\n- b\n"));
		assert_eq!(b.len(), 1);
		match &b[0] {
			Block::List { items, .. }	=> assert_eq!(items.len(), 2),
			other				=> panic!("expected a list, got {:?}", other),
		}
		Ok(())
	}

	/// A list item holds every block written under it, not merely a line.
	#[test]
	fn test_a_list_item_holds_blocks_20() -> Outcome<()> {
		let b = res!(parse("- first\n\n  second\n\n- next\n"));
		match &b[0] {
			Block::List { items, .. }	=> {
				assert_eq!(items.len(), 2);
				assert_eq!(said(&items[0]), vec!["first", "second"]);
			}
			other				=> panic!("expected a list, got {:?}", other),
		}
		Ok(())
	}

	/// A list ends where a paragraph that is nobody's item begins.
	#[test]
	fn test_a_list_ends_at_an_unindented_paragraph_21() -> Outcome<()> {
		let b = res!(parse("- a\n- b\n\nAfter the list.\n"));
		assert_eq!(b.len(), 2);
		assert_eq!(said(&b[1..]), vec!["After the list."]);
		Ok(())
	}

	/// Three or more of a break's characters make a thematic break.
	#[test]
	fn test_three_characters_make_a_thematic_break_22() -> Outcome<()> {
		for src in ["---\n", "***\n", "___\n", "- - -\n", "*****\n"] {
			assert_eq!(res!(parse(src)), vec![Block::Rule], "for {:?}", src);
		}
		// Two is not enough.
		assert_eq!(said(&res!(parse("--\n"))), vec!["--"]);
		Ok(())
	}

	/// An underline under a paragraph is a heading, and the same characters alone are a break.
	#[test]
	fn test_an_underline_beats_a_thematic_break_23() -> Outcome<()> {
		let b = res!(parse("A title\n---\n"));
		assert_eq!(b, vec![Block::Heading { level: 2, content: vec![Inline::Text("A title".into())] }]);
		// With nothing above it to underline, the same line is a break.
		let b = res!(parse("\n---\n"));
		assert_eq!(b, vec![Block::Rule]);
		// And a break after a blank line is a break, not a heading.
		let b = res!(parse("A title\n\n---\n"));
		assert_eq!(b, vec![
			Block::Para(vec![Inline::Text("A title".into())]),
			Block::Rule,
		]);
		Ok(())
	}

	/// Equals signs underline a first-level heading, which has no thematic break to argue with.
	#[test]
	fn test_equals_signs_underline_a_first_level_heading_24() -> Outcome<()> {
		let b = res!(parse("A title\n===\n"));
		assert_eq!(b, vec![Block::Heading { level: 1, content: vec![Inline::Text("A title".into())] }]);
		// Alone, they are only what they are.
		assert_eq!(said(&res!(parse("===\n"))), vec!["==="]);
		Ok(())
	}

	/// A heading, a fence, a break or a quotation ends the paragraph above it without a blank line.
	#[test]
	fn test_a_block_may_interrupt_a_paragraph_25() -> Outcome<()> {
		let b = res!(parse("A paragraph\n# A heading\n"));
		assert_eq!(b.len(), 2);
		let b = res!(parse("A paragraph\n> quoted\n"));
		assert_eq!(b.len(), 2);
		let b = res!(parse("A paragraph\n***\n"));
		assert_eq!(b.len(), 2);
		let b = res!(parse("A paragraph\n```\ncode\n```\n"));
		assert_eq!(b.len(), 2);
		Ok(())
	}

	/// A number that opens a sentence is prose, because a list that interrupts a paragraph counts
	/// from one.
	#[test]
	fn test_a_year_does_not_begin_a_list_26() -> Outcome<()> {
		let b = res!(parse("The year was\n2024. A good one.\n"));
		assert_eq!(b.len(), 1);
		assert_eq!(said(&b), vec!["The year was 2024. A good one."]);
		// But a list that counts from one does begin.
		let b = res!(parse("A paragraph\n1. an item\n"));
		assert_eq!(b.len(), 2);
		Ok(())
	}

	/// A dash with no space after it is a word, not an item.
	#[test]
	fn test_a_dash_without_a_space_is_text_27() -> Outcome<()> {
		let b = res!(parse("-not-a-list\n"));
		assert_eq!(b, vec![Block::Para(vec![Inline::Text("-not-a-list".into())])]);
		Ok(())
	}

	/// Nesting past the limit is refused, which is the parser's one refusal.
	#[test]
	fn test_nesting_past_the_limit_is_refused_28() -> Outcome<()> {
		// A quotation for every level the limit allows is read.
		let ok = format!("{} deep\n", ">".repeat(DEPTH_LIMIT));
		assert!(parse(&ok).is_ok());
		// One past it, and past by far, is not.
		let deep = format!("{} deep\n", ">".repeat(DEPTH_LIMIT + 8));
		assert!(parse(&deep).is_err());
		let very = format!("{} deep\n", ">".repeat(2000));
		assert!(parse(&very).is_err());
		Ok(())
	}

	/// An empty document is a document with nothing in it, and not a failure.
	#[test]
	fn test_an_empty_document_holds_nothing_29() -> Outcome<()> {
		assert_eq!(res!(parse("")), Vec::<Block>::new());
		assert_eq!(res!(parse("\n\n \n")), Vec::<Block>::new());
		Ok(())
	}

	/// Windows line endings are line endings.
	#[test]
	fn test_carriage_returns_are_not_text_30() -> Outcome<()> {
		let b = res!(parse("# A heading\r\n\r\nA paragraph.\r\n"));
		assert_eq!(said(&b), vec!["A heading", "A paragraph."]);
		Ok(())
	}

	/// A tab indents a list item's content as spaces would.
	#[test]
	fn test_a_tab_indents_an_item_31() -> Outcome<()> {
		let b = res!(parse("-\tan item\n"));
		match &b[0] {
			Block::List { items, .. }	=> assert_eq!(said(&items[0]), vec!["an item"]),
			other				=> panic!("expected a list, got {:?}", other),
		}
		Ok(())
	}

	/// A fence's indentation is taken off the code it holds, and no more.
	#[test]
	fn test_a_fence_strips_its_own_indent_32() -> Outcome<()> {
		let b = res!(parse("  ```\n  code\n    deeper\n  ```\n"));
		assert_eq!(b, vec![Block::Code { lang: None, text: "code\n  deeper\n".into() }]);
		Ok(())
	}

	/// A paragraph the author hard wrapped is one run of prose, and reflows to whatever reads it.
	///
	/// Prose arrives wrapped to the width its author wrote at. That width means nothing, so it is
	/// not kept: a paragraph is what it says, and where its lines fall is the reader's to decide.
	#[test]
	fn test_a_hard_wrapped_paragraph_reflows_34() -> Outcome<()> {
		let b = res!(parse("A paragraph that the author\nhard wrapped at a narrow width\nacross three lines.\n"));
		assert_eq!(b, vec![Block::Para(vec![
			Inline::Text("A paragraph that the author hard wrapped at a narrow width across three lines.".into()),
		])]);
		Ok(())
	}

	/// A break the author did ask for survives the paragraph it is in.
	#[test]
	fn test_a_hard_break_survives_a_paragraph_35() -> Outcome<()> {
		let b = res!(parse("one  \ntwo\n"));
		assert_eq!(b, vec![Block::Para(vec![
			Inline::Text("one".into()),
			Inline::Break,
			Inline::Text("two".into()),
		])]);
		// A backslash asks as plainly as two spaces do.
		let b = res!(parse("one\\\ntwo\n"));
		assert_eq!(b, vec![Block::Para(vec![
			Inline::Text("one".into()),
			Inline::Break,
			Inline::Text("two".into()),
		])]);
		Ok(())
	}

	/// A whole document of every block reads as the document it is.
	#[test]
	fn test_a_document_of_every_block_33() -> Outcome<()> {
		let src = "\
# Title

An opening paragraph.

## A section

- one
- two
  - nested

> A quotation.

```sh
echo hi
```

---

The end.
";
		let b = res!(parse(src));
		assert!(matches!(b[0], Block::Heading { level: 1, .. }));
		assert!(matches!(b[1], Block::Para(_)));
		assert!(matches!(b[2], Block::Heading { level: 2, .. }));
		assert!(matches!(b[3], Block::List { ordered: false, .. }));
		assert!(matches!(b[4], Block::Quote(_)));
		assert!(matches!(b[5], Block::Code { .. }));
		assert!(matches!(b[6], Block::Rule));
		assert!(matches!(b[7], Block::Para(_)));
		assert_eq!(b.len(), 8);
		Ok(())
	}
}
