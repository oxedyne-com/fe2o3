mod fixture;

use fixture::{
    ore,
    words,
    BACK_EXCESS,
    BACK_MISSING,
    FORGET_MISSING,
    REST_MISSING,
};

use oxedyne_fe2o3_syntax::{
    argv::{
        self,
        Parsed,
    },
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
    help::Page,
    key::Key,
    msg::{
        Msg,
        MsgCmd,
    },
    val::Val,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    byte::{
        Encoding,
        ToBytes,
    },
};
use oxedyne_fe2o3_jdat::prelude::*;

use std::ffi::OsString;


fn run(line: &str) -> Outcome<Msg> {
    let syntax = res!(ore());
    let msg = Msg::new(syntax);
    msg.rx_argv(words(line), Some(argv::SIMILARITY_THRESHOLD))
}

fn cmd<'a>(msg: &'a Msg, name: &str) -> Outcome<&'a MsgCmd> {
    msg.get_cmd(name).ok_or_else(|| err!("No command '{}' in {}.", name, msg; Missing))
}

fn refusal(line: &str) -> Outcome<String> {
    match run(line) {
        Ok(msg) => Err(err!("'{}' was accepted as {}, and should not have been.", line, msg;
            Unexpected)),
        Err(e) => Ok(e.plain()),
    }
}

fn strs(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

// G5: a word the shell has unquoted is the value, not daticle text to decode.
#[test]
fn argv_takes_string_values_as_they_stand() -> Outcome<()> {
    let msg = res!(run_words(vec!["mark", "42", "(x)", "don't", "[a]", "{b}"]));
    let mark = res!(cmd(&msg, "mark"));
    req!(res!(mark.str_vals()), vec!["42", "(x)", "don't", "[a]", "{b}"]);
    // The same words read as a REPL line are decoded, which is what argv mode is for.
    let line = Msg::new(res!(ore()));
    if let Ok(msg) = line.rx_text_iter(strs(&["mark", "42"]), None) {
        return Err(err!("A line reading of 'mark 42' took 42 as {:?}, a string value.",
            msg; Unexpected));
    }
    Ok(())
}

fn run_words(v: Vec<&str>) -> Outcome<Msg> {
    let msg = Msg::new(res!(ore()));
    msg.rx_argv(strs(&v), Some(argv::SIMILARITY_THRESHOLD))
}

// G1: an option need not have a short form, and one without it registers no bare "-".
#[test]
fn long_only_options() -> Outcome<()> {
    let syntax = res!(ore());
    let sync = res!(syntax.get_cmd("sync").ok_or_else(|| err!("no sync"; Missing)));
    req!(sync.args.contains_key(&Key::from("-")), false);
    req!(sync.args.contains_key(&Key::from("--dry-run")), true);
    let msg = res!(run("sync --dry-run ../x"));
    let sync = res!(cmd(&msg, "sync"));
    req!(sync.has_arg("--dry-run"), true);
    req!(res!(sync.str_vals()), vec!["../x"]);
    // The internal name of an option is a value on a command line, not the option.
    let msg = res!(run("sync dry-run"));
    let sync = res!(cmd(&msg, "sync"));
    req!(sync.has_arg("--dry-run"), false);
    req!(res!(sync.str_vals()), vec!["dry-run"]);
    Ok(())
}

// G3, G7: an optional value may be absent, and a value may follow options.
#[test]
fn optional_values_and_values_after_options() -> Outcome<()> {
    for (line, dir, unsigned, mirror) in [
        ("init",                    None,       false,  None),
        ("init d",                  Some("d"),  false,  None),
        ("init --unsigned d",       Some("d"),  true,   None),
        ("init d --mirror m",       Some("d"),  false,  Some("m")),
        ("init --mirror m d",       Some("d"),  false,  Some("m")),
        ("init --unsigned",         None,       true,   None),
    ] {
        let msg = res!(run(line));
        let init = res!(cmd(&msg, "init"));
        let vals = res!(init.str_vals());
        req!(vals.first().copied(), dir, "'{}'", line);
        let few = vals.len() <= 1;
        req!(few, true, "'{}'", line);
        req!(init.has_arg("--unsigned"), unsigned, "'{}'", line);
        let got = res!(init.str_arg_vals("--mirror"));
        req!(got.first().copied(), mirror, "'{}'", line);
    }
    let e = res!(refusal("init a b"));
    let ok = e.contains("'b'") && e.contains("position 3") && e.contains("'init'");
    req!(ok, true, "{}", e);
    Ok(())
}

// G4: a repeated value takes every remaining word, and '--' starts the rest.
#[test]
fn repeated_values_and_the_rest() -> Outcome<()> {
    let msg = res!(run("mark n"));
    req!(res!(res!(cmd(&msg, "mark")).str_vals()), vec!["n"]);
    let msg = res!(run("mark n fixed the log"));
    // 'log' is a command's name, and with one command per line it is only a word here.
    req!(res!(res!(cmd(&msg, "mark")).str_vals()), vec!["n", "fixed", "the", "log"]);
    req!(msg.cmds.len(), 1);

    let msg = res!(run("forget a --reason r b"));
    let forget = res!(cmd(&msg, "forget"));
    req!(res!(forget.str_vals()), vec!["a", "b"]);
    req!(res!(forget.str_arg_vals("--reason")), vec!["r"]);

    let msg = res!(run("back m -- p q"));
    let back = res!(cmd(&msg, "back"));
    req!(res!(back.str_vals()), vec!["m"]);
    let rest = res!(back.get_rest().ok_or_else(|| err!("no rest"; Missing)));
    req!(rest.clone(), vec![dat!("p"), dat!("q")]);
    // After '--' nothing is an option or a command.
    let msg = res!(run("back m -- --dry-run -h sync"));
    let back = res!(cmd(&msg, "back"));
    let rest = res!(back.get_rest().ok_or_else(|| err!("no rest"; Missing)));
    req!(rest.clone(), vec![dat!("--dry-run"), dat!("-h"), dat!("sync")]);
    // No '--', no rest, which is not the same as an empty one.
    let msg = res!(run("back m"));
    req!(res!(cmd(&msg, "back")).get_rest().is_none(), true);
    Ok(())
}

// G12: a command's own sentences replace the generic ones.
#[test]
fn commands_own_sentences_for_absence_and_excess() -> Outcome<()> {
    let e = res!(refusal("back"));
    req!(e.contains(BACK_MISSING), true, "{}", e);
    let e = res!(refusal("back a b"));
    let ok = e.contains(BACK_EXCESS) && e.contains("'b'") && e.contains("position 3");
    req!(ok, true, "{}", e);
    let e = res!(refusal("back m --"));
    req!(e.contains(REST_MISSING), true, "{}", e);
    let e = res!(refusal("forget"));
    req!(e.contains(FORGET_MISSING), true, "{}", e);
    let e = res!(refusal("forget --dry-run"));
    req!(e.contains(FORGET_MISSING), true, "{}", e);
    // Without a sentence of its own, the refusal names the command and the value.
    let e = res!(refusal("sync"));
    req!(e.contains("The command 'sync' needs <repo|url>."), true, "{}", e);
    let e = res!(refusal("forget a --reason"));
    req!(e.contains("The option '--reason' needs <text>."), true, "{}", e);
    Ok(())
}

// G3 for an option's value.
#[test]
fn optional_option_values() -> Outcome<()> {
    let msg = res!(run("key --veil --veil-key"));
    let key = res!(cmd(&msg, "key"));
    req!(key.has_arg("--veil"), true);
    req!(key.get_arg_vals("--veil").is_none(), true);
    req!(key.has_arg("--veil-key"), true);
    let msg = res!(run("key --veil k"));
    req!(res!(res!(cmd(&msg, "key")).str_arg_vals("--veil")), vec!["k"]);
    // An owed option value may itself begin with a hyphen.
    let msg = res!(run("forget a --reason -x"));
    req!(res!(res!(cmd(&msg, "forget")).str_arg_vals("--reason")), vec!["-x"]);
    Ok(())
}

// G6: exactly one command per line.
#[test]
fn one_command_per_line() -> Outcome<()> {
    let e = res!(refusal("sync x log"));
    let ok = e.contains("'log'") && e.contains("position 3") && e.contains("'sync'");
    req!(ok, true, "{}", e);
    // A syntax without one_cmd still reads several commands on a line, as a REPL does.
    let mut s = Syntax::from(SyntaxConfig { name: fmt!("repl"), ..Default::default() });
    s = res!(s.add_cmd(Cmd::from(CmdConfig {
        name: fmt!("a"), vals: vec![Val::text("x")], ..Default::default() })));
    s = res!(s.add_cmd(Cmd::from(CmdConfig { name: fmt!("b"), ..Default::default() })));
    let msg = Msg::new(SyntaxRef::new(s));
    let msg = res!(msg.rx_argv(strs(&["a", "1", "b"]), None));
    req!(msg.cmds.len(), 2);
    Ok(())
}

// Every command refuses an option it does not know, naming the nearest one.
#[test]
fn unknown_options_and_commands_are_refused_with_a_suggestion() -> Outcome<()> {
    let e = res!(refusal("sync --dry-rn x"));
    req!(e.contains("Did you mean '--dry-run'?"), true, "{}", e);
    req!(e.contains("'--dry-rn' at position 2"), true, "{}", e);
    let e = res!(refusal("mark n --force"));
    let ok = e.contains("'--force'") && e.contains("'mark'");
    req!(ok, true, "{}", e);
    // After a '--' that ends the options there is still only one repository to name.
    let e = res!(refusal("sync x -- y"));
    let ok = e.contains("'y' at position 4") && e.contains("more than the command 'sync'");
    req!(ok, true, "{}", e);
    let e = res!(refusal("-- sync"));
    req!(e.contains("no command has been named"), true, "{}", e);
    // G11: the suggestion is a question, not a question with a full stop after it.
    let e = res!(refusal("mrak n"));
    req!(e.contains("Did you mean 'mark'? 'mrak' at position 1 is not a command."), true, "{}", e);
    req!(e.contains("?."), false, "{}", e);
    Ok(())
}

// An unknown verb points at help, generically, not with the old "is not an argument,
// and neither is it a command of" jargon.
#[test]
fn unknown_verb_names_the_help_command() -> Outcome<()> {
    let e = res!(refusal("frobnicate"));
    req!(e.contains("'frobnicate' at position 1 is not a command."), true, "{}", e);
    req!(e.contains("Type 'ore help' for the list."), true, "{}", e);
    // The suggestion form points at help too.
    let e = res!(refusal("mrak n"));
    req!(e.contains("Type 'ore help' for the list."), true, "{}", e);
    Ok(())
}

#[test]
fn numeric_values_are_still_read_as_numbers() -> Outcome<()> {
    let msg = res!(run("sync x --oresyn-max 3"));
    let sync = res!(cmd(&msg, "sync"));
    let n = res!(sync.get_arg_vals("--oresyn-max").ok_or_else(|| err!("none"; Missing)));
    req!(n.clone(), vec![Dat::U8(3)]);
    let e = res!(refusal("sync x --oresyn-max 300"));
    let ok = e.contains("'300' at position 4") && e.contains("must be a u8");
    req!(ok, true, "{}", e);
    let e = res!(refusal("sync x --oresyn-max many"));
    req!(e.contains("'many'"), true, "{}", e);
    Ok(())
}

// G10: a word that is not UTF-8 is refused by position, and nothing panics.
#[cfg(unix)]
#[test]
fn non_utf8_words_are_refused_by_position() -> Outcome<()> {
    use std::os::unix::ffi::OsStringExt;
    let syntax = res!(ore());
    let args = vec![
        OsString::from("ore"),
        OsString::from("mark"),
        OsString::from_vec(vec![b'n', 0xff, b'x']),
    ];
    match argv::parse(&syntax, args) {
        Ok(p) => Err(err!("A non-UTF-8 word was accepted: {:?}", p; Unexpected)),
        Err(e) => {
            let e = e.plain();
            let ok = e.contains("position 2") && e.contains("not UTF-8");
            req!(ok, true, "{}", e);
            Ok(())
        },
    }
}

// G9: help and version never reach the caller's dispatch.
#[test]
fn the_driver_answers_help_and_version_itself() -> Outcome<()> {
    let syntax = res!(ore());
    let parse = |v: &[&str]| {
        let mut args = vec![OsString::from("ore")];
        for w in v { args.push(OsString::from(w)); }
        argv::parse(&syntax, args)
    };
    for (v, want) in [
        (vec![],                        Page::Summary),
        (vec!["help"],                  Page::Summary),
        (vec!["--help"],                Page::Summary),
        (vec!["-h"],                    Page::Summary),
        (vec!["help", "--all"],         Page::All),
        (vec!["help", "sync"],          Page::Command(fmt!("sync"))),
        (vec!["help", "capture"],       Page::Topic(fmt!("capture"))),
        (vec!["sync", "--help"],        Page::Command(fmt!("sync"))),
        (vec!["sync", "x", "-h"],       Page::Command(fmt!("sync"))),
        (vec!["forget", "a", "--help"], Page::Command(fmt!("forget"))),
    ] {
        match res!(parse(&v)) {
            Parsed::Help(page) => req!(page, want, "{:?}", v),
            other => return Err(err!("{:?} gave {:?}, not help.", v, other; Unexpected)),
        }
    }
    match res!(parse(&["--version"])) {
        Parsed::Version => (),
        other => return Err(err!("--version gave {:?}.", other; Unexpected)),
    }
    // After '--', '-h' is a path, so the command runs.
    match res!(parse(&["back", "m", "--", "-h"])) {
        Parsed::Run(msg) => {
            let back = res!(cmd(&msg, "back"));
            req!(back.get_rest().cloned(), Some(vec![dat!("-h")]));
        },
        other => return Err(err!("'back m -- -h' gave {:?}.", other; Unexpected)),
    }
    match parse(&["help", "snyc"]) {
        Ok(p) => return Err(err!("'help snyc' gave {:?}.", p; Unexpected)),
        Err(e) => {
            let e = e.plain();
            req!(e.contains("Did you mean 'sync'?"), true, "{}", e);
        },
    }
    match parse(&["snyc", "--help"]) {
        Ok(p) => return Err(err!("'snyc --help' gave {:?}.", p; Unexpected)),
        Err(e) => {
            let e = e.plain();
            req!(e.contains("Did you mean 'sync'?"), true, "{}", e);
        },
    }
    match res!(parse(&["sync", "x", "--dry-run"])) {
        Parsed::Run(msg) => req!(res!(cmd(&msg, "sync")).has_arg("--dry-run"), true),
        other => return Err(err!("'sync x --dry-run' gave {:?}.", other; Unexpected)),
    }
    Ok(())
}

#[test]
fn syntax_shapes_a_parser_cannot_read_are_refused() -> Outcome<()> {
    let s = Syntax::from(SyntaxConfig { name: fmt!("bad"), ..Default::default() });
    let c = Cmd::from(CmdConfig {
        name:   fmt!("c"),
        vals:   vec![Val::text("a").many(), Val::text("b")],
        ..Default::default()
    });
    let refused = s.clone().add_cmd(c).is_err();
    req!(refused, true);
    let c = Cmd::from(CmdConfig {
        name:   fmt!("c"),
        vals:   vec![Val::text("a").optional(), Val::text("b")],
        ..Default::default()
    });
    let refused = s.clone().add_cmd(c).is_err();
    req!(refused, true);
    // A verbatim value that repeats takes every word left, so a rest after it is unreachable.
    let c = Cmd::from(CmdConfig {
        name:   fmt!("c"),
        vals:   vec![Val::text("a").many().verbatim()],
        rest:   Some(Val::text("b").many()),
        ..Default::default()
    });
    let refused = s.clone().add_cmd(c).is_err();
    req!(refused, true);
    // Two options answering to one name used to shadow each other silently.
    let c = Cmd::from(CmdConfig { name: fmt!("c"), ..Default::default() });
    let c = res!(c.add_arg(Arg::from(ArgConfig {
        name: fmt!("x"), hyph1: Some(fmt!("v")), ..Default::default() })));
    let refused = c.add_arg(Arg::from(ArgConfig {
        name: fmt!("y"), hyph1: Some(fmt!("v")), ..Default::default() })).is_err();
    req!(refused, true);
    Ok(())
}

/// The wire format of a fixed-arity syntax, pinned to the bytes the encoder produced before
/// values could be optional or repeated (fe2o3 47bfc8d).
fn fixed_syntax() -> Outcome<SyntaxRef> {
    let mut p = Syntax::from(SyntaxConfig {
        name:   fmt!("TestSyntax"),
        vals:   vec![(Kind::Str, fmt!("s")).into(), (Kind::I128, fmt!("n")).into()],
        ..Default::default()
    });
    p = res!(p.add_arg(Arg::from(ArgConfig {
        name: fmt!("Arg_a"), hyph1: Some(fmt!("a")), hyph2: Some(fmt!("a0")),
        vals: vec![(Kind::Str, fmt!("s")).into(), (Kind::I32, fmt!("n")).into()],
        ..Default::default()
    })));
    let mut c = Cmd::from(CmdConfig {
        name: fmt!("cmd"),
        vals: vec![(Kind::Str, fmt!("s")).into(), (Kind::I16, fmt!("n")).into()],
        ..Default::default()
    });
    c = res!(c.add_arg(Arg::from(ArgConfig {
        name: fmt!("Arg_b"), hyph1: Some(fmt!("b")), hyph2: Some(fmt!("b0")),
        vals: vec![(Kind::Str, fmt!("s")).into(), (Kind::U8, fmt!("n")).into()],
        ..Default::default()
    })));
    p = res!(p.add_cmd(c));
    Ok(SyntaxRef::new(p))
}

const WIRE_BINARY: [u8; 90] = [
    1, 51, 33, 25, 41, 33, 5, 104, 101, 108, 108, 111, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 42, 33, 1, 11, 0, 0, 51, 33, 15, 41, 33, 7, 103, 111, 111, 100, 98, 121, 101, 18,
    0, 0, 0, 1, 33, 1, 11, 0, 0, 51, 33, 11, 41, 33, 5, 97, 103, 97, 105, 110, 17, 255, 253, 33,
    1, 11, 0, 0, 51, 33, 11, 41, 33, 6, 100, 101, 106, 97, 118, 117, 10, 42,
];
const WIRE_TEXT: &str =
    "(str|\"hello\") (i128|42) -a (str|\"goodbye\") (i32|1) cmd (str|\"again\") (i16|-3) \
    -b (str|\"dejavu\") (u8|42)";

#[test]
fn the_wire_format_of_a_fixed_syntax_is_unchanged() -> Outcome<()> {
    let syntax = res!(fixed_syntax());
    let msg = Msg::new(syntax.clone());
    let mut m = res!(msg.from_str("hello 42 -a goodbye 1 cmd again -3 -b dejavu 42", None));
    m.set_encoding(Encoding::Binary);
    let bin = res!(m.to_bytes(Vec::new()));
    req!(bin, WIRE_BINARY.to_vec());
    m.set_encoding(Encoding::UTF8);
    let txt = res!(m.to_bytes(Vec::new()));
    req!(txt[0], 2u8);
    req!(String::from_utf8_lossy(&txt[1..]).to_string(), WIRE_TEXT.to_string());
    // And the pinned bytes still decode to the message.
    let back = res!(Msg::new(syntax).from_bytes(&WIRE_BINARY, None));
    req!(back.vals, vec![dat!("hello"), dat!(42i128)]);
    let c = res!(cmd(&back, "cmd"));
    req!(c.vals.clone(), vec![dat!("again"), dat!(-3i16)]);
    Ok(())
}

#[test]
fn a_rest_and_repeated_values_cross_the_wire() -> Outcome<()> {
    let syntax = res!(ore());
    for line in ["back m -- p q", "back m", "mark n a b c", "forget a b --reason r"] {
        let mut m = res!(Msg::new(syntax.clone()).rx_argv(words(line), None));
        m.set_encoding(Encoding::Binary);
        let bin = res!(m.to_bytes(Vec::new()));
        let back = res!(Msg::new(syntax.clone()).from_bytes(&bin, None));
        req!(back.cmds.len(), 1, "{}", line);
        for (name, sent) in &m.cmds {
            let got = res!(cmd(&back, name));
            req!(got.vals.clone(), sent.vals.clone(), "{}", line);
            req!(got.rest.clone(), sent.rest.clone(), "{}", line);
            req!(got.args.clone(), sent.args.clone(), "{}", line);
        }
    }
    Ok(())
}

/// A command shaped like `ore mark`: a name and a message, both verbatim, and one option.
fn say() -> Outcome<SyntaxRef> {
    let mut s = Syntax::from(SyntaxConfig {
        name:       fmt!("tool"),
        one_cmd:    true,
        ..Default::default()
    });
    let mut c = Cmd::from(CmdConfig {
        name:   fmt!("say"),
        vals:   vec![
            Val::text("name").missing("say needs a name.").verbatim(),
            Val::text("words").many().verbatim(),
        ],
        ..Default::default()
    });
    c = res!(c.add_arg(Arg::from(ArgConfig {
        name: fmt!("loud"), hyph2: Some(fmt!("loud")), ..Default::default() })));
    s = res!(s.add_cmd(c));
    let mut c = Cmd::from(CmdConfig {
        name:   fmt!("get"),
        vals:   vec![Val::text("what")],
        ..Default::default()
    });
    c = res!(c.add_arg(Arg::from(ArgConfig {
        name: fmt!("why"), hyph2: Some(fmt!("why")), vals: vec![Val::text("text")],
        ..Default::default() })));
    s = res!(s.add_cmd(c));
    Ok(SyntaxRef::new(s))
}

fn parse_say(v: &[&str]) -> Outcome<Parsed> {
    let syntax = res!(say());
    argv::read(&syntax, strs(v))
}

fn said(v: &[&str]) -> Outcome<(Vec<String>, bool)> {
    match res!(parse_say(v)) {
        Parsed::Run(msg) => {
            let c = res!(msg.cmds.values().next().ok_or_else(|| err!("no command"; Missing)));
            let vals = res!(c.str_vals()).iter().map(|v| fmt!("{}", v)).collect();
            Ok((vals, c.has_arg("--loud")))
        },
        other => Err(err!("{:?} gave {:?}, not a message.", v, other; Unexpected)),
    }
}

fn is_page(v: &[&str], name: &str) -> Outcome<()> {
    match res!(parse_say(v)) {
        Parsed::Help(page) => { req!(page, Page::Command(fmt!("{}", name)), "{:?}", v); Ok(()) },
        other => Err(err!("{:?} gave {:?}, not the page of '{}'.", v, other, name; Unexpected)),
    }
}

// A message is what was typed, so a word of it is never read as an option or as help.
#[test]
fn a_verbatim_value_takes_words_as_they_were_typed() -> Outcome<()> {
    // Once the words have begun, every word is one of them.
    req!(res!(said(&["say", "n", "-h", "--loud", "--", "--help", "x"])),
        (strs(&["n", "-h", "--loud", "--", "--help", "x"]), false));
    // Options come first.
    req!(res!(said(&["say", "--loud", "n", "w"])), (strs(&["n", "w"]), true));
    // A name may begin with a dash, as a commit's subject can.
    req!(res!(said(&["say", "-WIP", "w"])), (strs(&["-WIP", "w"]), false));
    req!(res!(said(&["say", "- fix typo", "w"])), (strs(&["- fix typo", "w"]), false));
    // Which is the price: a misspelt option there is a name, since nothing can tell them apart.
    req!(res!(said(&["say", "--lod", "w"])), (strs(&["--lod", "w"]), false));
    // Before the values begin, a help flag still asks for the page, and '--' still ends the
    // options, after which even a help flag is a name.
    res!(is_page(&["say", "-h"], "say"));
    res!(is_page(&["say", "--help", "w"], "say"));
    res!(is_page(&["say", "--loud", "-h"], "say"));
    req!(res!(said(&["say", "--", "-h", "w"])), (strs(&["-h", "w"]), false));
    req!(res!(said(&["say", "--", "--loud"])), (strs(&["--loud"]), false));
    match parse_say(&["say", "--"]) {
        Ok(p) => return Err(err!("'say --' gave {:?}.", p; Unexpected)),
        Err(e) => req!(e.plain().contains("say needs a name."), true, "{}", e.plain()),
    }
    // A command's own option is never a name.
    match parse_say(&["say", "--loud"]) {
        Ok(p) => return Err(err!("'say --loud' gave {:?}.", p; Unexpected)),
        Err(e) => req!(e.plain().contains("say needs a name."), true, "{}", e.plain()),
    }
    Ok(())
}

// A help flag is help only where an option could stand.
#[test]
fn a_help_flag_is_help_only_where_an_option_could_stand() -> Outcome<()> {
    res!(is_page(&["get", "--help"], "get"));
    res!(is_page(&["get", "x", "-h"], "get"));
    res!(is_page(&["get", "--why", "w", "--help"], "get"));
    // An option owed a value takes it.
    match res!(parse_say(&["get", "x", "--why", "-h"])) {
        Parsed::Run(msg) => req!(res!(res!(cmd(&msg, "get")).str_arg_vals("--why")), vec!["-h"]),
        other => return Err(err!("'get x --why -h' gave {:?}.", other; Unexpected)),
    }
    // After '--' it is a value.
    match res!(parse_say(&["get", "--", "--help"])) {
        Parsed::Run(msg) => req!(res!(res!(cmd(&msg, "get")).str_vals()), vec!["--help"]),
        other => return Err(err!("'get -- --help' gave {:?}.", other; Unexpected)),
    }
    // Words before it are still read, and a refusal among them is the answer.
    match parse_say(&["get", "--bogus", "-h"]) {
        Ok(p) => return Err(err!("'get --bogus -h' gave {:?}.", p; Unexpected)),
        Err(e) => req!(e.plain().contains("'--bogus'"), true, "{}", e.plain()),
    }
    Ok(())
}

// Only a word with the shape of an option is read as one; any other word is a value.
#[test]
fn only_option_shaped_words_are_options() -> Outcome<()> {
    for w in ["-v", "-abc", "-h", "--dry-run", "--x_y", "--dry-run=1", "--oresyn-max=x y"] {
        req!(Msg::looks_like_option(w), true, "{}", w);
    }
    for w in ["-", "--", "---", "-5", "-.5", "- fix typo", "-x.txt", "--no verify", "--=x", "-é"] {
        req!(Msg::looks_like_option(w), false, "{}", w);
    }
    let msg = res!(run("forget -x.txt"));
    req!(res!(res!(cmd(&msg, "forget")).str_vals()), vec!["-x.txt"]);
    let syntax = res!(ore());
    let msg = res!(Msg::new(syntax).rx_argv(strs(&["sync", "- a"]), None));
    req!(res!(res!(cmd(&msg, "sync")).str_vals()), vec!["- a"]);
    Ok(())
}

// '--' ends the options: every word after it is one of the command's values, until a
// command with a rest of its own gives the words after it to that instead.
#[test]
fn a_double_dash_ends_the_options() -> Outcome<()> {
    let msg = res!(run("forget --dry-run -- --reason b"));
    let forget = res!(cmd(&msg, "forget"));
    req!(res!(forget.str_vals()), vec!["--reason", "b"]);
    req!(forget.has_arg("--dry-run"), true);
    req!(forget.has_arg("--reason"), false);
    let msg = res!(run("sync -- --pull-only"));
    let sync = res!(cmd(&msg, "sync"));
    req!(res!(sync.str_vals()), vec!["--pull-only"]);
    req!(sync.has_arg("--pull-only"), false);
    let msg = res!(run("back m -- --dry-run"));
    let back = res!(cmd(&msg, "back"));
    req!(res!(back.str_vals()), vec!["m"]);
    req!(back.get_rest().cloned(), Some(vec![dat!("--dry-run")]));
    Ok(())
}
