//! 2026-09-23: rewritten for command lines as well as REPLs.  Commands are grouped under their
//! category in the order they were added, a command has a page of its own and a syntax can carry
//! named prose topics.  Text is wrapped to a width, and colour is used only when standard output
//! is a terminal that wants it.
use crate::{
    arg::Arg,
    cmd::Cmd,
    core::Syntax,
    val::Val,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_stds::chars::Term;

use std::{
    ffi::OsString,
    io::IsTerminal,
};


/// A named page of prose, reached with `help <name>`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Topic {
    pub name:   String,
    pub help:   String, // one line, the page's title
    pub text:   String, // paragraphs separated by blank lines
}

impl Topic {
    pub fn new<S1: Into<String>, S2: Into<String>, S3: Into<String>>(
        name:   S1,
        help:   S2,
        text:   S3,
    )
        -> Self
    {
        Self {
            name:   name.into(),
            help:   help.into(),
            text:   text.into(),
        }
    }
}

/// Which help a reader asked for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Page {
    Summary,            // the command table
    All,                // the table, every command page and every topic
    Command(String),
    Topic(String),
}

#[derive(Clone, Debug)]
pub struct HelpDisplayConfig {
    pub colour:         bool,
    pub width:          usize,
    pub indent:         usize,  // where names start
    pub table_col:      usize,  // where a command's line starts in the table
    pub desc_col:       usize,  // where a value's or option's text starts
    pub cmd_effect:     String,
    pub val_effect:     String,
    pub arg_effect:     String,
    pub about_effect:   String,
}

impl Default for HelpDisplayConfig {
    fn default() -> Self {
        Self::for_stdout()
    }
}

impl HelpDisplayConfig {

    pub const WIDTH_DEFAULT:    usize = 80;
    pub const WIDTH_MIN:        usize = 60;
    pub const WIDTH_MAX:        usize = 100;

    /// Help without colour, at the given width, for text that is not bound for a terminal.
    pub fn plain(width: usize) -> Self {
        Self {
            colour:         false,
            width,
            indent:         5,
            table_col:      17,
            desc_col:       26,
            cmd_effect:     Term::SET_BRIGHT_FORE_YELLOW.to_string() + Term::BOLD,
            val_effect:     Term::SET_BRIGHT_FORE_MAGENTA.to_string() + Term::BOLD,
            arg_effect:     Term::SET_BRIGHT_FORE_GREEN.to_string() + Term::BOLD,
            about_effect:   Term::ITALIC.to_string() + Term::BOLD + Term::SET_BRIGHT_FORE_RED,
        }
    }

    /// Help as standard output should have it: coloured only on a terminal that wants colour,
    /// and as wide as `COLUMNS` says within bounds.
    pub fn for_stdout() -> Self {
        let mut cfg = Self::plain(Self::width_for(std::env::var("COLUMNS").ok()));
        cfg.colour = Self::colour_for(
            std::io::stdout().is_terminal(),
            std::env::var_os("NO_COLOR"),
            std::env::var_os("TERM"),
        );
        cfg
    }

    /// Should help be coloured?  Not when the output is not a terminal, not when `NO_COLOR` is
    /// set to anything (<https://no-color.org>), and not on a `dumb` terminal.
    pub fn colour_for(
        is_tty:     bool,
        no_color:   Option<OsString>,
        term:       Option<OsString>,
    )
        -> bool
    {
        if !is_tty {
            return false;
        }
        if let Some(v) = no_color {
            if !v.is_empty() {
                return false;
            }
        }
        match term {
            Some(t) => t != "dumb",
            None => true,
        }
    }

    /// The width help is wrapped to, given the value of `COLUMNS` if any.
    pub fn width_for(columns: Option<String>) -> usize {
        match columns.and_then(|c| c.trim().parse::<usize>().ok()) {
            Some(w) => w.clamp(Self::WIDTH_MIN, Self::WIDTH_MAX),
            None => Self::WIDTH_DEFAULT,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Help {
    pub cfg: HelpDisplayConfig,
}

impl Help {

    pub fn new(cfg: HelpDisplayConfig) -> Self {
        Self {
            cfg,
        }
    }

    // ┌───────────────────────┐
    // │ PAGES                 │
    // └───────────────────────┘

    pub fn page(&self, syntax: &Syntax, page: &Page) -> Outcome<Vec<String>> {
        match page {
            Page::Summary       => Ok(self.summary(syntax)),
            Page::All           => Ok(self.all(syntax)),
            Page::Command(name) => self.command_page(syntax, name),
            Page::Topic(name)   => self.topic_page(syntax, name),
        }
    }

    /// The first line of `--version`.
    pub fn version_line(syntax: &Syntax) -> String {
        fmt!("{} {}", syntax.config().name, syntax.config().ver)
    }

    /// The top page: what the program is, how it is called, its commands by category and its
    /// topics.
    pub fn summary(&self, syntax: &Syntax) -> Vec<String> {
        let cfg = syntax.config();
        let mut lines = Vec::new();
        lines.append(&mut self.title_lines(&Self::version_line(syntax), cfg.about.as_deref()));
        lines.push(String::new());
        lines.push(fmt!("USAGE:"));
        lines.append(&mut self.synopsis(&cfg.name, &self.generic_synopsis(syntax)));
        if cfg.one_cmd {
            let first = fmt!("{} help <command | topic>", cfg.name);
            let second = fmt!("{} <command> --help", cfg.name);
            let pad = " ".repeat(std::cmp::max(2, 40usize.saturating_sub(first.chars().count())));
            let joined = fmt!("{}{}{}", first, pad, second);
            if self.cfg.indent + joined.chars().count() <= self.cfg.width {
                lines.push(fmt!("{}{}", self.ind(), joined));
            } else {
                lines.push(fmt!("{}{}", self.ind(), first));
                lines.push(fmt!("{}{}", self.ind(), second));
            }
        }
        lines.push(String::new());
        let margs = syntax.args_in_order();
        if !margs.is_empty() {
            lines.push(fmt!("OPTIONS:"));
            for arg in margs {
                lines.append(&mut self.arg_row(arg, self.cfg.indent));
            }
        }
        for (cat, cmds) in Self::categories(syntax) {
            lines.push(Self::heading(&cat));
            for cmd in cmds {
                lines.append(&mut self.row(
                    self.cfg.indent,
                    &[(self.cfg.cmd_effect.as_str(), cmd.config().name.clone())],
                    self.cfg.table_col,
                    &cmd.config().help.as_deref().map(Self::normalise).unwrap_or_default(),
                ));
            }
        }
        if !cfg.topics.is_empty() {
            lines.push(String::new());
            lines.push(fmt!("TOPICS:"));
            let names = cfg.topics.iter().map(|t| t.name.clone()).collect::<Vec<_>>();
            for line in Self::wrap_words(&names, self.text_width(self.cfg.indent), "  ") {
                lines.push(fmt!("{}{}", self.ind(), line));
            }
        }
        if let Some(footer) = &cfg.footer {
            lines.push(String::new());
            lines.append(&mut self.prose(footer));
        }
        lines
    }

    /// A command's own page: its synopsis, values, options, prose and pointers.
    pub fn command_page(&self, syntax: &Syntax, name: &str) -> Outcome<Vec<String>> {
        let cmd = match syntax.get_cmd(name) {
            Some(cmd) => cmd,
            None => return Err(err!(
                "There is no command '{}' in '{}' to show help for.",
                name, syntax.config().name;
            Input, Missing)),
        };
        let sname = &syntax.config().name;
        let ccfg = cmd.config();
        let mut lines = Vec::new();
        let title = fmt!("{} {}", sname, ccfg.name);
        lines.append(&mut self.title_lines(
            &title, ccfg.help.as_deref().map(Self::lowered).as_deref()));
        lines.push(String::new());
        lines.push(fmt!("USAGE:"));
        lines.append(&mut self.synopsis(&title, &Self::cmd_synopsis(cmd)));
        lines.push(String::new());
        if !ccfg.vals.is_empty() || ccfg.rest.is_some() {
            lines.push(fmt!("VALUES:"));
            for val in &ccfg.vals {
                lines.append(&mut self.val_row(val, "", self.cfg.indent));
            }
            if let Some(rest) = &ccfg.rest {
                lines.append(&mut self.val_row(rest, "-- ", self.cfg.indent));
            }
        }
        let args = cmd.args_in_order();
        if !args.is_empty() {
            lines.push(fmt!("OPTIONS:"));
            for arg in args {
                lines.append(&mut self.arg_row(arg, self.cfg.indent));
            }
        }
        if let Some(detail) = &ccfg.detail {
            if lines.last().map_or(false, |l| !l.is_empty()) {
                lines.push(String::new());
            }
            lines.append(&mut self.prose(detail));
        }
        if !ccfg.see.is_empty() {
            if lines.last().map_or(false, |l| !l.is_empty()) {
                lines.push(String::new());
            }
            let refs = ccfg.see.iter()
                .map(|s| fmt!("{} help {}", sname, s))
                .collect::<Vec<_>>()
                .join("   ");
            lines.push(fmt!("SEE ALSO:  {}", refs));
        }
        while lines.last().map_or(false, |l| l.is_empty()) {
            lines.pop();
        }
        Ok(lines)
    }

    pub fn topic_page(&self, syntax: &Syntax, name: &str) -> Outcome<Vec<String>> {
        let topic = match syntax.get_topic(name) {
            Some(topic) => topic,
            None => return Err(err!(
                "There is no help topic '{}' in '{}'.", name, syntax.config().name;
            Input, Missing)),
        };
        let mut lines = Vec::new();
        let title = fmt!("{} help {}", syntax.config().name, topic.name);
        let about = if topic.help.is_empty() { None } else { Some(topic.help.as_str()) };
        lines.append(&mut self.title_lines(&title, about));
        lines.push(String::new());
        lines.append(&mut self.prose(&topic.text));
        Ok(lines)
    }

    /// The table, then every command page, then every topic, for a reader who wants it all.
    pub fn all(&self, syntax: &Syntax) -> Vec<String> {
        let mut lines = self.summary(syntax);
        for cmd in syntax.cmds_in_order() {
            if let Ok(mut page) = self.command_page(syntax, &cmd.config().name) {
                lines.push(String::new());
                lines.append(&mut page);
            }
        }
        for topic in &syntax.config().topics {
            if let Ok(mut page) = self.topic_page(syntax, &topic.name) {
                lines.push(String::new());
                lines.append(&mut page);
            }
        }
        lines
    }

    /// One page holding everything, each command followed by its values and options, as a REPL
    /// shows it.
    pub fn to_lines(
        &self,
        syntax: &Syntax,
    )
        -> Outcome<Vec<String>>
    {
        let cfg = syntax.config();
        let mut lines = Vec::new();
        if let Some(about) = &cfg.about {
            lines.append(&mut self.title_lines("", Some(about)));
        }
        lines.push(String::new());
        lines.push(fmt!("USAGE:"));
        lines.append(&mut self.synopsis("", &self.generic_synopsis(syntax)));
        let sub = self.cfg.indent + 4;
        let margs = syntax.args_in_order();
        if !cfg.vals.is_empty() || !margs.is_empty() {
            lines.push(fmt!("MSG:"));
            for val in &cfg.vals {
                lines.append(&mut self.val_row(val, "", self.cfg.indent));
            }
            for arg in margs {
                lines.append(&mut self.arg_row(arg, self.cfg.indent));
            }
        }
        for (cat, cmds) in Self::categories(syntax) {
            lines.push(Self::heading(&cat));
            for cmd in cmds {
                lines.append(&mut self.row(
                    self.cfg.indent,
                    &[(self.cfg.cmd_effect.as_str(), cmd.config().name.clone())],
                    self.cfg.table_col,
                    &cmd.config().help.as_deref().map(Self::normalise).unwrap_or_default(),
                ));
                for val in &cmd.config().vals {
                    lines.append(&mut self.val_row(val, "", sub));
                }
                if let Some(rest) = &cmd.config().rest {
                    lines.append(&mut self.val_row(rest, "-- ", sub));
                }
                for arg in cmd.args_in_order() {
                    lines.append(&mut self.arg_row(arg, sub));
                }
            }
        }
        Ok(lines)
    }

    // ┌───────────────────────┐
    // │ PIECES                │
    // └───────────────────────┘

    /// Commands grouped by category, categories in the order a command first names them.
    pub fn categories(syntax: &Syntax) -> Vec<(String, Vec<&Cmd>)> {
        let mut out: Vec<(String, Vec<&Cmd>)> = Vec::new();
        for cmd in syntax.cmds_in_order() {
            let cat = &cmd.config().cat;
            match out.iter_mut().find(|(c, _)| c == cat) {
                Some((_, cmds)) => cmds.push(cmd),
                None => out.push((cat.clone(), vec![cmd])),
            }
        }
        out
    }

    fn heading(cat: &str) -> String {
        if cat.is_empty() {
            fmt!("COMMANDS:")
        } else {
            fmt!("{}:", cat.to_uppercase())
        }
    }

    /// A command's synopsis after its name: values, options, then what may follow `--`.
    pub fn cmd_synopsis(cmd: &Cmd) -> Vec<String> {
        let mut words = Vec::new();
        for val in &cmd.config().vals {
            words.push(val.synopsis());
        }
        for arg in cmd.args_in_order() {
            words.push(Self::arg_synopsis(arg));
        }
        // The "--" itself is never required; the rest's arity says what must follow it.
        if let Some(rest) = &cmd.config().rest {
            words.push(fmt!("[-- {}]", rest.synopsis()));
        }
        words
    }

    pub fn arg_synopsis(arg: &Arg) -> String {
        let mut s = arg.long_name();
        for val in &arg.config().vals {
            s.push(' ');
            s.push_str(&val.synopsis());
        }
        if arg.config().reqd {
            s
        } else {
            fmt!("[{}]", s)
        }
    }

    fn generic_synopsis(&self, syntax: &Syntax) -> Vec<String> {
        let mut words = Vec::new();
        for val in &syntax.config().vals {
            words.push(val.synopsis());
        }
        if !syntax.args.is_empty() {
            words.push(fmt!("[<options>]"));
        }
        if !syntax.cmds.is_empty() {
            words.push(fmt!("<command>"));
            words.push(fmt!("[<values>]"));
            words.push(fmt!("[<options>]"));
            if !syntax.config().one_cmd {
                words.push(fmt!(".."));
            }
        }
        words
    }

    /// Lays out a synopsis, wrapping so that continuation lines start under the first word
    /// after the lead.
    fn synopsis(&self, lead: &str, words: &[String]) -> Vec<String> {
        let mut lines = Vec::new();
        let start = if lead.is_empty() {
            self.cfg.indent
        } else {
            self.cfg.indent + lead.chars().count() + 1
        };
        let mut line = if lead.is_empty() {
            self.ind()
        } else {
            fmt!("{}{}", self.ind(), lead)
        };
        let mut len = line.chars().count();
        let mut first_on_line = lead.is_empty();
        for word in words {
            let wlen = word.chars().count();
            let sep = if first_on_line { 0 } else { 1 };
            if !first_on_line && len + sep + wlen > self.cfg.width && len > start {
                lines.push(line);
                line = " ".repeat(start);
                len = start;
                first_on_line = true;
            }
            if !first_on_line {
                line.push(' ');
                len += 1;
            }
            line.push_str(word);
            len += wlen;
            first_on_line = false;
        }
        lines.push(line);
        lines
    }

    fn val_row(&self, val: &Val, prefix: &str, indent: usize) -> Vec<String> {
        let label = match val.arity {
            crate::val::Arity::Many | crate::val::Arity::OneOrMore =>
                fmt!("{}<{}>...", prefix, val.label()),
            _ => fmt!("{}<{}>", prefix, val.label()),
        };
        self.row(
            indent,
            &[(self.cfg.val_effect.as_str(), label)],
            indent + (self.cfg.desc_col - self.cfg.indent),
            &Self::normalise_opt(&val.help),
        )
    }

    fn arg_row(&self, arg: &Arg, indent: usize) -> Vec<String> {
        let mut segs: Vec<(&str, String)> = Vec::new();
        let names = arg.hyphenated_names();
        let names = if names.is_empty() { vec![arg.canonical_name()] } else { names };
        segs.push((self.cfg.arg_effect.as_str(), names.join(", ")));
        for val in &arg.config().vals {
            segs.push(("", fmt!(" ")));
            segs.push((self.cfg.val_effect.as_str(), val.synopsis()));
        }
        let help = arg.config().help.as_deref().map(Self::normalise).unwrap_or_default();
        let mut lines = self.row(
            indent,
            &segs,
            indent + (self.cfg.desc_col - self.cfg.indent),
            &help,
        );
        // An option with several values describes each on a line of its own.
        if arg.config().vals.len() > 1 {
            for val in &arg.config().vals {
                lines.append(&mut self.val_row(val, "", indent + 4));
            }
        }
        lines
    }

    /// A label, then its description from `col`, wrapped with a hanging indent.  A label too
    /// long to leave two spaces before `col` puts the description on the lines below.
    fn row(
        &self,
        indent: usize,
        label:  &[(&str, String)],
        col:    usize,
        desc:   &str,
    )
        -> Vec<String>
    {
        let plain_len: usize = label.iter().map(|(_, t)| t.chars().count()).sum();
        let painted: String = label.iter().map(|(e, t)| self.paint(e, t)).collect();
        let head = fmt!("{}{}", " ".repeat(indent), painted);
        if desc.is_empty() {
            return vec![head];
        }
        let desc_lines = Self::wrap(desc, self.text_width(col));
        let mut lines = Vec::new();
        let mut rest = desc_lines.iter();
        if indent + plain_len + 2 <= col {
            match rest.next() {
                Some(first) => lines.push(fmt!(
                    "{}{}{}", head, " ".repeat(col - indent - plain_len), first)),
                None => lines.push(head),
            }
        } else {
            lines.push(head);
        }
        for line in rest {
            lines.push(fmt!("{}{}", " ".repeat(col), line));
        }
        lines
    }

    /// Paragraphs wrapped to the width.  A paragraph whose every line is indented is kept as it
    /// stands, for examples and recipes.
    fn prose(&self, text: &str) -> Vec<String> {
        let mut lines = Vec::new();
        for (i, para) in Self::paragraphs(text).iter().enumerate() {
            if i > 0 {
                lines.push(String::new());
            }
            let verbatim = para.iter().all(|l| l.starts_with(' ') || l.starts_with('\t'));
            if verbatim {
                for l in para {
                    lines.push(l.trim_end().to_string());
                }
            } else {
                lines.append(&mut Self::wrap(&para.join(" "), self.cfg.width));
            }
        }
        lines
    }

    fn paragraphs(text: &str) -> Vec<Vec<&str>> {
        let mut paras = Vec::new();
        let mut cur: Vec<&str> = Vec::new();
        for line in text.lines() {
            if line.trim().is_empty() {
                if !cur.is_empty() {
                    paras.push(std::mem::take(&mut cur));
                }
            } else {
                cur.push(line);
            }
        }
        if !cur.is_empty() {
            paras.push(cur);
        }
        paras
    }

    /// Greedy word wrap.  A word longer than the width has a line to itself.
    pub fn wrap(text: &str, width: usize) -> Vec<String> {
        let words = text.split_whitespace().map(|w| w.to_string()).collect::<Vec<_>>();
        Self::wrap_words(&words, width, " ")
    }

    fn wrap_words(words: &[String], width: usize, sep: &str) -> Vec<String> {
        let mut lines = Vec::new();
        let mut line = String::new();
        let mut len = 0;
        let slen = sep.chars().count();
        for word in words {
            let wlen = word.chars().count();
            if len > 0 && len + slen + wlen > width {
                lines.push(std::mem::take(&mut line));
                len = 0;
            }
            if len > 0 {
                line.push_str(sep);
                len += slen;
            }
            line.push_str(word);
            len += wlen;
        }
        if len > 0 {
            lines.push(line);
        }
        lines
    }

    /// A page's first line, ' name -- about', the about wrapped under itself.
    fn title_lines(&self, name: &str, about: Option<&str>) -> Vec<String> {
        let about = match about {
            Some(about) => about,
            None => return vec![fmt!(" {}", name)],
        };
        let lead = if name.is_empty() { fmt!(" ") } else { fmt!(" {} -- ", name) };
        let n = lead.chars().count();
        let mut lines = Vec::new();
        for (i, line) in Self::wrap(about, self.text_width(n)).iter().enumerate() {
            let pad = if i == 0 { lead.clone() } else { " ".repeat(n) };
            lines.push(fmt!("{}{}", pad, self.paint(&self.cfg.about_effect, line)));
        }
        lines
    }

    fn text_width(&self, col: usize) -> usize {
        std::cmp::max(20, self.cfg.width.saturating_sub(col))
    }

    fn ind(&self) -> String { " ".repeat(self.cfg.indent) }

    fn paint(&self, effect: &str, s: &str) -> String {
        if self.cfg.colour && !effect.is_empty() {
            fmt!("{}{}{}", effect, s, Term::RESET)
        } else {
            s.to_string()
        }
    }

    /// A one-line help text as a title's second half: no closing full stop, and a capital
    /// lowered unless it starts an acronym.
    fn lowered(s: &str) -> String {
        let s = s.trim_end_matches('.');
        let mut chars = s.chars();
        match (chars.next(), chars.next()) {
            (Some(a), Some(b)) if a.is_uppercase() && b.is_lowercase() =>
                a.to_lowercase().chain(s.chars().skip(1)).collect(),
            _ => s.to_string(),
        }
    }

    fn normalise_opt(s: &str) -> String {
        if s.is_empty() { String::new() } else { Self::normalise(s) }
    }

    pub fn normalise(s: &str) -> String {
        if s.ends_with('.') {
            s.to_string()
        } else {
            fmt!("{}.", s)
        }
    }
}
