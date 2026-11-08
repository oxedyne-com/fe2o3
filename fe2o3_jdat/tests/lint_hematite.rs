//! The Hematite gate, `fe2o3_text::lint::hematite`, run with the rule table at the fe2o3 root as
//! `fe2o3_jdat::lint::load_rules` reads it: a seeded breach of each rule is refused by name and
//! line, a clean change passes, a breach only in removed or unchanged lines passes, and a rule
//! switched off in the table passes.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    lint,
    prelude::*,
};
use oxedyne_fe2o3_text::lint::hematite::{
    self,
    Added,
    Report,
    Rules,
};

use std::{
    collections::BTreeSet,
    path::PathBuf,
};


// Mutant switches for the red script: HEMATITE_M<n> set switches rule G<n> off.
fn rules() -> Outcome<Rules> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("hematite_rules.jdat");
    let mut rules = res!(lint::load_rules(&path));
    for r in rules.rules.iter_mut() {
        if std::env::var(fmt!("HEMATITE_M{}", r.id.trim_start_matches('G'))).is_ok() {
            r.on = false;
        }
    }
    Ok(rules)
}

fn gate_lines(rules: &Rules, file: &str, src: &str, lines: &[usize]) -> Outcome<Report> {
    let a = Added { file: file.to_string(), src: src.to_string(), lines: lines.iter().copied().collect() };
    hematite::check(&[a], rules)
}

/// Every line of `src` added, as in a new file.
fn gate(rules: &Rules, file: &str, src: &str) -> Outcome<Report> {
    let n = src.split('\n').count();
    gate_lines(rules, file, src, &(1..=n).collect::<Vec<_>>())
}

fn hits(rep: &Report) -> Vec<(String, usize)> {
    rep.breaches.iter().map(|b| (b.rule.clone(), b.line)).collect()
}

fn seeds() -> Vec<(&'static str, &'static str, String, usize)> {
    // A credential shape split so that no scanner of this file finds one.
    let key = fmt!("sk_li{}", "ve_9fK2mQ7xR4tL8wZ3nB6vY1cH5");
    vec![
        ("G1",  "src/a.rs", fmt!("fn f(x: Option<u8>) -> u8 {{\n    x.unwrap()\n}}\n"), 2),
        ("G2",  "src/a.rs", fmt!("fn f(x: Option<u8>) -> u8 {{\n    x.expect(\"no\")\n}}\n"), 2),
        ("G3",  "src/a.rs", fmt!("fn f() {{\n    unsafe {{ g() }}\n}}\n"), 2),
        ("G4",  "src/a.rs", fmt!("fn f(x: &str) -> Outcome<u8> {{\n    let v = parse(x)?;\n    Ok(v)\n}}\n"), 2),
        ("G5",  "src/a.rs", fmt!("fn f() -> Outcome<()> {{\n    ok!(g(), \"context\");\n    Ok(())\n}}\n"), 2),
        ("G6",  "src/a.rs", fmt!("fn f(m: &Data) {{\n    let g = m.read();\n}}\n"), 2),
        ("G7",  "src/a.rs", fmt!("pub enum E {{\n    /// The first.\n    A,\n}}\n"), 2),
        ("G7",  "src/a.rs", fmt!("pub struct S {{\n    /// The size.\n    pub n: u8,\n}}\n"), 2),
        ("G7",  "src/a.rs", fmt!("/// The most.\npub const MOST: u8 = 3;\n"), 1),
        ("G7",  "src/a.rs", fmt!("pub struct S {{\n    /// The size.\n    #[optional]\n    pub n: u8,\n}}\n"), 2),
        ("G7",  "src/a.rs", fmt!("pub enum E {{\n    /// The first.\n    #[default]\n    A,\n}}\n"), 2),
        ("G7",  "src/a.rs", fmt!("/// The most.\n#[allow(dead_code)]\npub const MOST: u8 = 3;\n"), 1),
        ("G7",  "src/a.rs", fmt!("#[derive(Debug)]\npub struct S {{\n    /// The size.\n    #[optional]\n    #[serde(default)]\n    pub n: u8,\n}}\n"), 3),
        ("G7",  "src/a.rs", fmt!("pub enum E {{\n    A {{\n        /// The size.\n        #[optional]\n        n: u8,\n    }},\n}}\n"), 3),
        ("G7",  "src/a.rs", fmt!("impl S {{\n    /// The most.\n    #[optional]\n    pub(crate) const MOST: u8 = 3;\n}}\n"), 2),
        ("G8",  "src/a.rs", fmt!("/// Parses.\n///\n/// # Errors\nfn f() {{}}\n"), 3),
        ("G9",  "src/a.rs", fmt!("use std::fmt;\nuse crate::x;\n"), 2),
        ("G9",  "src/a.rs", fmt!("use crate::a; use crate::b;\n"), 1),
        ("G10", "notes.txt", fmt!("one\nkey = {}\n", key), 2),
        ("G11", "src/a.rs", fmt!("fn f() {{\n    let p = \"/var/lib/app/state\";\n}}\n"), 2),
        ("G12", "src/a.rs", fmt!("fn f(x: Box<dyn Fn()>) {{}}\n"), 1),
        ("G13", "src/a.rs", fmt!("// The behavior of it.\nfn f() {{}}\n"), 1),
    ]
}

#[test]
fn test_hematite_each_rule_refused_by_name_and_line() -> Outcome<()> {
    let rules = res!(rules());
    for (id, file, src, line) in seeds() {
        let rep = res!(gate(&rules, file, &src));
        assert_eq!(hits(&rep), vec![(id.to_string(), line)], "seed for {}:\n{}", id, src);
        let msg = &rules.rules.iter().filter(|r| r.id == id).map(|r| r.msg.clone()).collect::<Vec<_>>()[0];
        assert_eq!(fmt!("{}", rep.breaches[0]), fmt!("{}:{}: {}", file, line, msg));
    }
    Ok(())
}

#[test]
fn test_hematite_clean_change_passes() -> Outcome<()> {
    let rules = res!(rules());
    let src = "\
use crate::x;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::RwLock;

/// Reads the value; `color` in backticks is a name, not a spelling.
pub fn f(m: &RwLock<u8>, s: &str) -> Outcome<u8> {
    let g = res!(lock_read!(m));
    let v = res!(s.parse::<u8>().map_err(|e| err!(e, \"Bad {}: {:?} x.unwrap()?\", s, 1; Invalid)));
    let line = std::io::stdin().lock();
    Ok(*g + v)
}

pub struct S {
    pub a: u8, // a field
}

pub const fn g<T: ?Sized>(_t: &T) -> u8 { 0 }

macro_rules! m { ($($x:expr),*$(,)?) => { $x.read() } }

fn h(x: Box<dyn Fn()>) {} // fixed: a closure chosen at run time, the caller's
";
    let rep = res!(gate(&rules, "src/a.rs", src));
    assert_eq!(hits(&rep), Vec::<(String, usize)>::new());
    Ok(())
}

#[test]
fn test_hematite_removed_and_unchanged_lines_pass() -> Outcome<()> {
    let rules = res!(rules());
    // The unwrap sits on an unchanged line 2, and the removed line held a `?`.
    let diff = "\
diff --git a/src/a.rs b/src/a.rs
--- a/src/a.rs
+++ b/src/a.rs
@@ -3 +3 @@
-    let v = parse(x)?;
+    let v = res!(parse(x));
";
    let added = hematite::added_lines(diff);
    let want: BTreeSet<usize> = [3].into_iter().collect();
    assert_eq!(added.get("src/a.rs"), Some(&want));
    let src = "fn f(x: &str, y: Option<u8>) -> Outcome<u8> {\n    let w = y.unwrap();\n    let v = res!(parse(x));\n    Ok(v + w)\n}\n";
    let rep = res!(gate_lines(&rules, "src/a.rs", src, &[3]));
    assert!(rep.is_clean(), "{:?}", rep);
    // The same file with every line counted as added is refused, so the pass above is the diff's.
    let rep = res!(gate(&rules, "src/a.rs", src));
    assert_eq!(hits(&rep), vec![(fmt!("G1"), 2)]);
    Ok(())
}

#[test]
fn test_hematite_added_lines_reads_hunks() -> Outcome<()> {
    // An added line that itself starts "++" is content, not a header; a deleted file adds nothing.
    let diff = "\
--- a/x.rs
+++ b/x.rs
@@ -1,2 +1,3 @@
-a
-b
+++c
+d
+e
@@ -10,0 +12,2 @@
+f
+g
--- a/gone.rs
+++ /dev/null
@@ -1 +0,0 @@
-h
";
    let added = hematite::added_lines(diff);
    let want: BTreeSet<usize> = [1, 2, 3, 12, 13].into_iter().collect();
    assert_eq!(added.get("x.rs"), Some(&want));
    assert_eq!(added.len(), 1);
    Ok(())
}

#[test]
fn test_hematite_rule_off_in_table_passes() -> Outcome<()> {
    let mut rules = res!(rules());
    let src = "fn f(x: &str) -> Outcome<u8> {\n    let v = parse(x)?;\n    Ok(v)\n}\n";
    assert!(!res!(gate(&rules, "src/a.rs", src)).is_clean());
    for r in rules.rules.iter_mut().filter(|r| r.id == "G4") {
        r.on = false;
    }
    assert!(res!(gate(&rules, "src/a.rs", src)).is_clean());
    Ok(())
}

#[test]
fn test_hematite_allowlist_passes_and_is_reported() -> Outcome<()> {
    let rules = res!(rules());
    let src = "fn f(x: &str) -> Outcome<u8> {\n    let v = parse(x)?; // allowlist hematite: a seeded case\n    Ok(v)\n}\n";
    let rep = res!(gate(&rules, "src/a.rs", src));
    assert!(rep.is_clean(), "{:?}", rep);
    assert_eq!(rep.allows.iter().map(|a| (a.line, a.rules.clone())).collect::<Vec<_>>(),
        vec![(2, vec![fmt!("G4")])]);
    Ok(())
}

#[test]
fn test_hematite_test_code_exempt_only_where_the_table_says() -> Outcome<()> {
    let rules = res!(rules());
    // G9 and G13 are exempt in tests, G1 is not.
    let src = "#[cfg(test)]\nmod tests {\n    use std::fmt;\n    use crate::x;\n    // The behavior.\n    fn t() { g().unwrap(); }\n}\n";
    let rep = res!(gate(&rules, "src/a.rs", src));
    assert_eq!(hits(&rep), vec![(fmt!("G1"), 6)]);
    let rep = res!(gate(&rules, "tests/a.rs", "// The color.\nuse std::fmt;\nuse crate::x;\n"));
    assert!(rep.is_clean(), "{:?}", rep);
    Ok(())
}

#[test]
fn test_hematite_unknown_check_is_refused_and_seq_rule_needs_no_code() -> Outcome<()> {
    let bad = res!(Dat::decode_string(fmt!("{{\"local_crates\": [], \"rules\": [{{\"id\": \"X\", \
        \"check\": \"nosuch\", \"on\": true, \"in_tests\": true, \"fixed_ok\": false, \"msg\": \"m\"}}]}}")));
    assert!(lint::rules_from_dat(&bad).is_err());
    let dat = res!(Dat::decode_string(fmt!("{{\"local_crates\": [], \"rules\": [{{\"id\": \"Q1\", \
        \"check\": \"seq\", \"seq\": [\"panic\", \"!\"], \"on\": true, \"in_tests\": true, \
        \"fixed_ok\": false, \"msg\": \"panic! -- return an err!\"}}]}}")));
    let rules = res!(lint::rules_from_dat(&dat));
    let rep = res!(gate(&rules, "src/a.rs", "fn f() {\n    panic!(\"no\");\n}\n"));
    assert_eq!(hits(&rep), vec![(fmt!("Q1"), 2)]);
    Ok(())
}

#[test]
fn test_hematite_built_in_table_is_the_file() -> Outcome<()> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("hematite_rules.jdat");
    let text = match std::fs::read_to_string(&path) {
        Ok(s)   => s,
        Err(e)  => return Err(err!(e, "While reading {}.", path.display(); IO, File, Read)),
    };
    assert_eq!(lint::HEMATITE_RULES, text);
    let rules = res!(lint::parse_rules(lint::HEMATITE_RULES));
    assert_eq!(rules.rules.len(), res!(lint::load_rules(&path)).rules.len());
    Ok(())
}
