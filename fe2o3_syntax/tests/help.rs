mod fixture;

use fixture::ore;

use oxedyne_fe2o3_syntax::{
    cmd::{
        Cmd,
        CmdConfig,
    },
    core::{
        Syntax,
        SyntaxConfig,
    },
    help::{
        Help,
        HelpDisplayConfig,
        Page,
    },
};

use oxedyne_fe2o3_core::prelude::*;

use std::ffi::OsString;


fn plain(width: usize) -> Help { Help::new(HelpDisplayConfig::plain(width)) }

fn check(got: Vec<String>, want: &str) -> Outcome<()> {
    let want: Vec<&str> = want.lines().collect();
    for (i, (g, w)) in got.iter().zip(want.iter()).enumerate() {
        req!(g.as_str(), *w, "line {} differs", i + 1);
    }
    req!(got.len(), want.len(), "line counts differ; got:\n{}", got.join("\n"));
    Ok(())
}

const SUMMARY: &str = r#" ore 0.1.0 -- version control whose unit of history is the whole edit

USAGE:
     ore <command> [<values>] [<options>]
     ore help <command | topic>              ore <command> --help

RECORD:
     init        Create a repository, minting a replica and a signing key.
     mark        Capture the working copy and name this point in history.
     key         Mint or replace this replica's keys: signing, content, veil.
GO BACK:
     back        Put the working copy back to a mark (records nothing).
     forget      Take content out of operations, keep their shape, record why.
EXCHANGE:
     sync        Exchange operations with another repository or a relay.

TOPICS:
     capture  relay

Every command except init and help begins by capturing the working copy, so
nothing has to be staged."#;

const SYNC: &str = r#" ore sync -- exchange operations with another repository or a relay

USAGE:
     ore sync <repo|url> [--pull-only] [--dry-run] [--oresyn-max <n>]

VALUES:
     <repo|url>           The root of another Ore repository this machine can
                          reach, or a relay named
                          https://<host>/<account>/<repository>.
OPTIONS:
     --pull-only          Take what the other end has, hand over nothing, and
                          record nothing here.
     --dry-run            Say what a sync would bring, and write nothing.
     --oresyn-max <n>     Hold this end to an older ORESYN vocabulary.

Both ends end up holding every operation either held, and nothing is ever
overwritten: a sync only adds.

A relay is reached the same way:

    ore sync https://relay.example/me/notes

Neither end needs to be still for a sync.

SEE ALSO:  ore help relay"#;

const BACK: &str = r#" ore back -- put the working copy back to a mark (records nothing)

USAGE:
     ore back <mark> [-- <path>...]

VALUES:
     <mark>               The mark to go back to.
     -- <path>...         Put back only these paths."#;

const RELAY: &str = r#" ore help relay -- exchanging through a server

A relay holds a repository's operations for machines that cannot reach each
other."#;

const REPL: &str = r#" version control whose unit of history is the whole edit

USAGE:
     <command> [<values>] [<options>]
RECORD:
     init        Create a repository, minting a replica and a signing key.
         <dir>                Where to create it. Default: the current
                              directory.
         --unsigned           Record without signing.
         --mirror <path>      Keep a git mirror at this path.
     mark        Capture the working copy and name this point in history.
         <name>               The mark's name.
         <message>...         Words of the mark's message.
     key         Mint or replace this replica's keys: signing, content, veil.
         --veil [<key>]       Veil the store.
         --veil-key           Print the veil key.
GO BACK:
     back        Put the working copy back to a mark (records nothing).
         <mark>               The mark to go back to.
         -- <path>...         Put back only these paths.
     forget      Take content out of operations, keep their shape, record why.
         <op|path>...         An operation or a path.
         --reason <text>      Why, for the record.
         --dry-run            Say what would be forgotten, and write nothing.
EXCHANGE:
     sync        Exchange operations with another repository or a relay.
         <repo|url>           The root of another Ore repository this machine
                              can reach, or a relay named
                              https://<host>/<account>/<repository>.
         --pull-only          Take what the other end has, hand over nothing,
                              and record nothing here.
         --dry-run            Say what a sync would bring, and write nothing.
         --oresyn-max <n>     Hold this end to an older ORESYN vocabulary."#;

#[test]
fn golden_summary() -> Outcome<()> {
    let syntax = res!(ore());
    check(res!(plain(80).page(&syntax, &Page::Summary)), SUMMARY)
}

#[test]
fn golden_command_pages() -> Outcome<()> {
    let syntax = res!(ore());
    res!(check(res!(plain(80).page(&syntax, &Page::Command(fmt!("sync")))), SYNC));
    check(res!(plain(80).page(&syntax, &Page::Command(fmt!("back")))), BACK)
}

#[test]
fn golden_topic_page() -> Outcome<()> {
    let syntax = res!(ore());
    check(res!(plain(80).page(&syntax, &Page::Topic(fmt!("relay")))), RELAY)
}

// The one-page layout a REPL prints for its 'help' command.
#[test]
fn golden_repl_page() -> Outcome<()> {
    let syntax = res!(ore());
    check(res!(plain(80).to_lines(&syntax)), REPL)
}

#[test]
fn all_is_the_table_then_every_page_then_every_topic() -> Outcome<()> {
    let syntax = res!(ore());
    let all = res!(plain(80).page(&syntax, &Page::All)).join("\n");
    let mut at = 0;
    for anchor in [
        " ore 0.1.0 --", " ore init --", " ore mark --", " ore key --", " ore back --",
        " ore forget --", " ore sync --", " ore help capture --", " ore help relay --",
    ] {
        match all[at..].find(anchor) {
            Some(i) => at += i + anchor.len(),
            None => return Err(err!("'{}' is missing or out of order in help --all.", anchor;
                Missing)),
        }
    }
    Ok(())
}

#[test]
fn colour_only_when_asked_for() -> Outcome<()> {
    let syntax = res!(ore());
    for page in [Page::All] {
        let text = res!(plain(80).page(&syntax, &page)).join("\n");
        req!(text.contains('\x1b'), false);
        let mut cfg = HelpDisplayConfig::plain(80);
        cfg.colour = true;
        let text = res!(Help::new(cfg).page(&syntax, &page)).join("\n");
        req!(text.contains('\x1b'), true);
    }
    let text = res!(plain(80).to_lines(&syntax)).join("\n");
    req!(text.contains('\x1b'), false);
    // no-color.org: set to anything but empty turns colour off.
    let tty = |nc: Option<&str>, term: Option<&str>| HelpDisplayConfig::colour_for(
        true, nc.map(OsString::from), term.map(OsString::from));
    req!(HelpDisplayConfig::colour_for(false, None, Some(OsString::from("xterm"))), false);
    req!(tty(None, Some("xterm-256color")), true);
    req!(tty(None, None), true);
    req!(tty(Some("1"), Some("xterm")), false);
    req!(tty(Some(""), Some("xterm")), true);
    req!(tty(None, Some("dumb")), false);
    Ok(())
}

#[test]
fn width_follows_columns_within_bounds() -> Outcome<()> {
    req!(HelpDisplayConfig::width_for(None), 80);
    req!(HelpDisplayConfig::width_for(Some(fmt!("90"))), 90);
    req!(HelpDisplayConfig::width_for(Some(fmt!("30"))), 60);
    req!(HelpDisplayConfig::width_for(Some(fmt!("300"))), 100);
    req!(HelpDisplayConfig::width_for(Some(fmt!("wide"))), 80);
    Ok(())
}

// Every line fits the width, except a preformatted line or a single word too long to break.
#[test]
fn wrapped_lines_fit_the_width() -> Outcome<()> {
    let syntax = res!(ore());
    for width in [60, 72, 100] {
        let mut lines = res!(plain(width).page(&syntax, &Page::All));
        lines.append(&mut res!(plain(width).to_lines(&syntax)));
        for line in lines {
            let n = line.chars().count();
            let unbreakable = line.trim().split_whitespace().count() == 1;
            let verbatim = line.starts_with("    ore sync https://");
            if n > width && !unbreakable && !verbatim {
                return Err(err!("A line of {} characters at width {}: '{}'", n, width, line;
                    Mismatch));
            }
        }
    }
    Ok(())
}

#[test]
fn categories_in_the_order_commands_name_them() -> Outcome<()> {
    let mut s = Syntax::from(SyntaxConfig { name: fmt!("x"), ..Default::default() });
    for (name, cat) in [("z", "Zeta"), ("a", "Alpha"), ("m", "Zeta"), ("b", "")] {
        s = res!(s.add_cmd(Cmd::from(CmdConfig {
            name: fmt!("{}", name), cat: fmt!("{}", cat), ..Default::default() })));
    }
    let lines = plain(80).summary(&s);
    let heads: Vec<&String> = lines.iter().filter(|l| l.ends_with(':')).collect();
    req!(heads, vec!["USAGE:", "ZETA:", "ALPHA:", "COMMANDS:"]);
    let names: Vec<String> = lines.iter()
        .filter(|l| l.starts_with("     ") && !l.contains('<'))
        .map(|l| l.trim().to_string()).collect();
    req!(names, vec!["z", "m", "a", "b"]);
    Ok(())
}
