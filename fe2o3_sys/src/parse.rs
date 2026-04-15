//! Shared helpers for reading and parsing `/proc` pseudo-files.

use oxedyne_fe2o3_core::prelude::*;

use std::{
    fs,
    path::Path,
    str::FromStr,
};

/// Read the entire contents of `path` into a `String`. Wraps the
/// standard-library read with the project error macros so callers
/// can `res!()` it.
pub fn read_to_string<P: AsRef<Path>>(path: P) -> Outcome<String> {
    let p = path.as_ref();
    match fs::read_to_string(p) {
        Ok(s) => Ok(s),
        Err(e) => Err(err!(
            "Failed to read {:?}: {}",
            p, e;
            IO, File, Read)),
    }
}

/// Parse a whitespace-separated numeric token into `T`. Returns
/// an error whose message names the token so broken `/proc`
/// output is visible in the log.
pub fn parse_num<T: FromStr>(token: &str, field: &str) -> Outcome<T> {
    match token.parse::<T>() {
        Ok(v) => Ok(v),
        Err(_) => Err(err!(
            "Cannot parse {} token {:?} as number.",
            field, token;
            Input, Invalid, Decode)),
    }
}

/// Split a line on whitespace and return a vector of non-empty
/// tokens. `/proc` files use varying numbers of spaces as
/// separators so splitwise iteration is simplest.
pub fn tokens(line: &str) -> Vec<&str> {
    line.split_whitespace().collect()
}
