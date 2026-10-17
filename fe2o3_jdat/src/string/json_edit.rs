//! One JSON object's text, edited member by member in place.
//!
//! A file of records, one JSON object to a line, is often written by more than one program, and
//! a newer writer adds members an older one has never heard of. A writer that decodes a record,
//! changes a field and encodes it again drops those members, reorders the rest and changes the
//! spacing. [`JsonObjectText`] changes the bytes of the members it is asked to and no others.
//!
//! The text is held to RFC 8259 as `Dat::decode_json_strict` holds it, and refused on the same
//! grounds, an object naming a member twice among them. The walk here builds no daticle: it reads
//! the grammar and records where each top-level member lies.

use crate::{
    bdat::limits::DecodeLimits,
    string::enc::escape_json_string,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    ops::Range,
    str,
};


/// One top-level member of a [`JsonObjectText`]: its name as the text says it, unescaped, the byte
/// its name's opening quote stands at, and the span of its value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JsonMember {
    pub key:    String,
    pub start:  usize,
    pub val:    Range<usize>,
}

/// One JSON object held as its text and edited member by member, in place.
///
/// A member is set, added or removed and every byte outside it stays as it was: the other
/// members, known to the caller or not, their order, the whitespace and the line endings.
///
/// Anything that is not exactly one JSON object, with only JSON whitespace around it, is refused,
/// and so is an object that names a member twice, since there is then no one member to edit. Only
/// top-level members are edited; a member of the same name nested deeper is never touched.
#[derive(Clone, Debug)]
pub struct JsonObjectText {
    text:       String,
    members:    Vec<JsonMember>,
    close:      usize,  // the byte of the object's closing brace
    limits:     DecodeLimits,
}

impl JsonObjectText {

    /// Reads `text` as one JSON object under the text limits.
    pub fn new(text: &str) -> Outcome<Self> {
        Self::new_with_limits(text, &DecodeLimits::text())
    }

    /// Reads `text` as one JSON object; `limits` bound its length and nesting, the object at depth
    /// 1, here and after each edit.
    pub fn new_with_limits(text: &str, limits: &DecodeLimits) -> Outcome<Self> {
        let (members, close) = res!(Self::scan(text, limits));
        Ok(Self { text: text.to_string(), members, close, limits: *limits })
    }

    pub fn as_str(&self) -> &str            { &self.text }
    pub fn into_string(self) -> String      { self.text }
    pub fn members(&self) -> &[JsonMember]  { &self.members }

    /// The names of the top-level members, in the order written.
    pub fn keys(&self) -> Vec<String> {
        self.members.iter().map(|m| m.key.clone()).collect()
    }

    /// The text of the value of the member named `key`, as written.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.find(key).map(|i| &self.text[self.members[i].val.clone()])
    }

    /// Gives the member named `key` the value `val`, JSON text, in place of the value it had, or
    /// adds it after the last member when there is none. `val` must be exactly one JSON value,
    /// and a refused `val` changes nothing.
    pub fn set(&mut self, key: &str, val: &str) -> Outcome<()> {
        res!(self.limits.check_len(val.len()));
        let mut w = Walk { b: val.as_bytes(), pos: 0, limits: self.limits };
        w.ws();
        let v0 = w.pos;
        res!(w.value(2));
        let v1 = w.pos;
        w.ws();
        if w.pos < w.b.len() {
            return Err(err!(
                "The value for the member {:?} must be one JSON value, and more follows it at \
                byte {} of {}.", key, w.pos, w.b.len();
            Invalid, Input, Decode));
        }
        let val = &val[v0..v1];
        let mut text = self.text.clone();
        match self.find(key) {
            Some(i) => text.replace_range(self.members[i].val.clone(), val),
            None => {
                let (at, sep) = match self.members.last() {
                    Some(m) => (m.val.end, ","),
                    None    => (self.close, ""),
                };
                text.insert_str(at, &fmt!("{}\"{}\":{}", sep, escape_json_string(key), val));
            },
        }
        self.commit(text)
    }

    /// Removes the member named `key` with the comma that joined it, the one before it or, for
    /// the first member, the one after. Whether there was such a member.
    pub fn remove(&mut self, key: &str) -> Outcome<bool> {
        let i = match self.find(key) {
            Some(i) => i,
            None    => return Ok(false),
        };
        let ms = &self.members;
        let span = if i > 0 {
            ms[i - 1].val.end..ms[i].val.end
        } else if let Some(next) = ms.get(1) {
            ms[0].start..next.start
        } else {
            ms[0].start..ms[0].val.end
        };
        let mut text = self.text.clone();
        text.replace_range(span, "");
        res!(self.commit(text));
        Ok(true)
    }

    fn find(&self, key: &str) -> Option<usize> {
        self.members.iter().position(|m| m.key == key)
    }

    // An edit is read back before it is kept, so the text always holds one object.
    fn commit(&mut self, text: String) -> Outcome<()> {
        let (members, close) = res!(Self::scan(&text, &self.limits));
        self.text = text;
        self.members = members;
        self.close = close;
        Ok(())
    }

    fn scan(text: &str, limits: &DecodeLimits) -> Outcome<(Vec<JsonMember>, usize)> {
        res!(limits.check_len(text.len()));
        let mut w = Walk { b: text.as_bytes(), pos: 0, limits: *limits };
        w.ws();
        if w.peek() != Some(b'{') {
            return Err(err!(
                "The text of a JSON object opens with '{{', and byte {} holds {}.",
                w.pos, w.found();
            Invalid, Input, Decode));
        }
        res!(limits.check_depth(1, w.pos));
        let (members, close) = res!(w.object(1));
        w.ws();
        if w.pos < w.b.len() {
            return Err(err!(
                "The text of a JSON object holds one object, and more follows it at byte {} of \
                {}.", w.pos, w.b.len();
            Invalid, Input, Decode));
        }
        Ok((members, close))
    }
}

/// A walk through JSON text that checks the grammar and builds no value.
struct Walk<'a> {
    b:      &'a [u8],
    pos:    usize,
    limits: DecodeLimits,
}

impl<'a> Walk<'a> {

    fn peek(&self) -> Option<u8> {
        self.b.get(self.pos).copied()
    }

    fn ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.peek() {
            self.pos += 1;
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.pos;
        while let Some(b'0'..=b'9') = self.peek() {
            self.pos += 1;
        }
        self.pos - start
    }

    /// What the byte at the cursor is, for an error.
    fn found(&self) -> String {
        match self.peek() {
            Some(b) if b.is_ascii_graphic() => fmt!("'{}'", b as char),
            Some(b) => fmt!("byte 0x{:02x}", b),
            None    => fmt!("the end of the text"),
        }
    }

    fn value(&mut self, depth: usize) -> Outcome<()> {
        res!(self.limits.check_depth(depth, self.pos));
        match self.peek() {
            Some(b'{')                  => self.object(depth).map(|_| ()),
            Some(b'[')                  => self.array(depth),
            Some(b'"')                  => self.string().map(|_| ()),
            Some(b't')                  => self.literal("true"),
            Some(b'f')                  => self.literal("false"),
            Some(b'n')                  => self.literal("null"),
            Some(b'-' | b'0'..=b'9')    => self.number(),
            _ => Err(err!(
                "A JSON value begins with '{{', '[', '\"', a digit, '-', true, false or null, \
                and byte {} holds {}.", self.pos, self.found();
            Invalid, Input, Decode)),
        }
    }

    /// The members of the object whose '{' is at the cursor, and the byte of its '}'.
    fn object(&mut self, depth: usize) -> Outcome<(Vec<JsonMember>, usize)> {
        let open = self.pos;
        self.pos += 1;
        let mut members: Vec<JsonMember> = Vec::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok((members, self.pos - 1));
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return Err(err!(
                    "A member of the JSON object opened at byte {} is named by a string, and \
                    byte {} holds {}.", open, self.pos, self.found();
                Invalid, Input, Decode));
            }
            let start = self.pos;
            let key = res!(self.string());
            self.ws();
            if self.peek() != Some(b':') {
                return Err(err!(
                    "A member name in the JSON object opened at byte {} is followed by ':', and \
                    byte {} holds {}.", open, self.pos, self.found();
                Invalid, Input, Decode));
            }
            self.pos += 1;
            self.ws();
            let v0 = self.pos;
            res!(self.value(depth + 1));
            if members.iter().any(|m| m.key == key) {
                return Err(err!(
                    "The JSON object opened at byte {} names the member {:?} a second time, at \
                    byte {}.", open, key, start;
                Invalid, Input, Duplicate));
            }
            members.push(JsonMember { key, start, val: v0..self.pos });
            self.ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok((members, self.pos - 1));
                },
                _ => return Err(err!(
                    "A member of the JSON object opened at byte {} is followed by ',' or '}}', \
                    and byte {} holds {}.", open, self.pos, self.found();
                Invalid, Input, Decode)),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Outcome<()> {
        let open = self.pos;
        self.pos += 1;
        self.ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(());
        }
        loop {
            self.ws();
            res!(self.value(depth + 1));
            self.ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(());
                },
                _ => return Err(err!(
                    "An item of the JSON array opened at byte {} is followed by ',' or ']', and \
                    byte {} holds {}.", open, self.pos, self.found();
                Invalid, Input, Decode)),
            }
        }
    }

    fn literal(&mut self, word: &str) -> Outcome<()> {
        if self.b[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(())
        } else {
            Err(err!(
                "Byte {} begins a word that is not the JSON literal {}.", self.pos, word;
            Invalid, Input, Decode))
        }
    }

    fn number(&mut self) -> Outcome<()> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.pos += 1;
                if let Some(b'0'..=b'9') = self.peek() {
                    return Err(err!(
                        "The JSON number at byte {} has a leading zero.", start;
                    Invalid, Input, Decode));
                }
            },
            Some(b'1'..=b'9') => { self.digits(); },
            _ => return Err(err!(
                "The JSON number at byte {} has no digit before byte {}, which holds {}.",
                start, self.pos, self.found();
            Invalid, Input, Decode)),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if self.digits() == 0 {
                return Err(err!(
                    "The JSON number at byte {} has no digit after its '.'.", start;
                Invalid, Input, Decode));
            }
        }
        if let Some(b'e' | b'E') = self.peek() {
            self.pos += 1;
            if let Some(b'+' | b'-') = self.peek() {
                self.pos += 1;
            }
            if self.digits() == 0 {
                return Err(err!(
                    "The JSON number at byte {} has no digit in its exponent.", start;
                Invalid, Input, Decode));
            }
        }
        Ok(())
    }

    /// The string whose '"' is at the cursor, unescaped.
    fn string(&mut self) -> Outcome<String> {
        let open = self.pos;
        self.pos += 1;
        let mut out = String::new();
        let mut run = self.pos; // start of the unescaped run being read
        loop {
            match self.peek() {
                Some(b'"') => {
                    out.push_str(res!(str::from_utf8(&self.b[run..self.pos]), Decode, UTF8));
                    self.pos += 1;
                    return Ok(out);
                },
                Some(b'\\') => {
                    out.push_str(res!(str::from_utf8(&self.b[run..self.pos]), Decode, UTF8));
                    self.pos += 1;
                    out.push(res!(self.escape()));
                    run = self.pos;
                },
                Some(b) if b < 0x20 => return Err(err!(
                    "The JSON string opened at byte {} holds the control byte 0x{:02x} at byte \
                    {}, which must be escaped.", open, b, self.pos;
                Invalid, Input, Decode)),
                Some(_) => self.pos += 1,
                None => return Err(err!(
                    "The JSON string opened at byte {} is never closed.", open;
                Invalid, Input, Decode)),
            }
        }
    }

    /// The character an escape stands for, cursor just past its backslash.
    fn escape(&mut self) -> Outcome<char> {
        let at = self.pos - 1;
        let c = match self.peek() {
            Some(c) => c,
            None => return Err(err!(
                "The JSON text ends inside the escape at byte {}.", at;
            Invalid, Input, Decode)),
        };
        self.pos += 1;
        Ok(match c {
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
                        if !self.b[self.pos..].starts_with(b"\\u") {
                            return Err(err!(
                                "The escape at byte {} is a high surrogate with no low \
                                surrogate escape after it.", at;
                            Invalid, Input, Decode));
                        }
                        self.pos += 2;
                        let low = res!(self.hex4());
                        if !(0xDC00..=0xDFFF).contains(&low) {
                            return Err(err!(
                                "The escape at byte {} is a high surrogate followed by \
                                \\u{:04X}, which is not a low surrogate.", at, low;
                            Invalid, Input, Decode));
                        }
                        0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00)
                    },
                    0xDC00..=0xDFFF => return Err(err!(
                        "The escape at byte {} is a low surrogate with no high surrogate \
                        before it.", at;
                    Invalid, Input, Decode)),
                    _ => unit,
                };
                match char::from_u32(cp) {
                    Some(ch) => ch,
                    None => return Err(err!(
                        "The escape at byte {} names U+{:X}, which is not a character.", at, cp;
                    Invalid, Input, Decode)),
                }
            },
            _ => return Err(err!(
                "The escape at byte {} is '\\{}', which JSON does not have.", at,
                (c as char).escape_default();
            Invalid, Input, Decode)),
        })
    }

    fn hex4(&mut self) -> Outcome<u32> {
        let mut v = 0u32;
        for _ in 0..4 {
            let d = match self.peek() {
                Some(b @ b'0'..=b'9') => b - b'0',
                Some(b @ b'a'..=b'f') => b - b'a' + 10,
                Some(b @ b'A'..=b'F') => b - b'A' + 10,
                _ => return Err(err!(
                    "A \\u escape takes four hex digits, and byte {} holds {}.",
                    self.pos, self.found();
                Invalid, Input, Decode)),
            };
            v = (v << 4) | d as u32;
            self.pos += 1;
        }
        Ok(v)
    }
}
