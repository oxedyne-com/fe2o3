//! A jdat file read for a browser (U0, D-20261005-12). The settings file of an app is jdat, with
//! its comments, and a browser has `JSON.parse` and no jdat reader, so the Rust decoder, the one
//! parser, is compiled to wasm and exports `jdat_to_json`.

use crate::{
    prelude::*,
    bdat::limits::DecodeLimits,
    string::{
        dec::DecoderConfig,
        enc::escape_json_string,
    },
    usr::{
        UsrKind,
        UsrKindCode,
        UsrKindId,
    },
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_num::float::Float64;

use std::collections::BTreeMap;


// The largest whole number a browser's `Number` holds exactly, 2^53 - 1.
const JS_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

impl Dat {

    /// Reads jdat text with `decode_string`'s rules and writes it as plain JSON, members in the
    /// order written and every comment dropped, for a browser's `JSON.parse`.
    ///
    /// A comment is a note on a value to the decoder (a `Dat::ABox`), and a note standing alone is
    /// an entry with an empty key and value; both go. A value missing where one is needed, as in
    /// `[1,,2]`, is refused naming its key. With `js_safe`, a whole number past 2^53 - 1 in size
    /// and a float that is not finite are refused too, since `JSON.parse` would round them
    /// silently.
    pub fn jdat_to_json(
        s:          &str,
        limits:     &DecodeLimits,
        js_safe:    bool,
    )
        -> Outcome<String>
    {
        let mut cfg = DecoderConfig::<
            BTreeMap<UsrKindCode, UsrKind>,
            BTreeMap<String, UsrKindId>,
        >::default().with_limits(*limits);
        cfg.use_ordmaps = true;
        let dat = res!(Self::decode_string_with_config(s, &cfg));
        let mut out = String::with_capacity(s.len());
        let mut path = Vec::new();
        res!(write_json(&dat, js_safe, &mut path, &mut out));
        Ok(out)
    }
}

// The daticle under any notes.
fn bare(dat: &Dat) -> &Dat {
    match dat {
        Dat::ABox(_, inner, _)  => bare(inner),
        _                       => dat,
    }
}

// Does this entry hold nothing but a comment? A note on nothing is an `ABox` round an empty
// daticle, as key or as value, and a bare empty key and value is a stray comma instead.
fn is_note(key: &Dat, val: &Dat) -> bool {
    (matches!(key, Dat::ABox(..)) || matches!(val, Dat::ABox(..)))
        && matches!(bare(key), Dat::Empty)
        && matches!(bare(val), Dat::Empty)
}

fn at(path: &[String]) -> String {
    let mut s = String::new();
    for seg in path {
        if !s.is_empty() && !seg.starts_with('[') {
            s.push('.');
        }
        s.push_str(seg);
    }
    s
}

fn key_text(key: &Dat, path: &[String]) -> Outcome<String> {
    match bare(key) {
        Dat::Str(s) => Ok(s.clone()),
        Dat::Empty  => Err(err!(
            "A member has no name, in the entry after key '{}'.", at(path);
            Invalid, Input, Decode, Missing)),
        other       => Err(err!(
            "A name must be a string in JSON, found a {:?} after key '{}'.", other.kind(), at(path);
            Invalid, Input, Decode)),
    }
}

fn write_number(dat: &Dat, js_safe: bool, path: &[String], out: &mut String) -> Outcome<()> {
    let integer = matches!(dat,
        Dat::U8(_) | Dat::U16(_) | Dat::U32(_) | Dat::U64(_) | Dat::U128(_) |
        Dat::I8(_) | Dat::I16(_) | Dat::I32(_) | Dat::I64(_) | Dat::I128(_) | Dat::Aint(_));
    if integer {
        match (dat.get_i64(), dat.get_u64()) {
            (Some(n), _) => {
                if js_safe && n.unsigned_abs() > JS_SAFE_INTEGER as u64 {
                    return Err(err!(
                        "The whole number at key '{}' is past 2^53 - 1 in size, which a browser \
                        cannot hold exactly.", at(path); Invalid, Input, Decode, Excessive));
                }
                out.push_str(&fmt!("{}", n));
            },
            (None, Some(n)) if !js_safe => out.push_str(&fmt!("{}", n)),
            _ => return Err(err!(
                "The whole number at key '{}' is past 2^53 - 1 in size, which a browser cannot \
                hold exactly.", at(path); Invalid, Input, Decode, Excessive)),
        }
        return Ok(());
    }
    match dat.get_float64() {
        Some(Float64(f)) if f.is_finite() => out.push_str(&fmt!("{}", f)),
        _ => return Err(err!(
            "The number at key '{}' is not a finite float64, which JSON cannot hold.", at(path);
            Invalid, Input, Decode, Excessive)),
    }
    Ok(())
}

fn write_members<'a, I: Iterator<Item = (&'a Dat, &'a Dat)>>(
    members:    I,
    js_safe:    bool,
    path:       &mut Vec<String>,
    out:        &mut String,
)
    -> Outcome<()>
{
    out.push('{');
    let mut n = 0;
    for (k, v) in members {
        if is_note(k, v) {
            continue;
        }
        let key = res!(key_text(k, path));
        if n > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(&escape_json_string(&key));
        out.push_str("\":");
        path.push(key);
        if matches!(bare(v), Dat::Empty) {
            return Err(err!(
                "The member '{}' has no value.", at(path); Invalid, Input, Decode, Missing));
        }
        res!(write_json(v, js_safe, path, out));
        path.pop();
        n += 1;
    }
    out.push('}');
    Ok(())
}

// A daticle as plain JSON, members in the order held.
fn write_json(dat: &Dat, js_safe: bool, path: &mut Vec<String>, out: &mut String) -> Outcome<()> {
    match dat {
        Dat::ABox(_, inner, _)  => return write_json(inner, js_safe, path, out),
        Dat::Bool(b)            => out.push_str(if *b { "true" } else { "false" }),
        Dat::Opt(o) if o.is_none() => out.push_str("null"),
        Dat::Str(s)             => {
            out.push('"');
            out.push_str(&escape_json_string(s));
            out.push('"');
        },
        Dat::U8(_)  | Dat::U16(_) | Dat::U32(_) | Dat::U64(_) | Dat::U128(_) |
        Dat::I8(_)  | Dat::I16(_) | Dat::I32(_) | Dat::I64(_) | Dat::I128(_) |
        Dat::F32(_) | Dat::F64(_) | Dat::Aint(_) | Dat::Adec(_) =>
            res!(write_number(dat, js_safe, path, out)),
        Dat::List(items)        => {
            out.push('[');
            let mut n = 0;
            for item in items {
                if matches!(item, Dat::ABox(..)) && matches!(bare(item), Dat::Empty) {
                    continue; // A comment standing alone.
                }
                if matches!(bare(item), Dat::Empty) {
                    return Err(err!(
                        "A list item is missing after item {} at key '{}'.", n, at(path);
                        Invalid, Input, Decode, Missing));
                }
                if n > 0 {
                    out.push(',');
                }
                path.push(fmt!("[{}]", n));
                res!(write_json(item, js_safe, path, out));
                path.pop();
                n += 1;
            }
            out.push(']');
        },
        Dat::OrdMap(m)          => res!(write_members(m.iter().map(|(k, v)| (k.dat(), v)), js_safe, path, out)),
        Dat::Map(m)             => res!(write_members(m.iter(), js_safe, path, out)),
        other => return Err(err!(
            "A daticle of kind {:?} at key '{}' has no plain JSON form.", other.kind(), at(path);
            Invalid, Input, Unimplemented)),
    }
    Ok(())
}
