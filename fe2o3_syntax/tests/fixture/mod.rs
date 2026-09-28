//! A command line shaped like Ore's, the first real caller of the argv path: long-only options,
//! optional and repeated values, words after `--`, and commands' own sentences for absence and
//! excess.
use oxedyne_fe2o3_syntax::{
    arg::{
        Arg,
        ArgConfig,
    },
    cmd::{
        Cmd,
        CmdConfig,
    },
    core::{
        Syntax,
        SyntaxConfig,
        SyntaxRef,
    },
    help::Topic,
    val::Val,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    version::SemVer,
};

pub const BACK_MISSING: &str    = "back names one state to go back to.";
pub const BACK_EXCESS: &str     = "back names one state to go back to, and paths go after --.";
pub const REST_MISSING: &str    = "There is nothing after the --: name the paths to put back.";
pub const FORGET_MISSING: &str  = "forget needs something to forget.";

fn long(name: &str, help: &str) -> Arg {
    Arg::from(ArgConfig {
        name:   fmt!("{}", name),
        hyph2:  Some(fmt!("{}", name)),
        help:   Some(fmt!("{}", help)),
        ..Default::default()
    })
}

fn long_val(name: &str, val: Val, help: &str) -> Arg {
    Arg::from(ArgConfig {
        name:   fmt!("{}", name),
        hyph2:  Some(fmt!("{}", name)),
        vals:   vec![val],
        help:   Some(fmt!("{}", help)),
        ..Default::default()
    })
}

pub fn ore() -> Outcome<SyntaxRef> {
    let mut s = Syntax::from(SyntaxConfig {
        name:       fmt!("ore"),
        ver:        SemVer::new(0, 1, 0),
        about:      Some(fmt!("version control whose unit of history is the whole edit")),
        one_cmd:    true,
        footer:     Some(fmt!("Every command except init and help begins by capturing the \
                    working copy, so nothing has to be staged.")),
        ..Default::default()
    });

    // Record
    let mut c = Cmd::from(CmdConfig {
        name:   fmt!("init"),
        help:   Some(fmt!("Create a repository, minting a replica and a signing key.")),
        cat:    fmt!("Record"),
        vals:   vec![Val::text("dir").optional()
                    .help("Where to create it. Default: the current directory.")],
        ..Default::default()
    });
    c = res!(c.add_arg(long("unsigned", "Record without signing")));
    c = res!(c.add_arg(long_val("mirror", Val::text("path"), "Keep a git mirror at this path")));
    s = res!(s.add_cmd(c));

    let c = Cmd::from(CmdConfig {
        name:   fmt!("mark"),
        help:   Some(fmt!("Capture the working copy and name this point in history.")),
        cat:    fmt!("Record"),
        vals:   vec![
            Val::text("name").help("The mark's name"),
            Val::text("message").many().help("Words of the mark's message"),
        ],
        ..Default::default()
    });
    s = res!(s.add_cmd(c));

    let mut c = Cmd::from(CmdConfig {
        name:   fmt!("key"),
        help:   Some(fmt!("Mint or replace this replica's keys: signing, content, veil.")),
        cat:    fmt!("Record"),
        ..Default::default()
    });
    c = res!(c.add_arg(long_val("veil", Val::text("key").optional(), "Veil the store")));
    c = res!(c.add_arg(long("veil-key", "Print the veil key")));
    s = res!(s.add_cmd(c));

    // Go back
    let c = Cmd::from(CmdConfig {
        name:   fmt!("back"),
        help:   Some(fmt!("Put the working copy back to a mark (records nothing).")),
        cat:    fmt!("Go back"),
        vals:   vec![Val::text("mark").missing(BACK_MISSING).help("The mark to go back to")],
        rest:   Some(Val::text("path").one_or_more().missing(REST_MISSING)
                    .help("Put back only these paths")),
        excess: Some(fmt!("{}", BACK_EXCESS)),
        ..Default::default()
    });
    s = res!(s.add_cmd(c));

    let mut c = Cmd::from(CmdConfig {
        name:   fmt!("forget"),
        help:   Some(fmt!("Take content out of operations, keep their shape, record why.")),
        cat:    fmt!("Go back"),
        vals:   vec![Val::text("op|path").one_or_more().missing(FORGET_MISSING)
                    .help("An operation or a path")],
        ..Default::default()
    });
    c = res!(c.add_arg(long_val("reason", Val::text("text"), "Why, for the record")));
    c = res!(c.add_arg(long("dry-run", "Say what would be forgotten, and write nothing")));
    s = res!(s.add_cmd(c));

    // Exchange
    let mut c = Cmd::from(CmdConfig {
        name:   fmt!("sync"),
        help:   Some(fmt!("Exchange operations with another repository or a relay.")),
        cat:    fmt!("Exchange"),
        vals:   vec![Val::text("repo|url").help("The root of another Ore repository this \
                    machine can reach, or a relay named https://<host>/<account>/<repository>.")],
        detail: Some(fmt!("Both ends end up holding every operation either held, and \
                    nothing is ever overwritten: a sync only adds.\n\
                    \n\
                    A relay is reached the same way:\n\
                    \n\
                    \x20   ore sync https://relay.example/me/notes\n\
                    \n\
                    Neither end needs to be still for a sync.")),
        see:    vec![fmt!("relay")],
        ..Default::default()
    });
    c = res!(c.add_arg(long("pull-only",
        "Take what the other end has, hand over nothing, and record nothing here")));
    c = res!(c.add_arg(long("dry-run", "Say what a sync would bring, and write nothing")));
    c = res!(c.add_arg(long_val("oresyn-max", Val::new(Kind::U8, "n"),
        "Hold this end to an older ORESYN vocabulary")));
    s = res!(s.add_cmd(c));

    s = res!(s.add_topic(Topic::new(
        "capture",
        "what every command does first",
        "Every command but init begins by capturing the working copy: each file that has \
        changed since the last capture becomes an operation in the log.",
    )));
    s = res!(s.add_topic(Topic::new(
        "relay",
        "exchanging through a server",
        "A relay holds a repository's operations for machines that cannot reach each other.",
    )));

    Ok(SyntaxRef::new(s))
}

#[allow(dead_code)]
pub fn words(line: &str) -> Vec<String> {
    line.split_whitespace().map(|w| w.to_string()).collect()
}
