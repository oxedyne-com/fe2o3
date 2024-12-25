use crate::{
    cmd::{
        Cmd,
        CmdConfig,
    },
    core::SyntaxPrefs,
    key::Key,
};

use oxedize_fe2o3_core::{
    prelude::*,
    map::Recursive,
};
use oxedize_fe2o3_jdat::kind::Kind;

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
    pub vals:   Vec<(Kind, String)>,// Expected value kindicles, with help text.
    pub reqd:   bool,               // The argument is required?
    // CLI
    pub hyph1:  String,             // Short form switch.
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

    pub fn hyphen_check(&self, s: &str) -> Outcome<()> {
        if s.starts_with('-') || s.contains(' ') {
            return Err(err!(errmsg!(
                "Single hyphen name '{}' for argument '{}' should not start with a \
                hyphen or contain spaces",
                s,
                self.config().name,
            ), ErrTag::Input, ErrTag::Invalid));
        }
        Ok(())
    }

    pub fn hyph1<S: Into<String>>(mut self, s: S) -> Outcome<Self> {
        let s = s.into();
        res!(self.hyphen_check(s.as_str()));
        self.cfg.hyph1 = s;
        Ok(self)
    }

    pub fn hyph2<S: Into<String>>(mut self, s: S) -> Outcome<Self> {
        let s = s.into();
        res!(self.hyphen_check(s.as_str()));
        self.cfg.hyph2 = Some(s);
        Ok(self)
    }

    pub fn expected_vals(mut self, vals: Vec<(Kind, String)>) -> Self {
        self.cfg.vals = vals;
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
        // Complete possible many-to-one mappings use the Arg before it gets moved when inserted.
        map.insert(
            Key::Str(self.canonical_name()),
            Recursive::Key(Key::Id(self.id)),
        );
        
        let hyph1 = &self.config().hyph1;
        let mut s = self.config().prefs.arg_hyph1_pfx.clone();
        s.push_str(hyph1);
        map.insert(
            Key::Str(s),
            Recursive::Key(Key::Id(self.id)),
        );
        
        if let Some(hyph2) = &self.config().hyph2 {
            let mut s = self.config().prefs.arg_hyph2_pfx.clone();
            s.push_str(hyph2);
            map.insert(
                Key::Str(s),
                Recursive::Key(Key::Id(self.id)),
            );
        }
        map.insert(Key::Str(self.config().name.clone()), Recursive::Key(Key::Id(self.id)));
        if required {
            rargs.push(self.canonical_name());
        }
        map.insert(Key::Id(self.id), Recursive::Val(self));
        Ok(())
    }
}

impl fmt::Display for Arg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "'{}'", self.config().hyph1)?;
        if let Some(long) = &self.config().hyph2 {
            write!(f, ", '{}'", long)?;
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

