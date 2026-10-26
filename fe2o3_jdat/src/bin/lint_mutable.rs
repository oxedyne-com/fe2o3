//! `lint_mutable --root <dir> [--baseline <file>] [--write]`: the mutable-literal gate for any
//! tree. Exits non-zero when the tree has a literal beyond its baseline, a stale baseline entry, a
//! thin `// fixed:` marker or an allow entry without a why. `--write` regenerates the baseline from
//! the tree, keeping its allow list.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::lint;
use oxedyne_fe2o3_text::lint::mutable;

use std::path::PathBuf;


// fixed: the bootstrap name of the baseline at a tree root, overridden by --baseline.
const BASELINE_NAME: &str = "mutable_literals.jdat";

fn main() {
    match run() {
        Ok(true)    => {}
        Ok(false)   => std::process::exit(1),
        Err(e)      => {
            eprintln!("{}", e);
            std::process::exit(2);
        }
    }
}

fn run() -> Outcome<bool> {
    let mut root: Option<PathBuf> = None;
    let mut base_path: Option<PathBuf> = None;
    let mut write = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--root"        => root = args.next().map(PathBuf::from),
            "--baseline"    => base_path = args.next().map(PathBuf::from),
            "--write"       => write = true,
            _               => return Err(err!(
                "Unknown argument '{}'; usage: lint_mutable --root <dir> [--baseline <file>] \
                [--write].", a; Invalid, Input)),
        }
    }
    let root = res!(root.ok_or_else(|| err!("Missing --root <dir>."; Missing, Input)));
    let base_path = base_path.unwrap_or_else(|| root.join(BASELINE_NAME));
    let base = res!(lint::load_baseline(&base_path));
    if write {
        let (scan, files) = res!(mutable::scan_tree(&root));
        let next = base.regenerate(&scan.hits);
        res!(lint::save_baseline(&next, &base_path));
        println!("Wrote {} entries ({} hits, {} files) to {}.",
            next.entries.len(), scan.hits.len(), files, base_path.display());
        return Ok(true);
    }
    let rep = res!(mutable::check_tree(&root, &base));
    println!("{}", rep);
    Ok(rep.is_clean())
}
