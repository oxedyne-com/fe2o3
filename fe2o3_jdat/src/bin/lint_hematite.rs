//! `lint_hematite --rules <file> [--repo <dir>] (--staged | --base <rev> [--head <rev>])`: the
//! Hematite gate on the lines a change adds, the staged change or `<base>...<head>`. Prints one
//! `<file>:<line>: <rule> -- <fix>` line per breach and one line per `allowlist hematite`, and
//! exits 1 on a breach, 2 when it cannot run. `lint_hematite --print-rules` prints the table built
//! in, for an installer to write outside the tree.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::lint;
use oxedyne_fe2o3_text::lint::hematite::{
    self,
    Added,
};

use std::{
    path::PathBuf,
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

fn git(repo: &PathBuf, args: &[&str]) -> Outcome<String> {
    let out = match Command::new("git").arg("-C").arg(repo).args(["-c", "core.quotePath=false"])
        .args(args).output()
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
    let mut diff_args = vec!["diff", "-U0", "--no-color", "--no-ext-diff", "--src-prefix=a/",
        "--dst-prefix=b/"];
    let range;
    let prefix = match (staged, &base) {
        (true, None)    => {
            diff_args.push("--cached");
            String::from(":")
        }
        (false, Some(b)) => {
            range = fmt!("{}...{}", b, head);
            diff_args.push(&range);
            fmt!("{}:", head)
        }
        _               => return Err(err!("Give exactly one of --staged and --base <rev>.";
            Invalid, Input)),
    };
    let diff = res!(git(&repo, &diff_args));
    let mut added = Vec::new();
    for (file, lines) in hematite::added_lines(&diff) {
        let src = res!(git(&repo, &["show", &fmt!("{}{}", prefix, file)]));
        added.push(Added { file, src, lines });
    }
    let rep = res!(hematite::check(&added, &rules));
    for b in &rep.breaches {
        println!("{}", b);
    }
    for a in &rep.allows {
        println!("{}", a);
    }
    Ok(rep.is_clean())
}
