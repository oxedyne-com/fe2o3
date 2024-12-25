//! `Base2x` is a more flexible Unicode alternative to ASCII-only Base64, allowing an arbitrary
//! user-supplied alphabet and padding characters, but only for binary tokens of fixed size.  The
//! alphabet must consist of 2^X Unicode characters for X up to 32.  Each alphabet character is
//! associated with a binary token of fixed length X.
//!
//! Encoding schemes exist (e.g. Base62) that allow an arbitrary alphabet length by using tokens of
//! variable size.  Base2x is limited to lengths of 2^x for simplicity and speed.
//!
//! Encoding a byte slice to a string automatically appends padding characters if necessary.  In
//! this case, the padding always consists of precisely three characters, namely the alphabet
//! character associated with a zero-padded partial token, the user-specified padding separator
//! (e.g. '=') and a padding character from the user-supplied padding set.  The index of the
//! padding character gives the number of padding bits, ranging from 1 to X-1.
//!
//! Note that this fixed size padding scheme potentially adds one character more than the Base64
//! variable scheme using one or two '=' characters.  A `normalise` method is provided to add this
//! padding if it is not present.  Decoding an arbitrary string that uses the correct alphabet
//! without padding executes without error, but the last byte may not match a proper encoding of
//! the original binary.  It's best to always use padding, and to normalise if unsure.

use oxedize_fe2o3_core::prelude::*;

use std::collections::HashSet;


pub const MAX_X: usize = 32;
pub const MAX_A: usize = 2_usize.pow(MAX_X as u32);

// Some const instances.
pub const BASE64: Base2x<64, 6>         = base64();
pub const HEMATITE64: Base2x<64, 6>     = hematite64();
pub const HEMATITE32: Base2x<32, 5>     = hematite32();
pub const HEX: Base2x<16, 4>            = hex();

pub const BASE64_ALPHABET: [char; 64] = [
    // "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    // This is not a replacement for standard Base64, because a different padding scheme is used.
    'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H',
    'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P',
    'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X',
    'Y', 'Z', 'a', 'b', 'c', 'd', 'e', 'f',
    'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n',
    'o', 'p', 'q', 'r', 's', 't', 'u', 'v',
    'w', 'x', 'y', 'z', '0', '1', '2', '3',
    '4', '5', '6', '7', '8', '9', '+', '/',
];
pub const fn base64() -> Base2x<64, 6> {
    Base2x::<64, 6>{
        alphabet: BASE64_ALPHABET,
        padding: Some(('=', ['1', '2', '3', '4', '5', '_' ])),
    }
}

pub const HEMATITE64_ALPHABET: [char; 64] = [
    // Start with Base64:
    // "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    // place numerals first and make the substitutions
    //  'I' -> '#'
    //  'O' -> '@',
    //  'l' -> '%',
    //  'o' -> '-',
    //  '/' -> '*',
    // "0123456789ABCDEFGH#JKLMN@PQRSTUVWXYZabcdef%hijklmn-pqrstuvwxyz+/";
    '0', '1', '2', '3', '4', '5', '6', '7',
    '8', '9', 'A', 'B', 'C', 'D', 'E', 'F',
    'G', 'H', '#', 'J', 'K', 'L', 'M', 'N',
    '@', 'P', 'Q', 'R', 'S', 'T', 'U', 'V',
    'W', 'X', 'Y', 'Z', 'a', 'b', 'c', 'd',
    'e', 'f', 'g', 'h', 'i', 'j', 'k', '%',
    'm', 'n', '-', 'p', 'q', 'r', 's', 't',
    'u', 'v', 'w', 'x', 'y', 'z', '+', '*',
];
pub const fn hematite64() -> Base2x<64, 6> {
    Base2x::<64, 6>{
        alphabet: HEMATITE64_ALPHABET,
        padding: Some(('=', ['1', '2', '3', '4', '5', '_' ])),
    }
}

pub const HEX_ALPHABET: [char; 16] = [
    '0', '1', '2', '3', '4', '5', '6', '7',
    '8', '9', 'A', 'B', 'C', 'D', 'E', 'F',
];
pub const fn hex() -> Base2x<16, 4> {
    Base2x::<16, 4>{
        alphabet: HEX_ALPHABET,
        padding: Some(('=', ['1', '2', '3', '_' ])),
    }
}

pub const HEMATITE32_ALPHABET: [char; 32] = [
    // Simply extend the use of alphabetical characters from 'A'..'F' to 'A'..'M', 'a'..'f' to
    // 'a'..'m', excluding 'I', 'i', 'L' and 'l'.
    '0', '1', '2', '3', '4', '5', '6', '7',
    '8', '9', 'A', 'B', 'C', 'D', 'E', 'F',
    'G', 'H', 'J', 'K', 'M', 'a', 'b', 'c',
    'd', 'e', 'f', 'g', 'h', 'j', 'k', 'm',
];
pub const fn hematite32() -> Base2x<32, 5> {
    Base2x::<32, 5>{
        alphabet: HEMATITE32_ALPHABET,
        padding: Some(('=', ['1', '2', '3', '4', '_' ])),
    }
}

pub const BINHEX4_ALPHABET: &'static str =
    // Removes '7', 'O', 'g', 'n', 'o', "stuvwxyz"
    "!\"#$%&'()*+,-012345689@ABCDEFGHIJKLMNPQRSTUVXYZ[`abcdefhijklmpqr";

pub const fn alphabet_size(x: u32) -> usize {
    2_usize.pow(x)
}

pub struct Base2x<
    const A: usize, // Alphabet length, 2^X.
    const X: usize, // Token (binary) length.
> {
    alphabet:   [char; A],
    padding:    Option<(char, [char; X])>,
}

impl<
    const A: usize,
    const X: usize,
>
    Base2x<A, X>
{

    const TOKEN_MASK: u64 = Self::lower_bit_mask(X);

    pub fn new(
        alphabet:   [char; A],
        padding:    Option<(char, [char; X])>,
    )
        -> Outcome<Self>
    {
        if X < 2 || X > MAX_X {
            return Err(err!(errmsg!(
                "The token size X provided, {}, must be at least 2 and no \
                more than {}.", X, MAX_X,
            ), Invalid, Input));
        }
        let padding_reqd = !(X == 2 || X == 4 || X == 8);
        if !padding_reqd && padding.is_some() {
            return Err(err!(errmsg!(
                "Padding is not required for a token size of {}, so padding parameters \
                should be set to None.", X,
            ), Invalid, Input, Mismatch));
        }
        if padding_reqd && padding.is_none() {
            return Err(err!(errmsg!(
                "Padding is required for a token size {} but no padding characters \
                were provided.", X,
            ), Invalid, Input, Missing));
        }
        let x = match Self::validate_size(A) {
            Some(x) => x,
            None => return Err(err!(errmsg!(
                "Alphabet length {} must be a power of 2, and less than {}",
                A, MAX_A,
            ), Invalid, Input)),
        };
        if x != X {
            return Err(err!(errmsg!(
                "The alphabet length {} must equal 2^X ({}) where the X \
                supplied is {}.", A, 2_usize.pow(X as u32), X,
            ), Invalid, Input));
        }
        let non_unique = Self::find_non_unique_chars(&alphabet, &padding.map(|(c, _)| c));
        if non_unique.len() > 0 {
            return Err(err!(errmsg!(
                "Alphabet characters must be unique, these ones repeat: {:?}.", non_unique,
            ), Invalid, Input));
        }
        if padding_reqd {
            if let Some(p) = padding {
                let non_unique = Self::find_non_unique_chars(&p.1, &Some(p.0));
                if non_unique.len() > 0 {
                    return Err(err!(errmsg!(
                        "Padding set characters must be unique, these ones repeat: {:?}.",
                        non_unique,
                    ), Invalid, Input));
                }
            }
        }
        Ok(Self {
            alphabet,
            padding,
        })
    }

    /// Returns a set of non-unique characters in the proposed alphabet.  None of the characters
    /// should match the padding character.
    fn find_non_unique_chars<const T: usize>(a: &[char; T], pad_sep: &Option<char>) -> HashSet<char> {
        let mut seen = HashSet::new();
        let mut non_unique = HashSet::new();
    
        for &c in a {
            if !seen.insert(c) || Some(c) == *pad_sep {
                non_unique.insert(c);
            }
        }
    
        non_unique
    }

    /// Checks the size of the alphabet and returns the base two logarithm of the size if it is an
    /// integer, or `None` otherwise.
    fn validate_size(n: usize) -> Option<usize> {
        if n == 0 || n > MAX_A { return None; }
    
        let mut exponent = 0;
        let mut value = n;
    
        while value != 1 {
            if value % 2 != 0 {
                return None;
            }
            value /= 2;
            exponent += 1;
        }
    
        Some(exponent)
    }

    /// A helper for preparing alphabets into the required array form.
    pub fn prepare_alphabet(s: &str) -> Outcome<[char; A]> {
        let chars: Vec<char> = s.chars().collect();
        if chars.len() != A {
            return Err(err!(errmsg!(
                "The number of characters in the given alphabet, {}, does not match \
                your generic parameter, {}.", chars.len(), A,
            ), Invalid, Size, Input, Mismatch));
        }
        match chars.try_into() {
            Ok(a) => Ok(a),
            Err(_) => Err(err!(errmsg!(
                "Failed to convert Vec<char> to [char; {}].", A,
            ), Conversion)),
        }
    }

    /// A helper for preparing padding sets into the required array form.
    pub fn prepare_pad_set(s: &str) -> Outcome<[char; X]> {
        let chars: Vec<char> = s.chars().collect();
        if chars.len() != X {
            return Err(err!(errmsg!(
                "The number of characters in the padding set, {}, does not match \
                your generic parameter, {}.", chars.len(), X,
            ), Invalid, Size, Input, Mismatch));
        }
        match chars.try_into() {
            Ok(a) => Ok(a),
            Err(_) => Err(err!(errmsg!(
                "Failed to convert Vec<char> to [char; {}].", X,
            ), Conversion)),
        }
    }

    pub fn pad_sep(&self) -> Option<char> { self.padding.map(|(c, _)| c) }
    pub fn alphabet_size(&self) -> usize { A }
    pub fn token_size(&self) -> usize { X }

    pub fn fmt_pad_set(&self) -> String {
        let mut result = String::new();
        if let Some((_, pad_set)) = self.padding {
            for c in pad_set {
                result.push(c); 
            }
        }
        result
    }

    pub fn fmt_char_map(&self) -> Vec<String> {
        let mut result = Vec::new();
        for (i, c) in self.alphabet.iter().enumerate() {
            result.push(fmt!("'{}' -> {:0width$b}", c, i, width = X));
        }
        result
    }

    pub fn get_char(&self, token: u32) -> char {
        self.alphabet[token as usize]
    }

    pub fn get_pad_char(&self, padding: u8) -> Option<char> {
        self.padding.map(|(_, pad_set)| pad_set[padding as usize])
    }

    pub fn get_token(&self, c: char) -> Option<u32> {
        self.alphabet.iter().position(|&x| x == c)
            .and_then(|v| u32::try_from(v).ok())
    }

    pub fn get_padding_bits(&self, p: char) -> Option<u8> {
        match self.padding {
            Some((_, pad_set)) => pad_set.iter().position(|&c| c == p)
                .and_then(|v| u8::try_from(v).ok()),
            None => None,
        } 
    }

    /// Append padding characters to the given string. Padding value must be > 0.
    pub fn push_pad(&self, encoded: &mut String, padding: u8) {
        if let Some((pad_sep, _)) = self.padding {
            encoded.push(pad_sep);
        }
        if let Some(c) = self.get_pad_char(padding - 1) {
            encoded.push(c);
        }
    }

    pub fn to_string(&self, input: &[u8]) -> String {
        let mut encoded = String::new();
        let (tokens, padding) = self.tokenise(input);
        for token in tokens {
            encoded.push(self.get_char(token));
        }
        if padding > 0 {
            //trace!("padding = {}, '{}'", padding, self.fmt_pad_set());
            self.push_pad(&mut encoded, padding);
        }
        encoded
    }

    #[inline]
    const fn lower_bit_mask(z: usize) -> u64 {
        (1 << z) - 1
    }

    fn tokenise(&self, data: &[u8]) -> (Vec<u32>, u8) {
        //trace!("mask: {}",
        //    Stringer::new(fmt!("{:0width$b}", Self::TOKEN_MASK, width = 64)).insert_every("_", 8),
        //);
        let mut tokens = Vec::new();
        let mut buf: u64 = 0; // Handle overflow.
        let mut bits_in_buf = 0;
    
        for &byt in data {
            //buf |= (*byt as u64) << bits_in_buf;
            buf = (buf << 8) | byt as u64;
            bits_in_buf += 8;
            //trace!("bite: shift next byte in from left, buf {} {:08b} {}",
            //    Stringer::new(fmt!("{:0width$b}", buf, width = 64)).insert_every("_", 8),
            //    byt, bits_in_buf,
            //);
    
            // Extract as many tokens as possible.
            while bits_in_buf >= X {
                let token = (buf >> (bits_in_buf - X)) & Self::TOKEN_MASK;
                //trace!(" chew: buf right shifted by {} {}", bits_in_buf - X,
                //    Stringer::new(fmt!("{:0width$b}", buf >> (bits_in_buf - X), width = 64)).insert_every("_", 8),
                //);
                //trace!(" chew: token {}",
                //    Stringer::new(fmt!("{:0width$b}", token, width = X)).insert_every("_", 8),
                //);
                tokens.push(token as u32);
                bits_in_buf -= X;
                //trace!(" chew: buf {} token {:0width$b} {}",
                //    Stringer::new(fmt!("{:0width$b}", buf, width = 64)).insert_every("_", 8),
                //    token, bits_in_buf, width = X,
                //);
            }
        }
    
        let padding = if bits_in_buf > 0 {
            let padding = X - bits_in_buf;
            let token = buf << padding & Self::TOKEN_MASK;
            tokens.push(token as u32);
            //trace!(" final token {:0width$b} {} padding {}", token, bits_in_buf, padding, width = X);
            padding
        } else {
            0
        };
    
        (tokens, padding as u8)
    }

    pub fn string_to_tokens(&self, encoded: String) -> Outcome<Vec<u32>> {
        let mut tokens = Vec::new();
        for c in encoded.chars() {
            let index = self.get_token(c);
            match index {
                Some(i) => tokens.push(i),
                None => return Err(err!(errmsg!(
                    "Character '{}' not recognised by this Base2x alphabet.", c,
                ), Unknown, Invalid, Input, String, Decode)),
            }
        }
        Ok(tokens)
    }

    pub fn from_str(&self, s: &str) -> Outcome<Vec<u8>> {
        if s.len() == 0 {
            return Ok(Vec::new());
        }
        let (encoded, padding) = res!(self.parse_pad(s));
        if padding > 0 {
            res!(self.validate(&encoded, padding));
        }// else {
        //    padding = self.normalise(&mut encoded);
        //}
        //trace!("from_str: '{}', padding {}", encoded, padding);
        let tokens = res!(self.string_to_tokens(encoded));
        // We just needed to ensure any partial token is present at the end, and no longer need the
        // padding amount, since the padding bits are automatically filled with zeros.
        Ok(self.detokenise(&tokens))
    }

    fn validate(&self, encoded: &String, padding: u8) -> Outcome<()> {
        let bits = encoded.chars().count() * X - ( padding as usize );
        if padding > 0 {
            //trace!("validation: padding = {} bits = {}", padding, bits);
            if bits % 8 != 0 {
                return Err(err!(errmsg!(
                    "The padding of {} zero bits is not valid because string decoding \
                    will lead to a total bit length that is not divisible by 8.", padding,
                ), Invalid, Input, Mismatch, String, Decode));
            }
        }
        Ok(())
    }

    pub fn normalise(&self, encoded: &mut String) -> u8 {
        let bits = encoded.len() * X;
        let rem = (bits + 7) / 8 * 8 - bits;
        let mut padding = 0;
        if rem > 0 {
            padding = (X - rem) as u8;
            (*encoded).push(self.get_char(0));
            //trace!("normalisation: padding = {} new encoded = '{}'", padding, encoded);
        }
        padding
    }

    fn parse_pad(&self, encoded: &str) -> Outcome<(String, u8)> {
        let mut encoded = encoded.to_string();
        if let Some((pad_sep, _)) = self.padding {
            if encoded.ends_with(|c: char| c.is_digit(10)) &&
               encoded.chars().nth_back(1) == Some(pad_sep)
            {
                if let Some(padding_amount_char) = encoded.pop() {
                    if encoded.pop() == Some(pad_sep) {
                        match self.get_padding_bits(padding_amount_char) {
                            Some(padding) => return Ok((encoded, padding + 1)),
                            None => return Err(err!(errmsg!(
                                "Padding character '{}' is invalid, must be in the \
                                set {}.", padding_amount_char, self.fmt_pad_set(),
                            ), Invalid, Input, String, Decode)),
                        }
                    } else {
                        unreachable!()
                    }
                }
            }
        }
        Ok((encoded, 0))
    }

    fn detokenise(&self, tokens: &[u32]) -> Vec<u8> {
        let mut data = Vec::new();
        let mut buf: u64 = 0;
        let mut bits_in_buf = 0;

        for token in tokens {
            buf = (buf << X) | *token as u64;
            bits_in_buf += X;
            //trace!("bite: buf {} {} token {:0width$b}",
            //    Stringer::new(fmt!("{:0width$b}", buf, width = 64)).insert_every("_", 8),
            //    bits_in_buf, token, width = X,
            //);

            while bits_in_buf >= 8 {
                let byt = (buf >> (bits_in_buf - 8)) & 0xff; // Extract byte from the top.
                //trace!(" chew: buf {} byt {:08b}",
                //    Stringer::new(fmt!("{:0width$b}", buf, width = 64)).insert_every("_", 8),
                //    byt,
                //);
                data.push(byt as u8);
                bits_in_buf -= 8;
            }
        }

        data
    }
}
