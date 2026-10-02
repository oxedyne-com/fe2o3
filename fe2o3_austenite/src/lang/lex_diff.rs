// LX: the differential between the gate's hand-written classifier (`lex_old`) and the view over the syntax
// tree (`lex`). Test-only, deleted with `lex_old` once every disagreement is settled.
//
// [bio]: a root under `~/usr/books/` and every file it includes is read in this process and never shown. For
// those the report holds counts per root and per kind pair, and nothing else: no name, no position, no text.
#![allow(dead_code)]

use super::lex;
use super::lex_old;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

/// One group of files, and whether what it holds may be shown.
struct Group {
	name:	String,
	files:	Vec<PathBuf>,
	show:	bool,
}

/// What the two classifiers disagree on within one group.
#[derive(Default)]
struct Tally {
	files:	usize,
	bytes:	usize,
	counts:	BTreeMap<(String, String, String), usize>,	// (metric, old, new) -> how many
	notes:	Vec<String>,								// shown groups only
	quiet:	usize,										// notes dropped
	seen:	BTreeMap<(String, String), usize>,			// notes kept, by file and kind
}

impl Tally {
	fn count(&mut self, metric: &str, old: String, new: String) {
		self.add(metric, old, new, 1);
	}

	fn add(&mut self, metric: &str, old: String, new: String, n: usize) {
		*self.counts.entry((metric.to_string(), old, new)).or_insert(0) += n;
	}

	fn note(&mut self, show: bool, line: String) {
		if !show {
			return;
		}
		// Six of a kind in a file are enough to see what it is.
		let key = line.splitn(3, ':').take(2).collect::<Vec<_>>().join(":");
		let kind = line.split(": ").nth(1).unwrap_or("").split(' ').take(3).collect::<Vec<_>>().join(" ");
		let seen = self.seen.entry((key.rsplit_once(':').map_or(key.clone(), |(f, _)| f.to_string()), kind)).or_insert(0);
		*seen += 1;
		if *seen > 6 {
			self.quiet += 1;
			return;
		}
		if self.notes.len() < 600 {
			self.notes.push(line);
		} else {
			self.quiet += 1;
		}
	}
}

fn tok_name(t: lex::Tok) -> String {
	format!("{:?}", t)
}

fn old_name(t: lex_old::Tok) -> String {
	format!("{:?}", t)
}

fn place_name<T: std::fmt::Debug>(p: Option<T>) -> String {
	match p {
		Some(p)	=> format!("{:?}", p),
		None	=> "None".to_string(),
	}
}

/// The text around byte `at`, on one line, for a shown group.
fn around(src: &str, at: usize, to: usize) -> String {
	let mut a = at.saturating_sub(24);
	while !src.is_char_boundary(a) {
		a -= 1;
	}
	let mut b = (to + 24).min(src.len());
	while !src.is_char_boundary(b) {
		b += 1;
	}
	src[a..b].replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t")
}

fn esc(s: &str) -> String {
	s.replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t")
}

/// The run `at..to` marked in its surroundings.
fn ctx(src: &str, at: usize, to: usize) -> String {
	let mut a = at.saturating_sub(16);
	while !src.is_char_boundary(a) {
		a -= 1;
	}
	let mut b = (to + 16).min(src.len());
	while !src.is_char_boundary(b) {
		b += 1;
	}
	let mut m = (at + 24).min(to);
	while !src.is_char_boundary(m) {
		m += 1;
	}
	esc(&format!("{}<<{}>>{}", &src[a..at], &src[at..m], &src[to.min(b)..b]))
}

fn line_of(src: &str, at: usize) -> usize {
	src[..at.min(src.len())].bytes().filter(|&b| b == b'\n').count() + 1
}

type OldFlow = (u8, usize, usize, Vec<(Option<(usize, usize)>, (usize, usize), bool)>, bool, bool, bool);

fn old_flows(src: &str, level: lex_old::Level) -> Vec<OldFlow> {
	lex_old::flows_in(src, level).into_iter().map(|f| {
		let kw = match f.kw {
			lex_old::Kw::If		=> 0,
			lex_old::Kw::While	=> 1,
			lex_old::Kw::For	=> 2,
			lex_old::Kw::Done	=> 3,
		};
		(kw, f.start, f.end, f.arms.iter().map(|a| (a.cond, a.body, a.content)).collect(), f.whole, f.coded, f.math)
	}).collect()
}

fn new_flows(src: &str, level: lex::Level) -> Vec<OldFlow> {
	lex::flows_in(src, level).into_iter().map(|f| {
		let kw = match f.kw {
			lex::Kw::If		=> 0,
			lex::Kw::While	=> 1,
			lex::Kw::For	=> 2,
		};
		(kw, f.start, f.end, f.arms.iter().map(|a| (a.cond, a.body, a.content)).collect(), f.whole, f.coded, f.math)
	}).collect()
}

/// The old `split_top_args`, as `parse.rs` had it over the streaming lexer.
fn old_split(inner: &str) -> Vec<String> {
	let chars: Vec<char>		= inner.chars().collect();
	let mut args: Vec<String>	= Vec::new();
	let mut cur					= String::new();
	let mut state				= lex_old::Lexer::code();
	let mut i					= 0;
	while i < chars.len() {
		if !state.is_open() && chars[i] == ',' {
			args.push(std::mem::take(&mut cur));
			i += 1;
			continue;
		}
		let (consumed, _, kept) = state.arg_step(&chars, i);
		match kept {
			lex_old::Kept::Text		=> cur.extend(&chars[i..i + consumed]),
			lex_old::Kept::Space	=> cur.push(' '),
			lex_old::Kept::Nothing	=> {},
		}
		i += consumed;
	}
	if !cur.trim().is_empty() {
		args.push(cur);
	}
	args
}

/// The old `named_arg`.
fn old_named_arg(arg: &str) -> Option<(String, String)> {
	let chars: Vec<char>	= arg.chars().collect();
	let mut state			= lex_old::Lexer::code();
	let mut i				= 0;
	while i < chars.len() {
		if !state.is_open() && chars[i] == ':' {
			let key: String = chars[..i].iter().collect();
			let key = key.trim().to_string();
			if !key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
				let val: String = chars[i + 1..].iter().collect();
				return Some((key, val.trim().to_string()));
			}
			return None;
		}
		i += state.step(&chars, i).0;
	}
	None
}

/// The old `read_group`'s end, in bytes.
fn old_group(src: &str, at: usize) -> Option<usize> {
	lex_old::group_end(src, at)
}

fn compare(src: &str, label: &str, show: bool, t: &mut Tally) {
	t.files += 1;
	t.bytes += src.len();

	// 1. What each byte is.
	let a = lex_old::byte_toks(src);
	let b = lex::byte_toks(src);
	if a.len() != b.len() {
		t.count("bytes", format!("{}", a.len()), format!("{}", b.len()));
		return;
	}
	let mut i = 0;
	while i < a.len() {
		let (x, y) = (old_name(a[i]), tok_name(b[i]));
		if x == y {
			i += 1;
			continue;
		}
		let mut j = i;
		while j < a.len() && old_name(a[j]) == x && tok_name(b[j]) == y {
			j += 1;
		}
		// A space between an embedded statement and the comment or line end after it: Typst's tree puts it
		// beside the statement, not in it.
		let space = src[i..j].chars().all(|c| c == ' ' || c == '\t');
		let after = src[j..].trim_start_matches([' ', '\t']);
		let lead = i == 0 || (i > 0 && src[..i].ends_with(|c: char| !c.is_whitespace()));
		let seam = after.is_empty() || after.starts_with(['\n', '\r']) || after.starts_with("//") || after.starts_with("/*");
		let y = if x == "Code" && y == "Text" && space && lead && seam { "Text [spaces after a statement]".to_string() } else { y };
		t.add("tok", x.clone(), y.clone(), j - i);
		t.note(show, format!("{}:{}: tok {} -> {} x{} in: {}", label, line_of(src, i), x, y, j - i, ctx(src, i, j)));
		i = j;
	}

	// 2. Where each line stands, and the state the streaming scan keeps at each.
	let old_lines	= lex_old::placed_lines(src);
	let lx			= lex::Lexer::markup(src);
	let new_lines	= lx.lines(src);
	let mut old_state = lex_old::Lexer::markup_over(src);
	let mut at = 0usize;
	let mut k = 0usize;
	for raw in src.split_inclusive('\n') {
		let op = old_state.place_of(raw);
		let np = lx.place_at(at);
		if op.map(|p| format!("{:?}", p)) != np.map(|p| format!("{:?}", p)) {
			t.count("place", place_name(op), place_name(np));
			t.note(show, format!("{}:{}: place {} -> {}: {}", label, line_of(src, at), place_name(op), place_name(np), around(src, at, at + raw.len().min(40))));
		}
		let (od, nd) = (old_state.depth(), lx.depth_at(at));
		if od != nd {
			t.count("depth", format!("{}", od), format!("{}", nd));
			t.note(show, format!("{}:{}: depth {} -> {}: {}", label, line_of(src, at), od, nd, around(src, at, at + raw.len().min(40))));
		}
		let (ol, nl) = (old_state.markup_level(), lx.markup_level_at(at));
		if ol != nl {
			t.count("level", format!("{:?}", ol), format!("{:?}", nl));
			t.note(show, format!("{}:{}: level {:?} -> {:?}: {}", label, line_of(src, at), ol, nl, around(src, at, at + raw.len().min(40))));
		}
		let (ob, nb) = (old_state.bare_line(raw), lx.bare_line_at(at));
		if ob != nb {
			t.count("bare", format!("{}", ob), format!("{}", nb));
			t.note(show, format!("{}:{}: bare {} -> {}: {}", label, line_of(src, at), ob, nb, around(src, at, at + raw.len().min(40))));
		}
		// A line read alone, as the readers that gather a construct line by line read it.
		let line = raw.strip_suffix('\n').unwrap_or(raw);
		let line = line.strip_suffix('\r').unwrap_or(line).trim_start();
		let mut fresh = lex_old::Lexer::markup();
		fresh.feed_line(line);
		if fresh.is_open() != lex::open_after(line) {
			t.count("line-open", format!("{}", fresh.is_open()), format!("{}", lex::open_after(line)));
			t.note(show, format!("{}:{}: line-open old {} new {}: {}", label, line_of(src, at), fresh.is_open(), lex::open_after(line), esc(line)));
		}
		old_state.feed(raw);
		at += raw.len();
		let (oo, no) = (old_state.is_open(), lx.open_at(at) > 0);
		// The last line has no break to end an embedded expression, which the streaming scan leaves open.
		if oo != no && raw.ends_with('\n') {
			t.count("open", format!("{}", oo), format!("{}", no));
			t.note(show, format!("{}:{}: open-after {} -> {}: {}", label, line_of(src, at.saturating_sub(1)), oo, no, around(src, at.saturating_sub(raw.len().min(40)), at)));
		}
		k += 1;
	}
	let _ = (old_lines.len(), new_lines.len(), k);

	// 3. Conditionals, loops and bindings.
	for (name, level_old, level_new) in [
		("flows-own",	lex_old::Level::Own,	lex::Level::Own),
		("flows-deep",	lex_old::Level::Deep,	lex::Level::Deep),
	] {
		let (fo, fn_) = (old_flows(src, level_old), new_flows(src, level_new));
		let starts: BTreeSet<usize> = fo.iter().chain(fn_.iter()).map(|f| f.1).collect();
		for s in starts {
			let (x, y) = (fo.iter().find(|f| f.1 == s), fn_.iter().find(|f| f.1 == s));
			match (x, y) {
				(Some(x), Some(y)) if x == y	=> {},
				(Some(x), Some(y))	=> {
					let which = [(x.0 != y.0, "kw"), (x.2 != y.2, "end"), (x.3 != y.3, "arms"), (x.4 != y.4, "whole"), (x.5 != y.5, "coded"), (x.6 != y.6, "math")]
						.iter().filter(|(d, _)| *d).map(|(_, n)| *n).collect::<Vec<_>>().join("+");
					t.count(name, "listed".to_string(), format!("differs: {}", which));
					t.note(show, format!("{}:{}: {} differs ({}): old {:?} new {:?}", label, line_of(src, s), name, which, x, y));
				},
				(Some(x), None)	=> {
					t.count(name, "listed".to_string(), "absent".to_string());
					t.note(show, format!("{}:{}: {} only old: {:?}", label, line_of(src, s), name, x));
				},
				(None, Some(y))	=> {
					t.count(name, "absent".to_string(), "listed".to_string());
					t.note(show, format!("{}:{}: {} only new: {:?}", label, line_of(src, s), name, y));
				},
				(None, None)	=> {},
			}
		}
	}
	let ob: Vec<_> = lex_old::bindings(src).into_iter().map(|b| (b.start, b.text, b.at, b.until)).collect();
	let nb: Vec<_> = lex::bindings(src).into_iter().map(|b| (b.start, b.text, b.at, b.until)).collect();
	let starts: BTreeSet<usize> = ob.iter().chain(nb.iter()).map(|b| b.0).collect();
	for s in starts {
		let (x, y) = (ob.iter().find(|b| b.0 == s), nb.iter().find(|b| b.0 == s));
		match (x, y) {
			(Some(x), Some(y)) if x == y	=> {},
			(Some(x), Some(y))	=> {
				// The statement's trailing space and comment are part of the old one's text and not the new's.
				let slack = x.1.trim_end() == y.1.trim_end() && x.3 == y.3 && x.2 >= y.2
					&& lex_old::uncommented(src)[y.2..x.2].trim().is_empty();
				let which = [(x.1 != y.1, "text"), (x.2 != y.2, "at"), (x.3 != y.3, "until")]
					.iter().filter(|(d, _)| *d).map(|(_, n)| *n).collect::<Vec<_>>().join("+");
				t.count("bindings", "listed".to_string(), format!("differs: {}{}", which, if slack { " (trailing space only)" } else { "" }));
				if !slack {
					t.note(show, format!("{}:{}: binding differs ({}): old {:?} new {:?}", label, line_of(src, s), which, x, y));
				}
			},
			(Some(x), None)	=> {
				t.count("bindings", "listed".to_string(), "absent".to_string());
				t.note(show, format!("{}:{}: binding only old: {:?}", label, line_of(src, s), x));
			},
			(None, Some(y))	=> {
				t.count("bindings", "absent".to_string(), "listed".to_string());
				t.note(show, format!("{}:{}: binding only new: {:?}", label, line_of(src, s), y));
			},
			(None, None)	=> {},
		}
	}
	if lex_old::lone_dollars(src) != lex::lone_dollars(src) {
		t.count("lone", "x".to_string(), "y".to_string());
		t.note(show, format!("{}: lone dollars differ: old {:?} new {:?}", label, lex_old::lone_dollars(src), lex::lone_dollars(src)));
	}

	// 4. Groups, as the code readers meet them: at each opener either classifier reads.
	for (at, c) in src.char_indices() {
		if !matches!(c, '(' | '[' | '{') || (a[at] != lex_old::Tok::Open && b[at] != lex::Tok::Open) {
			continue;
		}
		let (go, gn) = (old_group(src, at), lex::group_end(src, at));
		if go != gn {
			t.count("group_end", format!("{:?}", go.is_some()), format!("{:?}", gn.is_some()));
			t.note(show, format!("{}:{}: group_end at {} old {:?} new {:?}: {}", label, line_of(src, at), at, go, gn, around(src, at, at + 40)));
			continue;
		}
		let Some(end) = go else { continue; };
		if c != '(' {
			continue;
		}
		// The same group over chars, which is how the readers ask for it.
		let chars: Vec<char> = src[at..].chars().collect();
		let ce = lex::group_end_chars(&chars, 0).map(|k| at + chars[..k].iter().map(|c| c.len_utf8()).sum::<usize>());
		if ce != Some(end) {
			t.count("group_chars", "x".to_string(), "y".to_string());
			t.note(show, format!("{}:{}: group_end_chars {:?} against {:?}: {}", label, line_of(src, at), ce, end, around(src, at, at + 40)));
		}
		let inner = &src[at + 1..end - 1];
		for part in old_split(inner) {
			let old_named = old_named_arg(&part);
			let new_named = lex::top_colon(&part).and_then(|k| {
				let key = part[..k].trim();
				(!key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')).then(|| (key.to_string(), part[k + 1..].trim().to_string()))
			});
			if old_named != new_named {
				t.count("named_arg", "x".to_string(), "y".to_string());
				t.note(show, format!("{}:{}: named_arg {:?} against {:?}: {}", label, line_of(src, at), old_named, new_named, around(src, at, at + 40)));
			}
		}
		if old_split(inner) != lex::split_args(inner) {
			t.count("split_args", "x".to_string(), "y".to_string());
			t.note(show, format!("{}:{}: split_args differ: {}", label, line_of(src, at), around(src, at, at + 60)));
		}
		let (ao, an): (Vec<_>, Vec<_>) = (
			lex_old::args(inner).into_iter().map(|a| (a.key, a.value)).collect(),
			lex::args(inner).into_iter().map(|a| (a.key, a.value)).collect(),
		);
		if ao != an {
			t.count("args", "x".to_string(), "y".to_string());
			t.note(show, format!("{}:{}: args differ: {}", label, line_of(src, at), around(src, at, at + 60)));
		}
		if lex_old::top_comma(inner) != lex::top_comma(inner) {
			t.count("top_comma", "x".to_string(), "y".to_string());
			t.note(show, format!("{}:{}: top_comma differs: {}", label, line_of(src, at), around(src, at, at + 60)));
		}
		if lex_old::top_parens(inner) != lex::top_parens(inner) {
			t.count("top_parens", "x".to_string(), "y".to_string());
			t.note(show, format!("{}:{}: top_parens differs: {}", label, line_of(src, at), around(src, at, at + 60)));
		}
	}
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
	let Ok(rd) = fs::read_dir(dir) else { return; };
	let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
	entries.sort();
	for p in entries {
		if p.is_dir() {
			collect(&p, out);
		} else if p.extension().is_some_and(|e| e == "typ") {
			out.push(p);
		}
	}
}

/// The files a root reaches by `#include` and `#import` of a `.typ` path, itself first.
fn closure(root: &Path, tyroot: &Path) -> Vec<PathBuf> {
	let mut seen	= BTreeSet::new();
	let mut stack	= vec![root.to_path_buf()];
	while let Some(f) = stack.pop() {
		if !seen.insert(f.clone()) {
			continue;
		}
		let Ok(src) = fs::read_to_string(&f) else { continue; };
		let live = lex::live_text(&src);
		for kw in ["#include", "#import"] {
			let mut from = 0usize;
			while let Some(k) = live[from..].find(kw) {
				let at		= from + k + kw.len();
				from		= at;
				let rest	= live[at..].trim_start();
				let Some(rest) = rest.strip_prefix('"') else { continue; };
				let Some(q) = rest.find('"') else { continue; };
				let rel = &src[src.len() - rest.len()..][..q];
				if !rel.ends_with(".typ") {
					continue;
				}
				let path = match rel.strip_prefix('/') {
					Some(r)	=> tyroot.join(r),
					None	=> f.parent().unwrap_or(Path::new("")).join(rel),
				};
				if path.is_file() {
					stack.push(path);
				}
			}
		}
	}
	seen.into_iter().collect()
}

fn groups() -> Vec<Group> {
	let home = std::env::var("HOME").unwrap_or_default();
	let mut out = Vec::new();
	let mut tests = Vec::new();
	collect(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests"), &mut tests);
	out.push(Group { name: "tests".to_string(), files: tests, show: true });
	let mut pkgs = Vec::new();
	collect(&PathBuf::from(&home).join(".cache/typst/packages/preview"), &mut pkgs);
	out.push(Group { name: "packages".to_string(), files: pkgs, show: true });
	let oxeweb	= PathBuf::from(&home).join("usr/complement/projects/oxegen/oxeweb/doc");
	let books	= PathBuf::from(&home).join("usr/books/elearnity");
	let roots: [(&str, PathBuf, PathBuf, bool); 6] = [
		("oxeweb-overview",		oxeweb.join("Overview/overview.typ"),			oxeweb.join("Overview"),	false),
		("oxeweb-techspec",		oxeweb.join("TechSpec/techspec.typ"),			oxeweb.join("TechSpec"),	false),
		("cheapthinking-ch03",	books.join("CheapThinking/thinking_chap_03.typ"),	books.clone(),			true),
		("glossary-oracle",		books.join("CheapThinking/glossary_oracle.typ"),	books.clone(),			true),
		("index-oracle",		books.join("CheapThinking/index_oracle.typ"),		books.clone(),			true),
		("backmatter-oracle",	books.join("CheapThinking/backmatter_oracle.typ"),	books.clone(),			true),
	];
	for (name, root, tyroot, bio) in roots {
		out.push(Group { name: format!("{}{}", name, if bio { " [bio]" } else { "" }), files: closure(&root, &tyroot), show: !bio });
	}
	out
}

/// Prints where the two classifiers disagree on the text in the file `LX_PROBE_FILE`, run by run.
#[test]
#[ignore]
fn probe() {
	let path = std::env::var("LX_PROBE_FILE").unwrap_or_default();
	let Ok(src) = fs::read_to_string(&path) else { return; };
	let a = lex_old::byte_toks(&src);
	let b = lex::byte_toks(&src);
	let mut i = 0;
	let mut runs = Vec::new();
	while i < a.len() {
		let mut j = i;
		while j < a.len() && old_name(a[j]) == old_name(a[i]) && tok_name(b[j]) == tok_name(b[i]) {
			j += 1;
		}
		runs.push((i, j));
		i = j;
	}
	for (i, j) in runs {
		let mark = if old_name(a[i]) == tok_name(b[i]) { "  " } else { "!!" };
		println!("{} {:>5}..{:<5} old {:7} new {:7} {:?}", mark, i, j, old_name(a[i]), tok_name(b[i]), &src[i..j]);
	}
	let lx = lex::Lexer::markup(&src);
	let mut old_state = lex_old::Lexer::markup_over(&src);
	let mut at = 0usize;
	for raw in src.split_inclusive('\n') {
		println!("line@{:<4} old place {:?} depth {} level {:?} bare {} | new place {:?} depth {} level {:?} bare {} | {:?}", at,
			old_state.place_of(raw), old_state.depth(), old_state.markup_level(), old_state.bare_line(raw),
			lx.place_at(at), lx.depth_at(at), lx.markup_level_at(at), lx.bare_line_at(at), raw);
		old_state.feed(raw);
		at += raw.len();
	}
}

/// Prints the syntax errors of the text in the file `LX_PROBE_FILE` as `line:col message`, for comparison with
/// `typst compile --diagnostic-format short`.
#[test]
#[ignore]
fn probe_errors() {
	let path = std::env::var("LX_PROBE_FILE").unwrap_or_default();
	let Ok(src) = fs::read_to_string(&path) else { return; };
	let root = crate::syntax::parser::parse(&src, crate::syntax::FileId(0));
	for (span, e) in root.errors() {
		let before = &src[..(span.start as usize).min(src.len())];
		let line = before.matches('\n').count() + 1;
		let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
		println!("ERR {}:{}: {}", line, col, e.message);
	}
}

#[test]
#[ignore]
fn differential() {
	let only = std::env::var("LX_DIFF_ONLY").unwrap_or_default();
	for g in groups() {
		if !only.is_empty() && !g.name.starts_with(&only) {
			continue;
		}
		let mut t = Tally::default();
		for f in &g.files {
			let Ok(src) = fs::read_to_string(f) else { continue; };
			// A fixture of cases is one document each, as its own test reads it.
			for (k, case) in src.split("\n// ---- case ----\n").enumerate() {
				let label = if g.show { format!("{}#{}", f.display(), k) } else { String::new() };
				compare(case, &label, g.show, &mut t);
			}
		}
		println!("== {}: {} files, {} bytes{}", g.name, t.files, t.bytes, if g.show { "" } else { " (counts only)" });
		let mut metrics: BTreeMap<String, usize> = BTreeMap::new();
		for ((m, o, n), c) in &t.counts {
			println!("   {:12} {} -> {}: {}", m, o, n, c);
			*metrics.entry(m.clone()).or_insert(0) += c;
		}
		if t.counts.is_empty() {
			println!("   no disagreement");
		}
		for n in &t.notes {
			println!("   {}", n);
		}
		if t.quiet > 0 {
			println!("   ... {} more notes", t.quiet);
		}
	}
}
