//! What a configuration is, and how a value in one is allowed to point
//! somewhere else.
//!
//! [`Config`] is the trait a configuration type implements. [`resolve`] and
//! [`resolve_dat`] are the indirection every Oxedyne configuration file has: a
//! value that must not sit in the file itself is written `{file:secret}` or
//! `{env:VAR}` and is fetched when the file is read. [`resolve_root`] is the
//! app-root rule every app shares.

use crate::{
    prelude::*,
    file::JdatMapFile,
};

use oxedyne_fe2o3_core::{
    prelude::*,
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
/// The environment is expanded first, so a variable may name a file.
pub fn resolve(value: &str, root: &Path)
    -> Outcome<String>
{
    let named = res!(resolve_env(value));
    resolve_files(&named, value, root)
}

/// Expand only the `{env:VAR[:default]}` references, reading no file.
pub fn resolve_env(whole: &str)
    -> Outcome<String>
{
    let mut out = fmt!("{}", whole);
    while let Some(start) = out.find("{env:") {
        let end = match out[start..].find('}') {
            Some(i) => start + i,
            None => return Err(err!(
                "The configuration value {:?} opens an {{env: reference and does not \
                close it.", whole;
            Invalid, Input)),
        };
        let inner = fmt!("{}", &out[start + 5..end]);
        let (name, fallback) = match inner.find(':') {
            Some(i) => (&inner[..i], Some(&inner[i + 1..])),
            None    => (inner.as_str(), None),
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
        out.replace_range(start..=end, &got);
    }
    Ok(out)
}

/// Every file [`resolve`] would read for `value`, in order, after the environment is expanded
/// and each path joined to `root` as `resolve` joins it, without reading any. A caller that vets
/// where secrets come from can refuse a path before anything is read.
pub fn file_refs(value: &str, root: &Path)
    -> Outcome<Vec<PathBuf>>
{
    let named = res!(resolve_env(value));
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(fref) = res!(first_file_ref(&named[pos..], value)) {
        out.push(root.join(&named[pos + fref.start + fref.marker..pos + fref.end]));
        pos += fref.end + 1;
    }
    Ok(out)
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
        path if path.starts_with("~/") => match std::env::var("HOME") {
            Ok(home) => PathBuf::from(fmt!("{}{}", home, &path[1..])),
            Err(_) => return Err(err!(
                "The app root {:?} starts with '~/' and HOME is not set.", path;
            Configuration, Missing, Invalid)),
        },
        path if path.starts_with("./") => res!(std::env::current_dir()).join(&path[2..]),
        path => PathBuf::from(path),
    })
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

/// The earliest file reference in `s`. `whole` is the original value, named in an error.
fn first_file_ref(s: &str, whole: &str)
    -> Outcome<Option<FileRef>>
{
    // '{file:' is not a substring of '{file?:', so neither search can match the other.
    let (start, marker) = match (s.find("{file:"), s.find("{file?:")) {
        (None,    None)             => return Ok(None),
        (Some(r), None)             => (r, 6),
        (None,    Some(o))          => (o, 7),
        (Some(r), Some(o)) if o < r => (o, 7),
        (Some(r), Some(_))          => (r, 6),
    };
    match s[start..].find('}') {
        Some(i) => Ok(Some(FileRef { start, end: start + i, marker })),
        None    => Err(err!(
            "The configuration value {:?} opens a {{file{}: reference and does not close it.",
            whole, if marker == 7 { "?" } else { "" };
        Invalid, Input)),
    }
}

fn resolve_files(value: &str, whole: &str, root: &Path)
    -> Outcome<String>
{
    let mut out = fmt!("{}", value);
    while let Some(fref) = res!(first_file_ref(&out, whole)) {
        let rel = fmt!("{}", &out[fref.start + fref.marker..fref.end]);
        let path = root.join(&rel);
        let mark = if fref.optional() { "?" } else { "" };
        let held = match std::fs::read_to_string(&path) {
            Ok(t) => fmt!("{}", t.trim()),
            // Only not-found is tolerated, so a present file that cannot be read still fails.
            Err(e) if fref.optional()
                && e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(err!(e,
                "The configuration reads {{file{}:{}}}, and {:?} could not be read.",
                mark, rel, path;
            IO, File, Read)),
        };
        out.replace_range(fref.start..=fref.end, &held);
    }
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
