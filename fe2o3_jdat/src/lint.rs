//! The `.jdat` files of the source gates in `fe2o3_text::lint`: the mutable-literal gate's ratchet
//! baseline, and the Hematite gate's rule table. The gates live in fe2o3_text, which this crate
//! depends on, so the files are read and written here.
//!
//! ```text
//! {
//!     "entries": [ { "file": "fe2o3_net/src/llm.rs", "class": "host", "text": "…", "count": 1 } ],
//!     "allow": [ { "file": "…", "class": "named", "text": "…", "why": "Generated tables." } ],
//! }
//! ```

use crate::{
    bdat::DecodeLimits,
    prelude::*,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::{
    lint::{
        hematite::{
            self,
            Cargo,
            Check,
            Package,
            Rule,
            Rules,
            Target,
            TargetKind,
        },
        mutable::{
            self,
            Allow,
            Baseline,
            Class,
            Key,
            Report,
        },
    },
};

use std::{
    collections::{
        BTreeMap,
        BTreeSet,
    },
    ffi::OsString,
    fs,
    path::{
        Component,
        Path,
        PathBuf,
    },
};


/// Reads a baseline file. An absent file is an empty baseline, so a new tree starts at zero debt.
pub fn load_baseline(path: &Path) -> Outcome<Baseline> {
    if !path.exists() {
        return Ok(Baseline::default());
    }
    let s = match fs::read_to_string(path) {
        Ok(s)   => s,
        Err(e)  => return Err(err!(e, "While reading baseline {}.", path.display(); IO, File, Read)),
    };
    let dat = res!(Dat::decode_string(s));
    baseline_from_dat(&dat)
}

/// Writes a baseline file, sorted by key so a migration's diff is only its own lines.
pub fn save_baseline(base: &Baseline, path: &Path) -> Outcome<()> {
    let mut text = res!(baseline_to_dat(base).jdat_to_lines("\t"));
    text.push('\n');
    match fs::write(path, text) {
        Ok(())  => Ok(()),
        Err(e)  => Err(err!(e, "While writing baseline {}.", path.display(); IO, File, Write)),
    }
}

/// Scans the tree under `root` and checks it against the baseline file at `baseline_path`.
pub fn check_tree(root: &Path, baseline_path: &Path) -> Outcome<Report> {
    let base = res!(load_baseline(baseline_path));
    mutable::check_tree(root, &base, &res!(parse_rules(HEMATITE_RULES)).test_attrs)
}

pub fn baseline_to_dat(base: &Baseline) -> Dat {
    let entries: Vec<Dat> = base.entries.iter().map(|(k, n)| mapdat!{
        "file"  => k.file.clone(),
        "class" => k.class.name(),
        "text"  => k.text.clone(),
        "count" => *n as u64,
    }).collect();
    let allow: Vec<Dat> = base.allow.iter().map(|a| mapdat!{
        "file"  => a.file.clone(),
        "class" => a.class.name(),
        "text"  => a.text.clone(),
        "why"   => a.why.clone(),
    }).collect();
    mapdat!{
        "entries"   => Dat::List(entries),
        "allow"     => Dat::List(allow),
    }
}

pub fn baseline_from_dat(dat: &Dat) -> Outcome<Baseline> {
    let mut base = Baseline::default();
    for (i, e) in res!(dat.map_get_list(&dat!("entries"))).iter().enumerate() {
        let (key, n) = match key_from_dat(e).and_then(|k| e.map_get_u64(&dat!("count")).map(|n| (k, n))) {
            Ok(v)   => v,
            Err(e)  => return Err(err!(e, "In baseline entry {}.", i; Invalid, Input)),
        };
        *base.entries.entry(key).or_insert(0) += n as usize;
    }
    if let Some(list) = res!(dat.map_get(&dat!("allow"))) {
        let list = match list {
            Dat::List(v)    => v,
            _               => return Err(err!(
                "Baseline 'allow' is a {:?}, expected a list.", list.kind(); Invalid, Input)),
        };
        for (i, a) in list.iter().enumerate() {
            let key = match key_from_dat(a) {
                Ok(k)   => k,
                Err(e)  => return Err(err!(e, "In baseline allow entry {}.", i; Invalid, Input)),
            };
            // A missing why loads as empty, so the gate reports it rather than the load failing.
            let why = match res!(a.map_get(&dat!("why"))) {
                Some(Dat::Str(s))   => s.clone(),
                _                   => String::new(),
            };
            base.allow.push(Allow { file: key.file, class: key.class, text: key.text, why });
        }
    }
    Ok(base)
}

fn key_from_dat(dat: &Dat) -> Outcome<Key> {
    Ok(Key {
        file:   res!(dat.map_get_string(&dat!("file"))),
        class:  res!(Class::from_name(&res!(dat.map_get_string(&dat!("class"))))),
        text:   res!(dat.map_get_string(&dat!("text"))),
    })
}

// The release's rule table, so an installer can write it where a daimon cannot reach.
pub const HEMATITE_RULES: &str = include_str!("../hematite_rules.jdat");

/// Reads a Hematite rule table. Unlike a baseline, an absent table is an error: a gate with no
/// rules would pass everything.
///
/// ```text
/// {
///     "local_crates": ["oxedyne_"],
///     "test_attrs": ["test", "tokio::test"],
///     "rules": [ { "id": "G1", "check": "seq", "seq": [".", "unwrap", "(", ")"], "on": true,
///                  "in_tests": true, "fixed_ok": false, "msg": "unwrap() -- use res!(x)" } ],
/// }
/// ```
pub fn load_rules(path: &Path) -> Outcome<Rules> {
    let s = match fs::read_to_string(path) {
        Ok(s)   => s,
        Err(e)  => return Err(err!(e, "While reading the Hematite rule table {}.", path.display();
            IO, File, Read)),
    };
    match parse_rules(&s) {
        Ok(r)   => Ok(r),
        Err(e)  => Err(err!(e, "In the Hematite rule table {}.", path.display(); Invalid, Input)),
    }
}

pub fn parse_rules(text: &str) -> Outcome<Rules> {
    let dat = res!(Dat::decode_string(text));
    rules_from_dat(&dat)
}

pub fn rules_from_dat(dat: &Dat) -> Outcome<Rules> {
    let mut rules = Rules::default();
    for c in res!(dat.map_get_list(&dat!("local_crates"))) {
        match c {
            Dat::Str(s) => rules.local_crates.push(s.clone()),
            _           => return Err(err!("A 'local_crates' entry is a {:?}, expected a string.",
                c.kind(); Invalid, Input)),
        }
    }
    for c in res!(dat.map_get_list(&dat!("test_attrs"))) {
        match c {
            Dat::Str(s) => rules.test_attrs.push(s.clone()),
            _           => return Err(err!("A 'test_attrs' entry is a {:?}, expected a string.",
                c.kind(); Invalid, Input)),
        }
    }
    for (i, r) in res!(dat.map_get_list(&dat!("rules"))).iter().enumerate() {
        match rule_from_dat(r) {
            Ok(rule)    => rules.rules.push(rule),
            Err(e)      => return Err(err!(e, "In rule {}.", i; Invalid, Input)),
        }
    }
    Ok(rules)
}

fn rule_from_dat(dat: &Dat) -> Outcome<Rule> {
    let id = res!(dat.map_get_string(&dat!("id")));
    let name = res!(dat.map_get_string(&dat!("check")));
    let mut list = Vec::new();
    for key in ["seq", "words"] {
        if let Some(v) = res!(dat.map_get(&dat!(key))) {
            let v = match v {
                Dat::List(v)    => v,
                _               => return Err(err!("Rule '{}': '{}' is a {:?}, expected a list.",
                    id, key, v.kind(); Invalid, Input)),
            };
            for w in v {
                match w {
                    Dat::Str(s) => list.push(s.clone()),
                    _           => return Err(err!("Rule '{}': a '{}' entry is a {:?}, expected \
                        a string.", id, key, w.kind(); Invalid, Input)),
                }
            }
        }
    }
    let msg = res!(dat.map_get_string(&dat!("msg")));
    if msg.trim().is_empty() {
        return Err(err!("Rule '{}' has an empty 'msg'.", id; Invalid, Input, Missing));
    }
    Ok(Rule {
        on:         res!(dat.map_get_bool(&dat!("on"))),
        in_tests:   res!(dat.map_get_bool(&dat!("in_tests"))),
        fixed_ok:   res!(dat.map_get_bool(&dat!("fixed_ok"))),
        check:      res!(Check::of(&name, list)),
        msg,
        id,
    })
}

// cargo metadata environment
pub const CARGO_ENV: [&str; 4] = ["PATH", "HOME", "CARGO_HOME", "RUSTUP_HOME"]; // fixed: what Cargo needs to find itself and its registry cache
pub const CARGO_SET: [(&str, &str); 1] = [("CARGO_NET_OFFLINE", "true")]; // fixed: never fetch

/// The arguments of the `cargo metadata` call [`cargo_of`] asks for.
pub fn metadata_args(manifest: &Path, full: bool) -> Vec<OsString> {
    let mut a: Vec<OsString> = ["metadata", "--format-version", "1", "--offline"] // fixed: Cargo's metadata format
        .iter().map(OsString::from).collect();
    if !full {
        a.push(OsString::from("--no-deps"));
    }
    a.push(OsString::from("--manifest-path"));
    a.push(manifest.as_os_str().to_os_string());
    a
}

const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index"; // fixed: Cargo's id of the crates.io registry

/// The packages Cargo builds from a tree whose root is `real` on this machine, worked out in `dir`,
/// an empty directory of mode 0700 that the caller then removes, under no Cargo manifest or Cargo
/// config but `$CARGO_HOME`'s and no directory another account can write to (a group-writable
/// directory passes only when the group holds the caller alone). The tree is written at
/// its own path under `dir`, beside links to its ancestors' other entries, so that a path
/// dependency leaving the tree reaches what Cargo reaches from `real`. `listing` is the tree as `git ls-tree -r -z` or `git ls-files -s -z` gives it and
/// `read(path)` a file's content. Each `Cargo.toml` and `Cargo.lock` is written with its content
/// and every other `.rs` file empty, for Cargo to find the targets; nothing else, so no `.cargo`
/// config or toolchain file of the tree is read. `metadata(manifest, full)` runs `cargo metadata
/// --format-version 1 --offline --manifest-path <manifest>`, with `--no-deps` when not `full`,
/// and returns what it prints. The manifest each file of `changed` lies under most closely, and a
/// changed manifest, must be read; any other that Cargo cannot read is passed over, since Cargo
/// cannot build it either.
pub fn cargo_of(
    dir:            &Path,
    real:           &Path,
    listing:        &str,
    changed:        &[String],
    mut read:       impl FnMut(&str) -> Outcome<String>,
    mut metadata:   impl FnMut(&Path, bool) -> Outcome<String>,
)
    -> Outcome<Cargo>
{
    let base = res!(fs::canonicalize(dir).map_err(|e| err!(e, "While finding the Cargo scratch directory {}.", dir.display(); IO, File)));
    // Cargo looks above a manifest for the workspace that claims it.
    if let Some(a) = base.ancestors().find(|a| a.join("Cargo.toml").exists()) { // fixed: Cargo's manifest name
        return Err(err!("The Cargo scratch directory {} lies under the Cargo manifest of {}, which \
            Cargo would read as the tree's workspace.", base.display(), a.display(); Invalid, Input));
    }
    // Cargo reads config from the ancestors of its working directory, `dir`, and a config can name
    // a program (`rustc-wrapper`) that `cargo metadata` runs.
    let home = std::env::var_os("CARGO_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo"))) // fixed: Cargo's default home
        .and_then(|c| fs::canonicalize(c).ok());
    for a in base.ancestors() {
        let at = a.join(".cargo"); // fixed: Cargo's config directory
        let config = ["config", "config.toml"].iter().any(|c| fs::symlink_metadata(at.join(c)).is_ok()); // fixed: Cargo's config file names
        if config && fs::canonicalize(&at).ok() != home {
            return Err(err!("The Cargo scratch directory {} lies under the Cargo config of {}, which \
                Cargo would read and which can name a program for it to run.", base.display(),
                at.display(); Invalid, Input));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let stat = |a: &Path| fs::metadata(a).map_err(|e| err!(e, "While reading {}.", a.display(); IO, File));
        let m = res!(stat(&base));
        let me = m.uid();
        if m.mode() & 0o077 != 0 { // fixed: the group and other bits
            return Err(err!("The Cargo scratch directory {} can be opened by other accounts; the \
                tree's manifests are written there, so make it 0700.", base.display(); Invalid, Input));
        }
        for a in base.ancestors() {
            let m = res!(stat(a));
            // Another account able to write here could plant a Cargo config or manifest.
            let group = m.mode() & 0o020 != 0 && !group_only(me, m.gid()); // fixed: the group write bit
            if (m.uid() != me && m.uid() != 0) || m.mode() & 0o002 != 0 || group { // fixed: root's uid, the other write bit
                return Err(err!("The Cargo scratch directory {} lies under {}, which another \
                    account can write to and so plant a Cargo config. Use a private directory, \
                    such as one under $XDG_RUNTIME_DIR.", base.display(), a.display(); Invalid, Input));
            }
        }
    }
    let dir = res!(mirror(&base, real));
    let mut manifests = Vec::new();
    for (mode, path) in hematite::tree_paths(listing) {
        let name = path.rsplit('/').next().unwrap_or(path);
        let manifest = name == "Cargo.toml"; // fixed: Cargo's manifest name
        let body = match (manifest || name == "Cargo.lock", mode) { // fixed: Cargo's lock file name
            (true, "120000")                                    => return Err(err!( // fixed: git's mode of a symbolic link
                "The Cargo file {} is a symlink, which the Hematite gate does not follow.", path;
                Invalid, Input)),
            (true, m) if m.starts_with("100")                   => res!(read(path)), // fixed: git's modes of a file
            (false, m) if m.starts_with("100") && name.ends_with(".rs") => String::new(),
            _                                                   => continue,
        };
        let at = dir.join(path);
        if let Some(d) = at.parent() {
            res!(fs::create_dir_all(d).map_err(|e| err!(e, "While making {}.", d.display(); IO, File, Write)));
        }
        res!(fs::write(&at, body).map_err(|e| err!(e, "While writing {}.", at.display(); IO, File, Write)));
        if manifest {
            manifests.push(path.to_string());
        }
    }
    let root_of = |m: &str| m.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
    let mut need = BTreeSet::new();
    for c in changed {
        let near = manifests.iter().filter(|m| {
            let r = root_of(m);
            *m == c || r.is_empty() || c.strip_prefix(r.as_str()).map(|t| t.starts_with('/')).unwrap_or(false)
        }).max_by_key(|m| if *m == c { usize::MAX } else { m.len() });
        if let Some(m) = near {
            need.insert(m.clone());
        }
    }
    // Roots first, so that one call reads a whole workspace.
    manifests.sort_by_key(|m| (m.matches('/').count(), m.clone()));
    let mut cargo = Cargo::default();
    for m in &manifests {
        if cargo.packages.iter().any(|p| p.root == root_of(m)) {
            continue;
        }
        let at = dir.join(m);
        let mut got = metadata(&at, true).and_then(|o| packages_of(&o, &dir, true));
        if got.is_err() {
            got = metadata(&at, false).and_then(|o| packages_of(&o, &dir, false));
        }
        match got {
            Ok(ps)                      => cargo.packages.extend(ps.into_iter()
                .filter(|p| !cargo.packages.iter().any(|q| q.root == p.root)).collect::<Vec<_>>()),
            Err(e) if need.contains(m)  => {
                // Cargo names the first dependency it could not load and the manifest it looked
                // for, under the scratch twin of this machine's root.
                let text = fmt!("{}", e);
                let quoted = |after: &str| text.rfind(after).and_then(|i| {
                    let t = &text[i + after.len()..];
                    t.find('`').map(|j| t[..j].to_string())
                });
                return Err(match (quoted("for dependency `"), quoted("failed to read `")) {
                    (Some(name), Some(read))    => {
                        let at = read.strip_suffix("/Cargo.toml").unwrap_or(&read); // fixed: Cargo's manifest name
                        let at = match Path::new(at).strip_prefix(&base) {
                            Ok(r)   => fmt!("/{}", r.display()),
                            Err(_)  => at.to_string(),
                        };
                        err!(e, "Cargo could not read the manifest {}: its dependency {} is not at \
                            {} on this machine. Check that dependency out at {} so the Hematite \
                            gate can tell test code from product.", m, name, at, at; Invalid, Input)
                    }
                    _                           => err!(e, "Cargo could not read the manifest {}, \
                        so the Hematite gate cannot tell test code from product.", m; Invalid, Input),
                });
            }
            Err(_)                      => {}
        }
    }
    Ok(cargo)
}

/// Is `uid` the only account in group `gid`, as `/etc/passwd` and `/etc/group` list them (a user
/// private group, whose write bit lets no one else in)? An account database they do not hold, such
/// as a directory service, reads as no.
#[cfg(unix)]
fn group_only(uid: u32, gid: u32) -> bool {
    let fields = |p: &str| fs::read_to_string(p).unwrap_or_default().lines()
        .map(|l| l.split(':').map(|f| f.to_string()).collect::<Vec<_>>())
        .filter(|f| f.len() >= 4).collect::<Vec<_>>(); // fixed: name, password, id, then gid or members
    let users = fields("/etc/passwd"); // fixed: the system's account database
    let name = match users.iter().find(|f| f[2].parse::<u32>().ok() == Some(uid)) {
        Some(f) => f[0].clone(),
        None    => return false,
    };
    if users.iter().any(|f| f[0] != name && f[3].parse::<u32>().ok() == Some(gid)) {
        return false;
    }
    let groups: Vec<_> = fields("/etc/group").into_iter() // fixed: the system's group database
        .filter(|f| f[2].parse::<u32>().ok() == Some(gid)).collect();
    !groups.is_empty() && groups.iter().all(|f| f[3].split(',').all(|m| m.is_empty() || m == name))
}

/// The directory under `dir` at which to write a tree whose root is `real`: each ancestor of `real`
/// is a directory under `dir` holding links to the ancestor's other entries.
fn mirror(dir: &Path, real: &Path) -> Outcome<PathBuf> {
    let real = res!(fs::canonicalize(real).map_err(|e| err!(e, "While finding the tree's root {}.", real.display(); IO, File)));
    let mut at = PathBuf::from("/"); // fixed: the root of a canonical path
    let mut twin = dir.to_path_buf();
    for c in real.components() {
        let next = match c {
            Component::Normal(n)    => n,
            _                       => continue,
        };
        // An ancestor the user cannot list holds nothing Cargo could reach either.
        if let Ok(rd) = fs::read_dir(&at) {
            for e in rd.flatten() {
                if e.file_name() != next {
                    res!(link(&e.path(), &twin.join(e.file_name())));
                }
            }
        }
        at.push(next);
        twin.push(next);
        res!(fs::create_dir(&twin).map_err(|e| err!(e, "While making {}.", twin.display(); IO, File, Write)));
    }
    Ok(twin)
}

#[cfg(unix)]
fn link(to: &Path, at: &Path) -> Outcome<()> {
    std::os::unix::fs::symlink(to, at).map_err(|e| err!(e, "While linking {} to {}.", at.display(), to.display(); IO, File, Write))
}

#[cfg(not(unix))]
fn link(to: &Path, at: &Path) -> Outcome<()> {
    Err(err!("The Hematite gate links {} to {} for Cargo, which needs a Unix file system.",
        at.display(), to.display(); Unimplemented))
}

/// The workspace members of `cargo metadata` output, their paths relative to `dir`, with what
/// each extern name resolves to when `full`.
fn packages_of(out: &str, dir: &Path, full: bool) -> Outcome<Vec<Package>> {
    let dat = res!(Dat::decode_json_strict(out, &DecodeLimits::default()));
    let rel = |abs: String| -> Outcome<String> {
        match Path::new(&abs).strip_prefix(dir).map(|p| p.to_str().map(|s| s.to_string())) {
            Ok(Some(r)) => Ok(r),
            _           => Err(err!("Cargo names {}, outside the tree.", abs; Invalid, Input)),
        }
    };
    let str_of = |d: &Dat, k: &str| match d.map_get(&dat!(k)) {
        Ok(Some(Dat::Str(s)))   => Some(s.clone()),
        _                       => None,
    };
    let members: BTreeSet<String> = res!(dat.map_get_list(&dat!("workspace_members"))).iter()
        .filter_map(|m| match m { Dat::Str(s) => Some(s.clone()), _ => None }).collect();
    let all = res!(dat.map_get_list(&dat!("packages")));
    // By package id: is it the crates.io crate of the extern name `_` for `-`?
    let ids: BTreeMap<String, (String, bool)> = all.iter()
        .filter_map(|p| match (str_of(p, "id"), str_of(p, "name")) {
            (Some(id), Some(n)) => Some((id, (n.replace('-', "_"),
                str_of(p, "source").as_deref() == Some(CRATES_IO)))),
            _                   => None,
        })
        .collect();
    let nodes = match full {
        true    => res!(res!(dat.map_get_map(&dat!("resolve"))).map_get_list(&dat!("nodes"))).clone(),
        false   => Vec::new(),
    };
    let mut out = Vec::new();
    for p in all {
        let id = res!(p.map_get_string(&dat!("id")));
        if !members.contains(&id) {
            continue;
        }
        let manifest = res!(rel(res!(p.map_get_string(&dat!("manifest_path")))));
        let root = manifest.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
        let mut targets = Vec::new();
        for t in res!(p.map_get_list(&dat!("targets"))) {
            let kinds: Vec<String> = res!(t.map_get_list(&dat!("kind"))).iter()
                .filter_map(|k| match k { Dat::Str(s) => Some(s.clone()), _ => None }).collect();
            let kind = match kinds.first().map(|k| k.as_str()) {
                Some("test")            => TargetKind::Test,
                Some("custom-build")    => TargetKind::Build,
                _                       => TargetKind::Product,
            };
            targets.push(Target { kind, src: res!(rel(res!(t.map_get_string(&dat!("src_path"))))) });
        }
        let crates = match full {
            true    => {
                let node = res!(nodes.iter().find(|n| str_of(n, "id").as_deref() == Some(id.as_str()))
                    .ok_or_else(|| err!("Cargo resolved no node for {}.", id; Missing, Input)));
                let mut m: BTreeMap<String, bool> = BTreeMap::new();
                for d in res!(node.map_get_list(&dat!("deps"))) {
                    let name = res!(d.map_get_string(&dat!("name")));
                    let ok = str_of(d, "pkg").and_then(|k| ids.get(&k))
                        .map(|(n, crates_io)| *crates_io && *n == name).unwrap_or(false);
                    // One name taken by two packages, as a normal and a dev dependency, is
                    // crates.io's only if both are.
                    *m.entry(name).or_insert(true) &= ok;
                }
                Some(m)
            }
            false   => None,
        };
        out.push(Package { root, targets, crates });
    }
    Ok(out)
}
