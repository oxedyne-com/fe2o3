//! Shell-style path globbing.
//!
//! `*` matches any run of characters within one path segment, `?` matches one such character,
//! `**` matches any number of whole segments including none, `[abc]` and `[a-z]` and `[!abc]`
//! match one character from a set, and `{a,b}` offers alternatives.
//!
//! A pattern with no `/` in it is matched against the file's name alone, which is what makes
//! `*.rs` mean "any Rust file anywhere" -- the convention every search tool uses.  A pattern that
//! does contain a `/` is matched against the whole relative path.

use oxedyne_fe2o3_core::prelude::*;


/// The most alternatives a pattern's braces may expand to.
///
/// Nested braces multiply, so `{a,b}{c,d}{e,f}` is eight; the cap stops a pattern from becoming a
/// denial of service against the process reading it.
const MAX_ALTS: usize = 256;


/// One piece of a single path segment.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Part {
    /// Literal text.
    Lit(String),
    /// `*`: any run of characters, `/` excepted.
    Star,
    /// `?`: exactly one character, `/` excepted.
    One,
    /// `[...]`: one character from a set, which may be negated.
    Set {
        /// Whether the set is negated, `[!...]` or `[^...]`.
        neg:    bool,
        /// Single characters the set admits.
        chars:  Vec<char>,
        /// Inclusive ranges the set admits.
        ranges: Vec<(char, char)>,
    },
}

/// One path segment of a pattern.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Seg {
    /// `**`: any number of whole segments, including none.
    Deep,
    /// An ordinary segment, matched piecewise.
    Parts(Vec<Part>),
}

/// A compiled glob pattern.
#[derive(Clone, Debug)]
pub struct Glob {
    /// One segment list per brace alternative; a path matching any of them matches the glob.
    alts:       Vec<Vec<Seg>>,
    /// Whether the pattern named a path rather than a file name.
    on_path:    bool,
    /// The pattern as written, for error messages.
    src:        String,
}

impl Glob {

    /// Compile a glob pattern.
    ///
    /// # Arguments
    /// * `pattern` - The pattern, e.g. `**/*_test.rs` or `*.{md,typ}`.
    ///
    /// # Returns
    /// The compiled glob, or an error naming what could not be read.
    pub fn new(pattern: &str) -> Outcome<Self> {
        if pattern.trim().is_empty() {
            return Err(err!("glob: the pattern is empty."; Invalid, Input, Missing));
        }
        let expanded = res!(expand_braces(pattern));
        let on_path = pattern.contains('/');
        let mut alts = Vec::with_capacity(expanded.len());
        for e in &expanded {
            alts.push(res!(compile(e)));
        }
        Ok(Self { alts, on_path, src: pattern.to_string() })
    }

    /// The pattern as it was written.
    pub fn as_str(&self) -> &str {
        &self.src
    }

    /// Whether `path` matches.
    ///
    /// # Arguments
    /// * `path` - A relative path with `/` separators, e.g. `src/wasm/opfs.rs`.
    pub fn matches(&self, path: &str) -> bool {
        let subject = if self.on_path {
            path.trim_start_matches("./")
        } else {
            // A bare pattern names a file, so only the last segment is offered to it.
            match path.rsplit('/').next() {
                Some(name) => name,
                None       => path,
            }
        };
        let segs: Vec<&str> = subject.split('/').filter(|s| !s.is_empty()).collect();
        for alt in &self.alts {
            if seg_match(alt, &segs) {
                return true;
            }
        }
        false
    }
}

/// Whether the pattern segments match the path segments, `**` spanning as many as it needs.
///
/// # Arguments
/// * `pat` - The pattern's segments.
/// * `path` - The path's segments.
fn seg_match(pat: &[Seg], path: &[&str]) -> bool {
    match pat.split_first() {
        None => path.is_empty(),
        Some((Seg::Deep, rest)) => {
            // `**` takes nothing, then one segment, then two, until something fits.
            for take in 0..=path.len() {
                if seg_match(rest, &path[take..]) {
                    return true;
                }
            }
            false
        }
        Some((Seg::Parts(parts), rest)) => {
            match path.split_first() {
                Some((head, tail)) => part_match(parts, &head.chars().collect::<Vec<char>>())
                    && seg_match(rest, tail),
                None => false,
            }
        }
    }
}

/// Whether one segment's parts match one segment's characters.
///
/// # Arguments
/// * `parts` - The pattern pieces.
/// * `seg` - The segment, as characters.
fn part_match(parts: &[Part], seg: &[char]) -> bool {
    match parts.split_first() {
        None => seg.is_empty(),
        Some((Part::Star, rest)) => {
            for take in 0..=seg.len() {
                if part_match(rest, &seg[take..]) {
                    return true;
                }
            }
            false
        }
        Some((Part::One, rest)) => !seg.is_empty() && part_match(rest, &seg[1..]),
        Some((Part::Set { neg, chars, ranges }, rest)) => {
            match seg.first() {
                None => false,
                Some(&c) => {
                    let mut hit = chars.contains(&c);
                    if !hit {
                        for (a, b) in ranges {
                            if c >= *a && c <= *b {
                                hit = true;
                                break;
                            }
                        }
                    }
                    (hit != *neg) && part_match(rest, &seg[1..])
                }
            }
        }
        Some((Part::Lit(text), rest)) => {
            let want: Vec<char> = text.chars().collect();
            seg.len() >= want.len()
                && seg[..want.len()] == want[..]
                && part_match(rest, &seg[want.len()..])
        }
    }
}

/// Expand `{a,b}` alternatives into one pattern each, leftmost group first.
///
/// # Arguments
/// * `pattern` - The pattern, possibly with braces.
fn expand_braces(pattern: &str) -> Outcome<Vec<String>> {
    let chars: Vec<char> = pattern.chars().collect();
    // Find the first unescaped `{` and its matching `}`.
    let mut open = None;
    let mut depth = 0usize;
    let mut close = None;
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '\\' => { i += 1; }
            '{'  => {
                if open.is_none() {
                    open = Some(i);
                }
                depth += 1;
            }
            '}'  => {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(i);
                        break;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    let (o, c) = match (open, close) {
        (Some(o), Some(c)) => (o, c),
        (Some(_), None)    => return Err(err!(
            "glob '{}': unclosed '{{'.", pattern; Invalid, Input)),
        _                  => return Ok(vec![pattern.to_string()]),
    };
    // Split the body on top-level commas.
    let mut bodies = Vec::new();
    let mut cur = String::new();
    let mut d = 0usize;
    let mut j = o + 1;
    while j < c {
        let ch = chars[j];
        match ch {
            '{' => { d += 1; cur.push(ch); }
            '}' => { d -= 1; cur.push(ch); }
            ',' if d == 0 => { bodies.push(std::mem::take(&mut cur)); }
            _   => cur.push(ch),
        }
        j += 1;
    }
    bodies.push(cur);
    let head: String = chars[..o].iter().collect();
    let tail: String = chars[c + 1..].iter().collect();
    let mut out = Vec::new();
    for b in bodies {
        for rest in res!(expand_braces(&fmt!("{}{}{}", head, b, tail))) {
            if out.len() >= MAX_ALTS {
                return Err(err!(
                    "glob '{}': braces expand to more than {} alternatives.", pattern, MAX_ALTS;
                    Excessive, Input));
            }
            out.push(rest);
        }
    }
    Ok(out)
}

/// Compile one brace-free pattern into segments.
///
/// # Arguments
/// * `pattern` - The pattern, with no `{}` left in it.
fn compile(pattern: &str) -> Outcome<Vec<Seg>> {
    let mut segs = Vec::new();
    for raw in pattern.trim_start_matches("./").split('/') {
        if raw.is_empty() {
            continue; // a doubled or trailing slash names no segment
        }
        if raw == "**" {
            segs.push(Seg::Deep);
            continue;
        }
        segs.push(Seg::Parts(res!(compile_seg(raw, pattern))));
    }
    if segs.is_empty() {
        return Err(err!("glob '{}': names no path.", pattern; Invalid, Input));
    }
    Ok(segs)
}

/// Compile one segment into its parts.
///
/// # Arguments
/// * `seg` - The segment source.
/// * `whole` - The whole pattern, for error messages.
fn compile_seg(seg: &str, whole: &str) -> Outcome<Vec<Part>> {
    let chars: Vec<char> = seg.chars().collect();
    let mut parts: Vec<Part> = Vec::new();
    let mut lit = String::new();
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '*' => {
                if !lit.is_empty() {
                    parts.push(Part::Lit(std::mem::take(&mut lit)));
                }
                // `***` and `a**b` are all just "any run within the segment".
                while i < chars.len() && chars[i] == '*' {
                    i += 1;
                }
                parts.push(Part::Star);
            }
            '?' => {
                if !lit.is_empty() {
                    parts.push(Part::Lit(std::mem::take(&mut lit)));
                }
                parts.push(Part::One);
                i += 1;
            }
            '[' => {
                if !lit.is_empty() {
                    parts.push(Part::Lit(std::mem::take(&mut lit)));
                }
                let (part, next) = res!(compile_set(&chars, i, whole));
                parts.push(part);
                i = next;
            }
            '\\' if i + 1 < chars.len() => {
                lit.push(chars[i + 1]);
                i += 2;
            }
            c => { lit.push(c); i += 1; }
        }
    }
    if !lit.is_empty() {
        parts.push(Part::Lit(lit));
    }
    Ok(parts)
}

/// Compile a `[...]` set beginning at `start`, returning it and the index after the `]`.
///
/// # Arguments
/// * `chars` - The segment's characters.
/// * `start` - Index of the `[`.
/// * `whole` - The whole pattern, for error messages.
fn compile_set(chars: &[char], start: usize, whole: &str) -> Outcome<(Part, usize)> {
    let mut i = start + 1;
    let neg = if chars.get(i) == Some(&'!') || chars.get(i) == Some(&'^') {
        i += 1;
        true
    } else {
        false
    };
    let mut set_chars = Vec::new();
    let mut ranges = Vec::new();
    // A `]` first thing is a literal `]`, as the shell has it.
    if chars.get(i) == Some(&']') {
        set_chars.push(']');
        i += 1;
    }
    loop {
        let c = match chars.get(i) {
            Some(']') => { i += 1; break; }
            Some(&c)  => c,
            None      => return Err(err!("glob '{}': unclosed '['.", whole; Invalid, Input)),
        };
        i += 1;
        if chars.get(i) == Some(&'-') && chars.get(i + 1).map(|x| *x != ']').unwrap_or(false) {
            let hi = match chars.get(i + 1) {
                Some(&h) => h,
                None     => return Err(err!("glob '{}': unclosed '['.", whole; Invalid, Input)),
            };
            if hi < c {
                return Err(err!(
                    "glob '{}': the range '{}-{}' runs backwards.", whole, c, hi; Invalid, Input));
            }
            ranges.push((c, hi));
            i += 2;
        } else {
            set_chars.push(c);
        }
    }
    if set_chars.is_empty() && ranges.is_empty() {
        return Err(err!("glob '{}': '[]' admits nothing.", whole; Invalid, Input));
    }
    Ok((Part::Set { neg, chars: set_chars, ranges }, i))
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Compile and match, so a test reads as one line.
    fn m(pat: &str, path: &str) -> bool {
        match Glob::new(pat) {
            Ok(g)  => g.matches(path),
            Err(e) => panic!("compiling '{}': {}", pat, e),
        }
    }

    #[test]
    fn test_a_bare_pattern_matches_the_file_name_anywhere() {
        assert!(m("*.rs", "src/wasm/opfs.rs"));
        assert!(m("*.rs", "opfs.rs"));
        assert!(!m("*.rs", "src/wasm/opfs.js"));
        assert!(m("Cargo.toml", "a/b/Cargo.toml"));
    }

    #[test]
    fn test_a_pattern_with_a_slash_matches_the_whole_path() {
        assert!(m("src/*.rs", "src/tools.rs"));
        assert!(!m("src/*.rs", "src/wasm/opfs.rs"), "'*' must not cross a '/'");
        assert!(m("src/**/*.rs", "src/wasm/opfs.rs"));
        assert!(m("src/**/*.rs", "src/tools.rs"), "'**' must be able to take no segments at all");
        assert!(m("**/*_test.rs", "a/b/c/thing_test.rs"));
        assert!(m("**/*_test.rs", "thing_test.rs"));
    }

    #[test]
    fn test_question_and_sets() {
        assert!(m("?.rs", "a.rs"));
        assert!(!m("?.rs", "ab.rs"));
        assert!(m("[abc]*.rs", "b_thing.rs"));
        assert!(!m("[abc]*.rs", "z_thing.rs"));
        assert!(m("[a-z][0-9].txt", "a1.txt"));
        assert!(m("[!x]*.rs", "a.rs"));
        assert!(!m("[!x]*.rs", "x.rs"));
    }

    #[test]
    fn test_braces_offer_alternatives() {
        assert!(m("*.{md,typ}", "notes.md"));
        assert!(m("*.{md,typ}", "notes.typ"));
        assert!(!m("*.{md,typ}", "notes.txt"));
        assert!(m("src/{a,b}/*.rs", "src/b/x.rs"));
        assert!(m("{a,b}{c,d}.rs", "ad.rs"));
    }

    #[test]
    fn test_a_leading_dot_slash_is_not_a_segment() {
        assert!(m("src/*.rs", "./src/tools.rs"));
        assert!(m("./src/*.rs", "src/tools.rs"));
    }

    #[test]
    fn test_bad_patterns_are_refused_with_a_reason() {
        for (pat, want) in [
            ("a[bc",    "unclosed '['"),
            ("a{b,c",   "unclosed '{'"),
            ("[z-a]",   "runs backwards"),
            ("",        "empty"),
        ] {
            let e = Glob::new(pat).expect_err(&fmt!("'{}' should not compile", pat));
            let msg = fmt!("{}", e);
            assert!(msg.contains(want), "'{}' should say '{}', said: {}", pat, want, msg);
        }
    }

    #[test]
    fn test_a_dotted_name_is_matched_like_any_other() {
        // Unlike the shell, a leading dot is not special here: a search tool that hid dotfiles
        // from an explicit pattern would be answering a question nobody asked.
        assert!(m("*.yml", ".github/workflows/ci.yml"));
        assert!(m(".github/**", ".github/workflows/ci.yml"));
    }
}
