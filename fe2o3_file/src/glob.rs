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
//! Git's ignore files have no `{a,b}`, so neither [`Glob::new`] nor
//! [`IgnoreFile`] reads one. [`Glob::with_braces`] compiles a pattern whose
//! braces offer alternatives, as a search tool's pattern does, matching what any
//! of its expansions matches.
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
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
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
    files_only: bool,       // set by the caller, never by the pattern: not a directory
    alts:       Vec<Alt>,   // one, unless braces were expanded; any may match
}

/// One brace-free pattern, as one line of an ignore file compiles to.
#[derive(Clone, Debug)]
struct Alt {
    dir_only:   bool,       // the pattern ended with `/`, so directories only
    // A leading Comp::Globstar stands in for an unanchored pattern's freedom to
    // match at any depth.
    comps:      Vec<Comp>,
}

/// One piece of a pattern being read for braces.
#[derive(Clone, Debug)]
enum Piece {
    Bytes(Vec<u8>),         // copied as written, escapes and classes kept
    Alts(Vec<Vec<Piece>>),  // `{a,b}`: one sequence per alternative
}

impl Glob {

    /// Compiles one pattern, as written on one line of an ignore file.
    ///
    /// The line must already be free of comments and trailing unescaped spaces,
    /// which are the file's business rather than the pattern's; see
    /// [`IgnoreFile::parse`]. A line that is empty, or empty once its `!` and
    /// `/` dressing is removed, is an error, since it can match nothing.
    pub fn new(line: &[u8]) -> Outcome<Self> {
        let (negated, rest) = Self::negation(line);
        match Self::alt(rest) {
            Some(alt)   => Ok(Self { negated, files_only: false, alts: vec![alt] }),
            None        => Err(Self::empty(line)),
        }
    }

    /// Compiles a pattern in which `{a,b}` offers alternatives, as a shell's
    /// brace expansion and most search tools read it; git's ignore files do not,
    /// so [`Glob::new`] and [`IgnoreFile`] take a brace literally.
    ///
    /// The pattern matches what any of its expansions would match compiled alone
    /// by [`Glob::new`], so each alternative decides its own anchoring and
    /// trailing `/`: `{*.rs,src/*.toml}` finds a Rust file at any depth and the
    /// manifests directly under `src`. A leading `!` is read from the whole line
    /// before the braces. Alternatives nest, `{}` around one alternative is just
    /// that alternative, and an empty alternative stands for nothing, so
    /// `*.rs{,.bak}` matches `a.rs` and `a.rs.bak`. A `\{`, `\,` or `\}` is
    /// literal, as is a brace or comma inside a class, a `{` that never closes,
    /// and a `,` or `}` outside any braces.
    ///
    /// Nested and successive braces multiply, so a pattern expanding to more
    /// than `alts_max` alternatives is refused rather than compiled.
    pub fn with_braces(line: &[u8], alts_max: usize) -> Outcome<Self> {
        let (negated, rest) = Self::negation(line);
        let seq = Self::pieces(rest);
        let n = Self::count(&seq);
        if n > alts_max {
            return Err(err!(
                "The pattern {:?} expands to {} alternatives, more than the {} allowed.",
                String::from_utf8_lossy(line), n, alts_max;
            Invalid, Input, Excessive));
        }
        let mut alts = Vec::new();
        for one in Self::spell(&seq) {
            // An alternative left with no content can match no path.
            if let Some(alt) = Self::alt(&one) {
                alts.push(alt);
            }
        }
        if alts.is_empty() {
            return Err(Self::empty(line));
        }
        Ok(Self { negated, files_only: false, alts })
    }

    fn negation(line: &[u8]) -> (bool, &[u8]) {
        match line.first() {
            Some(b'!')  => (true, &line[1..]),
            _           => (false, line),
        }
    }

    fn empty(line: &[u8]) -> Error<ErrTag> {
        err!(
            "The pattern {:?} has no content to match a path against.",
            String::from_utf8_lossy(line);
        Invalid, Input)
    }

    /// Compiles a pattern free of its `!`; nothing when no content is left.
    fn alt(line: &[u8]) -> Option<Alt> {
        let mut rest = line;
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
            return None;
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
        Some(Alt { dir_only, comps })
    }

    /// How many bytes at `i` must be read as one: an escape pair, a class (which
    /// ends, as in [`Glob::new`], before any `/`), or a single byte.
    fn unit(s: &[u8], i: usize) -> usize {
        match s[i] {
            b'\\' if i + 1 < s.len() => 2,
            b'[' => {
                let end = match s[i..].iter().position(|b| *b == b'/') {
                    Some(p) => i + p,
                    None    => s.len(),
                };
                match Self::class(&s[i..end]) {
                    Some((_, used)) => used,
                    None            => 1,
                }
            },
            _ => 1,
        }
    }

    /// Where the brace opened at `open` closes, with the commas that divide it at
    /// its own depth; nothing when it never closes.
    fn close(s: &[u8], open: usize) -> Option<(Vec<usize>, usize)> {
        let mut depth = 0usize;
        let mut cuts = Vec::new();
        let mut i = open;
        while i < s.len() {
            match s[i] {
                b'{'                => depth += 1,
                b'}'                => {
                    depth -= 1;
                    if depth == 0 {
                        return Some((cuts, i));
                    }
                },
                b',' if depth == 1  => cuts.push(i),
                _                   => (),
            }
            i += Self::unit(s, i);
        }
        None
    }

    fn pieces(s: &[u8]) -> Vec<Piece> {
        let mut seq = Vec::new();
        let mut lit = Vec::new();
        let mut i = 0;
        while i < s.len() {
            if s[i] == b'{' {
                match Self::close(s, i) {
                    Some((cuts, close)) => {
                        if !lit.is_empty() {
                            seq.push(Piece::Bytes(std::mem::take(&mut lit)));
                        }
                        let mut alts = Vec::new();
                        let mut from = i + 1;
                        for cut in cuts.into_iter().chain(std::iter::once(close)) {
                            alts.push(Self::pieces(&s[from..cut]));
                            from = cut + 1;
                        }
                        seq.push(Piece::Alts(alts));
                        i = close + 1;
                    },
                    // A brace that never closes is a literal one, escaped so
                    // that it stays one.
                    None => {
                        lit.extend_from_slice(b"\\{");
                        i += 1;
                    },
                }
                continue;
            }
            let n = Self::unit(s, i);
            lit.extend_from_slice(&s[i..i + n]);
            i += n;
        }
        if !lit.is_empty() {
            seq.push(Piece::Bytes(lit));
        }
        seq
    }

    /// How many patterns a sequence spells, saturating rather than overflowing.
    fn count(seq: &[Piece]) -> usize {
        let mut n = 1usize;
        for piece in seq {
            if let Piece::Alts(alts) = piece {
                let sum = alts.iter().fold(0usize, |acc, a| acc.saturating_add(Self::count(a)));
                n = n.saturating_mul(sum);
            }
        }
        n
    }

    fn spell(seq: &[Piece]) -> Vec<Vec<u8>> {
        let mut out = vec![Vec::new()];
        for piece in seq {
            match piece {
                Piece::Bytes(b) => for one in out.iter_mut() {
                    one.extend_from_slice(b);
                },
                Piece::Alts(alts) => {
                    let mut ends = Vec::new();
                    for a in alts {
                        ends.extend(Self::spell(a));
                    }
                    let mut next = Vec::with_capacity(out.len() * ends.len());
                    for head in &out {
                        for end in &ends {
                            let mut one = head.clone();
                            one.extend_from_slice(end);
                            next.push(one);
                        }
                    }
                    out = next;
                },
            }
        }
        out
    }

    /// Narrows the pattern to what is not a directory, which no ignore-file syntax
    /// can say. A directory is never matched, whatever it is called, and a pattern
    /// that also ended in `/` matches nothing at all.
    pub fn files_only(mut self) -> Self {
        self.files_only = true;
        self
    }

    pub fn is_negated(&self) -> bool {
        self.negated
    }

    /// Does every alternative of the pattern match directories alone?
    pub fn is_dir_only(&self) -> bool {
        self.alts.iter().all(|a| a.dir_only)
    }

    pub fn is_files_only(&self) -> bool {
        self.files_only
    }

    /// Reports whether a path matches, where `is_dir` says what the path names.
    ///
    /// The path is relative, `/`-joined, with no leading or trailing slash. A
    /// directory-only pattern refuses anything that is not a directory, and a
    /// files-only one refuses a directory; neither by itself speaks for the files
    /// beneath, which is the business of [`IgnoreFile::excludes`].
    pub fn matches(&self, path: &[u8], is_dir: bool) -> bool {
        if (self.files_only && is_dir) || path.is_empty() {
            return false;
        }
        let parts: Vec<&[u8]> = path.split(|b| *b == b'/').collect();
        self.alts.iter().any(|a| (is_dir || !a.dir_only) && Self::match_comps(&a.comps, &parts))
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
        Self::parse_as(bytes, false)
    }

    /// As [`IgnoreFile::parse`], but every rule speaks for files alone: a
    /// directory is never matched, however it is named. The built-in lists of
    /// names an editor or Syncthing writes ([`EDITOR_DROPPINGS`] and
    /// [`SYNC_DROPPINGS`]) are read this way, because neither writes a
    /// directory by one of them, and one holding somebody's work under such a
    /// name must not be kept out whole.
    pub fn parse_files_only(bytes: &[u8]) -> Self {
        Self::parse_as(bytes, true)
    }

    fn parse_as(bytes: &[u8], files_only: bool) -> Self {
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
                rules.push(if files_only { glob.files_only() } else { glob });
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


// Names a terminal editor writes beside the file it is working on.  A caller lays the list beneath
// a repository's own rules, as `oxedyne_fe2o3_text::secret::SECRET_PATHS` is laid: parse these
// lines first, so that the repository's own `!*~` or `!.#*` still re-includes one by name.  They
// name files, so compile them with `IgnoreFile::parse_files_only`: `drafts~/` is somebody's folder.
pub const EDITOR_DROPPINGS: &[&str] = &[
	"*.swp",		// vim swap file
	"*.swo",		// vim swap file, second crash
	"*.swx",		// vim swap file, third crash
	"4913",			// vim's probe, written to see that a directory takes a new file
	"\\#*#",		// emacs autosave; escaped, as a leading # starts a comment
	".#*",			// emacs lock, a link to a target that is never there
	"*~",			// the backup convention several editors share
	"*.kate-swp",	// kate swap file
];

// Names Syncthing writes while it is part way through bringing a file in.  Laid beneath the
// repository's own rules like `EDITOR_DROPPINGS`, and compiled files-only like it.  `*.sync-conflict-*` is deliberately absent: a
// conflict copy is the other machine's content, and keeping it out would lose it.
pub const SYNC_DROPPINGS: &[&str] = &[
	".syncthing.*.tmp",	// the transfer file, renamed over its target when complete
	"~syncthing~*.tmp",	// the same, as Syncthing spells it on Windows
];
