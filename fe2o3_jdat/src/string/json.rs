//! Why a second reader: the text decoder with `DecoderConfig::json` still reads JDAT's own forms,
//! so a kind annotation `(u64|5)`, a hex integer, an unquoted word, a single-quoted string, a
//! trailing comma, digits run together across a space and anything after the first value all
//! decode there. Found by the presentation audit of 2026-09-23. Input that must mean exactly one
//! thing comes through `Dat::decode_json_strict` instead.
//!
//! The same reader, set to `Syntax::Jdat`, reads a settings file: JSON as jdat writes it, with
//! `!...!` and `#...#` comments, trailing commas and `(kind|value)` annotations, and nothing else.
//! It exists because the text decoder's leniency, right for jdat's other uses, is wrong for a file
//! a person edits by hand: `{"size": 29x.4}` reads there as the string `"29x.4"`, a brace left
//! open returns the inner map and drops the rest, and a comment never closed becomes a key (found
//! building the Oxegen settings reader, D-20261005-12). Here each is an error naming the line, the
//! column and the key, and `Dat::jdat_to_json` hands the browser plain JSON to `JSON.parse`.

use crate::{
    prelude::*,
    bdat::limits::DecodeLimits,
    string::enc::escape_json_string,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_num::{
    float::Float64,
    string::NumberString,
};

use std::{
    collections::BTreeMap,
    str,
};


impl Dat {

    /// Decodes JSON text as RFC 8259 defines it and refuses everything else.
    ///
    /// An object becomes a `Dat::Map` with string keys, an array a `Dat::List`, a string a
    /// `Dat::Str`, `true` and `false` a `Dat::Bool` and `null` a `Dat::Opt` holding nothing. A
    /// number becomes the daticle the JDAT text decoder makes of the same digits. The input may
    /// hold one value, with only JSON's four whitespace characters around it.
    ///
    /// Refused as well, where RFC 8259 leaves the reader a choice: an object naming a member twice,
    /// and a `\u` escape that leaves half a surrogate pair, since neither has one meaning.
    /// `limits` bounds the length of the text and the depth of nesting, the root at depth 1.
    pub fn decode_json_strict(s: &str, limits: &DecodeLimits) -> Outcome<Self> {
        Self::decode_members(s, limits, Syntax::Json, false)
    }

    /// `decode_json_strict`, with each object read as a `Dat::OrdMap` that keeps its members in the
    /// order they were written. A plain `Dat::Map` sorts them by name, which is no matter to a
    /// reader that looks a member up and is all of it to one that reads the document through, as a
    /// scan for a run of words across its strings does.
    pub fn decode_json_strict_ordered(s: &str, limits: &DecodeLimits) -> Outcome<Self> {
        Self::decode_members(s, limits, Syntax::Json, true)
    }

    /// Decodes a settings file: JSON, plus `!...!` and `#...#` comments wherever whitespace may be,
    /// a trailing comma before `}` or `]`, and `(kind|value)` around a value, and refuses
    /// everything else.
    ///
    /// A kind is `u8` to `i64`, `f32`, `f64` or `str`, and the value must fit it; the value is
    /// what is kept. A number is a whole number within 2^53 - 1 of zero or an `f64` that is finite,
    /// which is all a browser holds exactly. A comment ends at the next mark of the kind it began
    /// with, so it can hold the other kind, quotes and braces, but not its own mark.
    ///
    /// Every refusal names the line and column, and the key in the file's own dotted form
    /// (`roles.qr.size`, `screens.main[2]`), so a person can find the fault. Anything that
    /// `Dat::decode_json_strict` refuses, apart from what is listed above, is refused here.
    pub fn decode_jdat_strict(s: &str, limits: &DecodeLimits) -> Outcome<Self> {
        Self::decode_members(s, limits, Syntax::Jdat, false)
    }

    /// `decode_jdat_strict`, with each object read as a `Dat::OrdMap` in the order written.
    pub fn decode_jdat_strict_ordered(s: &str, limits: &DecodeLimits) -> Outcome<Self> {
        Self::decode_members(s, limits, Syntax::Jdat, true)
    }

    /// Reads a settings file as `decode_jdat_strict_ordered` does and writes it as plain JSON,
    /// members in the order written, no comments, for a browser's `JSON.parse`. This is the
    /// function the wasm core exports, so the browser and the server read a file with one parser.
    pub fn jdat_to_json(s: &str, limits: &DecodeLimits) -> Outcome<String> {
        let dat = res!(Self::decode_jdat_strict_ordered(s, limits));
        let mut out = String::with_capacity(s.len());
        res!(write_json(&dat, &mut out));
        Ok(out)
    }

    fn decode_members(s: &str, limits: &DecodeLimits, syntax: Syntax, ordered: bool) -> Outcome<Self> {
        res!(limits.check_len(s.len()));
        let mut r = Reader {
            src:        s.as_bytes(),
            pos:        0,
            limits:     *limits,
            syntax,
            ordered,
            path:       Vec::new(),
        };
        res!(r.skip_ws());
        let value = res!(r.value(1));
        res!(r.skip_ws());
        if r.pos < r.src.len() {
            return Err(r.bad(r.pos, fmt!(
                "The text holds one value, and {} follows it", r.found())));
        }
        Ok(value)
    }
}

// Which text the reader accepts.
#[derive(Clone, Copy, PartialEq)]
enum Syntax {
    Json,   // RFC 8259 and nothing else
    Jdat,   // JSON, comments, trailing commas and kind annotations
}

// The largest whole number a browser holds exactly, 2^53 - 1.
const JS_SAFE_INTEGER: i128 = 9_007_199_254_740_991;

struct Reader<'a> {
    src:        &'a [u8],
    pos:        usize,
    limits:     DecodeLimits,
    syntax:     Syntax,
    ordered:    bool,           // are objects read with their members in the order written
    path:       Vec<String>,    // the keys and indices down to the value being read
}

impl<'a> Reader<'a> {

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    // Is the comment syntax on.
    fn jdat(&self) -> bool {
        self.syntax == Syntax::Jdat
    }

    // Whitespace, and in a settings file the comments between tokens.
    fn skip_ws(&mut self) -> Outcome<()> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\n' | b'\r') => self.pos += 1,
                Some(mark @ (b'!' | b'#')) if self.jdat() => {
                    let open = self.pos;
                    self.pos += 1;
                    loop {
                        match self.peek() {
                            Some(c) if c == mark => {
                                self.pos += 1;
                                break;
                            },
                            Some(_) => self.pos += 1,
                            None => return Err(self.bad(open, fmt!(
                                "The comment opened with '{}' is never closed, and a comment ends \
                                at the next '{}'", mark as char, mark as char))),
                        }
                    }
                },
                _ => return Ok(()),
            }
        }
    }

    fn digit_run(&mut self) {
        while let Some(b'0'..=b'9') = self.peek() {
            self.pos += 1;
        }
    }

    /// What the byte at the cursor is, for an error.
    fn found(&self) -> String {
        match self.peek() {
            Some(b) if b.is_ascii_graphic() => fmt!("'{}'", b as char),
            Some(b) => fmt!("byte 0x{:02x}", b),
            None    => fmt!("the end of the text"),
        }
    }

    // Where a byte offset is: the offset in plain JSON, and in a settings file the line and the
    // column in characters.
    fn place(&self, pos: usize) -> String {
        if !self.jdat() {
            return fmt!("byte {}", pos);
        }
        let pos = pos.min(self.src.len());
        let before = &self.src[..pos];
        let line = 1 + before.iter().filter(|b| **b == b'\n').count();
        let start = before.iter().rposition(|b| *b == b'\n').map_or(0, |n| n + 1);
        let col = 1 + String::from_utf8_lossy(&before[start..]).chars().count();
        fmt!("line {}, column {}", line, col)
    }

    // The key being read, in the file's own dotted form, with `[n]` for a list element.
    fn key(&self) -> String {
        let mut key = String::new();
        for seg in self.path.iter() {
            if !key.is_empty() && !seg.starts_with('[') {
                key.push('.');
            }
            key.push_str(seg);
        }
        key
    }

    // Where a refusal is: the place, and in a settings file the key.
    fn at(&self, pos: usize) -> String {
        if self.jdat() && !self.path.is_empty() {
            fmt!("{}, key {}", self.place(pos), self.key())
        } else {
            self.place(pos)
        }
    }

    // A refusal: what is wrong and where.
    fn bad(&self, pos: usize, what: String) -> Error<ErrTag> {
        err!("{} (at {}).", what, self.at(pos); Invalid, Input, Decode)
    }

    fn value(&mut self, depth: usize) -> Outcome<Dat> {
        res!(self.limits.check_depth(depth, self.pos));
        match self.peek() {
            Some(b'{')              => self.object(depth),
            Some(b'[')              => self.array(depth),
            Some(b'(') if self.jdat() => self.annotated(depth),
            Some(b'"')              => Ok(Dat::Str(res!(self.string()))),
            Some(b't')              => self.literal("true", Dat::Bool(true)),
            Some(b'f')              => self.literal("false", Dat::Bool(false)),
            Some(b'n')              => self.literal("null", Dat::Opt(Box::new(None))),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.not_a_value()),
        }
    }

    // The error arms below are out of line, so a level of nesting costs a frame without them: a
    // text nested to the limit must return an error and not exhaust the stack.
    #[inline(never)]
    fn not_a_value(&self) -> Error<ErrTag> {
        self.bad(self.pos, fmt!(
            "A value begins with '{{', '[', '\"', a digit, '-', true, false or null{}, and {} is \
            here",
            if self.jdat() { " or a '(kind|value)'" } else { "" }, self.found()))
    }

    #[inline(never)]
    fn not_a_name(&self, open: usize) -> Error<ErrTag> {
        if self.peek().is_none() {
            self.bad(self.pos, fmt!("The object opened at {} is never closed", self.place(open)))
        } else {
            self.bad(self.pos, fmt!("A member of the object opened at {} is named by a string, \
                and {} is here", self.place(open), self.found()))
        }
    }

    #[inline(never)]
    fn no_colon(&self) -> Error<ErrTag> {
        self.bad(self.pos, fmt!("A member name is followed by ':', and {} is here", self.found()))
    }

    #[inline(never)]
    fn twice(&self, at: usize, open: usize, key: &Dat, first: usize) -> Error<ErrTag> {
        self.bad(at, fmt!("The object opened at {} names the member {:?} a second time; the \
            first is at {}", self.place(open), key, self.place(first)))
    }

    #[inline(never)]
    fn after_member(&self, open: usize, what: &str, close: &str) -> Error<ErrTag> {
        if self.jdat() && self.peek().is_none() {
            self.bad(self.pos, fmt!("The {} opened at {} is never closed", what, self.place(open)))
        } else {
            self.bad(self.pos, fmt!("An element of the {} opened at {} is followed by ',' or \
                '{}', and {} is here", what, self.place(open), close, self.found()))
        }
    }

    fn object(&mut self, depth: usize) -> Outcome<Dat> {
        let open = self.pos;
        self.pos += 1;
        let mut map = DaticleMap::new();
        let mut order = Vec::new();     // the names in the order written, kept if the caller asked
        let mut firsts: BTreeMap<String, usize> = BTreeMap::new();  // where each name was first written
        res!(self.skip_ws());
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(self.members(map, order));
        }
        loop {
            res!(self.skip_ws());
            if self.peek() == Some(b'}') && self.jdat() && !map.is_empty() {
                self.pos += 1;  // a trailing comma
                return Ok(self.members(map, order));
            }
            if self.peek() != Some(b'"') {
                return Err(self.not_a_name(open));
            }
            let at = self.pos;
            let name = res!(self.string());
            let member = name.clone();
            let key = Dat::Str(name.clone());
            res!(self.skip_ws());
            self.path.push(name);
            if self.peek() != Some(b':') {
                return Err(self.no_colon());
            }
            self.pos += 1;
            res!(self.skip_ws());
            let value = res!(self.value(depth + 1));
            if let Some(first) = firsts.get(&member) {
                return Err(self.twice(at, open, &key, *first));
            }
            firsts.insert(member, at);
            if self.ordered {
                order.push(key.clone());
            }
            map.insert(key, value);
            res!(self.skip_ws());
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    self.path.pop();
                },
                Some(b'}') => {
                    self.pos += 1;
                    self.path.pop();
                    return Ok(self.members(map, order));
                },
                _ => return Err(self.after_member(open, "object", "}")),
            }
        }
    }

    // The members of an object, as a plain map or as one that remembers the order they came in.
    fn members(&self, mut map: DaticleMap, order: Vec<Dat>) -> Dat {
        if !self.ordered {
            return Dat::Map(map);
        }
        let mut out = OrdDaticleMap::new();
        for (i, key) in order.into_iter().enumerate() {
            if let Some(value) = map.remove(&key) {
                out.insert(MapKey::new(i as u64, key), value);
            }
        }
        Dat::OrdMap(out)
    }

    fn array(&mut self, depth: usize) -> Outcome<Dat> {
        let open = self.pos;
        self.pos += 1;
        let mut list = Vec::new();
        res!(self.skip_ws());
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Dat::List(list));
        }
        loop {
            res!(self.skip_ws());
            if self.peek() == Some(b']') && self.jdat() && !list.is_empty() {
                self.pos += 1;  // a trailing comma
                return Ok(Dat::List(list));
            }
            self.path.push(fmt!("[{}]", list.len()));
            list.push(res!(self.value(depth + 1)));
            res!(self.skip_ws());
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    self.path.pop();
                },
                Some(b']') => {
                    self.pos += 1;
                    self.path.pop();
                    return Ok(Dat::List(list));
                },
                _ => return Err(self.after_member(open, "list", "]")),
            }
        }
    }

    // `(kind|value)`: the value, once it is known to fit the kind.
    #[inline(never)]
    fn annotated(&mut self, depth: usize) -> Outcome<Dat> {
        let open = self.pos;
        self.pos += 1;
        let start = self.pos;
        while let Some(b'a'..=b'z' | b'0'..=b'9') = self.peek() {
            self.pos += 1;
        }
        let label = res!(str::from_utf8(&self.src[start..self.pos]), Decode, UTF8).to_string();
        res!(self.skip_ws());
        if self.peek() != Some(b'|') {
            return Err(self.bad(self.pos, fmt!(
                "A kind annotation is (kind|value), and {} is here where '|' should follow the \
                kind", self.found())));
        }
        self.pos += 1;
        res!(self.skip_ws());
        let value = res!(self.value(depth + 1));
        res!(self.skip_ws());
        if self.peek() != Some(b')') {
            return Err(self.bad(self.pos, fmt!(
                "The kind annotation opened at {} is closed by ')', and {} is here",
                self.place(open), self.found())));
        }
        self.pos += 1;
        let whole = |d: &Dat| match d {
            Dat::U64(n) => Some(*n as i128),
            Dat::I64(n) => Some(*n as i128),
            _ => None,
        };
        let span: Option<(i128, i128)> = match label.as_str() {
            "u8"    => Some((0, u8::MAX as i128)),
            "u16"   => Some((0, u16::MAX as i128)),
            "u32"   => Some((0, u32::MAX as i128)),
            "u64"   => Some((0, u64::MAX as i128)),
            "i8"    => Some((i8::MIN as i128, i8::MAX as i128)),
            "i16"   => Some((i16::MIN as i128, i16::MAX as i128)),
            "i32"   => Some((i32::MIN as i128, i32::MAX as i128)),
            "i64"   => Some((i64::MIN as i128, i64::MAX as i128)),
            _       => None,
        };
        if let Some((lo, hi)) = span {
            return match whole(&value) {
                Some(n) if n >= lo && n <= hi => Ok(value),
                Some(n) => Err(self.bad(open, fmt!("The value {} does not fit the kind {}", n, label))),
                None => Err(self.bad(open, fmt!("The kind {} takes a whole number", label))),
            };
        }
        match (label.as_str(), &value) {
            ("f32" | "f64", Dat::F64(_)) | ("str", Dat::Str(_)) => Ok(value),
            ("f32" | "f64", _) if whole(&value).is_some() => Ok(value),
            ("f32" | "f64" | "str", _) => Err(self.bad(open, fmt!(
                "The kind {} does not take this value", label))),
            _ => Err(self.bad(open, fmt!(
                "The kind {:?} is not one a settings file annotates; use u8, u16, u32, u64, i8, \
                i16, i32, i64, f32, f64 or str", label))),
        }
    }

    fn literal(&mut self, word: &str, dat: Dat) -> Outcome<Dat> {
        if !self.src[self.pos..].starts_with(word.as_bytes()) {
            return Err(self.bad(self.pos, fmt!("The value begins like {} and is not it", word)));
        }
        self.pos += word.len();
        Ok(dat)
    }

    /// A number: `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?` and nothing else, so no
    /// plus sign, leading zero, bare point, radix prefix or digit separator.
    fn number(&mut self) -> Outcome<Dat> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.pos += 1;
                if let Some(b'0'..=b'9') = self.peek() {
                    return Err(self.bad(start, fmt!("The number has a leading zero")));
                }
            },
            Some(b'1'..=b'9') => self.digit_run(),
            _ => return Err(self.bad(start, fmt!(
                "The number has no digit after its sign, and {} is here", self.found()))),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.bad(start, fmt!("The number has no digit after its point")));
            }
            self.digit_run();
        }
        if let Some(b'e' | b'E') = self.peek() {
            self.pos += 1;
            if let Some(b'+' | b'-') = self.peek() {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.bad(start, fmt!("The number has no digit in its exponent")));
            }
            self.digit_run();
        }
        let text = res!(str::from_utf8(&self.src[start..self.pos]), Decode, UTF8);
        if !self.jdat() {
            return Dat::from_untyped_number(&res!(NumberString::new(text)));
        }
        // A settings number is one a browser holds exactly.
        if !text.contains(|c| matches!(c, '.' | 'e' | 'E')) {
            return match text.parse::<i128>() {
                Ok(n) if n.abs() <= JS_SAFE_INTEGER =>
                    Ok(if n < 0 { Dat::I64(n as i64) } else { Dat::U64(n as u64) }),
                _ => Err(self.bad(start, fmt!(
                    "The whole number {} is beyond 2^53 - 1, which a browser cannot hold exactly; \
                    write it as a string", text))),
            };
        }
        match text.parse::<f64>() {
            Ok(f) if f.is_finite() => Ok(Dat::F64(Float64(f))),
            _ => Err(self.bad(start, fmt!("The number {} is not a finite f64", text))),
        }
    }

    /// A string, cursor on its opening quote. Characters below U+0020 must be escaped, and the
    /// only escapes are RFC 8259's eight and `\u` with four hex digits.
    fn string(&mut self) -> Outcome<String> {
        let open = self.pos;
        self.pos += 1;
        let mut out = String::new();
        let mut run = self.pos; // start of the unescaped run being read
        loop {
            match self.peek() {
                Some(b'"') => {
                    out.push_str(res!(str::from_utf8(&self.src[run..self.pos]), Decode, UTF8));
                    self.pos += 1;
                    return Ok(out);
                },
                Some(b'\\') => {
                    out.push_str(res!(str::from_utf8(&self.src[run..self.pos]), Decode, UTF8));
                    self.pos += 1;
                    out.push(res!(self.escape()));
                    run = self.pos;
                },
                Some(b) if b < 0x20 => return Err(self.bad(self.pos, fmt!(
                    "The string opened at {} holds the control byte 0x{:02x}, which must be \
                    escaped", self.place(open), b))),
                Some(_) => self.pos += 1,
                None => return Err(self.bad(open, fmt!("The string is never closed"))),
            }
        }
    }

    /// The character an escape stands for, cursor just past its backslash.
    fn escape(&mut self) -> Outcome<char> {
        let at = self.pos - 1;
        let c = match self.peek() {
            Some(c) => c,
            None => return Err(self.bad(at, fmt!("The text ends inside an escape"))),
        };
        self.pos += 1;
        let ch = match c {
            b'"'    => '"',
            b'\\'   => '\\',
            b'/'    => '/',
            b'b'    => '\u{0008}',
            b'f'    => '\u{000C}',
            b'n'    => '\n',
            b'r'    => '\r',
            b't'    => '\t',
            b'u'    => {
                let unit = res!(self.hex4());
                let cp = match unit {
                    0xD800..=0xDBFF => {
                        if !self.src[self.pos..].starts_with(b"\\u") {
                            return Err(self.bad(at, fmt!(
                                "The escape is a high surrogate with no low surrogate escape \
                                after it")));
                        }
                        self.pos += 2;
                        let low = res!(self.hex4());
                        if !(0xDC00..=0xDFFF).contains(&low) {
                            return Err(self.bad(at, fmt!(
                                "The escape is a high surrogate followed by \\u{:04X}, which is \
                                not a low surrogate", low)));
                        }
                        0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00)
                    },
                    0xDC00..=0xDFFF => return Err(self.bad(at, fmt!(
                        "The escape is a low surrogate with no high surrogate before it"))),
                    _ => unit,
                };
                match char::from_u32(cp) {
                    Some(ch) => ch,
                    None => return Err(self.bad(at, fmt!(
                        "The escape names U+{:X}, which is not a character", cp))),
                }
            },
            _ => return Err(self.bad(at, fmt!(
                "The escape is '\\{}', which JSON does not have", (c as char).escape_default()))),
        };
        Ok(ch)
    }

    fn hex4(&mut self) -> Outcome<u32> {
        let mut v = 0u32;
        for _ in 0..4 {
            let d = match self.peek() {
                Some(b @ b'0'..=b'9') => b - b'0',
                Some(b @ b'a'..=b'f') => b - b'a' + 10,
                Some(b @ b'A'..=b'F') => b - b'A' + 10,
                _ => return Err(self.bad(self.pos, fmt!(
                    "A \\u escape takes four hex digits, and {} is here", self.found()))),
            };
            v = (v << 4) | d as u32;
            self.pos += 1;
        }
        Ok(v)
    }
}

// A daticle as plain JSON, members in the order held, for what a settings reader produces.
fn write_json(dat: &Dat, out: &mut String) -> Outcome<()> {
    match dat {
        Dat::Bool(b)        => out.push_str(if *b { "true" } else { "false" }),
        Dat::Opt(o) if o.is_none() => out.push_str("null"),
        Dat::U64(n)         => out.push_str(&fmt!("{}", n)),
        Dat::I64(n)         => out.push_str(&fmt!("{}", n)),
        Dat::F64(Float64(f)) if f.is_finite() => out.push_str(&fmt!("{}", f)),
        Dat::Str(s)         => {
            out.push('"');
            out.push_str(&escape_json_string(s));
            out.push('"');
        },
        Dat::List(items)    => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                res!(write_json(item, out));
            }
            out.push(']');
        },
        Dat::OrdMap(m)      => {
            out.push('{');
            for (i, (k, v)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                res!(write_json(k.dat(), out));
                out.push(':');
                res!(write_json(v, out));
            }
            out.push('}');
        },
        Dat::Map(m)         => {
            out.push('{');
            for (i, (k, v)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                res!(write_json(k, out));
                out.push(':');
                res!(write_json(v, out));
            }
            out.push('}');
        },
        other => return Err(err!(
            "A daticle of kind {:?} has no plain JSON form.", other.kind();
            Invalid, Input, Unimplemented)),
    }
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;

    fn strict(s: &str) -> Outcome<Dat> {
        Dat::decode_json_strict(s, &DecodeLimits::text())
    }

    /// Nesting past the limit is refused before it is descended into.
    #[test]
    fn test_depth_limit_00() -> Outcome<()> {
        let limits = DecodeLimits::new(3, 1024);
        res!(Dat::decode_json_strict("[[1]]", &limits));
        assert!(Dat::decode_json_strict("[[[1]]]", &limits).is_err(), "depth 4 is past 3");
        let deep = fmt!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
        assert!(strict(&deep).is_err(), "a hundred thousand levels must be refused, not recursed");
        Ok(())
    }

    /// The length limit is checked before any byte is read.
    #[test]
    fn test_length_limit_00() {
        let limits = DecodeLimits::new(8, 4);
        assert!(Dat::decode_json_strict("12345", &limits).is_err(), "five bytes is past four");
    }
}
