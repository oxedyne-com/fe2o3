//! Per-user workspace — the sandboxed directory the agent operates in.
//!
//! A workspace is a single directory on the Red host.  All agent file
//! operations resolve through `resolve()`, which jails paths to the
//! workspace root.  In the trusted, self-hosted environment (plan D0)
//! this is an *accident* guardrail — keeping the agent inside the
//! workspace by default — not a hardened *attack* boundary.
//!
//! The `resolve` / `display_rel` path logic is pure and target-agnostic.
//! The backing store is `std::fs`, which compiles on wasm32 but returns
//! "unsupported" at runtime — the browser filesystem is OPFS.
// TODO(wasm-opfs): back `Workspace` (and the file tools in `tools.rs`)
// with an OPFS-backed store on wasm32.  This requires an async fs
// surface (OPFS access is async), so it is deferred to the browser
// tool-execution stage rather than bolted on here.

use oxedyne_fe2o3_core::prelude::*;

use std::path::{Component, Path, PathBuf};


/// A sandboxed working directory for one user.
#[derive(Clone, Debug)]
pub struct Workspace {
    /// Canonical absolute path to the workspace root.
    root: PathBuf,
}

impl Workspace {

    /// Open (creating if necessary) a workspace rooted at `root`.
    pub fn new(root: PathBuf) -> Outcome<Self> {
        if !root.exists() {
            res!(std::fs::create_dir_all(&root)
                .map_err(|e| err!(e, "Workspace: create root {:?} failed.", root; IO, File)));
        }
        let root = res!(std::fs::canonicalize(&root)
            .map_err(|e| err!(e, "Workspace: canonicalise {:?} failed.", root; IO, File)));
        Ok(Self { root })
    }

    /// The workspace root path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a workspace-relative path to an absolute path, jailed to
    /// the root.  Absolute inputs and `..` traversal that escapes the
    /// root are rejected.  The path is built lexically (no filesystem
    /// access), then checked to remain within the root.
    pub fn resolve(&self, rel: &str) -> Outcome<PathBuf> {
        let rel = rel.trim_start_matches('/');
        let mut out = self.root.clone();
        for comp in Path::new(rel).components() {
            match comp {
                Component::Normal(c) => out.push(c),
                Component::CurDir    => {},
                Component::ParentDir => {
                    // Pop, but never above the root.
                    if !out.pop() || !out.starts_with(&self.root) {
                        return Err(err!(
                            "Workspace: path '{}' escapes the workspace.", rel;
                            Invalid, Input, Path));
                    }
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(err!(
                        "Workspace: absolute path '{}' is not allowed.", rel;
                        Invalid, Input, Path));
                }
            }
        }
        if !out.starts_with(&self.root) {
            return Err(err!(
                "Workspace: path '{}' escapes the workspace.", rel;
                Invalid, Input, Path));
        }
        Ok(out)
    }

    /// Display a resolved path as a workspace-relative string (for
    /// user-facing tool output).  Falls back to the full path if the
    /// path is somehow outside the root.
    pub fn display_rel(&self, p: &Path) -> String {
        match p.strip_prefix(&self.root) {
            Ok(r) => {
                let s = r.to_string_lossy().to_string();
                if s.is_empty() { ".".to_string() } else { s }
            }
            Err(_) => p.to_string_lossy().to_string(),
        }
    }
}


// ┌───────────────────────────────────────────────────────────────┐
// │ Tests                                                          │
// └───────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_ws() -> Workspace {
        let mut dir = std::env::temp_dir();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        dir.push(fmt!("red_ws_test_{}", n));
        Workspace::new(dir).expect("workspace")
    }

    #[test]
    fn test_resolve_normal() {
        let ws = tmp_ws();
        let p = ws.resolve("sub/file.txt").expect("resolve");
        assert!(p.starts_with(ws.root()));
        assert!(p.ends_with("sub/file.txt"));
    }

    #[test]
    fn test_resolve_leading_slash_treated_relative() {
        let ws = tmp_ws();
        let p = ws.resolve("/etc/passwd").expect("resolve");
        assert!(p.starts_with(ws.root()));
        assert!(p.ends_with("etc/passwd"));
    }

    #[test]
    fn test_resolve_escape_rejected() {
        let ws = tmp_ws();
        assert!(ws.resolve("../../../etc/passwd").is_err());
        assert!(ws.resolve("a/../../b").is_err());
    }

    #[test]
    fn test_resolve_curdir_and_reentry_ok() {
        let ws = tmp_ws();
        assert!(ws.resolve("./a/b").is_ok());
        // Leaves a subdir then re-enters the root — stays inside.
        assert!(ws.resolve("a/../b").is_ok());
    }

    #[test]
    fn test_display_rel() {
        let ws = tmp_ws();
        let p = ws.resolve("x/y.rs").expect("resolve");
        assert_eq!(ws.display_rel(&p), "x/y.rs");
        assert_eq!(ws.display_rel(ws.root()), ".");
    }
}
