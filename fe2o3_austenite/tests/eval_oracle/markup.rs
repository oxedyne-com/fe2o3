// A tag tokenizer for the three markup outputs the oracle is read through: Typst's HTML export, its
// SVG pages and `pdftotext -bbox-layout`. None of them needs a DOM, only the tag stream in order,
// so this is a scanner, not a parser; it does not validate nesting.

#[derive(Clone, Debug)]
pub enum Tok {
	Open { name: String, attrs: Vec<(String, String)>, closed: bool },	// `closed` for `<x/>`
	Close(String),
	Text(String),
}

impl Tok {
	pub fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
		attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
	}
}

/// Elements whose text is not document text.
const RAW_TEXT: [&str; 2] = ["style", "script"];

pub fn tokens(src: &str) -> Vec<Tok> {
	let b = src.as_bytes();
	let mut out = Vec::new();
	let mut i = 0;
	let mut text_start = 0;
	while i < b.len() {
		if b[i] != b'<' {
			i += 1;
			continue;
		}
		if i > text_start {
			out.push(Tok::Text(decode(&src[text_start..i])));
		}
		// Comments, doctype and processing instructions carry nothing.
		if src[i..].starts_with("<!--") {
			i = match src[i..].find("-->") {
				Some(k)	=> i + k + 3,
				None	=> b.len(),
			};
			text_start = i;
			continue;
		}
		if src[i..].starts_with("<!") || src[i..].starts_with("<?") {
			i = match src[i..].find('>') {
				Some(k)	=> i + k + 1,
				None	=> b.len(),
			};
			text_start = i;
			continue;
		}
		let end = match tag_end(b, i + 1) {
			Some(e)	=> e,
			None	=> b.len() - 1,
		};
		let inner = &src[i + 1..end];
		i = end + 1;
		text_start = i;
		if let Some(name) = inner.strip_prefix('/') {
			out.push(Tok::Close(name.trim().to_ascii_lowercase()));
			continue;
		}
		let closed = inner.ends_with('/');
		let inner = inner.trim_end_matches('/');
		let (name, attrs) = split_tag(inner);
		let raw = RAW_TEXT.contains(&name.as_str()) && !closed;
		out.push(Tok::Open { name: name.clone(), attrs, closed });
		if raw {
			let close = fmt_close(&name);
			let k = match src[i..].find(&close) {
				Some(k)	=> i + k,
				None	=> b.len(),
			};
			i = k;
			text_start = k;
		}
	}
	if text_start < b.len() {
		out.push(Tok::Text(decode(&src[text_start..])));
	}
	out
}

fn fmt_close(name: &str) -> String {
	let mut s = String::from("</");
	s.push_str(name);
	s
}

// The `>` that ends a tag opened just before `from`, skipping quoted attribute values.
fn tag_end(b: &[u8], from: usize) -> Option<usize> {
	let mut q: Option<u8> = None;
	let mut i = from;
	while i < b.len() {
		let c = b[i];
		match q {
			Some(qc) if c == qc	=> q = None,
			Some(_)				=> (),
			None if c == b'"' || c == b'\''	=> q = Some(c),
			None if c == b'>'	=> return Some(i),
			None				=> (),
		}
		i += 1;
	}
	None
}

fn split_tag(inner: &str) -> (String, Vec<(String, String)>) {
	let inner = inner.trim();
	let n = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
	let name = inner[..n].to_ascii_lowercase();
	let mut attrs = Vec::new();
	let rest = inner[n..].as_bytes();
	let s = &inner[n..];
	let mut i = 0;
	while i < rest.len() {
		while i < rest.len() && rest[i].is_ascii_whitespace() {
			i += 1;
		}
		let k0 = i;
		while i < rest.len() && rest[i] != b'=' && !rest[i].is_ascii_whitespace() {
			i += 1;
		}
		let key = s[k0..i].to_ascii_lowercase();
		if key.is_empty() {
			i += 1;
			continue;
		}
		while i < rest.len() && rest[i].is_ascii_whitespace() {
			i += 1;
		}
		if i < rest.len() && rest[i] == b'=' {
			i += 1;
			while i < rest.len() && rest[i].is_ascii_whitespace() {
				i += 1;
			}
			if i < rest.len() && (rest[i] == b'"' || rest[i] == b'\'') {
				let q = rest[i];
				let v0 = i + 1;
				i = v0;
				while i < rest.len() && rest[i] != q {
					i += 1;
				}
				attrs.push((key, decode(&s[v0..i.min(rest.len())])));
				i += 1;
			} else {
				let v0 = i;
				while i < rest.len() && !rest[i].is_ascii_whitespace() {
					i += 1;
				}
				attrs.push((key, decode(&s[v0..i])));
			}
		} else {
			attrs.push((key, String::new()));
		}
	}
	(name, attrs)
}

/// Decodes the character references HTML, SVG and poppler's XML emit.
pub fn decode(s: &str) -> String {
	if !s.contains('&') {
		return s.to_string();
	}
	let mut out = String::with_capacity(s.len());
	let mut rest = s;
	while let Some(k) = rest.find('&') {
		out.push_str(&rest[..k]);
		let after = &rest[k + 1..];
		let semi = match after.find(';') {
			Some(p) if p <= 10	=> p,
			_ => {
				out.push('&');
				rest = after;
				continue;
			}
		};
		let ent = &after[..semi];
		let ch = match ent {
			"amp"	=> Some('&'),
			"lt"	=> Some('<'),
			"gt"	=> Some('>'),
			"quot"	=> Some('"'),
			"apos"	=> Some('\''),
			"nbsp"	=> Some('\u{a0}'),
			_ if ent.starts_with("#x") || ent.starts_with("#X")	=>
				u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
			_ if ent.starts_with('#')	=> ent[1..].parse::<u32>().ok().and_then(char::from_u32),
			_						=> None,
		};
		match ch {
			Some(c)	=> {
				out.push(c);
				rest = &after[semi + 1..];
			}
			None	=> {
				out.push('&');
				rest = after;
			}
		}
	}
	out.push_str(rest);
	out
}
