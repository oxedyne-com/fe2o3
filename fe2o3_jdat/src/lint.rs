//! The `.jdat` files of the source gates in `fe2o3_text::lint`: the mutable-literal gate's ratchet
//! baseline, and the Hematite gate's rule table. The gates live in fe2o3_text, which this crate
//! depends on, so the files are read and written here.
//!
//! ```text
//! {
//!     "entries": [ { "file": "fe2o3_net/src/llm.rs", "class": "host", "text": "…", "count": 1 } ],
//!     "allow": [ { "file": "…", "class": "named", "text": "…", "why": "Generated tables." } ],
//! }
//! ```

use crate::prelude::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::{
    lint::{
        hematite::{
            Check,
            Rule,
            Rules,
        },
        mutable::{
            self,
            Allow,
            Baseline,
            Class,
            Key,
            Report,
        },
    },
};

use std::{
    fs,
    path::Path,
};


/// Reads a baseline file. An absent file is an empty baseline, so a new tree starts at zero debt.
pub fn load_baseline(path: &Path) -> Outcome<Baseline> {
    if !path.exists() {
        return Ok(Baseline::default());
    }
    let s = match fs::read_to_string(path) {
        Ok(s)   => s,
        Err(e)  => return Err(err!(e, "While reading baseline {}.", path.display(); IO, File, Read)),
    };
    let dat = res!(Dat::decode_string(s));
    baseline_from_dat(&dat)
}

/// Writes a baseline file, sorted by key so a migration's diff is only its own lines.
pub fn save_baseline(base: &Baseline, path: &Path) -> Outcome<()> {
    let mut text = res!(baseline_to_dat(base).jdat_to_lines("\t"));
    text.push('\n');
    match fs::write(path, text) {
        Ok(())  => Ok(()),
        Err(e)  => Err(err!(e, "While writing baseline {}.", path.display(); IO, File, Write)),
    }
}

/// Scans the tree under `root` and checks it against the baseline file at `baseline_path`.
pub fn check_tree(root: &Path, baseline_path: &Path) -> Outcome<Report> {
    let base = res!(load_baseline(baseline_path));
    mutable::check_tree(root, &base)
}

pub fn baseline_to_dat(base: &Baseline) -> Dat {
    let entries: Vec<Dat> = base.entries.iter().map(|(k, n)| mapdat!{
        "file"  => k.file.clone(),
        "class" => k.class.name(),
        "text"  => k.text.clone(),
        "count" => *n as u64,
    }).collect();
    let allow: Vec<Dat> = base.allow.iter().map(|a| mapdat!{
        "file"  => a.file.clone(),
        "class" => a.class.name(),
        "text"  => a.text.clone(),
        "why"   => a.why.clone(),
    }).collect();
    mapdat!{
        "entries"   => Dat::List(entries),
        "allow"     => Dat::List(allow),
    }
}

pub fn baseline_from_dat(dat: &Dat) -> Outcome<Baseline> {
    let mut base = Baseline::default();
    for (i, e) in res!(dat.map_get_list(&dat!("entries"))).iter().enumerate() {
        let (key, n) = match key_from_dat(e).and_then(|k| e.map_get_u64(&dat!("count")).map(|n| (k, n))) {
            Ok(v)   => v,
            Err(e)  => return Err(err!(e, "In baseline entry {}.", i; Invalid, Input)),
        };
        *base.entries.entry(key).or_insert(0) += n as usize;
    }
    if let Some(list) = res!(dat.map_get(&dat!("allow"))) {
        let list = match list {
            Dat::List(v)    => v,
            _               => return Err(err!(
                "Baseline 'allow' is a {:?}, expected a list.", list.kind(); Invalid, Input)),
        };
        for (i, a) in list.iter().enumerate() {
            let key = match key_from_dat(a) {
                Ok(k)   => k,
                Err(e)  => return Err(err!(e, "In baseline allow entry {}.", i; Invalid, Input)),
            };
            // A missing why loads as empty, so the gate reports it rather than the load failing.
            let why = match res!(a.map_get(&dat!("why"))) {
                Some(Dat::Str(s))   => s.clone(),
                _                   => String::new(),
            };
            base.allow.push(Allow { file: key.file, class: key.class, text: key.text, why });
        }
    }
    Ok(base)
}

fn key_from_dat(dat: &Dat) -> Outcome<Key> {
    Ok(Key {
        file:   res!(dat.map_get_string(&dat!("file"))),
        class:  res!(Class::from_name(&res!(dat.map_get_string(&dat!("class"))))),
        text:   res!(dat.map_get_string(&dat!("text"))),
    })
}

/// The release's rule table, so an installer can write it where a daimon cannot reach.
pub const HEMATITE_RULES: &str = include_str!("../../hematite_rules.jdat");

/// Reads a Hematite rule table. Unlike a baseline, an absent table is an error: a gate with no
/// rules would pass everything.
///
/// ```text
/// {
///     "local_crates": ["oxedyne_"],
///     "rules": [ { "id": "G1", "check": "seq", "seq": [".", "unwrap", "(", ")"], "on": true,
///                  "in_tests": true, "fixed_ok": false, "msg": "unwrap() -- use res!(x)" } ],
/// }
/// ```
pub fn load_rules(path: &Path) -> Outcome<Rules> {
    let s = match fs::read_to_string(path) {
        Ok(s)   => s,
        Err(e)  => return Err(err!(e, "While reading the Hematite rule table {}.", path.display();
            IO, File, Read)),
    };
    match parse_rules(&s) {
        Ok(r)   => Ok(r),
        Err(e)  => Err(err!(e, "In the Hematite rule table {}.", path.display(); Invalid, Input)),
    }
}

pub fn parse_rules(text: &str) -> Outcome<Rules> {
    let dat = res!(Dat::decode_string(text));
    rules_from_dat(&dat)
}

pub fn rules_from_dat(dat: &Dat) -> Outcome<Rules> {
    let mut rules = Rules::default();
    for c in res!(dat.map_get_list(&dat!("local_crates"))) {
        match c {
            Dat::Str(s) => rules.local_crates.push(s.clone()),
            _           => return Err(err!("A 'local_crates' entry is a {:?}, expected a string.",
                c.kind(); Invalid, Input)),
        }
    }
    for (i, r) in res!(dat.map_get_list(&dat!("rules"))).iter().enumerate() {
        match rule_from_dat(r) {
            Ok(rule)    => rules.rules.push(rule),
            Err(e)      => return Err(err!(e, "In rule {}.", i; Invalid, Input)),
        }
    }
    Ok(rules)
}

fn rule_from_dat(dat: &Dat) -> Outcome<Rule> {
    let id = res!(dat.map_get_string(&dat!("id")));
    let name = res!(dat.map_get_string(&dat!("check")));
    let mut list = Vec::new();
    for key in ["seq", "words"] {
        if let Some(v) = res!(dat.map_get(&dat!(key))) {
            let v = match v {
                Dat::List(v)    => v,
                _               => return Err(err!("Rule '{}': '{}' is a {:?}, expected a list.",
                    id, key, v.kind(); Invalid, Input)),
            };
            for w in v {
                match w {
                    Dat::Str(s) => list.push(s.clone()),
                    _           => return Err(err!("Rule '{}': a '{}' entry is a {:?}, expected \
                        a string.", id, key, w.kind(); Invalid, Input)),
                }
            }
        }
    }
    let msg = res!(dat.map_get_string(&dat!("msg")));
    if msg.trim().is_empty() {
        return Err(err!("Rule '{}' has an empty 'msg'.", id; Invalid, Input, Missing));
    }
    Ok(Rule {
        on:         res!(dat.map_get_bool(&dat!("on"))),
        in_tests:   res!(dat.map_get_bool(&dat!("in_tests"))),
        fixed_ok:   res!(dat.map_get_bool(&dat!("fixed_ok"))),
        check:      res!(Check::of(&name, list)),
        msg,
        id,
    })
}
