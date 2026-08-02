//! A small backtracking regular-expression engine.
//!
//! Enough of the common syntax to serve a search tool: literals, `.`, character classes,
//! the `\d \w \s` shorthands and their negations, the `^ $` anchors, `\b` word boundaries,
//! groups, alternation, and the `* + ? {n,m}` quantifiers in both greedy and lazy forms.
//!
//! Deliberately absent: capture groups, backreferences and look-around.  A search tool reports
//! the lines that matched, never the pieces of a match, so leaving captures out keeps the
//! matcher a single recursive walk with an explicit continuation and no capture-slot bookkeeping.
//! A group is therefore a grouping only, and `(?:...)` is accepted as a synonym for `(...)`.
//!
//! Backtracking can be made to cost exponential time by a pattern such as `(a+)+b`, and to cost
//! unbounded stack by a repeated group over a long line, so every search carries a step budget and
//! a stack budget.  [`Regex::find`] returns an error rather than a wrong answer when either runs
//! out: a caller that reported "no match" there would be reporting a silence it had not earned,
//! and a stack overflow is not an answer at all but an aborted process.

use oxedyne_fe2o3_core::prelude::*;


/// The most matcher steps one [`Regex::find`] may take before it gives up and says so.
///
/// Reached only by a pathological pattern; an ordinary one over an ordinary line costs a few
/// hundred.
const MAX_STEPS: u64 = 2_000_000;

/// The most stack, in bytes, one search may use before it gives up and says so.
///
/// A step budget alone does not bound the *stack*: `(ab)+` against a long line recurses once per
/// iteration, and enough iterations abort the process rather than returning an answer -- in a
/// browser, taking the whole page with it.  A repetition of a single character is looped rather
/// than recursed (see [`Regex::repeat`]), which removes the common case; this bounds the rest.
///
/// Bytes rather than a frame count, because a frame is not a fixed size: an unoptimised build of
/// this matcher spends about five kilobytes per level and an optimised one a fraction of that, so
/// any frame count safe for the first wastes most of the second.  Half a megabyte sits inside the
/// one-megabyte stack a wasm module is given and well inside a thread's two.
const MAX_STACK: usize = 512 * 1024;

/// The largest repetition count a `{n,m}` quantifier may name.
const MAX_REPEAT: u32 = 10_000;


/// A half-open byte range within the haystack.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    /// Byte offset of the first byte of the match.
    pub start: usize,
    /// Byte offset one past the last byte of the match.
    pub end: usize,
}

/// One item inside a character class.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Item {
    /// A single character.
    Ch(char),
    /// An inclusive range of characters, low first.
    Range(char, char),
    /// `\d` when true, `\D` when false.
    Digit(bool),
    /// `\w` when true, `\W` when false.
    Word(bool),
    /// `\s` when true, `\S` when false.
    Space(bool),
}

/// A bracketed character class, `[...]` or `[^...]`.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Class {
    /// Whether the class is negated, `[^...]`.
    neg:    bool,
    /// What the class admits, before negation.
    items:  Vec<Item>,
}

/// One node of the parsed pattern.
///
/// An enum rather than a trait object, per the house style: the matcher is one `match` and the
/// whole tree is a plain value that can be cloned, compared and printed.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Node {
    /// A literal character, already case-folded when the pattern is case-insensitive.
    Lit(char),
    /// Any character bar a newline.
    Any,
    /// A bracketed character class.
    Cls(Class),
    /// The start of the haystack, or of a line.
    Start,
    /// The end of the haystack, or of a line.
    End,
    /// A word boundary when true, a non-boundary when false.
    Bound(bool),
    /// A sequence matched in order.
    Seq(Vec<Node>),
    /// The first alternative that matches, in written order.
    Alt(Vec<Node>),
    /// A repetition of the node it wraps.
    Rep {
        /// What is repeated.
        node:   Box<Node>,
        /// Fewest repetitions that will do.
        min:    u32,
        /// Most repetitions allowed.
        max:    u32,
        /// Whether to prefer more repetitions over fewer.
        greedy: bool,
    },
}

/// What is left to match once the current node has matched.
///
/// The continuation is what makes backtracking work without closures: a repetition tries one more
/// iteration and, if the rest of the pattern then fails, gives that iteration back.  Every variant
/// borrows, so the whole chain lives on the stack.
enum Cont<'a> {
    /// Nothing left; the match is complete.
    Done,
    /// The rest of a sequence, then whatever followed it.
    Seq {
        /// Nodes still to match, in order.
        seq:    &'a [Node],
        /// What follows them.
        next:   &'a Cont<'a>,
    },
    /// One more turn round a repetition, then whatever follows the repetition.
    Rep {
        /// The [`Node::Rep`] being repeated.
        rep:    &'a Node,
        /// How many iterations have matched so far.
        done:   u32,
        /// Where the iteration just completed began, so an empty one can be spotted.
        at:     usize,
        /// What follows the repetition.
        next:   &'a Cont<'a>,
    },
}

/// The haystack and the budget, carried through the recursion.
struct St<'h> {
    /// The haystack as characters, so a class or a quantifier counts characters, not bytes.
    chars:  &'h [char],
    /// Whether comparisons are case-insensitive.
    ci:     bool,
    /// Steps still available before the search gives up.
    budget: u64,
    /// How deep the matcher currently is, for the message when it gives up.
    depth:  u32,
    /// Address of a local in the frame that started the search, against which the stack in use
    /// is measured.
    base:   usize,
}

/// A compiled regular expression.
#[derive(Clone, Debug)]
pub struct Regex {
    /// The parsed pattern.
    root:   Node,
    /// Whether comparisons are case-insensitive.
    ci:     bool,
    /// The pattern as written, for error messages.
    src:    String,
}

impl Regex {

    /// Compile a case-sensitive pattern.
    ///
    /// # Arguments
    /// * `pattern` - The regular expression source.
    ///
    /// # Returns
    /// The compiled expression, or an error naming what in the pattern could not be read.
    pub fn new(pattern: &str) -> Outcome<Self> {
        Self::with_case(pattern, false)
    }

    /// Compile a pattern, choosing whether it is case-insensitive.
    ///
    /// Case folding is done at compile time for literals and at match time for the haystack, so a
    /// case-insensitive search costs no more than a case-sensitive one.
    ///
    /// # Arguments
    /// * `pattern` - The regular expression source.
    /// * `ci` - Whether to ignore case.
    pub fn with_case(pattern: &str, ci: bool) -> Outcome<Self> {
        let chars: Vec<char> = pattern.chars().collect();
        let mut p = Parser { pat: &chars, at: 0, ci };
        let root = res!(p.alt());
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
        Ok(Self { root, ci, src: pattern.to_string() })
    }

    /// The pattern as it was written.
    pub fn as_str(&self) -> &str {
        &self.src
    }

    /// Whether the pattern matches anywhere in `hay`.
    ///
    /// # Arguments
    /// * `hay` - The text to search.
    pub fn is_match(&self, hay: &str) -> Outcome<bool> {
        Ok(res!(self.find(hay)).is_some())
    }

    /// The leftmost match in `hay`, as byte offsets, or `None` when there is none.
    ///
    /// Alternation is leftmost-first, as Perl and the `regex` crate have it: `a|ab` matches the
    /// `a` of `ab`, not the whole of it.
    ///
    /// # Arguments
    /// * `hay` - The text to search.
    ///
    /// # Returns
    /// The match span, `None` for no match, or an error when the step budget ran out -- which
    /// means the answer is unknown, not that there was no match.
    pub fn find(&self, hay: &str) -> Outcome<Option<Span>> {
        // Byte offsets alongside the characters, so a span can be reported in the caller's terms.
        let chars: Vec<char> = hay.chars().collect();
        let mut offs: Vec<usize> = Vec::with_capacity(chars.len() + 1);
        let mut b = 0usize;
        for c in &chars {
            offs.push(b);
            b += c.len_utf8();
        }
        offs.push(b);

        let mut st = St { chars: &chars, ci: self.ci, budget: MAX_STEPS, depth: 0, base: 0 };
        let seq = std::slice::from_ref(&self.root);
        for start in 0..=chars.len() {
            match self.run(seq, start, &Cont::Done, &mut st) {
                Ok(Some(end)) => return Ok(Some(Span { start: offs[start], end: offs[end] })),
                Ok(None)      => {},
                Err(e)        => return Err(e),
            }
        }
        Ok(None)
    }

    /// Match `seq` at `pos`, then whatever `cont` says follows it.
    ///
    /// # Arguments
    /// * `seq` - Nodes to match in order.
    /// * `pos` - Character index to match at.
    /// * `cont` - What follows the sequence.
    /// * `st` - Haystack and remaining budget.
    fn run<'a>(
        &self,
        seq:    &'a [Node],
        pos:    usize,
        cont:   &Cont<'a>,
        st:     &mut St,
    )
        -> Outcome<Option<usize>>
    {
        if st.budget == 0 {
            return Err(err!(
                "regex '{}': gave up after {} steps -- the pattern backtracks too much on this \
                input to answer. Simplify it (nested quantifiers such as '(a+)+' are the usual \
                cause).", self.src, MAX_STEPS;
                Excessive, Input));
        }
        st.budget -= 1;
        // How much stack this search has taken: the distance from a local in the frame that
        // started it to a local in this one.  `abs_diff` rather than a subtraction because the
        // direction the stack grows is the platform's business, not this function's.
        let probe = 0u8;
        let here = &probe as *const u8 as usize;
        if st.base == 0 {
            st.base = here;
        }
        if st.base.abs_diff(here) > MAX_STACK {
            return Err(err!(
                "regex '{}': gave up {} levels deep, having used {} bytes of stack -- the pattern \
                nests or repeats too far on this input to answer.",
                self.src, st.depth, MAX_STACK;
                Excessive, Input));
        }
        // Measured here rather than in each recursive call, because every cycle of the recursion
        // passes through this function.  The decrement is unconditional: an early return that
        // skipped it would leak depth and misreport a later, innocent match.
        st.depth += 1;
        let out = self.walk(seq, pos, cont, st);
        st.depth -= 1;
        out
    }

    /// One step of the walk, with the budget and the depth already charged by [`Regex::run`].
    ///
    /// # Arguments
    /// * `seq` - Nodes to match in order.
    /// * `pos` - Character index to match at.
    /// * `cont` - What follows the sequence.
    /// * `st` - Haystack and remaining budget.
    fn walk<'a>(
        &self,
        seq:    &'a [Node],
        pos:    usize,
        cont:   &Cont<'a>,
        st:     &mut St,
    )
        -> Outcome<Option<usize>>
    {
        let (head, tail) = match seq.split_first() {
            Some(x) => x,
            None    => return self.resume(cont, pos, st),
        };
        let rest = Cont::Seq { seq: tail, next: cont };

        Ok(match head {
            Node::Lit(want) => {
                match st.chars.get(pos) {
                    Some(&c) if fold(c, st.ci) == *want =>
                        res!(self.run(&[], pos + 1, &rest, st)),
                    _ => None,
                }
            }
            Node::Any => {
                match st.chars.get(pos) {
                    Some(&c) if c != '\n' => res!(self.run(&[], pos + 1, &rest, st)),
                    _                     => None,
                }
            }
            Node::Cls(cl) => {
                match st.chars.get(pos) {
                    Some(&c) if class_has(cl, c, st.ci) =>
                        res!(self.run(&[], pos + 1, &rest, st)),
                    _ => None,
                }
            }
            Node::Start => {
                let at = pos == 0 || st.chars.get(pos - 1) == Some(&'\n');
                if at { res!(self.run(&[], pos, &rest, st)) } else { None }
            }
            Node::End => {
                let at = pos == st.chars.len() || st.chars.get(pos) == Some(&'\n');
                if at { res!(self.run(&[], pos, &rest, st)) } else { None }
            }
            Node::Bound(want) => {
                let before = pos > 0 && is_word(st.chars[pos - 1]);
                let after  = pos < st.chars.len() && is_word(st.chars[pos]);
                if (before != after) == *want {
                    res!(self.run(&[], pos, &rest, st))
                } else {
                    None
                }
            }
            Node::Seq(inner) => res!(self.run(inner, pos, &rest, st)),
            Node::Alt(branches) => {
                let mut hit = None;
                for b in branches {
                    if let Some(end) = res!(self.run(std::slice::from_ref(b), pos, &rest, st)) {
                        hit = Some(end);
                        break;
                    }
                }
                hit
            }
            Node::Rep { .. } => res!(self.repeat(head, 0, pos, &rest, st)),
        })
    }

    /// Take up a continuation at `pos`.
    ///
    /// # Arguments
    /// * `cont` - What is left to match.
    /// * `pos` - Character index reached.
    /// * `st` - Haystack and remaining budget.
    fn resume(
        &self,
        cont:   &Cont<'_>,
        pos:    usize,
        st:     &mut St,
    )
        -> Outcome<Option<usize>>
    {
        match cont {
            Cont::Done => Ok(Some(pos)),
            Cont::Seq { seq, next } => self.run(seq, pos, next, st),
            Cont::Rep { rep, done, at, next } => {
                // An iteration that consumed nothing would repeat for ever; stop and go on, which
                // is what `(a*)*` against `b` must do.
                if pos == *at {
                    return self.resume(next, pos, st);
                }
                self.repeat(rep, *done, pos, next, st)
            }
        }
    }

    /// Continue a repetition that has matched `done` iterations and reached `pos`.
    ///
    /// # Arguments
    /// * `rep` - The [`Node::Rep`] being repeated.
    /// * `done` - Iterations matched so far.
    /// * `pos` - Character index reached.
    /// * `next` - What follows the repetition.
    /// * `st` - Haystack and remaining budget.
    fn repeat<'a>(
        &self,
        rep:    &'a Node,
        done:   u32,
        pos:    usize,
        next:   &Cont<'a>,
        st:     &mut St,
    )
        -> Outcome<Option<usize>>
    {
        let (node, min, max, greedy) = match rep {
            Node::Rep { node, min, max, greedy } => (node.as_ref(), *min, *max, *greedy),
            // Unreachable by construction: `repeat` is only ever handed a `Rep`.
            _ => return Err(err!("regex '{}': internal -- repeat on a non-repeat node.", self.src;
                Bug, Invalid)),
        };
        // A repetition of a one-character node is counted in a loop.  Recursing once per
        // character is what turns `.*` over a long line into an aborted process rather than an
        // answer, and a minified file is one long line.
        if one_char(node) {
            let mut reached = done;
            let mut end = pos;
            while reached < max && one_char_at(node, end, st) {
                end += 1;
                reached += 1;
            }
            if reached < min {
                return Ok(None);
            }
            let lo = min.max(done);
            // Candidate lengths, in the order this quantifier prefers them.
            let mut count = if greedy { reached } else { lo };
            loop {
                if st.budget == 0 {
                    return Err(err!(
                        "regex '{}': gave up after {} steps.", self.src, MAX_STEPS;
                        Excessive, Input));
                }
                st.budget -= 1;
                let at = pos + (count - done) as usize;
                if let Some(e) = res!(self.resume(next, at, st)) {
                    return Ok(Some(e));
                }
                if greedy {
                    if count == lo { return Ok(None); }
                    count -= 1;
                } else {
                    if count == reached { return Ok(None); }
                    count += 1;
                }
            }
        }
        // Try one more turn round the loop.
        let more = |me: &Self, st: &mut St| -> Outcome<Option<usize>> {
            if done >= max {
                return Ok(None);
            }
            let again = Cont::Rep { rep, done: done + 1, at: pos, next };
            me.run(std::slice::from_ref(node), pos, &again, st)
        };
        // Or stop here and match what follows.
        let stop = |me: &Self, st: &mut St| -> Outcome<Option<usize>> {
            if done < min {
                return Ok(None);
            }
            me.resume(next, pos, st)
        };
        if greedy {
            match res!(more(self, st)) {
                Some(e) => Ok(Some(e)),
                None    => stop(self, st),
            }
        } else {
            match res!(stop(self, st)) {
                Some(e) => Ok(Some(e)),
                None    => more(self, st),
            }
        }
    }
}

/// Escape every character that means something to the parser, so `quote(s)` matches `s` exactly.
///
/// # Arguments
/// * `literal` - Text to be matched verbatim.
pub fn quote(literal: &str) -> String {
    let mut out = String::with_capacity(literal.len() + 8);
    for c in literal.chars() {
        if "\\.+*?()|[]{}^$".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Case-fold one character when the search ignores case.
///
/// Uses the first character of the Unicode lowercase mapping, which is the identity for every
/// mapping that does not expand -- and an expanding one (Turkish dotted capital I, German
/// sharp s) is not a case a line search needs to get right.
fn fold(c: char, ci: bool) -> char {
    if ci { c.to_lowercase().next().unwrap_or(c) } else { c }
}

/// Whether this node matches exactly one character, so a repetition of it can be counted in a
/// loop rather than one stack frame at a time.
fn one_char(node: &Node) -> bool {
    matches!(node, Node::Lit(_) | Node::Any | Node::Cls(_))
}

/// Whether a one-character node matches at `pos`.
///
/// # Arguments
/// * `node` - A node [`one_char`] admits.
/// * `pos` - Character index to test.
/// * `st` - The haystack.
fn one_char_at(node: &Node, pos: usize, st: &St) -> bool {
    match (node, st.chars.get(pos)) {
        (Node::Lit(want), Some(&c)) => fold(c, st.ci) == *want,
        (Node::Any,       Some(&c)) => c != '\n',
        (Node::Cls(cl),   Some(&c)) => class_has(cl, c, st.ci),
        _                           => false,
    }
}

/// Whether a character counts as a word character for `\w` and `\b`.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether a class admits `c`.
///
/// Under case-insensitivity the character is tried in each of its cases rather than the class
/// being rewritten, so `[A-Z]` admits `a` without the ranges having to be expanded.
///
/// # Arguments
/// * `cl` - The class.
/// * `c` - The candidate character.
/// * `ci` - Whether to ignore case.
fn class_has(cl: &Class, c: char, ci: bool) -> bool {
    let mut hit = class_has_exact(cl, c);
    if ci && !hit {
        for alt in c.to_lowercase().chain(c.to_uppercase()) {
            if alt != c && class_has_exact(cl, alt) {
                hit = true;
                break;
            }
        }
    }
    // Negation is applied once, after every case has been tried: `[^a]` must refuse `A` under
    // case-insensitivity, and refusing it means the un-negated test found `A` through `a`.
    if cl.neg { !hit } else { hit }
}

/// Whether the class's items admit `c`, before negation and without case folding.
fn class_has_exact(cl: &Class, c: char) -> bool {
    for it in &cl.items {
        let hit = match it {
            Item::Ch(x)         => *x == c,
            Item::Range(a, b)   => c >= *a && c <= *b,
            Item::Digit(want)   => c.is_ascii_digit() == *want,
            Item::Word(want)    => is_word(c) == *want,
            Item::Space(want)   => c.is_whitespace() == *want,
        };
        if hit {
            return true;
        }
    }
    false
}


// ── Parsing ─────────────────────────────────────────────────────────

/// A recursive-descent parser over the pattern's characters.
struct Parser<'a> {
    /// The pattern.
    pat:    &'a [char],
    /// How far it has been read.
    at:     usize,
    /// Whether literals are folded as they are read.
    ci:     bool,
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

    /// Parse a run of quantified atoms, stopping at `|` or `)`.
    fn seq(&mut self) -> Outcome<Node> {
        let mut nodes = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            nodes.push(res!(self.quantified()));
        }
        Ok(Node::Seq(nodes))
    }

    /// Parse one atom and any quantifier that follows it.
    fn quantified(&mut self) -> Outcome<Node> {
        let atom = res!(self.atom());
        let (min, max) = match self.peek() {
            Some('*') => { self.at += 1; (0, u32::MAX) }
            Some('+') => { self.at += 1; (1, u32::MAX) }
            Some('?') => { self.at += 1; (0, 1) }
            Some('{') if self.brace_is_a_quantifier() => res!(self.brace()),
            _         => return Ok(atom),
        };
        // A trailing `?` makes the quantifier lazy.
        let greedy = if self.peek() == Some('?') {
            self.at += 1;
            false
        } else {
            // A trailing `+` is a possessive quantifier elsewhere; here it would silently mean
            // something else, so it is refused rather than mis-read.
            if self.peek() == Some('+') {
                return Err(err!(
                    "regex: possessive quantifiers ('{}+') are not supported.",
                    if max == 1 { "?" } else if min == 1 { "+" } else { "*" };
                    Unimplemented, Input));
            }
            true
        };
        Ok(Node::Rep { node: Box::new(atom), min, max, greedy })
    }

    /// Whether the `{` at the cursor opens a `{n}`, `{n,}` or `{n,m}` quantifier.
    ///
    /// A `{` that does not is an ordinary character -- `\d{` and `a{b}` are both legal patterns
    /// meaning what they look like.
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
            '(' => {
                self.at += 1;
                // `(?:` is a non-capturing group; since nothing here captures, it is a synonym.
                if self.peek() == Some('?') && self.pat.get(self.at + 1) == Some(&':') {
                    self.at += 2;
                } else if self.peek() == Some('?') {
                    return Err(err!(
                        "regex: '(?' groups other than the non-capturing '(?:' are not supported \
                        -- there is no look-around and there are no named groups.";
                        Unimplemented, Input));
                }
                let inner = res!(self.alt());
                if self.peek() != Some(')') {
                    return Err(err!("regex: unclosed '('."; Invalid, Input));
                }
                self.at += 1;
                Ok(inner)
            }
            '[' => self.class(),
            '.' => { self.at += 1; Ok(Node::Any) }
            '^' => { self.at += 1; Ok(Node::Start) }
            '$' => { self.at += 1; Ok(Node::End) }
            '*' | '+' | '?' => Err(err!(
                "regex: '{}' has nothing before it to repeat.", c; Invalid, Input)),
            ')' => Err(err!("regex: ')' with no '(' before it."; Invalid, Input)),
            '\\' => {
                self.at += 1;
                let e = match self.peek() {
                    Some(e) => e,
                    None    => return Err(err!("regex: the pattern ends with a lone '\\'.";
                        Invalid, Input)),
                };
                self.at += 1;
                Ok(match e {
                    'd' => Node::Cls(Class { neg: false, items: vec![Item::Digit(true)] }),
                    'D' => Node::Cls(Class { neg: false, items: vec![Item::Digit(false)] }),
                    'w' => Node::Cls(Class { neg: false, items: vec![Item::Word(true)] }),
                    'W' => Node::Cls(Class { neg: false, items: vec![Item::Word(false)] }),
                    's' => Node::Cls(Class { neg: false, items: vec![Item::Space(true)] }),
                    'S' => Node::Cls(Class { neg: false, items: vec![Item::Space(false)] }),
                    'b' => Node::Bound(true),
                    'B' => Node::Bound(false),
                    _   => Node::Lit(fold(res!(escape_char(e)), self.ci)),
                })
            }
            _ => { self.at += 1; Ok(Node::Lit(fold(c, self.ci))) }
        }
    }

    /// Parse a bracketed class, the cursor sitting on the `[`.
    fn class(&mut self) -> Outcome<Node> {
        self.at += 1; // the '['
        let neg = if self.peek() == Some('^') { self.at += 1; true } else { false };
        let mut items = Vec::new();
        // A `]` first thing is a literal `]`, as every other engine has it.
        if self.peek() == Some(']') {
            self.at += 1;
            items.push(Item::Ch(']'));
        }
        loop {
            let c = match self.peek() {
                Some(']') => { self.at += 1; break; }
                Some(c)   => c,
                None      => return Err(err!("regex: unclosed '['."; Invalid, Input)),
            };
            self.at += 1;
            // A shorthand inside a class stands for its whole set and cannot be a range end.
            if c == '\\' {
                let e = match self.peek() {
                    Some(e) => e,
                    None    => return Err(err!("regex: the pattern ends with a lone '\\'.";
                        Invalid, Input)),
                };
                self.at += 1;
                match e {
                    'd' => { items.push(Item::Digit(true));  continue; }
                    'D' => { items.push(Item::Digit(false)); continue; }
                    'w' => { items.push(Item::Word(true));   continue; }
                    'W' => { items.push(Item::Word(false));  continue; }
                    's' => { items.push(Item::Space(true));  continue; }
                    'S' => { items.push(Item::Space(false)); continue; }
                    _   => items.push(Item::Ch(res!(escape_char(e)))),
                }
            } else {
                items.push(Item::Ch(c));
            }
            // A `-` between two single characters makes the pair a range.
            if self.peek() == Some('-')
                && self.pat.get(self.at + 1).map(|c| *c != ']').unwrap_or(false)
            {
                let lo = match items.pop() {
                    Some(Item::Ch(lo)) => lo,
                    // Not a range after all: `\d-x` keeps the shorthand and the `-` is literal.
                    Some(other)        => { items.push(other); continue; }
                    None               => continue,
                };
                self.at += 1; // the '-'
                let mut hi = match self.peek() {
                    Some(h) => h,
                    None    => return Err(err!("regex: unclosed '['."; Invalid, Input)),
                };
                self.at += 1;
                if hi == '\\' {
                    let e = match self.peek() {
                        Some(e) => e,
                        None    => return Err(err!("regex: the pattern ends with a lone '\\'.";
                            Invalid, Input)),
                    };
                    self.at += 1;
                    hi = res!(escape_char(e));
                }
                if hi < lo {
                    return Err(err!(
                        "regex: the range '{}-{}' runs backwards.", lo, hi; Invalid, Input));
                }
                items.push(Item::Range(lo, hi));
            }
        }
        if items.is_empty() {
            return Err(err!("regex: '[]' admits nothing."; Invalid, Input));
        }
        Ok(Node::Cls(Class { neg, items }))
    }

    /// The character at the cursor, or `None` at the end of the pattern.
    fn peek(&self) -> Option<char> {
        self.pat.get(self.at).copied()
    }
}

/// The character an escape stands for, outside the shorthand classes.
///
/// # Arguments
/// * `e` - The character after the backslash.
fn escape_char(e: char) -> Outcome<char> {
    Ok(match e {
        'n' => '\n',
        't' => '\t',
        'r' => '\r',
        '0' => '\0',
        // Every other escape is the character itself, which is how `\.` and `\\` work.
        _   => e,
    })
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
        assert!(m("a\\.c", "a.c"));
        assert!(!m("a\\.c", "abc"), "an escaped dot is a literal dot");
    }

    #[test]
    fn test_anchors_and_boundaries() {
        assert!(m("^abc", "abc"));
        assert!(!m("^abc", "xabc"));
        assert!(m("abc$", "xabc"));
        assert!(!m("abc$", "abcx"));
        assert!(m("\\bcat\\b", "the cat sat"));
        assert!(!m("\\bcat\\b", "concatenate"));
        assert!(m("\\Bcat", "concat"));
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
    fn test_a_pathological_pattern_says_it_gave_up_rather_than_saying_no() {
        // The classic exponential case. A "no match" here would be a lie: the answer is unknown.
        let r = Regex::new("(a+)+$").expect("compile");
        let hay = "a".repeat(40) + "b";
        let e = r.find(&hay).expect_err("this must not quietly answer 'no match'");
        assert!(fmt!("{}", e).contains("gave up"), "{}", e);
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
        // A minified file is one enormous line.  Recursing once per character against it overflows
        // the stack, and a stack overflow is an abort, not an answer -- in a browser it takes the
        // whole page down.  Reaching these assertions at all is the proof that it did not.
        let hay = "a".repeat(500_000);
        let star = Regex::new("^a*$").expect("compile");
        assert!(star.is_match(&hay).expect("a one-character repetition must be looped"));
        let mixed = Regex::new("a+b?a").expect("compile");
        assert!(mixed.is_match(&hay).expect("and must backtrack without recursing"));
        // A repeated GROUP cannot be looped, so it is depth-bounded instead: it must say it gave
        // up rather than take the process with it.
        let grouped = Regex::new("^(?:ab)+$").expect("compile");
        let pairs = "ab".repeat(200_000);
        let e = grouped.is_match(&pairs).expect_err("a group repeated that far must give up");
        assert!(fmt!("{}", e).contains("gave up"), "{}", e);
    }

    #[test]
    fn test_bad_patterns_are_refused_with_a_reason() {
        for (pat, want) in [
            ("(ab",     "unclosed '('"),
            ("[ab",     "unclosed '['"),
            ("a)",      "')' with no '('"),
            ("*a",      "nothing before it"),
            ("a{3,2}",  "at least 3"),
            ("[z-a]",   "runs backwards"),
            ("a\\",     "lone '\\'"),
        ] {
            let e = Regex::new(pat).expect_err(&fmt!("'{}' should not compile", pat));
            let msg = fmt!("{}", e);
            assert!(msg.contains(want), "'{}' should say '{}', said: {}", pat, want, msg);
        }
    }

    #[test]
    fn test_quote_makes_a_literal_of_anything() {
        let raw = "a.b*c(d)[e]{f}|g^h$i+j?k\\l";
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
}
