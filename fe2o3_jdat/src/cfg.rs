//! What a configuration is, and how a value in one is allowed to point
//! somewhere else.
//!
//! [`Config`] is the trait a configuration type implements. [`resolve`] and
//! [`resolve_dat`] are the indirection every Oxedyne configuration file has: a
//! value that must not sit in the file itself is written `{file:secret}` or
//! `{env:VAR}` and is fetched when the file is read. [`resolve_root`] is the
//! app-root rule every app shares, and [`path_at`] places one location value
//! under the root it chose.

use crate::{
    prelude::*,
    file::JdatMapFile,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    path::NormalPath,
};

use std::path::{
    Path,
    PathBuf,
};


pub trait Config:
    Clone
    + std::fmt::Debug
    + Default
    + Eq
    + PartialEq
    + FromDatMap
    + ToDatMap
{
    // Required.
    fn check_and_fix(&mut self) -> Outcome<()> {
        Err(err!(
            "Don't forget to implement checks on the configuration.";
        Unimplemented, Configuration))
    }

    // Provided.
    fn dump(self) -> Outcome<()> {
        let dat = Self::to_datmap(self);
        for line in dat.to_lines("    ", true) {
            info!("{}", line);
        }
        Ok(())
    }

}

impl<T: Config> JdatMapFile for T {}


/// Expand every reference in a configuration value.
///
/// * `{env:VAR}` is the variable's value, refused when it is unset or empty.
/// * `{env:VAR:default}` is the value, or `default` when unset or empty.
/// * `{file:path}` is the file's trimmed contents, read from `root.join(path)`, so an absolute
///   path replaces the root.
/// * `{file?:path}` is the same, or the empty string when the file does not exist. Any other
///   read failure is still refused, so a present but unreadable key is never silently dropped.
///
/// The environment is expanded first, so a variable may name a file. A variable's value and a
/// file's contents are both inserted verbatim and never scanned for references themselves: a
/// `{file:` or `}` inside a variable's value is text, so it neither starts nor ends a reference.
pub fn resolve(value: &str, root: &Path)
    -> Outcome<String>
{
    let named = res!(expand_env(value));
    resolve_files(&named, value, root)
}

/// Expand only the `{env:VAR[:default]}` references, reading no file.
pub fn resolve_env(whole: &str)
    -> Outcome<String>
{
    Ok(res!(expand_env(whole)).text)
}

/// A value with its environment expanded, and the byte ranges in it each variable's value
/// fills, so the file scan can tell the value's own text from what the environment put there.
struct EnvExpanded {
    text:   String,
    filled: Vec<std::ops::Range<usize>>,
}

impl EnvExpanded {
    /// Did the environment put the byte at `i` there?
    fn filled_at(&self, i: usize) -> bool {
        self.filled.iter().any(|r| r.contains(&i))
    }
}

// One pass, forward: each value is inserted verbatim and the scan resumes after it, never
// through it. A rescan from the start hung on a variable naming itself (QA gwmain P3-3).
fn expand_env(whole: &str)
    -> Outcome<EnvExpanded>
{
    let mut out = EnvExpanded { text: String::with_capacity(whole.len()), filled: Vec::new() };
    let mut rest = whole;
    while let Some(start) = rest.find("{env:") {
        let end = match rest[start..].find('}') {
            Some(i) => start + i,
            None => return Err(err!(
                "The configuration value {:?} opens an {{env: reference and does not \
                close it.", whole;
            Invalid, Input)),
        };
        let inner = &rest[start + 5..end];
        let (name, fallback) = match inner.find(':') {
            Some(i) => (&inner[..i], Some(&inner[i + 1..])),
            None    => (inner, None),
        };
        let got = match std::env::var(name) {
            Ok(v) if !v.is_empty() => v,
            _ => match fallback {
                Some(d) => fmt!("{}", d),
                None => return Err(err!(
                    "The configuration names the environment variable {:?}, which is \
                    not set, and gives no default.", name;
                Invalid, Input, Missing)),
            },
        };
        out.text.push_str(&rest[..start]);
        let at = out.text.len();
        out.text.push_str(&got);
        out.filled.push(at..out.text.len());
        rest = &rest[end + 1..];
    }
    out.text.push_str(rest);
    Ok(out)
}

/// Every file [`resolve`] would read for `value`, in order, after the environment is expanded
/// and each path joined to `root` as `resolve` joins it, without reading any. A caller that vets
/// where secrets come from can refuse a path before anything is read.
pub fn file_refs(value: &str, root: &Path)
    -> Outcome<Vec<PathBuf>>
{
    let named = res!(expand_env(value));
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(fref) = res!(first_file_ref(&named, pos, value)) {
        out.push(root.join(&named.text[fref.start + fref.marker..fref.end]));
        pos = fref.end + 1;
    }
    Ok(out)
}

/// The config file an app was asked to load, and the arguments left once that request is taken out.
/// `--config <path>` or `--config=<path>` names it; otherwise it is `default_name` in the working
/// directory.
pub fn config_arg(args: Vec<String>, default_name: &str)
    -> Outcome<(PathBuf, Vec<String>)>
{
    let mut path = None;
    let mut rest = Vec::with_capacity(args.len());
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        let given = if arg == "--config" {
            Some(res!(iter.next().ok_or_else(|| err!(
                "--config needs a path after it."; Missing, Input))))
        } else if let Some(p) = arg.strip_prefix("--config=") {
            Some(p.to_string())
        } else {
            rest.push(arg);
            None
        };
        if let Some(p) = given {
            if p.is_empty() {
                return Err(err!("--config was given an empty path."; Invalid, Input));
            }
            if path.is_some() {
                return Err(err!("--config was given twice."; Invalid, Input));
            }
            path = Some(PathBuf::from(p));
        }
    }
    let path = match path {
        Some(p) => p,
        None    => Path::new("./").join(default_name),
    };
    Ok((path, rest))
}

/// Turn an app-root spec from a configuration into a path.
///
/// * `current`, `here`, `auto` or `.` is the working directory.
/// * `env` is the value of the variable `env_var`, refused when unset.
/// * `~/path` is under `HOME`; `./path` is under the working directory.
/// * Anything else is taken as written.
pub fn resolve_root(spec: &str, env_var: &str)
    -> Outcome<PathBuf>
{
    Ok(match spec {
        "current" | "here" | "auto" | "." => res!(std::env::current_dir()),
        "env" => match std::env::var(env_var) {
            Ok(path) => PathBuf::from(path),
            Err(_) => return Err(err!(
                "The app root is 'env' and the environment variable {:?} is not set.", env_var;
            Configuration, Missing, Invalid)),
        },
        path if path.starts_with("~/") => {
            let home = std::env::var_os("HOME").map(PathBuf::from); // fixed: the POSIX home variable
            res!(home_for(path, home.as_deref())).join(&path[2..])
        },
        path if path.starts_with("./") => res!(std::env::current_dir()).join(&path[2..]),
        path => PathBuf::from(path),
    })
}

/// The home a `~/` value is placed under, as `resolve_root` and `path_at` share it.
/// `resolve_root` gives `HOME`; a caller of `path_at` may give the passwd home instead.
fn home_for<'a>(value: &str, home: Option<&'a Path>)
    -> Outcome<&'a Path>
{
    match home {
        Some(home) => Ok(home),
        None       => Err(err!(
            "The path {} starts with '~/' and there is no home directory to put it under.",
            value;
        Configuration, Missing, Invalid)),
    }
}

/// Place one location value from a configuration under `root`, a root already chosen (by
/// [`resolve_root`], say).
///
/// * An absolute value is used as written, normalised.
/// * `~/path` is under `home`, absolute, and may not escape it.
/// * Anything else is `root.join(value)`, normalised, and may not escape `root`.
///
/// An empty value, a NUL byte, an unresolved `{env:` or `{file:` reference, and a relative
/// `root` are refused.
pub fn path_at(value: &str, root: &Path, home: Option<&Path>)
    -> Outcome<PathBuf>
{
    if value.is_empty() {
        return Err(err!("A configured path is empty."; Configuration, Missing, Path));
    }
    if value.contains('\0') {
        return Err(err!(
            "The configured path {:?} holds a NUL byte.", value.replace('\0', "\\0");
        Configuration, Invalid, Path));
    }
    for marker in ["{env:", "{file:", "{file?:"] {
        if value.contains(marker) {
            return Err(err!(
                "The configured path {} holds an unresolved {}...}} reference; resolve the \
                configuration before placing its paths.", value, marker;
            Configuration, Invalid, Path));
        }
    }
    if Path::new(value).is_absolute() {
        // NormalPath turns /a/b into ./a/b, so the result is re-rooted.
        return Ok(Path::new(value).normalise().absolute().as_pathbuf());
    }
    let (base, rel) = match value.strip_prefix("~/") {
        Some(rel) => (res!(home_for(value, home)), rel),
        None      => (root, value),
    };
    if !base.is_absolute() {
        return Err(err!(
            "The configured path {} is placed under {:?}, which is not absolute.", value, base;
        Configuration, Invalid, Path));
    }
    let tail = Path::new(rel).normalise();
    if tail.escapes() {
        return Err(err!(
            "The configured path {} escapes {:?}, the directory it must stay under.", value, base;
        Configuration, Invalid, Path));
    }
    Ok(base.normalise().join(tail).normalise().absolute().as_pathbuf())
}

/// Where a `{file:` or `{file?:` reference sits in a string.
struct FileRef {
    start:  usize,  // the opening brace
    end:    usize,  // the closing brace
    marker: usize,  // 6 for `{file:`, 7 for `{file?:`
}

impl FileRef {
    /// Is this the `{file?:` form?
    fn optional(&self) -> bool { self.marker == 7 }
}

/// The earliest file reference in `named` at or after byte `from`, its positions absolute. Only
/// the value's own text opens or closes one, never what the environment filled in. `whole` is
/// the original value, named in an error.
fn first_file_ref(named: &EnvExpanded, from: usize, whole: &str)
    -> Outcome<Option<FileRef>>
{
    let s = &named.text;
    // '{file:' is not a substring of '{file?:', so neither marker can match the other.
    let opener = |at: usize| -> Option<usize> {
        if named.filled_at(at) {
            None
        } else if s[at..].starts_with("{file:") {
            Some(6)
        } else if s[at..].starts_with("{file?:") {
            Some(7)
        } else {
            None
        }
    };
    let (start, marker) = match s[from..].match_indices('{')
        .find_map(|(i, _)| opener(from + i).map(|m| (from + i, m)))
    {
        Some(found) => found,
        None        => return Ok(None),
    };
    let close = s[start + marker..].match_indices('}')
        .map(|(i, _)| start + marker + i)
        .find(|&i| !named.filled_at(i));
    match close {
        Some(end) => Ok(Some(FileRef { start, end, marker })),
        None    => Err(err!(
            "The configuration value {:?} opens a {{file{}: reference and does not close it.",
            whole, if marker == 7 { "?" } else { "" };
        Invalid, Input)),
    }
}

fn resolve_files(named: &EnvExpanded, whole: &str, root: &Path)
    -> Outcome<String>
{
    // A file's contents are inserted verbatim and the scan continues after them, never through
    // them: a secret holding `{file:...}` would otherwise loop, read a file no path check saw, or
    // put part of itself in an error.
    let text = &named.text;
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    while let Some(fref) = res!(first_file_ref(named, pos, whole)) {
        let rel = &text[fref.start + fref.marker..fref.end];
        let path = root.join(rel);
        let mark = if fref.optional() { "?" } else { "" };
        out.push_str(&text[pos..fref.start]);
        match std::fs::read_to_string(&path) {
            Ok(t) => out.push_str(t.trim()),
            // Only not-found is tolerated, so a present file that cannot be read still fails.
            Err(e) if fref.optional()
                && e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(err!(e,
                "The configuration reads {{file{}:{}}}, and {:?} could not be read.",
                mark, rel, path;
            IO, File, Read)),
        }
        pos = fref.end + 1;
    }
    out.push_str(&text[pos..]);
    Ok(out)
}

pub fn resolve_dat(dat: &Dat, root: &Path)
    -> Outcome<Dat>
{
    Ok(match dat {
        Dat::Str(s) => Dat::Str(res!(resolve(s, root))),
        Dat::List(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(res!(resolve_dat(item, root)));
            }
            Dat::List(out)
        },
        Dat::Map(map) => {
            let mut out = DaticleMap::new();
            for (k, v) in map {
                out.insert(res!(resolve_dat(k, root)), res!(resolve_dat(v, root)));
            }
            Dat::Map(out)
        },
        other => other.clone(),
    })
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reference_is_expanded_or_refused() -> Outcome<()> {
        let root = Path::new(".");
        std::env::set_var("FE2O3_JDAT_CFG_TEST_VALUE", "expanded");
        assert_eq!(res!(resolve("{env:FE2O3_JDAT_CFG_TEST_VALUE}", root)), "expanded");
        assert_eq!(res!(resolve("a {env:FE2O3_JDAT_CFG_TEST_VALUE} b", root)),
            "a expanded b");
        assert_eq!(res!(resolve("{env:FE2O3_JDAT_CFG_TEST_ABSENT:fallback}", root)),
            "fallback");
        assert!(resolve("{env:FE2O3_JDAT_CFG_TEST_ABSENT}", root).is_err(),
            "a reference with no value and no default is refused");
        assert!(resolve("{env:unclosed", root).is_err());
        assert_eq!(res!(resolve("nothing to expand", root)), "nothing to expand");
        Ok(())
    }

    #[test]
    fn what_names_nothing_is_untouched() -> Outcome<()> {
        let root = Path::new(".");
        let dat = Dat::List(vec![
            Dat::Str(fmt!("plain")),
            Dat::U8(42),
        ]);
        let out = res!(resolve_dat(&dat, root));
        assert_eq!(out, dat);
        Ok(())
    }
}
