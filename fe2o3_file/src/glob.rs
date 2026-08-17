//! Glob matching over byte paths, in the shapes git's ignore files use.
//!
//! A [`Glob`] is one compiled pattern and an [`IgnoreFile`] is an ordered list
//! of them, parsed from the byte content of a `.gitignore`-shaped file. The
//! semantics are git's:
//!
//! - `*` matches any run of bytes within one path component, `?` matches one
//!   byte within a component, and a character class `[a-z]` (negated with a
//!   leading `!` or `^`) matches one byte. None of these ever matches a `/`.
//! - A component that is exactly `**` spans any number of components: a leading
//!   `**/` matches at any depth, a trailing `/**` matches everything inside a
//!   directory, and `a/**/b` matches zero or more directories between. A `**`
//!   sitting among other bytes in a component is an ordinary `*`.
//! - A pattern containing a `/` anywhere but its end is anchored to the
//!   directory the ignore file sits in; one without matches at any depth. A
//!   leading `/` anchors without contributing a component.
//! - A trailing `/` makes the pattern match directories only.
//! - A leading `!` negates: a path the pattern matches is re-included. The
//!   *last* matching pattern in an [`IgnoreFile`] decides.
//! - `\` escapes the byte after it, so `\*` is a literal asterisk and `\!` at
//!   the start of a line is a literal exclamation mark.
//! - In a file, blank lines are nothing and lines beginning `#` are comments.
//!   Trailing spaces are trimmed unless escaped with `\`.
//!
//! # Paths are bytes
//!
//! A path here is a `&[u8]` of components joined by `/`, relative to the
//! directory the rules speak from, with no leading or trailing slash. Matching
//! is byte-for-byte: a pattern applies to a path that is not UTF-8 exactly as
//! it applies to one that is, with `?` and `[a-z]` consuming one *byte*, not
//! one character. That is the only footing on which a rule can treat every
//! legal path, since a filesystem name need not be text.
//!
//! Two shapes are deliberately absent: POSIX named classes (`[[:alpha:]]`) are
//! not recognised, and a `/` cannot appear inside a character class or be
//! escaped, because the pattern is split on every `/` before anything else is
//! read. An unclosed `[` is a literal `[`, as it is a match failure in git.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;


/// One token of a compiled pattern, within a single path component.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Lit(u8),    // one literal byte
    Any,        // `?`: exactly one byte
    Star,       // `*`: any run of bytes, including none
    // `[...]`: one byte drawn from, or kept out of, a set of ranges.
    Class {
        negated:    bool,             // the class began `[!` or `[^`
        ranges:     Vec<(u8, u8)>,    // inclusive; a lone member is a range of one
    },
}

/// One component of a compiled pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Comp {
    Globstar,       // a component that was exactly `**`: any number of path components
    One(Vec<Tok>),  // an ordinary component, matched against exactly one path component
}

/// One compiled ignore pattern.
///
/// Compile with [`Glob::new`], ask with [`Glob::matches`]. A `Glob` carries its
/// own negation flag but does not apply it: negation only means something in
/// the ordered context of an [`IgnoreFile`], which is where it is read.
#[derive(Clone, Debug)]
pub struct Glob {
    negated:    bool,       // the pattern began with `!`
    dir_only:   bool,       // the pattern ended with `/`, so directories only
    // A leading Comp::Globstar stands in for an unanchored pattern's freedom to
    // match at any depth.
    comps:      Vec<Comp>,
}

impl Glob {

    /// Compiles one pattern, as written on one line of an ignore file.
    ///
    /// The line must already be free of comments and trailing unescaped spaces,
    /// which are the file's business rather than the pattern's; see
    /// [`IgnoreFile::parse`]. A line that is empty, or empty once its `!` and
    /// `/` dressing is removed, is an error, since it can match nothing.
    pub fn new(line: &[u8]) -> Outcome<Self> {
        let mut rest = line;
        let mut negated = false;
        if rest.first() == Some(&b'!') {
            negated = true;
            rest = &rest[1..];
        }
        let mut dir_only = false;
        if rest.last() == Some(&b'/') {
            dir_only = true;
            rest = &rest[..rest.len() - 1];
        }
        // A leading slash anchors and says nothing else. Anchoring is otherwise
        // decided by whether a slash survives inside the pattern.
        let anchored = if rest.first() == Some(&b'/') {
            rest = &rest[1..];
            true
        } else {
            rest.contains(&b'/')
        };
        if rest.is_empty() {
            return Err(err!(
                "The pattern {:?} has no content to match a path against.",
                String::from_utf8_lossy(line);
            Invalid, Input));
        }
        let mut comps = Vec::new();
        if !anchored {
            comps.push(Comp::Globstar);
        }
        for part in rest.split(|b| *b == b'/') {
            if part == b"**" {
                comps.push(Comp::Globstar);
            } else {
                comps.push(Comp::One(Self::tokens(part)));
            }
        }
        Ok(Self { negated, dir_only, comps })
    }

    pub fn is_negated(&self) -> bool {
        self.negated
    }

    pub fn is_dir_only(&self) -> bool {
        self.dir_only
    }

    /// Reports whether a path matches, where `is_dir` says what the path names.
    ///
    /// The path is relative, `/`-joined, with no leading or trailing slash. A
    /// directory-only pattern refuses anything that is not a directory; it does
    /// not by itself speak for the files beneath, which is the business of
    /// [`IgnoreFile::excludes`].
    pub fn matches(&self, path: &[u8], is_dir: bool) -> bool {
        if self.dir_only && !is_dir {
            return false;
        }
        if path.is_empty() {
            return false;
        }
        let parts: Vec<&[u8]> = path.split(|b| *b == b'/').collect();
        Self::match_comps(&self.comps, &parts)
    }

    fn match_comps(comps: &[Comp], parts: &[&[u8]]) -> bool {
        match comps.first() {
            None => parts.is_empty(),
            Some(Comp::Globstar) => {
                if comps.len() == 1 {
                    // A trailing `**` names what is *inside*: it must consume
                    // at least one component, so `a/**` matches `a/b` and not
                    // `a` itself.
                    return !parts.is_empty();
                }
                for i in 0..=parts.len() {
                    if Self::match_comps(&comps[1..], &parts[i..]) {
                        return true;
                    }
                }
                false
            },
            Some(Comp::One(toks)) => match parts.first() {
                None		=> false,
                Some(part)	=> Self::match_toks(toks, part)
                    && Self::match_comps(&comps[1..], &parts[1..]),
            },
        }
    }

    fn match_toks(toks: &[Tok], s: &[u8]) -> bool {
        match toks.first() {
            None => s.is_empty(),
            Some(Tok::Star) => {
                for i in 0..=s.len() {
                    if Self::match_toks(&toks[1..], &s[i..]) {
                        return true;
                    }
                }
                false
            },
            Some(Tok::Any) => !s.is_empty()
                && Self::match_toks(&toks[1..], &s[1..]),
            Some(Tok::Lit(b)) => s.first() == Some(b)
                && Self::match_toks(&toks[1..], &s[1..]),
            Some(Tok::Class { negated, ranges }) => match s.first() {
                None => false,
                Some(c) => {
                    let inside = ranges.iter().any(|(lo, hi)| lo <= c && c <= hi);
                    inside != *negated && Self::match_toks(&toks[1..], &s[1..])
                },
            },
        }
    }

    fn tokens(part: &[u8]) -> Vec<Tok> {
        let mut toks = Vec::new();
        let mut i = 0;
        while i < part.len() {
            match part[i] {
                b'\\' if i + 1 < part.len() => {
                    toks.push(Tok::Lit(part[i + 1]));
                    i += 2;
                },
                b'*' => {
                    // A run of asterisks that is not a whole component is one
                    // ordinary star, as it is in git.
                    if toks.last() != Some(&Tok::Star) {
                        toks.push(Tok::Star);
                    }
                    i += 1;
                },
                b'?' => {
                    toks.push(Tok::Any);
                    i += 1;
                },
                b'[' => match Self::class(&part[i..]) {
                    Some((tok, used)) => {
                        toks.push(tok);
                        i += used;
                    },
                    // An unclosed class is a literal bracket.
                    None => {
                        toks.push(Tok::Lit(b'['));
                        i += 1;
                    },
                },
                b => {
                    toks.push(Tok::Lit(b));
                    i += 1;
                },
            }
        }
        toks
    }

    /// The `usize` is how many bytes the class took; nothing where it never
    /// closes.
    fn class(s: &[u8]) -> Option<(Tok, usize)> {
        let mut i = 1; // Past the opening bracket.
        let mut negated = false;
        if i < s.len() && (s[i] == b'!' || s[i] == b'^') {
            negated = true;
            i += 1;
        }
        let mut ranges = Vec::new();
        let mut first = true;
        while i < s.len() {
            let b = s[i];
            // A closing bracket as the very first member is a literal.
            if b == b']' && !first {
                return Some((Tok::Class { negated, ranges }, i + 1));
            }
            first = false;
            let lo = if b == b'\\' && i + 1 < s.len() {
                i += 1;
                s[i]
            } else {
                b
            };
            // A dash with a member on each side is a range; anywhere else it is
            // itself.
            if i + 2 < s.len() && s[i + 1] == b'-' && s[i + 2] != b']' {
                let hb = s[i + 2];
                let hi = if hb == b'\\' && i + 3 < s.len() {
                    i += 1;
                    s[i + 2]
                } else {
                    hb
                };
                ranges.push((lo, hi));
                i += 3;
            } else {
                ranges.push((lo, lo));
                i += 1;
            }
        }
        None
    }
}


/// An ordered list of patterns, as a `.gitignore`-shaped file holds them.
///
/// Parse with [`IgnoreFile::parse`]. The questions it answers are
/// [`IgnoreFile::ignores`], which applies last-match-wins to one path, and
/// [`IgnoreFile::excludes`], which also holds a path to git's rule that nothing
/// inside an ignored directory can be re-included.
#[derive(Clone, Debug, Default)]
pub struct IgnoreFile {
    rules: Vec<Glob>,   // in the order the file gives them
}

impl IgnoreFile {

    /// Parses the byte content of an ignore file.
    ///
    /// Lines are split on `\n`, with a trailing `\r` dropped so a file written
    /// on Windows reads the same. Blank lines and lines beginning `#` say
    /// nothing; `\#` begins a pattern with a literal hash. Trailing spaces are
    /// trimmed unless the space is escaped with `\`. A line no rule can be
    /// compiled from is passed over, as git passes over a pattern it cannot
    /// read.
    pub fn parse(bytes: &[u8]) -> Self {
        let mut rules = Vec::new();
        for raw in bytes.split(|b| *b == b'\n') {
            let mut line = raw;
            if line.last() == Some(&b'\r') {
                line = &line[..line.len() - 1];
            }
            if line.is_empty() || line.first() == Some(&b'#') {
                continue;
            }
            line = Self::trim_trailing_spaces(line);
            if line.is_empty() {
                continue;
            }
            if let Ok(glob) = Glob::new(line) {
                rules.push(glob);
            }
        }
        Self { rules }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Returns what the file says about a path, if it says anything.
    ///
    /// The last pattern that matches decides: `Some(true)` means ignored,
    /// `Some(false)` means a `!` pattern re-included it, and `None` means no
    /// pattern spoke.
    pub fn decides(&self, path: &[u8], is_dir: bool) -> Option<bool> {
        let mut decision = None;
        for rule in &self.rules {
            if rule.matches(path, is_dir) {
                decision = Some(!rule.negated);
            }
        }
        decision
    }

    /// Reports whether the path itself is ignored, last match winning.
    ///
    /// This asks about the path alone. To also honour an ignored directory
    /// swallowing everything beneath it, ask [`IgnoreFile::excludes`].
    pub fn ignores(&self, path: &[u8], is_dir: bool) -> bool {
        self.decides(path, is_dir).unwrap_or(false)
    }

    /// Reports whether the path is excluded, counting its ancestors.
    ///
    /// A path inside an ignored directory is excluded no matter what a `!`
    /// pattern says about the path itself, because git does not descend into a
    /// directory it has ignored. Each ancestor is judged in its own right,
    /// last match winning, before the path is.
    pub fn excludes(&self, path: &[u8], is_dir: bool) -> bool {
        for (i, b) in path.iter().enumerate() {
            if *b == b'/' && self.ignores(&path[..i], true) {
                return true;
            }
        }
        self.ignores(path, is_dir)
    }

    /// Strips trailing spaces that are not escaped with a backslash.
    ///
    /// An even number of backslashes before a space leaves the space bare, so
    /// it goes; an odd number quotes it, so it and everything before it stays.
    fn trim_trailing_spaces(mut line: &[u8]) -> &[u8] {
        while line.last() == Some(&b' ') {
            let body = &line[..line.len() - 1];
            let slashes = body.iter().rev().take_while(|b| **b == b'\\').count();
            if slashes % 2 == 1 {
                break;
            }
            line = body;
        }
        line
    }
}
