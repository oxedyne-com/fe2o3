use crate::{
    arg::Arg,
    core::{
        Syntax,
        SyntaxPrefs,
    },
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
    pub vals:   Vec<Val>,               // Expected values.
    pub rest:   Option<Val>,            // Words after "--", taken as they stand.
    pub rargs:  Vec<String>,            // Which arguments are required?
    pub excess: Option<String>,         // The command's own sentence for a word too many.
    // CLI
    pub help:   Option<String>,         // One line, for the command table.
    pub detail: Option<String>,         // Prose for the command's own page.
    pub see:    Vec<String>,            // Commands or topics named under SEE ALSO.
    pub prefs:  SyntaxPrefs,
    pub cat:    String,                 // Command category, a help heading.
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
    
    pub fn expected_vals<V: Into<Val>>(mut self, vals: Vec<V>) -> Self {
        self.cfg.vals = vals.into_iter().map(|v| v.into()).collect();
        self
    }

    pub fn rest(mut self, val: Val) -> Self {
        self.cfg.rest = Some(val);
        self
    }

    pub fn detail<S: Into<String>>(mut self, s: S) -> Self {
        self.cfg.detail = Some(s.into());
        self
    }

    pub fn cat<S: Into<String>>(mut self, s: S) -> Self {
        self.cfg.cat = s.into();
        self
    }

    pub fn excess<S: Into<String>>(mut self, s: S) -> Self {
        self.cfg.excess = Some(s.into());
        self
    }

    /// The command's arguments in the order they were added.
    pub fn args_in_order(&self) -> Vec<&Arg> {
        self.args.iter().filter_map(|(k, v)| match (k, v) {
            (Key::Id(_), Recursive::Val(arg)) => Some(arg),
            _ => None,
        }).collect()
    }

    pub fn help<S: Into<String>>(mut self, s: S) -> Self {
        self.cfg.help = Some(s.into());
        self
    }

    /// Collects the short form switch strings of those arguments that have one.
    pub fn collect_short_arg_names(&self) -> Vec<String> {
        self.args_in_order()
            .into_iter()
            .filter_map(|arg| arg.config().hyph1.clone())
            .collect()
    }
}

