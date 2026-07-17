//! Inline structure: the pass that reads a line of prose into text, emphasis, links, images, code
//! spans, attributed spans and hard breaks.
//!
//! This is the second of the two passes described in [`crate::doc::djot::block`]. It is given the text
//! of one block, along with the document's reference definitions, and returns the run of inlines it is
//! made of.
//!
//! # Where Djot parts from Markdown
//!
//! The markers are swapped and single. A single `_` is ordinary emphasis and a single `*` is strong,
//! so Djot needs no doubling: `_it_` is italic and `*it*` is bold, the reverse of a Markdown reader's
//! reflex. A run of a marker either side opens and closes by whitespace alone -- a marker may sit
//! against a word on the inside and not on the outside -- which is what lets emphasis fall inside a
//! word without the intraword rule Markdown carries for its underscore.
//!
//! A bracket that is not a link may be a span. `[text]{.c}` is an [`Inline::Span`] carrying the
//! attributes in the braces, the inline counterpart of a `:::` division; `[text](url)` is a link as
//! ever; and `[text]` on its own is a reference link where the document defined the reference, and
//! literal brackets where it did not.

use crate::doc::{
	Attrs,
	Inline,
	text_of,
	djot::block::DEPTH_LIMIT,
};

use std::collections::HashMap;

use oxedyne_fe2o3_core::prelude::*;

/// Reads a block's text into its run of inline elements.
///
/// The reference definitions are the map [`crate::doc::djot::block`] gathered in its first pass, by
/// which a `[text][ref]` or a bare `[text]` resolves to the destination the document named elsewhere.
pub fn parse(src: &str, refs: &HashMap<String, String>) -> Outcome<Vec<Inline>> {
	run(src, 0, refs)
}

/// One element of the scan: either an inline that is settled, or a run of emphasis characters still
/// looking for the run that answers it.
///
/// Emphasis cannot be read in one pass, because whether a `*` opens anything is only known once
/// something closes it. So the scan lays the delimiters out alongside the text it is sure of, and
/// [`resolve`] pairs them off afterwards.
enum Node {
	/// An inline that needs nothing more, and how deep the tree it makes runs.
	Done(Inline, usize),
	/// A run of emphasis characters.
	Delim {
		/// The character the run is made of.
		ch:	u8,
		/// How many characters the run has left to spend.
		len:	usize,
		/// Whether the run may open emphasis.
		open:	bool,
		/// Whether the run may close emphasis.
		close:	bool,
	},
}

/// Reads a run of text into inlines, at the given nesting depth.
fn run(src: &str, depth: usize, refs: &HashMap<String, String>) -> Outcome<Vec<Inline>> {
	if depth > DEPTH_LIMIT {
		return Err(err!(
			"Djot inlines nest more than {} deep, which no prose written to be read \
			does.", DEPTH_LIMIT;
			Excessive, Input));
	}
	let nodes = res!(scan(src, depth, refs));
	let (out, _) = res!(resolve(nodes, depth));
	Ok(out)
}

/// Lays the text out as settled inlines and unsettled emphasis delimiters.
fn scan(src: &str, depth: usize, refs: &HashMap<String, String>) -> Outcome<Vec<Node>> {
	let mut out: Vec<Node> = Vec::new();
	let mut buf = String::new();	// Text gathered since the last settled inline.
	let b = src.as_bytes();
	let mut i = 0;
	while i < b.len() {
		match b[i] {
			b'\\'	=> {
				if i + 1 < b.len() && b[i + 1] == b'\n' {
					// A backslash at the end of a line is a break the author asked for.
					flush(&mut out, &mut buf);
					out.push(Node::Done(Inline::Break, 1));
					i += 2;
				} else if i + 1 < b.len() && b[i + 1].is_ascii_punctuation() {
					// A backslash before punctuation says the punctuation is only itself.
					buf.push(b[i + 1] as char);
					i += 2;
				} else {
					buf.push('\\');
					i += 1;
				}
			}
			b'\n'	=> {
				// Djot has no two-space hard break, so a bare line ending is always soft. The
				// spaces before it mean nothing and are dropped.
				let sp = buf.len() - buf.trim_end_matches(' ').len();
				buf.truncate(buf.len() - sp);
				if !(out.is_empty() && buf.is_empty()) && i + 1 < b.len() {
					// A soft break: where the author's editor wrapped the line, not where the
					// author meant a break. It says a space, so that prose hard wrapped to one
					// width reflows to whatever width reads it. A soft break at either end of the
					// run says nothing at all.
					buf.push(' ');
				}
				i += 1;
			}
			b'`'	=> {
				let n = run_len(b, i, b'`');
				match code_span(src, i, n) {
					Some((code, end))	=> {
						flush(&mut out, &mut buf);
						out.push(Node::Done(Inline::Code(code), 1));
						i = end;
					}
					None			=> {
						// Nothing closed it, so the backticks are backticks.
						for _ in 0..n {
							buf.push('`');
						}
						i += n;
					}
				}
			}
			b'<'	=> {
				match autolink(src, i) {
					Some((to, end))	=> {
						flush(&mut out, &mut buf);
						out.push(Node::Done(Inline::Link {
							to:		to.clone(),
							content:	vec![Inline::Text(to)],
						}, 1));
						i = end;
					}
					None		=> {
						buf.push('<');
						i += 1;
					}
				}
			}
			b'!' if i + 1 < b.len() && b[i + 1] == b'[' => {
				match image(src, i, depth, refs) {
					Some((res, end))	=> {
						flush(&mut out, &mut buf);
						out.push(Node::Done(res!(res), 1));
						i = end;
					}
					None			=> {
						buf.push('!');
						i += 1;
					}
				}
			}
			b'['	=> {
				match bracket(src, i, depth, refs) {
					Some((res, end))	=> {
						flush(&mut out, &mut buf);
						out.push(Node::Done(res!(res), 1));
						i = end;
					}
					None			=> {
						buf.push('[');
						i += 1;
					}
				}
			}
			b'*' | b'_'	=> {
				let ch = b[i];
				let len = run_len(b, i, ch);
				let (open, close) = flank(src, i, i + len);
				flush(&mut out, &mut buf);
				out.push(Node::Delim { ch, len, open, close });
				i += len;
			}
			_	=> {
				// Anything else is itself, taken a whole character at a time.
				let j = char_end(src, i);
				buf.push_str(&src[i..j]);
				i = j;
			}
		}
	}
	flush(&mut out, &mut buf);
	Ok(out)
}

/// Adds the text gathered so far as one run, so that adjacent text is never split in two.
fn flush(out: &mut Vec<Node>, buf: &mut String) {
	if !buf.is_empty() {
		out.push(Node::Done(Inline::Text(std::mem::take(buf)), 1));
	}
}

// ── Emphasis ─────────────────────────────────────────────────────

/// Pairs emphasis delimiters with the runs that answer them, and makes text of the rest.
///
/// Each run that could close is offered to the nearest run before it that could open. A run that
/// finds no partner is not markup at all, and comes out as the characters it is made of -- which is
/// why a stray asterisk is an asterisk. A pairing spends one character of each run, and the character
/// decides the kind: a `*` makes strong emphasis and a `_` ordinary emphasis, the Djot reading rather
/// than the Markdown one.
///
/// Returns the inlines, and how deep the deepest of them runs. The depth is carried rather than
/// measured afterwards because a run of emphasis characters nests one level per pair without the
/// parser recursing once: ten thousand asterisks would build a tree too deep to walk, and only a
/// count kept as it is built catches that before it exists.
fn resolve(mut nodes: Vec<Node>, depth: usize) -> Outcome<(Vec<Inline>, usize)> {
	let mut i = 0;
	while i < nodes.len() {
		let ch = match &nodes[i] {
			Node::Delim { ch, close: true, .. }	=> *ch,
			_					=> {
				i += 1;
				continue;
			}
		};
		// Look back for the nearest run of the same character that could open.
		let mut found = None;
		let mut k = i;
		while k > 0 {
			k -= 1;
			if let Node::Delim { ch: c, open: true, .. } = &nodes[k] {
				if *c == ch {
					found = Some(k);
					break;
				}
			}
		}
		let k = match found {
			Some(k)	=> k,
			None	=> {
				// Nothing opened it, so it closes nothing and is only what it looks like.
				if let Node::Delim { close, .. } = &mut nodes[i] {
					*close = false;
				}
				i += 1;
				continue;
			}
		};
		// Everything between the two runs is what they emphasise.
		let inner: Vec<Node> = nodes.drain(k + 1..i).collect();
		let (content, cd) = res!(resolve(inner, depth + 1));
		let d = cd + 1;
		if depth + d > DEPTH_LIMIT {
			return Err(err!(
				"Djot emphasis nests more than {} deep, which no prose written to be \
				read does.", DEPTH_LIMIT;
				Excessive, Input));
		}
		// A pairing spends one character of each run, whichever length the runs are.
		if let Node::Delim { len, .. } = &mut nodes[k] {
			*len -= 1;
		}
		if let Node::Delim { len, .. } = &mut nodes[k + 1] {
			*len -= 1;
		}
		nodes.insert(k + 1, Node::Done(Inline::Emph { strong: ch == b'*', content }, d));
		// The closing run now sits past the emphasis it made. A run with characters left over may
		// still make more, so it is looked at again.
		let mut ci = k + 2;
		if matches!(&nodes[ci], Node::Delim { len: 0, .. }) {
			nodes.remove(ci);
		}
		if matches!(&nodes[k], Node::Delim { len: 0, .. }) {
			nodes.remove(k);
			ci -= 1;
		}
		i = ci;
	}
	let mut out = Vec::new();
	let mut md = 0;	// How deep the deepest inline runs.
	for node in nodes {
		match node {
			Node::Done(item, d)		=> {
				if d > md {
					md = d;
				}
				push(&mut out, item);
			}
			Node::Delim { ch, len, .. }	=> {
				if len > 0 {
					md = md.max(1);
					push(&mut out, Inline::Text(
						std::iter::repeat(ch as char).take(len).collect()));
				}
			}
		}
	}
	Ok((out, md))
}

/// Adds an inline, joining it to the run before it when both are text.
fn push(out: &mut Vec<Inline>, item: Inline) {
	if let Inline::Text(t) = &item {
		if let Some(Inline::Text(last)) = out.last_mut() {
			last.push_str(t);
			return;
		}
	}
	out.push(item);
}

/// Whether a run of emphasis characters may open, and may close, by the whitespace either side of it.
///
/// Djot's rule is the plain one: a run may open where a non-space follows it, and close where a
/// non-space precedes it. There is no intraword exception, because there is nothing to except: an
/// underscore against letters on both sides both opens and closes, and so emphasis falls inside a word
/// where an author writes it there.
fn flank(src: &str, start: usize, end: usize) -> (bool, bool) {
	let prev = src[..start].chars().next_back();
	let next = src[end..].chars().next();
	let pre_ws = match prev { Some(c) => c.is_whitespace(), None => true };
	let post_ws = match next { Some(c) => c.is_whitespace(), None => true };
	// Opens where a non-space follows; closes where a non-space precedes.
	(!post_ws, !pre_ws)
}

// ── Code spans, links, images and spans ──────────────────────────

/// A code span opened by a run of `n` backticks at `i`, and the offset just past it.
fn code_span(src: &str, i: usize, n: usize) -> Option<(String, usize)> {
	let b = src.as_bytes();
	let mut j = i + n;
	while j < b.len() {
		if b[j] == b'`' {
			// Only a run of the same length closes: a longer or shorter one is code.
			let m = run_len(b, j, b'`');
			if m == n {
				return Some((code_text(&src[i + n..j]), j + m));
			}
			j += m;
			continue;
		}
		j += 1;
	}
	None
}

/// A code span's text: line endings become spaces, and a space at each end is dropped so that a span
/// may hold a backtick of its own.
fn code_text(raw: &str) -> String {
	let s: String = raw.chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
	if s.len() >= 2 && s.starts_with(' ') && s.ends_with(' ') && !s.trim().is_empty() {
		s[1..s.len() - 1].to_string()
	} else {
		s
	}
}

/// An autolink at the offset: the URI it names, and the offset just past it.
fn autolink(src: &str, i: usize) -> Option<(String, usize)> {
	let b = src.as_bytes();
	let mut j = i + 1;
	while j < b.len() {
		match b[j] {
			b'>'					=> break,
			b'<' | b' ' | b'\t' | b'\n'		=> return None,
			_					=> j += 1,
		}
	}
	if j >= b.len() {
		return None;
	}
	let inner = &src[i + 1..j];
	if !is_uri(inner) {
		return None;
	}
	Some((inner.to_string(), j + 1))
}

/// Whether the text is a URI with a scheme, which is what an autolink must be.
fn is_uri(s: &str) -> bool {
	let colon = match s.find(':') {
		Some(c)	=> c,
		None	=> return false,
	};
	let b = s.as_bytes();
	if colon < 2 || colon > 32 || !b[0].is_ascii_alphabetic() {
		return false;
	}
	b[1..colon].iter().all(|c| c.is_ascii_alphanumeric() || *c == b'+' || *c == b'.' || *c == b'-')
}

/// What a `[` at the offset settles to, and the offset just past it, or nothing where the bracket is
/// only a bracket.
///
/// The character against the closing bracket decides. A `(` makes an inline link, a `{` an attributed
/// span, and a `[` a reference link; a bare bracket is a shortcut reference where the document defined
/// its text, and otherwise nothing, so the caller writes the `[` as the character it is.
fn bracket(src: &str, i: usize, depth: usize, refs: &HashMap<String, String>)
	-> Option<(Outcome<Inline>, usize)>
{
	let b = src.as_bytes();
	let close = match bracket_end(src, i) {
		Some(c)	=> c,
		None	=> return None,
	};
	let inner = &src[i + 1..close];
	let after = close + 1;
	// A destination against the bracket makes a link.
	if after < b.len() && b[after] == b'(' {
		if let Some((to, end)) = dest(src, after) {
			return Some((link(to, inner, depth, refs), end));
		}
	}
	// An attribute block against the bracket makes a span.
	if after < b.len() && b[after] == b'{' {
		if let Some((attrs, used)) = attrs_of(&src[after..]) {
			return Some((span(attrs, inner, depth, refs), after + used));
		}
	}
	// A second bracket names a reference the document may have defined.
	if after < b.len() && b[after] == b'[' {
		if let Some((label, end)) = ref_label(src, after) {
			let key = if label.trim().is_empty() { normalise(inner) } else { normalise(&label) };
			if let Some(to) = refs.get(&key) {
				return Some((link(to.clone(), inner, depth, refs), end));
			}
		}
	}
	// A bracket on its own is a shortcut reference where its own text was defined.
	if let Some(to) = refs.get(&normalise(inner)) {
		return Some((link(to.clone(), inner, depth, refs), after));
	}
	None
}

/// What a `![` at the offset settles to, and the offset just past it, or nothing where it is not an
/// image at all.
fn image(src: &str, i: usize, depth: usize, refs: &HashMap<String, String>)
	-> Option<(Outcome<Inline>, usize)>
{
	let b = src.as_bytes();
	let close = match bracket_end(src, i + 1) {
		Some(c)	=> c,
		None	=> return None,
	};
	let inner = &src[i + 2..close];
	let after = close + 1;
	if after < b.len() && b[after] == b'(' {
		if let Some((to, end)) = dest(src, after) {
			return Some((img(to, inner, depth, refs), end));
		}
	}
	if after < b.len() && b[after] == b'[' {
		if let Some((label, end)) = ref_label(src, after) {
			let key = if label.trim().is_empty() { normalise(inner) } else { normalise(&label) };
			if let Some(to) = refs.get(&key) {
				return Some((img(to.clone(), inner, depth, refs), end));
			}
		}
	}
	if let Some(to) = refs.get(&normalise(inner)) {
		return Some((img(to.clone(), inner, depth, refs), after));
	}
	None
}

/// A link to `to`, its text read as prose in its own right.
fn link(to: String, inner: &str, depth: usize, refs: &HashMap<String, String>) -> Outcome<Inline> {
	let content = res!(run(inner, depth + 1, refs));
	Ok(Inline::Link { to, content })
}

/// An image at `src`, its alt text the flattened words of what stood for it.
fn img(to: String, inner: &str, depth: usize, refs: &HashMap<String, String>) -> Outcome<Inline> {
	// An image stands for itself in words, so its alt is flattened.
	let alt = text_of(&res!(run(inner, depth + 1, refs)));
	Ok(Inline::Image { src: to, alt })
}

/// A span carrying the attributes in the braces, its content read as prose.
fn span(attrs: Attrs, inner: &str, depth: usize, refs: &HashMap<String, String>) -> Outcome<Inline> {
	let content = res!(run(inner, depth + 1, refs));
	Ok(Inline::Span { attrs, content })
}

/// The offset of the `]` that closes the `[` at `i`, if one does.
///
/// Brackets nest, and a bracket within a code span is code and not a bracket, so both are stepped
/// over the way the inline pass steps over them everywhere else.
fn bracket_end(src: &str, i: usize) -> Option<usize> {
	let b = src.as_bytes();
	let mut j = i + 1;
	let mut d = 1;	// Bracket depth.
	while j < b.len() {
		match b[j] {
			b'\\'	=> {
				j = skip_esc(src, j);
				continue;
			}
			b'`'	=> {
				let n = run_len(b, j, b'`');
				j = match code_span(src, j, n) {
					Some((_, end))	=> end,
					None		=> j + n,
				};
				continue;
			}
			b'['	=> d += 1,
			b']'	=> {
				d -= 1;
				if d == 0 {
					return Some(j);
				}
			}
			_	=> {}
		}
		j += 1;
	}
	None
}

/// The label of a `[ref]` at `i`, and the offset just past its closing bracket.
///
/// A collapsed reference `[]` gives an empty label, which the caller reads as a call to use the link's
/// own text as the label instead.
fn ref_label(src: &str, i: usize) -> Option<(String, usize)> {
	let b = src.as_bytes();
	let mut j = i + 1;
	while j < b.len() {
		match b[j] {
			b'\\'	=> {
				j = skip_esc(src, j);
				continue;
			}
			b']'	=> return Some((src[i + 1..j].to_string(), j + 1)),
			_	=> j += 1,
		}
	}
	None
}

/// A link destination in parentheses at the offset: the destination, and the offset just past the
/// parenthesis that closes it.
///
/// A title, which this tree keeps no room for, is read only so as to be stepped over.
fn dest(src: &str, i: usize) -> Option<(String, usize)> {
	let b = src.as_bytes();
	let mut j = skip_ws(b, i + 1);
	let to;
	if j < b.len() && b[j] == b'<' {
		// An angled destination runs to its closing angle, and may hold spaces.
		let s = j + 1;
		let mut k = s;
		while k < b.len() && b[k] != b'>' && b[k] != b'\n' {
			k = if b[k] == b'\\' { skip_esc(src, k) } else { k + 1 };
		}
		if k >= b.len() || b[k] != b'>' {
			return None;
		}
		to = unescape(&src[s..k]);
		j = k + 1;
	} else {
		let s = j;
		let mut d = 0;	// Parenthesis depth.
		let mut k = j;
		while k < b.len() {
			match b[k] {
				b'\\'			=> {
					k = skip_esc(src, k);
					continue;
				}
				b'('			=> d += 1,
				b')'			=> {
					if d == 0 {
						break;
					}
					d -= 1;
				}
				b' ' | b'\t' | b'\n'	=> break,
				_			=> {}
			}
			k += 1;
		}
		to = unescape(&src[s..k]);
		j = k;
	}
	j = skip_ws(b, j);
	if j < b.len() && (b[j] == b'"' || b[j] == b'\'' || b[j] == b'(') {
		let shut = if b[j] == b'(' { b')' } else { b[j] };
		let mut k = j + 1;
		while k < b.len() && b[k] != shut {
			k = if b[k] == b'\\' { skip_esc(src, k) } else { k + 1 };
		}
		if k >= b.len() {
			return None;
		}
		j = skip_ws(b, k + 1);
	}
	if j >= b.len() || b[j] != b')' {
		return None;
	}
	Some((to, j + 1))
}

/// Text with its backslash escapes of punctuation resolved.
fn unescape(s: &str) -> String {
	let b = s.as_bytes();
	let mut out = String::new();
	let mut i = 0;
	while i < b.len() {
		if b[i] == b'\\' && i + 1 < b.len() && b[i + 1].is_ascii_punctuation() {
			out.push(b[i + 1] as char);
			i += 2;
			continue;
		}
		let j = char_end(s, i);
		out.push_str(&s[i..j]);
		i = j;
	}
	out
}

// ── Attributes ───────────────────────────────────────────────────

/// The attributes a `{...}` at the start of `src` names, and how many bytes it runs to, or nothing
/// where the braces do not close.
///
/// Shared by the inline span, the `:::` division and the standalone attributes line, so that all three
/// read a brace group the one way. A quoted value may hold a `}` of its own, so the scan for the
/// closing brace steps over what a quote encloses.
pub fn attrs_of(src: &str) -> Option<(Attrs, usize)> {
	let b = src.as_bytes();
	if b.is_empty() || b[0] != b'{' {
		return None;
	}
	let mut j = 1;
	let mut quoted = false;
	while j < b.len() {
		match b[j] {
			b'"'			=> quoted = !quoted,
			b'}' if !quoted	=> return Some((parse_attrs(&src[1..j]), j + 1)),
			_			=> {}
		}
		j += 1;
	}
	None
}

/// Parses the interior of a brace group into its id, classes and pairs.
///
/// The items are space-separated: `.name` adds a class, `#name` sets the id, and `key=value` or
/// `key="quoted value"` adds a pair. The reading is lenient, since an attribute block is the author's
/// note to a stylesheet and not a program: what does not parse is passed over rather than refused.
pub fn parse_attrs(inner: &str) -> Attrs {
	let mut a = Attrs::default();
	let b = inner.as_bytes();
	let mut i = 0;
	while i < b.len() {
		match b[i] {
			b' ' | b'\t' | b'\n' | b'\r' | b','	=> i += 1,
			b'.'	=> {
				let s = i + 1;
				let mut j = s;
				while j < b.len() && !is_attr_sep(b[j]) {
					j += 1;
				}
				if j > s {
					a.classes.push(inner[s..j].to_string());
				}
				i = j;
			}
			b'#'	=> {
				let s = i + 1;
				let mut j = s;
				while j < b.len() && !is_attr_sep(b[j]) {
					j += 1;
				}
				if j > s {
					// The last id written wins, as Djot has it.
					a.id = Some(inner[s..j].to_string());
				}
				i = j;
			}
			_	=> {
				let s = i;
				let mut j = i;
				while j < b.len() && b[j] != b'=' && !is_attr_sep(b[j]) {
					j += 1;
				}
				let key = &inner[s..j];
				if j < b.len() && b[j] == b'=' {
					let k = j + 1;
					if k < b.len() && b[k] == b'"' {
						let vs = k + 1;
						let mut m = vs;
						while m < b.len() && b[m] != b'"' {
							m += 1;
						}
						if !key.is_empty() {
							a.pairs.push((key.to_string(), inner[vs..m].to_string()));
						}
						i = if m < b.len() { m + 1 } else { m };
					} else {
						let vs = k;
						let mut m = vs;
						while m < b.len() && !is_attr_sep(b[m]) {
							m += 1;
						}
						if !key.is_empty() {
							a.pairs.push((key.to_string(), inner[vs..m].to_string()));
						}
						i = m;
					}
				} else {
					// A bare word with no value names nothing this tree carries, so it is passed
					// over. The offset still advances, so a stray character cannot loop.
					i = if j > s { j } else { i + 1 };
				}
			}
		}
	}
	a
}

/// Whether the byte ends an attribute item.
fn is_attr_sep(c: u8) -> bool {
	c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' || c == b','
}

/// A reference label folded to the form two spellings of it share: trimmed, its inner whitespace
/// collapsed, and its case set aside.
///
/// A reference is matched by what it names and not by how it was typed, so `[My Ref]` and `[my  ref]`
/// reach the one definition. The block pass folds a definition's label the same way, so the two meet.
pub fn normalise(s: &str) -> String {
	s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

// ── Offsets ──────────────────────────────────────────────────────

/// How many of the given character run on from the offset.
fn run_len(b: &[u8], i: usize, ch: u8) -> usize {
	let mut n = 0;
	while i + n < b.len() && b[i + n] == ch {
		n += 1;
	}
	n
}

/// The offset just past the character at the offset.
fn char_end(s: &str, i: usize) -> usize {
	let mut j = i + 1;
	while j < s.len() && !s.is_char_boundary(j) {
		j += 1;
	}
	j
}

/// The offset just past a backslash at `i` and whatever it escapes.
fn skip_esc(s: &str, i: usize) -> usize {
	if i + 1 < s.len() { char_end(s, i + 1) } else { i + 1 }
}

/// The offset past any spaces, tabs and line endings at `i`.
fn skip_ws(b: &[u8], i: usize) -> usize {
	let mut j = i;
	while j < b.len() && (b[j] == b' ' || b[j] == b'\t' || b[j] == b'\n') {
		j += 1;
	}
	j
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A run of literal text, for the tests that expect one.
	fn t(s: &str) -> Inline {
		Inline::Text(s.to_string())
	}

	/// Reads inline text with no reference definitions, which is what most tests want.
	fn parse0(src: &str) -> Outcome<Vec<Inline>> {
		parse(src, &HashMap::new())
	}

	/// Plain prose is one run of text and not a run for every character.
	#[test]
	fn test_plain_prose_is_one_run_of_text_00() -> Outcome<()> {
		assert_eq!(res!(parse0("Just some prose.")), vec![t("Just some prose.")]);
		Ok(())
	}

	/// An underscore either side is ordinary emphasis, and an asterisk either side is strong: the
	/// Djot reading, and the reverse of Markdown's.
	#[test]
	fn test_the_markers_are_the_djot_way_round_01() -> Outcome<()> {
		assert_eq!(res!(parse0("an _italic_ word")), vec![
			t("an "),
			Inline::Emph { strong: false, content: vec![t("italic")] },
			t(" word"),
		]);
		assert_eq!(res!(parse0("a *bold* word")), vec![
			t("a "),
			Inline::Emph { strong: true, content: vec![t("bold")] },
			t(" word"),
		]);
		Ok(())
	}

	/// The markers are not swapped: `_` is never strong and `*` is never ordinary.
	#[test]
	fn test_the_markers_are_not_swapped_02() -> Outcome<()> {
		match &res!(parse0("_x_"))[0] {
			Inline::Emph { strong, .. }	=> assert!(!strong, "an underscore is ordinary emphasis"),
			other				=> panic!("expected emphasis, got {:?}", other),
		}
		match &res!(parse0("*x*"))[0] {
			Inline::Emph { strong, .. }	=> assert!(*strong, "an asterisk is strong emphasis"),
			other				=> panic!("expected emphasis, got {:?}", other),
		}
		Ok(())
	}

	/// Emphasis nests within emphasis, of either kind.
	#[test]
	fn test_emphasis_nests_03() -> Outcome<()> {
		assert_eq!(res!(parse0("_a *b* c_")), vec![
			Inline::Emph {
				strong:		false,
				content:	vec![
					t("a "),
					Inline::Emph { strong: true, content: vec![t("b")] },
					t(" c"),
				],
			},
		]);
		Ok(())
	}

	/// Emphasis falls inside a word where an author writes it there, since Djot keeps no intraword
	/// exception.
	#[test]
	fn test_emphasis_falls_inside_a_word_04() -> Outcome<()> {
		assert_eq!(res!(parse0("a_b_c")), vec![
			t("a"),
			Inline::Emph { strong: false, content: vec![t("b")] },
			t("c"),
		]);
		Ok(())
	}

	/// A marker with space against it emphasises nothing, and stays the character it is.
	#[test]
	fn test_a_marker_with_space_is_a_character_05() -> Outcome<()> {
		assert_eq!(res!(parse0("2 * 3 * 4")), vec![t("2 * 3 * 4")]);
		assert_eq!(res!(parse0("a _ b")), vec![t("a _ b")]);
		// A marker that opens nothing that closes is only itself.
		assert_eq!(res!(parse0("*not strong")), vec![t("*not strong")]);
		Ok(())
	}

	/// Backticks make a verbatim span, and what is in it is exactly what was written.
	#[test]
	fn test_backticks_make_a_verbatim_span_06() -> Outcome<()> {
		assert_eq!(res!(parse0("a `let x = *y*;` b")), vec![
			t("a "),
			Inline::Code("let x = *y*;".to_string()),
			t(" b"),
		]);
		// A longer run lets a span hold a backtick of its own.
		assert_eq!(res!(parse0("`` a ` b ``")), vec![Inline::Code("a ` b".to_string())]);
		Ok(())
	}

	/// A bracket against a destination is a link, its text prose in its own right.
	#[test]
	fn test_a_bracket_against_a_destination_is_a_link_07() -> Outcome<()> {
		assert_eq!(res!(parse0("[text](https://a.b)")), vec![
			Inline::Link {
				to:		"https://a.b".to_string(),
				content:	vec![t("text")],
			},
		]);
		assert_eq!(res!(parse0("[a _b_](c)")), vec![
			Inline::Link {
				to:		"c".to_string(),
				content:	vec![t("a "), Inline::Emph { strong: false, content: vec![t("b")] }],
			},
		]);
		Ok(())
	}

	/// A bang before a link makes an image, whose alt is its text in words.
	#[test]
	fn test_a_bang_before_a_link_makes_an_image_08() -> Outcome<()> {
		assert_eq!(res!(parse0("![a picture](p.png)")), vec![
			Inline::Image { src: "p.png".to_string(), alt: "a picture".to_string() },
		]);
		// The alt is flattened, so emphasis within it does not lose its words.
		assert_eq!(res!(parse0("![an _italic_ picture](p.png)")), vec![
			Inline::Image { src: "p.png".to_string(), alt: "an italic picture".to_string() },
		]);
		Ok(())
	}

	/// A bracket followed by an attribute block is a span, carrying the attributes the braces named.
	#[test]
	fn test_a_bracket_with_attributes_is_a_span_09() -> Outcome<()> {
		assert_eq!(res!(parse0("[text]{.cls #id key=val}")), vec![
			Inline::Span {
				attrs:		Attrs {
					id:		Some("id".to_string()),
					classes:	vec!["cls".to_string()],
					pairs:		vec![("key".to_string(), "val".to_string())],
				},
				content:	vec![t("text")],
			},
		]);
		// A quoted value holds its spaces.
		match &res!(parse0("[t]{key=\"a value\"}"))[0] {
			Inline::Span { attrs, .. }	=> assert_eq!(
				attrs.pairs,
				vec![("key".to_string(), "a value".to_string())],
			),
			other				=> panic!("expected a span, got {:?}", other),
		}
		Ok(())
	}

	/// A bracket against neither a destination nor an attribute block is only brackets.
	#[test]
	fn test_a_bare_bracket_is_text_10() -> Outcome<()> {
		assert_eq!(res!(parse0("[just brackets]")), vec![t("[just brackets]")]);
		assert_eq!(res!(parse0("an [unclosed bracket")), vec![t("an [unclosed bracket")]);
		Ok(())
	}

	/// A reference link resolves to the destination the document defined, whether named or collapsed
	/// or a bare shortcut.
	#[test]
	fn test_a_reference_link_resolves_11() -> Outcome<()> {
		let mut refs = HashMap::new();
		refs.insert("ref".to_string(), "https://x".to_string());
		// A named reference.
		assert_eq!(res!(parse("[text][ref]", &refs)), vec![
			Inline::Link { to: "https://x".to_string(), content: vec![t("text")] },
		]);
		// A collapsed reference reads the label from the text.
		assert_eq!(res!(parse("[ref][]", &refs)), vec![
			Inline::Link { to: "https://x".to_string(), content: vec![t("ref")] },
		]);
		// A shortcut reference, the same way.
		assert_eq!(res!(parse("[ref]", &refs)), vec![
			Inline::Link { to: "https://x".to_string(), content: vec![t("ref")] },
		]);
		// A reference nobody defined is only brackets.
		assert_eq!(res!(parse("[undefined]", &refs)), vec![t("[undefined]")]);
		Ok(())
	}

	/// A soft line ending says a space, so that prose reflows to whatever reads it.
	#[test]
	fn test_a_soft_line_ending_is_a_space_12() -> Outcome<()> {
		assert_eq!(res!(parse0("one\ntwo")), vec![t("one two")]);
		// A soft break at either end says nothing.
		assert_eq!(res!(parse0("one\n")), vec![t("one")]);
		assert_eq!(res!(parse0("\none")), vec![t("one")]);
		Ok(())
	}

	/// A backslash before a line ending is a break the author asked for, and two trailing spaces are
	/// not, since Djot has no such break.
	#[test]
	fn test_a_backslash_before_a_line_ending_breaks_13() -> Outcome<()> {
		assert_eq!(res!(parse0("one\\\ntwo")), vec![t("one"), Inline::Break, t("two")]);
		// Two trailing spaces are only a soft break, and say a space.
		assert_eq!(res!(parse0("one  \ntwo")), vec![t("one two")]);
		Ok(())
	}

	/// A backslash before punctuation says the punctuation is only itself.
	#[test]
	fn test_a_backslash_escapes_punctuation_14() -> Outcome<()> {
		assert_eq!(res!(parse0("\\*not strong\\*")), vec![t("*not strong*")]);
		assert_eq!(res!(parse0("\\[not a link\\]")), vec![t("[not a link]")]);
		Ok(())
	}

	/// Syntax the reader does not yet read survives as the text it is, and does not crash it.
	#[test]
	fn test_deferred_syntax_survives_as_text_15() -> Outcome<()> {
		// Inline maths, a symbol, a footnote reference and a superscript are all not yet read.
		assert_eq!(res!(parse0("an equation $x$ here")), vec![t("an equation $x$ here")]);
		assert_eq!(res!(parse0("a smile :smile: here")), vec![t("a smile :smile: here")]);
		assert_eq!(res!(parse0("a note[^1] here")), vec![t("a note[^1] here")]);
		assert_eq!(res!(parse0("H~2~O and x^2^")), vec![t("H~2~O and x^2^")]);
		Ok(())
	}

	/// Emphasis nested past the limit is refused, as it is in the block pass.
	#[test]
	fn test_nesting_past_the_limit_is_refused_16() -> Outcome<()> {
		// A run the limit allows is read.
		let ok = format!("{}a{}", "*".repeat(DEPTH_LIMIT - 1), "*".repeat(DEPTH_LIMIT - 1));
		assert!(parse0(&ok).is_ok());
		// A run built to exhaust the stack of whatever reads the tree is refused rather than built.
		for n in [DEPTH_LIMIT + 2, 100_000] {
			let src = format!("{}a{}", "*".repeat(n), "*".repeat(n));
			assert!(parse0(&src).is_err(), "for a run of {}", n);
		}
		Ok(())
	}

	/// A verbatim span outranks the emphasis and brackets that appear to be within it.
	#[test]
	fn test_a_verbatim_span_outranks_what_is_in_it_17() -> Outcome<()> {
		assert_eq!(res!(parse0("`[a](b)`")), vec![Inline::Code("[a](b)".to_string())]);
		Ok(())
	}
}
