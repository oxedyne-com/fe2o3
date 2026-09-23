//! Why a second reader: the text decoder with `DecoderConfig::json` still reads JDAT's own forms,
//! so a kind annotation `(u64|5)`, a hex integer, an unquoted word, a single-quoted string, a
//! trailing comma, digits run together across a space and anything after the first value all
//! decode there. Found by the presentation audit of 2026-09-23. Input that must mean exactly one
//! thing comes through `Dat::decode_json_strict` instead.

use crate::{
    prelude::*,
    bdat::limits::DecodeLimits,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_num::string::NumberString;

use std::str;


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
        res!(limits.check_len(s.len()));
        let mut r = Reader {
            src:    s.as_bytes(),
            pos:    0,
            limits: *limits,
        };
        r.skip_ws();
        let value = res!(r.value(1));
        r.skip_ws();
        if r.pos < r.src.len() {
            return Err(err!(
                "JSON text holds one value, and more follows it at byte {} of {}.",
                r.pos, r.src.len();
            Invalid, Input, Decode));
        }
        Ok(value)
    }
}

struct Reader<'a> {
    src:    &'a [u8],
    pos:    usize,
    limits: DecodeLimits,
}

impl<'a> Reader<'a> {

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.peek() {
            self.pos += 1;
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

    fn value(&mut self, depth: usize) -> Outcome<Dat> {
        res!(self.limits.check_depth(depth, self.pos));
        match self.peek() {
            Some(b'{')              => self.object(depth),
            Some(b'[')              => self.array(depth),
            Some(b'"')              => Ok(Dat::Str(res!(self.string()))),
            Some(b't')              => self.literal("true", Dat::Bool(true)),
            Some(b'f')              => self.literal("false", Dat::Bool(false)),
            Some(b'n')              => self.literal("null", Dat::Opt(Box::new(None))),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(err!(
                "A JSON value begins with '{{', '[', '\"', a digit, '-', true, false or null, \
                and byte {} holds {}.", self.pos, self.found();
            Invalid, Input, Decode)),
        }
    }

    fn object(&mut self, depth: usize) -> Outcome<Dat> {
        let open = self.pos;
        self.pos += 1;
        let mut map = DaticleMap::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Dat::Map(map));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(err!(
                    "A member of the JSON object opened at byte {} is named by a string, and \
                    byte {} holds {}.", open, self.pos, self.found();
                Invalid, Input, Decode));
            }
            let at = self.pos;
            let key = Dat::Str(res!(self.string()));
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(err!(
                    "A member name in the JSON object opened at byte {} is followed by ':', and \
                    byte {} holds {}.", open, self.pos, self.found();
                Invalid, Input, Decode));
            }
            self.pos += 1;
            self.skip_ws();
            let value = res!(self.value(depth + 1));
            if map.contains_key(&key) {
                return Err(err!(
                    "The JSON object opened at byte {} names the member {:?} a second time, at \
                    byte {}.", open, key, at;
                Invalid, Input, Duplicate));
            }
            map.insert(key, value);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Dat::Map(map));
                },
                _ => return Err(err!(
                    "A member of the JSON object opened at byte {} is followed by ',' or '}}', \
                    and byte {} holds {}.", open, self.pos, self.found();
                Invalid, Input, Decode)),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Outcome<Dat> {
        let open = self.pos;
        self.pos += 1;
        let mut list = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Dat::List(list));
        }
        loop {
            self.skip_ws();
            list.push(res!(self.value(depth + 1)));
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Dat::List(list));
                },
                _ => return Err(err!(
                    "An element of the JSON array opened at byte {} is followed by ',' or ']', \
                    and byte {} holds {}.", open, self.pos, self.found();
                Invalid, Input, Decode)),
            }
        }
    }

    fn literal(&mut self, word: &str, dat: Dat) -> Outcome<Dat> {
        if !self.src[self.pos..].starts_with(word.as_bytes()) {
            return Err(err!(
                "The JSON value at byte {} begins like {} and is not it.", self.pos, word;
            Invalid, Input, Decode));
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
                    return Err(err!(
                        "The JSON number at byte {} has a leading zero.", start;
                    Invalid, Input, Decode));
                }
            },
            Some(b'1'..=b'9') => self.digit_run(),
            _ => return Err(err!(
                "The JSON number at byte {} has no digit after its sign; byte {} holds {}.",
                start, self.pos, self.found();
            Invalid, Input, Decode)),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(err!(
                    "The JSON number at byte {} has no digit after its point.", start;
                Invalid, Input, Decode));
            }
            self.digit_run();
        }
        if let Some(b'e' | b'E') = self.peek() {
            self.pos += 1;
            if let Some(b'+' | b'-') = self.peek() {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(err!(
                    "The JSON number at byte {} has no digit in its exponent.", start;
                Invalid, Input, Decode));
            }
            self.digit_run();
        }
        let text = res!(str::from_utf8(&self.src[start..self.pos]), Decode, UTF8);
        Dat::from_untyped_number(&res!(NumberString::new(text)))
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
