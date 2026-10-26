//! The mutable-literal gate (D-20261026-02): finds hard-coded data that a second deployment of the
//! same build could rightly want different (locations, hosts, model ids, timings, limits), so that
//! it moves to config.
//!
//! Strings and comments are tokens from `fmt::lex`, never regex guesses. Three things are flagged:
//! a string literal shaped like a path, URL, host, address or model id; a `const` or `static` whose
//! name is in the mutable vocabulary and whose value holds a literal; and a timing call with a
//! literal argument (`Duration::from_secs(30)`, `sleep(30)`, `setTimeout(f, 30)`). Bare numerals in
//! expressions are never flagged.
//!
//! Exempt: comments, the arguments of message macros, `#[cfg(test)]` and `#[test]` items, the body
//! of `impl Default` for a config type (the one sanctioned home of a default), and anything marked
//! `// fixed: <why>`, trailing on its line or as a banner over a group that ends at the first
//! blank line. A why of fewer than two words is itself a failure.
//!
//! A ratchet `Baseline` lists the debt that exists: a hit beyond it fails, and so does an entry no
//! longer found, so the baseline only shrinks. Entries are keyed by file, class and token text,
//! never by line, so edits elsewhere do not churn it.

use crate::fmt::{
    cst::{
        Token,
        TokenKind,
        Trivia,
    },
    lex::{
        js_tokens,
        lex,
        rust_tokens,
    },
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::{
        BTreeMap,
        BTreeSet,
    },
    fmt,
    fs,
    path::Path,
};


/// Source language of a scanned file.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Lang {
    Rust,
    Js,
}

impl Lang {
    /// The language of a file, by extension, or `None` when it is not scanned.
    pub fn of(path: &Path) -> Option<Self> {
        match path.extension().and_then(|e| e.to_str()) {
            Some("rs")                      => Some(Self::Rust),
            Some("js") | Some("mjs")        => Some(Self::Js),
            _                               => None,
        }
    }
}

/// What kind of mutable datum a hit is.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Class {
    Path,   // A file location.
    Host,   // A URL, host, address or `host:port`.
    Model,  // A model id.
    Named,  // A `const` or `static` named in the mutable vocabulary.
    Timing, // A timing call with a literal argument.
}

impl Class {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Path      => "path",
            Self::Host      => "host",
            Self::Model     => "model",
            Self::Named     => "named",
            Self::Timing    => "timing",
        }
    }

    pub fn from_name(s: &str) -> Outcome<Self> {
        match s {
            "path"      => Ok(Self::Path),
            "host"      => Ok(Self::Host),
            "model"     => Ok(Self::Model),
            "named"     => Ok(Self::Named),
            "timing"    => Ok(Self::Timing),
            _           => Err(err!("Unknown mutable-literal class '{}'.", s; Invalid, Input)),
        }
    }
}

impl fmt::Display for Class {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// One flagged literal. `file` is relative to the scanned root, with `/` separators.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hit {
    pub file:   String,
    pub line:   usize,
    pub class:  Class,
    pub text:   String, // Normalised token text, the baseline key.
}

impl Hit {
    pub fn key(&self) -> Key {
        Key { file: self.file.clone(), class: self.class, text: self.text.clone() }
    }
}

impl fmt::Display for Hit {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}:{}: {}: {}", self.file, self.line, self.class, self.text)
    }
}

/// The line-independent identity of a hit.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Key {
    pub file:   String,
    pub class:  Class,
    pub text:   String,
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}: {}", self.file, self.class, self.text)
    }
}

/// A permanent exception that cannot carry a `// fixed:` comment (generated code, embedded data).
/// A `text` of `*` covers every hit of the class in the file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Allow {
    pub file:   String,
    pub class:  Class,
    pub text:   String,
    pub why:    String,
}

impl Allow {
    pub fn key(&self) -> Key {
        Key { file: self.file.clone(), class: self.class, text: self.text.clone() }
    }

    /// Does this exception cover the key?
    pub fn covers(&self, key: &Key) -> bool {
        self.file == key.file && self.class == key.class && (self.text == "*" || self.text == key.text)
    }
}

/// The ratchet: the debt that exists, as a count per key, and the permanent exceptions.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Baseline {
    pub entries:    BTreeMap<Key, usize>,
    pub allow:      Vec<Allow>,
}

impl Baseline {
    pub fn from_hits(hits: &[Hit]) -> Self {
        let mut base = Self::default();
        for h in hits {
            *base.entries.entry(h.key()).or_insert(0) += 1;
        }
        base
    }

    /// A baseline of the given hits that keeps this one's allow list, dropping entries the allow
    /// list already covers.
    pub fn regenerate(&self, hits: &[Hit]) -> Self {
        let kept: Vec<Hit> = hits.iter()
            .filter(|h| { let k = h.key(); !self.allow.iter().any(|a| a.covers(&k)) })
            .cloned().collect();
        let mut base = Self::from_hits(&kept);
        base.allow = self.allow.clone();
        base
    }
}

/// A `// fixed:` marker whose why is too thin to name a standard, format or identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Marker {
    pub file:   String,
    pub line:   usize,
    pub why:    String,
}

/// The hits of one source, and its thin `// fixed:` markers.
#[derive(Clone, Debug, Default)]
pub struct Scan {
    pub hits:       Vec<Hit>,
    pub bad_fixed:  Vec<Marker>,
}

/// The verdict of a tree against its baseline.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub new:        Vec<Hit>,   // Hits beyond the baseline count for their key.
    pub stale:      Vec<Key>,   // Baseline entries with fewer hits than their count.
    pub bad_fixed:  Vec<Marker>,// `// fixed:` markers with a why under two words.
    pub bad_allow:  Vec<Allow>, // Allow entries with an empty why.
    pub files:      usize,      // Files scanned.
}

impl Report {
    pub fn is_clean(&self) -> bool {
        self.new.is_empty()
            && self.stale.is_empty()
            && self.bad_fixed.is_empty()
            && self.bad_allow.is_empty()
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.is_clean() {
            return write!(f, "No new mutable literals in {} files.", self.files);
        }
        if !self.new.is_empty() {
            writeln!(f, "{} new mutable literal(s); move each to config, or mark it \
                `// fixed: <why>` when a protocol, format or identity fixes it:", self.new.len())?;
            for h in &self.new {
                writeln!(f, "  {}", h)?;
            }
        }
        if !self.stale.is_empty() {
            writeln!(f, "{} stale baseline entr(y/ies); delete them from the baseline in this \
                commit:", self.stale.len())?;
            for k in &self.stale {
                writeln!(f, "  {}", k)?;
            }
        }
        if !self.bad_fixed.is_empty() {
            writeln!(f, "{} `// fixed:` marker(s) without a why naming the standard, format or \
                identity:", self.bad_fixed.len())?;
            for h in &self.bad_fixed {
                writeln!(f, "  {}:{}", h.file, h.line)?;
            }
        }
        if !self.bad_allow.is_empty() {
            writeln!(f, "{} allow entr(y/ies) with an empty why:", self.bad_allow.len())?;
            for a in &self.bad_allow {
                writeln!(f, "  {}", a.key())?;
            }
        }
        Ok(())
    }
}

/// Compares the hits of a tree with its baseline.
pub fn check(hits: &[Hit], bad_fixed: &[Marker], base: &Baseline) -> Report {
    let mut rep = Report::default();
    rep.bad_fixed = bad_fixed.to_vec();
    let mut allowed = Vec::new();
    for a in &base.allow {
        if a.why.trim().is_empty() {
            rep.bad_allow.push(a.clone());
        } else {
            allowed.push(a);
        }
    }
    let mut seen: BTreeMap<Key, usize> = BTreeMap::new();
    for h in hits {
        let key = h.key();
        if allowed.iter().any(|a| a.covers(&key)) {
            continue;
        }
        let n = seen.entry(key.clone()).or_insert(0);
        *n += 1;
        let limit = base.entries.get(&key).copied().unwrap_or(0);
        if *n > limit {
            rep.new.push(h.clone());
        }
    }
    for (key, count) in &base.entries {
        if seen.get(key).copied().unwrap_or(0) < *count {
            rep.stale.push(key.clone());
        }
    }
    rep
}

/// Scans every `.rs` and `.js` source under `root`, skipping build output, version-control
/// metadata and the `tests/`, `benches/` and `examples/` trees.
pub fn scan_tree(root: &Path) -> Outcome<(Scan, usize)> {
    let mut files = Vec::new();
    res!(collect(root, root, &mut files));
    files.sort();
    let mut all = Scan::default();
    for (rel, path, lang) in &files {
        let src = match fs::read_to_string(path) {
            Ok(s)   => s,
            Err(e)  => return Err(err!(e, "While reading source {}.", path.display(); IO, File, Read)),
        };
        let scan = res!(scan(&src, *lang, rel));
        all.hits.extend(scan.hits);
        all.bad_fixed.extend(scan.bad_fixed);
    }
    Ok((all, files.len()))
}

/// Scans the tree under `root` and checks it against `base`.
pub fn check_tree(root: &Path, base: &Baseline) -> Outcome<Report> {
    let (scan, files) = res!(scan_tree(root));
    let mut rep = check(&scan.hits, &scan.bad_fixed, base);
    rep.files = files;
    Ok(rep)
}

// fixed: build output, metadata and test-only trees, the gate's own scope rule.
const SKIP_DIRS: [&str; 8] = ["target", "tests", "benches", "examples", "node_modules", ".git", ".ore", "test"];

fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, std::path::PathBuf, Lang)>) -> Outcome<()> {
    let rd = match fs::read_dir(dir) {
        Ok(rd)  => rd,
        Err(e)  => return Err(err!(e, "While listing {}.", dir.display(); IO, File, Read)),
    };
    for entry in rd {
        let entry = res!(entry);
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let ft = res!(entry.file_type());
        if ft.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                res!(collect(root, &path, out));
            }
            continue;
        }
        if !ft.is_file() || name.ends_with("_tests.rs") || name == "tests.rs" || name.ends_with(".min.js") {
            continue;
        }
        if let Some(lang) = Lang::of(&path) {
            let rel = match path.strip_prefix(root) {
                Ok(r)   => r.to_string_lossy().replace('\\', "/"),
                Err(_)  => path.to_string_lossy().to_string(),
            };
            out.push((rel, path, lang));
        }
    }
    Ok(())
}

// Message macros, whose string arguments are messages rather than data, and compile-time includes.
// fixed: the names of std and fe2o3_core macros.
const MESSAGE_MACROS: [&str; 26] = [
    "err", "info", "warn", "error", "debug", "trace", "test", "println", "eprintln", "print",
    "eprint", "panic", "assert", "assert_eq", "assert_ne", "debug_assert", "debug_assert_eq",
    "debug_assert_ne", "unreachable", "todo", "unimplemented", "include_str", "include_bytes",
    "include", "env", "option_env",
];

// fixed: the console and error methods of the JavaScript standard library.
const JS_MESSAGE_CALLS: [&str; 6] = ["log", "warn", "error", "info", "debug", "trace"];

/// Scans one source. `file` is the name recorded in each hit.
pub fn scan(src: &str, lang: Lang, file: &str) -> Outcome<Scan> {
    let toks = match lang {
        Lang::Rust  => res!(lex(src, &rust_tokens())),
        Lang::Js    => res!(lex(src, &js_tokens())),
    };
    let lines = LineIndex::new(src);
    let mut out = Scan::default();
    let fixed = fixed_lines(src, &toks, &lines, file, &mut out.bad_fixed);
    let skip = skipped(&toks, lang);
    let waits = numeric_consts(&toks, lang, &lines, &fixed, &skip);

    let mut i = 0;
    while i < toks.len() {
        if skip[i] {
            i += 1;
            continue;
        }
        let line = lines.line_of(toks[i].span.start);
        let covered = fixed.contains(&line);
        // A const or static item.
        if let Some((name, end, literal)) = const_item(&toks, i, lang) {
            if is_mutable_name(&name) && literal {
                if !covered {
                    out.hits.push(Hit { file: file.to_string(), line, class: Class::Named, text: name });
                }
                i = end;
                continue;
            }
        }
        if let Some((text, end)) = timing_call(&toks, i, lang, &waits) {
            if !covered {
                out.hits.push(Hit { file: file.to_string(), line, class: Class::Timing, text });
            }
            i = end;
            continue;
        }
        // A string, folded with the literal pieces joined to it by `concat!` or `+`, so that a
        // value split across pieces is classified whole. The hit is on the first piece's line.
        let folded = match concat_macro(&toks, i, lang) {
            Some(f) => Some(f),
            None if toks[i].kind == TokenKind::StringLit
                && !(lang == Lang::Js && is_module_specifier(&toks, i)) => Some(plus_chain(&toks, i, &skip)),
            None => None,
        };
        if let Some((text, end)) = folded {
            if let Some(class) = classify(&text) {
                if !covered {
                    out.hits.push(Hit { file: file.to_string(), line, class, text });
                }
            }
            i = end;
            continue;
        }
        i += 1;
    }
    Ok(out)
}

struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(src: &str) -> Self {
        let mut starts = vec![0];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        Self { starts }
    }

    /// The 1-based line holding byte `pos`.
    fn line_of(&self, pos: usize) -> usize {
        match self.starts.binary_search(&pos) {
            Ok(i)   => i + 1,
            Err(i)  => i,
        }
    }
}

/// The lines covered by a `// fixed:` marker, trailing on its line or as a banner over the lines
/// that follow it up to the first blank line. Thin markers still cover, and are reported.
fn fixed_lines(
    src:    &str,
    toks:   &[Token],
    lines:  &LineIndex,
    file:   &str,
    bad:    &mut Vec<Marker>,
)
    -> BTreeSet<usize>
{
    let blank: Vec<bool> = src.split('\n').map(|l| l.trim().is_empty()).collect();
    let mut covered = BTreeSet::new();
    let mut prev_end: Option<usize> = None;
    for tok in toks {
        let mut line = match prev_end {
            Some(e) => lines.line_of(e.saturating_sub(1)),
            None    => 1,
        };
        let mut has_code = prev_end.is_some();
        for t in &tok.leading_trivia {
            match t {
                Trivia::Newline             => { line += 1; has_code = false; }
                Trivia::Whitespace(_)       => {}
                Trivia::BlockComment(c)     => { line += c.matches('\n').count(); }
                Trivia::LineComment(c)      => {
                    let body = c.trim_start_matches('/').trim();
                    if let Some(why) = body.strip_prefix("fixed:") {
                        let why = why.trim();
                        if why.split_whitespace().count() < 2 {
                            bad.push(Marker { file: file.to_string(), line, why: why.to_string() });
                        }
                        if has_code {
                            covered.insert(line);
                        } else {
                            let mut l = line + 1;
                            while l <= blank.len() && !blank[l - 1] {
                                covered.insert(l);
                                l += 1;
                            }
                        }
                    }
                }
            }
        }
        prev_end = Some(tok.span.end);
    }
    covered
}

fn is_punct(t: &Token, c: char) -> bool {
    t.kind == TokenKind::Punct(c)
}

fn is_ident(t: &Token, s: &str) -> bool {
    t.kind == TokenKind::Ident && t.text == s
}

fn is_keyword(t: &Token, s: &str) -> bool {
    matches!(&t.kind, TokenKind::Keyword(k) if k == s)
}

/// The index just past the group that opens at `open`, or the end of the tokens.
fn group_end(toks: &[Token], open: usize) -> usize {
    let mut depth = 0isize;
    let mut i = open;
    while i < toks.len() {
        match toks[i].kind {
            TokenKind::Punct('(') | TokenKind::Punct('[') | TokenKind::Punct('{') => depth += 1,
            TokenKind::Punct(')') | TokenKind::Punct(']') | TokenKind::Punct('}') => {
                depth -= 1;
                if depth <= 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    toks.len()
}

/// The index just past the item that starts at `i`: its first depth-0 `;`, or the close of its
/// first brace block.
fn item_end(toks: &[Token], mut i: usize) -> usize {
    while i < toks.len() && toks[i].kind == TokenKind::Attribute {
        i += 1;
    }
    while i < toks.len() {
        match toks[i].kind {
            TokenKind::Punct(';')                           => return i + 1,
            TokenKind::Punct('{')                           => return group_end(toks, i),
            TokenKind::Punct('(') | TokenKind::Punct('[')   => i = group_end(toks, i),
            _                                               => i += 1,
        }
    }
    toks.len()
}

/// Marks the tokens no rule looks at: test items, message macro arguments, and the `Default` of
/// a config type.
fn skipped(toks: &[Token], lang: Lang) -> Vec<bool> {
    let mut skip = vec![false; toks.len()];
    let configs = config_types(toks);
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        let mut span: Option<(usize, usize)> = None;
        if t.kind == TokenKind::Attribute {
            let a: String = t.text.chars().filter(|c| !c.is_whitespace()).collect();
            if a == "#[test]" || a.starts_with("#[cfg(test") || a.contains("::test]")
                || a.starts_with("#[cfg(all(test") || a.starts_with("#[cfg(any(test")
            {
                span = Some((i, item_end(toks, i + 1)));
            }
        } else if t.kind == TokenKind::Ident && i + 2 < toks.len() && is_punct(&toks[i + 1], '!')
            && MESSAGE_MACROS.contains(&t.text.as_str())
            && matches!(toks[i + 2].kind,
                TokenKind::Punct('(') | TokenKind::Punct('[') | TokenKind::Punct('{'))
        {
            span = Some((i, group_end(toks, i + 2)));
        } else if lang == Lang::Js && is_ident(t, "console") {
            // `console.log(…)`.
            if i + 3 < toks.len() && is_punct(&toks[i + 1], '.')
                && JS_MESSAGE_CALLS.contains(&toks[i + 2].text.as_str()) && is_punct(&toks[i + 3], '(')
            {
                span = Some((i, group_end(toks, i + 3)));
            }
        } else if lang == Lang::Js && is_keyword(t, "throw") {
            // `throw [new] X(…)`, where `(` is reached within the statement: `throw e;` is not a call.
            if let Some(j) = throw_call_open(toks, i) {
                span = Some((i, group_end(toks, j)));
            }
        } else if lang == Lang::Rust && is_keyword(t, "impl") {
            // `impl Default for T {…}` where T is a config type.
            let mut j = i + 1;
            if j < toks.len() && is_punct(&toks[j], '<') {
                while j < toks.len() && !is_punct(&toks[j], '>') {
                    j += 1;
                }
                j += 1;
            }
            if j + 2 < toks.len() && is_ident(&toks[j], "Default") && is_keyword(&toks[j + 1], "for")
                && configs.contains(&toks[j + 2].text)
            {
                span = Some((i, item_end(toks, j)));
            }
        }
        match span {
            Some((a, b)) => {
                for k in a..b.min(toks.len()) {
                    skip[k] = true;
                }
                i = b.max(i + 1);
            }
            None => i += 1,
        }
    }
    skip
}

/// The `(` of the error constructor in `throw [new] X.Y(…)` at `i`, if the statement is that call.
/// The walk stops at `;`, `}` or a token that starts a new line, so it never leaves the statement.
fn throw_call_open(toks: &[Token], i: usize) -> Option<usize> {
    let mut j = i + 1;
    if toks.get(j).map_or(false, |t| is_keyword(t, "new") || is_ident(t, "new")) {
        j += 1;
    }
    let mut want_name = true;
    while j < toks.len() {
        let t = &toks[j];
        if t.leading_trivia.iter().any(|v| matches!(v, Trivia::Newline)) {
            return None;
        }
        match t.kind {
            TokenKind::Ident if want_name   => want_name = false,
            TokenKind::Punct('.') if !want_name => want_name = true,
            TokenKind::Punct('(') if !want_name => return Some(j),
            _                               => return None,
        }
        j += 1;
    }
    None
}

/// The types in this source that are config types: derive `FromDatMap`, or implement `Config`.
fn config_types(toks: &[Token]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut pending = false;
    for (i, t) in toks.iter().enumerate() {
        if t.kind == TokenKind::Attribute && t.text.contains("FromDatMap") {
            pending = true;
        } else if pending && (is_keyword(t, "struct") || is_keyword(t, "enum")) {
            if let Some(n) = toks.get(i + 1) {
                out.insert(n.text.clone());
            }
            pending = false;
        } else if is_keyword(t, "impl") {
            // `impl Config for T` or `impl cfg::Config for T`.
            let mut j = i + 1;
            while j < toks.len() && j < i + 6 && !is_keyword(&toks[j], "for") {
                j += 1;
            }
            if j + 1 < toks.len() && j > i + 1 && is_ident(&toks[j - 1], "Config") {
                out.insert(toks[j + 1].text.clone());
            }
        }
    }
    out
}

/// A `const` or `static` item at `i`: its name, the index past it, and whether its value holds
/// a number or string literal.
fn const_item(toks: &[Token], i: usize, lang: Lang) -> Option<(String, usize, bool)> {
    if !(is_keyword(&toks[i], "const") || is_keyword(&toks[i], "static")) {
        return None;
    }
    let mut j = i + 1;
    if j < toks.len() && (is_keyword(&toks[j], "mut") || is_keyword(&toks[j], "ref")) {
        j += 1;
    }
    let name_tok = toks.get(j)?;
    if name_tok.kind != TokenKind::Ident {
        return None; // `const fn`, `const {`, or destructuring.
    }
    let name = name_tok.text.clone();
    // Find the `=` before any depth-0 `;`, `{`, `,` or `>`.
    let mut k = j + 1;
    let mut depth = 0isize;
    while k < toks.len() {
        match toks[k].kind {
            TokenKind::Punct('<') | TokenKind::Punct('(') | TokenKind::Punct('[') => depth += 1,
            TokenKind::Punct('>') | TokenKind::Punct(')') | TokenKind::Punct(']') => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            TokenKind::Punct('=') if depth == 0 => break,
            TokenKind::Punct(';') | TokenKind::Punct('{') | TokenKind::Punct(',') if depth == 0 => {
                return None;
            }
            _ => {}
        }
        k += 1;
    }
    if k >= toks.len() {
        return None;
    }
    // A JavaScript object or array is a table, not a scalar setting; its strings are still scanned.
    if lang == Lang::Js && toks.get(k + 1).map_or(false, |t| is_punct(t, '{') || is_punct(t, '[')) {
        return None;
    }
    let mut literal = false;
    let mut m = k + 1;
    let mut depth = 0isize;
    while m < toks.len() {
        let t = &toks[m];
        if depth == 0 && m > k + 1 && lang == Lang::Js
            && t.leading_trivia.iter().any(|v| matches!(v, Trivia::Newline))
            && !matches!(toks[m - 1].kind, TokenKind::Punct(_) | TokenKind::Operator(_))
        {
            break; // JavaScript without a semicolon.
        }
        match t.kind {
            TokenKind::Punct(';') if depth == 0                                 => { m += 1; break; }
            TokenKind::Punct('(') | TokenKind::Punct('[') | TokenKind::Punct('{') => depth += 1,
            TokenKind::Punct(')') | TokenKind::Punct(']') | TokenKind::Punct('}') => depth -= 1,
            TokenKind::Number | TokenKind::StringLit                            => literal = true,
            TokenKind::Eof                                                      => break,
            _ => {}
        }
        m += 1;
    }
    Some((name, m, literal))
}

/// Is the string at `i` a JavaScript module specifier (`from "…"`, `import("…")`,
/// `require("…")`), which names code rather than data?
fn is_module_specifier(toks: &[Token], i: usize) -> bool {
    if i >= 1 && (is_keyword(&toks[i - 1], "from") || is_ident(&toks[i - 1], "from")
        || is_keyword(&toks[i - 1], "import"))
    {
        return true;
    }
    i >= 2 && is_punct(&toks[i - 1], '(')
        && (is_keyword(&toks[i - 2], "import") || is_ident(&toks[i - 2], "require"))
}

/// Is this item name in the mutable vocabulary?
fn is_mutable_name(name: &str) -> bool {
    if name.chars().any(|c| c.is_ascii_lowercase()) {
        return false; // Constants are upper snake case; anything else is not named data.
    }
    let parts: Vec<&str> = name.split('_').filter(|p| !p.is_empty()).collect();
    if parts.contains(&"PER") {
        return false; // A unit conversion, `SECONDS_PER_HOUR`.
    }
    if parts.first() == Some(&"ENABLE") {
        return true;
    }
    parts.iter().any(|p| {
        matches!(*p,
            "MAX" | "LIMIT" | "LIMITS" | "CAP" | "CAPACITY" | "TTL" | "PORT" | "PORTS"
            | "HOST" | "HOSTS" | "URL" | "URLS" | "MODEL" | "MODELS" | "PRICE" | "PRICES" | "FEE"
            | "FEES" | "COST" | "COSTS" | "SCHEDULE" | "MS" | "SECS" | "SEC" | "SECONDS" | "MILLIS")
        || p.starts_with("TIMEOUT") || p.starts_with("THRESH") || p.starts_with("RETR")
        || p.starts_with("INTERVAL") || p.starts_with("ATTEMPT") || p.starts_with("BACKOFF")
    })
}

/// The consts in this source that a timing call may take as its argument: bound to literal-only
/// arithmetic, named outside the mutable vocabulary (those are flagged as named already), and not
/// marked `// fixed:`.
fn numeric_consts(
    toks:   &[Token],
    lang:   Lang,
    lines:  &LineIndex,
    fixed:  &BTreeSet<usize>,
    skip:   &[bool],
)
    -> BTreeSet<String>
{
    let mut out = BTreeSet::new();
    for i in 0..toks.len() {
        if skip[i] || fixed.contains(&lines.line_of(toks[i].span.start)) {
            continue;
        }
        if let Some((name, end, _)) = const_item(toks, i, lang) {
            if is_mutable_name(&name) {
                continue;
            }
            let eq = match (i + 1..end).find(|&k| is_punct(&toks[k], '=')) {
                Some(k) => k,
                None    => continue,
            };
            let mut stop = end;
            if stop > eq + 1 && is_punct(&toks[stop - 1], ';') {
                stop -= 1;
            }
            if numeric_expr(&toks[eq + 1..stop]).is_some() {
                out.insert(name);
            }
        }
    }
    out
}

/// Is a number token nonzero?
fn nonzero(t: &Token) -> bool {
    t.kind == TokenKind::Number && t.text.trim_start_matches(|c| c == '0' || c == '.' || c == '_')
        .chars().next().map_or(false, |c| c.is_ascii_digit())
}

/// Literal-only arithmetic (`30`, `5 * 60`, `(1 << 4)`), as normalised text with whether any
/// number in it is nonzero.
fn numeric_expr(toks: &[Token]) -> Option<(String, bool)> {
    if toks.is_empty() {
        return None;
    }
    let mut text = String::new();
    let mut any = false;
    let mut seen = false;
    for t in toks {
        let piece = match &t.kind {
            TokenKind::Number => {
                seen = true;
                any = any || nonzero(t);
                t.text.as_str()
            }
            TokenKind::Punct(c) if "*+-/%()".contains(*c)   => t.text.as_str(),
            TokenKind::Operator(o) if o == "<<" || o == ">>" => t.text.as_str(),
            _                                               => return None,
        };
        if !text.is_empty() && !text.ends_with('(') && piece != ")" {
            text.push(' ');
        }
        text.push_str(piece);
    }
    if seen { Some((text, any)) } else { None }
}

/// The normalised text of a timing call's argument list `toks[open + 1..close]`, when every
/// argument is literal-only arithmetic or a const of `waits`, and one at least is nonzero.
fn timing_args(toks: &[Token], waits: &BTreeSet<String>) -> Option<String> {
    let mut args = Vec::new();
    let mut live = false;
    for arg in toks.split(|t| is_punct(t, ',')) {
        if arg.len() == 1 && arg[0].kind == TokenKind::Ident && waits.contains(&arg[0].text) {
            live = true;
            args.push(arg[0].text.clone());
        } else {
            let (text, any) = numeric_expr(arg)?;
            live = live || any;
            args.push(text);
        }
    }
    if live { Some(args.join(", ")) } else { None }
}

/// A timing call with a literal argument at `i`: its normalised text and the index past it. The
/// argument may be literal-only arithmetic or a same-file const in `waits`.
fn timing_call(toks: &[Token], i: usize, lang: Lang, waits: &BTreeSet<String>) -> Option<(String, usize)> {
    let t = &toks[i];
    if t.kind != TokenKind::Ident {
        return None;
    }
    // The arguments of the flat group at `open`, which must hold no nested call.
    let flat = |open: usize| -> Option<(usize, String)> {
        let end = group_end(toks, open);
        if end < open + 2 || !is_punct(&toks[end - 1], ')') {
            return None;
        }
        Some((end, timing_args(&toks[open + 1..end - 1], waits)?))
    };
    match lang {
        Lang::Rust => {
            if t.text == "Duration" && i + 3 < toks.len()
                && toks[i + 1].kind == TokenKind::Operator("::".to_string())
                && (toks[i + 2].text.starts_with("from_") || toks[i + 2].text == "new")
                && is_punct(&toks[i + 3], '(')
            {
                let (end, args) = flat(i + 3)?;
                return Some((fmt!("Duration::{}({})", toks[i + 2].text, args), end));
            }
            if t.text == "sleep" && i + 1 < toks.len() && is_punct(&toks[i + 1], '(') {
                let (end, args) = flat(i + 1)?;
                return Some((fmt!("sleep({})", args), end));
            }
            None
        }
        Lang::Js => {
            if (t.text == "setTimeout" || t.text == "setInterval") && i + 1 < toks.len()
                && is_punct(&toks[i + 1], '(')
            {
                let end = group_end(toks, i + 1);
                // The last argument, after the final depth-1 comma.
                let mut depth = 0isize;
                let mut comma = None;
                for k in i + 1..end {
                    match toks[k].kind {
                        TokenKind::Punct('(') | TokenKind::Punct('[') | TokenKind::Punct('{') => depth += 1,
                        TokenKind::Punct(')') | TokenKind::Punct(']') | TokenKind::Punct('}') => depth -= 1,
                        TokenKind::Punct(',') if depth == 1 => comma = Some(k),
                        _ => {}
                    }
                }
                let c = comma?;
                if end >= 1 && is_punct(&toks[end - 1], ')') {
                    let args = timing_args(&toks[c + 1..end - 1], waits)?;
                    return Some((fmt!("{}({})", t.text, args), end));
                }
            }
            None
        }
    }
}

/// Rust `concat!(…)` at `i`: its literal arguments joined, and the index past it. Any other
/// argument stands as `{}`, as in a format string.
fn concat_macro(toks: &[Token], i: usize, lang: Lang) -> Option<(String, usize)> {
    if lang != Lang::Rust || !is_ident(&toks[i], "concat") || i + 2 >= toks.len()
        || !is_punct(&toks[i + 1], '!') || !is_punct(&toks[i + 2], '(')
    {
        return None;
    }
    let end = group_end(toks, i + 2);
    let mut text = String::new();
    let mut j = i + 3;
    while j + 1 < end {
        let t = &toks[j];
        match t.kind {
            TokenKind::StringLit                    => { text.push_str(string_content(&t.text)); j += 1; }
            TokenKind::Number | TokenKind::CharLit  => { text.push_str(string_content(&t.text)); j += 1; }
            TokenKind::Punct(',')                   => j += 1,
            _ => {
                // One non-literal argument, up to its depth-0 comma.
                text.push_str("{}");
                while j + 1 < end && !is_punct(&toks[j], ',') {
                    j = match toks[j].kind {
                        TokenKind::Punct('(') | TokenKind::Punct('[') | TokenKind::Punct('{') => {
                            group_end(toks, j)
                        }
                        _ => j + 1,
                    };
                }
            }
        }
    }
    Some((text, end))
}

/// The string at `i` joined with the literals that follow it in a `+` chain, and the index past
/// the last piece. A piece may sit in a one-argument call (`String::from("…")`) or take a
/// no-argument method (`"…".to_string()`); the chain ends at the first operand that is not a literal.
fn plus_chain(toks: &[Token], i: usize, skip: &[bool]) -> (String, usize) {
    let mut text = string_content(&toks[i].text).to_string();
    let mut j = i; // The current piece's string.
    loop {
        let mut k = j + 1;
        if j >= 1 && is_punct(&toks[j - 1], '(') && toks.get(k).map_or(false, |t| is_punct(t, ')')) {
            k += 1;
        }
        if k + 3 < toks.len() && is_punct(&toks[k], '.') && toks[k + 1].kind == TokenKind::Ident
            && is_punct(&toks[k + 2], '(') && is_punct(&toks[k + 3], ')')
        {
            k += 4;
        }
        match (toks.get(k), toks.get(k + 1)) {
            (Some(p), Some(n)) if is_punct(p, '+') && n.kind == TokenKind::StringLit && !skip[k + 1] => {
                text.push_str(string_content(&n.text));
                j = k + 1;
            }
            _ => return (text, j + 1),
        }
    }
}

/// The content of a string literal token, without prefix, hashes or delimiters.
fn string_content(text: &str) -> &str {
    let s = text.trim_start_matches(|c| c == 'b' || c == 'r' || c == 'c');
    let s = s.trim_start_matches('#').trim_end_matches('#');
    let mut cs = s.chars();
    match (cs.next(), s.chars().last()) {
        (Some(a), Some(z)) if s.len() >= 2 && a == z && (a == '"' || a == '\'' || a == '`') => {
            &s[1..s.len() - 1]
        }
        _ => s,
    }
}

// fixed: top-level domains that mark a bare host name, a heuristic of the gate itself.
const TLDS: [&str; 16] = [
    "com", "org", "net", "io", "dev", "app", "ai", "co", "uk", "au", "gov", "edu", "cloud", "sh",
    "me", "info",
];

// fixed: state and config file extensions, a heuristic of the gate itself.
const STATE_EXTS: [&str; 17] = [
    "jdat", "json", "pem", "key", "crt", "cer", "sock", "log", "db", "sqlite", "toml", "yaml",
    "yml", "pid", "conf", "cfg", "ini",
];

// fixed: model id families of the major providers, a heuristic of the gate itself.
const MODEL_PREFIXES: [&str; 24] = [
    "gpt-", "claude-", "gemini-", "llama-", "llama3", "mistral-", "mixtral-", "o1-", "o3-", "o4-",
    "text-embedding-", "whisper-", "dall-e-", "deepseek-", "qwen", "command-r", "grok-", "sonar-",
    "pixtral-", "codestral-", "openai/", "anthropic/", "google/", "meta-llama/",
];

/// The class of a string literal's content, if it is mutable data.
fn classify(s: &str) -> Option<Class> {
    if s.is_empty() || s.chars().any(|c| c.is_whitespace()) || s.contains("\\n") {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    if MODEL_PREFIXES.iter().any(|p| lower.starts_with(p)) && lower.len() > 4
        && !lower.ends_with('-')
    {
        return Some(Class::Model);
    }
    if let Some(p) = s.find("://") {
        if p > 0 && s[..p].chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
            && s.len() > p + 3
        {
            return Some(Class::Host);
        }
    }
    if is_host(s) {
        return Some(Class::Host);
    }
    if is_path(s) {
        return Some(Class::Path);
    }
    None
}

fn is_host(s: &str) -> bool {
    let (host, port) = match s.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.len() <= 5 && p.chars().all(|c| c.is_ascii_digit()) => {
            (h, true)
        }
        _ => (s, false),
    };
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() == 4 && labels.iter().all(|l| !l.is_empty() && l.len() <= 3
        && l.chars().all(|c| c.is_ascii_digit()))
    {
        return true; // IPv4.
    }
    if host == "localhost" {
        return port || s == "localhost";
    }
    labels.len() >= 2
        && labels.iter().all(|l| !l.is_empty()
            && l.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
        && labels.last().map_or(false, |t| TLDS.contains(t))
}

fn is_path(s: &str) -> bool {
    let path_char = |c: char| c.is_ascii_alphanumeric() || "/._-~{}".contains(c);
    if !s.chars().all(path_char) {
        return false;
    }
    let mut cs = s.chars();
    let first = cs.next();
    let second = cs.next();
    match (first, second) {
        // Two characters at least after the slash: `/t` is a command switch, not a location.
        (Some('/'), Some(c)) if s.len() >= 3 && (c.is_ascii_alphanumeric() || c == '.' || c == '_') => {
            return true;
        }
        _ => {}
    }
    if (s.starts_with("~/") || s.starts_with("./") || s.starts_with("../")) && s.len() > 2 {
        return true;
    }
    match s.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty() && !stem.ends_with('/') && STATE_EXTS.contains(&ext)
                && stem.chars().next().map_or(false, |c| c.is_ascii_alphanumeric() || c == '{')
        }
        None => false,
    }
}
