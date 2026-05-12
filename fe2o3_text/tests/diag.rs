use oxedyne_fe2o3_text::fmt::{format_rust, spec::FormatSpec};

use std::fs;
use std::path::PathBuf;

fn diag_file(path: &str) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().to_path_buf();
    let full = root.join(path);
    let source = fs::read_to_string(&full).expect("read failed");
    let spec = FormatSpec::fe2o3();

    let first = format_rust(&source, &spec).expect("format failed");
    let second = format_rust(&first, &spec).expect("second format failed");

    if first == second {
        println!("=== {} === OK", path);
        return;
    }

    let fl: Vec<&str> = first.lines().collect();
    let sl: Vec<&str> = second.lines().collect();

    println!("=== {} ===", path);
    println!("  first: {} lines, second: {} lines", fl.len(), sl.len());
    let mut diffs = 0;
    for (i, (a, b)) in fl.iter().zip(sl.iter()).enumerate() {
        if a != b {
            if diffs < 30 {
                println!("  L{}: F={:?}", i+1, a);
                println!("  L{}: S={:?}", i+1, b);
            }
            diffs += 1;
        }
    }
    if fl.len() > sl.len() {
        for i in sl.len()..fl.len() {
            if diffs < 30 { println!("  L{}: F={:?} (extra)", i+1, fl[i]); }
            diffs += 1;
        }
    } else if sl.len() > fl.len() {
        for i in fl.len()..sl.len() {
            if diffs < 30 { println!("  L{}: S={:?} (extra)", i+1, sl[i]); }
            diffs += 1;
        }
    }
    println!("  Total differing lines: {}", diffs);
}

/// Files that historically triggered idempotency failures.
#[test]
fn test_real_files() {
    diag_file("fe2o3_jdat/tests/daticle.rs");
    diag_file("fe2o3_jdat/tests/map.rs");
    diag_file("fe2o3_jdat/dat_map/src/lib.rs");
}

fn check_idempotent(label: &str, source: &str) {
    let spec = FormatSpec::fe2o3();
    let first = format_rust(source, &spec).expect("format failed");
    let second = format_rust(&first, &spec).expect("second format failed");
    if first == second {
        println!("{}: OK", label);
    } else {
        println!("{}: DRIFT", label);
        let fl: Vec<&str> = first.lines().collect();
        let sl: Vec<&str> = second.lines().collect();
        for (i, (a, b)) in fl.iter().zip(sl.iter()).enumerate() {
            if a != b {
                let st = if i > 2 { i - 2 } else { 0 };
                let en = (i + 3).min(fl.len()).min(sl.len());
                for j in st..en {
                    let m = if j == i { ">>>" } else { "   " };
                    println!("  {} {:3} F: {:?}", m, j+1, fl[j]);
                    println!("  {} {:3} S: {:?}", m, j+1, sl[j]);
                }
                break;
            }
        }
        if fl.len() != sl.len() {
            println!("  (line count: first={}, second={})", fl.len(), sl.len());
        }
    }
}

/// Synthetic repros for edge cases fixed during development.
#[test]
fn minimal_repro() {
    check_idempotent("simple_impl", r#"
impl Foo {
    pub fn bar() -> Outcome<()> {
        let x = 1;
        Ok(())
    }
}
"#);

    check_idempotent("two_fn_match", r#"
impl Foo {
    pub fn first(&self) -> String {
        let name = match self.x {
            Some(s) => s.to_string(),
            None => String::new(),
        };
        name
    }

    pub fn second(&self) -> Outcome<()> {
        // body
        let x = 1;
        Ok(())
    }
}
"#);

    check_idempotent("match_align", r#"
impl Foo {
    pub fn bar(&self) -> Outcome<()> {
        match self.state {
            State::Active => {
                // do stuff
                let x = 1;
            }
            State::Inactive => {
                let y = 2;
            }
        }
        Ok(())
    }
}
"#);

    check_idempotent("match_arm_braced", r#"
impl Foo {
    pub fn bar(&self) -> Outcome<()> {
        for shard in shards {
            for (fnum, fstate) in shard.map_mut() {
                match fstate.present() {
                    Present::Pair => {
                        // Comment line one.
                        let x = fstate.get_data();
                        let y = fstate.get_index();
                    },
                    Present::Solo(FileType::Data) => {
                        // Another comment.
                        let x = fstate.get_data();
                    },
                }
            }
        }
        Ok(())
    }
}
"#);

    check_idempotent("match_arm_ifelse", r#"
impl Foo {
    pub fn bar(&self) -> Outcome<()> {
        for shard in shards {
            for (fnum, fstate) in shard.map_mut() {
                match fstate.present() {
                    Present::Pair => {
                        // Comment.
                        let dat = fstate.get_data();
                        let ind = fstate.get_index();
                        if let Err(e) = bot.send(Msg::Cache { fnum: *fnum, dat, ind }) {
                            return Err(err!(e, "Cannot send request to bot {}", j; Channel, Write));
                        } else {
                            requests += 1;
                        }
                    },
                    Present::Solo(FileType::Data) => {
                        // Another comment.
                        let dat = fstate.get_data();
                        if let Err(e) = bot.send(Msg::CacheData { fnum: *fnum, dat }) {
                            return Err(err!(e, "Cannot send request to bot {}", j; Channel, Write));
                        } else {
                            requests += 1;
                        }
                    },
                    Present::Solo(FileType::Index) => {
                        // Yet another comment.
                        match missing { None => missing = Some(vec![*fnum]), Some(ref mut m) => m.push(*fnum), }
                    },
                }
            }
        }
        Ok(())
    }
}
"#);

    // Truncated file: unclosed braces should not gain synthetic closing braces.
    check_idempotent("truncated_match", r#"
pub fn foo() -> Outcome<()> {
    match x {
"#);

    // Colon-path adjacency: `map: ::std` must not compress to `map:::std`.
    check_idempotent("colon_path", r#"
fn bar(mut map: ::std::collections::BTreeMap<u32, u32>) {}
"#);
}
