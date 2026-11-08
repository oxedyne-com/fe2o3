//! The Hematite gate, `fe2o3_text::lint::hematite`, run with the rule table in fe2o3_jdat as
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
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("hematite_rules.jdat");
    let mut rules = res!(lint::load_rules(&path));
    for r in rules.rules.iter_mut() {
        if std::env::var(fmt!("HEMATITE_M{}", r.id.trim_start_matches('G'))).is_ok() {
            r.on = false;
        }
    }
    Ok(rules)
}

fn gate_lines(rules: &Rules, file: &str, src: &str, lines: &[usize]) -> Outcome<Report> {
    let a = Added { file: file.to_string(), src: src.to_string(), lines: lines.iter().copied().collect(),
        link: false };
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
        ("G1",  "src/a.rs", fmt!("fn f(x: Option<u8>) -> u8 {{\n    Option::unwrap(x)\n}}\n"), 2),
        ("G1",  "src/a.rs", fmt!("fn f(v: Vec<Option<u8>>) -> Vec<u8> {{\n    v.into_iter().map(Option::unwrap).collect()\n}}\n"), 2),
        ("G2",  "src/a.rs", fmt!("fn f(x: Option<u8>) -> u8 {{\n    x.expect(\"no\")\n}}\n"), 2),
        ("G2",  "src/a.rs", fmt!("fn f(x: Option<u8>) -> u8 {{\n    Option::expect(x, \"no\")\n}}\n"), 2),
        ("G3",  "src/a.rs", fmt!("fn f() {{\n    unsafe {{ g() }}\n}}\n"), 2),
        ("G3",  "src/a.rs", fmt!("fn f() {{}}\n#[unsafe(no_mangle)]\npub fn g() {{}}\n"), 2),
        ("G4",  "src/a.rs", fmt!("fn f(x: &str) -> Outcome<u8> {{\n    let v = parse(x)?;\n    Ok(v)\n}}\n"), 2),
        ("G4",  "src/a.rs", fmt!("fn f(x: Outcome<u8>) -> Outcome<u8> {{\n    let v = x?;\n    Ok(v)\n}}\n"), 2),
        ("G4",  "src/a.rs", fmt!("fn f(p: (Outcome<u8>,)) -> Outcome<u8> {{\n    Ok(p.0?)\n}}\n"), 2),
        ("G4",  "src/a.rs", fmt!("macro_rules! q {{\n    ($e:expr) => {{ $e? }};\n}}\n"), 2),
        ("G5",  "src/a.rs", fmt!("fn f() -> Outcome<()> {{\n    ok!(g(), \"context\");\n    Ok(())\n}}\n"), 2),
        ("G6",  "src/a.rs", fmt!("fn f(m: &Data) {{\n    let g = m.read();\n}}\n"), 2),
        ("G6",  "src/a.rs", fmt!("fn f(m: &RwLock<u8>) {{\n    let g = RwLock::read(m);\n}}\n"), 2),
        ("G6",  "src/a.rs", fmt!("fn f(m: &RwLock<u8>) {{\n    let g = RwLock::write(m);\n}}\n"), 2),
        ("G6",  "src/a.rs", fmt!("fn f(m: &Mutex<u8>) {{\n    let g = Mutex::lock(m);\n}}\n"), 2),
        ("G7",  "src/a.rs", fmt!("pub enum E {{\n    /// The first.\n    A,\n}}\n"), 2),
        ("G7",  "src/a.rs", fmt!("pub struct S {{\n    /// The size.\n    pub n: u8,\n}}\n"), 2),
        ("G7",  "src/a.rs", fmt!("/// The most.\npub const MOST: u8 = 3;\n"), 1),
        ("G7",  "src/a.rs", fmt!("pub struct S {{\n    /// The size.\n    #[optional]\n    pub n: u8,\n}}\n"), 2),
        ("G7",  "src/a.rs", fmt!("pub enum E {{\n    /// The first.\n    #[default]\n    A,\n}}\n"), 2),
        ("G7",  "src/a.rs", fmt!("/// The most.\n#[allow(dead_code)]\npub const MOST: u8 = 3;\n"), 1),
        ("G7",  "src/a.rs", fmt!("#[derive(Debug)]\npub struct S {{\n    /// The size.\n    #[optional]\n    #[serde(default)]\n    pub n: u8,\n}}\n"), 3),
        ("G7",  "src/a.rs", fmt!("pub enum E {{\n    A {{\n        /// The size.\n        #[optional]\n        n: u8,\n    }},\n}}\n"), 3),
        ("G7",  "src/a.rs", fmt!("impl S {{\n    /// The most.\n    #[optional]\n    pub(crate) const MOST: u8 = 3;\n}}\n"), 2),
        ("G7",  "src/a.rs", fmt!("pub struct S<F: Fn(u8)> {{\n    /// The call.\n    pub f: F,\n}}\n"), 2),
        ("G7",  "src/a.rs", fmt!("pub struct S<F> where F: Fn() {{\n    /// The call.\n    pub f: F,\n}}\n"), 2),
        ("G7",  "src/a.rs", fmt!("pub enum E<T: Into<Vec<u8>>> {{\n    /// The value.\n    A(T),\n}}\n"), 2),
        ("G7",  "src/a.rs", fmt!("pub struct S {{\n    /** The size. */\n    pub n: u8,\n}}\n"), 2),
        ("G7",  "src/a.rs", fmt!("/** The most. */\npub const MOST: u8 = 3;\n"), 1),
        ("G8",  "src/a.rs", fmt!("/// Parses.\n///\n/// # Errors\nfn f() {{}}\n"), 3),
        ("G8",  "src/a.rs", fmt!("/// Parses.\n///\n/// # Errors:\nfn f() {{}}\n"), 3),
        ("G9",  "src/a.rs", fmt!("use std::fmt;\nuse crate::x;\n"), 2),
        ("G9",  "src/a.rs", fmt!("use crate::a; use crate::b;\n"), 1),
        ("G10", "notes.txt", fmt!("one\nkey = {}\n", key), 2),
        ("G11", "src/a.rs", fmt!("fn f() {{\n    let p = \"/var/lib/app/state\";\n}}\n"), 2),
        ("G11", "src/a.rs", fmt!("fn f(h: &str) -> bool {{\n    h == \"api.example.com\"\n}}\n"), 2),
        ("G11", "src/a.rs", fmt!("fn f(m: &str) -> bool {{\n    m != \"claude-opus-4-1\"\n}}\n"), 2),
        ("G11", "src/a.rs", fmt!("fn f(p: &str) -> bool {{\n    p.starts_with(\"/home/jason/usr/\")\n}}\n"), 2),
        ("G11", "src/a.rs", fmt!("fn f(u: &str) -> bool {{\n    u.contains(\"https://api.example.com/v1\")\n}}\n"), 2),
        ("G11", "src/a.rs", fmt!("fn f(u: &str) -> bool {{\n    \"/etc/kimberlite/runner.jdat\".contains(u)\n}}\n"), 2),
        ("G11", "src/a.rs", fmt!("fn f(h: &str) -> bool {{\n    h == \"localhost\"\n}}\n"), 2),
        ("G11", "src/a.rs", fmt!("#[cfg(any(test, unix))]\nfn f() {{\n    let p = \"/var/lib/app/state\";\n}}\n"), 3),
        ("G12", "src/a.rs", fmt!("fn f(x: Box<dyn Fn()>) {{}}\n"), 1),
        ("G13", "src/a.rs", fmt!("// The behavior of it.\nfn f() {{}}\n"), 1),
        ("G13", "src/a.rs", fmt!("fn f() {{}} // Serialize it. Then Color it.\n"), 1),
        ("G14", "src/a.rs", fmt!("#[path = \"imp.txt\"]\nmod imp;\n"), 1),
        ("G14", "src/a.rs", fmt!("#[cfg_attr(unix, path = \"unix.in\")]\nmod imp;\n"), 1),
        ("G14", "src/a.rs", fmt!("fn f() {{\n    include!(\"gen.in\");\n}}\n"), 2),
        ("G14", "src/a.rs", fmt!("fn f() {{\n    std::include!(concat!(\"gen\", \".in\"));\n}}\n"), 2),
        ("G14", "src/a.rs", fmt!("mod rt {{\n    pub use tokio::main as test;\n}}\n"), 2),
        ("G14", "src/a.rs", fmt!("extern crate tokio_x as tokio;\n"), 1),
        ("G14", "src/a.rs", fmt!("#[path = \"../tests/imp.rs\"]\nmod imp;\n"), 1),
        ("G14", "build.rs", fmt!("fn main() {{\n    println!(\"cargo:{}-cfg=test\");\n}}\n", "rustc"), 2),
        ("G14", "build.rs", fmt!("fn main() {{\n    println!(\"cargo::{}-cfg={{}}\", c);\n}}\n", "rustc"), 2),
        ("G14", ".cargo/config.toml", fmt!("[build]\nrustflags = [\"-{}\", \"test\"]\n", "-cfg"), 2),
        ("G14", "tests/run.sh", fmt!("RUSTFLAGS='-{}=test' cargo build\n", "-cfg"), 1),
        ("G14", "src/a.rs", fmt!("fn f() {{\n    include!(\"../tests/imp.rs\");\n}}\n"), 2),
        ("G14", "Cargo.toml", fmt!("[package]\nname = \"x\"\n\n[lib]\npath = \"tests/lib.rs\"\n"), 5),
        ("G14", "x/Cargo.toml", fmt!("[package]\nbuild = 'tests/build.rs'\n"), 2),
        ("G14", "Cargo.toml", fmt!("[dependencies]\nh = {{ version = \"1\", path = \"helper/tests\" }}\n"), 2),
        ("G14", "Cargo.toml", fmt!("[lib]\nname = \"x\"\n\n[[test]]\npath = \"tests/t.rs\"\n[lib.x]\npath = \"tests/lib.rs\"\n"), 7),
    ]
}

#[test]
fn test_hematite_each_rule_refused_by_name_and_line() -> Outcome<()> {
    let rules = res!(rules());
    for (id, file, src, line) in seeds() {
        let rep = res!(gate(&rules, file, &src));
        assert_eq!(hits(&rep), vec![(id.to_string(), line)], "seed for {}:\n{}", id, src);
        let msgs: Vec<String> = rules.rules.iter().filter(|r| r.id == id)
            .map(|r| fmt!("{}:{}: {}", file, line, r.msg)).collect();
        assert!(msgs.contains(&fmt!("{}", rep.breaches[0])), "seed for {}: {}", id, rep.breaches[0]);
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

#[path = \"imp_unix.rs\"]
mod imp;

const T: &str = include_str!(\"table.jdat\");

fn k() { include!(\"gen.rs\"); }

// Derives serde's Serialize, so the map reads as JSON.
fn p(s: &str, c: char) -> bool {
    s.starts_with(\"./\") || s.ends_with(\"../\") || \"~/\" != s
        || \"/._-~\".contains(c) || s.strip_prefix(\"~/\").is_some()
}

async fn q(m: &tokio::sync::RwLock<u8>) -> u8 { *m.read().await + *m.write().await }
";
    let rep = res!(gate(&rules, "src/a.rs", src));
    assert_eq!(hits(&rep), Vec::<(String, usize)>::new());
    // A build script setting a cfg of its own, and one whose name only begins `test`.
    let src = "fn main() {\n    println!(\"cargo:rustc-cfg=SABER_SCHEME=\\\"SABER\\\"\");\n    println!(\"cargo:rustc-cfg=testing\");\n}\n";
    assert!(res!(gate(&rules, "build.rs", src)).is_clean());
    // Cargo's own test layout, and a test module reading a file under `tests/`.
    let toml = "[package]\ninclude = [\"src/**\", \"tests/**\"]\n\n[[test]]\nname = \"t\"\npath = \"tests/t.rs\"\n";
    assert!(res!(gate(&rules, "Cargo.toml", toml)).is_clean());
    let src = "#[cfg(test)]\nmod t {\n    #[path = \"../tests/common.rs\"]\n    mod common;\n}\n";
    assert!(res!(gate(&rules, "src/a.rs", src)).is_clean());
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
    let added = hematite::file_diffs(diff);
    let want: BTreeSet<usize> = [3].into_iter().collect();
    assert_eq!(added.get("src/a.rs").map(|d| &d.lines), Some(&want));
    let src = "fn f(x: &str, y: Option<u8>) -> Outcome<u8> {\n    let w = y.unwrap();\n    let v = res!(parse(x));\n    Ok(v + w)\n}\n";
    let rep = res!(gate_lines(&rules, "src/a.rs", src, &[3]));
    assert!(rep.is_clean(), "{:?}", rep);
    // The same file with every line counted as added is refused, so the pass above is the diff's.
    let rep = res!(gate(&rules, "src/a.rs", src));
    assert_eq!(hits(&rep), vec![(fmt!("G1"), 2)]);
    Ok(())
}

#[test]
fn test_hematite_file_diffs_reads_hunks() -> Outcome<()> {
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
diff --git a/src/imp.rs b/src/imp.rs
new file mode 120000
index 0000000..1c5c3f5
--- /dev/null
+++ b/src/imp.rs
@@ -0,0 +1 @@
+../data/imp.txt
\\ No newline at end of file
";
    let added = hematite::file_diffs(diff);
    let want: BTreeSet<usize> = [1, 2, 3, 12, 13].into_iter().collect();
    assert_eq!(added.get("x.rs").map(|d| (&d.lines, d.link)), Some((&want, false)));
    let one: BTreeSet<usize> = [1].into_iter().collect();
    assert_eq!(added.get("src/imp.rs").map(|d| (&d.lines, d.link)), Some((&one, true)));
    assert_eq!(added.len(), 2);
    let hunks = added.get("x.rs").map(|d| (d.old, d.hunks.iter()
        .map(|h| (h.old_start, h.old_count, h.new_start, h.new_count)).collect::<Vec<_>>()));
    assert_eq!(hunks, Some((true, vec![(1, 2, 1, 3), (10, 0, 12, 2)])));
    assert_eq!(added.get("src/imp.rs").map(|d| d.old), Some(false));
    Ok(())
}

#[test]
fn test_hematite_line_moved_from_test_to_product_is_added() -> Outcome<()> {
    // Deleting a `#[cfg(test)]` line makes its module product code (QA hgate4 E).
    let rules = res!(rules());
    let old = "pub fn a() {}\n#[cfg(test)]\nmod t {\n    fn f(x: Box<dyn Fn()>) {}\n}\n";
    let new = "pub fn a() {}\nmod t {\n    fn f(x: Box<dyn Fn()>) {}\n}\n";
    let diff = "--- a/src/a.rs\n+++ b/src/a.rs\n@@ -2 +1,0 @@\n-#[cfg(test)]\n";
    let d = res!(hematite::file_diffs(diff).remove("src/a.rs").ok_or_else(|| err!("no diff"; Missing)));
    let a = res!(hematite::added(fmt!("src/a.rs"), d, fmt!("{}", new), Some(old), &rules));
    assert_eq!(hits(&res!(hematite::check(&[a], &rules))), vec![(fmt!("G12"), 3)]);
    // A line added above a test module leaves the module test code.
    let new = "use crate::x;\npub fn a() {}\n#[cfg(test)]\nmod t {\n    fn f(x: Box<dyn Fn()>) {}\n}\n";
    let diff = "--- a/src/a.rs\n+++ b/src/a.rs\n@@ -0,0 +1 @@\n+use crate::x;\n";
    let d = res!(hematite::file_diffs(diff).remove("src/a.rs").ok_or_else(|| err!("no diff"; Missing)));
    let a = res!(hematite::added(fmt!("src/a.rs"), d, fmt!("{}", new), Some(old), &rules));
    let want: BTreeSet<usize> = [1].into_iter().collect();
    assert_eq!(a.lines, want);
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
    // Line 3 only names the escape, so it breaks no rule and is not listed.
    let src = "fn f(x: &str) -> Outcome<u8> {\n    let v = parse(x)?; // allowlist hematite: a seeded case\n    // Mark a line allowlist hematite to let it through.\n    Ok(v)\n}\n";
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
    let rep = res!(gate(&rules, "fe2o3_x/tests/data/a.rs", "// The color.\n"));
    assert!(rep.is_clean(), "{:?}", rep);
    // A cfg that only holds under test is test code; one that also builds outside tests is not.
    let src = "#[cfg(any(test, unix))]\nfn t() {\n    // The behavior.\n}\n";
    assert_eq!(hits(&res!(gate(&rules, "src/a.rs", src))), vec![(fmt!("G13"), 3)]);
    for cfg in ["all(test, unix)", "all(unix, test)", "all(unix, any(test, all(test, windows)))"] {
        let src = fmt!("#[cfg({})]\nfn t() {{\n    // The behavior.\n    let p = \"/var/lib/app/state\";\n}}\n", cfg);
        let rep = res!(gate(&rules, "src/a.rs", &src));
        assert!(rep.is_clean(), "{}: {:?}", cfg, rep);
    }
    for cfg in ["any(test, unix)", "not(test)", "all(unix, not(test))", "testing", "all(testing, unix)"] {
        let src = fmt!("#[cfg({})]\nfn t() {{\n    let p = \"/var/lib/app/state\";\n}}\n", cfg);
        assert_eq!(hits(&res!(gate(&rules, "src/a.rs", &src))), vec![(fmt!("G11"), 3)], "{}", cfg);
    }
    // A test attribute by its exact path only, never an alias ending `::test` (QA hgate4 H).
    for attr in ["rt::test", "self::test", "crate::rt::test", "tokio::testing", "tokio::test_x"] {
        let src = fmt!("#[{}]\npub async fn serve() {{\n    let p = \"/var/lib/app/state\";\n}}\n", attr);
        assert_eq!(hits(&res!(gate(&rules, "src/a.rs", &src))), vec![(fmt!("G11"), 3)], "{}", attr);
    }
    for attr in ["test", "tokio::test", "::tokio::test", "tokio::test(flavor = \"multi_thread\")"] {
        let src = fmt!("#[{}]\nasync fn t() {{\n    let p = \"/var/lib/app/state\";\n}}\n", attr);
        assert!(res!(gate(&rules, "src/a.rs", &src)).is_clean(), "{}", attr);
    }
    // A crate placed under `tests/` is built as product (QA hgate4 C).
    for file in ["src/util_tests.rs", "src/tests.rs", "src/tests/mod.rs", "fe2o3_x/src/tests/a.rs",
        "helper/tests/src/lib.rs", "tests/x/src/a.rs"]
    {
        assert_eq!(hits(&res!(gate(&rules, file, "// The color.\n"))), vec![(fmt!("G13"), 1)], "{}", file);
    }
    Ok(())
}

#[test]
fn test_hematite_unknown_check_is_refused_and_seq_rule_needs_no_code() -> Outcome<()> {
    let bad = res!(Dat::decode_string(fmt!("{{\"local_crates\": [], \"test_attrs\": [], \"rules\": [{{\"id\": \"X\", \
        \"check\": \"nosuch\", \"on\": true, \"in_tests\": true, \"fixed_ok\": false, \"msg\": \"m\"}}]}}")));
    assert!(lint::rules_from_dat(&bad).is_err());
    let dat = res!(Dat::decode_string(fmt!("{{\"local_crates\": [], \"test_attrs\": [], \"rules\": [{{\"id\": \"Q1\", \
        \"check\": \"seq\", \"seq\": [\"panic\", \"!\"], \"on\": true, \"in_tests\": true, \
        \"fixed_ok\": false, \"msg\": \"panic! -- return an err!\"}}]}}")));
    let rules = res!(lint::rules_from_dat(&dat));
    let rep = res!(gate(&rules, "src/a.rs", "fn f() {\n    panic!(\"no\");\n}\n"));
    assert_eq!(hits(&rep), vec![(fmt!("Q1"), 2)]);
    Ok(())
}

#[test]
fn test_hematite_built_in_table_is_the_file() -> Outcome<()> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("hematite_rules.jdat");
    let text = match std::fs::read_to_string(&path) {
        Ok(s)   => s,
        Err(e)  => return Err(err!(e, "While reading {}.", path.display(); IO, File, Read)),
    };
    assert_eq!(lint::HEMATITE_RULES, text);
    let rules = res!(lint::parse_rules(lint::HEMATITE_RULES));
    assert_eq!(rules.rules.len(), res!(lint::load_rules(&path)).rules.len());
    Ok(())
}

#[test]
fn test_hematite_the_gate_meets_its_own_rules() -> Outcome<()> {
    let rules = res!(rules());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    for file in ["fe2o3_text/src/lint/hematite.rs", "fe2o3_jdat/src/lint.rs",
        "fe2o3_jdat/src/bin/lint_hematite.rs"]
    {
        let path = root.join(file);
        let src = match std::fs::read_to_string(&path) {
            Ok(s)   => s,
            Err(e)  => return Err(err!(e, "While reading {}.", path.display(); IO, File, Read)),
        };
        let rep = res!(gate(&rules, file, &src));
        assert!(rep.is_clean(), "{}:\n{}", file,
            rep.breaches.iter().map(|b| fmt!("{}", b)).collect::<Vec<_>>().join("\n"));
    }
    Ok(())
}
