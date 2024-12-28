use crate::{
    arg::Arg,
    cmd::Cmd,
    core::{
        Syntax,
        SyntaxRef,
    },
    key::Key,
};

use oxedize_fe2o3_core::{
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
use oxedize_fe2o3_jdat::prelude::*;
use oxedize_fe2o3_text::split::StringSplitter;

use std::{
    collections::{
        BTreeMap,
        BTreeSet,
    },
    fmt,
    slice::Iter,
};

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
            if !first { write!(f, " ")?; } 
            write!(f, "{:?}", val)?;
            first = false;
        }
        for (k, argvals) in &self.args {
            if !first { write!(f, " ")?; } 
            write!(f, "{}", k)?;
            for val in argvals {
                write!(f, " {:?}", val)?;
            }
            first = false;
        }
        for (k, cmd) in &self.cmds {
            if !first { write!(f, " ")?; } 
            write!(f, "{} {}", k, cmd)?;
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
        // Check whether it has already been added to the Msg.
        if self.args.contains_key(&arg_name) {
            return Err(err!(
                "Argument '{}' has already been added to message.", arg_name;
            Invalid, Input, Exists));
        }
        self.args.insert(arg_name, Vec::new());
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
        let exp_vals: Vec<(Kind, String)> = match &arg_opt {
            Some(arg_name) => {
                let arg = res!(self.get_syntax_arg(arg_name.clone()));
                arg.config().vals.clone()
            },
            None => self.syntax().config().vals.clone(),
        };

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

        if v.len() >= exp_vals.len() {
            return Err(err!(
                "Message already has all {} of its expected values.", v.len();
            Invalid, Input, Exists));
        }

        match val_opt {
            Some(val) => {
                if val.kind() == exp_vals[v.len()].0 {
                    v.push(val);
                } else {
                    return Err(err!(
                        "Message already has {} values, and the next one must be a {:?}, \
                        not a {:?}.", v.len(), exp_vals[v.len()], val.kind();
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
        match self.args.get(&(a.into())) {
            Some(vals) => if vals.len() == 0 {
                None
            } else {
                Some(&vals)
            },
            None => None,
        }
    }

    pub fn get_arg_vals_mut<S: Into<String>>(&mut self, a: S) -> Option<&mut Vec<Dat>> {
        match self.args.get_mut(&(a.into())) {
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
        self.args.contains_key(&(a.into()))
    }

    pub fn has_only_arg<S: Into<String>>(&self, a: S) -> Outcome<bool> {
        let a = a.into();
        let has = self.args.contains_key(&a);
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
        similarity_threshold:   Option<f64>,
    )
        -> Outcome<&Cmd>
    {
        if let Some(cmd) = self.syntax().cmds.get_recursive(word) {
            // We found it in the syntax, it's a command.
            return Ok(&cmd);
        } else {
            if let Some(similarity_threshold) = similarity_threshold {
                if let Key::Str(word) = word {
                    let mut min_dist = word.chars().count();
                    let mut closest = None;
                    // Loop through other commands to find those that are similar to the given
                    // word, using the specified threshold.
                    for cmd_key in self.syntax().cmds.keys() {
                        if let Key::Str(cmd) = cmd_key {
                            let dist = levenshtein::levenshtein(word, cmd);
                            if dist < min_dist {
                                min_dist = dist;
                                let similarity_ratio: f64 =
                                    1.0
                                    - (
                                        dist as f64
                                        / std::cmp::max(
                                            word.chars().count(),
                                            cmd.chars().count(),
                                        ) as f64
                                    );
                                if similarity_ratio > similarity_threshold { 
                                    closest = Some(cmd);
                                }
                            }
                        }
                    }
                    if let Some(suggestion) = closest {
                        return Err(err!(
                            "Did you mean '{}'?. The word '{}' is not an argument, but \
                            neither was it recognised as a command in the '{}' syntax.",
                            suggestion, word, self.syntax().config().name;
                        Input, Invalid, Suggestion))
                    }
                }
            }
        }
        Err(err!(
            "The word '{:?}' is not an argument, but neither was it \
            recognised as a command in the '{}' syntax.",
            word, self.syntax().config().name;
        Input, Invalid))
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
        let iter = seq.into_iter();
        // Read-only syntax args and cmds using many-to-one arg treemaps
        let mut active_cmd: Option<&Cmd> = None;
        let mut active_arg: Option<&Arg> = None;
        let mut val_kind_iter: Option<Iter<'_, (Kind, String)>> = None;
        // Read and write struct into which args and cmd data are one-to-one treemapped
        let mut msgrx = Msg::new(self.syntaxref());
        let mut rargs: Vec<&str> = Vec::new();
        for arg_name in &self.syntax().config().rargs {
            rargs.push(arg_name);   
        }
        let mut msg = String::new();
        let mut first = true;
        let mut collecting_vals = Collecting::None;
        if self.syntax().config().vals.len() > 0 {
            collecting_vals = Collecting::Message;   
            val_kind_iter = Some(self.syntax().config().vals.iter()); 
        }

        for word in iter {
            if first {
                msg.push_str(&word);
                first = false;
            } else {
                msg.push(' ');
                msg.push_str(&word);
            }
            let word_key = Key::Str(word.clone());
            if collecting_vals != Collecting::None {
                // VAL block
                if let Some(vkiter) = val_kind_iter.as_mut() {
                    //match res!(val_kind_iter.as_mut().ok_or_else(|| err!(
                    //    "val_kind_iter should not be None here.",
                    //), Bug, Unexpected))).next() {
                    match vkiter.next() {
                        Some((kind, _)) => {
                            // We're expecting another value.
                            if active_cmd.is_none() {
                                // Is the word a recognised message arg?
                                if self.syntax().args.contains_key(&word_key) {
                                    return Err(err!(
                                        "The syntax '{}' expects a value of kind '{:?}' but \
                                        found a message argument '{}'.",
                                        self.syntax().config().name, kind, word;
                                    Input, Missing));
                                }
                                // Is the word a recognised command?
                                if self.syntax().cmds.contains_key(&word_key) {
                                    return Err(err!(
                                        "The syntax '{}' expects a value of kind '{:?}' but \
                                        found a command '{}'.",
                                        self.syntax().config().name, kind, word;
                                    Input, Missing));
                                }
                            } else {
                                // Is the word a recognised command arg?
                                if let Some(cmd) = active_cmd.as_ref() {
                                    if cmd.args.contains_key(&word_key) {
                                        return Err(err!(
                                            "The syntax '{}' expects a value of kind '{:?}' but \
                                            found a command argument '{}'.",
                                            self.syntax().config().name, kind, word;
                                        Input, Missing));
                                    }
                                }
                            }
                            let mut d = res!(Dat::decode_string(&word));
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
                                _ => (),
                            }

                            if *kind == d.kind() || *kind == Kind::Unknown {
                                // The value kind matches what we expected, it's a valid value.
                                match collecting_vals {
                                    Collecting::Message => {
                                        msgrx.vals.push(d);
                                    },
                                    Collecting::MessageArg => {
                                        if let Some(arg) = active_arg.as_ref() {
                                            let akey = arg.canonical_name();
                                            // Key message arg to the hyph1 forms e.g. "-arg".
                                            let entry = msgrx.args.entry(akey).or_insert(Vec::new());
                                            entry.push(d);
                                        }
                                    },
                                    Collecting::Command => {
                                        if let Some(cmd) = active_cmd.as_ref() {
                                            let ckey = cmd.config().name.clone();
                                            let entry = msgrx.cmds.entry(ckey.clone())
                                                .or_insert(res!(MsgCmd::new(self.syntaxref(), ckey)));
                                            entry.vals.push(d);
                                        }
                                    },
                                    Collecting::CommandArg => {
                                        if let Some(cmd) = active_cmd.as_ref() {
                                            let ckey = cmd.config().name.clone();
                                            let entry1 = msgrx.cmds.entry(ckey.clone())
                                                .or_insert(res!(MsgCmd::new(self.syntaxref(), ckey)));
                                            if let Some(arg) = active_arg.as_ref() {
                                                let akey = arg.canonical_name();
                                                // Key command arg to the hyph1 forms e.g. "-arg".
                                                let entry2 = entry1.args.entry(akey).or_insert(Vec::new());
                                                entry2.push(d);
                                            }
                                        }
                                    },
                                    _ => {},
                                }
                                continue;
                            } else {
                                return Err(err!(
                                    "The syntax '{}' expects a value of kind '{:?}' \
                                    but the kind for received value '{}' is '{:?}'.",
                                    self.syntax().config().name, kind, word, d.kind();
                                Input, Invalid));
                            }
                        },
                        None => {
                            // Expected values exhausted.
                            collecting_vals = Collecting::None;
                            val_kind_iter = None;
                            if active_arg.is_some() {
                                active_arg = None;   
                            }
                        },
                    }
                } else {
                    return Err(err!(
                        "val_kind_iter should not be None here.";
                    Bug, Unexpected));
                }
            }
            if active_arg.is_none() {
                // CMD ARG block
                match active_cmd {
                    Some(cmd) => {
                        // It may be a command argument.
                        if let Some(arg) = cmd.args.get_recursive(&word_key) {
                            // We found it in the syntax, it's a command argument.
                            if let Some(cmdrx) = msgrx.get_cmd_mut(&cmd.config().name) {
                                if cmdrx.has_arg(&arg.canonical_name()) {
                                    return Err(err!(
                                        "The argument '{}' for command '{}' in the \
                                        syntax '{}' has already been detected.",
                                        word, cmd.config().name, self.syntax().config().name;
                                    Input, Invalid));
                                } else {
                                    cmdrx.args.insert(arg.canonical_name(), Vec::new());    
                                }
                            }
                            Self::revise_reqd_arg_list(&arg.canonical_name(), &mut rargs);
                            active_arg = Some(&arg);
                            collecting_vals = Collecting::CommandArg;
                            val_kind_iter = Some(arg.config().vals.iter());
                        }
                    },
                    None => {
                        // MSG ARG block
                        if let Some(arg) = self.syntax().args.get_recursive(&word_key) {
                            // We found it in the syntax, it's a message argument.
                            if msgrx.args.contains_key(&arg.canonical_name()) {
                                return Err(err!(
                                    "The message argument '{}' in the syntax \
                                    '{}' has already been detected.",
                                    word, self.syntax().config().name;
                                Input, Invalid));
                            } else {
                                msgrx.args.insert(arg.canonical_name(), Vec::new());    
                            }
                            Self::revise_reqd_arg_list(&arg.canonical_name(), &mut rargs);
                            active_arg = Some(&arg);
                            collecting_vals = Collecting::MessageArg;
                            val_kind_iter = Some(arg.config().vals.iter());
                        }
                    },
                }

                if active_arg.is_none() {
                    // CMD block
                    let cmd = res!(self.is_word_a_cmd(&word_key, similarity_threshold));
                    // Yep, checks out, we found it in the syntax, the word is a command.
                    msgrx.cmds.insert(
                        cmd.config().name.clone(),
                        res!(MsgCmd::new(
                            self.syntaxref(),
                            cmd.config().name.clone(),
                        )),
                    );
                    active_cmd = Some(cmd);
                    collecting_vals = Collecting::Command;
                    val_kind_iter = Some(cmd.config().vals.iter());
                    res!(self.check_required_txt_args(
                        rargs,
                        &None,
                    ));
                    // Reset rargs for specified command.
                    rargs = Vec::new();
                    if let Some(cmd) = active_cmd {
                        for arg_name in &cmd.config().rargs {
                            rargs.push(&arg_name);   
                        }
                    }
                }
            }
        }

        res!(self.check_required_txt_args(
            rargs,
            &active_cmd,
        ));

        msgrx.end = MsgEndState {
            vals:
            if let Some(iter) = val_kind_iter {
                iter.map(|(kind, _)| kind.clone()).collect::<Vec<Kind>>()
            } else {
                Vec::new()
            },
            arg:    match active_arg {
                Some(a) => Some(a.canonical_name()),
                None => None,
            },
            cmd:    match active_cmd {
                Some(c) => Some(c.config().name.clone()),
                None => None,
            },
        };
        msgrx.enc = Encoding::UTF8;

        Ok(msgrx)
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
        expected_vals:  &Vec<(Kind, String)>,
        actual_vals:    &Vec<Dat>,
        active_cmd:     &Option<&Cmd>,
    )
        -> Outcome<()>
    {
        if expected_vals.len() != actual_vals.len() {
            return Err(err!(
                "The syntax '{}' expects {} {} value(s), found {}.",
                self.syntax().config().name,
                expected_vals.len(),
                Self::msg_or_cmd_string(active_cmd),
                actual_vals.len();
            Input, Missing));
        }
        for i in 0..expected_vals.len() {
            if expected_vals[i].0 != actual_vals[i].kind() {
                return Err(err!(
                    "The syntax '{}' expects {} value {} to be a '{:?}', \
                    {:?} was found.",
                    self.syntax().config().name,
                    Self::msg_or_cmd_string(active_cmd),
                    actual_vals[i],
                    expected_vals[i].0,
                    actual_vals[i].kind();
                Input, Missing));
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
}

#[derive(Clone, Debug)]
pub struct MsgCmd {
    pub syntax: SyntaxRef,
    pub sname:  String, // Syntax name.
    pub name:   String, // Command name.
    pub vals:   Vec<Dat>,
    pub args:   BTreeMap<String, Vec<Dat>>, // one-to-one
}

impl fmt::Display for MsgCmd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ", self.name)?;
        let mut first = true;
        for val in &self.vals {
            if !first { write!(f, " ")?; } 
            write!(f, "{}", val)?;
            first = false;
        }
        for (k, argvals) in &self.args {
            if !first { write!(f, " ")?; } 
            write!(f, "{}", k)?;
            for val in argvals {
                write!(f, " {}", val)?;
            }
            first = false;
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
        // Check whether it has already been added to the MsgCmd.
        if self.args.contains_key(&arg_name) {
            return Err(err!(
                "Command '{}' argument '{}' has already been added to message.",
                self.name, arg_name;
            Invalid, Input, Exists));
        }
        self.args.insert(arg_name, Vec::new());
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
        let (exp_vals, arg_name): (Vec<(Kind, String)>, String) = match &arg_opt {
            Some(arg_name) => {
                let arg = res!(self.get_syntax_arg(arg_name.clone()));
                (arg.config().vals.clone(), fmt!("argument '{}' ", arg_name))
            },
            None => (res!(self.get_syntax_cmd()).config().vals.clone(), fmt!("")),
        };

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

        if v.len() >= exp_vals.len() {
            return Err(err!(
                "Command '{}' {}already has all {} of its \
                expected values.", self.name, arg_name, v.len();
            Invalid, Input, Exists));
        }

        match val_opt {
            Some(val) => {
                if exp_vals[v.len()].0 == Kind::Unknown || val.kind() == exp_vals[v.len()].0 {
                    v.push(val);
                } else {
                    return Err(err!(
                        "Command '{}' {}already has {} values, and \
                        the next one must be a {:?}, not a {:?}.",
                        self.name, arg_name, v.len(), exp_vals[v.len()], val.kind();
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
        match self.args.get(&(a.into())) {
            Some(vals) => if vals.len() == 0 {
                None
            } else {
                Some(&vals)
            },
            None => None,
        }
    }

    pub fn get_arg_vals_mut<S: Into<String>>(&mut self, a: S) -> Option<&mut Vec<Dat>> {
        match self.args.get_mut(&(a.into())) {
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
        self.args.contains_key(&(a.into()))
    }

    pub fn has_args(&self) -> bool {
        self.args.len() > 0
    }

    pub fn has_only_arg<S: Into<String>>(&self, a: S) -> Outcome<bool> {
        let a = a.into();
        let has = self.args.contains_key(&a);
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
