//! A backtracking regular-expression engine with the syntax of the Rust `regex` crate.
//!
//! The syntax, and the meaning given to it, follow the `regex` crate because that is what Typst's
//! `regex(...)` compiles with, and a document's `show regex(...)` rules and `str.matches`,
//! `str.replace` and `str.split` calls must mean here what they mean there.  Supported: literals
//! and the escapes `\n \t \r \a \f \v \x7F \x{...} \u0041 \u{...} \U0001F600 \U{...}`; `.`;
//! bracketed classes with ranges, nesting, the ASCII classes `[[:alpha:]]` and the set operations
//! `&&`, `--` and `~~`; the Unicode shorthands `\d \w \s` and their negations; Unicode property
//! classes `\pL`, `\p{Greek}`, `\p{sc=Grek}`, `\p{scx=Grek}`, `\p{gc!=L}` and `\P{...}` (see
//! [`crate::unicode::property`]); the anchors `^ $ \A \z` and the word boundaries
//! `\b \B \< \> \b{start} \b{end} \b{start-half} \b{end-half}`; numbered and named capture groups,
//! `(...)`, `(?P<name>...)` and `(?<name>...)`, and non-capturing `(?:...)`; the flags `i m s x U`
//! set inline as `(?im)` or scoped as `(?i:...)`, and cleared with `-`; alternation; and the
//! quantifiers `* + ? {n} {n,} {n,m}`, greedy or lazy.
//!
//! As in the `regex` crate, `\d`, `\w`, `\s` and `\b` are Unicode-aware, `^` and `$` match only at
//! the ends of the haystack unless `m` is set, alternation is leftmost-first, and an iteration
//! never yields an empty match where the previous match ended.  Absent, as there: backreferences
//! and look-around.  A `{` that does not open a counted repetition is taken literally, where the
//! `regex` crate refuses it.
//!
//! The matcher is a backtracker over a compiled program, built as the `regex` crate's own
//! bounded backtracker is: the pattern compiles to a small instruction graph of the same shape as
//! the crate's NFA, the choice points live on a heap stack rather than the call stack, and a
//! visited set lets each (instruction, position) pair be explored at most once per search.  So a
//! search costs time linear in the pattern times the text -- `(a+)+$` answers rather than running
//! away -- no text is long enough to overflow the stack, and where a repetition could loop on an
//! empty iteration the visited set ends it exactly where the crate's does, which decides the
//! captures `(a*)*` reports.  The visited set is a bit per pair, so a search whose pattern and
//! text together would need more than [`MAX_VISITED`] bits is refused with an error rather than
//! answered wrongly.
//!
//! ```
//! use oxedyne_fe2o3_text::regex::Regex;
//!
//! let re = Regex::new(r"(?<word>\p{Greek}+)\s(\d+)").expect("compile");
//! let caps = re.captures("see λόγος 12").expect("search").expect("a match");
//! assert_eq!(caps.name_text("word"), Some("λόγος"));
//! assert_eq!(caps.text(2), Some("12"));
//! assert_eq!(re.replace_all("λόγος 12", "$2 ${word}").expect("replace"), "12 λόγος");
//! ```

use crate::unicode::property::{
    self,
    CharClass,
};

use oxedyne_fe2o3_core::prelude::*;


// Limits
pub const MAX_VISITED:  usize   = 1 << 28;  // visited-set bits, instructions x characters: 32 MiB
const MAX_INSTS:        usize   = 1 << 18;  // compiled instructions; `(a{100}){100}` is 10,000
const MAX_REPEAT:       u32     = 10_000;   // largest count a `{n,m}` may name


/// A half-open byte range within the haystack.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    pub start:  usize,
    pub end:    usize,
}

impl Span {

    pub fn len(&self) -> usize { self.end - self.start }

    pub fn is_empty(&self) -> bool { self.start == self.end }

    /// The text of the span within the haystack it was found in.
    pub fn as_str<'h>(&self, hay: &'h str) -> &'h str {
        hay.get(self.start..self.end).unwrap_or("")
    }
}

/// How two operands of a class set operation combine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SetOp {
    And,    // `&&`
    Minus,  // `--`
    Xor,    // `~~`
}

/// One item inside a character class.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Item {
    Ch(char),
    Range(char, char),          // inclusive, low first
    Digit(bool),                // `\d` when true, `\D` when false
    Word(bool),
    Space(bool),
    Prop(CharClass, bool),      // `\p{..}` when true, `\P{..}` when false
    Nested(Class),
}

/// The contents of a class: a union of items, or a set operation on two such contents.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Set {
    Union(Vec<Item>),
    Op(Box<Set>, SetOp, Box<Set>),
}

/// A bracketed class, `[...]` or `[^...]`, or a shorthand standing alone.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Class {
    neg:    bool,
    ci:     bool,   // case-insensitive
    set:    Set,
}

/// A zero-width assertion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Look {
    Start,          // `\A`, or `^` without `m`
    End,            // `\z`, or `$` without `m`
    LineStart,
    LineEnd,
    Word,           // `\b`
    NotWord,        // `\B`
    WordStart,      // `\<`, `\b{start}`
    WordEnd,        // `\>`, `\b{end}`
    WordStartHalf,
    WordEndHalf,
}

/// One node of the parsed pattern.
///
/// An enum rather than a trait object, per the house style: the matcher is one `match` and the
/// whole tree is a plain value that can be cloned, compared and printed.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Node {
    Lit(char),
    LitCi(char),        // already case-folded
    Any,                // any character bar a newline
    AnyNl,              // any character at all, under `s`
    Cls(Class),
    Look(Look),
    Group(usize, Box<Node>),
    Seq(Vec<Node>),
    Alt(Vec<Node>),     // the first alternative that matches, in written order
    Rep {
        node:   Box<Node>,
        min:    u32,
        max:    u32,
        greedy: bool,
    },
}

/// One instruction of a compiled pattern.  Control passes to the next instruction unless the
/// instruction names another.
#[derive(Clone, Debug)]
enum Inst {
    Char(char),
    CharCi(char),           // already case-folded
    Any,
    AnyNl,
    Class(Class),
    Look(Look),
    Save(usize),            // record the position in a capture slot
    Split(usize, usize),    // try the first, then the second
    Jmp(usize),
    Match,
}

/// A pending piece of backtracking work.
enum Frame {
    Step(usize, usize),             // resume at instruction, position
    Restore(usize, Option<usize>),  // put a capture slot back as it was
}

/// Capture spans as character indices, group 0 being the whole match.
type Slots = Vec<Option<(usize, usize)>>;

/// The (instruction, position) pairs a search has explored, a bit each, laid out position by
/// position so that the pairs one search touched form one run to clear for the next.
struct Visited {
    bits:   Vec<u64>,
    ninst:  usize,
    base:   usize,  // first character position covered
    lo:     usize,  // lowest position touched since the last clear
    hi:     usize,  // one past the highest
}

impl Visited {

    fn new(ninst: usize, base: usize, len: usize, src: &str) -> Outcome<Self> {
        let width = len.saturating_sub(base) + 1;
        let n = match ninst.checked_mul(width) {
            Some(n) if n <= MAX_VISITED => n,
            _ => return Err(err!(
                "regex '{}': a pattern of {} instructions over {} characters needs more than {} \
                bits of search state; search a shorter text.", src, ninst, width, MAX_VISITED;
                Excessive, Input)),
        };
        Ok(Self { bits: vec![0; (n + 63) / 64], ninst, base, lo: usize::MAX, hi: 0 })
    }

    /// Marks a pair, answering whether it was new.
    fn insert(&mut self, pc: usize, at: usize) -> bool {
        let i = (at - self.base) * self.ninst + pc;
        let bit = 1u64 << (i % 64);
        match self.bits.get_mut(i / 64) {
            Some(w) if *w & bit == 0 => {
                *w |= bit;
                self.lo = self.lo.min(at);
                self.hi = self.hi.max(at + 1);
                true
            }
            _ => false,
        }
    }

    fn clear(&mut self) {
        if self.lo < self.hi {
            let a = (self.lo - self.base) * self.ninst / 64;
            let b = (((self.hi - self.base) * self.ninst + 63) / 64).min(self.bits.len());
            if let Some(ws) = self.bits.get_mut(a..b) {
                for w in ws {
                    *w = 0;
                }
            }
        }
        self.lo = usize::MAX;
        self.hi = 0;
    }
}

/// A haystack decoded once, so a run of searches over it does not decode it again.
struct Hay<'h> {
    text:   &'h str,
    chars:  Vec<char>,
    offs:   Vec<usize>,     // byte offset of each character, and of the end
}

impl<'h> Hay<'h> {

    fn new(text: &'h str) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let mut offs = Vec::with_capacity(chars.len() + 1);
        for (b, _) in text.char_indices() {
            offs.push(b);
        }
        offs.push(text.len());
        Self { text, chars, offs }
    }

    /// The character index at byte offset `b`, which must fall on a character boundary.
    fn index(&self, b: usize) -> Outcome<usize> {
        match self.offs.binary_search(&b) {
            Ok(i)   => Ok(i),
            Err(_)  => Err(err!(
                "regex: search start {} is not a character boundary of a {} byte haystack.",
                b, self.text.len(); Invalid, Input, Range)),
        }
    }

    fn span(&self, (s, e): (usize, usize)) -> Span {
        Span {
            start:  self.offs.get(s).copied().unwrap_or(self.text.len()),
            end:    self.offs.get(e).copied().unwrap_or(self.text.len()),
        }
    }
}

/// A compiled regular expression.
#[derive(Clone, Debug)]
pub struct Regex {
    prog:   Vec<Inst>,
    names:  Vec<Option<String>>,    // one per group, group 0 included and unnamed
    src:    String,
}

impl Regex {

    pub fn new(pattern: &str) -> Outcome<Self> {
        Self::with_case(pattern, false)
    }

    /// Compiles a pattern, case-insensitive from the start when `ci` is set, as though it began
    /// with `(?i)`.
    pub fn with_case(pattern: &str, ci: bool) -> Outcome<Self> {
        let chars: Vec<char> = pattern.chars().collect();
        let mut p = Parser {
            pat:    &chars,
            at:     0,
            flags:  Flags { i: ci, ..Flags::default() },
            names:  vec![None],
        };
        let root = match p.alt() {
            Ok(n)   => n,
            Err(e)  => return Err(err!(e,
                "regex '{}': could not be compiled, at character {}.", pattern, p.at + 1;
                Invalid, Input)),
        };
        if p.at < p.pat.len() {
            if p.pat[p.at] == ')' {
                return Err(err!(
                    "regex '{}': ')' with no '(' before it, at character {}.", pattern, p.at + 1;
                    Invalid, Input));
            }
            return Err(err!(
                "regex '{}': unexpected '{}' at character {}.",
                pattern, p.pat[p.at], p.at + 1;
                Invalid, Input));
        }
        let mut c = Compiler { prog: Vec::new(), src: pattern };
        res!(c.node(&root));
        res!(c.push(Inst::Match));
        Ok(Self { prog: c.prog, names: p.names, src: pattern.to_string() })
    }

    pub fn as_str(&self) -> &str {
        &self.src
    }

    /// The number of capture groups, counting the whole match as group 0.
    pub fn captures_len(&self) -> usize {
        self.names.len()
    }

    /// The name of each group in order, `None` for group 0 and for unnamed groups.
    pub fn capture_names(&self) -> impl Iterator<Item = Option<&str>> {
        self.names.iter().map(|n| n.as_deref())
    }

    /// The number of the group with this name.
    pub fn group_index(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n.as_deref() == Some(name))
    }

    pub fn is_match(&self, hay: &str) -> Outcome<bool> {
        Ok(res!(self.find(hay)).is_some())
    }

    /// The leftmost match in `hay`.  An error means the step or stack budget ran out, so the
    /// answer is unknown, not that there was no match.
    pub fn find(&self, hay: &str) -> Outcome<Option<Span>> {
        self.find_at(hay, 0)
    }

    /// The leftmost match starting at or after byte `start`.  The text before `start` still
    /// counts as context: `^` does not match at `start` unless it is 0, and `\b` looks back past
    /// it.
    pub fn find_at(&self, hay: &str, start: usize) -> Outcome<Option<Span>> {
        let h = Hay::new(hay);
        let from = res!(h.index(start));
        let mut vis = res!(Visited::new(self.prog.len(), from, h.chars.len(), &self.src));
        Ok(res!(self.search(&h, from, &mut vis))
            .and_then(|s| s.first().copied().flatten().map(|m| h.span(m))))
    }

    pub fn captures<'r, 'h>(&'r self, hay: &'h str) -> Outcome<Option<Captures<'r, 'h>>> {
        self.captures_at(hay, 0)
    }

    /// The captures of the leftmost match starting at or after byte `start`, with the text
    /// before it as context, as for [`Regex::find_at`].
    pub fn captures_at<'r, 'h>(
        &'r self,
        hay:    &'h str,
        start:  usize,
    )
        -> Outcome<Option<Captures<'r, 'h>>>
    {
        let h = Hay::new(hay);
        let from = res!(h.index(start));
        let mut vis = res!(Visited::new(self.prog.len(), from, h.chars.len(), &self.src));
        Ok(res!(self.search(&h, from, &mut vis)).map(|s| self.wrap(&h, &s)))
    }

    /// Every successive non-overlapping match, leftmost first.  After an error the iterator
    /// yields nothing more.
    pub fn find_iter<'r, 'h>(&'r self, hay: &'h str) -> Matches<'r, 'h> {
        Matches { it: Walk::new(self, hay) }
    }

    pub fn captures_iter<'r, 'h>(&'r self, hay: &'h str) -> CaptureMatches<'r, 'h> {
        CaptureMatches { it: Walk::new(self, hay) }
    }

    /// The pieces of `hay` between successive matches, including the empty pieces before a match
    /// at the start and after one at the end.
    pub fn split<'h>(&self, hay: &'h str) -> Outcome<Vec<&'h str>> {
        let mut out = Vec::new();
        let mut last = 0;
        for m in self.find_iter(hay) {
            let m = res!(m);
            out.push(hay.get(last..m.start).unwrap_or(""));
            last = m.end;
        }
        out.push(hay.get(last..).unwrap_or(""));
        Ok(out)
    }

    /// Replaces every match with `rep`, expanded as by [`Captures::expand`].
    pub fn replace_all(&self, hay: &str, rep: &str) -> Outcome<String> {
        self.replacen(hay, 0, rep)
    }

    /// Replaces the first `limit` matches, or every match when `limit` is zero.
    pub fn replacen(&self, hay: &str, limit: usize, rep: &str) -> Outcome<String> {
        let mut out = String::with_capacity(hay.len());
        let mut last = 0;
        for (n, caps) in self.captures_iter(hay).enumerate() {
            if limit > 0 && n >= limit {
                break;
            }
            let caps = res!(caps);
            let m = caps.whole();
            out.push_str(hay.get(last..m.start).unwrap_or(""));
            caps.expand(rep, &mut out);
            last = m.end;
        }
        out.push_str(hay.get(last..).unwrap_or(""));
        Ok(out)
    }

    fn wrap<'r, 'h>(&'r self, h: &Hay<'h>, slots: &Slots) -> Captures<'r, 'h> {
        Captures {
            hay:    h.text,
            spans:  slots.iter().map(|s| s.map(|m| h.span(m))).collect(),
            names:  &self.names,
        }
    }

    /// The leftmost match at or after character `from`, as capture spans.  The visited set is
    /// kept across start positions, as the crate keeps it: a pair that failed from one start
    /// fails from any later one, since what follows it does not depend on where the match began.
    fn search(&self, h: &Hay, from: usize, vis: &mut Visited) -> Outcome<Option<Slots>> {
        let out = self.backtrack(h, from, vis);
        vis.clear();
        out
    }

    fn backtrack(&self, h: &Hay, from: usize, vis: &mut Visited) -> Outcome<Option<Slots>> {
        let chars = &h.chars;
        let mut slots: Vec<Option<usize>> = vec![None; 2 * self.names.len()];
        let mut stack: Vec<Frame> = Vec::new();
        for start in from..=chars.len() {
            stack.push(Frame::Step(0, start));
            while let Some(frame) = stack.pop() {
                let (mut pc, mut at) = match frame {
                    Frame::Step(pc, at) => (pc, at),
                    Frame::Restore(i, old) => {
                        if let Some(s) = slots.get_mut(i) {
                            *s = old;
                        }
                        continue;
                    }
                };
                loop {
                    if !vis.insert(pc, at) {
                        break;
                    }
                    let inst = match self.prog.get(pc) {
                        Some(i) => i,
                        None    => return Err(err!(
                            "regex '{}': internal -- instruction {} of {} is missing.",
                            self.src, pc, self.prog.len(); Bug, Index)),
                    };
                    let c = chars.get(at).copied();
                    let hit = match inst {
                        Inst::Char(want)    => c == Some(*want),
                        Inst::CharCi(want)  => c.map(|c| fold(c) == *want).unwrap_or(false),
                        Inst::Any           => c.map(|c| c != '\n').unwrap_or(false),
                        Inst::AnyNl         => c.is_some(),
                        Inst::Class(cl)     => c.map(|c| class_has(cl, c)).unwrap_or(false),
                        Inst::Look(look) => {
                            if look_at(*look, at, chars) {
                                pc += 1;
                                continue;
                            }
                            break;
                        }
                        Inst::Save(i) => {
                            let old = slots.get(*i).copied().flatten();
                            stack.push(Frame::Restore(*i, old));
                            if let Some(s) = slots.get_mut(*i) {
                                *s = Some(at);
                            }
                            pc += 1;
                            continue;
                        }
                        Inst::Split(a, b) => {
                            stack.push(Frame::Step(*b, at));
                            pc = *a;
                            continue;
                        }
                        Inst::Jmp(to) => {
                            pc = *to;
                            continue;
                        }
                        Inst::Match => {
                            let mut out: Slots = Vec::with_capacity(self.names.len());
                            out.push(Some((start, at)));
                            for g in 1..self.names.len() {
                                let pair = match (slots.get(2 * g).copied().flatten(),
                                    slots.get(2 * g + 1).copied().flatten())
                                {
                                    (Some(a), Some(b))  => Some((a, b)),
                                    _                   => None,
                                };
                                out.push(pair);
                            }
                            return Ok(Some(out));
                        }
                    };
                    if !hit {
                        break;
                    }
                    pc += 1;
                    at += 1;
                }
            }
        }
        Ok(None)
    }
}

/// Compiles a parsed pattern into instructions, in the shape the `regex` crate's Thompson
/// compiler gives its NFA.  The shape is not incidental: where an iteration can match nothing,
/// the visited set cuts the loop at the loop's own split, and the captures a match reports
/// depend on which split that is.
struct Compiler<'s> {
    prog:   Vec<Inst>,
    src:    &'s str,
}

impl<'s> Compiler<'s> {

    fn push(&mut self, inst: Inst) -> Outcome<usize> {
        if self.prog.len() >= MAX_INSTS {
            return Err(err!(
                "regex '{}': compiles to more than {} instructions; reduce the repetition counts.",
                self.src, MAX_INSTS; Excessive, Input));
        }
        self.prog.push(inst);
        Ok(self.prog.len() - 1)
    }

    /// Points the split at `at` to `body` first and `out` second, or the other way round for a
    /// lazy repetition.
    fn aim(&mut self, at: usize, body: usize, out: usize, greedy: bool) {
        if let Some(i) = self.prog.get_mut(at) {
            *i = if greedy { Inst::Split(body, out) } else { Inst::Split(out, body) };
        }
    }

    fn node(&mut self, n: &Node) -> Outcome<()> {
        match n {
            Node::Lit(c)    => { res!(self.push(Inst::Char(*c))); }
            Node::LitCi(c)  => { res!(self.push(Inst::CharCi(*c))); }
            Node::Any       => { res!(self.push(Inst::Any)); }
            Node::AnyNl     => { res!(self.push(Inst::AnyNl)); }
            Node::Cls(cl)   => { res!(self.push(Inst::Class(cl.clone()))); }
            Node::Look(l)   => { res!(self.push(Inst::Look(*l))); }
            Node::Group(i, inner) => {
                res!(self.push(Inst::Save(2 * i)));
                res!(self.node(inner));
                res!(self.push(Inst::Save(2 * i + 1)));
            }
            Node::Seq(v) => {
                for x in v {
                    res!(self.node(x));
                }
            }
            Node::Alt(branches) => {
                let mut exits = Vec::new();
                for (k, b) in branches.iter().enumerate() {
                    if k + 1 == branches.len() {
                        res!(self.node(b));
                    } else {
                        let split = res!(self.push(Inst::Split(0, 0)));
                        res!(self.node(b));
                        exits.push(res!(self.push(Inst::Jmp(0))));
                        let next = self.prog.len();
                        self.aim(split, split + 1, next, true);
                    }
                }
                let end = self.prog.len();
                for j in exits {
                    if let Some(i) = self.prog.get_mut(j) {
                        *i = Inst::Jmp(end);
                    }
                }
            }
            Node::Rep { node, min, max, greedy } => res!(self.rep(node, *min, *max, *greedy)),
        }
        Ok(())
    }

    fn rep(&mut self, x: &Node, min: u32, max: u32, greedy: bool) -> Outcome<()> {
        if max == u32::MAX {
            if min == 0 {
                let head = res!(self.push(Inst::Split(0, 0)));
                if min_len(x) > 0 {
                    // One split that is both the loop's head and its exit.
                    res!(self.node(x));
                    res!(self.push(Inst::Jmp(head)));
                    let end = self.prog.len();
                    self.aim(head, head + 1, end, greedy);
                } else {
                    // `x*` as `(x+)?` when `x` can match nothing, as the crate compiles it, so
                    // that the first iteration may be empty and a later empty one is cut.
                    let body = head + 1;
                    res!(self.node(x));
                    let back = res!(self.push(Inst::Split(0, 0)));
                    let end = self.prog.len();
                    self.aim(head, body, end, greedy);
                    self.aim(back, body, end, greedy);
                }
            } else {
                for _ in 1..min {
                    res!(self.node(x));
                }
                let body = self.prog.len();
                res!(self.node(x));
                let back = res!(self.push(Inst::Split(0, 0)));
                self.aim(back, body, back + 1, greedy);
            }
            return Ok(());
        }
        for _ in 0..min {
            res!(self.node(x));
        }
        // Each optional copy may skip straight to the end.
        let mut splits = Vec::new();
        for _ in min..max {
            splits.push(res!(self.push(Inst::Split(0, 0))));
            res!(self.node(x));
        }
        let end = self.prog.len();
        for sp in splits {
            self.aim(sp, sp + 1, end, greedy);
        }
        Ok(())
    }
}

/// The fewest characters a node can match.
fn min_len(n: &Node) -> usize {
    match n {
        Node::Lit(_) | Node::LitCi(_) | Node::Any | Node::AnyNl | Node::Cls(_) => 1,
        Node::Look(_)               => 0,
        Node::Group(_, inner)       => min_len(inner),
        Node::Seq(v)                => v.iter().map(min_len).sum(),
        Node::Alt(v)                => v.iter().map(min_len).min().unwrap_or(0),
        Node::Rep { node, min, .. } => min_len(node).saturating_mul(*min as usize),
    }
}

/// The groups of one match, as byte spans into the haystack.
#[derive(Clone, Debug)]
pub struct Captures<'r, 'h> {
    hay:    &'h str,
    spans:  Vec<Option<Span>>,
    names:  &'r [Option<String>],
}

impl<'r, 'h> Captures<'r, 'h> {

    /// The number of groups, counting the whole match as group 0.
    pub fn len(&self) -> usize { self.spans.len() }

    /// Always false, since group 0 is always there; present to pair with `len`.
    pub fn is_empty(&self) -> bool { self.spans.is_empty() }

    /// The span of the whole match.
    pub fn whole(&self) -> Span {
        self.get(0).unwrap_or(Span { start: 0, end: 0 })
    }

    /// The span of group `i`, `None` when the group took no part in the match.
    pub fn get(&self, i: usize) -> Option<Span> {
        self.spans.get(i).copied().flatten()
    }

    pub fn text(&self, i: usize) -> Option<&'h str> {
        self.get(i).map(|s| s.as_str(self.hay))
    }

    pub fn name(&self, name: &str) -> Option<Span> {
        match self.names.iter().position(|n| n.as_deref() == Some(name)) {
            Some(i) => self.get(i),
            None    => None,
        }
    }

    pub fn name_text(&self, name: &str) -> Option<&'h str> {
        self.name(name).map(|s| s.as_str(self.hay))
    }

    /// Every group's span in order, group 0 first.
    pub fn spans(&self) -> &[Option<Span>] {
        &self.spans
    }

    /// Appends `template` to `out` with each group reference replaced by that group's text, as
    /// the `regex` crate does: `$2` or `${2}` by number, `$name` or `${name}` by name, `$$` for a
    /// literal `$`.  An unbraced reference takes the longest run of letters, digits and `_`, so
    /// `$1a` names a group `1a`; write `${1}a`.  A reference to a group that does not exist or
    /// did not take part becomes nothing.
    pub fn expand(&self, template: &str, out: &mut String) {
        let mut rest = template;
        while let Some(i) = rest.find('$') {
            out.push_str(&rest[..i]);
            rest = &rest[i + 1..];
            if let Some(after) = rest.strip_prefix('$') {
                out.push('$');
                rest = after;
                continue;
            }
            let (name, after) = if let Some(braced) = rest.strip_prefix('{') {
                match braced.find('}') {
                    Some(j) if j > 0 => (&braced[..j], &braced[j + 1..]),
                    _ => {
                        out.push('$');
                        continue;
                    }
                }
            } else {
                let n = rest
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(rest.len());
                if n == 0 {
                    out.push('$');
                    continue;
                }
                (&rest[..n], &rest[n..])
            };
            let text = if name.bytes().all(|b| b.is_ascii_digit()) {
                match name.parse::<usize>() {
                    Ok(i)   => self.text(i),
                    Err(_)  => None,
                }
            } else {
                self.name_text(name)
            };
            out.push_str(text.unwrap_or(""));
            rest = after;
        }
        out.push_str(rest);
    }
}

/// The search state shared by the match iterators.
struct Walk<'r, 'h> {
    re:     &'r Regex,
    hay:    Hay<'h>,
    vis:    Option<Visited>,    // made on the first search, so an iterator costs nothing unused
    at:     usize,          // character index the next search starts from
    last:   Option<usize>,  // where the previous match ended
    done:   bool,
}

impl<'r, 'h> Walk<'r, 'h> {

    fn new(re: &'r Regex, text: &'h str) -> Self {
        Self { re, hay: Hay::new(text), vis: None, at: 0, last: None, done: false }
    }

    /// The next match, by the rule of the `regex` crate: an empty match where the previous one
    /// ended is passed over, and the search tried again one character on.
    fn next_slots(&mut self) -> Option<Outcome<Slots>> {
        if self.done || self.at > self.hay.chars.len() {
            return None;
        }
        if self.vis.is_none() {
            match Visited::new(self.re.prog.len(), 0, self.hay.chars.len(), &self.re.src) {
                Ok(v)   => self.vis = Some(v),
                Err(e)  => { self.done = true; return Some(Err(e)); }
            }
        }
        let vis = match self.vis.as_mut() {
            Some(v) => v,
            None    => { self.done = true; return None; }
        };
        let mut found = match self.re.search(&self.hay, self.at, vis) {
            Ok(f)   => f,
            Err(e)  => { self.done = true; return Some(Err(e)); }
        };
        if let Some((s, e)) = found.as_ref().and_then(|x| x.first().copied().flatten()) {
            if s == e && Some(e) == self.last {
                if self.at + 1 > self.hay.chars.len() {
                    self.done = true;
                    return None;
                }
                found = match self.re.search(&self.hay, self.at + 1, vis) {
                    Ok(f)   => f,
                    Err(e)  => { self.done = true; return Some(Err(e)); }
                };
            }
        }
        let slots = match found {
            Some(s) => s,
            None    => { self.done = true; return None; }
        };
        match slots.first().copied().flatten() {
            Some((_, e)) => {
                self.at = e;
                self.last = Some(e);
                Some(Ok(slots))
            }
            None => {
                self.done = true;
                None
            }
        }
    }
}

/// An iterator over successive match spans; see [`Regex::find_iter`].
pub struct Matches<'r, 'h> {
    it: Walk<'r, 'h>,
}

impl<'r, 'h> Iterator for Matches<'r, 'h> {
    type Item = Outcome<Span>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.it.next_slots() {
            Some(Ok(s)) => {
                let m = s.first().copied().flatten().unwrap_or((0, 0));
                Some(Ok(self.it.hay.span(m)))
            }
            Some(Err(e)) => Some(Err(e)),
            None => None,
        }
    }
}

/// An iterator over successive matches with their groups; see [`Regex::captures_iter`].
pub struct CaptureMatches<'r, 'h> {
    it: Walk<'r, 'h>,
}

impl<'r, 'h> Iterator for CaptureMatches<'r, 'h> {
    type Item = Outcome<Captures<'r, 'h>>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.it.next_slots() {
            Some(Ok(s))     => Some(Ok(self.it.re.wrap(&self.it.hay, &s))),
            Some(Err(e))    => Some(Err(e)),
            None            => None,
        }
    }
}

/// Escapes every character that means something to the parser, so `quote(s)` matches `s`
/// exactly.
pub fn quote(literal: &str) -> String {
    let mut out = String::with_capacity(literal.len() + 8);
    for c in literal.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The single character a case mapping gives, or `c` itself when the mapping expands.
fn single(mut it: impl Iterator<Item = char>, c: char) -> char {
    match (it.next(), it.next()) {
        (Some(x), None) => x,
        _               => c,
    }
}

/// Case-folds one character, approximating Unicode simple case folding: upper case then lower,
/// so that `ſ` and `s`, `ς` and `σ`, `K` (Kelvin) and `k` agree, without letting a mapping that
/// expands (`ß` to `SS`) turn one character into another.  Dotless `ı` is kept apart from `i`, as
/// simple folding keeps it.
fn fold(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    if c == 'ı' {
        return c;
    }
    let up = single(c.to_uppercase(), c);
    single(up.to_lowercase(), up)
}

/// The case forms a case-insensitive class tries a character in.
fn variants(c: char) -> [char; 4] {
    [c, single(c.to_lowercase(), c), single(c.to_uppercase(), c), fold(c)]
}

/// Does the assertion hold at `pos`?
fn look_at(look: Look, pos: usize, chars: &[char]) -> bool {
    let before  = pos > 0 && chars.get(pos - 1).map(|c| property::is_word(*c)).unwrap_or(false);
    let after   = chars.get(pos).map(|c| property::is_word(*c)).unwrap_or(false);
    match look {
        Look::Start         => pos == 0,
        Look::End           => pos == chars.len(),
        Look::LineStart     => pos == 0 || chars.get(pos - 1) == Some(&'\n'),
        Look::LineEnd       => pos == chars.len() || chars.get(pos) == Some(&'\n'),
        Look::Word          => before != after,
        Look::NotWord       => before == after,
        Look::WordStart     => !before && after,
        Look::WordEnd       => before && !after,
        Look::WordStartHalf => !before,
        Look::WordEndHalf   => !after,
    }
}

/// Does a class admit `c`?  Negation comes after case folding, as in the `regex` crate: `(?i)[^a]`
/// refuses `A` as well as `a`.
fn class_has(cl: &Class, c: char) -> bool {
    set_has(&cl.set, c, cl.ci) != cl.neg
}

fn set_has(set: &Set, c: char, ci: bool) -> bool {
    match set {
        Set::Union(items) => items.iter().any(|it| item_has(it, c, ci)),
        Set::Op(a, op, b) => {
            let (x, y) = (set_has(a, c, ci), set_has(b, c, ci));
            match op {
                SetOp::And      => x && y,
                SetOp::Minus    => x && !y,
                SetOp::Xor      => x != y,
            }
        }
    }
}

/// Does an item admit `c`?  Under case-insensitivity the positive form of the item is tried on
/// each case of `c` and only then negated, so `(?i)\P{Lu}` refuses both `A` and `a`.
fn item_has(it: &Item, c: char, ci: bool) -> bool {
    let base = |x: char| -> bool {
        match it {
            Item::Ch(y)         => x == *y,
            Item::Range(a, b)   => *a <= x && x <= *b,
            Item::Digit(_)      => property::is_digit(x),
            Item::Word(_)       => property::is_word(x),
            Item::Space(_)      => property::is_space(x),
            Item::Prop(p, _)    => p.contains(x),
            Item::Nested(_)     => false,
        }
    };
    let want = match it {
        Item::Nested(cl) => return class_has(cl, c),
        Item::Digit(w) | Item::Word(w) | Item::Space(w) | Item::Prop(_, w) => *w,
        Item::Ch(_) | Item::Range(..) => true,
    };
    let hit = if ci { variants(c).iter().any(|v| base(*v)) } else { base(c) };
    hit == want
}


// ── Parsing ─────────────────────────────────────────────────────────

/// The flags a pattern can set inline.
#[derive(Clone, Copy, Debug, Default)]
struct Flags {
    i: bool,    // case-insensitive
    m: bool,    // `^` and `$` match at line ends
    s: bool,    // `.` matches a newline
    x: bool,    // white space and `#` comments ignored
    u: bool,    // `U`: greed swapped
}

/// A recursive-descent parser over the pattern's characters.
struct Parser<'a> {
    pat:    &'a [char],
    at:     usize,
    flags:  Flags,
    names:  Vec<Option<String>>,    // one per group so far, group 0 included
}

impl<'a> Parser<'a> {

    /// Parse `seq ('|' seq)*`.
    fn alt(&mut self) -> Outcome<Node> {
        let mut branches = vec![res!(self.seq())];
        while self.peek() == Some('|') {
            self.at += 1;
            branches.push(res!(self.seq()));
        }
        Ok(if branches.len() == 1 {
            // `remove` cannot fail: the vector was built with one element and only grows.
            branches.remove(0)
        } else {
            Node::Alt(branches)
        })
    }

    /// Parse a run of quantified atoms, stopping at `|` or `)`.  A bare flag group, `(?i)`,
    /// changes the flags for the rest of the enclosing group, alternatives included.
    fn seq(&mut self) -> Outcome<Node> {
        let mut nodes = Vec::new();
        loop {
            self.skip_x();
            match self.peek() {
                None | Some('|') | Some(')') => break,
                _ => {},
            }
            if self.peek() == Some('(') && self.pat.get(self.at + 1) == Some(&'?') {
                // Only a bare flag group is taken here; anything else, errors included, is left
                // for `group` to parse and to report.
                let save = self.at;
                self.at += 2;
                if let Ok(f) = self.flag_list() {
                    if self.peek() == Some(')') {
                        self.at += 1;
                        self.flags = f;
                        continue;
                    }
                }
                self.at = save;
            }
            nodes.push(res!(self.quantified()));
        }
        Ok(Node::Seq(nodes))
    }

    /// Skip white space and `#` comments when the `x` flag is set.
    fn skip_x(&mut self) {
        if !self.flags.x {
            return;
        }
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.at += 1;
            } else if c == '#' {
                while let Some(c) = self.peek() {
                    self.at += 1;
                    if c == '\n' {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    /// Read flag letters from the cursor, `i`, `m`, `s`, `x`, `U`, `u`, with `-` clearing those
    /// after it, stopping before `:` or `)`.  Returns the flags as they would then stand.
    fn flag_list(&mut self) -> Outcome<Flags> {
        let mut f = self.flags;
        let mut on = true;
        let mut any = false;
        while let Some(c) = self.peek() {
            match c {
                'i' => f.i = on,
                'm' => f.m = on,
                's' => f.s = on,
                'x' => f.x = on,
                'U' => f.u = on,
                'u' => {}, // Unicode is always on
                '-' => {
                    if !on {
                        return Err(err!("regex: a flag group has two '-'."; Invalid, Input));
                    }
                    on = false;
                }
                ':' | ')' => break,
                _ => return Err(err!(
                    "regex: '(?{}' is not a flag group this engine knows -- there is no \
                    look-around, and the flags are i, m, s, x and U.", c;
                    Unimplemented, Input)),
            }
            any = true;
            self.at += 1;
        }
        if !any && self.peek() == Some(')') {
            return Err(err!("regex: '(?)' is an empty flag group."; Invalid, Input));
        }
        Ok(f)
    }

    /// Parse one atom and any quantifier that follows it.
    fn quantified(&mut self) -> Outcome<Node> {
        let atom = res!(self.atom());
        self.skip_x();
        let (min, max) = match self.peek() {
            Some('*') => { self.at += 1; (0, u32::MAX) }
            Some('+') => { self.at += 1; (1, u32::MAX) }
            Some('?') => { self.at += 1; (0, 1) }
            Some('{') if self.brace_is_a_quantifier() => res!(self.brace()),
            _         => return Ok(atom),
        };
        // A trailing `?` makes the quantifier lazy; under `U` it makes it greedy.
        let lazy = if self.peek() == Some('?') {
            self.at += 1;
            true
        } else {
            // A trailing `+` is a possessive quantifier elsewhere; here it would silently mean
            // something else, so it is refused rather than mis-read.
            if self.peek() == Some('+') {
                return Err(err!(
                    "regex: possessive quantifiers ('{}+') are not supported.",
                    if max == 1 { "?" } else if min == 1 { "+" } else { "*" };
                    Unimplemented, Input));
            }
            false
        };
        Ok(Node::Rep { node: Box::new(atom), min, max, greedy: lazy == self.flags.u })
    }

    /// Does the `{` at the cursor open a `{n}`, `{n,}` or `{n,m}` quantifier?  A `{` that does not
    /// is an ordinary character -- `\d{` and `a{b}` both mean what they look like.
    fn brace_is_a_quantifier(&self) -> bool {
        let mut i = self.at + 1;
        let mut digits = 0;
        while i < self.pat.len() && self.pat[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
        if digits == 0 {
            return false;
        }
        if i < self.pat.len() && self.pat[i] == ',' {
            i += 1;
            while i < self.pat.len() && self.pat[i].is_ascii_digit() {
                i += 1;
            }
        }
        i < self.pat.len() && self.pat[i] == '}'
    }

    /// Parse `{n}`, `{n,}` or `{n,m}`, the cursor sitting on the `{`.
    fn brace(&mut self) -> Outcome<(u32, u32)> {
        self.at += 1; // the '{'
        let min = res!(self.number());
        let max = if self.peek() == Some(',') {
            self.at += 1;
            if self.peek() == Some('}') { u32::MAX } else { res!(self.number()) }
        } else {
            min
        };
        if self.peek() != Some('}') {
            return Err(err!("regex: unterminated '{{n,m}}' quantifier."; Invalid, Input));
        }
        self.at += 1;
        if min > max {
            return Err(err!(
                "regex: '{{{},{}}}' asks for at least {} repetitions and at most {}.",
                min, max, min, max; Invalid, Input));
        }
        if min > MAX_REPEAT || (max != u32::MAX && max > MAX_REPEAT) {
            return Err(err!(
                "regex: a repetition count above {} is refused.", MAX_REPEAT; Excessive, Input));
        }
        Ok((min, max))
    }

    /// Read a decimal number at the cursor.
    fn number(&mut self) -> Outcome<u32> {
        let start = self.at;
        while self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            self.at += 1;
        }
        if start == self.at {
            return Err(err!("regex: a number was expected."; Invalid, Input, Missing));
        }
        let s: String = self.pat[start..self.at].iter().collect();
        s.parse::<u32>().map_err(|e| err!(e, "regex: '{}' is not a count.", s; Invalid, Input))
    }

    /// Parse one atom: a group, a class, a metacharacter, an escape, or a literal.
    fn atom(&mut self) -> Outcome<Node> {
        let c = match self.peek() {
            Some(c) => c,
            None    => return Err(err!("regex: the pattern ends where an atom was expected.";
                Invalid, Input, Missing)),
        };
        match c {
            '(' => self.group(),
            '[' => {
                let cl = res!(self.class());
                Ok(Node::Cls(cl))
            }
            '.' => { self.at += 1; Ok(if self.flags.s { Node::AnyNl } else { Node::Any }) }
            '^' => {
                self.at += 1;
                Ok(Node::Look(if self.flags.m { Look::LineStart } else { Look::Start }))
            }
            '$' => {
                self.at += 1;
                Ok(Node::Look(if self.flags.m { Look::LineEnd } else { Look::End }))
            }
            '*' | '+' | '?' => Err(err!(
                "regex: '{}' has nothing before it to repeat.", c; Invalid, Input)),
            '{' if self.brace_is_a_quantifier() => Err(err!(
                "regex: a '{{n,m}}' repetition has nothing before it to repeat."; Invalid, Input)),
            ')' => Err(err!("regex: ')' with no '(' before it."; Invalid, Input)),
            '\\' => {
                self.at += 1;
                self.escape()
            }
            _ => { self.at += 1; Ok(self.lit(c)) }
        }
    }

    fn lit(&self, c: char) -> Node {
        if self.flags.i { Node::LitCi(fold(c)) } else { Node::Lit(c) }
    }

    /// Parse a group, the cursor sitting on the `(`.  The flags in force outside are restored
    /// at its `)`.
    fn group(&mut self) -> Outcome<Node> {
        self.at += 1;
        let outer = self.flags;
        let mut idx = None;
        if self.peek() == Some('?') {
            self.at += 1;
            let named = match (self.peek(), self.pat.get(self.at + 1)) {
                (Some('P'), Some('<'))  => { self.at += 2; true }
                (Some('<'), Some(n)) if *n != '=' && *n != '!' => { self.at += 1; true }
                (Some('='), _) | (Some('!'), _) | (Some('<'), _) => return Err(err!(
                    "regex: look-around '(?{}' is not supported.", self.pat[self.at];
                    Unimplemented, Input)),
                _ => false,
            };
            if named {
                let name = res!(self.group_name());
                if self.names.iter().any(|n| n.as_deref() == Some(name.as_str())) {
                    return Err(err!("regex: the group name '{}' is used twice.", name;
                        Invalid, Input));
                }
                idx = Some(self.names.len());
                self.names.push(Some(name));
            } else {
                self.flags = res!(self.flag_list());
                if self.peek() != Some(':') {
                    return Err(err!("regex: a flag group must end with ':' or ')'."; Invalid, Input));
                }
                self.at += 1;
            }
        } else {
            idx = Some(self.names.len());
            self.names.push(None);
        }
        let inner = res!(self.alt());
        if self.peek() != Some(')') {
            return Err(err!("regex: unclosed '('."; Invalid, Input));
        }
        self.at += 1;
        self.flags = outer;
        Ok(match idx {
            Some(i) => Node::Group(i, Box::new(inner)),
            None    => inner,
        })
    }

    /// Read a group name up to and past its `>`.
    fn group_name(&mut self) -> Outcome<String> {
        let start = self.at;
        while let Some(c) = self.peek() {
            if c == '>' {
                break;
            }
            let ok = if self.at == start {
                c == '_' || c.is_alphabetic()
            } else {
                c == '_' || c == '.' || c == '[' || c == ']' || c.is_alphanumeric()
            };
            if !ok {
                return Err(err!("regex: '{}' cannot appear in a group name.", c; Invalid, Input));
            }
            self.at += 1;
        }
        if self.peek() != Some('>') {
            return Err(err!("regex: unclosed group name."; Invalid, Input));
        }
        if start == self.at {
            return Err(err!("regex: an empty group name."; Invalid, Input, Missing));
        }
        let name: String = self.pat[start..self.at].iter().collect();
        self.at += 1;
        Ok(name)
    }

    /// Parse what follows a `\` outside a class.
    fn escape(&mut self) -> Outcome<Node> {
        let e = match self.peek() {
            Some(e) => e,
            None    => return Err(err!("regex: the pattern ends with a lone '\\'."; Invalid, Input)),
        };
        let one = |item: Item, ci: bool| Node::Cls(Class { neg: false, ci, set: Set::Union(vec![item]) });
        match e {
            'b' => {
                self.at += 1;
                if self.peek() != Some('{') {
                    return Ok(Node::Look(Look::Word));
                }
                let end = match self.pat[self.at..].iter().position(|c| *c == '}') {
                    Some(n) => self.at + n,
                    None    => return Err(err!("regex: unclosed '\\b{{'."; Invalid, Input)),
                };
                let kind: String = self.pat[self.at + 1..end].iter().collect();
                self.at = end + 1;
                Ok(Node::Look(match kind.as_str() {
                    "start"         => Look::WordStart,
                    "end"           => Look::WordEnd,
                    "start-half"    => Look::WordStartHalf,
                    "end-half"      => Look::WordEndHalf,
                    _ => return Err(err!("regex: '\\b{{{}}}' is not a known boundary.", kind;
                        Invalid, Input)),
                }))
            }
            'B' => { self.at += 1; Ok(Node::Look(Look::NotWord)) }
            'A' => { self.at += 1; Ok(Node::Look(Look::Start)) }
            'z' => { self.at += 1; Ok(Node::Look(Look::End)) }
            '<' => { self.at += 1; Ok(Node::Look(Look::WordStart)) }
            '>' => { self.at += 1; Ok(Node::Look(Look::WordEnd)) }
            _ => match res!(self.class_escape()) {
                Esc::Item(it)   => Ok(one(it, self.flags.i)),
                Esc::Char(c)    => Ok(self.lit(c)),
            },
        }
    }

    /// Parse a `\` escape that may also appear inside a class, the cursor after the `\`.
    fn class_escape(&mut self) -> Outcome<Esc> {
        let e = match self.peek() {
            Some(e) => e,
            None    => return Err(err!("regex: the pattern ends with a lone '\\'."; Invalid, Input)),
        };
        self.at += 1;
        Ok(match e {
            'd' => Esc::Item(Item::Digit(true)),
            'D' => Esc::Item(Item::Digit(false)),
            'w' => Esc::Item(Item::Word(true)),
            'W' => Esc::Item(Item::Word(false)),
            's' => Esc::Item(Item::Space(true)),
            'S' => Esc::Item(Item::Space(false)),
            'p' | 'P' => {
                let (cc, pos) = res!(self.property());
                Esc::Item(Item::Prop(cc, pos == (e == 'p')))
            }
            'n' => Esc::Char('\n'),
            't' => Esc::Char('\t'),
            'r' => Esc::Char('\r'),
            'a' => Esc::Char('\x07'),
            'f' => Esc::Char('\x0C'),
            'v' => Esc::Char('\x0B'),
            'x' => Esc::Char(res!(self.hex(2))),
            'u' => Esc::Char(res!(self.hex(4))),
            'U' => Esc::Char(res!(self.hex(8))),
            '0'..='9' => return Err(err!(
                "regex: '\\{}' -- backreferences and octal escapes are not supported.", e;
                Unimplemented, Input)),
            _ if e.is_ascii_alphanumeric() => return Err(err!(
                "regex: '\\{}' is not a known escape here.", e; Invalid, Input)),
            _ => Esc::Char(e),
        })
    }

    /// Read a hexadecimal code point, `digits` long or braced, `{...}`.
    fn hex(&mut self, digits: usize) -> Outcome<char> {
        let (start, end, next) = if self.peek() == Some('{') {
            match self.pat[self.at..].iter().position(|c| *c == '}') {
                Some(n) => (self.at + 1, self.at + n, self.at + n + 1),
                None    => return Err(err!("regex: unclosed hexadecimal escape '{{'."; Invalid, Input)),
            }
        } else {
            (self.at, self.at + digits, self.at + digits)
        };
        let s: String = match self.pat.get(start..end) {
            Some(cs) => cs.iter().collect(),
            None     => return Err(err!("regex: a hexadecimal escape needs {} digits.", digits;
                Invalid, Input, Missing)),
        };
        if s.is_empty() || s.len() > 8 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(err!("regex: '{}' is not a hexadecimal code point.", s; Invalid, Input));
        }
        let v = res!(u32::from_str_radix(&s, 16)
            .map_err(|e| err!(e, "regex: '{}' is not hexadecimal.", s; Invalid, Input)));
        self.at = next;
        match char::from_u32(v) {
            Some(c) => Ok(c),
            None    => Err(err!("regex: U+{:X} is not a Unicode scalar value.", v; Invalid, Input)),
        }
    }

    /// Parse the name after `\p` or `\P`: one letter, or braces holding a name, `name=value`,
    /// `name:value` or `name!=value`.  The flag says whether the class is positive before any
    /// `\P`.
    fn property(&mut self) -> Outcome<(CharClass, bool)> {
        let body: String = match self.peek() {
            Some('{') => {
                let end = match self.pat[self.at..].iter().position(|c| *c == '}') {
                    Some(n) => self.at + n,
                    None    => return Err(err!("regex: unclosed '\\p{{'."; Invalid, Input)),
                };
                let s = self.pat[self.at + 1..end].iter().collect();
                self.at = end + 1;
                s
            }
            Some(c) => { self.at += 1; c.to_string() }
            None    => return Err(err!("regex: '\\p' with no property after it."; Invalid, Input)),
        };
        match body.split_once("!=") {
            Some((k, v)) => Ok((res!(CharClass::parse(&fmt!("{}={}", k, v))), false)),
            None         => Ok((res!(CharClass::parse(&body)), true)),
        }
    }

    /// Parse a bracketed class, the cursor sitting on the `[`.
    fn class(&mut self) -> Outcome<Class> {
        self.at += 1; // the '['
        let neg = if self.peek() == Some('^') { self.at += 1; true } else { false };
        let mut set = Set::Union(res!(self.class_union(true)));
        loop {
            let op = match (self.peek(), self.pat.get(self.at + 1)) {
                (Some('&'), Some('&')) => SetOp::And,
                (Some('-'), Some('-')) => SetOp::Minus,
                (Some('~'), Some('~')) => SetOp::Xor,
                _ => break,
            };
            self.at += 2;
            let rhs = Set::Union(res!(self.class_union(false)));
            set = Set::Op(Box::new(set), op, Box::new(rhs));
        }
        if self.peek() != Some(']') {
            return Err(err!("regex: unclosed '['."; Invalid, Input));
        }
        self.at += 1;
        Ok(Class { neg, ci: self.flags.i, set })
    }

    /// Parse the items of a class up to a set operator or the closing `]`, which is left for
    /// the caller.  A `]` first thing in the class is a literal.
    fn class_union(&mut self, first: bool) -> Outcome<Vec<Item>> {
        let mut items = Vec::new();
        if first && self.peek() == Some(']') {
            self.at += 1;
            items.push(Item::Ch(']'));
        }
        loop {
            if self.flags.x {
                while self.peek().map(|c| c.is_whitespace()).unwrap_or(false) {
                    self.at += 1;
                }
            }
            let c = match self.peek() {
                Some(']')   => break,
                Some(c)     => c,
                None        => return Err(err!("regex: unclosed '['."; Invalid, Input)),
            };
            let pair = self.pat.get(self.at + 1).copied();
            if matches!((c, pair), ('&', Some('&')) | ('-', Some('-')) | ('~', Some('~'))) {
                break;
            }
            let lo = if c == '[' {
                if pair == Some(':') {
                    if let Some(it) = res!(self.posix()) {
                        items.push(it);
                        continue;
                    }
                }
                let inner = res!(self.class());
                items.push(Item::Nested(inner));
                continue;
            } else if c == '\\' {
                self.at += 1;
                match res!(self.class_escape()) {
                    Esc::Item(it) => { items.push(it); continue; }
                    Esc::Char(ch) => ch,
                }
            } else {
                self.at += 1;
                c
            };
            // A `-` between two single characters makes the pair a range.
            let dash_then = self.pat.get(self.at + 1).copied();
            if self.peek() == Some('-') && dash_then.is_some() && dash_then != Some(']')
                && dash_then != Some('-')
            {
                self.at += 1; // the '-'
                let hi = match self.peek() {
                    Some('\\') => {
                        self.at += 1;
                        match res!(self.class_escape()) {
                            Esc::Char(ch) => ch,
                            Esc::Item(_) => return Err(err!(
                                "regex: a class shorthand cannot end the range from '{}'.", lo;
                                Invalid, Input)),
                        }
                    }
                    Some(h) => { self.at += 1; h }
                    None    => return Err(err!("regex: unclosed '['."; Invalid, Input)),
                };
                if hi < lo {
                    return Err(err!(
                        "regex: the range '{}-{}' runs backwards.", lo, hi; Invalid, Input));
                }
                items.push(Item::Range(lo, hi));
            } else {
                items.push(Item::Ch(lo));
            }
        }
        Ok(items)
    }

    /// Parse an ASCII class such as `[:alpha:]` or `[:^digit:]`, the cursor on its `[`.  `None`,
    /// with the cursor unmoved, when what follows is not one, so the `[` opens a nested class.
    fn posix(&mut self) -> Outcome<Option<Item>> {
        let rest = &self.pat[self.at + 2..];
        let end = match rest.windows(2).position(|w| w == [':', ']']) {
            Some(n) => n,
            None    => return Ok(None),
        };
        let mut name: String = rest[..end].iter().collect();
        let neg = name.starts_with('^');
        if neg {
            name.remove(0);
        }
        let r = |a: char, b: char| Item::Range(a, b);
        let items = match name.as_str() {
            "alnum"     => vec![r('0', '9'), r('A', 'Z'), r('a', 'z')],
            "alpha"     => vec![r('A', 'Z'), r('a', 'z')],
            "ascii"     => vec![r('\0', '\x7F')],
            "blank"     => vec![Item::Ch('\t'), Item::Ch(' ')],
            "cntrl"     => vec![r('\0', '\x1F'), Item::Ch('\x7F')],
            "digit"     => vec![r('0', '9')],
            "graph"     => vec![r('!', '~')],
            "lower"     => vec![r('a', 'z')],
            "print"     => vec![r(' ', '~')],
            "punct"     => vec![r('!', '/'), r(':', '@'), r('[', '`'), r('{', '~')],
            "space"     => vec![r('\t', '\r'), Item::Ch(' ')],
            "upper"     => vec![r('A', 'Z')],
            "word"      => vec![r('0', '9'), r('A', 'Z'), r('a', 'z'), Item::Ch('_')],
            "xdigit"    => vec![r('0', '9'), r('A', 'F'), r('a', 'f')],
            _           => return Ok(None),
        };
        self.at += 2 + end + 2;
        Ok(Some(Item::Nested(Class { neg, ci: self.flags.i, set: Set::Union(items) })))
    }

    fn peek(&self) -> Option<char> {
        self.pat.get(self.at).copied()
    }
}

/// What an escape stands for: a class item, or one character.
enum Esc {
    Item(Item),
    Char(char),
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Compile and match, so a test reads as one line.
    fn m(pat: &str, hay: &str) -> bool {
        let r = match Regex::new(pat) {
            Ok(r)  => r,
            Err(e) => panic!("compiling '{}': {}", pat, e),
        };
        match r.is_match(hay) {
            Ok(b)  => b,
            Err(e) => panic!("matching '{}' against '{}': {}", pat, hay, e),
        }
    }

    #[test]
    fn test_literals_and_dot() {
        assert!(m("abc", "xxabcxx"));
        assert!(!m("abc", "xxabxx"));
        assert!(m("a.c", "abc"));
        assert!(!m("a.c", "a\nc"), "'.' must not cross a newline");
        assert!(m("(?s)a.c", "a\nc"), "unless 's' is set");
        assert!(m("a\\.c", "a.c"));
        assert!(!m("a\\.c", "abc"), "an escaped dot is a literal dot");
    }

    #[test]
    fn test_anchors_and_boundaries() {
        assert!(m("^abc", "abc"));
        assert!(!m("^abc", "xabc"));
        assert!(m("abc$", "xabc"));
        assert!(!m("abc$", "abcx"));
        assert!(!m("^b", "a\nb"), "'^' is the start of the text without 'm'");
        assert!(m("(?m)^b$", "a\nb\nc"), "and of a line with it");
        assert!(m("\\bcat\\b", "the cat sat"));
        assert!(!m("\\bcat\\b", "concatenate"));
        assert!(m("\\Bcat", "concat"));
        assert!(m("\\bκαι\\b", "λόγος και"), "a word boundary is Unicode-aware");
    }

    #[test]
    fn test_classes() {
        assert!(m("[abc]+", "zzbbzz"));
        assert!(!m("[abc]", "xyz"));
        assert!(m("[^abc]", "x"));
        assert!(!m("[^abc]", "a"));
        assert!(m("[a-f0-9]{6}", "colour #ff00aa here"));
        assert!(!m("[a-f0-9]{6}", "#ffz0aa"));
        assert!(m("[]]", "]"), "a ']' first thing in a class is a literal");
        assert!(m("[a-]", "-"), "a '-' last thing in a class is a literal");
        assert!(m("\\d\\d:\\d\\d", "at 09:45 today"));
        assert!(m("[\\d.]+", "3.14"));
        assert!(m("^[a-z&&[^aeiou]]+$", "rhythm"));
        assert!(!m("^[a-z&&[^aeiou]]+$", "rhyme"));
        assert!(m("^[[:alpha:]]+$", "Abc"));
    }

    #[test]
    fn test_quantifiers() {
        assert!(m("ab*c", "ac"));
        assert!(m("ab*c", "abbbc"));
        assert!(!m("ab+c", "ac"));
        assert!(m("ab?c", "ac"));
        assert!(m("a{3}", "aaa"));
        assert!(!m("^a{3}$", "aa"));
        assert!(m("a{2,3}b", "aaab"));
        assert!(!m("^a{2,3}$", "aaaa"));
        assert!(m("a{2,}b", "aaaaab"));
        // A '{' that opens no quantifier is a literal brace.
        assert!(m("a{b}", "a{b}"));
    }

    #[test]
    fn test_lazy_quantifiers_take_the_shorter_match() {
        let r = Regex::new("<.+?>").expect("compile");
        let span = r.find("<a><b>").expect("find").expect("a match");
        assert_eq!(Span { start: 0, end: 3 }, span, "the lazy '+?' should stop at the first '>'");
        let g = Regex::new("<.+>").expect("compile");
        let all = g.find("<a><b>").expect("find").expect("a match");
        assert_eq!(Span { start: 0, end: 6 }, all, "and the greedy '+' should run to the last");
        let u = Regex::new("(?U)<.+>").expect("compile");
        let swapped = u.find("<a><b>").expect("find").expect("a match");
        assert_eq!(Span { start: 0, end: 3 }, swapped, "'U' swaps greed");
    }

    #[test]
    fn test_alternation_and_groups() {
        assert!(m("cat|dog", "a dog here"));
        assert!(m("(cat|dog)s", "two dogs"));
        assert!(!m("(cat|dog)s", "two dog"));
        assert!(m("(?:ab)+c", "ababc"));
        assert!(m("^(a|b)*$", "abba"));
    }

    #[test]
    fn test_case_insensitivity() {
        let r = Regex::with_case("HeLLo", true).expect("compile");
        assert!(r.is_match("say hello there").expect("match"));
        let c = Regex::with_case("[A-Z]+", true).expect("compile");
        assert!(c.is_match("lower").expect("match"), "a range should fold too");
        let n = Regex::with_case("[^a]", true).expect("compile");
        assert!(!n.is_match("A").expect("match"),
            "a negated class must refuse the other case of what it excludes");
        assert!(m("(?i)ΣΟΦΟΣ", "σοφος"), "folding is Unicode");
        assert!(m("a(?i:B)c", "abc"));
        assert!(!m("a(?i:B)c", "abC"), "a scoped flag ends with its group");
    }

    #[test]
    fn test_a_span_is_reported_in_bytes_through_multibyte_text() {
        let r = Regex::new("naïve").expect("compile");
        let hay = "a colour — naïve";
        let span = r.find(hay).expect("find").expect("a match");
        assert_eq!("naïve", &hay[span.start..span.end],
            "the span must index the original bytes");
    }

    #[test]
    fn test_a_pathological_pattern_is_answered_in_linear_time() {
        // The classic exponential case for a backtracker without a visited set.
        let r = Regex::new("(a+)+$").expect("compile");
        let hay = "a".repeat(40) + "b";
        assert_eq!(r.find(&hay).expect("an answer, not a give-up"), None,
            "there is no match: the text ends in 'b'");
    }

    #[test]
    fn test_an_empty_repetition_terminates() {
        // `(a*)*` can match nothing for ever; the matcher must notice and move on.
        assert!(m("^(a*)*$", ""));
        assert!(m("^(a*)*$", "aaa"));
        assert!(!m("^(a*)*$", "aab"));
    }

    #[test]
    fn test_a_very_long_line_is_answered_rather_than_aborting_the_process() {
        // A minified file is one enormous line.  The backtracking stack is on the heap, so neither
        // a character repetition nor a repeated group can overflow the call stack; reaching these
        // assertions at all is the proof.
        let hay = "a".repeat(500_000);
        let star = Regex::new("^a*$").expect("compile");
        assert!(star.is_match(&hay).expect("a long repetition"));
        let mixed = Regex::new("a+b?a").expect("compile");
        assert!(mixed.is_match(&hay).expect("and one that must backtrack"));
        let grouped = Regex::new("^(?:ab)+$").expect("compile");
        let pairs = "ab".repeat(200_000);
        assert!(grouped.is_match(&pairs).expect("a group repeated two hundred thousand times"));
    }

    #[test]
    fn test_a_search_too_large_to_hold_says_so() {
        // Instructions times characters beyond the visited-set limit is refused, not guessed.
        let r = Regex::new("(?:a|b){2000}").expect("compile");
        let hay = "a".repeat(200_000);
        let e = r.find(&hay).expect_err("this search needs more state than allowed");
        assert!(fmt!("{}", e).contains("bits of search state"), "{}", e);
    }

    #[test]
    fn test_bad_patterns_are_refused_with_a_reason() {
        for (pat, want) in [
            ("(ab",         "unclosed '('"),
            ("[ab",         "unclosed '['"),
            ("a)",          "')' with no '('"),
            ("*a",          "nothing before it"),
            ("a{3,2}",      "at least 3"),
            ("[z-a]",       "runs backwards"),
            ("a\\",         "lone '\\'"),
            ("(?=a)",       "look-around"),
            ("(a)\\1",      "backreferences"),
            ("\\p{Nope}",   "not known"),
            ("(?<n>a)(?<n>b)", "used twice"),
            ("\\q",         "not a known escape"),
        ] {
            let e = Regex::new(pat).expect_err(&fmt!("'{}' should not compile", pat));
            let msg = fmt!("{}", e);
            assert!(msg.contains(want), "'{}' should say '{}', said: {}", pat, want, msg);
        }
    }

    #[test]
    fn test_quote_makes_a_literal_of_anything() {
        let raw = "a.b*c(d)[e]{f}|g^h$i+j?k\\l#m&&n--o~~p";
        let r = Regex::new(&quote(raw)).expect("a quoted literal must compile");
        assert!(r.is_match(raw).expect("match"), "and must match itself");
        assert!(!r.is_match("axbxc").expect("match"), "without meaning anything else");
    }

    #[test]
    fn test_alternation_is_leftmost_first() {
        let r = Regex::new("a|ab").expect("compile");
        let s = r.find("ab").expect("find").expect("a match");
        assert_eq!(Span { start: 0, end: 1 }, s, "the first alternative wins, as in Perl");
    }

    #[test]
    fn test_empty_matches_follow_the_regex_crate() {
        // The `regex` crate documents that `a*` over "baaa" yields 0..0 and 1..4 and nothing at
        // 4..4: an empty match where the previous match ended is passed over.
        let r = Regex::new("a*").expect("compile");
        let got: Vec<Span> = r.find_iter("baaa").map(|x| x.expect("find")).collect();
        assert_eq!(got, vec![Span { start: 0, end: 0 }, Span { start: 1, end: 4 }]);
        let e = Regex::new("").expect("compile");
        assert_eq!(e.split("abc").expect("split"), vec!["", "a", "b", "c", ""]);
    }

    #[test]
    fn test_expand_follows_the_regex_crate() {
        let r = Regex::new("(?P<y>\\d{4})-(\\d{2})").expect("compile");
        let c = r.captures("on 2026-09").expect("search").expect("a match");
        let mut out = String::new();
        c.expand("$2/${y} $$ $1a ${1}a $9 $", &mut out);
        assert_eq!(out, "09/2026 $  2026a  $");
    }
}
