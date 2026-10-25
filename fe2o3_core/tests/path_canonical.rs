//! `path::canonicalise_allow_missing` follows a symlink in the part of a path that exists and
//! places the part that does not, so a containment check cannot be fooled by either.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::{
    prelude::*,
    path::canonicalise_allow_missing,
};

use std::path::PathBuf;

/// A fresh directory under the cargo target, never `/tmp` (a tmpfs).
fn scratch(label: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(fmt!("{}_{}", label, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::canonicalize(&dir).expect("canonical scratch dir")
}

#[test]
fn test_canonicalise_allow_missing_places_a_path_that_is_not_there() -> Outcome<()> {
    let root = scratch("cam_missing");
    let got = res!(canonicalise_allow_missing(&root.join("keys/./oregami/../oregami/service_key")));
    assert_eq!(got, root.join("keys/oregami/service_key"));
    Ok(())
}

#[test]
fn test_canonicalise_allow_missing_follows_a_symlink_out() -> Outcome<()> {
    let root    = scratch("cam_link");
    let outside = scratch("cam_outside");
    std::os::unix::fs::symlink(&outside, root.join("keys")).expect("symlink");
    // The link exists, the file below it does not: the answer is where the link points.
    let got = res!(canonicalise_allow_missing(&root.join("keys/sk")));
    assert_eq!(got, outside.join("sk"));
    assert!(!got.starts_with(&root), "a symlink out must not read as under the root");
    Ok(())
}

#[test]
fn test_canonicalise_allow_missing_climbs_out_with_dot_dot() -> Outcome<()> {
    let root = scratch("cam_up");
    let got = res!(canonicalise_allow_missing(&root.join("nope/../../elsewhere")));
    assert_eq!(Some(got.as_path()), root.parent().map(|p| p.join("elsewhere")).as_deref());
    Ok(())
}
