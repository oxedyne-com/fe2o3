// Level 3: realised structure, compared as a skeleton both engines can be brought to.
//
// The oracle is `typst compile --features html --format html`, which realises the document with its
// show rules and writes the model elements as HTML. The skeleton keeps the structural tags and the
// text, and drops what is presentation (`div`, `span`, attributes, the style sheet). Three
// normalisations keep it to structure: whitespace collapses; smart quotes read as plain ones (the
// glyph a quote resolves to is level 4's business); and footnotes are left to levels 2 and 4,
// because the HTML export moves them into an endnote section a paged document does not have.
//
// Typst's HTML export ignores much of the layout library (`align`, `pad`, `grid`, `place`, `h`,
// `v`, `stack`, `columns`, shapes), dropping the element and its content with a warning. A fixture
// whose export warns that way has no level-3 oracle; the harness reports it as not applicable
// rather than comparing against a document Typst itself has mutilated.

use crate::harness::markup::{
	tokens,
	Tok,
};

use oxedyne_fe2o3_core::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sk {
	Tag(String, Vec<Sk>),
	Text(String),
}

pub const TAGS: &[&str] = &[
	"h1", "h2", "h3", "h4", "h5", "h6", "p", "ul", "ol", "li", "dl", "dt", "dd", "figure", "figcaption",
	"table", "td", "th", "blockquote", "pre", "code", "strong", "em", "a", "sup", "sub", "mark", "math",
	"img", "br",
];

const BLOCK: &[&str] = &[
	"h1", "h2", "h3", "h4", "h5", "h6", "p", "ul", "ol", "li", "dl", "dt", "dd", "figure", "figcaption",
	"table", "td", "th", "blockquote", "pre",
];

/// Ignored elements that carry no content, so the export that drops them still has the document's
/// whole structure: page setup, breaks and spacing.
pub const HARMLESS: &[&str] = &["page set rule", "pagebreak", "colbreak", "h", "v"];

const VOID: &[&str] = &["br", "img", "hr", "meta", "link", "input", "col", "wbr"];

/// The body of a Typst HTML export as a skeleton.
pub fn from_html(html: &str) -> Vec<Sk> {
	// Frames: (tag kept, children). A dropped subtree is counted, not framed.
	let mut stack: Vec<(Option<String>, Vec<Sk>)> = vec![(None, Vec::new())];
	let mut in_body = false;
	let mut drop_depth = 0usize;
	let mut open_names: Vec<String> = Vec::new();
	for t in tokens(html) {
		match t {
			Tok::Open { name, attrs, closed } => {
				if name == "body" {
					in_body = true;
					continue;
				}
				if !in_body {
					continue;
				}
				let void = closed || VOID.contains(&name.as_str());
				let role = Tok::attr(&attrs, "role").unwrap_or("");
				let dropped = role == "doc-noteref" || role == "doc-endnotes" || role == "doc-backlink";
				if drop_depth > 0 || dropped {
					if !void {
						drop_depth += 1;
						open_names.push(name);
					}
					continue;
				}
				if void {
					if TAGS.contains(&name.as_str()) {
						if let Some(top) = stack.last_mut() {
							top.1.push(Sk::Tag(name, Vec::new()));
						}
					}
					continue;
				}
				if name == "math" {
					// MathML is level 4's business; the equation is one leaf.
					drop_depth += 1;
					open_names.push(name.clone());
					if let Some(top) = stack.last_mut() {
						top.1.push(Sk::Tag(name, Vec::new()));
					}
					continue;
				}
				open_names.push(name.clone());
				let keep = TAGS.contains(&name.as_str());
				stack.push((if keep { Some(name) } else { None }, Vec::new()));
			}
			Tok::Close(name) => {
				if name == "body" {
					in_body = false;
					continue;
				}
				if !in_body {
					continue;
				}
				// Pop to the matching open tag; HTML here is well formed, so this is one pop.
				let pos = open_names.iter().rposition(|n| *n == name);
				let pos = match pos {
					Some(p)	=> p,
					None	=> continue,
				};
				while open_names.len() > pos {
					open_names.pop();
					if drop_depth > 0 {
						drop_depth -= 1;
						continue;
					}
					if stack.len() > 1 {
						if let Some((tag, kids)) = stack.pop() {
							if let Some(top) = stack.last_mut() {
								match tag {
									Some(t)	=> top.1.push(Sk::Tag(t, kids)),
									None	=> top.1.extend(kids),
								}
							}
						}
					}
				}
			}
			Tok::Text(s) => {
				if in_body && drop_depth == 0 {
					if let Some(top) = stack.last_mut() {
						top.1.push(Sk::Text(s));
					}
				}
			}
		}
	}
	while stack.len() > 1 {
		if let Some((tag, kids)) = stack.pop() {
			if let Some(top) = stack.last_mut() {
				match tag {
					Some(t)	=> top.1.push(Sk::Tag(t, kids)),
					None	=> top.1.extend(kids),
				}
			}
		}
	}
	normalise(stack.pop().map(|f| f.1).unwrap_or_default())
}

/// The elements Typst's HTML export reports it ignored, from its warnings.
pub fn ignored_elements(stderr: &str) -> Vec<String> {
	let mut out: Vec<String> = Vec::new();
	for line in stderr.lines() {
		if let Some(rest) = line.trim().strip_prefix("warning: ") {
			if let Some(name) = rest.strip_suffix(" was ignored during HTML export") {
				if !out.iter().any(|n| n == name) {
					out.push(name.to_string());
				}
			}
		}
	}
	out
}

fn quotes(s: &str) -> String {
	s.chars().map(|c| match c {
		'\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{00ab}' | '\u{00bb}'	=> '"',
		'\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{2039}' | '\u{203a}'	=> '\'',
		'\u{a0}'													=> ' ',
		c															=> c,
	}).collect()
}

/// Merges adjacent text, collapses whitespace, trims it at block edges and drops empty blocks'
/// paragraphs, so two skeletons differ only where the structure does.
pub fn normalise(kids: Vec<Sk>) -> Vec<Sk> {
	let mut out: Vec<Sk> = Vec::new();
	for k in kids {
		match k {
			Sk::Text(t)	=> {
				let t = quotes(&t);
				let mut c = String::new();
				let mut ws = false;
				for ch in t.chars() {
					if ch.is_whitespace() {
						ws = true;
					} else {
						if ws {
							c.push(' ');
						}
						ws = false;
						c.push(ch);
					}
				}
				if ws {
					c.push(' ');
				}
				if c.is_empty() {
					continue;
				}
				match out.last_mut() {
					Some(Sk::Text(prev))	=> {
						if prev.ends_with(' ') && c.starts_with(' ') {
							c.remove(0);
						}
						prev.push_str(&c);
					}
					_						=> out.push(Sk::Text(c)),
				}
			}
			Sk::Tag(name, inner) => {
				let mut inner = normalise(inner);
				if BLOCK.contains(&name.as_str()) {
					trim_edges(&mut inner);
					if name == "p" && inner.is_empty() {
						continue;
					}
					// A block ends the text before it.
					if let Some(Sk::Text(prev)) = out.last_mut() {
						let t = prev.trim_end().to_string();
						*prev = t;
					}
					if matches!(out.last(), Some(Sk::Text(p)) if p.is_empty()) {
						out.pop();
					}
				}
				out.push(Sk::Tag(name, inner));
			}
		}
	}
	// Text after a block starts afresh.
	let mut i = 1;
	while i < out.len() {
		let after_block = matches!(&out[i - 1], Sk::Tag(n, _) if BLOCK.contains(&n.as_str()));
		if after_block {
			if let Sk::Text(t) = &mut out[i] {
				let s = t.trim_start().to_string();
				*t = s;
			}
		}
		i += 1;
	}
	out.retain(|k| !matches!(k, Sk::Text(t) if t.is_empty()));
	trim_edges_if_blocky(&mut out);
	out
}

fn trim_edges(v: &mut Vec<Sk>) {
	if let Some(Sk::Text(t)) = v.first_mut() {
		let s = t.trim_start().to_string();
		*t = s;
	}
	if let Some(Sk::Text(t)) = v.last_mut() {
		let s = t.trim_end().to_string();
		*t = s;
	}
	v.retain(|k| !matches!(k, Sk::Text(t) if t.is_empty()));
}

fn trim_edges_if_blocky(v: &mut Vec<Sk>) {
	if v.iter().any(|k| matches!(k, Sk::Tag(n, _) if BLOCK.contains(&n.as_str()))) {
		trim_edges(v);
	}
}

pub fn render(v: &[Sk]) -> String {
	let mut s = String::new();
	for (i, k) in v.iter().enumerate() {
		if i > 0 {
			s.push(' ');
		}
		match k {
			Sk::Text(t)			=> s.push_str(&fmt!("{:?}", t)),
			Sk::Tag(n, inner)	=> {
				s.push_str(n);
				if !inner.is_empty() {
					s.push('[');
					s.push_str(&render(inner));
					s.push(']');
				}
			}
		}
	}
	s
}

/// Level-3 differences: the first place the skeletons part, with a little context either side.
pub fn compare(want: &[Sk], got: &[Sk], out: &mut Vec<String>) {
	let a = render(want);
	let b = render(got);
	if a == b {
		return;
	}
	let ac: Vec<char> = a.chars().collect();
	let bc: Vec<char> = b.chars().collect();
	let mut i = 0;
	while i < ac.len() && i < bc.len() && ac[i] == bc[i] {
		i += 1;
	}
	let from = i.saturating_sub(40);
	let ta: String = ac[from..(i + 60).min(ac.len())].iter().collect();
	let tb: String = bc[from..(i + 60).min(bc.len())].iter().collect();
	out.push(fmt!("structure parts at character {}: typst ...{}... austenite ...{}...", i, ta, tb));
}
