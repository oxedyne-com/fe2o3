//! Inline structure: the pass that reads a line of prose into text, emphasis, links, images, code
//! spans and hard breaks.
//!
//! This is the second of the two passes described in [`crate::markdown::block`]. It is given the text
//! of one block and returns the run of inlines it is made of.

use crate::markdown::{
	Inline,
	block::DEPTH_LIMIT,
	text_of,
};

use oxedyne_fe2o3_core::prelude::*;

/// Reads a block's text into its run of inline elements.
pub fn parse(src: &str) -> Outcome<Vec<Inline>> {
	run(src, 0)
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
fn run(src: &str, depth: usize) -> Outcome<Vec<Inline>> {
	if depth > DEPTH_LIMIT {
		return Err(err!(
			"Markdown inlines nest more than {} deep, which no prose written to be read \
			does.", DEPTH_LIMIT;
			Excessive, Input));
	}
	let nodes = res!(scan(src, depth));
	let (out, _) = res!(resolve(nodes, depth));
	Ok(out)
}

/// Lays the text out as settled inlines and unsettled emphasis delimiters.
fn scan(src: &str, depth: usize) -> Outcome<Vec<Node>> {
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
				// The spaces before a line ending say what the ending meant.
				let sp = buf.len() - buf.trim_end_matches(' ').len();
				buf.truncate(buf.len() - sp);
				if sp >= 2 {
					// Two or more are a break the author asked for.
					flush(&mut out, &mut buf);
					out.push(Node::Done(Inline::Break, 1));
				} else if !(out.is_empty() && buf.is_empty()) && i + 1 < b.len() {
					// Fewer are a soft break: where the author's editor wrapped the line,
					// not where the author meant a break. It says a space, so that prose
					// hard wrapped to one width reflows to whatever width reads it. A soft
					// break at either end of the run says nothing at all.
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
				match bracket(src, i + 1) {
					Some((alt, to, end))	=> {
						flush(&mut out, &mut buf);
						// An image stands for itself in words, so its alt is flattened.
						let alt = text_of(&res!(run(&alt, depth + 1)));
						out.push(Node::Done(Inline::Image { src: to, alt }, 1));
						i = end;
					}
					None			=> {
						buf.push('!');
						i += 1;
					}
				}
			}
			b'['	=> {
				match bracket(src, i) {
					Some((txt, to, end))	=> {
						flush(&mut out, &mut buf);
						out.push(Node::Done(Inline::Link {
							to,
							content:	res!(run(&txt, depth + 1)),
						}, 1));
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
				let (open, close) = flank(src, i, i + len, ch);
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
/// why a stray asterisk is an asterisk.
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
		// Two characters at each end make emphasis strong; one makes it ordinary.
		let n = match (&nodes[k], &nodes[i]) {
			(Node::Delim { len: a, .. }, Node::Delim { len: b, .. })
				if *a >= 2 && *b >= 2	=> 2,
			_				=> 1,
		};
		// Everything between the two runs is what they emphasise.
		let inner: Vec<Node> = nodes.drain(k + 1..i).collect();
		let (content, cd) = res!(resolve(inner, depth + 1));
		let d = cd + 1;
		if depth + d > DEPTH_LIMIT {
			return Err(err!(
				"Markdown emphasis nests more than {} deep, which no prose written to be \
				read does.", DEPTH_LIMIT;
				Excessive, Input));
		}
		if let Node::Delim { len, .. } = &mut nodes[k] {
			*len -= n;
		}
		if let Node::Delim { len, .. } = &mut nodes[k + 1] {
			*len -= n;
		}
		nodes.insert(k + 1, Node::Done(Inline::Emph { strong: n == 2, content }, d));
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

/// Whether a run of emphasis characters may open, and may close, by what sits either side of it.
fn flank(src: &str, start: usize, end: usize, ch: u8) -> (bool, bool) {
	let prev = src[..start].chars().next_back();
	let next = src[end..].chars().next();
	let pre_ws = match prev { Some(c) => c.is_whitespace(), None => true };
	let post_ws = match next { Some(c) => c.is_whitespace(), None => true };
	let pre_pn = match prev { Some(c) => is_punct(c), None => false };
	let post_pn = match next { Some(c) => is_punct(c), None => false };
	// A run leans left or right by which side has text against it: `*a` leans right onto the `a`,
	// `a*` leans left onto it, and `a * b` leans nowhere and so is an asterisk.
	let left = !post_ws && (!post_pn || pre_ws || pre_pn);
	let right = !pre_ws && (!pre_pn || post_ws || post_pn);
	if ch == b'_' {
		// An underscore within a word is part of the word, so that an identifier survives.
		(left && (!right || pre_pn), right && (!left || post_pn))
	} else {
		(left, right)
	}
}

/// Whether the character counts as punctuation in judging a run of emphasis characters.
///
/// Beyond ASCII this is an approximation of Unicode's punctuation and symbol categories: what is
/// neither a letter, a digit, a space nor a control is taken to be punctuation.
fn is_punct(c: char) -> bool {
	c.is_ascii_punctuation() || (!c.is_alphanumeric() && !c.is_whitespace() && !c.is_control())
}

// ── Code spans, links and images ─────────────────────────────────

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

/// A bracketed link at the offset: its text, its destination, and the offset just past it.
fn bracket(src: &str, i: usize) -> Option<(String, String, usize)> {
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
				// A bracket within a code span is code, not a bracket.
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
					break;
				}
			}
			_	=> {}
		}
		j += 1;
	}
	if j >= b.len() || d != 0 || j + 1 >= b.len() || b[j + 1] != b'(' {
		return None;	// Without a destination against it, a bracket is a bracket.
	}
	match dest(src, j + 1) {
		Some((to, end))	=> Some((src[i + 1..j].to_string(), to, end)),
		None		=> None,
	}
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

	use oxedyne_fe2o3_core::prelude::*;

	/// A run of literal text, for the tests that expect one.
	fn t(s: &str) -> Inline {
		Inline::Text(s.to_string())
	}

	/// Plain prose is one run of text and not a run for every character.
	#[test]
	fn test_plain_prose_is_one_run_of_text_00() -> Outcome<()> {
		assert_eq!(res!(parse("Just some prose.")), vec![t("Just some prose.")]);
		Ok(())
	}

	/// One asterisk or underscore either side makes ordinary emphasis.
	#[test]
	fn test_one_character_either_side_emphasises_01() -> Outcome<()> {
		let want = vec![t("a "), Inline::Emph { strong: false, content: vec![t("b")] }, t(" c")];
		assert_eq!(res!(parse("a *b* c")), want);
		assert_eq!(res!(parse("a _b_ c")), want);
		Ok(())
	}

	/// Two characters either side make strong emphasis.
	#[test]
	fn test_two_characters_either_side_emphasise_strongly_02() -> Outcome<()> {
		let want = vec![t("a "), Inline::Emph { strong: true, content: vec![t("b")] }, t(" c")];
		assert_eq!(res!(parse("a **b** c")), want);
		assert_eq!(res!(parse("a __b__ c")), want);
		Ok(())
	}

	/// Emphasis nests within emphasis.
	#[test]
	fn test_emphasis_nests_03() -> Outcome<()> {
		assert_eq!(res!(parse("*a **b** c*")), vec![
			Inline::Emph {
				strong:		false,
				content:	vec![
					t("a "),
					Inline::Emph { strong: true, content: vec![t("b")] },
					t(" c"),
				],
			},
		]);
		// Three characters either side are both at once.
		assert_eq!(res!(parse("***a***")), vec![
			Inline::Emph {
				strong:		false,
				content:	vec![Inline::Emph { strong: true, content: vec![t("a")] }],
			},
		]);
		Ok(())
	}

	/// An asterisk with space around it is arithmetic, not emphasis.
	#[test]
	fn test_an_asterisk_alone_is_an_asterisk_04() -> Outcome<()> {
		assert_eq!(res!(parse("2 * 3 * 4")), vec![t("2 * 3 * 4")]);
		assert_eq!(res!(parse("a * b")), vec![t("a * b")]);
		Ok(())
	}

	/// An asterisk that opens nothing that closes is an asterisk.
	#[test]
	fn test_an_unmatched_delimiter_is_text_05() -> Outcome<()> {
		assert_eq!(res!(parse("*not emphasis")), vec![t("*not emphasis")]);
		assert_eq!(res!(parse("closing only*")), vec![t("closing only*")]);
		// The nearest opener wins, and the one left over is text.
		assert_eq!(res!(parse("*a *b*")), vec![
			t("*a "),
			Inline::Emph { strong: false, content: vec![t("b")] },
		]);
		Ok(())
	}

	/// An underscore within a word is part of the word, so an identifier survives.
	#[test]
	fn test_an_underscore_within_a_word_is_a_word_06() -> Outcome<()> {
		assert_eq!(res!(parse("snake_case_name")), vec![t("snake_case_name")]);
		// An asterisk within a word still emphasises, as Markdown has it.
		assert_eq!(res!(parse("a*b*c")), vec![
			t("a"),
			Inline::Emph { strong: false, content: vec![t("b")] },
			t("c"),
		]);
		Ok(())
	}

	/// Backticks make a code span, and what is in it is exactly what was written.
	#[test]
	fn test_backticks_make_a_code_span_07() -> Outcome<()> {
		assert_eq!(res!(parse("a `let x = *y*;` b")), vec![
			t("a "),
			Inline::Code("let x = *y*;".to_string()),
			t(" b"),
		]);
		Ok(())
	}

	/// A longer run of backticks lets a span hold a backtick of its own.
	#[test]
	fn test_a_longer_run_of_backticks_holds_one_08() -> Outcome<()> {
		assert_eq!(res!(parse("`` a ` b ``")), vec![Inline::Code("a ` b".to_string())]);
		// A run nothing closes is only backticks.
		assert_eq!(res!(parse("`` unclosed")), vec![t("`` unclosed")]);
		Ok(())
	}

	/// A bracket against a destination is a link.
	#[test]
	fn test_a_bracket_against_a_destination_is_a_link_09() -> Outcome<()> {
		assert_eq!(res!(parse("[text](https://a.b)")), vec![
			Inline::Link {
				to:		"https://a.b".to_string(),
				content:	vec![t("text")],
			},
		]);
		// A link's text is prose in its own right.
		assert_eq!(res!(parse("[a *b*](c)")), vec![
			Inline::Link {
				to:		"c".to_string(),
				content:	vec![t("a "), Inline::Emph { strong: false, content: vec![t("b")] }],
			},
		]);
		Ok(())
	}

	/// A destination may be angled, may be nothing, and may be followed by a title nobody keeps.
	#[test]
	fn test_a_destination_takes_several_forms_10() -> Outcome<()> {
		assert_eq!(res!(parse("[a](<b c>)")), vec![
			Inline::Link { to: "b c".to_string(), content: vec![t("a")] },
		]);
		assert_eq!(res!(parse("[a]()")), vec![
			Inline::Link { to: String::new(), content: vec![t("a")] },
		]);
		assert_eq!(res!(parse("[a](b \"a title\")")), vec![
			Inline::Link { to: "b".to_string(), content: vec![t("a")] },
		]);
		Ok(())
	}

	/// A bracket with no destination against it is a bracket.
	#[test]
	fn test_a_bracket_with_no_destination_is_text_11() -> Outcome<()> {
		assert_eq!(res!(parse("[just brackets]")), vec![t("[just brackets]")]);
		assert_eq!(res!(parse("an [unclosed bracket")), vec![t("an [unclosed bracket")]);
		assert_eq!(res!(parse("a ] alone")), vec![t("a ] alone")]);
		Ok(())
	}

	/// A bang before a link makes an image, whose alt is its text in words.
	#[test]
	fn test_a_bang_before_a_link_makes_an_image_12() -> Outcome<()> {
		assert_eq!(res!(parse("![a picture](p.png)")), vec![
			Inline::Image { src: "p.png".to_string(), alt: "a picture".to_string() },
		]);
		// The alt is flattened, so emphasis within it does not lose its words.
		assert_eq!(res!(parse("![a *loud* picture](p.png)")), vec![
			Inline::Image { src: "p.png".to_string(), alt: "a loud picture".to_string() },
		]);
		// A bang before anything else is a bang.
		assert_eq!(res!(parse("Look! [here](x)")), vec![
			t("Look! "),
			Inline::Link { to: "x".to_string(), content: vec![t("here")] },
		]);
		Ok(())
	}

	/// A URI in angle brackets is a link to itself.
	#[test]
	fn test_a_uri_in_angle_brackets_is_a_link_13() -> Outcome<()> {
		assert_eq!(res!(parse("See <https://a.b/c>.")), vec![
			t("See "),
			Inline::Link {
				to:		"https://a.b/c".to_string(),
				content:	vec![t("https://a.b/c")],
			},
			t("."),
		]);
		// Without a scheme, angle brackets are angle brackets.
		assert_eq!(res!(parse("a <b> c")), vec![t("a <b> c")]);
		assert_eq!(res!(parse("1 < 2")), vec![t("1 < 2")]);
		Ok(())
	}

	/// Two spaces before a line ending are a break the author asked for.
	#[test]
	fn test_trailing_spaces_make_a_break_14() -> Outcome<()> {
		assert_eq!(res!(parse("one  \ntwo")), vec![t("one"), Inline::Break, t("two")]);
		// Three or more do as well.
		assert_eq!(res!(parse("one   \ntwo")), vec![t("one"), Inline::Break, t("two")]);
		Ok(())
	}

	/// A line ending the author did not ask for is a space, so that prose reflows.
	///
	/// Prose is hard wrapped to whatever width its author was writing at. That width is an artefact
	/// of their editor and means nothing, so a soft break says a space and the reader's window
	/// decides where the lines fall.
	#[test]
	fn test_a_soft_line_ending_is_a_space_23() -> Outcome<()> {
		assert_eq!(res!(parse("one\ntwo")), vec![t("one two")]);
		// A single trailing space is not two, so it is soft, and says one space and not two.
		assert_eq!(res!(parse("one \ntwo")), vec![t("one two")]);
		// A paragraph wrapped across three lines is one run that reflows.
		assert_eq!(res!(parse("A paragraph that the author\nhard wrapped at a narrow width\nacross three lines.")),
			vec![t("A paragraph that the author hard wrapped at a narrow width across three lines.")]);
		Ok(())
	}

	/// A soft break at either end of a run says nothing, and leaves no space behind.
	#[test]
	fn test_a_soft_line_ending_at_an_end_says_nothing_24() -> Outcome<()> {
		assert_eq!(res!(parse("one\n")), vec![t("one")]);
		assert_eq!(res!(parse("\none")), vec![t("one")]);
		assert_eq!(res!(parse("one \n")), vec![t("one")]);
		assert_eq!(res!(parse("\n")), Vec::<Inline>::new());
		Ok(())
	}

	/// A soft break beside an inline is still a space, and joins the text around it.
	#[test]
	fn test_a_soft_line_ending_beside_an_inline_25() -> Outcome<()> {
		assert_eq!(res!(parse("*a*\nb")), vec![
			Inline::Emph { strong: false, content: vec![t("a")] },
			t(" b"),
		]);
		assert_eq!(res!(parse("a\n`b`")), vec![t("a "), Inline::Code("b".to_string())]);
		Ok(())
	}

	/// A backslash before a line ending is a break the author asked for.
	#[test]
	fn test_a_backslash_before_a_line_ending_breaks_15() -> Outcome<()> {
		assert_eq!(res!(parse("one\\\ntwo")), vec![t("one"), Inline::Break, t("two")]);
		Ok(())
	}

	/// A backslash before punctuation says the punctuation is only itself.
	#[test]
	fn test_a_backslash_escapes_punctuation_16() -> Outcome<()> {
		assert_eq!(res!(parse("\\*not emphasis\\*")), vec![t("*not emphasis*")]);
		assert_eq!(res!(parse("\\[not a link\\]")), vec![t("[not a link]")]);
		assert_eq!(res!(parse("a \\\\ backslash")), vec![t("a \\ backslash")]);
		// Before anything else, a backslash is a backslash.
		assert_eq!(res!(parse("\\a")), vec![t("\\a")]);
		Ok(())
	}

	/// Text either side of an inline joins into one run, and is never split per character.
	#[test]
	fn test_text_around_an_inline_coalesces_17() -> Outcome<()> {
		// The delimiters that came to nothing rejoin the text around them.
		match &res!(parse("a * b _ c"))[..] {
			[Inline::Text(s)]	=> assert_eq!(s, "a * b _ c"),
			other			=> panic!("expected one run of text, got {:?}", other),
		}
		// Text on both sides of emphasis is one run each side.
		let got = res!(parse("one *two* three four"));
		assert_eq!(got.len(), 3);
		assert_eq!(got[2], t(" three four"));
		Ok(())
	}

	/// Punctuation and Unicode either side of a delimiter do not stop emphasis.
	#[test]
	fn test_emphasis_survives_punctuation_around_it_18() -> Outcome<()> {
		assert_eq!(res!(parse("(*a*)")), vec![
			t("("),
			Inline::Emph { strong: false, content: vec![t("a")] },
			t(")"),
		]);
		assert_eq!(res!(parse("*naïve*")), vec![
			Inline::Emph { strong: false, content: vec![t("naïve")] },
		]);
		Ok(())
	}

	/// A code span outranks the emphasis and brackets that appear to be within it.
	#[test]
	fn test_a_code_span_outranks_what_is_in_it_19() -> Outcome<()> {
		assert_eq!(res!(parse("`[a](b)`")), vec![Inline::Code("[a](b)".to_string())]);
		// And a code span within a link's text is code.
		assert_eq!(res!(parse("[`a]`](b)")), vec![
			Inline::Link { to: "b".to_string(), content: vec![Inline::Code("a]".to_string())] },
		]);
		Ok(())
	}

	/// Nothing at all is a run of nothing.
	#[test]
	fn test_nothing_is_a_run_of_nothing_20() -> Outcome<()> {
		assert_eq!(res!(parse("")), Vec::<Inline>::new());
		Ok(())
	}

	/// Emphasis nested past the limit is refused, as deep block nesting is.
	///
	/// A long run of emphasis characters nests a level for every pair it spends, and spends two a
	/// level when it makes strong emphasis. So the limit is reached at twice its own count.
	#[test]
	fn test_nesting_past_the_limit_is_refused_21() -> Outcome<()> {
		// Nesting the limit allows is read.
		let ok = format!("{}a{}", "*".repeat(2 * (DEPTH_LIMIT - 1)), "*".repeat(2 * (DEPTH_LIMIT - 1)));
		assert!(parse(&ok).is_ok());
		// Past it is not, and a run built to exhaust the stack of whatever reads the tree is
		// refused rather than built.
		for n in [2 * DEPTH_LIMIT, 2 * DEPTH_LIMIT + 8, 100_000] {
			let src = format!("{}a{}", "*".repeat(n), "*".repeat(n));
			assert!(parse(&src).is_err(), "for a run of {}", n);
		}
		// Links nested past the limit are refused likewise.
		let mut deep = "x".to_string();
		for _ in 0..DEPTH_LIMIT + 4 {
			deep = format!("[{}](y)", deep);
		}
		assert!(parse(&deep).is_err());
		Ok(())
	}

	/// A run of characters that emphasises across a line ending still does.
	#[test]
	fn test_emphasis_crosses_a_line_ending_22() -> Outcome<()> {
		assert_eq!(res!(parse("*a\nb*")), vec![
			Inline::Emph { strong: false, content: vec![t("a b")] },
		]);
		Ok(())
	}
}
