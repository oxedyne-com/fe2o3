//! What a configuration is, and how a value in one is allowed to point
//! somewhere else.
//!
//! [`Config`] is the trait a configuration type implements. [`resolve`] and
//! [`resolve_dat`] are the indirection every Oxedyne configuration file has: a
//! value that must not sit in the file itself is written `{file:secret}` or
//! `{env:VAR}` and is fetched when the file is read.

use crate::{
    prelude::*,
    file::JdatMapFile,
};

use oxedyne_fe2o3_core::{
    prelude::*,
};

use std::path::Path;


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


pub fn resolve(value: &str, root: &Path)
    -> Outcome<String>
{
    // The environment first, so an environment value may name a file.
    let named = res!(resolve_env(value));
    resolve_files(&named, value, root)
}

fn resolve_env(whole: &str)
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

fn resolve_files(value: &str, whole: &str, root: &Path)
    -> Outcome<String>
{
    let mut out = fmt!("{}", value);
    while let Some(start) = out.find("{file:") {
        let end = match out[start..].find('}') {
            Some(i) => start + i,
            None => return Err(err!(
                "The configuration value {:?} opens a {{file: reference and does not \
                close it.", whole;
            Invalid, Input)),
        };
        let rel = fmt!("{}", &out[start + 6..end]);
        let path = root.join(&rel);
        let held = match std::fs::read_to_string(&path) {
            Ok(t) => fmt!("{}", t.trim()),
            Err(e) => return Err(err!(e,
                "The configuration reads {{file:{}}}, and {:?} could not be read.",
                rel, path;
            IO, File, Read)),
        };
        out.replace_range(start..=end, &held);
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
