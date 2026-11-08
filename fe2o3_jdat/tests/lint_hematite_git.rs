//! The `lint_hematite` CLI on scratch repositories: the repository cannot decide what counts as a
//! change (QA hgate 08 Nov, F1-F3, F5). Each case commits a `x.unwrap()` (or a `dyn` moved out of
//! `tests/`) that a repository setting would hide, and the gate must still refuse it.

use oxedyne_fe2o3_core::prelude::*;

use std::{
    path::{
        Path,
        PathBuf,
    },
    process::Command,
};


fn git(repo: &Path, args: &[&str]) -> Outcome<()> {
    // Hooks off: a throwaway repository under the temporary directory, never kept.
    let out = res!(Command::new("git").arg("-C").arg(repo)
        .args(["-c", "core.hooksPath=/dev/null", "-c", "user.email=q@x", "-c", "user.name=q"])
        .args(args).output().map_err(|e| err!(e, "Could not run git {:?}.", args; IO)));
    if !out.status.success() {
        return Err(err!("git {:?} failed: {}", args, String::from_utf8_lossy(&out.stderr); IO));
    }
    Ok(())
}

fn write(repo: &Path, file: &str, body: &[u8]) -> Outcome<()> {
    let p = repo.join(file);
    if let Some(d) = p.parent() {
        res!(std::fs::create_dir_all(d).map_err(|e| err!(e, "mkdir {}", d.display(); IO)));
    }
    res!(std::fs::write(&p, body).map_err(|e| err!(e, "write {}", p.display(); IO)));
    Ok(())
}

/// A repository with a base commit on `main` and a branch `c` checked out.
fn repo(name: &str) -> Outcome<PathBuf> {
    let r = std::env::temp_dir().join(fmt!("hematite_git_{}_{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&r);
    res!(std::fs::create_dir_all(&r).map_err(|e| err!(e, "mkdir {}", r.display(); IO)));
    res!(git(&r, &["init", "-q", "-b", "main", "."]));
    res!(write(&r, "src/lib.rs", b"pub fn base() {}\n"));
    res!(write(&r, "tests/movable.rs", b"pub trait T {}\npub fn moved(x: Box<dyn T>) -> Box<dyn T> { x }\n"));
    res!(git(&r, &["add", "-A"]));
    res!(git(&r, &["commit", "-qm", "base"]));
    res!(git(&r, &["checkout", "-q", "-b", "c"]));
    Ok(r)
}

fn commit(r: &Path) -> Outcome<()> {
    res!(git(r, &["add", "-A"]));
    git(r, &["commit", "-qm", "change"])
}

/// The CLI's exit code and output for the change `main...HEAD`.
fn gate(r: &Path) -> Outcome<(i32, String)> {
    let rules = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("hematite_rules.jdat");
    let out = res!(Command::new(env!("CARGO_BIN_EXE_lint_hematite"))
        .arg("--rules").arg(&rules).arg("--repo").arg(r).args(["--base", "main", "--head", "HEAD"])
        .output().map_err(|e| err!(e, "Could not run lint_hematite."; IO)));
    let _ = std::fs::remove_dir_all(r);
    Ok((out.status.code().unwrap_or(-1), fmt!("{}{}", String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr))))
}

const UNWRAP: &[u8] = b"pub fn f(x: Option<u8>) -> u8 { x.unwrap() }\n";

#[test]
fn test_hematite_git_attributes_in_the_change_refused() -> Outcome<()> {
    let r = res!(repo("f1"));
    res!(write(&r, ".gitattributes", b"*.rs -diff\n"));
    res!(write(&r, "src/a.rs", UNWRAP));
    res!(commit(&r));
    let (rc, out) = res!(gate(&r));
    assert!(rc == 2 && out.contains(".gitattributes"), "rc {}: {}", rc, out);
    Ok(())
}

#[test]
fn test_hematite_git_info_attributes_refused() -> Outcome<()> {
    let r = res!(repo("f2"));
    res!(write(&r, "src/a.rs", UNWRAP));
    res!(commit(&r));
    res!(write(&r, ".git/info/attributes", b"*.rs binary\n"));
    let (rc, out) = res!(gate(&r));
    assert!(rc == 2 && out.contains("info/attributes"), "rc {}: {}", rc, out);
    Ok(())
}

#[test]
fn test_hematite_git_nul_in_a_comment_still_read() -> Outcome<()> {
    let r = res!(repo("f3"));
    res!(write(&r, "src/a.rs", b"// a \0 byte\npub fn f(x: Option<u8>) -> u8 { x.unwrap() }\n"));
    res!(commit(&r));
    let (rc, out) = res!(gate(&r));
    assert!(rc == 1 && out.contains("src/a.rs:2: unwrap()"), "rc {}: {}", rc, out);
    Ok(())
}

#[test]
fn test_hematite_git_rename_out_of_tests_read_whole() -> Outcome<()> {
    let r = res!(repo("f5"));
    res!(git(&r, &["mv", "tests/movable.rs", "src/movable.rs"]));
    res!(commit(&r));
    let (rc, out) = res!(gate(&r));
    assert!(rc == 1 && out.contains("src/movable.rs:2: dyn Trait"), "rc {}: {}", rc, out);
    Ok(())
}

#[test]
fn test_hematite_git_symlink_refused() -> Outcome<()> {
    // Rust behind a link to a file the gate would not read as Rust (QA hgate4 B).
    let r = res!(repo("link"));
    res!(write(&r, "data/imp.txt", UNWRAP));
    res!(write(&r, "src/lib.rs", b"pub fn base() {}\npub mod imp;\n"));
    match std::os::unix::fs::symlink("../data/imp.txt", r.join("src/imp.rs")) {
        Ok(())  => {}
        Err(e)  => return Err(err!(e, "symlink"; IO)),
    }
    res!(commit(&r));
    let (rc, out) = res!(gate(&r));
    assert!(rc == 1 && out.contains("src/imp.rs:1: symlink"), "rc {}: {}", rc, out);
    Ok(())
}

#[test]
fn test_hematite_git_repo_config_cannot_hide_lines() -> Outcome<()> {
    // An attributes file named by the repository's config, and hunks merged across an empty line
    // that the config prints bare.
    let r = res!(repo("cfg"));
    let af = r.join(".git").join("qa_attrs");
    res!(write(&r, ".git/qa_attrs", b"*.rs binary\n"));
    res!(git(&r, &["config", "core.attributesFile", &fmt!("{}", af.display())]));
    res!(git(&r, &["config", "diff.interHunkContext", "50"]));
    res!(git(&r, &["config", "diff.suppressBlankEmpty", "true"]));
    res!(git(&r, &["checkout", "-q", "main"]));
    res!(write(&r, "src/b.rs", b"pub fn one() {}\n\npub fn two() {}\n"));
    res!(commit(&r));
    res!(git(&r, &["checkout", "-q", "-B", "c"]));
    res!(write(&r, "src/b.rs", b"pub fn one() { }\n\npub fn f(x: Option<u8>) -> u8 { x.unwrap() }\n"));
    res!(commit(&r));
    let (rc, out) = res!(gate(&r));
    assert!(rc == 1 && out.contains("src/b.rs:3: unwrap()"), "rc {}: {}", rc, out);
    Ok(())
}
