//! The Hematite gate (D-20261107-02): the mechanical rules of `~/usr/CLAUDE.md`, checked only on
//! the lines a change adds, so that new code meets the spec and existing code is left alone.
//!
//! The rules are a table (`Rules`, read from `.jdat` by `fe2o3_jdat::lint::load_rules`). Each rule
//! names one of the `Check`s below, so a rule made of a token sequence or a word list is added to
//! the table without code. This is a token scanner, not a parser. What it does not count:
//!
//! - strings, chars and comments, which are tokens or trivia from `fmt::lex` (bar `CommentWords`,
//!   which reads only comments, and outside backticks there),
//! - a `?` that does not follow an expression (`?Sized`, `<?>`), a `?` that closes a `$( … )`
//!   repetition, and any raw lock call in a `macro_rules!` body, where the lock macros are defined,
//! - `.lock()` on `stdin()`, `stdout()` or `stderr()`, which are not data locks, and a lock call
//!   that is awaited (an async lock, which the lock macros do not take),
//! - a capitalised word within a sentence of a comment, which is a name (`Serialize`), not a
//!   spelling,
//! - `pub use` in import order, since a re-export is placed for the reader of the module,
//! - `include_str!` and `include_bytes!`, which read data, not code (G14 refuses only `include!`
//!   and `#[path]` of a file not ending `.rs`, which the gate would not read),
//! - test code (a `tests` directory with no `src` above it, an item under an attribute whose path
//!   is exactly one of `Rules::test_attrs` or under a `cfg` that holds only under test, such as
//!   `all(test, unix)`) for a rule whose `in_tests` is false,
//! - a line marked `// fixed: <why>` for a rule whose `fixed_ok` is true,
//! - any line carrying `allowlist hematite`, which is reported in `Report::allows` so that the
//!   escape is shown, not silent.

use crate::{
    fmt::{
        cst::{
            Token,
            TokenKind,
            Trivia,
        },
        lex::{
            lex,
            rust_tokens,
        },
    },
    lint::mutable::{
        self,
        fixed_lines,
        group_end,
        is_ident,
        is_keyword,
        is_punct,
        item_end,
        Lang,
        LineIndex,
    },
    secret,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::{
        BTreeMap,
        BTreeSet,
    },
    fmt,
};


/// What a rule looks for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Check {
    Seq(Vec<String>),           // consecutive code tokens with these texts
    Question,                   // the `?` operator after an expression
    OkArgs,                     // `ok!` given more than one argument
    RawLock,                    // `.read()`, `.write()` or `.lock()` with no argument
    DocMember,                  // `///` on an enum variant, struct field or constant
    DocHeading(Vec<String>),    // a `# <word>` heading in a doc comment
    ImportOrder,                // local, local libraries, std, third party; one `use` a line
    Credential,                 // a credential shape, in a file of any kind
    Mutable,                    // a literal of mutable data, as `lint::mutable` finds it
    CommentWords(Vec<String>),  // a comment word starting with one of these, case aside
    Placement,                  // `#[path]` or `include!` of a file not ending `.rs`
    TestAlias,                  // a rename to a name that `Rules::test_attrs` begins with
    Link,                       // a symlink the change adds, whose target the gate does not read
    TestPath,                   // product code placed under `tests/`, where test code is exempt
    CfgTest,                    // a build setting `test`, which makes `#[cfg(test)]` code product
}

impl Check {
    /// The check named `name`; `list` is the `seq` or `words` of the checks that take one.
    pub fn of(name: &str, list: Vec<String>) -> Outcome<Self> {
        let takes_list = matches!(name, "seq" | "doc_heading" | "comment_words");
        if takes_list && list.is_empty() {
            return Err(err!("The Hematite check '{}' needs a non-empty list.", name;
                Invalid, Input, Missing));
        }
        if !takes_list && !list.is_empty() {
            return Err(err!("The Hematite check '{}' takes no list.", name; Invalid, Input));
        }
        Ok(match name {
            "seq"           => Self::Seq(list),
            "question"      => Self::Question,
            "ok_args"       => Self::OkArgs,
            "raw_lock"      => Self::RawLock,
            "doc_member"    => Self::DocMember,
            "doc_heading"   => Self::DocHeading(list),
            "import_order"  => Self::ImportOrder,
            "credential"    => Self::Credential,
            "mutable"       => Self::Mutable,
            "comment_words" => Self::CommentWords(list),
            "placement"     => Self::Placement,
            "test_alias"    => Self::TestAlias,
            "link"          => Self::Link,
            "test_path"     => Self::TestPath,
            "cfg_test"      => Self::CfgTest,
            _               => return Err(err!("Unknown Hematite check '{}'.", name;
                Invalid, Input)),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rule {
    pub id:         String,
    pub on:         bool,
    pub in_tests:   bool,   // applies in test code too
    pub fixed_ok:   bool,   // a `// fixed: <why>` marker exempts the line
    pub msg:        String, // told to the author after `<file>:<line>: `
    pub check:      Check,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Rules {
    pub local_crates:   Vec<String>, // name prefixes of the local libraries, the second import group
    pub test_attrs:     Vec<String>, // attribute paths that mark a test item, matched exactly
    pub rules:          Vec<Rule>,
}

/// A file as a change leaves it, with the lines the change adds (1-based).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Added {
    pub file:   String,
    pub src:    String,             // a symlink's target
    pub lines:  BTreeSet<usize>,
    pub link:   bool,
}

/// One file of a diff taken with [`diff_args`], by its new path.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FileDiff {
    pub lines:  BTreeSet<usize>,    // added, 1-based in the new file
    pub link:   bool,               // the new side is a symlink
    pub old:    bool,               // the file has an old side
    pub hunks:  Vec<Hunk>,
}

/// A hunk header's spans; with no context, a count of 0 has its start on the line before.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Hunk {
    pub old_start:  usize,
    pub old_count:  usize,
    pub new_start:  usize,
    pub new_count:  usize,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Breach {
    pub file:   String,
    pub line:   usize,
    pub rule:   String,
    pub msg:    String,
}

impl fmt::Display for Breach {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.msg)
    }
}

/// An added line carrying `allowlist hematite` that would otherwise break a rule.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Allowed {
    pub file:   String,
    pub line:   usize,
    pub text:   String,
    pub rules:  Vec<String>,    // ids of the rules the line would otherwise break
}

impl fmt::Display for Allowed {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}:{}: allowlist hematite ({}): {}", self.file, self.line, self.rules.join(","),
            self.text.trim())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Report {
    pub breaches:   Vec<Breach>,
    pub allows:     Vec<Allowed>,
}

impl Report {
    pub fn is_clean(&self) -> bool {
        self.breaches.is_empty()
    }
}

// The environment of every git call that feeds the gate: no system config or attributes, no
// replaced objects and no grafts, so that what a commit holds, and what its parents are, is what
// its hash names.  A graft in the repository could otherwise make a commit the change adds into
// an ancestor of the base.
pub const GIT_ENV: [(&str, &str); 4] = [
    ("GIT_CONFIG_NOSYSTEM",     "1"),
    ("GIT_ATTR_NOSYSTEM",       "1"),
    ("GIT_NO_REPLACE_OBJECTS",  "1"),
    ("GIT_GRAFT_FILE",          "/dev/null"), // fixed: an empty graft file
];

pub const INFO_ATTRIBUTES: &str = "info/attributes"; // fixed: git's layout, under `git rev-parse --git-path`

/// The arguments of the one `git` call whose output [`file_diffs`] reads, `revs` being
/// `--cached` or a base and a head. The gate, not the repository, decides what counts as a change:
/// every file is text (a `-diff` attribute or a NUL byte cannot hide one), no textconv or external
/// driver runs, a rename is a deletion and an addition (so a file moved out of `tests/` is read
/// whole), and the repository's config cannot reshape the hunks.
pub fn diff_args(revs: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = [
        "-c", "core.quotePath=false",
        "-c", "core.attributesFile=",
        "-c", "diff.noprefix=false",
        "-c", "diff.mnemonicPrefix=false",
        "-c", "diff.relative=false",
        "-c", "diff.suppressBlankEmpty=false",
        "diff", "-U0", "--inter-hunk-context=0", "--no-color", "--text", "--no-textconv",
        "--no-renames", "--no-ext-diff", "--src-prefix=a/", "--dst-prefix=b/",
    ].iter().map(|a| a.to_string()).collect();
    v.extend(revs.iter().map(|r| r.to_string()));
    v
}

/// The `.gitattributes` files among `names` (as `git ls-tree -r --name-only -z` or
/// `git ls-files -z` gives them), which the gate refuses: attributes decide whether git reads a
/// file as text.
pub fn attributes_in(names: &str) -> Vec<String> {
    names.split(['\0', '\n'])
        .filter(|n| *n == ".gitattributes" || n.ends_with("/.gitattributes")) // fixed: git's attributes file name
        .map(|n| n.to_string())
        .collect()
}

/// Each file a unified diff taken with [`diff_args`] adds lines to, by the file's new path.
pub fn file_diffs(diff: &str) -> BTreeMap<String, FileDiff> {
    let mut out: BTreeMap<String, FileDiff> = BTreeMap::new();
    let mut file: Option<String> = None;
    let mut link = false;
    let mut old_side = false;
    let mut old_left = 0usize;
    let mut new_left = 0usize;
    let mut at = 0usize;
    for l in diff.lines() {
        if old_left > 0 || new_left > 0 {
            match l.as_bytes().first() {
                Some(b'+') if new_left > 0 => {
                    if let Some(f) = &file {
                        out.entry(f.clone()).or_default().lines.insert(at);
                    }
                    at += 1;
                    new_left -= 1;
                    continue;
                }
                Some(b'-') if old_left > 0 => {
                    old_left -= 1;
                    continue;
                }
                Some(b' ') => {
                    old_left = old_left.saturating_sub(1);
                    new_left = new_left.saturating_sub(1);
                    at += 1;
                    continue;
                }
                Some(b'\\') => continue,
                _ => {
                    old_left = 0;
                    new_left = 0;
                }
            }
        }
        if l.starts_with("diff --git ") {
            file = None;
            link = false;
            old_side = false;
        } else if let Some(p) = l.strip_prefix("--- ") {
            old_side = p != "/dev/null"; // fixed: git diff format
        } else if l.starts_with("new file mode ") || l.starts_with("new mode ")
            || l.starts_with("index ")
        {
            // fixed: git's mode of a symbolic link.
            link |= l.ends_with(" 120000");
        } else if let Some(p) = l.strip_prefix("+++ ") {
            file = match p {
                "/dev/null" => None, // fixed: git diff format
                _           => Some(p.strip_prefix("b/").unwrap_or(p).to_string()),
            };
            if let Some(f) = &file {
                let d = out.entry(f.clone()).or_default();
                d.link = link;
                d.old = old_side;
            }
        } else if let Some(h) = l.strip_prefix("@@ ") {
            let mut parts = h.split_whitespace();
            let old = parts.next().and_then(|p| p.strip_prefix('-')).map(span_of);
            let new = parts.next().and_then(|p| p.strip_prefix('+')).map(span_of);
            if let (Some((os, o)), Some((s, n))) = (old, new) {
                old_left = o;
                new_left = n;
                at = s;
                if let Some(f) = &file {
                    out.entry(f.clone()).or_default().hunks.push(Hunk { old_start: os, old_count: o,
                        new_start: s, new_count: n });
                }
            }
        }
    }
    out
}

/// `start[,count]` of a hunk header, the count 1 when absent.
fn span_of(s: &str) -> (usize, usize) {
    let mut it = s.splitn(2, ',');
    let start = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let count = it.next().map(|v| v.parse().unwrap_or(0)).unwrap_or(1);
    (start, count)
}

/// The file `file` as the change leaves it, `src`, with the lines the change adds: those of the
/// diff `d`, and those it leaves in place that were test code in `old` (the file at the base) and
/// are product code now, as when a `#[cfg(test)]` line is deleted.
pub fn added(file: String, d: FileDiff, src: String, old: Option<&str>, rules: &Rules)
    -> Outcome<Added>
{
    let mut lines = d.lines;
    let rust = d.old && !d.link && file.ends_with(".rs") && !is_test_path(&file);
    if let (Some(old), true) = (old, rust) {
        let was = res!(RustSrc::new(old, &file, &rules.test_attrs)).tests;
        let now = res!(RustSrc::new(&src, &file, &rules.test_attrs)).tests;
        for n in 1..=src.split('\n').count() {
            if lines.contains(&n) || now.contains(&n) {
                continue;
            }
            // An unchanged line moves by what each hunk above it removes less what it adds.
            let o = d.hunks.iter().filter(|h| n >= h.new_start + h.new_count.max(1))
                .fold(n as isize, |o, h| o + h.old_count as isize - h.new_count as isize);
            if o >= 1 && was.contains(&(o as usize)) {
                lines.insert(n);
            }
        }
    }
    Ok(Added { file, src, lines, link: d.link })
}

/// Checks the added lines of each file against the rules that are on.
pub fn check(added: &[Added], rules: &Rules) -> Outcome<Report> {
    let mut rep = Report::default();
    for a in added {
        res!(check_one(a, rules, &mut rep));
    }
    rep.breaches.sort_by(|x, y| (&x.file, x.line, &x.rule).cmp(&(&y.file, y.line, &y.rule)));
    rep.breaches.dedup();
    Ok(rep)
}

fn check_one(a: &Added, rules: &Rules, rep: &mut Report) -> Outcome<()> {
    if a.lines.is_empty() {
        return Ok(());
    }
    let text: Vec<&str> = a.src.split('\n').collect();
    let mut allowed: BTreeMap<usize, Allowed> = BTreeMap::new();
    for n in &a.lines {
        if let Some(l) = text.get(n.wrapping_sub(1)) {
            if allow_marked(l) {
                allowed.insert(*n, Allowed { file: a.file.clone(), line: *n, text: l.to_string(),
                    rules: Vec::new() });
            }
        }
    }
    let rust = match a.file.ends_with(".rs") && !a.link {
        true    => Some(res!(RustSrc::new(&a.src, &a.file, &rules.test_attrs))),
        false   => None,
    };
    let test_file = is_test_path(&a.file);
    for r in rules.rules.iter().filter(|r| r.on) {
        let lines = match (&r.check, &rust) {
            (Check::Credential, _) => match secret::skip_path(a.file.as_bytes()) {
                true    => Vec::new(),
                false   => secret::scan(a.src.as_bytes()).iter().map(|f| f.line).collect(),
            },
            (Check::Link, _) => match a.link {
                true    => vec![1],
                false   => Vec::new(),
            },
            (Check::CfgTest, _) => cfg_test_lines(&a.src),
            (Check::TestPath, None) => match a.file == "Cargo.toml" || a.file.ends_with("/Cargo.toml") { // fixed: cargo's manifest name
                true    => cargo_test_paths(&a.src),
                false   => Vec::new(),
            },
            (_, Some(rs))   => res!(rs.find(&r.check, rules, &a.lines)),
            (_, None)       => Vec::new(),
        };
        for line in lines {
            if !a.lines.contains(&line) {
                continue;
            }
            let in_test = test_file || rust.as_ref().map(|rs| rs.tests.contains(&line)).unwrap_or(false);
            if in_test && !r.in_tests {
                continue;
            }
            if r.fixed_ok && rust.as_ref().map(|rs| rs.fixed.contains(&line)).unwrap_or(false) {
                continue;
            }
            if let Some(al) = allowed.get_mut(&line) {
                if !al.rules.contains(&r.id) {
                    al.rules.push(r.id.clone());
                }
                continue;
            }
            rep.breaches.push(Breach {
                file:   a.file.clone(),
                line,
                rule:   r.id.clone(),
                msg:    r.msg.clone(),
            });
        }
    }
    // A line that only names the escape breaks no rule, so is not one.
    rep.allows.extend(allowed.into_values().filter(|al| !al.rules.is_empty()));
    Ok(())
}

/// Does the line carry `allowlist hematite`, with a space, `_` or `-` between, in any case?
fn allow_marked(line: &str) -> bool {
    line.to_lowercase().replace(['_', '-'], " ").contains("allowlist hematite")
}

/// Is the file under a `tests` directory with no `src` above or below it?  A `src` below is a
/// crate's, built as product wherever it is placed.
fn is_test_path(file: &str) -> bool {
    let mut dirs = file.split('/').collect::<Vec<_>>();
    dirs.pop();
    dirs.contains(&"tests") && !dirs.contains(&"src")
}

/// The lines that set the cfg `test` for a build: `rustc-cfg` (a build script's `cargo:` line)
/// or `--cfg` followed by the word `test`, or a `rustc-cfg` whose value is formatted at run time.
fn cfg_test_lines(src: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for (n, l) in src.split('\n').enumerate() {
        let words: Vec<&str> = l.split(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | ':')))
            .filter(|w| !w.is_empty()).collect();
        let sets = words.windows(2).any(|w| (w[0].ends_with("rustc-cfg") || w[0] == "--cfg")
            && w[1] == "test");
        let formatted = l.match_indices("rustc-cfg").any(|(at, k)| l[at + k.len()..].starts_with("={"));
        if sets || formatted {
            out.push(n + 1);
        }
    }
    out
}

/// Does a path name a `tests` directory?
fn into_tests(path: &str) -> bool {
    path.split(['/', '\\']).any(|c| c == "tests")
}

/// The lines of a Cargo manifest whose `path` or `build` value leads into a `tests` directory,
/// outside a `[[test]]` target, so building product code from test files.
fn cargo_test_paths(src: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut test_target = false;
    for (n, l) in src.split('\n').enumerate() {
        let t = l.trim();
        if t.starts_with('[') {
            test_target = t.replace(' ', "") == "[[test]]";
            continue;
        }
        if test_target || t.starts_with('#') {
            continue;
        }
        let hit = ["path", "build"].iter().any(|k| l.match_indices(k).any(|(at, _)| {
            let lead = l[..at].chars().last();
            let word = !matches!(lead, Some(c) if c.is_alphanumeric() || c == '_' || c == '-');
            let rest = l[at + k.len()..].trim_start();
            word && rest.strip_prefix('=').map(|v| {
                let v = v.trim_start();
                let q = v.chars().next().unwrap_or(' ');
                matches!(q, '"' | '\'') && v[1..].split(q).next().map(into_tests).unwrap_or(false)
            }).unwrap_or(false)
        }));
        if hit {
            out.push(n + 1);
        }
    }
    out
}

/// One Rust source, lexed once for every rule.
struct RustSrc<'a> {
    src:    &'a str,
    toks:   Vec<Token>,
    lines:  LineIndex,
    code:   Vec<usize>,         // indices of the tokens that are code, not docs or attributes
    attrs:  Vec<(usize, Vec<Token>)>, // each attribute's inner tokens, by its byte offset
    macros: Vec<bool>,          // inside a `macro_rules!` body
    tests:  BTreeSet<usize>,    // lines inside a test item
    fixed:  BTreeSet<usize>,    // lines covered by a `// fixed:` marker
    test_attrs: &'a [String],   // `Rules::test_attrs`
}

impl<'a> RustSrc<'a> {
    fn new(src: &'a str, file: &str, test_attrs: &'a [String]) -> Outcome<Self> {
        let toks = res!(lex(src, &rust_tokens()));
        let lines = LineIndex::new(src);
        let code = (0..toks.len()).filter(|i| !matches!(toks[*i].kind,
            TokenKind::DocComment(_) | TokenKind::Attribute | TokenKind::Eof)).collect();
        // An attribute the lexer cannot read on its own holds no code tokens to match.
        let attrs = toks.iter().filter(|t| t.kind == TokenKind::Attribute)
            .map(|t| (t.span.start + 2, lex(t.text.get(2..).unwrap_or(""), &rust_tokens())
                .map(|v| v.into_iter().filter(|x| x.kind != TokenKind::Eof).collect())
                .unwrap_or_default()))
            .collect();
        let mut thin = Vec::new();
        let fixed = fixed_lines(src, &toks, &lines, file, &mut thin);
        let mut macros = vec![false; toks.len()];
        let mut tests = BTreeSet::new();
        let mut i = 0;
        while i < toks.len() {
            let t = &toks[i];
            if is_ident(t, "macro_rules") && i + 3 < toks.len() && is_punct(&toks[i + 1], '!') {
                let end = group_end(&toks, i + 3);
                for m in macros.iter_mut().take(end).skip(i) {
                    *m = true;
                }
                i = end.max(i + 1);
                continue;
            }
            if t.kind == TokenKind::Attribute {
                let a: String = t.text.chars().filter(|c| !c.is_whitespace()).collect();
                if mutable::is_test_attr(&a, test_attrs) {
                    let end = item_end(&toks, i + 1).min(toks.len());
                    if end > i {
                        let first = lines.line_of(t.span.start);
                        let last = lines.line_of(toks[end - 1].span.end.saturating_sub(1));
                        tests.extend(first..=last);
                    }
                    i = end.max(i + 1);
                    continue;
                }
            }
            i += 1;
        }
        Ok(Self { src, toks, lines, code, attrs, macros, tests, fixed, test_attrs })
    }

    fn line(&self, i: usize) -> usize {
        self.lines.line_of(self.toks[i].span.start)
    }

    /// The lines where `check` is breached; `added` lets a multi-line breach name its added line.
    fn find(&self, check: &Check, rules: &Rules, added: &BTreeSet<usize>) -> Outcome<Vec<usize>> {
        Ok(match check {
            Check::Seq(seq)             => self.seq(seq),
            Check::Question             => self.question(),
            Check::OkArgs               => self.ok_args(),
            Check::RawLock              => self.raw_lock(),
            Check::DocMember            => self.doc_member(added),
            Check::DocHeading(words)    => self.doc_heading(words),
            Check::ImportOrder          => self.import_order(&rules.local_crates, added),
            Check::CommentWords(words)  => self.comment_words(words),
            Check::Placement            => self.placement(),
            Check::TestAlias            => self.test_alias(),
            Check::TestPath             => self.placed().into_iter()
                .filter(|(_, lit)| lit.as_deref().map(into_tests).unwrap_or(false))
                .map(|(line, _)| line).collect(),
            Check::Mutable              => {
                let scan = res!(mutable::scan(self.src, Lang::Rust, "", self.test_attrs));
                scan.hits.iter().map(|h| h.line)
                    .chain(scan.bad_fixed.iter().map(|m| m.line)).collect()
            }
            Check::Credential | Check::Link | Check::CfgTest => Vec::new(),
        })
    }

    fn seq(&self, seq: &[String]) -> Vec<usize> {
        let mut out = Vec::new();
        if seq.len() > self.code.len() {
            return out;
        }
        for w in self.code.windows(seq.len()) {
            if w.iter().zip(seq).all(|(i, s)| self.toks[*i].text == *s) {
                out.push(self.line(w[0]));
            }
        }
        // An attribute can hold code: `#[unsafe(no_mangle)]`.
        for (at, toks) in &self.attrs {
            for w in toks.windows(seq.len()) {
                if w.iter().zip(seq).all(|(t, s)| t.text == *s) {
                    out.push(self.lines.line_of(at + w[0].span.start));
                }
            }
        }
        out
    }

    fn question(&self) -> Vec<usize> {
        let mut out = Vec::new();
        for (k, i) in self.code.iter().enumerate() {
            if !is_punct(&self.toks[*i], '?') || k == 0 {
                continue;
            }
            let prev = &self.toks[self.code[k - 1]];
            if self.macros[*i] && is_punct(prev, ')') && self.repetition(k - 1) {
                continue;
            }
            let after_expr = match &prev.kind {
                TokenKind::Punct(')') | TokenKind::Punct(']') | TokenKind::Punct('}') => true,
                TokenKind::Ident                                                     => true,
                // A tuple field, `p.0?`.
                TokenKind::Number       => k >= 2 && is_punct(&self.toks[self.code[k - 2]], '.'),
                TokenKind::Keyword(w)   => w == "await" || w == "self",
                _                       => false,
            };
            let bound = self.code.get(k + 1)
                .map(|n| self.toks[*n].kind == TokenKind::Ident).unwrap_or(false);
            if after_expr && !bound {
                out.push(self.line(*i));
            }
        }
        out
    }

    /// Rust the gate would not read: a `path` attribute or an `include!` naming anything but a
    /// literal ending `.rs`.
    fn placement(&self) -> Vec<usize> {
        self.placed().into_iter()
            .filter(|(_, lit)| !lit.as_deref().map(|p| p.ends_with(".rs")).unwrap_or(false))
            .map(|(line, _)| line).collect()
    }

    /// Each `path` attribute and `include!` with its literal, `None` when it names no literal the
    /// lexer can close.
    fn placed(&self) -> Vec<(usize, Option<String>)> {
        let mut out = Vec::new();
        for (i, t) in self.toks.iter().enumerate() {
            if t.kind != TokenKind::Attribute {
                continue;
            }
            let a: String = t.text.chars().filter(|c| !c.is_whitespace()).collect();
            for (at, _) in a.match_indices("path=") {
                if matches!(a[..at].chars().last(), Some('[') | Some('(') | Some(',')) {
                    let lit = a[at + 5..].strip_prefix('"')
                        .and_then(|r| r.split_once('"'))
                        .map(|(name, _)| name.to_string());
                    out.push((self.line(i), lit));
                }
            }
        }
        for (k, i) in self.code.iter().enumerate() {
            if !is_ident(&self.toks[*i], "include")
                || !self.code.get(k + 1).map(|n| is_punct(&self.toks[*n], '!')).unwrap_or(false)
            {
                continue;
            }
            let lit = match (self.code.get(k + 3).map(|n| &self.toks[*n]),
                self.code.get(k + 4).map(|n| &self.toks[*n]))
            {
                (Some(lit), Some(close)) if lit.kind == TokenKind::StringLit
                    && lit.text.len() >= 2 && lit.text.starts_with('"') && lit.text.ends_with('"')
                    && matches!(close.kind, TokenKind::Punct(')') | TokenKind::Punct(']')
                        | TokenKind::Punct('}') | TokenKind::Punct(',')) =>
                    Some(lit.text[1..lit.text.len() - 1].to_string()),
                _ => None,
            };
            out.push((self.line(*i), lit));
        }
        out
    }

    /// `as <name>` or, for a path of two segments or more, `mod <name>`, where `<name>` begins a
    /// test attribute's path: after `use tokio::main as test;`, `#[test]` is not a test.
    fn test_alias(&self) -> Vec<usize> {
        let mut out = Vec::new();
        for w in self.code.windows(2) {
            let (k, n) = (&self.toks[w[0]], &self.toks[w[1]]);
            let hit = self.test_attrs.iter().any(|a| {
                let mut segs = a.split("::");
                let first = segs.next().unwrap_or("");
                n.text == first && (k.text == "as" || (k.text == "mod" && segs.next().is_some()))
            });
            if hit {
                out.push(self.line(w[0]));
            }
        }
        out
    }

    /// Does the `)` at code index `close` end a `$( … )` macro repetition?
    fn repetition(&self, close: usize) -> bool {
        let mut depth = 0isize;
        for k in (0..=close).rev() {
            match self.toks[self.code[k]].kind {
                TokenKind::Punct(')') | TokenKind::Punct(']') | TokenKind::Punct('}') => depth += 1,
                TokenKind::Punct('(') | TokenKind::Punct('[') | TokenKind::Punct('{') => {
                    depth -= 1;
                    if depth == 0 {
                        return k > 0 && is_punct(&self.toks[self.code[k - 1]], '$');
                    }
                }
                _ => {}
            }
        }
        false
    }

    fn ok_args(&self) -> Vec<usize> {
        let mut out = Vec::new();
        for (k, i) in self.code.iter().enumerate() {
            if !is_ident(&self.toks[*i], "ok") || k + 2 >= self.code.len()
                || !is_punct(&self.toks[self.code[k + 1]], '!')
            {
                continue;
            }
            let mut depth = 0isize;
            for (n, j) in self.code[k + 2..].iter().enumerate() {
                match self.toks[*j].kind {
                    TokenKind::Punct('(') | TokenKind::Punct('[') | TokenKind::Punct('{') => depth += 1,
                    TokenKind::Punct(')') | TokenKind::Punct(']') | TokenKind::Punct('}') => depth -= 1,
                    TokenKind::Punct(',') if depth == 1 => {
                        // A trailing comma is still one argument.
                        let next = self.code.get(k + 3 + n).map(|x| &self.toks[*x].kind);
                        if !matches!(next, Some(TokenKind::Punct(')')) | Some(TokenKind::Punct(']'))
                            | Some(TokenKind::Punct('}')))
                        {
                            out.push(self.line(*i));
                            break;
                        }
                    }
                    _ => {}
                }
                if depth <= 0 {
                    break;
                }
            }
        }
        out
    }

    fn raw_lock(&self) -> Vec<usize> {
        let mut out = Vec::new();
        for (k, i) in self.code.iter().enumerate() {
            let t = &self.toks[*i];
            if self.macros[*i] || k == 0 || k + 2 >= self.code.len()
                || !(is_ident(t, "read") || is_ident(t, "write") || is_ident(t, "lock"))
                || !is_punct(&self.toks[self.code[k - 1]], '.')
                || !is_punct(&self.toks[self.code[k + 1]], '(')
                || !is_punct(&self.toks[self.code[k + 2]], ')')
            {
                continue;
            }
            // An async lock (tokio's), which the lock macros do not take.
            let awaited = self.code.get(k + 3).map(|n| is_punct(&self.toks[*n], '.')).unwrap_or(false)
                && self.code.get(k + 4).map(|n| is_keyword(&self.toks[*n], "await")).unwrap_or(false);
            let io = k >= 4
                && is_punct(&self.toks[self.code[k - 2]], ')')
                && is_punct(&self.toks[self.code[k - 3]], '(')
                && ["stdin", "stdout", "stderr"].iter()
                    .any(|n| is_ident(&self.toks[self.code[k - 4]], n));
            if !io && !awaited {
                out.push(self.line(*i));
            }
        }
        out
    }

    /// The first added line of each `///` run on a field, variant or constant.
    fn doc_member(&self, added: &BTreeSet<usize>) -> Vec<usize> {
        let toks = &self.toks;
        // The braces that open a struct, union or enum body, or an enum variant's fields.
        let mut members = BTreeSet::new();
        let mut enums = BTreeSet::new();
        let mut i = 0;
        while i < toks.len() {
            let t = &toks[i];
            let is_enum = is_keyword(t, "enum");
            if (is_enum || is_keyword(t, "struct") || is_ident(t, "union"))
                && toks.get(i + 1).map(|n| n.kind == TokenKind::Ident).unwrap_or(false)
            {
                let mut j = i + 2;
                // Generics and a `where` clause may hold `Fn(..)`, which is not a tuple body.
                if j < toks.len() && is_punct(&toks[j], '<') {
                    let mut depth = 0isize;
                    while j < toks.len() {
                        depth += match &toks[j].kind {
                            TokenKind::Punct('<')                   => 1,
                            TokenKind::Punct('>')                   => -1,
                            TokenKind::Operator(o) if o == "<<"     => 2,
                            TokenKind::Operator(o) if o == ">>"     => -2,
                            _                                       => 0,
                        };
                        j += 1;
                        if depth <= 0 {
                            break;
                        }
                    }
                }
                let body: fn(&Token) -> bool = match toks.get(j).map(|t| is_keyword(t, "where")).unwrap_or(false) {
                    true    => |t: &Token| matches!(t.kind, TokenKind::Punct('{') | TokenKind::Punct(';')),
                    false   => |t: &Token| matches!(t.kind,
                        TokenKind::Punct('{') | TokenKind::Punct(';') | TokenKind::Punct('(')),
                };
                while j < toks.len() && !body(&toks[j]) {
                    j += 1;
                }
                if j < toks.len() && is_punct(&toks[j], '{') {
                    members.insert(j);
                    if is_enum {
                        enums.insert(j);
                    }
                }
            }
            i += 1;
        }
        let mut out = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        let mut i = 0;
        while i < toks.len() {
            let in_members = stack.last().map(|o| members.contains(o)).unwrap_or(false);
            if in_members || self.const_at(i) {
                if let Some(l) = self.block_doc_lines(i).into_iter().find(|l| added.contains(l)) {
                    out.push(l);
                }
            }
            match &toks[i].kind {
                TokenKind::Punct('{') => {
                    if stack.last().map(|o| enums.contains(o)).unwrap_or(false)
                        && i > 0 && toks[i - 1].kind == TokenKind::Ident
                    {
                        members.insert(i);
                    }
                    stack.push(i);
                }
                TokenKind::Punct('}') => {
                    stack.pop();
                }
                TokenKind::DocComment(d) if d.starts_with("///") => {
                    let mut end = i;
                    while end < toks.len() && matches!(&toks[end].kind,
                        TokenKind::DocComment(d) if d.starts_with("///"))
                    {
                        end += 1;
                    }
                    let in_members = stack.last().map(|o| members.contains(o)).unwrap_or(false);
                    if in_members || self.const_at(end) {
                        if let Some(l) = (i..end).map(|n| self.line(n)).find(|l| added.contains(l)) {
                            out.push(l);
                        }
                    }
                    i = end;
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
        out
    }

    /// The lines of each `/** … */` doc comment in the trivia before token `i`, which the lexer
    /// keeps as a block comment.
    fn block_doc_lines(&self, i: usize) -> Vec<usize> {
        let t = &self.toks[i];
        let mut end = self.line(i);
        let mut out = Vec::new();
        for tr in t.leading_trivia.iter().rev() {
            match tr {
                Trivia::Newline         => end = end.saturating_sub(1),
                Trivia::BlockComment(c) => {
                    let start = end.saturating_sub(c.matches('\n').count());
                    if c.starts_with("/**") && !c.starts_with("/***") && c != "/**/" {
                        out.extend(start..=end);
                    }
                    end = start;
                }
                _ => {}
            }
        }
        out.sort();
        out
    }

    /// Does a `const` item, not a `const fn`, start at `i`, past attributes and visibility?
    fn const_at(&self, mut i: usize) -> bool {
        let toks = &self.toks;
        while i < toks.len() && toks[i].kind == TokenKind::Attribute {
            i += 1;
        }
        if i < toks.len() && is_keyword(&toks[i], "pub") {
            i += 1;
            if i < toks.len() && is_punct(&toks[i], '(') {
                i = group_end(toks, i);
            }
        }
        i + 1 < toks.len() && is_keyword(&toks[i], "const")
            && !matches!(&toks[i + 1].kind, TokenKind::Keyword(k)
                if k == "fn" || k == "unsafe" || k == "async" || k == "extern")
    }

    fn doc_heading(&self, words: &[String]) -> Vec<usize> {
        let mut out = Vec::new();
        for (i, t) in self.toks.iter().enumerate() {
            if let TokenKind::DocComment(d) = &t.kind {
                let body = d.get(3..).unwrap_or("").trim_start();
                if let Some(h) = body.strip_prefix('#') {
                    let word = h.trim_start_matches('#').split_whitespace().next().unwrap_or("")
                        .trim_end_matches(|c: char| !c.is_alphanumeric());
                    if words.iter().any(|w| w == word) {
                        out.push(self.line(i));
                    }
                }
            }
        }
        out
    }

    fn import_order(&self, local: &[String], added: &BTreeSet<usize>) -> Vec<usize> {
        let toks = &self.toks;
        let mut out = Vec::new();
        // (class, line) of the highest group in the block so far, and where the block continues.
        let mut top: Option<(u8, usize)> = None;
        let mut next_use: Option<usize> = None;
        let mut last_line = 0usize;
        for (k, i) in self.code.iter().enumerate() {
            if !is_keyword(&toks[*i], "use") {
                continue;
            }
            let prev = match k {
                0 => None,
                _ => Some(&toks[self.code[k - 1]]),
            };
            if prev.map(|p| is_keyword(p, "pub") || is_punct(p, ')')).unwrap_or(false) {
                continue;
            }
            let line = self.line(*i);
            if next_use != Some(*i) {
                top = None;
            } else if line == last_line {
                out.push(line);
            }
            let class = self.use_class(*i, local);
            match top {
                Some((c, l)) if class < c => {
                    if added.contains(&line) {
                        out.push(line);
                    } else if added.contains(&l) {
                        out.push(l);
                    }
                }
                Some((c, _)) if class == c => {}
                _ => top = Some((class, line)),
            }
            let mut j = item_end(toks, *i);
            while j < toks.len() && matches!(toks[j].kind,
                TokenKind::Attribute | TokenKind::DocComment(_))
            {
                j += 1;
            }
            next_use = Some(j);
            last_line = self.lines.line_of(toks[j.min(toks.len()) - 1].span.start);
        }
        out
    }

    /// 0 local, 1 local library, 2 std, 3 third party, by the first segment of a `use` path.
    fn use_class(&self, i: usize, local: &[String]) -> u8 {
        let mut j = i + 1;
        while j < self.toks.len() && self.toks[j].kind == TokenKind::Operator(fmt!("::")) {
            j += 1;
        }
        let first = match self.toks.get(j) {
            Some(t) => t.text.as_str(),
            None    => "",
        };
        match first {
            "crate" | "self" | "super"                          => 0,
            _ if local.iter().any(|p| first.starts_with(p.as_str())) => 1,
            "std" | "core" | "alloc"                            => 2,
            _                                                   => 3,
        }
    }

    fn comment_words(&self, words: &[String]) -> Vec<usize> {
        let mut out = Vec::new();
        let mut from = 0usize;
        for t in &self.toks {
            let gap = self.src.get(from..t.span.start).unwrap_or("");
            self.gap_comments(gap, from, words, &mut out);
            if let TokenKind::DocComment(d) = &t.kind {
                if has_word(d, words) {
                    out.push(self.lines.line_of(t.span.start));
                }
            }
            from = t.span.end.max(from);
        }
        out
    }

    /// The lines of comments in `gap` (whitespace and comments only, at byte `at`) holding a word.
    fn gap_comments(&self, gap: &str, at: usize, words: &[String], out: &mut Vec<usize>) {
        let mut rest = gap;
        let mut pos = at;
        while let Some(s) = rest.find('/') {
            let after = &rest[s..];
            let len = match (after.starts_with("//"), after.starts_with("/*")) {
                (true, _)   => after.find('\n').unwrap_or(after.len()),
                (_, true)   => after.find("*/").map(|e| e + 2).unwrap_or(after.len()),
                _           => 1,
            };
            if len > 1 {
                let mut line_at = pos + s;
                for piece in after[..len].split('\n') {
                    if has_word(piece, words) {
                        out.push(self.lines.line_of(line_at));
                    }
                    line_at += piece.len() + 1;
                }
            }
            pos += s + len;
            rest = &after[len..];
        }
    }
}

/// Does `text`, outside backticks, hold a word starting with one of `words`, case aside?  A
/// capitalised word within a sentence is a name (serde's `Serialize`), not a spelling.
fn has_word(text: &str, words: &[String]) -> bool {
    let plain = text.split('`').step_by(2).collect::<Vec<_>>().join(" ");
    let mut start = true; // at the start of a sentence
    let mut word = String::new();
    for c in plain.chars().chain(std::iter::once(' ')) {
        if c.is_alphanumeric() {
            word.push(c);
            continue;
        }
        if !word.is_empty() {
            let name = !start && word.chars().next().map(char::is_uppercase).unwrap_or(false);
            let low = word.to_lowercase();
            if !name && words.iter().any(|p| low.starts_with(p.to_lowercase().as_str())) {
                return true;
            }
            start = false;
            word.clear();
        }
        if matches!(c, '.' | '!' | '?' | ':') {
            start = true;
        }
    }
    false
}
