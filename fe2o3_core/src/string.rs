use crate::{
    prelude::*,
    byte::B32,
};

use std::fmt;

/// Tests whether a static string is contained in the given slice.
pub fn contains_str(target: &'static str, list: &[&'static str]) -> bool {
    for &s in list {
        if s == target {
            return true;
        }
    }
    false
}

/// Inspect the Unicode value of the first characters of the given string.
pub fn inspect(s: &str, n: usize) {
    for (i, c) in s.chars().enumerate() {
        println!("Character {}: '{}' (U+{:04X})", i, c, c as u32);
        if i >= n {
            break; 
        }
    }
}

pub trait ToHexString: std::fmt::LowerHex + Copy + Sized {
    fn to_hex_string(&self) -> String {
        to_hex_string(*self)
    }
}

fn to_hex_string<T: std::fmt::LowerHex>(v: T) -> String {
    let max_digits = std::mem::size_of::<T>() * 2;
    format!("{:0width$x}", v, width = max_digits)
}

impl ToHexString for B32 {
    fn to_hex_string(&self) -> String {
        format!("{:x}", self)
    }
}

impl fmt::LowerHex for B32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byt in &self.0 {
            write!(f, "{:02x}", byt)?;
        }
        Ok(())
    }
}

impl ToHexString for usize {
    fn to_hex_string(&self) -> String {
        format!("{:x}", self)
    }
}

pub fn parse_hex_char(c: char) -> Outcome<u8> {
    match c {
        '0'..='9' => Ok(c as u8 - b'0'),
        'a'..='f' => Ok(c as u8 - b'a' + 10),
        'A'..='F' => Ok(c as u8 - b'A' + 10),
        _ => Err(err!(errmsg!(
            "'{}' is not a valid hexadecimal digit.", c,
        ), Invalid, Input, String, Decode)),
    }
}
