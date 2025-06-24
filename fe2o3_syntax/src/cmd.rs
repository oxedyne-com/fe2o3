use crate::{
    arg::Arg,
    core::{
        Syntax,
        SyntaxPrefs,
    },
    key::Key,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    map::Recursive,
};
use oxedyne_fe2o3_jdat::kind::Kind;

use std::{
    collections::BTreeMap,
    fmt,
};

#[derive(Clone, Default)]
pub struct Cmd {
    pub cfg:        CmdConfig,
    pub args:       BTreeMap<Key, Recursive<Key, Arg>>,
    // Binary
    pub id:         u16,
    pub next_arg_id:u16,
}

#[derive(Clone, Default)]
pub struct CmdConfig {
    pub name:   String,
    pub vals:   Vec<(Kind, String)>,    // Expected value kindicles, with help text.
    pub rargs:  Vec<String>,            // Which arguments are required?
    // CLI
    pub help:   Option<String>,         // Command help text.
    pub prefs:  SyntaxPrefs,
    pub cat:    String,                 // Command category.
}

impl fmt::Debug for Cmd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f,
            "Cmd {{ name: {}, kinds: {:?}, args: {:?} }}",
            &self.config().name,
            &self.config().vals,
            &self.args,
        )
    }
}

impl Cmd {

    pub fn new<S: Into<String>>(name: S) -> Outcome<Self> {
        let name = name.into();
        if name.starts_with('-') || name.contains(' ') {
            Err(err!(
                "Command name '{}' should not start with a hyphen or contain spaces", name;
            Input, Invalid))
        } else {
            let cfg = CmdConfig {
                name: name,
                ..Default::default()
            };
            Ok(Self {
                cfg: cfg,
                ..Default::default()
            })
        }
    }

    pub fn config(&self) -> &CmdConfig { &self.cfg }

    pub fn add_arg(mut self, mut a: Arg) -> Outcome<Self> {
        a.id = self.next_arg_id;
        self.next_arg_id = res!(Syntax::inc_counter(
            self.next_arg_id,
            fmt!("command '{}' arguments", self.config().name),
        ));
        res!(a.attach_arg(
            &mut self.args,
            &mut self.cfg.rargs,
        ));
        Ok(self)
    }
    
    pub fn expected_vals(mut self, vals: Vec<(Kind, String)>) -> Self {
        self.cfg.vals = vals;
        self
    }

    pub fn help<S: Into<String>>(mut self, s: S) -> Self {
        self.cfg.help = Some(s.into());
        self
    }

    /// Collects all hyph1 (short form switch) strings from the command's arguments.
    pub fn collect_short_arg_names(&self) -> Vec<String> {
        self.args
            .iter()
            .filter_map(|(_, arg_rec)| {
                // Get the actual Arg from the Recursive enum.
                if let Recursive::Val(arg) = arg_rec {
                    Some(arg.config().hyph1.clone())
                } else {
                    None
                }
            })
            .collect()
    }
}

