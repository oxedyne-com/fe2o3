//! `lint_hematite --rules <file> [--repo <dir>] (--staged | --base <rev> [--head <rev>])`: the
//! Hematite gate on the lines a change adds, the staged change or `<base>...<head>`. Prints one
//! `<file>:<line>: <rule> -- <fix>` line per breach and one line per `allowlist hematite`, and
//! exits 1 on a breach, 2 when it cannot run. `lint_hematite --print-rules` prints the table built
//! in, for an installer to write outside the tree.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::lint;
use oxedyne_fe2o3_text::lint::hematite;

use std::{
    collections::BTreeSet,
    path::{
        Path,
        PathBuf,
    },
    process::Command,
};


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

fn git<S: AsRef<str>>(repo: &PathBuf, args: &[S]) -> Outcome<String> {
    let args: Vec<&str> = args.iter().map(|a| a.as_ref()).collect();
    let out = match Command::new("git").arg("-C").arg(repo).args(["-c", "core.quotePath=false"])
        .envs(hematite::GIT_ENV).args(&args).output()
    {
        Ok(o)   => o,
        Err(e)  => return Err(err!(e, "Could not run git {}.", args.join(" "); IO, Missing)),
    };
    if !out.status.success() {
        return Err(err!("git {} failed: {}", args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim(); IO, Invalid));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn run() -> Outcome<bool> {
    let mut rules: Option<PathBuf> = None;
    let mut repo = PathBuf::from(".");
    let mut staged = false;
    let mut base: Option<String> = None;
    let mut head = String::from("HEAD");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--print-rules" => {
                print!("{}", lint::HEMATITE_RULES);
                return Ok(true);
            }
            "--rules"   => rules = args.next().map(PathBuf::from),
            "--repo"    => repo = res!(args.next().map(PathBuf::from)
                .ok_or_else(|| err!("--repo needs a directory."; Missing, Input))),
            "--staged"  => staged = true,
            "--base"    => base = args.next(),
            "--head"    => head = res!(args.next()
                .ok_or_else(|| err!("--head needs a revision."; Missing, Input))),
            _           => return Err(err!(
                "Unknown argument '{}'; usage: lint_hematite --rules <file> [--repo <dir>] \
                (--staged | --base <rev> [--head <rev>]) | --print-rules.", a; Invalid, Input)),
        }
    }
    let rules = res!(lint::load_rules(&res!(rules
        .ok_or_else(|| err!("Missing --rules <file>."; Missing, Input)))));
    let range;
    let (revs, prefix, old_prefix, now_list, was_list) = match (staged, &base) {
        (true, None)        => (vec!["--cached"], String::from(":"), String::from("HEAD:"),
            res!(git(&repo, &["ls-files", "-s", "-z"])), res!(git(&repo, &["ls-tree", "-r", "-z", "HEAD"]))),
        (false, Some(b))    => {
            range = fmt!("{}...{}", b, head);
            let mb = res!(git(&repo, &["merge-base", b.as_str(), head.as_str()]));
            let mb = mb.trim();
            (vec![range.as_str()], fmt!("{}:", head), fmt!("{}:", mb),
                res!(git(&repo, &["ls-tree", "-r", "-z", &head])), res!(git(&repo, &["ls-tree", "-r", "-z", mb])))
        }
        _                   => return Err(err!("Give exactly one of --staged and --base <rev>.";
            Invalid, Input)),
    };
    let mut attrs = hematite::attributes_in(&now_list);
    let info = PathBuf::from(res!(git(&repo, &["rev-parse", "--path-format=absolute", "--git-path",
        hematite::INFO_ATTRIBUTES])).trim());
    if info.exists() {
        attrs.push(fmt!("{}", info.display()));
    }
    if !attrs.is_empty() {
        return Err(err!("The repository carries git attributes ({}), which decide what git reads \
            as a change; remove them so the Hematite gate can read every line.", attrs.join(", ");
            Invalid, Input));
    }
    let diff = res!(git(&repo, &hematite::diff_args(&revs)));
    let diffs = hematite::file_diffs(&diff);
    let changed: Vec<String> = diffs.keys().cloned().collect();
    let scratch = PathBuf::from(res!(git(&repo, &["rev-parse", "--path-format=absolute", "--git-path",
        &fmt!("hematite-cargo-{}", std::process::id())])).trim());
    let now = res!(cargo_of(&repo, &scratch.join("now"), &now_list, &changed, &prefix));
    let was = res!(cargo_of(&repo, &scratch.join("was"), &was_list, &changed, &old_prefix));
    let _ = std::fs::remove_dir_all(&scratch);
    // A file new to the tree is read whole, so only one the base holds can be turned.
    let was_files: BTreeSet<&str> = hematite::tree_paths(&was_list).into_iter().map(|(_, p)| p).collect();
    let files: Vec<String> = hematite::tree_paths(&now_list).into_iter()
        .filter(|(_, p)| p.ends_with(".rs") && was_files.contains(p)).map(|(_, p)| p.to_string()).collect();
    let turned = now.turned(&was, &files, &rules.test_attrs);
    let mut added = Vec::new();
    for (file, d) in diffs {
        let src = res!(git(&repo, &["cat-file", "blob", &fmt!("{}{}", prefix, file)]));
        let old = match d.old {
            true    => Some(res!(git(&repo, &["cat-file", "blob", &fmt!("{}{}", old_prefix, file)]))),
            false   => None,
        };
        added.push(res!(hematite::added(file, d, src, old.as_deref(), &rules, &now, &was)));
    }
    let rep = res!(hematite::check(&added, &turned, &rules));
    for b in &rep.breaches {
        println!("{}", b);
    }
    for a in &rep.allows {
        println!("{}", a);
    }
    Ok(rep.is_clean())
}

/// The packages of the tree `listing`, its files read as `<prefix><path>`, worked out in `dir`.
fn cargo_of(repo: &PathBuf, dir: &Path, listing: &str, changed: &[String], prefix: &str)
    -> Outcome<hematite::Cargo>
{
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir).map_err(|e| err!(e, "While making {}.", dir.display(); IO, File, Write)));
    let got = lint::cargo_of(dir, listing, changed,
        |p| git(repo, &["cat-file", "blob", &fmt!("{}{}", prefix, p)]),
        |m, full| {
            let mut c = Command::new("cargo"); // fixed: the program's name, found on PATH
            c.env_clear();
            for k in ["PATH", "HOME", "CARGO_HOME", "RUSTUP_HOME"] {
                if let Some(v) = std::env::var_os(k) {
                    c.env(k, v);
                }
            }
            c.env("CARGO_NET_OFFLINE", "true").current_dir(dir)
                .args(["metadata", "--format-version", "1", "--offline"]); // fixed: Cargo's metadata format
            if !full {
                c.arg("--no-deps");
            }
            let out = res!(c.arg("--manifest-path").arg(m).output()
                .map_err(|e| err!(e, "Could not run cargo metadata."; IO, Missing)));
            match out.status.success() {
                true    => Ok(String::from_utf8_lossy(&out.stdout).to_string()),
                false   => Err(err!("cargo metadata on {} failed: {}", m.display(),
                    String::from_utf8_lossy(&out.stderr).trim(); IO, Invalid)),
            }
        });
    let _ = std::fs::remove_dir_all(dir);
    got
}
