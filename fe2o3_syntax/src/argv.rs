//! The driver between a process's command line and a syntax: `help`, `--help` and `--version`
//! are answered here, so they never reach the caller's dispatch, and the rest is parsed as one
//! message.
use crate::{
    core::SyntaxRef,
    help::Page,
    msg::Msg,
};

use oxedyne_fe2o3_core::prelude::*;

use std::ffi::OsString;


/// What a command line asked for.
#[derive(Clone, Debug)]
pub enum Parsed {
    Run(Msg),
    Help(Page),
    Version,
}

/// How close a misspelt command or option must be to a real one to be suggested.
pub const SIMILARITY_THRESHOLD: f64 = 0.4;

/// Reads a process's arguments, `std::env::args_os()` or the like, program name first.
///
/// With no words, or `help`, `-h` or `--help` first, the answer is the command table.
/// `help <command>` or `help <topic>` is that page, `help --all` is everything, and a
/// command followed anywhere before `--` by `-h` or `--help` is the command's page.
/// `--version` or `-V` alone is the version.  Otherwise the words are a message.
pub fn parse<I, S>(syntax: &SyntaxRef, args: I) -> Outcome<Parsed>
    where
        I: IntoIterator<Item=S>,
        S: Into<OsString>,
{
    let words = res!(words(args));
    read(syntax, words)
}

/// The words after the program name, each of which must be UTF-8.  A word that is not is
/// refused by position rather than panicking as `std::env::args` does.
pub fn words<I, S>(args: I) -> Outcome<Vec<String>>
    where
        I: IntoIterator<Item=S>,
        S: Into<OsString>,
{
    let mut out = Vec::new();
    for (i, arg) in args.into_iter().enumerate().skip(1) {
        match arg.into().into_string() {
            Ok(s) => out.push(s),
            Err(os) => return Err(err!(
                "The word at position {} is not UTF-8 ('{}'), and command line words must \
                be.", i, os.to_string_lossy();
            Input, Invalid, UTF8)),
        }
    }
    Ok(out)
}

/// As `parse`, for words already taken from the command line.
pub fn read(syntax: &SyntaxRef, words: Vec<String>) -> Outcome<Parsed> {
    let is_help_flag = |w: &str| w == "--help" || w == "-h";
    let first = match words.first() {
        Some(w) => w.as_str(),
        None => return Ok(Parsed::Help(Page::Summary)),
    };
    if words.len() == 1 && (first == "--version" || first == "-V") {
        return Ok(Parsed::Version);
    }
    if is_help_flag(first) && words.len() == 1 {
        return Ok(Parsed::Help(Page::Summary));
    }
    if first == "help" && syntax.get_cmd("help").is_none() {
        return match words.get(1).map(|w| w.as_str()) {
            None => Ok(Parsed::Help(Page::Summary)),
            Some("--all") => Ok(Parsed::Help(Page::All)),
            Some(name) => {
                if words.len() > 2 {
                    return Err(err!(
                        "The word '{}' at position 3 is more than help takes: help names one \
                        command or topic.", words[2];
                    Input, Excessive));
                }
                page_for(syntax, name)
            },
        };
    }
    // A command's page, when the help flag comes before any "--".
    let before_rest = words.iter().take_while(|w| w.as_str() != "--");
    let mut asked = false;
    for w in before_rest {
        if is_help_flag(w) {
            asked = true;
            break;
        }
    }
    if asked {
        return match syntax.get_cmd(first) {
            Some(_) => Ok(Parsed::Help(Page::Command(first.to_string()))),
            None => if is_help_flag(first) {
                Ok(Parsed::Help(Page::Summary))
            } else {
                // Named wrongly, so the parser's refusal, with its suggestion, is the answer.
                let msg = Msg::new(syntax.clone());
                let _ = res!(msg.rx_argv(vec![first.to_string()], Some(SIMILARITY_THRESHOLD)));
                Ok(Parsed::Help(Page::Summary))
            },
        };
    }
    let msg = Msg::new(syntax.clone());
    let msg = res!(msg.rx_argv(words, Some(SIMILARITY_THRESHOLD)));
    Ok(Parsed::Run(msg))
}

fn page_for(syntax: &SyntaxRef, name: &str) -> Outcome<Parsed> {
    if syntax.get_cmd(name).is_some() {
        return Ok(Parsed::Help(Page::Command(name.to_string())));
    }
    if syntax.get_topic(name).is_some() {
        return Ok(Parsed::Help(Page::Topic(name.to_string())));
    }
    let mut names: Vec<String> = syntax.cmds_in_order().into_iter()
        .map(|c| c.config().name.clone()).collect();
    for topic in &syntax.config().topics {
        names.push(topic.name.clone());
    }
    Err(match Msg::closest(name, &names, SIMILARITY_THRESHOLD) {
        Some(suggestion) => err!(
            "Did you mean '{}'? There is no command or help topic '{}' in '{}'.",
            suggestion, name, syntax.config().name;
        Input, Invalid, Suggestion),
        None => err!(
            "There is no command or help topic '{}' in '{}'.",
            name, syntax.config().name;
        Input, Invalid),
    })
}
