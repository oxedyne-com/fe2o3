//! Block structure: the pass that divides Djot text into headings, paragraphs, lists, code,
//! quotations, divisions, tables and breaks.
//!
//! Djot is read in two passes, as Markdown is, because its two levels are decided by different things.
//! A block is decided by how a line *starts* and by the blank lines around it; an inline run is
//! decided by delimiters within a line. This module does the first pass and hands each block's text to
//! [`crate::doc::djot::inline`] for the second.
//!
//! Before either pass, a sweep gathers the document's reference definitions -- the `[ref]: url` lines
//! by which a `[text][ref]` finds its destination -- so that a reference may be defined anywhere and
//! still resolve a link that stands above it.

use crate::doc::{
	Align,
	Attrs,
	Block,
	Cell,
	Row,
	djot::inline,
};

use std::collections::HashMap;

use oxedyne_fe2o3_core::prelude::*;

/// How deep a document may nest its blocks before the parser refuses it.
///
/// A division inside a list inside a quotation is legitimate; a thousand of them is a document built
/// to exhaust the stack of whatever reads it. The limit is generous beside anything an author writes
/// and far below what would trouble the machine.
pub const DEPTH_LIMIT: usize = 32;

/// The columns a tab advances by, counting from a tab stop.
const TAB: usize = 4;

/// The column past which whitespace after a list marker is more than the marker's own.
const WIDE: usize = 4;

/// Divides Djot text into its blocks.
pub fn parse(src: &str) -> Outcome<Vec<Block>> {
	let lines: Vec<&str> = src.lines().collect();
	let refs = collect_refs(&lines);
	blocks(&lines, 0, &refs)
}

/// What a line begins, judged by how it starts.
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
	/// A division fence: a run of three or more colons.
	Div {
		/// How many colons the fence runs to, which its closing fence must match or exceed.
		len:	usize,
		/// The attributes the opening fence named, empty where it named none.
		attrs:	Attrs,
		/// Whether the fence opens a division rather than closing one.
		open:	bool,
	},
	/// A thematic break.
	Rule,
	/// A block quotation marker.
	Quote,
	/// A standalone attributes line, which attaches to the block that follows it.
	Attr(Attrs),
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
	/// Anything else, which is paragraph text.
	Text,
}

/// Reads a run of lines into the blocks they are, at the given nesting depth.
fn blocks(lines: &[&str], depth: usize, refs: &HashMap<String, String>) -> Outcome<Vec<Block>> {
	if depth > DEPTH_LIMIT {
		return Err(err!(
			"Djot blocks nest more than {} deep, which no document written to be read \
			does.", DEPTH_LIMIT;
			Excessive, Input));
	}
	let mut out = Vec::new();
	let mut pending: Option<Attrs> = None;	// A standalone attributes line, awaiting its block.
	let mut i = 0;
	while i < lines.len() {
		// A reference definition was gathered in the first pass and is not itself a block.
		if ref_def_of(lines[i]).is_some() {
			i += 1;
			continue;
		}
		match classify(lines[i]) {
			Start::Blank		=> i += 1,
			Start::Rule		=> {
				emit(&mut out, &mut pending, Block::Rule);
				i += 1;
			}
			Start::Atx(level, text)	=> {
				let (content, n) = res!(heading(&lines[i..], text, refs));
				emit(&mut out, &mut pending, Block::Heading { level, content });
				i += n;
			}
			Start::Fence { ch, len, ind, lang } => {
				let (b, n) = code_fenced(&lines[i..], ch, len, ind, lang);
				emit(&mut out, &mut pending, b);
				i += n;
			}
			Start::Div { len, attrs, .. }	=> {
				let (b, n) = res!(div(&lines[i..], depth, len, attrs, refs));
				emit(&mut out, &mut pending, b);
				i += n;
			}
			Start::Quote		=> {
				let (b, n) = res!(quote(&lines[i..], depth, refs));
				emit(&mut out, &mut pending, b);
				i += n;
			}
			Start::Attr(a)		=> {
				// A second attributes line before the block merges into the first.
				pending = Some(match pending.take() {
					Some(p)	=> merge_attrs(p, a),
					None	=> a,
				});
				i += 1;
			}
			Start::Item { .. }	=> {
				let (b, n) = res!(list(&lines[i..], depth, refs));
				emit(&mut out, &mut pending, b);
				i += n;
			}
			Start::Text		=> {
				let (b, n) = res!(para(&lines[i..], refs));
				emit(&mut out, &mut pending, b);
				i += n;
			}
		}
	}
	Ok(out)
}

/// Adds a block, wrapping it in a division where a standalone attributes line stood above it.
///
/// The attributes have nowhere else to go: this tree carries block attributes only on a division, so a
/// `{.note}` above a paragraph makes a division of the paragraph. Above a division of its own, the
/// attributes merge into it rather than wrap it in a second, so `{#a}` over `::: note` names the one
/// box.
fn emit(out: &mut Vec<Block>, pending: &mut Option<Attrs>, b: Block) {
	match pending.take() {
		None		=> out.push(b),
		Some(attrs)	=> match b {
			Block::Div { attrs: inner, content }	=> {
				out.push(Block::Div { attrs: merge_attrs(attrs, inner), content });
			}
			other					=> {
				out.push(Block::Div { attrs, content: vec![other] });
			}
		},
	}
}

/// Combines an outer set of attributes, written first, with an inner set the block carried of its own.
///
/// The inner set is written second, so where the two both name an id the inner wins, as the last id
/// written wins within a single brace group. Classes and pairs of both are kept, the outer's first.
fn merge_attrs(outer: Attrs, inner: Attrs) -> Attrs {
	let mut classes = outer.classes;
	classes.extend(inner.classes);
	let mut pairs = outer.pairs;
	pairs.extend(inner.pairs);
	Attrs {
		id:	inner.id.or(outer.id),
		classes,
		pairs,
	}
}

// ── Classification ───────────────────────────────────────────────

/// Judges what a line begins.
fn classify(line: &str) -> Start {
	if is_blank(line) {
		return Start::Blank;
	}
	let ind = indent_of(line);
	let off = ws_end(line);
	let rest = &line[off..];
	let b = rest.as_bytes();

	// A thematic break is judged first, so that `***` is a break and not a list of nothing.
	if is_rule(rest) {
		return Start::Rule;
	}

	// An ATX heading: one to six hashes, set apart from the text by a space.
	if b[0] == b'#' {
		let n = rest.bytes().take_while(|c| *c == b'#').count();
		let after = &rest[n..];
		if n <= 6 && (after.is_empty() || after.starts_with(' ') || after.starts_with('\t')) {
			return Start::Atx(n as u8, atx_text(after));
		}
	}

	// A division fence: three or more colons, and the class or attributes an opener names.
	if b[0] == b':' {
		let n = rest.bytes().take_while(|c| *c == b':').count();
		if n >= 3 {
			let info = rest[n..].trim();
			let open = !info.is_empty();
			let attrs = div_attrs(info);
			return Start::Div { len: n, attrs, open };
		}
	}

	// A code fence: three or more backticks or tildes, and what they say about the code.
	if b[0] == b'`' || b[0] == b'~' {
		let ch = b[0];
		let len = rest.bytes().take_while(|c| *c == ch).count();
		let info = rest[len..].trim();
		// A backtick fence's info string may hold no backtick, or `a ` b` would open one.
		if len >= 3 && (ch != b'`' || !info.contains('`')) {
			let lang = info.split_whitespace().next().map(|w| w.to_string());
			return Start::Fence { ch, len, ind, lang };
		}
	}

	if b[0] == b'>' {
		return Start::Quote;
	}

	// A line that is nothing but a brace group attaches its attributes to the next block.
	if b[0] == b'{' {
		let t = rest.trim_end();
		if let Some((a, used)) = inline::attrs_of(t) {
			if used == t.len() {
				return Start::Attr(a);
			}
		}
	}

	match item_start(line, off, ind) {
		Some(s)	=> s,
		None	=> Start::Text,
	}
}

/// The attributes a division's opening fence named: a bare class name, or a full brace group.
fn div_attrs(info: &str) -> Attrs {
	if info.is_empty() {
		Attrs::default()
	} else if info.starts_with('{') {
		match inline::attrs_of(info) {
			Some((a, _))	=> a,
			None		=> Attrs::default(),
		}
	} else {
		// A single word after the colons is a class, its leading dot optional.
		let mut a = Attrs::default();
		if let Some(tok) = info.split_whitespace().next() {
			a.classes.push(tok.trim_start_matches('.').to_string());
		}
		a
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
	// Whitespace far past the marker is content indented within the item, not part of the marker.
	let take = if sp > WIDE { 1 } else { sp };
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

/// The alignments a delimiter row gives, if the line is one and divides into as many cells as the
/// header above it did.
fn delim_of(line: &str, n: usize) -> Option<Vec<Align>> {
	let cells = cells_of(line);
	if cells.is_empty() || cells.len() != n {
		return None;
	}
	let mut cols = Vec::with_capacity(cells.len());
	for cell in &cells {
		match align_of(cell) {
			Some(align)	=> cols.push(align),
			None		=> return None,
		}
	}
	Some(cols)
}

/// The alignment one delimiter cell gives: a colon at the side the column is aligned to, and dashes
/// between.
fn align_of(cell: &str) -> Option<Align> {
	let b = cell.as_bytes();
	if b.is_empty() {
		return None;
	}
	let start = b[0] == b':';
	let end = b[b.len() - 1] == b':';
	let s = if start { 1 } else { 0 };	// Where the dashes begin.
	let e = b.len() - if end { 1 } else { 0 };	// Where they end.
	if s >= e || !b[s..e].iter().all(|c| *c == b'-') {
		return None;
	}
	Some(match (start, end) {
		(true,	true)	=> Align::Centre,
		(true,	false)	=> Align::Start,
		(false,	true)	=> Align::End,
		(false,	false)	=> Align::None,
	})
}

/// The cells a table row divides into, split on every pipe the author did not escape.
///
/// A pipe at either end of the row is the row's own edge and not an empty cell, so an author may draw
/// the edges or leave them off. A `\|` divides nothing: it stays in the cell for [`inline::parse`] to
/// resolve as the escape it is.
fn cells_of(line: &str) -> Vec<&str> {
	let t = trim(line);
	let b = t.as_bytes();
	let mut out = Vec::new();
	let mut i = if b.first() == Some(&b'|') { 1 } else { 0 };
	let mut s = i;	// Where the cell being read begins.
	while i < b.len() {
		match b[i] {
			b'\\'	=> i += 2,	// A backslash and whatever it escapes divide nothing.
			b'|'	=> {
				out.push(trim(&t[s..i]));
				i += 1;
				s = i;
			}
			_	=> i += 1,
		}
	}
	if s < t.len() {
		out.push(trim(&t[s..]));
	}
	out
}

/// A heading's text, with the whitespace around it taken off.
///
/// Djot heads a section with hashes at the front alone, and keeps no closing run of them, so a `#` at
/// the end of the line is part of what the heading says.
fn atx_text(after: &str) -> String {
	after.trim_matches(|c| c == ' ' || c == '\t').to_string()
}

/// Whether a line begins something that ends the paragraph above it.
fn interrupts(line: &str) -> bool {
	match classify(line) {
		Start::Blank
		| Start::Atx(..)
		| Start::Fence { .. }
		| Start::Div { .. }
		| Start::Rule
		| Start::Attr(_)
		| Start::Quote				=> true,
		// A list ends a paragraph only where it plainly begins one, so that a year opening a
		// sentence, or a full stop wrapping onto its own line, is prose and not a list.
		Start::Item { ord, num, bare, .. }	=> !bare && (!ord || num == Some(1)),
		Start::Text				=> false,
	}
}

/// The reference label and destination a `[ref]: url` line defines, if it is one.
fn ref_def_of(line: &str) -> Option<(String, String)> {
	let rest = &line[ws_end(line)..];
	let b = rest.as_bytes();
	if b.is_empty() || b[0] != b'[' {
		return None;
	}
	let mut j = 1;
	while j < b.len() {
		match b[j] {
			b'\\'	=> j += 2,
			b']'	=> break,
			_	=> j += 1,
		}
	}
	if j >= b.len() || b[j] != b']' {
		return None;
	}
	let label = &rest[1..j];
	if label.is_empty() {
		return None;
	}
	let k = j + 1;
	if k >= b.len() || b[k] != b':' {
		return None;
	}
	Some((label.to_string(), rest[k + 1..].trim().to_string()))
}

/// Gathers the document's reference definitions, folding each label the way a reference to it will be.
///
/// The sweep steps over fenced code, so that a line that looks like a definition within a fence is the
/// code it is and defines nothing. The first definition of a label wins, as Djot has it.
fn collect_refs(lines: &[&str]) -> HashMap<String, String> {
	let mut map = HashMap::new();
	let mut i = 0;
	while i < lines.len() {
		match classify(lines[i]) {
			Start::Fence { ch, len, .. }	=> {
				i += 1;
				while i < lines.len() && !fence_closes(lines[i], ch, len) {
					i += 1;
				}
				if i < lines.len() {
					i += 1;
				}
			}
			_				=> {
				if let Some((label, url)) = ref_def_of(lines[i]) {
					map.entry(inline::normalise(&label)).or_insert(url);
				}
				i += 1;
			}
		}
	}
	map
}

// ── The blocks themselves ────────────────────────────────────────

/// Reads a heading and the lazy lines that carry its text on.
///
/// A heading's text runs across the non-blank lines that follow it, to a blank line or to whatever
/// begins a block of its own, so an author may wrap a long heading. The lines are joined and read as
/// one run, where the soft breaks between them say spaces.
fn heading(lines: &[&str], text: String, refs: &HashMap<String, String>)
	-> Outcome<(Vec<crate::doc::Inline>, usize)>
{
	let mut acc = vec![text];
	let mut i = 1;
	while i < lines.len() {
		if is_blank(lines[i]) || ref_def_of(lines[i]).is_some() {
			break;
		}
		match classify(lines[i]) {
			Start::Text	=> {
				acc.push(strip_leading(lines[i]).to_string());
				i += 1;
			}
			_		=> break,
		}
	}
	let content = res!(inline::parse(&acc.join("\n"), refs));
	Ok((content, i))
}

/// Reads a paragraph, or the table a delimiter row turns its last line into.
fn para(lines: &[&str], refs: &HashMap<String, String>) -> Outcome<(Block, usize)> {
	let mut acc = vec![strip_leading(lines[0]).to_string()];
	let mut i = 1;
	while i < lines.len() {
		// A table's header row is a paragraph line until the delimiter row beneath it says otherwise.
		if let Some(cols) = delim_of(lines[i], cells_of(&acc[i - 1]).len()) {
			// The lines above the header are a paragraph of their own, and they end here. The header
			// goes back to the document to be read again, as the table's first line.
			if i > 1 {
				acc.pop();
				return Ok((
					Block::Para(res!(inline::parse(acc.join("\n").trim_end(), refs))),
					i - 1,
				));
			}
			return table(lines, cols, refs);
		}
		if ref_def_of(lines[i]).is_some() || interrupts(lines[i]) {
			break;
		}
		acc.push(strip_leading(lines[i]).to_string());
		i += 1;
	}
	Ok((Block::Para(res!(inline::parse(acc.join("\n").trim_end(), refs))), i))
}

/// Reads a table: the header row, the delimiter row that made it one, and the body beneath them.
fn table(lines: &[&str], cols: Vec<Align>, refs: &HashMap<String, String>)
	-> Outcome<(Block, usize)>
{
	let head = res!(row(lines[0], cols.len(), refs));
	let mut rows = Vec::new();
	let mut i = 2;
	// The table runs to the first line that begins a block of its own, a blank line among them.
	while i < lines.len() && !interrupts(lines[i]) {
		rows.push(res!(row(lines[i], cols.len(), refs)));
		i += 1;
	}
	Ok((Block::Table { head: Some(head), rows, cols }, i))
}

/// Reads one row of a table, held to the width the header set.
fn row(line: &str, n: usize, refs: &HashMap<String, String>) -> Outcome<Row> {
	let mut cells = Vec::with_capacity(n);
	for text in cells_of(line).iter().take(n) {
		cells.push(Cell(res!(inline::parse(text, refs))));
	}
	// A row of fewer cells than the header named columns is short of the grid rather than wrong, and
	// a row of more has said something the header made no column for, and what it has no column for is
	// dropped.
	while cells.len() < n {
		cells.push(Cell(Vec::new()));
	}
	Ok(Row(cells))
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

/// Whether the line closes a code fence of the given character and length.
fn fence_closes(line: &str, ch: u8, len: usize) -> bool {
	let rest = &line[ws_end(line)..];
	let n = rest.bytes().take_while(|c| *c == ch).count();
	// A closing fence is at least as long as the one it closes, and says nothing else.
	n >= len && is_blank(&rest[n..])
}

/// Reads a division and the blocks within it, which run to its closing fence or to the end of input.
///
/// The fences nest, so a division holds divisions. An opening fence names a class or attributes and an
/// inner opening fence deepens the nesting; a bare fence of colons closes the innermost division still
/// open. A division nobody closed runs to the end, as an unclosed code fence does.
fn div(lines: &[&str], depth: usize, len: usize, attrs: Attrs, refs: &HashMap<String, String>)
	-> Outcome<(Block, usize)>
{
	let mut nest = 1;
	let mut close = None;
	let mut i = 1;
	while i < lines.len() {
		if let Some((flen, open)) = div_fence(lines[i]) {
			if open {
				nest += 1;
			} else if flen >= len {
				nest -= 1;
				if nest == 0 {
					close = Some(i);
					break;
				}
			}
		}
		i += 1;
	}
	let end = match close {
		Some(c)	=> c,
		None	=> lines.len(),
	};
	let content = res!(blocks(&lines[1..end], depth + 1, refs));
	let consumed = match close {
		Some(c)	=> c + 1,
		None	=> lines.len(),
	};
	Ok((Block::Div { attrs, content }, consumed))
}

/// Whether the line is a division fence, how many colons it runs to, and whether it opens.
///
/// A fence that names a class or attributes opens a division; a bare run of colons closes one. An
/// anonymous division opened by a bare fence is read where it does not nest inside another anonymous
/// one, which no prose written to be read does.
fn div_fence(line: &str) -> Option<(usize, bool)> {
	let rest = &line[ws_end(line)..];
	let b = rest.as_bytes();
	if b.is_empty() || b[0] != b':' {
		return None;
	}
	let n = rest.bytes().take_while(|c| *c == b':').count();
	if n < 3 {
		return None;
	}
	Some((n, !rest[n..].trim().is_empty()))
}

/// Reads a block quotation and the blocks within it.
fn quote(lines: &[&str], depth: usize, refs: &HashMap<String, String>) -> Outcome<(Block, usize)> {
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
	let refs_in: Vec<&str> = body.iter().map(|s| s.as_str()).collect();
	Ok((Block::Quote(res!(blocks(&refs_in, depth + 1, refs))), i))
}

/// The line with its quotation marker taken off, if it carries one.
fn quote_strip(line: &str) -> Option<String> {
	match line[ws_end(line)..].strip_prefix('>') {
		// One space after the marker is the marker's own, and no more.
		Some(rest)	=> Some(strip_cols(rest, 1)),
		None		=> None,
	}
}

/// Reads a list: its first item, and every item that follows it at the same level with the same
/// marker.
fn list(lines: &[&str], depth: usize, refs: &HashMap<String, String>) -> Outcome<(Block, usize)> {
	let (ord, mark) = match classify(lines[0]) {
		Start::Item { ord, mark, .. }	=> (ord, mark),
		_				=> return Err(err!(
			"A list was read from a line that does not begin one."; Bug)),
	};
	let mut items = Vec::new();
	let mut i = 0;
	loop {
		// Blank lines may sit between items, but they are the list's only if an item follows.
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
		let refs_in: Vec<&str> = body.iter().map(|s| s.as_str()).collect();
		items.push(res!(blocks(&refs_in, depth + 1, refs)));
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
/// This is the laziness an author leans on when wrapping a quoted or listed paragraph: a line that is
/// only text goes on belonging to the paragraph above.
fn lazy(body: &[String], line: &str) -> bool {
	match body.last() {
		Some(l) if !is_blank(l)	=> matches!(classify(line), Start::Text),
		_			=> false,
	}
}

// ── Whitespace ───────────────────────────────────────────────────

/// The text with the spaces and tabs at either end of it taken off.
fn trim(s: &str) -> &str {
	s.trim_matches(|c| c == ' ' || c == '\t')
}

/// The line with the leading spaces and tabs taken off.
fn strip_leading(line: &str) -> &str {
	line.trim_start_matches(|c| c == ' ' || c == '\t')
}

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
	use crate::doc::Inline;

	/// The text of a block's inlines, for tests that care what a block says and not how.
	fn said(blocks: &[Block]) -> Vec<String> {
		blocks.iter().map(|b| match b {
			Block::Para(c)			=> crate::doc::text_of(c),
			Block::Heading { content, .. }	=> crate::doc::text_of(content),
			Block::Code { text, .. }	=> text.clone(),
			_				=> String::new(),
		}).collect()
	}

	/// What a table's rows say, cell by cell, the header first.
	fn grid(b: &Block) -> Vec<Vec<String>> {
		match b {
			Block::Table { head, rows, .. }	=> {
				let mut out = Vec::new();
				if let Some(head) = head {
					out.push(head.0.iter().map(|c| c.text_of()).collect());
				}
				for row in rows {
					out.push(row.0.iter().map(|c| c.text_of()).collect());
				}
				out
			}
			other				=> panic!("expected a table, got {:?}", other),
		}
	}

	/// A run of hashes and a space open a heading, at each of the six levels.
	#[test]
	fn test_hashes_open_a_heading_at_every_level_00() -> Outcome<()> {
		for n in 1..=6u8 {
			let src = format!("{} Heading\n", "#".repeat(n as usize));
			let b = res!(parse(&src));
			assert_eq!(b, vec![Block::Heading { level: n, content: vec![Inline::Text("Heading".into())] }]);
		}
		// Seven hashes is no heading, since there is no seventh level to give it.
		assert_eq!(said(&res!(parse("####### Seven\n"))), vec!["####### Seven"]);
		Ok(())
	}

	/// A heading's text runs across the lazy lines that follow it, to a blank line.
	#[test]
	fn test_a_heading_carries_on_lazily_01() -> Outcome<()> {
		let b = res!(parse("# A heading that\nwraps a long way\n\nA paragraph.\n"));
		assert_eq!(b.len(), 2);
		assert_eq!(said(&b[..1]), vec!["A heading that wraps a long way"]);
		assert_eq!(said(&b[1..]), vec!["A paragraph."]);
		Ok(())
	}

	/// A soft line ending within a paragraph says a space, so that prose reflows.
	#[test]
	fn test_a_soft_break_says_a_space_02() -> Outcome<()> {
		let b = res!(parse("A paragraph the author\nhard wrapped at a\nnarrow width.\n"));
		assert_eq!(b, vec![Block::Para(vec![
			Inline::Text("A paragraph the author hard wrapped at a narrow width.".into()),
		])]);
		Ok(())
	}

	/// A backslash at the end of a line is a break the author asked for.
	#[test]
	fn test_a_backslash_makes_a_hard_break_03() -> Outcome<()> {
		let b = res!(parse("one\\\ntwo\n"));
		assert_eq!(b, vec![Block::Para(vec![
			Inline::Text("one".into()),
			Inline::Break,
			Inline::Text("two".into()),
		])]);
		Ok(())
	}

	/// A fence holds code exactly as written, with a language or without, by backticks or by tildes.
	#[test]
	fn test_a_fence_holds_code_04() -> Outcome<()> {
		let b = res!(parse("```rust\nlet x = *y;\n```\n"));
		assert_eq!(b, vec![Block::Code { lang: Some("rust".into()), text: "let x = *y;\n".into() }]);
		let b = res!(parse("~~~\nplain\n~~~\n"));
		assert_eq!(b, vec![Block::Code { lang: None, text: "plain\n".into() }]);
		// A fence nobody closed runs to the end.
		let b = res!(parse("```\nstill code\n"));
		assert_eq!(b, vec![Block::Code { lang: None, text: "still code\n".into() }]);
		Ok(())
	}

	/// Three or more of a break's characters make a thematic break.
	#[test]
	fn test_a_thematic_break_05() -> Outcome<()> {
		for src in ["---\n", "***\n", "___\n", "* * *\n"] {
			assert_eq!(res!(parse(src)), vec![Block::Rule], "for {:?}", src);
		}
		Ok(())
	}

	/// A quotation holds blocks, and reads them as a document of its own.
	#[test]
	fn test_a_quotation_holds_blocks_06() -> Outcome<()> {
		let b = res!(parse("> # Heading\n>\n> A paragraph.\n"));
		match &b[0] {
			Block::Quote(inner)	=> assert_eq!(said(inner), vec!["Heading", "A paragraph."]),
			other			=> panic!("expected a quotation, got {:?}", other),
		}
		Ok(())
	}

	/// Bullets make an unordered list, and numbers an ordered one, nesting by indentation.
	#[test]
	fn test_lists_nest_by_indentation_07() -> Outcome<()> {
		let b = res!(parse("- a\n  - inner\n- b\n"));
		match &b[0] {
			Block::List { ordered, items }	=> {
				assert!(!ordered);
				assert_eq!(items.len(), 2);
				assert_eq!(said(&items[0][..1]), vec!["a"]);
				match &items[0][1] {
					Block::List { items: inner, .. }	=> assert_eq!(said(&inner[0]), vec!["inner"]),
					other					=> panic!("expected a nested list, got {:?}", other),
				}
			}
			other				=> panic!("expected a list, got {:?}", other),
		}
		// A number and a delimiter make an ordered list.
		let b = res!(parse("1. one\n2. two\n"));
		match &b[0] {
			Block::List { ordered, items }	=> {
				assert!(ordered);
				assert_eq!(items.len(), 2);
			}
			other				=> panic!("expected a list, got {:?}", other),
		}
		Ok(())
	}

	/// A header row and a delimiter row make a table, whose colons align the columns.
	#[test]
	fn test_a_pipe_table_08() -> Outcome<()> {
		let b = res!(parse("| w | x | y | z |\n| --- | :-- | :-: | --: |\n| 1 | 2 | 3 | 4 |\n"));
		assert_eq!(b.len(), 1);
		match &b[0] {
			Block::Table { cols, .. }	=> assert_eq!(
				cols,
				&vec![Align::None, Align::Start, Align::Centre, Align::End],
			),
			other				=> panic!("expected a table, got {:?}", other),
		}
		assert_eq!(grid(&b[0]), vec![
			vec!["w", "x", "y", "z"],
			vec!["1", "2", "3", "4"],
		]);
		Ok(())
	}

	/// A colon fence names a division, its opening word a class.
	#[test]
	fn test_a_colon_fence_names_a_division_09() -> Outcome<()> {
		let b = res!(parse("::: warning\nMind the step.\n:::\n"));
		assert_eq!(b.len(), 1);
		match &b[0] {
			Block::Div { attrs, content }	=> {
				assert_eq!(attrs.classes, vec!["warning".to_string()]);
				assert!(attrs.id.is_none());
				assert_eq!(said(content), vec!["Mind the step."]);
			}
			other				=> panic!("expected a division, got {:?}", other),
		}
		Ok(())
	}

	/// Divisions nest, one within another.
	#[test]
	fn test_divisions_nest_10() -> Outcome<()> {
		let b = res!(parse("::: outer\nbefore\n::: inner\nwithin\n:::\nafter\n:::\n"));
		assert_eq!(b.len(), 1);
		match &b[0] {
			Block::Div { attrs, content }	=> {
				assert_eq!(attrs.classes, vec!["outer".to_string()]);
				// A paragraph, then the inner division, then a paragraph.
				assert_eq!(content.len(), 3);
				assert_eq!(said(&content[..1]), vec!["before"]);
				match &content[1] {
					Block::Div { attrs, content }	=> {
						assert_eq!(attrs.classes, vec!["inner".to_string()]);
						assert_eq!(said(content), vec!["within"]);
					}
					other				=> panic!("expected an inner division, got {:?}", other),
				}
				assert_eq!(said(&content[2..]), vec!["after"]);
			}
			other				=> panic!("expected a division, got {:?}", other),
		}
		Ok(())
	}

	/// A full brace group after the colons names the division's id, classes and pairs.
	#[test]
	fn test_a_division_takes_a_brace_group_11() -> Outcome<()> {
		let b = res!(parse("::: {#box .note key=val}\nInside.\n:::\n"));
		match &b[0] {
			Block::Div { attrs, .. }	=> {
				assert_eq!(attrs.id, Some("box".to_string()));
				assert_eq!(attrs.classes, vec!["note".to_string()]);
				assert_eq!(attrs.pairs, vec![("key".to_string(), "val".to_string())]);
			}
			other				=> panic!("expected a division, got {:?}", other),
		}
		Ok(())
	}

	/// A standalone attributes line attaches its attributes to the block that follows it.
	#[test]
	fn test_a_standalone_attributes_line_attaches_12() -> Outcome<()> {
		let b = res!(parse("{.note #x}\nA plain paragraph.\n"));
		assert_eq!(b.len(), 1);
		match &b[0] {
			Block::Div { attrs, content }	=> {
				assert_eq!(attrs.classes, vec!["note".to_string()]);
				assert_eq!(attrs.id, Some("x".to_string()));
				assert_eq!(content.len(), 1);
				assert!(matches!(content[0], Block::Para(_)));
				assert_eq!(said(content), vec!["A plain paragraph."]);
			}
			other				=> panic!("expected a division, got {:?}", other),
		}
		Ok(())
	}

	/// A standalone attributes line above a division merges into it rather than wrapping it.
	#[test]
	fn test_a_standalone_line_merges_into_a_division_13() -> Outcome<()> {
		let b = res!(parse("{#a}\n::: note\nInside.\n:::\n"));
		assert_eq!(b.len(), 1);
		match &b[0] {
			Block::Div { attrs, content }	=> {
				// The one box carries both the line's id and the fence's class, not two boxes.
				assert_eq!(attrs.id, Some("a".to_string()));
				assert_eq!(attrs.classes, vec!["note".to_string()]);
				assert_eq!(said(content), vec!["Inside."]);
			}
			other				=> panic!("expected a division, got {:?}", other),
		}
		Ok(())
	}

	/// A reference definition names a link that stands above it, and is no block of its own.
	#[test]
	fn test_a_reference_definition_resolves_a_link_14() -> Outcome<()> {
		let b = res!(parse("See [the site][ref].\n\n[ref]: https://a.b\n"));
		// The definition line is not a block, so only the paragraph remains.
		assert_eq!(b.len(), 1);
		match &b[0] {
			Block::Para(content)	=> assert_eq!(content, &vec![
				Inline::Text("See ".into()),
				Inline::Link { to: "https://a.b".into(), content: vec![Inline::Text("the site".into())] },
				Inline::Text(".".into()),
			]),
			other			=> panic!("expected a paragraph, got {:?}", other),
		}
		Ok(())
	}

	/// A backslash escape makes a literal of the punctuation it stands before.
	#[test]
	fn test_a_backslash_escape_15() -> Outcome<()> {
		let b = res!(parse("A \\*literal\\* star.\n"));
		assert_eq!(b, vec![Block::Para(vec![Inline::Text("A *literal* star.".into())])]);
		Ok(())
	}

	/// Syntax the reader does not yet read survives as the text it is, without crashing.
	#[test]
	fn test_deferred_syntax_survives_16() -> Outcome<()> {
		// Display maths, a definition list and a task list are all not yet read.
		let b = res!(parse("$$x^2$$\n"));
		assert_eq!(said(&b), vec!["$$x^2$$"]);
		let b = res!(parse("A smile :smile: and a note[^1].\n"));
		assert_eq!(said(&b), vec!["A smile :smile: and a note[^1]."]);
		Ok(())
	}

	/// Nesting past the limit is refused, which is the parser's one refusal.
	#[test]
	fn test_nesting_past_the_limit_is_refused_17() -> Outcome<()> {
		let ok = format!("{} deep\n", ">".repeat(DEPTH_LIMIT));
		assert!(parse(&ok).is_ok());
		let deep = format!("{} deep\n", ">".repeat(DEPTH_LIMIT + 8));
		assert!(parse(&deep).is_err());
		Ok(())
	}

	/// An empty document holds nothing, and is not a failure.
	#[test]
	fn test_an_empty_document_holds_nothing_18() -> Outcome<()> {
		assert_eq!(res!(parse("")), Vec::<Block>::new());
		assert_eq!(res!(parse("\n\n \n")), Vec::<Block>::new());
		Ok(())
	}

	/// A whole document of every block reads as the document it is.
	#[test]
	fn test_a_document_of_every_block_19() -> Outcome<()> {
		let src = "\
# Title

An opening paragraph with _emphasis_ and *strength*.

::: note
A boxed aside.
:::

- one
- two
  - nested

> A quotation.

```sh
echo hi
```

| a | b |
| --- | --- |
| 1 | 2 |

---

The end.
";
		let b = res!(parse(src));
		assert!(matches!(b[0], Block::Heading { level: 1, .. }));
		assert!(matches!(b[1], Block::Para(_)));
		assert!(matches!(b[2], Block::Div { .. }));
		assert!(matches!(b[3], Block::List { ordered: false, .. }));
		assert!(matches!(b[4], Block::Quote(_)));
		assert!(matches!(b[5], Block::Code { .. }));
		assert!(matches!(b[6], Block::Table { .. }));
		assert!(matches!(b[7], Block::Rule));
		assert!(matches!(b[8], Block::Para(_)));
		assert_eq!(b.len(), 9);
		Ok(())
	}
}
