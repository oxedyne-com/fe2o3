//! The event envelope of Daimond's feed, `{v, d, n, b, t}` and then the payload.
//!
//! `v` is the schema version, `d` a short id for the device or source, `n` a persisted
//! per-source sequence number that makes redelivery idempotent for the reader, `b` the build tag
//! and `t` the wall clock in milliseconds. In `www/js/debugshare.js` an event goes to the wire as
//! a row `{ts, tag: "ev <kind>", data: <that JSON>}`, and `Row::entry` makes the same row.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    string::enc::escape_json_string,
};


// The envelope's own names. A payload key of the same name is dropped, so a caller cannot
// overwrite `n` or `t` by accident.
const ENVELOPE: [&str; 5] = ["v", "d", "n", "b", "t"];

/// One event: the envelope and the payload fields in the order they were added.
#[derive(Clone, Debug)]
pub struct Row {
    pub v:      u8,                     // schema version
    pub d:      String,                 // short device or source id
    pub n:      u64,                    // per-source sequence number
    pub b:      String,                 // build tag
    pub t:      u64,                    // wall clock, milliseconds since the epoch
    pub kind:   String,                 // the `<kind>` of the tag `ev <kind>`
    pub body:   Vec<(String, Dat)>,     // payload fields, never named v, d, n, b or t
}

/// A row as it goes to the wire and to the file: Daimond's `{ts, tag, data}`.
#[derive(Clone, Debug)]
pub struct Entry {
    pub ts:     i64,        // the client's own clock, kept beside the receive time to show skew
    pub tag:    String,
    pub data:   String,
}

impl Row {

    pub const VERSION: u8 = 1;

    pub fn new(d: &str, n: u64, b: &str, t: u64, kind: &str) -> Self {
        Self {
            v:      Self::VERSION,
            d:      d.to_string(),
            n,
            b:      b.to_string(),
            t,
            kind:   kind.to_string(),
            body:   Vec::new(),
        }
    }

    /// Adds a payload field, or replaces one of the same name where it stands. A name in the
    /// envelope is dropped.
    pub fn with(mut self, name: &str, val: Dat) -> Self {
        if ENVELOPE.contains(&name) {
            return self;
        }
        self.set(name, val);
        self
    }

    fn set(&mut self, name: &str, val: Dat) {
        for (k, v) in self.body.iter_mut() {
            if k == name {
                *v = val;
                return;
            }
        }
        self.body.push((name.to_string(), val));
    }

    /// The tag of the wire row, `ev <kind>`.
    pub fn tag(&self) -> String {
        fmt!("ev {}", self.kind)
    }

    /// The event as one JSON object, the envelope first.
    pub fn json(&self) -> Outcome<String> {
        let mut s = fmt!(
            "{{\"v\":{},\"d\":\"{}\",\"n\":{},\"b\":\"{}\",\"t\":{}",
            self.v, escape_json_string(&self.d), self.n, escape_json_string(&self.b), self.t);
        for (k, v) in &self.body {
            s.push_str(&fmt!(",\"{}\":{}", escape_json_string(k), res!(compact(v))));
        }
        s.push('}');
        Ok(s)
    }

    /// The event's JSON cut to `max` bytes, as `fit` does in `debugshare.js`.
    ///
    /// The largest string field goes first and only far enough to fit, so a short tool name
    /// survives a long error message being trimmed, and `tr:1` marks that something was cut. The
    /// envelope is never sacrificed: an event with no payload left still says which source,
    /// which build and where in the sequence it sits.
    pub fn fit(&self, max: usize) -> Outcome<String> {
        let s = res!(self.json());
        if s.len() <= max {
            return Ok(s);
        }
        let mut o = self.clone();
        o.set("tr", Dat::U8(1));
        for _ in 0..32 {
            let s = res!(o.json());
            if s.len() <= max {
                return Ok(s);
            }
            let over = s.len() - max;
            // The first of equally large strings, and never the marker.
            let mut big: Option<(usize, usize)> = None;
            for (i, (k, v)) in o.body.iter().enumerate() {
                if k == "tr" {
                    continue;
                }
                if let Dat::Str(x) = v {
                    if x.len() > big.map_or(0, |(_, n)| n) {
                        big = Some((i, x.len()));
                    }
                }
            }
            let i = match big {
                Some((i, _)) => i,
                None => break,
            };
            // Characters, not bytes: a multi-byte string loses at least `over` bytes this way,
            // never fewer, so the loop always converges.
            let chars = match &o.body[i].1 {
                Dat::Str(x) => x.chars().count(),
                _ => 0,
            };
            if chars > over + 1 {
                let keep = chars - over - 1;
                if let Dat::Str(x) = &mut o.body[i].1 {
                    *x = x.chars().take(keep).collect();
                }
            } else {
                o.body.remove(i);
            }
        }
        let s = res!(o.json());
        if s.len() <= max {
            return Ok(s);
        }
        // Nothing left but the envelope, and it is still worth sending.
        let mut bare = self.clone();
        bare.body.clear();
        bare.set("tr", Dat::U8(1));
        bare.json()
    }

    /// The wire row for this event, its JSON fitted to `max` bytes.
    pub fn entry(&self, max: usize) -> Outcome<Entry> {
        Ok(Entry {
            ts:     self.t as i64,
            tag:    self.tag(),
            data:   res!(self.fit(max)),
        })
    }
}

/// A `Dat` as compact JSON text, the way `JSON.stringify` writes it, with nothing between tokens.
///
/// A map's keys are taken as their text. A kind JSON has no word for, such as a float or a byte
/// string, is left to `Dat::json`.
pub fn compact(d: &Dat) -> Outcome<String> {
    let mut out = String::new();
    res!(compact_into(d, &mut out));
    Ok(out)
}

fn compact_into(d: &Dat, out: &mut String) -> Outcome<()> {
    match d {
        Dat::Bool(b)    => out.push_str(if *b { "true" } else { "false" }),
        Dat::U8(n)      => out.push_str(&fmt!("{}", n)),
        Dat::U16(n)     => out.push_str(&fmt!("{}", n)),
        Dat::U32(n)     => out.push_str(&fmt!("{}", n)),
        Dat::U64(n)     => out.push_str(&fmt!("{}", n)),
        Dat::U128(n)    => out.push_str(&fmt!("{}", n)),
        Dat::I8(n)      => out.push_str(&fmt!("{}", n)),
        Dat::I16(n)     => out.push_str(&fmt!("{}", n)),
        Dat::I32(n)     => out.push_str(&fmt!("{}", n)),
        Dat::I64(n)     => out.push_str(&fmt!("{}", n)),
        Dat::I128(n)    => out.push_str(&fmt!("{}", n)),
        Dat::Str(s)     => {
            out.push('"');
            out.push_str(&escape_json_string(s));
            out.push('"');
        },
        Dat::Empty      => out.push_str("null"),
        Dat::Opt(o)     => match &**o {
            Some(x) => res!(compact_into(x, out)),
            None    => out.push_str("null"),
        },
        Dat::Box(b)     => res!(compact_into(b, out)),
        Dat::List(l)    => {
            out.push('[');
            for (i, x) in l.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                res!(compact_into(x, out));
            }
            out.push(']');
        },
        Dat::Map(m)     => {
            let pairs: Vec<(&Dat, &Dat)> = m.iter().collect();
            res!(compact_members(&pairs, out));
        },
        Dat::OrdMap(m)  => {
            // Insertion order, which the map keeps in its keys' ordinals.
            let mut pairs: Vec<(u64, &Dat, &Dat)> = m.iter().map(|(k, v)| (k.ord(), k.dat(), v)).collect();
            pairs.sort_by_key(|p| p.0);
            let pairs: Vec<(&Dat, &Dat)> = pairs.into_iter().map(|(_, k, v)| (k, v)).collect();
            res!(compact_members(&pairs, out));
        },
        other           => out.push_str(&res!(other.json())),
    }
    Ok(())
}

fn compact_members(pairs: &[(&Dat, &Dat)], out: &mut String) -> Outcome<()> {
    out.push('{');
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let name = match k {
            Dat::Str(s) => s.clone(),
            other       => fmt!("{}", other),
        };
        out.push('"');
        out.push_str(&escape_json_string(&name));
        out.push_str("\":");
        res!(compact_into(v, out));
    }
    out.push('}');
    Ok(())
}

impl Entry {

    /// One NDJSON line, newline included: the receive time, the source's label, then the row.
    ///
    /// This is what Daimond's text block carried (`===== <recv ms> ... device=<d> =====` and a
    /// row under it), one object to a line. The data stays a string, as it is on the wire, so a
    /// clipped event is still a valid line.
    pub fn line(&self, device: &str, recv_ms: u64) -> String {
        fmt!(
            "{{\"bt\":{},\"device\":\"{}\",\"ts\":{},\"tag\":\"{}\",\"data\":\"{}\"}}\n",
            recv_ms,
            escape_json_string(device),
            self.ts,
            escape_json_string(&self.tag),
            escape_json_string(&self.data))
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_writes_what_json_stringify_writes() {
        let d = create_dat_ordmap(vec![
            (Dat::Str("b".to_string()), Dat::List(vec![Dat::U8(1), Dat::I64(-2), Dat::Bool(true), Dat::Empty])),
            (Dat::Str("a".to_string()), Dat::Str("q\"\n\\".to_string())),
            (Dat::Str("c".to_string()), Dat::Opt(Box::new(None))),
            (Dat::Str("d".to_string()), create_dat_ordmap(Vec::new())),
        ]);
        let got = compact(&d).unwrap_or_default();
        // The members stand in the order they were added, which a plain map would not keep.
        assert_eq!(got, "{\"b\":[1,-2,true,null],\"a\":\"q\\\"\\n\\\\\",\"c\":null,\"d\":{}}");
    }
}
