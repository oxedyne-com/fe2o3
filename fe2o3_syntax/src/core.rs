use crate::{
    arg::{
        Arg,
        ArgConfig,
    },
    cmd::Cmd,
    key::Key,
};

use oxedize_fe2o3_core::{
    prelude::*,
    map::{
        Recursive,
        MapRec,
    },
};
use oxedize_fe2o3_jdat::{
    kind::Kind,
    version::SemVer,
};

use std::{
    collections::BTreeMap,
    fmt,
    sync::Arc,
};


#[derive(Clone, Debug, Default)]
pub struct Syntax {
    pub cfg:    SyntaxConfig,
    pub args:   BTreeMap<Key, Recursive<Key, Arg>>,
    pub cmds:   BTreeMap<Key, Recursive<Key, Cmd>>,
    // Binary
    pub next_arg_id:    u16,
    pub next_cmd_id:    u16,
}

#[derive(Clone, Debug)]
pub struct SyntaxPrefs {
    pub arg_hyph1_pfx:  String,
    pub arg_hyph2_pfx:  String,
}

impl Default for SyntaxPrefs {
    fn default() -> Self {
        Self {
            arg_hyph1_pfx:  fmt!("-"),
            arg_hyph2_pfx:  fmt!("--"),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SyntaxConfig {
    pub name:   String,
    pub ver:    SemVer,
    pub vals:   Vec<(Kind, String)>,    // Expected value kindicles, with help text.
    pub rargs:  Vec<String>, // Required arguments.
    pub cmds:   BTreeMap<Key, Recursive<Key, Cmd>>,
    // CLI
    pub author: Option<String>, 
    pub about:  Option<String>,
    pub width:  usize, // text width for screen output
    // Customisation.
    pub prefs:  SyntaxPrefs,
}

impl From<SyntaxConfig> for Syntax {
    fn from(cfg: SyntaxConfig) -> Self {
        Self {
            cfg: cfg,
            ..Default::default()
        }
    }
}

impl Syntax {

    pub const HELP_COL1: usize = 5;
    pub const HELP_COL2: usize = 15;
    pub const HELP_COL4: usize = 50;

    /// Its ok for the `Syntax` name to contain separator characters such as spaces.
    pub fn new<S: Into<String>>(name: S) -> Self {
        let cfg = SyntaxConfig {
            name: name.into(),
            width: 80,
            ..Default::default()
        };
        Self {
            cfg:    cfg,
            ..Default::default()
        }
    }

    pub fn config(&self) -> &SyntaxConfig { &self.cfg }

    pub fn inc_counter(counter: u16, desc: String) -> Outcome<u16> {
        match counter.checked_add(1) {
            Some(i) => Ok(i),
            None => {
                return Err(err!(errmsg!(
                    "The id counter for {} has reached its upper \
                    limit of {}", desc, u16::MAX,
                ), ErrTag::Counter, ErrTag::Overflow));
            },
        }
    }

    pub fn add_arg(mut self, mut a: Arg) -> Outcome<Self> {
        a.id = self.next_arg_id;
        self.next_arg_id = res!(Syntax::inc_counter(
            self.next_arg_id,
            fmt!("syntax '{}' arguments", self),
        ));
        res!(a.attach_arg(
            &mut self.args,
            &mut self.cfg.rargs,
        ));
        Ok(self)
    }

    pub fn remove_arg<K: Into<Key>>(mut self, key: K) -> Self {
        let key = key.into();
        let mut id_opt = None;
        if let Some(Recursive::Key(id)) = self.args.get(&key) {
            id_opt = Some(id.clone());
        }
        if let Some(id) = id_opt {
            self.args.remove(&id.clone());
        }
        self.args.remove(&key);
        self
    }

    /// Add a command to a syntax.
    pub fn add_cmd(mut self, mut c: Cmd) -> Outcome<Self> {
        c.id = self.next_cmd_id;
        self.next_cmd_id = res!(Syntax::inc_counter(
            self.next_cmd_id,
            fmt!("syntax '{}' commands", self),
        ));
        self.cmds.insert(Key::Str(c.config().name.clone()), Recursive::Key(Key::Id(c.id)));
        self.cmds.insert(Key::Id(c.id), Recursive::Val(c));
        Ok(self)
    }

    pub fn remove_cmd<K: Into<Key>>(mut self, key: K) -> Self {
        let key = key.into();
        let mut id_opt = None;
        if let Some(Recursive::Key(id)) = self.cmds.get(&key) {
            id_opt = Some(id.clone());
        }
        if let Some(id) = id_opt {
            self.cmds.remove(&id);
        }
        self.cmds.remove(&key);
        self
    }

    pub fn get_cmd<K: Into<Key>>(&self, key: K) -> Option<&Cmd> {
        let key = key.into();
        self.cmds.get_recursive(&key)
    }

    pub fn expected_vals(mut self, vals: Vec<(Kind, String)>) -> Self {
        self.cfg.vals = vals;
        self
    }

    /// Add help as a command.
    pub fn with_default_help_cmd(self) -> Outcome<Self> {
        let c = res!(Cmd::new("help"))
            .help("Display helpful information");
        self.add_cmd(c)
    }

    /// Add help as an argument.
    pub fn with_default_help_arg(self) -> Outcome<Self> {
        let cfg = ArgConfig {
            name:   fmt!("help"),
            hyph1:  fmt!("h"),
            hyph2:  Some(fmt!("help")),
            reqd:   false,
            help:   Some(fmt!("Display helpful information")),
            ..Default::default()
        };
        self.add_arg(Arg {
            cfg: cfg,
            ..Default::default()
        })
    }

    pub fn ver(mut self, v: SemVer) -> Self {
        self.cfg.ver = v;
        self
    }

    pub fn about<S: Into<String>>(mut self, s: S) -> Self {
        self.cfg.about = Some(s.into());
        self
    }

    pub fn set_width(mut self, w: usize) {
        self.cfg.width = w;
    }

}

impl fmt::Display for Syntax {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f,
            "{}V{}",
            self.config().name,
            self.config().ver,
        )
    }
}

new_type!(SyntaxRef, Arc<Syntax>, Clone, Debug, Default);

/// This newtype makes it easier to give spawned `Msg` and `MsgCmd` objects their own immutable
/// reference to `Syntax`.  For a slight performance cost there is no need for lifetime
/// annotation and certain compile time borrowing checks encountered during message decoding are
/// pushed to runtime.
impl SyntaxRef {

    pub fn new(syntax: Syntax) -> Self {
        Self(Arc::new(syntax))
    }
}
