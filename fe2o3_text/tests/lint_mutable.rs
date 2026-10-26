//! Fixture tests for the mutable-literal gate, `lint::mutable`: one source per flagged class and
//! one per exemption, each asserting what is and is not a hit, then the ratchet baseline.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::lint::mutable::{
    self,
    Allow,
    Baseline,
    Class,
    Hit,
    Lang,
    Scan,
};

use std::path::PathBuf;


fn fixture(name: &str) -> Scan {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("lint_mutable");
    let src = match std::fs::read_to_string(dir.join(name)) {
        Ok(s)   => s,
        Err(e)  => panic!("Reading fixture {}: {}", name, e),
    };
    let lang = if name.ends_with(".js") { Lang::Js } else { Lang::Rust };
    match mutable::scan(&src, lang, name) {
        Ok(scan)    => scan,
        Err(e)      => panic!("Scanning fixture {}: {}", name, e),
    }
}

fn texts(scan: &Scan, class: Class) -> Vec<String> {
    scan.hits.iter().filter(|h| h.class == class).map(|h| h.text.clone()).collect()
}

#[test]
fn test_lint_path_strings_hit() -> Outcome<()> {
    let scan = fixture("hit_path.rs");
    assert_eq!(texts(&scan, Class::Path),
        vec!["/var/lib/app/state", "~/data", "./config.jdat", "store.jdat"]);
    assert_eq!(scan.hits.len(), 4);
    assert_eq!(scan.hits[0].line, 2);
    Ok(())
}

#[test]
fn test_lint_host_strings_hit() -> Outcome<()> {
    let scan = fixture("hit_host.rs");
    assert_eq!(texts(&scan, Class::Host),
        vec!["https://api.example.com/v1", "127.0.0.1:8443", "pool.ntp.org", "localhost:8080"]);
    assert_eq!(scan.hits.len(), 4);
    Ok(())
}

#[test]
fn test_lint_model_strings_hit() -> Outcome<()> {
    let scan = fixture("hit_model.rs");
    assert_eq!(texts(&scan, Class::Model), vec!["gpt-4o-mini"]);
    assert_eq!(scan.hits.len(), 1);
    Ok(())
}

#[test]
fn test_lint_named_consts_hit_by_vocabulary() -> Outcome<()> {
    let scan = fixture("hit_const.rs");
    assert_eq!(texts(&scan, Class::Named),
        vec!["REQUEST_TIMEOUT_SECS", "MAX_BODY", "RETRY_COUNT", "DEFAULT_PORT"]);
    assert_eq!(scan.hits.len(), 4);
    Ok(())
}

#[test]
fn test_lint_timing_calls_hit_bare_numbers_do_not() -> Outcome<()> {
    let scan = fixture("hit_call.rs");
    assert_eq!(texts(&scan, Class::Timing), vec!["Duration::from_millis(250)"]);
    assert_eq!(scan.hits.len(), 1);
    Ok(())
}

#[test]
fn test_lint_js_hits() -> Outcome<()> {
    let scan = fixture("hit_js.js");
    assert_eq!(texts(&scan, Class::Named),  vec!["POLL_INTERVAL_MS"]);
    assert_eq!(texts(&scan, Class::Host),   vec!["https://api.example.com"]);
    assert_eq!(texts(&scan, Class::Timing), vec!["setTimeout(1500)"]);
    assert_eq!(scan.hits.len(), 3);
    Ok(())
}

#[test]
fn test_lint_comments_are_skipped() -> Outcome<()> {
    assert!(fixture("skip_comment.rs").hits.is_empty());
    Ok(())
}

#[test]
fn test_lint_message_macros_are_skipped() -> Outcome<()> {
    let scan = fixture("skip_macro.rs");
    assert!(scan.hits.is_empty(), "{:?}", scan.hits);
    Ok(())
}

#[test]
fn test_lint_test_items_are_skipped() -> Outcome<()> {
    let scan = fixture("skip_test.rs");
    assert!(scan.hits.is_empty(), "{:?}", scan.hits);
    Ok(())
}

#[test]
fn test_lint_config_default_is_the_one_home() -> Outcome<()> {
    // The `Default` of a `FromDatMap` type is skipped; that of a plain type is not.
    let scan = fixture("skip_default.rs");
    assert_eq!(texts(&scan, Class::Host), vec!["https://plain.example.com"]);
    assert_eq!(scan.hits.len(), 1);
    Ok(())
}

#[test]
fn test_lint_fixed_marker_trailing_and_banner() -> Outcome<()> {
    let scan = fixture("skip_fixed.rs");
    assert_eq!(texts(&scan, Class::Path), vec!["/etc/app/x.jdat"]);
    assert_eq!(scan.hits.len(), 1);
    // A one-word why is itself a failure.
    assert_eq!(scan.bad_fixed.len(), 1);
    assert_eq!(scan.bad_fixed[0].line, 8);
    Ok(())
}

fn hit(file: &str, class: Class, text: &str) -> Hit {
    Hit { file: file.to_string(), line: 1, class, text: text.to_string() }
}

#[test]
fn test_lint_baseline_ratchet() -> Outcome<()> {
    let hits = vec![
        hit("a.rs", Class::Path, "/x"),
        hit("a.rs", Class::Path, "/x"),
        hit("b.rs", Class::Named, "MAX_N"),
    ];
    // A baseline made from the hits passes them.
    let base = Baseline::from_hits(&hits);
    let rep = mutable::check(&hits, &[], &base);
    assert!(rep.is_clean(), "{}", rep);

    // One more copy of a baselined literal is new.
    let mut more = hits.clone();
    more.push(hit("a.rs", Class::Path, "/x"));
    let rep = mutable::check(&more, &[], &base);
    assert_eq!(rep.new.len(), 1);
    assert!(!rep.is_clean());

    // A literal migrated away leaves a stale entry, which also fails.
    let rep = mutable::check(&hits[..2], &[], &base);
    assert_eq!(rep.stale.len(), 1);
    assert_eq!(rep.stale[0].text, "MAX_N");
    assert!(!rep.is_clean());

    // An allow entry covers a hit, but only with a non-empty why.
    let mut base2 = Baseline::from_hits(&hits[..2]);
    base2.allow.push(Allow {
        file:   "b.rs".to_string(),
        class:  Class::Named,
        text:   "MAX_N".to_string(),
        why:    "Generated from the Unicode tables.".to_string(),
    });
    assert!(mutable::check(&hits, &[], &base2).is_clean());
    base2.allow[0].why = String::new();
    let rep = mutable::check(&hits, &[], &base2);
    assert_eq!(rep.bad_allow.len(), 1);
    assert!(!rep.is_clean());
    Ok(())
}
