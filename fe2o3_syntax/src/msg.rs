use crate::{
    arg::Arg,
    cmd::Cmd,
    core::{
        Syntax,
        SyntaxRef,
    },
    key::Key,
    val::{
        Slots,
        Val,
    },
};

use oxedyne_fe2o3_core::{
    prelude::*,
    byte::{
        Encoding,
        IntoBytes,
        ToBytes,
        FromBytes,
    },
    map::{
        MapRec,
        Recursive,
    },
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_num::{
    float::Float64,
    prelude::bigdecimal::ToPrimitive,
};
use oxedyne_fe2o3_text::split::StringSplitter;

use std::{
    collections::{
        BTreeMap,
        BTreeSet,
    },
    fmt,
};

/// Where the words of a text message came from, which decides how a string value is read.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Source {
    #[default]
    Line,   // a REPL line or a wire message: every value is decoded as a daticle
    Argv,   // a process's arguments, already split and unquoted by a shell
}

#[derive(Debug, PartialEq)]
pub enum Collecting {
    None,
    Message,
    MessageArg,
    Command,
    CommandArg,
}

/// Used to capture the outstanding values and active command and/or argument at the end of message processing.
#[derive(Clone, Debug, Default)]
pub struct MsgEndState {
    pub vals:   Vec<Kind>,
    pub arg:    Option<String>,
    pub cmd:    Option<String>,
    pub help:   bool,           // a command line stopped at -h or --help
}

/// A `Syntax` specifies message structure for validation, while `Msg` is used for transmission
/// and receipt of messages using the syntax.  `Msg` constructors from binary and string formats
/// cannot be associated methods because of the essential requirement for an embedded
/// `SyntaxRef`.
#[derive(Clone, Debug)]
pub struct Msg {
    pub syntax: SyntaxRef,
    pub sname:  String, // Syntax name
    // Message contents
    pub vals:   Vec<Dat>,
    pub args:   BTreeMap<String, Vec<Dat>>, // one-to-one
    pub cmds:   BTreeMap<String, MsgCmd>, // one-to-one
    // Decoding
    pub end:    MsgEndState,
    // Encoding
    pub enc:    Encoding,
}

impl fmt::Display for Msg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for val in &self.vals {
            if !first { ok!(write!(f, " ")); } 
            ok!(write!(f, "{:?}", val));
            first = false;
        }
        for (k, argvals) in &self.args {
            if !first { ok!(write!(f, " ")); }
            // Written as a reader would type it, not as the message files it.
            ok!(write!(f, "{}", self.arg_short_name(k)));
            for val in argvals {
                ok!(write!(f, " {:?}", val));
            }
            first = false;
        }
        // A command prints its own name, so printing the key as well said it
        // twice.
        for (_k, cmd) in &self.cmds {
            if !first { ok!(write!(f, " ")); }
            ok!(write!(f, "{}", cmd));
            first = false;
        }
        Ok(())
    }
}

impl ToBytes for Msg {

    fn to_bytes(&self, mut buf: Vec<u8>) -> Outcome<Vec<u8>> {
        let encoding = self.encoding();
        buf.push(*encoding as u8);
        match encoding {
            Encoding::Binary => {
                // Message values
                buf.push(Dat::LIST_CODE);
                buf = res!(Dat::vec_to_bytes(&self.vals, buf));
                // Message arguments
                buf = res!(Dat::C64(self.args.len() as u64).to_bytes(buf));
                for (k, v) in &self.args {
                    if let Some(arg) = self.syntax().args.get_recursive(&Key::Str(k.clone())) {
                        buf = res!(Dat::U16(arg.id).to_bytes(buf));
                        buf.push(Dat::LIST_CODE);
                        buf = res!(Dat::vec_to_bytes(&v, buf));
                    }
                }
                // Commands
                buf = res!(Dat::C64(self.cmds.len() as u64).to_bytes(buf));
                for (k, msgcmd) in &self.cmds {
                    if let Some(cmd) = self.syntax().cmds.get_recursive(&Key::Str(k.clone())) {
                        buf = res!(Dat::U16(cmd.id).to_bytes(buf));
                        // Command values
                        buf.push(Dat::LIST_CODE);
                        buf = res!(Dat::vec_to_bytes(&msgcmd.vals, buf));
                        // Command arguments
                        buf = res!(Dat::C64(msgcmd.args.len() as u64).to_bytes(buf));
                        for (k, v) in &msgcmd.args {
                            if let Some(arg) = cmd.args.get_recursive(&Key::Str(k.clone())) {
                                buf = res!(Dat::U16(arg.id).to_bytes(buf));
                                buf.push(Dat::LIST_CODE);
                                buf = res!(Dat::vec_to_bytes(&v, buf));
                            }
                        }
                        // Only a command declaring a rest carries one, so the bytes of every
                        // other command are as they were.
                        if cmd.config().rest.is_some() {
                            match &msgcmd.rest {
                                Some(words) => {
                                    buf = res!(Dat::Bool(true).to_bytes(buf));
                                    buf.push(Dat::LIST_CODE);
                                    buf = res!(Dat::vec_to_bytes(words, buf));
                                },
                                None => buf = res!(Dat::Bool(false).to_bytes(buf)),
                            }
                        }
                    }
                }
            },
            Encoding::UTF8 => buf.extend_from_slice(self.to_string().as_bytes()),
            _ => return Err(err!("Unimplemented message encoding {:?}.", encoding;
                Unimplemented, Encode)),
        }
        debug!("{:02x?}", buf);
        Ok(buf)
    }
}

impl IntoBytes for Msg {

    fn into_bytes(self, buf: Vec<u8>) -> Outcome<Vec<u8>> {
        self.to_bytes(buf) // TODO check for potential optimisations
    }
}

impl Msg {

    pub fn new(syntax: SyntaxRef) -> Self {
        let sname = syntax.config().name.clone();
        Self { 
            syntax,
            sname,
            vals:   Vec::new(),
            args:   BTreeMap::new(),
            cmds:   BTreeMap::new(),
            end:    MsgEndState::default(),
            enc:    Encoding::default(),
        }
    }

    pub fn new_cmd<S: Into<String>>(&self, name: S) -> Outcome<MsgCmd> {
        MsgCmd::new(self.syntaxref(), name)
    }

    pub fn syntaxref(&self) -> SyntaxRef { self.syntax.clone() }
    pub fn syntax<'a>(&'a self) -> &'a Syntax { self.syntax.as_ref() }

    pub fn encoding(&self) -> &Encoding { &self.enc }
    pub fn set_encoding(&mut self, enc: Encoding) { self.enc = enc; }

    fn get_syntax_arg<'a, S: Into<String>>(
        &'a self,
        arg_name: S,
    )
        ->  Outcome<&'a Arg>
    {
        let arg_name = arg_name.into();
        self.syntax().args.get_recursive(&Key::Str(arg_name.clone())).ok_or_else(|| err!(
            "Cannot find this message argument '{}' in the syntax '{}'.",
            arg_name, self.sname;
        Missing))
    }

    fn arg_key<S: Into<String>>(&self, arg_name: S) -> String {
        let arg_name = arg_name.into();
        match self.syntax().args.get_recursive(&Key::Str(arg_name.clone())) {
            Some(arg) => arg.canonical_name(),
            None => arg_name,
        }
    }

    fn arg_short_name<S: Into<String>>(&self, arg_name: S) -> String {
        let arg_name = arg_name.into();
        match self.syntax().args.get_recursive(&Key::Str(arg_name.clone())) {
            Some(arg) => arg.short_name(),
            None => arg_name,
        }
    }

    pub fn add_cmd(
        mut self,
        msgcmd: MsgCmd,
    )
        ->  Outcome<Self>
    {
        let name = msgcmd.name.clone();
        if !self.syntax().cmds.contains_key(&Key::Str(name.clone())) {
            return Err(err!(
                "Can't find command '{}' in syntax '{}'.",
                name, self.sname;
            Invalid, Input));
        }
        self.cmds.insert(name, msgcmd);
        Ok(self)
    }
    
    pub fn add_arg<S: Into<String>>(
        mut self,
        arg_name: S,
    )
        ->  Outcome<Self>
    {
        let arg_name = arg_name.into();
        // Make sure the argument exists in the syntax.
        if !self.syntax().args.contains_key(&Key::Str(arg_name.clone())) {
            return Err(err!(
                "Can't find message argument '{}' in syntax '{}'.",
                arg_name, self.sname;
            Invalid, Input));
        }
        let key = self.arg_key(arg_name.clone());
        // Check whether it has already been added to the Msg.
        if self.args.contains_key(&key) {
            return Err(err!(
                "Argument '{}' has already been added to message.", arg_name;
            Invalid, Input, Exists));
        }
        self.args.insert(key, Vec::new());
        Ok(self)
    }

    pub fn add_msg_val(
        self,
        val: Dat,
    )
        ->  Outcome<Self>
    {
        self.add_val::<String>(None, Some(val))
    }

    pub fn add_arg_val<S: Into<String>>(
        self,
        arg_name:   S,
        val:        Option<Dat>,
    )
        ->  Outcome<Self>
    {
        self.add_val(Some(arg_name), val)
    }

    pub fn add_val<S: Into<String>>(
        mut self,
        arg_opt:    Option<S>,
        val_opt:    Option<Dat>,
    )
        ->  Outcome<Self>
    {
        let arg_opt = arg_opt.map(|s| s.into());
        let exp_vals: Vec<Val> = match &arg_opt {
            Some(arg_name) => {
                let arg = res!(self.get_syntax_arg(arg_name.clone()));
                arg.config().vals.clone()
            },
            None => self.syntax().config().vals.clone(),
        };
        // File the value under the argument's one name, whichever of its names
        // the caller wrote.
        let arg_opt = arg_opt.map(|name| self.arg_key(name));

        let v: &mut Vec<Dat> = match arg_opt {
            Some(arg_name) => match self.args.get_mut(&arg_name) {
                Some(v) => v,
                None => {
                    self.args.insert(arg_name.clone(), Vec::new());
                    match self.args.get_mut(&arg_name) {
                        Some(v) => v,
                        None => return Err(err!(
                            "Argument '{}' was just created, but no longer present.", arg_name;
                        Bug, Unreachable)),
                    }
                },
            },
            None => &mut self.vals,
        };

        let next = match Val::slot(&exp_vals, v.len()) {
            Some(next) => next,
            None => return Err(err!(
                "Message already has all {} of its expected values.", v.len();
            Invalid, Input, Exists)),
        };

        match val_opt {
            Some(val) => {
                if val.kind() == next.kind {
                    v.push(val);
                } else {
                    return Err(err!(
                        "Message already has {} values, and the next one must be a {:?}, \
                        not a {:?}.", v.len(), next.kind, val.kind();
                    Invalid, Input));
                }
            },
            None => (),
        }

        Ok(self)
    }

    pub fn get_vals(&self) -> Option<&Vec<Dat>> {
        if self.vals.len() == 0 {
            None
        } else {
            Some(&self.vals)
        }
    }

    pub fn get_arg_vals<S: Into<String>>(&self, a: S) -> Option<&Vec<Dat>> {
        match self.args.get(&self.arg_key(a)) {
            Some(vals) => if vals.len() == 0 {
                None
            } else {
                Some(&vals)
            },
            None => None,
        }
    }

    pub fn get_arg_vals_mut<S: Into<String>>(&mut self, a: S) -> Option<&mut Vec<Dat>> {
        let key = self.arg_key(a);
        match self.args.get_mut(&key) {
            Some(vals) => if vals.len() == 0 {
                None
            } else {
                Some(vals)
            },
            None => None,
        }
    }

    pub fn get_cmd<S: Into<String>>(&self, c: S) -> Option<&MsgCmd> {
        self.cmds.get(&(c.into()))
    }

    pub fn get_cmd_mut<S: Into<String>>(&mut self, c: S) -> Option<&mut MsgCmd> {
        self.cmds.get_mut(&(c.into()))
    }

    pub fn remove_cmd<S: Into<String>>(&mut self, c: S) -> Option<MsgCmd> {
        let c = c.into();
        self.cmds.remove(&c)
    }

    pub fn has_args(&self) -> bool {
        self.args.len() > 0
    }

    pub fn has_arg<S: Into<String>>(&self, a: S) -> bool {
        self.args.contains_key(&self.arg_key(a))
    }

    pub fn has_only_arg<S: Into<String>>(&self, a: S) -> Outcome<bool> {
        let a = a.into();
        let has = self.args.contains_key(&self.arg_key(a.clone()));
        if self.args.len() > 1 {
            Err(err!(
                "The argument '{}' {}", a, if has {
                    "does not exist and there are other arguments."
                } else {
                    "does exist but there are other arguments."
                };
            Input, Invalid, Excessive))
        } else {
            Ok(has)
        }
    }

    pub fn has_cmd<S: Into<String>>(&self, c: S) -> bool {
        self.cmds.contains_key(&(c.into()))
    }

    pub fn get_cmd_vals<S: Into<String>>(&self, c: S) -> Option<&Vec<Dat>> {
        if let Some(msgcmd) = self.get_cmd(&(c.into())) {
            return Some(&msgcmd.vals); 
        }
        None
    }

    pub fn get_cmd_vals_mut<S: Into<String>>(&mut self, c: S) -> Option<&mut Vec<Dat>> {
        if let Some(msgcmd) = self.get_cmd_mut(&(c.into())) {
            return Some(&mut msgcmd.vals); 
        }
        None
    }

    pub fn get_cmd_arg_vals<S: Into<String>>(
        &self,
        c: S,
        a: S,
    )
        -> Option<&Vec<Dat>>
    {
        if let Some(msgcmd) = self.get_cmd(&(c.into())) {
            return msgcmd.get_arg_vals(&(a.into())); 
        }
        None
    }

    pub fn validate(&self) -> Outcome<()> {
        let syntax = self.syntax();
        if self.vals.len() != syntax.config().vals.len() {
            return Err(err!(
                "The syntax '{}' requires {} message values, the message has {}.",
                syntax,
                syntax.config().vals.len(),
                self.vals.len();
            Input, Invalid));
        } else if self.args.len() < syntax.config().rargs.len() {
            return Err(err!(
                "The syntax '{}' requires {} message arguments, the message has {}.",
                syntax,
                syntax.config().rargs.len(),
                self.args.len();
            Input, Invalid));
        }

        res!(self.check_rargs(
            fmt!("syntax '{}' message argument", syntax), 
            &syntax.config().rargs,
            &syntax.args,
        ));

        Ok(())

    }

    fn check_rargs(
        &self,
        desc:   String,
        rargs:  &Vec<String>,
        args:   &BTreeMap<Key, Recursive<Key, Arg>>,
    )
        -> Outcome<()>
    {
        for arg_name in rargs {
            if let Some(arg) = args.get_recursive(&Key::Str(arg_name.clone())) {
                let mut found = false;
                for (k, _) in &self.args {
                    if *k == arg.canonical_name() {
                        found = true;
                        break;
                    }
                }
                if !found {
                    return Err(err!(
                        "The required {} '{}' was not found in the message.",
                        desc,
                        arg_name;
                    Input, Invalid, Missing));
                }
            }
        }

        Ok(())
    }

    // STRING IO
    //
    pub fn to_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.vals.len() > 0 {
            lines.push(fmt!("Message values [{}]:", self.vals.len()));
            for val in &self.vals {
                lines.push(fmt!("{:?}", val));
            }
        } else {
            lines.push(fmt!("No message values."));
        }
        if self.args.len() > 0 {
            for (k, vals) in &self.args {
                if vals.len() > 0 {
                    lines.push(fmt!("Message argument {:?} values [{}]:", k, self.args.len()));
                    for val in vals {
                        lines.push(fmt!("{:?}", val));
                    }
                } else {
                    lines.push(fmt!("Message argument {:?} (no values).", k));
                }
            }
        } else {
            lines.push(fmt!("No message arguments."));
        }
        if self.cmds.len() > 0 {
            for (kc, cmd) in &self.cmds {
                if cmd.vals.len() > 0 {
                    lines.push(fmt!("Command {:?} values [{}]:", kc, cmd.vals.len()));
                    for val in &cmd.vals {
                        lines.push(fmt!("{:?}", val));
                    }
                } else {
                    lines.push(fmt!("No command values."));
                }
                if cmd.args.len() > 0 {
                    for (ka, vals) in &cmd.args {
                        if vals.len() > 0 {
                            lines.push(fmt!(
                                "Command {:?} argument {:?} values [{}]:",
                                kc, ka, vals.len(),
                            ));
                            for val in vals {
                                lines.push(fmt!("{:?}", val));
                            }
                        } else {
                            lines.push(fmt!(
                                "Message command {:?} argument {:?} (no values).",
                                kc, ka,
                            ));
                        }
                    }
                } else {
                    lines.push(fmt!("No command {:?} arguments.", kc));
                }
            }
        } else {
            lines.push(fmt!("No message commands."));
        }
        lines.push(fmt!("Message end state:"));
        lines.push(fmt!(" Outstanding message value kinds: {:?}", self.end.vals));
        if let Some(cmd_name) = &self.end.cmd {
            lines.push(fmt!(" Active command: {}", cmd_name));
        } else {
            lines.push(fmt!(" No active command."));
        }
        if let Some(arg_name) = &self.end.arg {
            lines.push(fmt!(" Active message or command argument: {}", arg_name));
        } else {
            lines.push(fmt!(" No active message or command argument."));
        }
        lines
    }

    /// During processing of a text message, arguments are removed from a list of required
    /// arguments as they are received.  If no more arguments are expected and this list is not
    /// empty, an error needs to be raised indicating which required arguments were not received.
    fn check_required_txt_args(
        &self,
        rargs:      Vec<&str>,
        active_cmd: &Option<&Cmd>,
    )
        -> Outcome<()>
    {
        if rargs.len() > 0 {
            match active_cmd {
                Some(cmd) => return Err(err!(
                    "The syntax '{}' requires the presence of the arguments {:?} \
                    for command '{}', but the following were not detected before the \
                    presence of a command: {:?}",
                    self.syntax().config().name, cmd.config().rargs, cmd.config().name, rargs;
                Input, Missing)),
                None => return Err(err!(
                    "The syntax '{}' requires the presence of the commandless \
                    arguments {:?}, but the following were not detected before the \
                    presence of a command: {:?}",
                    self.syntax().config().name, self.syntax().config().rargs, rargs;
                Input, Missing)),
            }
        }
        Ok(())
    }

    fn is_word_a_cmd(
        &self,
        word:                   &Key,
        pos:                    usize,
        similarity_threshold:   Option<f64>,
    )
        -> Outcome<&Cmd>
    {
        if let Some(cmd) = self.syntax().cmds.get_recursive(word) {
            // We found it in the syntax, it's a command.
            return Ok(&cmd);
        }
        if let (Some(threshold), Key::Str(word)) = (similarity_threshold, word) {
            let names = self.syntax().cmds_in_order().into_iter()
                .map(|c| c.config().name.clone()).collect::<Vec<_>>();
            if let Some(suggestion) = Self::closest(word, &names, threshold) {
                return Err(err!(
                    "Did you mean '{}'? The word '{}' at position {} is not an argument, \
                    and neither is it a command of '{}'.",
                    suggestion, word, pos, self.syntax().config().name;
                Input, Invalid, Suggestion));
            }
        }
        Err(err!(
            "The word '{}' at position {} is not an argument, and neither is it a \
            command of '{}'.",
            word, pos, self.syntax().config().name;
        Input, Invalid))
    }

    /// The candidate nearest to the word by edit distance, if it is similar enough.
    pub fn closest(
        word:       &str,
        candidates: &[String],
        threshold:  f64,
    )
        -> Option<String>
    {
        let mut min_dist = word.chars().count();
        let mut closest = None;
        for cand in candidates {
            let dist = levenshtein::levenshtein(word, cand);
            if dist < min_dist {
                min_dist = dist;
                let ratio: f64 = 1.0 - (
                    dist as f64
                    / std::cmp::max(word.chars().count(), cand.chars().count()) as f64
                );
                if ratio > threshold {
                    closest = Some(cand.clone());
                }
            }
        }
        closest
    }

    /// If the given argument is on the given list of required arguments, the argument is removed
    /// from the list.
    fn revise_reqd_arg_list(
        arg_name: &str,
        rargs: &mut Vec<&str>,
    ) {
        for i in 0..rargs.len() {
            if rargs[i] == arg_name {
                rargs.remove(i);
                return;
            }
        }
    }

    pub fn from_str(
        &self,
        msg:                    &str,
        similarity_threshold:   Option<f64>,
    )
        -> Outcome<Self>
    {
        let iter = StringSplitter::default()
            .split(msg)
            .into_iter().map(|x| x.to_val());
        self.rx_text_iter(iter, similarity_threshold)
    }

    /// Takes a string iterator and interprets it as a syntax message.  Returns a valid message,
    /// or an error.
    pub fn rx_text_iter<I: IntoIterator<Item=String>>(
        &self,
        seq:                    I,
        similarity_threshold:   Option<f64>,
    )
        -> Outcome<Self>
    {
        self.rx_words(seq, similarity_threshold, Source::Line)
    }

    /// Reads the words of a process's command line, which a shell has already split and
    /// unquoted.  A string value is taken as it stands, and a value that is required but absent
    /// is an error rather than an outstanding value.
    pub fn rx_argv<I: IntoIterator<Item=String>>(
        &self,
        seq:                    I,
        similarity_threshold:   Option<f64>,
    )
        -> Outcome<Self>
    {
        self.rx_words(seq, similarity_threshold, Source::Argv)
    }

    /// Does the word have the shape of an option?  A dash and a letter, as in `-v` or `-abc`, or
    /// two dashes and a name, as in `--dry-run` or `--dry-run=x`.  A negative number does not, nor
    /// `-`, `--`, or a word holding a space or a stop, such as `- fix typo` or `-x.txt`: a word
    /// that could only be a value is read as one.
    pub fn looks_like_option(word: &str) -> bool {
        let name = |s: &str| {
            let mut chars = s.chars();
            match chars.next() {
                Some(c) if c.is_ascii_alphabetic() => chars.all(|c| c.is_ascii_alphanumeric()
                    || c == '-' || c == '_'),
                _ => false,
            }
        };
        if let Some(long) = word.strip_prefix("--") {
            return match long.split_once('=') {
                Some((head, _)) => name(head),
                None            => name(long),
            };
        }
        match word.strip_prefix('-') {
            Some(short) => {
                let mut chars = short.chars();
                match chars.next() {
                    Some(c) if c.is_ascii_alphabetic() => chars.all(|c| c.is_ascii_alphanumeric()),
                    _ => false,
                }
            },
            None => false,
        }
    }

    /// Is the word one of the two that ask for help?
    pub fn is_help_flag(word: &str) -> bool {
        word == "-h" || word == "--help"
    }

    /// The argument a word names, and whether it is a message argument rather than one of the
    /// active command's.  On a command line only a hyphenated name counts, so a value that
    /// happens to spell an argument's internal name stays a value.
    fn find_arg<'a>(
        syntax:     &'a Syntax,
        word:       &str,
        active_cmd: Option<&'a Cmd>,
        src:        Source,
    )
        -> Option<(&'a Arg, bool)>
    {
        if src == Source::Argv && !word.starts_with('-') {
            return None;
        }
        let key = Key::Str(word.to_string());
        match active_cmd {
            Some(cmd) => match cmd.args.get_recursive(&key) {
                Some(arg) => Some((arg, false)),
                None => if src == Source::Argv {
                    // A message option may follow the command on a command line.
                    syntax.args.get_recursive(&key).map(|arg| (arg, true))
                } else {
                    None
                },
            },
            None => syntax.args.get_recursive(&key).map(|arg| (arg, true)),
        }
    }

    fn owner_desc(
        coll:       &Collecting,
        active_cmd: Option<&Cmd>,
        active_arg: Option<&Arg>,
    )
        -> String
    {
        match coll {
            Collecting::MessageArg | Collecting::CommandArg => match active_arg {
                Some(arg) => fmt!("the option '{}'", arg.long_name()),
                None => fmt!("an option"),
            },
            Collecting::Command => match active_cmd {
                Some(cmd) => fmt!("the command '{}'", cmd.config().name),
                None => fmt!("a command"),
            },
            _ => fmt!("the message"),
        }
    }

    /// The refusal of a word a command has no value left for.
    fn excess(cmd: &Cmd, word: &str, pos: usize) -> Error<ErrTag> {
        match &cmd.config().excess {
            Some(sentence) => err!(
                "{} (found '{}' at position {}).", sentence, word, pos;
            Input, Excessive),
            None => err!(
                "The word '{}' at position {} is more than the command '{}' takes.",
                word, pos, cmd.config().name;
            Input, Excessive),
        }
    }

    fn push_val(
        msgrx:      &mut Msg,
        coll:       &Collecting,
        active_cmd: Option<&Cmd>,
        active_arg: Option<&Arg>,
        d:          Dat,
    )
        -> Outcome<()>
    {
        match coll {
            Collecting::Message => msgrx.vals.push(d),
            Collecting::MessageArg => if let Some(arg) = active_arg {
                msgrx.args.entry(arg.canonical_name()).or_insert(Vec::new()).push(d);
            },
            Collecting::Command => if let Some(cmd) = active_cmd {
                let ckey = cmd.config().name.clone();
                let entry = msgrx.cmds.entry(ckey.clone())
                    .or_insert(res!(MsgCmd::new(msgrx.syntax.clone(), ckey)));
                entry.vals.push(d);
            },
            Collecting::CommandArg => if let Some(cmd) = active_cmd {
                let ckey = cmd.config().name.clone();
                let entry = msgrx.cmds.entry(ckey.clone())
                    .or_insert(res!(MsgCmd::new(msgrx.syntax.clone(), ckey)));
                if let Some(arg) = active_arg {
                    entry.args.entry(arg.canonical_name()).or_insert(Vec::new()).push(d);
                }
            },
            Collecting::None => return Err(err!(
                "A value was read while nothing was collecting values.";
            Bug, Unexpected)),
        }
        Ok(())
    }

    /// Reads a word as a value of the expected kind.  From a command line a string is the word
    /// itself: decoding it would turn `42` into a number and `(x)` into a tuple, and leave
    /// `don't` with an unbalanced quote.
    pub fn decode_word(
        word:   &str,
        val:    &Val,
        src:    Source,
        pos:    usize,
        owner:  &str,
    )
        -> Outcome<Dat>
    {
        let kind = &val.kind;
        if src == Source::Argv && *kind == Kind::Str {
            return Ok(Dat::Str(word.to_string()));
        }
        let mut d = match Dat::decode_string(word) {
            Ok(d) => d,
            Err(e) => return Err(err!(e,
                "The word '{}' at position {}, <{}> for {}, could not be read.",
                word, pos, val.label(), owner;
            Input, Invalid, Decode)),
        };
        // Coercion may be necessary for positive values of signed kinds.
        match kind {
            Kind::I8 => if let Dat::U8(v) = d {
                d = Dat::I8(try_into!(i8, v));
            },
            Kind::I16 => match d {
                Dat::I8(v)  => d = Dat::I16(try_into!(i16, v)),
                Dat::U8(v)  => d = Dat::I16(try_into!(i16, v)),
                Dat::U16(v) => d = Dat::I16(try_into!(i16, v)),
                _ => (),
            },
            Kind::I32 => match d {
                Dat::I8(v)  => d = Dat::I32(try_into!(i32, v)),
                Dat::I16(v) => d = Dat::I32(try_into!(i32, v)),
                Dat::U8(v)  => d = Dat::I32(try_into!(i32, v)),
                Dat::U16(v) => d = Dat::I32(try_into!(i32, v)),
                Dat::U32(v) => d = Dat::I32(try_into!(i32, v)),
                _ => (),
            },
            Kind::I64 => match d {
                Dat::I8(v)  => d = Dat::I64(try_into!(i64, v)),
                Dat::I16(v) => d = Dat::I64(try_into!(i64, v)),
                Dat::I32(v) => d = Dat::I64(try_into!(i64, v)),
                Dat::U8(v)  => d = Dat::I64(try_into!(i64, v)),
                Dat::U16(v) => d = Dat::I64(try_into!(i64, v)),
                Dat::U32(v) => d = Dat::I64(try_into!(i64, v)),
                Dat::U64(v) => d = Dat::I64(try_into!(i64, v)),
                _ => (),
            },
            Kind::I128 => match d {
                Dat::I8(v)  => d = Dat::I128(try_into!(i128, v)),
                Dat::I16(v) => d = Dat::I128(try_into!(i128, v)),
                Dat::I32(v) => d = Dat::I128(try_into!(i128, v)),
                Dat::I64(v) => d = Dat::I128(try_into!(i128, v)),
                Dat::U8(v)  => d = Dat::I128(try_into!(i128, v)),
                Dat::U16(v) => d = Dat::I128(try_into!(i128, v)),
                Dat::U32(v) => d = Dat::I128(try_into!(i128, v)),
                Dat::U64(v) => d = Dat::I128(try_into!(i128, v)),
                Dat::U128(v) => d = Dat::I128(try_into!(i128, v)),
                _ => (),
            },
            Kind::U16 => match d {
                Dat::U8(v) => d = Dat::U16(try_into!(u16, v)),
                _ => (),
            },
            Kind::U32 => match d {
                Dat::U8(v)  => d = Dat::U32(try_into!(u32, v)),
                Dat::U16(v) => d = Dat::U32(try_into!(u32, v)),
                _ => (),
            },
            Kind::U64 => match d {
                Dat::U8(v)  => d = Dat::U64(try_into!(u64, v)),
                Dat::U16(v) => d = Dat::U64(try_into!(u64, v)),
                Dat::U32(v) => d = Dat::U64(try_into!(u64, v)),
                _ => (),
            },
            Kind::U128 => match d {
                Dat::U8(v)  => d = Dat::U128(try_into!(u128, v)),
                Dat::U16(v) => d = Dat::U128(try_into!(u128, v)),
                Dat::U32(v) => d = Dat::U128(try_into!(u128, v)),
                Dat::U64(v) => d = Dat::U128(try_into!(u128, v)),
                _ => (),
            },
            Kind::F64 => match d {
                Dat::U8(v)  => d = Dat::F64(Float64(v as f64)),
                Dat::U16(v) => d = Dat::F64(Float64(v as f64)),
                Dat::U32(v) => d = Dat::F64(Float64(v as f64)),
                Dat::U64(v) => d = Dat::F64(Float64(v as f64)),
                Dat::I8(v)  => d = Dat::F64(Float64(v as f64)),
                Dat::I16(v) => d = Dat::F64(Float64(v as f64)),
                Dat::I32(v) => d = Dat::F64(Float64(v as f64)),
                Dat::I64(v) => d = Dat::F64(Float64(v as f64)),
                Dat::F32(v) => d = Dat::F64(Float64(v.0 as f64)),
                Dat::Adec(v) => {
                    // Convert BigDecimal to f64.
                    match v.to_f64() {
                        Some(f) => d = Dat::F64(Float64(f)),
                        None => return Err(err!(
                            "Cannot convert BigDecimal '{}' to f64: value out of range", v;
                            Input, Invalid, Conversion)),
                    }
                },
                _ => (),
            },
            _ => (),
        }
        if *kind == d.kind() || *kind == Kind::Unknown {
            return Ok(d);
        }
        match res!(Self::coerce_to_expected_kind(kind, &d)) {
            Some(converted) => Ok(converted),
            None => Err(err!(
                "The word '{}' at position {}, <{}> for {}, must be a {} but reads as a {}.",
                word, pos, val.label(), owner, kind, d.kind();
            Input, Invalid)),
        }
    }

    /// Interprets a sequence of words as a message of the syntax.
    pub fn rx_words<I: IntoIterator<Item=String>>(
        &self,
        seq:                    I,
        similarity_threshold:   Option<f64>,
        src:                    Source,
    )
        -> Outcome<Self>
    {
        let syntax = self.syntax();
        let sname = syntax.config().name.clone();
        let argv = src == Source::Argv;
        let one_cmd = syntax.config().one_cmd;
        let mut msgrx = Msg::new(self.syntaxref());
        let mut active_cmd: Option<&Cmd> = None;
        let mut active_arg: Option<&Arg> = None;
        let mut msg_slots = Slots::new(&syntax.config().vals);
        let mut cmd_slots = Slots::new(&[]);
        let mut arg_slots = Slots::new(&[]);
        let mut collecting = if syntax.config().vals.is_empty() {
            Collecting::None
        } else {
            Collecting::Message
        };
        let mut rargs: Vec<&str> = syntax.config().rargs.iter().map(|s| s.as_str()).collect();
        let mut in_rest = false;
        let mut opts_done = false; // past a "--" that ends the options

        for (i, word) in seq.into_iter().enumerate() {
            let pos = i + 1;

            // Everything after "--" is the command's rest, whatever it looks like.
            if in_rest {
                if let Some(cmd) = active_cmd {
                    if let Some(rest) = &cmd.config().rest {
                        let d = res!(Self::decode_word(
                            &word, rest, src, pos, &fmt!("the command '{}'", cmd.config().name)));
                        if let Some(msgcmd) = msgrx.cmds.get_mut(&cmd.config().name) {
                            msgcmd.rest.get_or_insert_with(Vec::new).push(d);
                        }
                    }
                }
                continue;
            }

            // An option still taking values, whose next word the VAL block decides.
            let opt_open = matches!(collecting, Collecting::MessageArg | Collecting::CommandArg)
                && arg_slots.current().is_some();

            if argv && !opt_open {
                if let Some(cmd) = active_cmd {
                    let owner = fmt!("the command '{}'", cmd.config().name);
                    match cmd_slots.current() {
                        // After a "--" that ends the options, and once a repeating verbatim
                        // value is being filled, every word is a value as it stands.
                        Some(val) if opts_done
                            || (val.verbatim && val.arity.repeats() && cmd_slots.begun()) =>
                        {
                            let d = res!(Self::decode_word(&word, val, src, pos, &owner));
                            collecting = Collecting::Command;
                            active_arg = None;
                            res!(Self::push_val(&mut msgrx, &collecting, active_cmd, None, d));
                            cmd_slots.accept();
                            continue;
                        },
                        None if opts_done => return Err(Self::excess(cmd, &word, pos)),
                        _ => (),
                    }
                }
            }

            let word_key = Key::Str(word.clone());
            let is_rest_mark = word == "--"
                && active_cmd.map_or(false, |c| c.config().rest.is_some());
            let found_arg = Self::find_arg(syntax, &word, active_cmd, src);
            // With one command per message, a command's name after the command is a value.
            let is_cmd_word = syntax.cmds.contains_key(&word_key)
                && !(one_cmd && active_cmd.is_some());
            let is_help = argv && found_arg.is_none() && Self::is_help_flag(&word);
            let dash_dash = argv && word == "--";
            // A verbatim value takes a word shaped like an option that is not one of the
            // command's own, a help flag or a "--".
            let as_typed = argv && active_cmd.is_some() && !opt_open && !is_help && !dash_dash
                && cmd_slots.current().map_or(false, |v| v.verbatim);
            let dashy = argv && found_arg.is_none() && Self::looks_like_option(&word) && !as_typed;

            // VAL block
            if collecting != Collecting::None {
                let for_arg = matches!(collecting, Collecting::MessageArg | Collecting::CommandArg);
                let slots = match collecting {
                    Collecting::Message => &mut msg_slots,
                    Collecting::Command => &mut cmd_slots,
                    _                   => &mut arg_slots,
                };
                match slots.current() {
                    Some(val) => {
                        // An option's value that is still owed takes a word that merely looks
                        // like an option, such as '--reason -x', and "--" and a help flag too.
                        let owed = for_arg && !slots.satisfied();
                        let marker = found_arg.is_some()
                            || is_cmd_word
                            || is_rest_mark
                            || ((dashy || dash_dash) && !owed);
                        if !marker {
                            let owner = Self::owner_desc(&collecting, active_cmd, active_arg);
                            let d = res!(Self::decode_word(&word, val, src, pos, &owner));
                            res!(Self::push_val(&mut msgrx, &collecting, active_cmd, active_arg, d));
                            slots.accept();
                            continue;
                        }
                        if slots.satisfied() || (one_cmd && collecting == Collecting::Command)
                            || is_help || dash_dash
                        {
                            // The values may stop here, or, with one command per message, be
                            // given after the options. The word is read below.
                            if for_arg {
                                active_arg = None;
                            }
                            collecting = Collecting::None;
                        } else {
                            let owner = Self::owner_desc(&collecting, active_cmd, active_arg);
                            let wanted = slots.wanting().unwrap_or(val);
                            return Err(match &wanted.missing {
                                Some(sentence) => err!(
                                    "{} (found '{}' at position {}).", sentence, word, pos;
                                Input, Missing),
                                None => err!(
                                    "The syntax '{}' expects <{}> for {} at position {}, but \
                                    found '{}'.",
                                    sname, wanted.label(), owner, pos, word;
                                Input, Missing),
                            });
                        }
                    },
                    None => {
                        // Expected values exhausted.
                        collecting = Collecting::None;
                        active_arg = None;
                    },
                }
            }

            if is_help {
                // Asked where an option could stand, so the page is the answer, whatever the
                // rest of the line holds.
                msgrx.end = MsgEndState {
                    cmd:    active_cmd.map(|c| c.config().name.clone()),
                    help:   true,
                    ..Default::default()
                };
                return Ok(msgrx);
            }

            if is_rest_mark {
                in_rest = true;
                if let Some(cmd) = active_cmd {
                    if let Some(msgcmd) = msgrx.cmds.get_mut(&cmd.config().name) {
                        msgcmd.rest = Some(Vec::new());
                    }
                }
                continue;
            }

            // ARG block
            if let Some((arg, is_msg_arg)) = found_arg {
                let canon = arg.canonical_name();
                if is_msg_arg {
                    if msgrx.args.contains_key(&canon) {
                        return Err(err!(
                            "The message argument '{}' at position {} in the syntax '{}' \
                            has already been given.",
                            word, pos, sname;
                        Input, Invalid));
                    }
                    msgrx.args.insert(canon.clone(), Vec::new());
                    collecting = Collecting::MessageArg;
                } else if let Some(cmd) = active_cmd {
                    if let Some(cmdrx) = msgrx.get_cmd_mut(&cmd.config().name) {
                        if cmdrx.has_arg(&canon) {
                            return Err(err!(
                                "The argument '{}' at position {} for command '{}' in the \
                                syntax '{}' has already been given.",
                                word, pos, cmd.config().name, sname;
                            Input, Invalid));
                        }
                        cmdrx.args.insert(canon.clone(), Vec::new());
                    }
                    collecting = Collecting::CommandArg;
                }
                Self::revise_reqd_arg_list(&canon, &mut rargs);
                active_arg = Some(arg);
                arg_slots = Slots::new(&arg.config().vals);
                continue;
            }

            if dash_dash {
                // The end of the options: every word after it is one of the command's values.
                if active_cmd.is_none() {
                    return Err(err!(
                        "The '--' at position {} ends the options, but no command has been \
                        named.", pos;
                    Input, Invalid));
                }
                opts_done = true;
                collecting = Collecting::Command;
                active_arg = None;
                continue;
            }

            if dashy {
                let mut names = Vec::new();
                if let Some(cmd) = active_cmd {
                    for arg in cmd.args_in_order() {
                        names.append(&mut arg.hyphenated_names());
                    }
                }
                for arg in syntax.args_in_order() {
                    names.append(&mut arg.hyphenated_names());
                }
                let whose = match active_cmd {
                    Some(cmd) => fmt!("the command '{}'", cmd.config().name),
                    None => fmt!("'{}'", sname),
                };
                return Err(match similarity_threshold
                    .and_then(|t| Self::closest(&word, &names, t))
                {
                    Some(suggestion) => err!(
                        "Did you mean '{}'? The option '{}' at position {} is not one \
                        that {} takes.",
                        suggestion, word, pos, whose;
                    Input, Invalid, Suggestion),
                    None => err!(
                        "The option '{}' at position {} is not one that {} takes.",
                        word, pos, whose;
                    Input, Invalid),
                });
            }

            if one_cmd {
                if let Some(cmd) = active_cmd {
                    // Values may come after options, so a bare word fills the next one.
                    if let Some(val) = cmd_slots.current() {
                        let owner = fmt!("the command '{}'", cmd.config().name);
                        let d = res!(Self::decode_word(&word, val, src, pos, &owner));
                        collecting = Collecting::Command;
                        res!(Self::push_val(&mut msgrx, &collecting, active_cmd, None, d));
                        cmd_slots.accept();
                        continue;
                    }
                    return Err(Self::excess(cmd, &word, pos));
                }
            }

            // CMD block
            let cmd = res!(self.is_word_a_cmd(&word_key, pos, similarity_threshold));
            msgrx.cmds.insert(
                cmd.config().name.clone(),
                res!(MsgCmd::new(self.syntaxref(), cmd.config().name.clone())),
            );
            active_cmd = Some(cmd);
            active_arg = None;
            collecting = Collecting::Command;
            cmd_slots = Slots::new(&cmd.config().vals);
            res!(self.check_required_txt_args(rargs, &None));
            // Reset rargs for the command.
            rargs = cmd.config().rargs.iter().map(|s| s.as_str()).collect();
        }

        if argv {
            // From a command line nothing more is coming, so what is owed is missing.
            let mut owed: Vec<(&Val, String)> = Vec::new();
            if let Some(arg) = active_arg {
                if let Some(val) = arg_slots.wanting() {
                    owed.push((val, fmt!("the option '{}'", arg.long_name())));
                }
            }
            if let Some(val) = msg_slots.wanting() {
                owed.push((val, fmt!("'{}'", sname)));
            }
            if let Some(cmd) = active_cmd {
                if let Some(val) = cmd_slots.wanting() {
                    owed.push((val, fmt!("the command '{}'", cmd.config().name)));
                }
                if let (Some(rest), Some(msgcmd)) =
                    (&cmd.config().rest, msgrx.cmds.get(&cmd.config().name))
                {
                    if let Some(words) = &msgcmd.rest {
                        if words.is_empty() && !rest.arity.may_be_empty() {
                            owed.push((rest, fmt!("the command '{}' after '--'", cmd.config().name)));
                        }
                    }
                }
            }
            if let Some((val, owner)) = owed.first() {
                return Err(match &val.missing {
                    Some(sentence) => err!("{}", sentence; Input, Missing),
                    None => err!("{} needs <{}>.", Self::capitalise(owner), val.label();
                        Input, Missing),
                });
            }
        }

        res!(self.check_required_txt_args(rargs, &active_cmd));

        msgrx.end = MsgEndState {
            vals:   match collecting {
                Collecting::Message     => msg_slots.outstanding(),
                Collecting::Command     => cmd_slots.outstanding(),
                Collecting::MessageArg |
                Collecting::CommandArg  => arg_slots.outstanding(),
                Collecting::None        => Vec::new(),
            },
            arg:    active_arg.map(|a| a.canonical_name()),
            cmd:    active_cmd.map(|c| c.config().name.clone()),
            help:   false,
        };
        msgrx.enc = Encoding::UTF8;

        Ok(msgrx)
    }

    fn capitalise(s: &str) -> String {
        let mut chars = s.chars();
        match chars.next() {
            Some(c) => c.to_uppercase().chain(chars).collect(),
            None => String::new(),
        }
    }

    // BINARY IO
    //
    /// Decodes and validates a syntax `Msg`.  A message is prepended with the encoding type,
    /// currently either a UTF-8 string or a custom binary format.
    /// ````ignore
    ///    +--- Encoding variant (u8)
    ///    |          +--- encoded message, either UTF-8 or binary
    ///    |   _______|______
    ///    v  /              \
    ///  +---+---+---+---+---+
    ///  |   |   |  ...  |   |
    ///  +---+---+---+---+---+
    /// ````
    /// # Binary message format
    /// ````ignore
    ///    +-- number of message values (Dat::C64)
    ///    |                   +-- number of message arguments (C64)
    ///    |   message values  |   +-- index of first msg arg (U16)
    ///    |     (Dats)    |   |   +-- number of msg arg vals for i1
    ///    |   ______|______   |   |   |
    ///    v  /             \  v   v   v
    ///  +---+---+---+---+---+---+---+---+---+---+---+---+---+---+--
    ///  | v |   v msg vals  | a |i1 | n |  n arg vals   |i2 | n | ..
    ///  +---+---+---+---+---+---+---+---+---+---+---+---+---+---+--
    ///
    ///                   number of commands --+   +-- index of first cmd
    ///                                        |   |
    ///                                        v   v
    /// -+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+--
    ///  n arg vals  |ia | n |  n arg vals   | c |i1 | v |  v cmd vals
    /// -+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+--
    ///
    ///    +-- number of cmd args for cmd i1
    ///    |   +-- index of first cmd arg for cmd i1
    ///    |   |   +-- number of cmd arg vals or cmd i1
    ///    |   |   |
    ///    v   v   v
    /// -+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+--
    ///  | a |j1 | n |  n arg vals   |j2 | n |  n arg vals   |ja | n |
    /// -+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+--
    ///
    ///
    ///
    /// -+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+--
    ///  n arg vals  |i2 | v |  v cmd vals   | a |j1 | n |  n cmd vals ...
    /// -+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+--
    ///
    /// .. and so on for all remaining commands
    /// ````
    ///
    pub fn from_bytes(
        &self,
        buf:                    &[u8],
        similarity_threshold:   Option<f64>,
    )
        -> Outcome<Self>
    {
        if buf.len() <= 1 {
            return Err(err!("No bytes to decode."; Input, Missing));
        }
        let msgrx = match Encoding::from(buf[0]) {
            Encoding::Binary => res!(self.from_bytes_binary(&buf[1..])),
            Encoding::UTF8 => {
                let msg_str = res!(std::str::from_utf8(&buf[1..]), Decode, UTF8);
                res!(self.from_str(msg_str, similarity_threshold))
            },
            _ => return Err(err!("Unknown message encoding."; Unknown, Encode)),
        };
        Ok(msgrx)
    }

    pub fn from_bytes_binary(
        &self,
        buf: &[u8],
    )
        -> Outcome<Self>
    {
        let mut msgrx = Msg::new(self.syntaxref());
        let mut n: usize = 0;
        let mut last_arg: Option<String>;
        let mut last_cmd: Option<String> = None;
        let mut rargs: Vec<&str> = Vec::new();
        for arg_name in &self.syntax().config().rargs {
            rargs.push(arg_name);   
        }
        // Message values
        let (dat, nb) = res!(Dat::from_bytes(&buf));
        debug!("Daticle from bytes: {:?}", dat);
        n += nb;
        let vals = try_extract_dat!(dat, List);
        res!(self.check_expected_vals(
            &self.syntax().config().vals,
            &vals,
            &None,
        ));
        msgrx.vals = vals;
        // Message arguments
        let (args_map, nb, last_arg2) = res!(self.rx_binary_args(
            Collecting::MessageArg,
            &self.syntax().config().rargs,
            &self.syntax().args,
            &buf[n..],
            &None,
        ));
        n += nb;
        res!(self.check_required_bin_args(
            &self.syntax().config().rargs,
            &args_map.iter().map(|(k, _)| k.clone()).collect::<Vec<String>>(),
            &None,
        ));
        msgrx.args = args_map;
        last_arg = last_arg2;
        // Commands
        if let (Dat::C64(c), nb) = res!(Dat::from_bytes(&buf[n..])) {
            n += nb;
            let ncmd = c as usize; 
            for i in 0..ncmd {
                if let (Dat::U16(cmd_id), nb) = res!(Dat::from_bytes(&buf[n..])) {
                    n += nb;
                    if let Some(cmd) = self.syntax().cmds.get_recursive(&Key::Id(cmd_id)) {
                        let mut msgcmd = res!(MsgCmd::new(
                            self.syntaxref(),
                            cmd.config().name.clone(),
                        ));
                        // Command values
                        if let (Dat::List(vals), nb) = res!(Dat::from_bytes(&buf[n..])) {
                            n += nb;
                            res!(self.check_expected_vals(
                                &cmd.config().vals,
                                &vals,
                                &Some(&cmd),
                            ));
                            msgcmd.vals = vals;
                        }
                        // Command arguments
                        let (map, nb, last_arg2) = res!(self.rx_binary_args(
                            Collecting::CommandArg,
                            &cmd.config().rargs,
                            &cmd.args,
                            &buf[n..],
                            &Some(&cmd),
                        ));
                        n += nb;
                        msgcmd.args = map;
                        last_arg = last_arg2;
                        if cmd.config().rest.is_some() {
                            let (dat, nb) = res!(Dat::from_bytes(&buf[n..]));
                            n += nb;
                            if let Dat::Bool(true) = dat {
                                let (dat, nb) = res!(Dat::from_bytes(&buf[n..]));
                                n += nb;
                                msgcmd.rest = Some(try_extract_dat!(dat, List));
                            }
                        }
                        msgrx.cmds.insert(cmd.config().name.clone(), msgcmd);
                        if i == ncmd - 1 {
                            last_cmd = Some(cmd.config().name.clone());
                        }
                    } else {
                        return Err(err!(
                            "The binary message refers to a command with \
                            id {} but no such command exists in the '{}' syntax.",
                            cmd_id, self.syntax().config().name;
                        Input, Invalid));
                    }
                }
            }
        }

        msgrx.end = MsgEndState {
            arg:    last_arg,
            cmd:    last_cmd,
            ..Default::default()
        };
        msgrx.enc = Encoding::Binary;

        Ok(msgrx)
    }

    fn rx_binary_args(
        &self,
        coll:       Collecting,
        rargs:      &Vec<String>,
        args:       &BTreeMap<Key, Recursive<Key, Arg>>,
        buf:        &[u8],
        active_cmd: &Option<&Cmd>,
    )
        -> Outcome<(
            BTreeMap<String, Vec<Dat>>,
            usize,
            Option<String>,
        )>
    {
        let mut n: usize = 0;
        let mut last_arg: Option<String> = None;
        let (dat, nb) = res!(Dat::from_bytes(&buf));
        n += nb;
        let a = try_extract_dat!(dat, C64);
        let narg = a as usize;
        if narg < rargs.len() {
            return Err(err!(
                "The syntax '{}' requires {} {} argument(s), found {}.",
                self.syntax().config().name,
                rargs.len(),
                Self::msg_or_cmd_string(active_cmd),
                narg;
            Input, Missing));
        }
        let mut map = BTreeMap::new();
        for i in 0..narg {
            let (dat, nb) = res!(Dat::from_bytes(&buf[n..]));
            n += nb;
            let arg_id = try_extract_dat!(dat, U16);
            if let Some(arg) = args.get_recursive(&Key::Id(arg_id)) {

                // Collect the argument values.
                let (dat, nb) = res!(Dat::from_bytes(&buf[n..]));
                n += nb;
                let vals = try_extract_dat!(dat, List);
                res!(self.check_expected_vals(
                    &arg.config().vals,
                    &vals,
                    active_cmd,
                ));

                // Insert the values into the map.
                map.insert(arg.canonical_name(), vals);
                
                // Remember the last argument for return to the caller.
                if i == narg - 1 {
                    last_arg = Some(arg.canonical_name());
                }
            } else {
                return Err(err!(
                    "The binary message refers to a {:?} with \
                    id {} but no such argument exists in the syntax '{}'.",
                    coll, arg_id, self.syntax().config().name;
                Input, Invalid));
            }
        }
        res!(self.check_required_bin_args(
            rargs,
            &map.iter().map(|(k, _)| k.clone()).collect::<Vec<String>>(),
            active_cmd,
        ));
        Ok((map, n, last_arg))
    }

    /// Unlike text messages, binary messages allow us to know up-front what is coming, so we can
    /// check required arguments differently.
    fn check_required_bin_args(
        &self,
        required_args:  &Vec<String>,
        actual_args:    &Vec<String>,
        active_cmd:     &Option<&Cmd>,
    )
        -> Outcome<()>
    {
        let mut required_args: BTreeSet<String> =
            required_args.clone().into_iter().collect();
        for arg in actual_args {
            required_args.remove(arg);
        }
        if required_args.len() > 0 {
            return Err(err!(
                "The syntax '{}' expects {} arguments '{:?}' which \
                were not found.",
                self.syntax().config().name,
                Self::msg_or_cmd_string(active_cmd),
                required_args;
            Input, Missing));
        }
        Ok(())
    }

    fn check_expected_vals(
        &self,
        expected_vals:  &Vec<Val>,
        actual_vals:    &Vec<Dat>,
        active_cmd:     &Option<&Cmd>,
    )
        -> Outcome<()>
    {
        if !Val::count_fits(expected_vals, actual_vals.len()) {
            return Err(err!(
                "The syntax '{}' expects {} {} value(s), found {}.",
                self.syntax().config().name,
                expected_vals.len(),
                Self::msg_or_cmd_string(active_cmd),
                actual_vals.len();
            Input, Missing));
        }
        for (i, actual) in actual_vals.iter().enumerate() {
            if let Some(expected) = Val::slot(expected_vals, i) {
                if expected.kind != Kind::Unknown && expected.kind != actual.kind() {
                    return Err(err!(
                        "The syntax '{}' expects {} value {} to be a '{:?}', \
                        {:?} was found.",
                        self.syntax().config().name,
                        Self::msg_or_cmd_string(active_cmd),
                        actual,
                        expected.kind,
                        actual.kind();
                    Input, Missing));
                }
            }
        }
        Ok(())
    }

    fn msg_or_cmd_string(active_cmd: &Option<&Cmd>) -> String {
        match active_cmd {
            Some(cmd) => fmt!("'{}'", cmd.config().name),
            None => fmt!("message"),
        }
    }

    /// Coerce a Dat value to an expected Kind.
    /// 
    /// This handles conversions from default JDAT string parsing to specific typed variants.
    /// For example, "(2,6)" is parsed as Dat::Tup2(Box<[Dat; 2]>) but may need to become
    /// Dat::Tup2u8([u8; 2]) based on the syntax specification.
    fn coerce_to_expected_kind(expected: &Kind, actual: &Dat) -> Outcome<Option<Dat>> {
        match (expected, actual) {
            // Tup2 conversions.
            (Kind::Tup2u8, Dat::Tup2(boxed)) => {
                if let (Some(a), Some(b)) = (boxed[0].get_u8(), boxed[1].get_u8()) {
                    Ok(Some(Dat::Tup2u8([a, b])))
                } else {
                    Ok(None)
                }
            }
            (Kind::Tup2u16, Dat::Tup2(boxed)) => {
                if let (Some(a), Some(b)) = (boxed[0].get_u16(), boxed[1].get_u16()) {
                    Ok(Some(Dat::Tup2u16([a, b])))
                } else {
                    Ok(None)
                }
            }
            (Kind::Tup2u32, Dat::Tup2(boxed)) => {
                if let (Some(a), Some(b)) = (boxed[0].get_u32(), boxed[1].get_u32()) {
                    Ok(Some(Dat::Tup2u32([a, b])))
                } else {
                    Ok(None)
                }
            }
            (Kind::Tup2u64, Dat::Tup2(boxed)) => {
                if let (Some(a), Some(b)) = (boxed[0].get_u64(), boxed[1].get_u64()) {
                    Ok(Some(Dat::Tup2u64([a, b])))
                } else {
                    Ok(None)
                }
            }
            (Kind::Tup2i8, Dat::Tup2(boxed)) => {
                if let (Some(a), Some(b)) = (boxed[0].get_i8(), boxed[1].get_i8()) {
                    Ok(Some(Dat::Tup2i8([a, b])))
                } else {
                    Ok(None)
                }
            }
            (Kind::Tup2i16, Dat::Tup2(boxed)) => {
                if let (Some(a), Some(b)) = (boxed[0].get_i16(), boxed[1].get_i16()) {
                    Ok(Some(Dat::Tup2i16([a, b])))
                } else {
                    Ok(None)
                }
            }
            (Kind::Tup2i32, Dat::Tup2(boxed)) => {
                if let (Some(a), Some(b)) = (boxed[0].get_i32(), boxed[1].get_i32()) {
                    Ok(Some(Dat::Tup2i32([a, b])))
                } else {
                    Ok(None)
                }
            }
            (Kind::Tup2i64, Dat::Tup2(boxed)) => {
                if let (Some(a), Some(b)) = (boxed[0].get_i64(), boxed[1].get_i64()) {
                    Ok(Some(Dat::Tup2i64([a, b])))
                } else {
                    Ok(None)
                }
            }
            
            // Tup3 conversions.
            (Kind::Tup3u8, Dat::Tup3(boxed)) => {
                if let (Some(a), Some(b), Some(c)) = 
                    (boxed[0].get_u8(), boxed[1].get_u8(), boxed[2].get_u8()) {
                    Ok(Some(Dat::Tup3u8([a, b, c])))
                } else {
                    Ok(None)
                }
            }
            (Kind::Tup3u16, Dat::Tup3(boxed)) => {
                if let (Some(a), Some(b), Some(c)) = 
                    (boxed[0].get_u16(), boxed[1].get_u16(), boxed[2].get_u16()) {
                    Ok(Some(Dat::Tup3u16([a, b, c])))
                } else {
                    Ok(None)
                }
            }
            (Kind::Tup3u32, Dat::Tup3(boxed)) => {
                if let (Some(a), Some(b), Some(c)) = 
                    (boxed[0].get_u32(), boxed[1].get_u32(), boxed[2].get_u32()) {
                    Ok(Some(Dat::Tup3u32([a, b, c])))
                } else {
                    Ok(None)
                }
            }
            (Kind::Tup3u64, Dat::Tup3(boxed)) => {
                if let (Some(a), Some(b), Some(c)) = 
                    (boxed[0].get_u64(), boxed[1].get_u64(), boxed[2].get_u64()) {
                    Ok(Some(Dat::Tup3u64([a, b, c])))
                } else {
                    Ok(None)
                }
            }
            (Kind::Tup3i8, Dat::Tup3(boxed)) => {
                if let (Some(a), Some(b), Some(c)) = 
                    (boxed[0].get_i8(), boxed[1].get_i8(), boxed[2].get_i8()) {
                    Ok(Some(Dat::Tup3i8([a, b, c])))
                } else {
                    Ok(None)
                }
            }
            (Kind::Tup3i16, Dat::Tup3(boxed)) => {
                if let (Some(a), Some(b), Some(c)) = 
                    (boxed[0].get_i16(), boxed[1].get_i16(), boxed[2].get_i16()) {
                    Ok(Some(Dat::Tup3i16([a, b, c])))
                } else {
                    Ok(None)
                }
            }
            (Kind::Tup3i32, Dat::Tup3(boxed)) => {
                if let (Some(a), Some(b), Some(c)) = 
                    (boxed[0].get_i32(), boxed[1].get_i32(), boxed[2].get_i32()) {
                    Ok(Some(Dat::Tup3i32([a, b, c])))
                } else {
                    Ok(None)
                }
            }
            (Kind::Tup3i64, Dat::Tup3(boxed)) => {
                if let (Some(a), Some(b), Some(c)) = 
                    (boxed[0].get_i64(), boxed[1].get_i64(), boxed[2].get_i64()) {
                    Ok(Some(Dat::Tup3i64([a, b, c])))
                } else {
                    Ok(None)
                }
            }
            
            // Tup4 conversions.
            (Kind::Tup4u8, Dat::Tup4(boxed)) => {
                match Self::extract_u8_array::<4>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup4u8(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup4u16, Dat::Tup4(boxed)) => {
                match Self::extract_u16_array::<4>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup4u16(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup4u32, Dat::Tup4(boxed)) => {
                match Self::extract_u32_array::<4>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup4u32(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup4u64, Dat::Tup4(boxed)) => {
                match Self::extract_u64_array::<4>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup4u64(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup4i8, Dat::Tup4(boxed)) => {
                match Self::extract_i8_array::<4>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup4i8(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup4i16, Dat::Tup4(boxed)) => {
                match Self::extract_i16_array::<4>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup4i16(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup4i32, Dat::Tup4(boxed)) => {
                match Self::extract_i32_array::<4>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup4i32(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup4i64, Dat::Tup4(boxed)) => {
                match Self::extract_i64_array::<4>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup4i64(arr))),
                    None => Ok(None)
                }
            }
            
            // Tup5 conversions.
            (Kind::Tup5u8, Dat::Tup5(boxed)) => {
                match Self::extract_u8_array::<5>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup5u8(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup5u16, Dat::Tup5(boxed)) => {
                match Self::extract_u16_array::<5>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup5u16(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup5u32, Dat::Tup5(boxed)) => {
                match Self::extract_u32_array::<5>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup5u32(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup5u64, Dat::Tup5(boxed)) => {
                match Self::extract_u64_array::<5>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup5u64(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup5i8, Dat::Tup5(boxed)) => {
                match Self::extract_i8_array::<5>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup5i8(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup5i16, Dat::Tup5(boxed)) => {
                match Self::extract_i16_array::<5>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup5i16(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup5i32, Dat::Tup5(boxed)) => {
                match Self::extract_i32_array::<5>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup5i32(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup5i64, Dat::Tup5(boxed)) => {
                match Self::extract_i64_array::<5>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup5i64(arr))),
                    None => Ok(None)
                }
            }
            
            // Tup6 conversions.
            (Kind::Tup6u8, Dat::Tup6(boxed)) => {
                match Self::extract_u8_array::<6>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup6u8(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup6u16, Dat::Tup6(boxed)) => {
                match Self::extract_u16_array::<6>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup6u16(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup6u32, Dat::Tup6(boxed)) => {
                match Self::extract_u32_array::<6>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup6u32(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup6u64, Dat::Tup6(boxed)) => {
                match Self::extract_u64_array::<6>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup6u64(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup6i8, Dat::Tup6(boxed)) => {
                match Self::extract_i8_array::<6>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup6i8(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup6i16, Dat::Tup6(boxed)) => {
                match Self::extract_i16_array::<6>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup6i16(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup6i32, Dat::Tup6(boxed)) => {
                match Self::extract_i32_array::<6>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup6i32(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup6i64, Dat::Tup6(boxed)) => {
                match Self::extract_i64_array::<6>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup6i64(arr))),
                    None => Ok(None)
                }
            }
            
            // Tup7 conversions.
            (Kind::Tup7u8, Dat::Tup7(boxed)) => {
                match Self::extract_u8_array::<7>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup7u8(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup7u16, Dat::Tup7(boxed)) => {
                match Self::extract_u16_array::<7>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup7u16(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup7u32, Dat::Tup7(boxed)) => {
                match Self::extract_u32_array::<7>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup7u32(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup7u64, Dat::Tup7(boxed)) => {
                match Self::extract_u64_array::<7>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup7u64(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup7i8, Dat::Tup7(boxed)) => {
                match Self::extract_i8_array::<7>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup7i8(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup7i16, Dat::Tup7(boxed)) => {
                match Self::extract_i16_array::<7>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup7i16(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup7i32, Dat::Tup7(boxed)) => {
                match Self::extract_i32_array::<7>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup7i32(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup7i64, Dat::Tup7(boxed)) => {
                match Self::extract_i64_array::<7>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup7i64(arr))),
                    None => Ok(None)
                }
            }
            
            // Tup8 conversions.
            (Kind::Tup8u8, Dat::Tup8(boxed)) => {
                match Self::extract_u8_array::<8>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup8u8(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup8u16, Dat::Tup8(boxed)) => {
                match Self::extract_u16_array::<8>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup8u16(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup8u32, Dat::Tup8(boxed)) => {
                match Self::extract_u32_array::<8>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup8u32(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup8u64, Dat::Tup8(boxed)) => {
                match Self::extract_u64_array::<8>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup8u64(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup8i8, Dat::Tup8(boxed)) => {
                match Self::extract_i8_array::<8>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup8i8(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup8i16, Dat::Tup8(boxed)) => {
                match Self::extract_i16_array::<8>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup8i16(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup8i32, Dat::Tup8(boxed)) => {
                match Self::extract_i32_array::<8>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup8i32(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup8i64, Dat::Tup8(boxed)) => {
                match Self::extract_i64_array::<8>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup8i64(arr))),
                    None => Ok(None)
                }
            }
            
            // Tup9 conversions.
            (Kind::Tup9u8, Dat::Tup9(boxed)) => {
                match Self::extract_u8_array::<9>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup9u8(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup9u16, Dat::Tup9(boxed)) => {
                match Self::extract_u16_array::<9>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup9u16(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup9u32, Dat::Tup9(boxed)) => {
                match Self::extract_u32_array::<9>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup9u32(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup9u64, Dat::Tup9(boxed)) => {
                match Self::extract_u64_array::<9>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup9u64(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup9i8, Dat::Tup9(boxed)) => {
                match Self::extract_i8_array::<9>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup9i8(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup9i16, Dat::Tup9(boxed)) => {
                match Self::extract_i16_array::<9>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup9i16(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup9i32, Dat::Tup9(boxed)) => {
                match Self::extract_i32_array::<9>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup9i32(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup9i64, Dat::Tup9(boxed)) => {
                match Self::extract_i64_array::<9>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup9i64(arr))),
                    None => Ok(None)
                }
            }
            
            // Tup10 conversions.
            (Kind::Tup10u8, Dat::Tup10(boxed)) => {
                match Self::extract_u8_array::<10>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup10u8(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup10u16, Dat::Tup10(boxed)) => {
                match Self::extract_u16_array::<10>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup10u16(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup10u32, Dat::Tup10(boxed)) => {
                match Self::extract_u32_array::<10>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup10u32(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup10u64, Dat::Tup10(boxed)) => {
                match Self::extract_u64_array::<10>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup10u64(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup10i8, Dat::Tup10(boxed)) => {
                match Self::extract_i8_array::<10>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup10i8(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup10i16, Dat::Tup10(boxed)) => {
                match Self::extract_i16_array::<10>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup10i16(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup10i32, Dat::Tup10(boxed)) => {
                match Self::extract_i32_array::<10>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup10i32(arr))),
                    None => Ok(None)
                }
            }
            (Kind::Tup10i64, Dat::Tup10(boxed)) => {
                match Self::extract_i64_array::<10>(boxed) {
                    Some(arr) => Ok(Some(Dat::Tup10i64(arr))),
                    None => Ok(None)
                }
            }
            
            // No conversion needed or possible.
            _ => Ok(None)
        }
    }

    // Helper functions to extract typed arrays from Dat arrays.
    fn extract_u8_array<const N: usize>(dats: &[Dat; N]) -> Option<[u8; N]> {
        let mut result = [0u8; N];
        for i in 0..N {
            result[i] = ok!(dats[i].get_u8());
        }
        Some(result)
    }

    fn extract_u16_array<const N: usize>(dats: &[Dat; N]) -> Option<[u16; N]> {
        let mut result = [0u16; N];
        for i in 0..N {
            result[i] = ok!(dats[i].get_u16());
        }
        Some(result)
    }

    fn extract_u32_array<const N: usize>(dats: &[Dat; N]) -> Option<[u32; N]> {
        let mut result = [0u32; N];
        for i in 0..N {
            result[i] = ok!(dats[i].get_u32());
        }
        Some(result)
    }

    fn extract_u64_array<const N: usize>(dats: &[Dat; N]) -> Option<[u64; N]> {
        let mut result = [0u64; N];
        for i in 0..N {
            result[i] = ok!(dats[i].get_u64());
        }
        Some(result)
    }

    fn extract_i8_array<const N: usize>(dats: &[Dat; N]) -> Option<[i8; N]> {
        let mut result = [0i8; N];
        for i in 0..N {
            result[i] = ok!(dats[i].get_i8());
        }
        Some(result)
    }

    fn extract_i16_array<const N: usize>(dats: &[Dat; N]) -> Option<[i16; N]> {
        let mut result = [0i16; N];
        for i in 0..N {
            result[i] = ok!(dats[i].get_i16());
        }
        Some(result)
    }

    fn extract_i32_array<const N: usize>(dats: &[Dat; N]) -> Option<[i32; N]> {
        let mut result = [0i32; N];
        for i in 0..N {
            result[i] = ok!(dats[i].get_i32());
        }
        Some(result)
    }

    fn extract_i64_array<const N: usize>(dats: &[Dat; N]) -> Option<[i64; N]> {
        let mut result = [0i64; N];
        for i in 0..N {
            result[i] = ok!(dats[i].get_i64());
        }
        Some(result)
    }
}

#[derive(Clone, Debug)]
pub struct MsgCmd {
    pub syntax: SyntaxRef,
    pub sname:  String, // Syntax name.
    pub name:   String, // Command name.
    pub vals:   Vec<Dat>,
    pub args:   BTreeMap<String, Vec<Dat>>, // one-to-one
    pub rest:   Option<Vec<Dat>>,           // after "--", when it was given
}

impl fmt::Display for MsgCmd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        ok!(write!(f, "{}", self.name));
        for val in &self.vals {
            ok!(write!(f, " {:?}", val));
        }
        for (k, argvals) in &self.args {
            // Written as a reader would type it, not as the command files it.
            ok!(write!(f, " {}", self.arg_short_name(k)));
            for val in argvals {
                ok!(write!(f, " {:?}", val));
            }
        }
        if let Some(words) = &self.rest {
            ok!(write!(f, " --"));
            for val in words {
                ok!(write!(f, " {:?}", val));
            }
        }
        Ok(())
    }
}

impl MsgCmd {

    pub fn new<S: Into<String>>(
        syntax: SyntaxRef,
        name:   S,
    )
        -> Outcome<Self>
    {
        let name = name.into();
        let sname = syntax.config().name.clone();
        res!(syntax.cmds.get_recursive(&Key::Str(name.clone()))
            .ok_or_else(|| err!(
                "Cannot find this message command '{}' in the syntax '{}'.",
                name.clone(), sname;
            Invalid, Mismatch))
        );
        Ok(Self {
            syntax,
            sname,
            name,
            vals:   Vec::new(),
            args:   BTreeMap::new(),
            rest:   None,
        })
    }

    pub fn syntax<'a>(&'a self) -> &'a Syntax { self.syntax.as_ref() }

    fn get_syntax_cmd<'a>(
        &'a self,
    )
        ->  Outcome<&'a Cmd>
    {
        self.syntax().cmds.get_recursive(&Key::Str(self.name.clone())).ok_or_else(|| err!(
            "Cannot find this message command '{}' in the syntax '{}'.",
            self.name, self.sname;
        Invalid, Mismatch))
    }

    fn get_syntax_arg<'a, S: Into<String>>(
        &'a self,
        arg_name: S,
    )
        ->  Outcome<&'a Arg>
    {
        let arg_name = arg_name.into();
        res!(self.get_syntax_cmd()).args.get_recursive(&Key::Str(arg_name.clone()))
            .ok_or_else(|| err!(
                "Cannot find this command '{}' argument '{}' in the syntax '{}'.",
                self.name, arg_name, self.sname;
            Invalid, Mismatch))
    }

    fn arg_key<S: Into<String>>(&self, arg_name: S) -> String {
        let arg_name = arg_name.into();
        match self.get_syntax_cmd() {
            Ok(cmd) => match cmd.args.get_recursive(&Key::Str(arg_name.clone())) {
                Some(arg) => arg.canonical_name(),
                None => arg_name,
            },
            Err(_) => arg_name,
        }
    }

    fn arg_short_name<S: Into<String>>(&self, arg_name: S) -> String {
        let arg_name = arg_name.into();
        match self.get_syntax_cmd() {
            Ok(cmd) => match cmd.args.get_recursive(&Key::Str(arg_name.clone())) {
                Some(arg) => arg.short_name(),
                None => arg_name,
            },
            Err(_) => arg_name,
        }
    }

    pub fn add_arg<S: Into<String>>(
        mut self,
        arg_name: S,
    )
        ->  Outcome<Self>
    {
        let arg_name = arg_name.into();
        // Make sure the argument exists in the syntax.
        if !res!(self.get_syntax_cmd()).args.contains_key(&Key::Str(arg_name.clone())) {
            return Err(err!(
                "Can't find command '{}' argument '{}' in syntax '{}'.",
                self.name, arg_name, self.sname;
            Invalid, Input));
        }
        let key = self.arg_key(arg_name.clone());
        // Check whether it has already been added to the MsgCmd.
        if self.args.contains_key(&key) {
            return Err(err!(
                "Command '{}' argument '{}' has already been added to message.",
                self.name, arg_name;
            Invalid, Input, Exists));
        }
        self.args.insert(key, Vec::new());
        Ok(self)
    }
    
    pub fn add_cmd_val(
        self,
        val: Dat,
    )
        ->  Outcome<Self>
    {
        self.add_val::<String>(None, Some(val))
    }

    pub fn add_arg_val<S: Into<String>>(
        self,
        arg_name:   S,
        val:        Option<Dat>,
    )
        ->  Outcome<Self>
    {
        self.add_val(Some(arg_name), val)
    }

    pub fn add_val<S: Into<String>>(
        mut self,
        arg_opt:    Option<S>,
        val_opt:    Option<Dat>,
    )
        ->  Outcome<Self>
    {
        let arg_opt = arg_opt.map(|s| s.into());
        let (exp_vals, arg_name): (Vec<Val>, String) = match &arg_opt {
            Some(arg_name) => {
                let arg = res!(self.get_syntax_arg(arg_name.clone()));
                (arg.config().vals.clone(), fmt!("argument '{}' ", arg_name))
            },
            None => (res!(self.get_syntax_cmd()).config().vals.clone(), fmt!("")),
        };
        // File the value under the argument's one name, whichever of its names
        // the caller wrote.
        let arg_opt = arg_opt.map(|name| self.arg_key(name));

        let v: &mut Vec<Dat> = match arg_opt {
            Some(arg_name) => match self.args.get_mut(&arg_name) {
                Some(v) => v,
                None => {
                    self.args.insert(arg_name.clone(), Vec::new());
                    match self.args.get_mut(&arg_name) {
                        Some(v) => v,
                        None => return Err(err!(
                            "Argument '{}' was just created, but no longer present.", arg_name;
                        Bug, Unreachable)),
                    }
                },
            },
            None => &mut self.vals,
        };

        let next = match Val::slot(&exp_vals, v.len()) {
            Some(next) => next,
            None => return Err(err!(
                "Command '{}' {}already has all {} of its \
                expected values.", self.name, arg_name, v.len();
            Invalid, Input, Exists)),
        };

        match val_opt {
            Some(val) => {
                if next.kind == Kind::Unknown || val.kind() == next.kind {
                    v.push(val);
                } else {
                    return Err(err!(
                        "Command '{}' {}already has {} values, and \
                        the next one must be a {:?}, not a {:?}.",
                        self.name, arg_name, v.len(), next.kind, val.kind();
                    Invalid, Input));
                }
            },
            None => (),
        }

        Ok(self)
    }

    pub fn get_vals(&self) -> Option<&Vec<Dat>> {
        if self.vals.len() == 0 {
            None
        } else {
            Some(&self.vals)
        }
    }

    pub fn get_vals_mut(&mut self) -> Option<&mut Vec<Dat>> {
        if self.vals.len() == 0 {
            None
        } else {
            Some(&mut self.vals)
        }
    }

    pub fn get_arg_vals<S: Into<String>>(&self, a: S) -> Option<&Vec<Dat>> {
        match self.args.get(&self.arg_key(a)) {
            Some(vals) => if vals.len() == 0 {
                None
            } else {
                Some(&vals)
            },
            None => None,
        }
    }

    pub fn get_arg_vals_mut<S: Into<String>>(&mut self, a: S) -> Option<&mut Vec<Dat>> {
        let key = self.arg_key(a);
        match self.args.get_mut(&key) {
            Some(vals) => {
                if vals.len() == 0 {
                    None
                } else {
                    Some(vals)
                }
            },
            None => None,
        }
    }

    pub fn has_arg<S: Into<String>>(&self, a: S) -> bool {
        self.args.contains_key(&self.arg_key(a))
    }

    /// The words given after "--", or `None` when there was no "--".
    pub fn get_rest(&self) -> Option<&Vec<Dat>> { self.rest.as_ref() }

    /// The command's values, each of which must be a string, as a command line gives them.
    pub fn str_vals(&self) -> Outcome<Vec<&str>> {
        Self::strs(&self.vals, &fmt!("command '{}'", self.name))
    }

    /// The values of one of the command's arguments, each of which must be a string.
    pub fn str_arg_vals<S: Into<String>>(&self, a: S) -> Outcome<Vec<&str>> {
        let a = a.into();
        match self.args.get(&self.arg_key(a.clone())) {
            Some(vals) => Self::strs(vals, &fmt!("command '{}' argument '{}'", self.name, a)),
            None => Ok(Vec::new()),
        }
    }

    fn strs<'a>(vals: &'a [Dat], owner: &str) -> Outcome<Vec<&'a str>> {
        let mut out = Vec::with_capacity(vals.len());
        for (i, val) in vals.iter().enumerate() {
            match val {
                Dat::Str(s) => out.push(s.as_str()),
                other => return Err(err!(
                    "Value {} of the {} is a {:?}, not a string.", i + 1, owner, other.kind();
                Input, Mismatch)),
            }
        }
        Ok(out)
    }

    pub fn has_args(&self) -> bool {
        self.args.len() > 0
    }

    pub fn has_only_arg<S: Into<String>>(&self, a: S) -> Outcome<bool> {
        let a = a.into();
        let has = self.args.contains_key(&self.arg_key(a.clone()));
        if self.args.len() > 1 {
            Err(err!(
                "The argument '{}' {}", a, if has {
                    "does not exist and there are other arguments."    
                } else {
                    "does exist but there are other arguments."
                };
            Input, Invalid, Excessive))
        } else {
            Ok(has)
        }
    }
}
