use crate::{
    cmd::{
        Cmd,
        CmdConfig,
    },
    core::SyntaxPrefs,
    key::Key,
    val::Val,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    map::Recursive,
};
use std::{
    collections::BTreeMap,
    fmt,
};


#[derive(Clone, Debug, Default)]
pub struct Arg {
    pub cfg:    ArgConfig,
    pub id:     u16, // Binary
}

#[derive(Clone, Debug, Default)]
pub struct ArgConfig {
    pub name:   String,             // For internal use only.
    pub vals:   Vec<Val>,           // Expected values.
    pub reqd:   bool,               // The argument is required?
    // CLI
    pub hyph1:  Option<String>,     // Short form switch.
    pub hyph2:  Option<String>,     // Long form switch.
    pub help:   Option<String>,     // Argument help text.
    pub prefs:  SyntaxPrefs,
}

impl From<ArgConfig> for Arg {
    fn from(cfg: ArgConfig) -> Self {
        Self {
            cfg: cfg,
            ..Default::default()
        }
    }
}

impl Arg {

    /// The name of an `Arg` is for internal use only, unlike a `Cmd`. 
    pub fn new<S: Into<String>>(name: S) -> Outcome<Self> {
        let cfg = ArgConfig {
            name: name.into(),
            ..Default::default()
        };
        Ok(Self {
            cfg:    cfg,
            ..Default::default()
        })
    }

    pub fn config(&self) -> &ArgConfig { &self.cfg }

    pub fn required(mut self, b: bool) -> Self {
        self.cfg.reqd = b;
        self
    }

    pub fn canonical_name(&self) -> String {
        self.cfg.name.clone()
    }

    /// The argument's shortest name as it is written on a command line, hyphens
    /// and all: the short form, else the long form, else the canonical name.
    ///
    /// This is what a message prints itself with, because the canonical name is
    /// the one it is filed under and not one a reader could type back.
    pub fn short_name(&self) -> String {
        match (&self.cfg.hyph1, &self.cfg.hyph2) {
            (Some(h1), _)       => fmt!("{}{}", self.cfg.prefs.arg_hyph1_pfx, h1),
            (None, Some(h2))    => fmt!("{}{}", self.cfg.prefs.arg_hyph2_pfx, h2),
            (None, None)        => self.cfg.name.clone(),
        }
    }

    /// The argument's longest hyphenated name: the long form, else the short form, else the
    /// canonical name.
    pub fn long_name(&self) -> String {
        match (&self.cfg.hyph1, &self.cfg.hyph2) {
            (_, Some(h2))       => fmt!("{}{}", self.cfg.prefs.arg_hyph2_pfx, h2),
            (Some(h1), None)    => fmt!("{}{}", self.cfg.prefs.arg_hyph1_pfx, h1),
            (None, None)        => self.cfg.name.clone(),
        }
    }

    /// Every hyphenated name the argument answers to, short form first.
    pub fn hyphenated_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        if let Some(h1) = &self.cfg.hyph1 {
            names.push(fmt!("{}{}", self.cfg.prefs.arg_hyph1_pfx, h1));
        }
        if let Some(h2) = &self.cfg.hyph2 {
            names.push(fmt!("{}{}", self.cfg.prefs.arg_hyph2_pfx, h2));
        }
        names
    }

    pub fn hyphen_check(&self, s: &str) -> Outcome<()> {
        if s.starts_with('-') || s.contains(' ') {
            return Err(err!(
                "Single hyphen name '{}' for argument '{}' should not start with a \
                hyphen or contain spaces",
                s, self.config().name;
            Input, Invalid));
        }
        Ok(())
    }

    pub fn hyph1<S: Into<String>>(mut self, s: S) -> Outcome<Self> {
        let s = s.into();
        res!(self.hyphen_check(s.as_str()));
        self.cfg.hyph1 = Some(s);
        Ok(self)
    }

    pub fn hyph2<S: Into<String>>(mut self, s: S) -> Outcome<Self> {
        let s = s.into();
        res!(self.hyphen_check(s.as_str()));
        self.cfg.hyph2 = Some(s);
        Ok(self)
    }

    pub fn expected_vals<V: Into<Val>>(mut self, vals: Vec<V>) -> Self {
        self.cfg.vals = vals.into_iter().map(|v| v.into()).collect();
        self
    }

    /// Allows user to attach a help string to the `Arg`.
    pub fn help<S: Into<String>>(mut self, s: S) -> Self {
        self.cfg.help = Some(s.into());
        self
    }

    pub fn attach_arg(
        self,
        map:    &mut BTreeMap<Key, Recursive<Key, Arg>>,
        rargs:  &mut Vec<String>
    ) 
        -> Outcome<()>
    {
        let required = self.config().reqd;
        res!(Val::check_shape(&self.config().vals, &fmt!("argument '{}'", self.config().name)));
        // Complete possible many-to-one mappings use the Arg before it gets moved when inserted.
        // A missing short form registers nothing: it used to register a bare "-".
        let mut keys = self.hyphenated_names();
        keys.push(self.canonical_name());
        for k in keys {
            let k = Key::Str(k);
            if let Some(Recursive::Key(Key::Id(other))) = map.get(&k) {
                if *other != self.id {
                    return Err(err!(
                        "The argument '{}' would answer to '{}', which another argument \
                        already answers to.", self.config().name, k;
                    Input, Exists));
                }
            }
            map.insert(k, Recursive::Key(Key::Id(self.id)));
        }
        if required {
            rargs.push(self.canonical_name());
        }
        map.insert(Key::Id(self.id), Recursive::Val(self));
        Ok(())
    }
}

impl fmt::Display for Arg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = self.hyphenated_names();
        if names.is_empty() {
            ok!(write!(f, "'{}'", self.config().name));
        }
        for (i, name) in names.iter().enumerate() {
            if i > 0 { ok!(write!(f, ", ")); }
            ok!(write!(f, "'{}'", name));
        }
        Ok(())
    }
}

impl From<CmdConfig> for Cmd {
    fn from(cfg: CmdConfig) -> Self {
        Self {
            cfg: cfg,
            ..Default::default()
        }
    }
}

