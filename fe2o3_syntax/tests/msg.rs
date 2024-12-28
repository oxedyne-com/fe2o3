use oxedize_fe2o3_syntax::{
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
        SyntaxRef,
        SyntaxConfig,
    },
    key::Key,
    msg::Msg,
};

use oxedize_fe2o3_core::{
    prelude::*,
    byte::{
        Encoding,
        IntoBytes,
        ToBytes,
        FromBytes,
    },
    map::MapRec,
    test::test_it,
};
use oxedize_fe2o3_jdat::{
    prelude::*,
    version::SemVer,
};

fn make_test_syntax_00() -> Outcome<SyntaxRef> {
    let a1 = Arg::from(ArgConfig {
        name:   fmt!("Arg_c1a1"),
        hyph1:  fmt!("c"),
        hyph2:  Some(fmt!("c1a1")),
        reqd:   true,
        vals:   vec![
            (Kind::Str, fmt!("A string value for Arg_c1a1.")),
            (Kind::I16, fmt!("A number for c1a1.")),
        ],
        help:   Some(fmt!("Some help text")),
        ..Default::default()
    });
    let a2 = Arg::from(ArgConfig {
        name:   fmt!("Arg_c1a2"),
        hyph1:  fmt!("d"),
        hyph2:  Some(fmt!("c1a2")),
        reqd:   false,
        help:   Some(fmt!("Some help text")),
        ..Default::default()
    });
    let a3 = Arg::from(ArgConfig {
        name:   fmt!("Arg_c1a3"),
        hyph1:  fmt!("e"),
        hyph2:  Some(fmt!("c1a3")),
        reqd:   false,
        help:   Some(fmt!("Some help text")),
        ..Default::default()
    });

    let mut c = Cmd::from(CmdConfig {
        name:   fmt!("cmd1"),
        help:   Some(fmt!("Command One help text")),
        ..Default::default()
    });
    c = res!(c.add_arg(a1));
    c = res!(c.add_arg(a2));
    c = res!(c.add_arg(a3));

    let a1 = Arg::from(ArgConfig {
        name:   fmt!("Arg_a"),
        hyph1:  fmt!("a"),
        hyph2:  Some(fmt!("a0")),
        reqd:   true,
        vals:   vec![
            (Kind::Str, fmt!("A string value for Arg_a.")),
        ],
        help:   Some(fmt!("Some help text")),
        ..Default::default()
    });
    let a2 = Arg::from(ArgConfig {
        name:   fmt!("Arg_b"),
        hyph1:  fmt!("b"),
        hyph2:  Some(fmt!("b0")),
        reqd:   false,
        help:   Some(fmt!("Some help text")),
        ..Default::default()
    });

    let mut p = Syntax::from(SyntaxConfig {
        name:   fmt!("TestSyntax"),
        ver:    SemVer::new(0, 1, 0),
        about:  Some(fmt!("Testing")),
        ..Default::default()
    });

    p = res!(p.add_arg(a1));
    p = res!(p.add_arg(a2));
    p = res!(p.add_cmd(c));

    Ok(SyntaxRef::new(p))
}

fn make_test_syntax_01() -> Outcome<SyntaxRef> {
    let a1 = Arg::from(ArgConfig {
        name:   fmt!("Arga"),
        hyph1:  fmt!("a"),
        hyph2:  Some(fmt!("arga")),
        vals:   vec![
            (Kind::Str, fmt!("A string value for Arga.")),
            (Kind::I16, fmt!("A number value for Arga.")),
        ],
        help:   Some(fmt!("Activate the a argument")),
        ..Default::default()
    });
    let a2 = Arg::from(ArgConfig {
        name:   fmt!("Argb"),
        hyph1:  fmt!("b"),
        hyph2:  Some(fmt!("argb")),
        help:   Some(fmt!("Activate the b argument")),
        ..Default::default()
    });
    let a3 = Arg::from(ArgConfig {
        name:   fmt!("Argc"),
        hyph1:  fmt!("c"),
        hyph2:  Some(fmt!("argc")),
        vals:   vec![
            (Kind::Str, fmt!("A string value for Argc.")),
        ],
        help:   Some(fmt!("Activate the c argument")),
        ..Default::default()
    });
    let a4 = Arg::from(ArgConfig {
        name:   fmt!("Argd"),
        hyph1:  fmt!("d"),
        hyph2:  Some(fmt!("argd")),
        vals:   vec![
            (Kind::I32, fmt!("A number value for Argd.")),
        ],
        help:   Some(fmt!("Activate the d argument")),
        ..Default::default()
    });

    let mut cmd1 = Cmd::from(CmdConfig {
        name:   fmt!("cmd1"),
        help:   Some(fmt!("Do the first thing")),
        vals:   vec![
            (Kind::U8, fmt!("A number value for cmd1.")),
        ],
        ..Default::default()
    });
    cmd1 = res!(cmd1.add_arg(a1.required(true)));
    cmd1 = res!(cmd1.add_arg(a2));

    let mut cmd2 = Cmd::from(CmdConfig {
        name:   fmt!("cmd2"),
        help:   Some(fmt!("Do the second thing")),
        ..Default::default()
    });
    cmd2 = res!(cmd2.add_arg(a3));

    let mut p = Syntax::from(SyntaxConfig {
        name:   fmt!("TestSyntax"),
        ver:    SemVer::new(0, 1, 0),
        about:  Some(fmt!("Testing")),
        ..Default::default()
    });

    p = res!(p.add_arg(a4));
    p = res!(p.add_cmd(cmd1));
    p = res!(p.add_cmd(cmd2));

    Ok(SyntaxRef::new(p))
}

pub fn test_msg(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["Default help", "all", "help"], || {
        let p = res!(Syntax::new("test").with_default_help_arg());
        let msgrx = Msg::new(SyntaxRef::new(p));
        let msgrx = res!(msgrx.from_str("help", None));
        //for line in Stringer::new(fmt!("{:?}", msgrx)).to_lines("  ") {
        //    debug!("{}", line);
        //}
        assert!(msgrx.has_arg("help"));
        Ok(())
    }));

    res!(test_it(filter, &["Msgrx read simple 000", "all", "msgrx"], || {
        let p = res!(make_test_syntax_00());

        if let Some(arg) = p.args.get_recursive(&Key::from("-b")) {
            req!(arg.config().name, "Arg_b".to_string());
        }

        let msgrx = Msg::new(p);
        let msgrx = res!(msgrx.from_str("-a av1 -b cmd1 --c1a3 --c1a1 \" c1a1v1  \" (I16|-945)", None));

        if let Some(list) = msgrx.get_arg_vals("Arg_a") {
            req!(*list, vec![ dat!("av1") ]);
        } else {
            return Err(err!(
                "Could not find the argument '-a'.";
            Input, Missing));
        }

        if let Some(cmdrx) = msgrx.get_cmd("cmd1") {
            if let Some(list) = cmdrx.get_arg_vals("Arg_c1a1") {
                req!(*list, vec![ dat!(" c1a1v1  "), dat!(-945i16) ]);
            } else {
                return Err(err!(
                    "Could not find the command 'cmd1' argument '-c'.";
                Input, Missing));
            }
        } else {
            return Err(err!(
                "Could not find the command 'cmd1'.";
            Input, Missing));
        }
        Ok(())
    }));

    res!(test_it(filter, &["Msgrx read simple 001", "all", "msgrx"], || {

        let p = res!(make_test_syntax_00());
        let msgrx = Msg::new(p);

        match msgrx.from_str("-a av1 av2 -b cmd1 --c1a3 --c1a1 \" c1a1v1  \" (I16|-945)", None) {
            Ok(_) => Err(err!(
                "The additional value 'av2' should have triggered an error.";
            Invalid, Output)),
            _ => Ok(()),
        }
    }));

    res!(test_it(filter, &["Msgrx read simple 002", "all", "msgrx"], || {

        let p = res!(make_test_syntax_00());
        let msgrx = Msg::new(p);

        match msgrx.from_str("-a -b cmd1 --c1a3 --c1a1 \" c1a1v1  \" (I16|-945)", None) {
            Ok(_) => Err(err!(
                "The appearance of -b in the place of an expected value \
                for -a should trigger an error.";
            Invalid, Output)),
            _ => Ok(()),
        }
    }));

    res!(test_it(filter, &["Msgrx read simple 003", "all", "msgrx"], || {

        let p = res!(make_test_syntax_00());
        let msgrx = Msg::new(p);

        match msgrx.from_str("-b cmd1 --c1a3 --c1a1 \" c1a1v1  \" (I16|-945)", None) {
            Ok(_) => Err(err!(
                "The missing message argument Arg_a should trigger an error";
            Invalid, Output)),
            _ => Ok(()),
        }
    }));

    res!(test_it(filter, &["Msgrx read simple 004", "all", "msgrx"], || {

        let p = res!(make_test_syntax_00());
        let msgrx = Msg::new(p);

        match msgrx.from_str("-a av1 -b cmd2", None) {
            Ok(_) => Err(err!(
                "The unknown word cmd2 should trigger an error.";
            Invalid, Output)),
            _ => Ok(()),
        }
    }));

    res!(test_it(filter, &["Msgrx empty arg", "all", "msgrx"], || {
        let mut p = Syntax::from(SyntaxConfig {
            name:   fmt!("TestSyntax"),
            ver:    SemVer::new(0, 1, 0),
            about:  Some(fmt!("Testing")),
            ..Default::default()
        });
        let a1 = Arg::from(ArgConfig {
            name:   fmt!("Arg_a"),
            hyph1:  fmt!("a"),
            hyph2:  Some(fmt!("a0")),
            reqd:   false,
            vals:   vec![
                (Kind::Str, fmt!("A string value for Arg_a.")),
            ],
            help:   Some(fmt!("Some help text")),
            ..Default::default()
        });
        let a2 = Arg::from(ArgConfig {
            name:   fmt!("Arg_b"),
            hyph1:  fmt!("b"),
            hyph2:  Some(fmt!("b0")),
            reqd:   false,
            help:   Some(fmt!("Some help text")),
            ..Default::default()
        });
        p = res!(p.add_arg(a1));
        p = res!(p.add_arg(a2));
        let p = SyntaxRef::new(p);
        let msgrx = Msg::new(p);

        if msgrx.from_str("-a -b", None).is_ok() {
            return Err(err!(
                "The syntax incorrectly accepted an empty -a value.";
            Invalid, Output));
        }

        let msgrx = res!(msgrx.from_str("Arg_a", None));
        if let Some(argrx) = &msgrx.end.arg {
            if argrx == &fmt!("Arg_a") {
                if msgrx.end.vals.len() != 1 {
                    return Err(err!(
                        "The message processor correctly identified the \
                        incomplete argument but found {} outstanding \
                        arg values, when 1 was expected",
                        msgrx.end.vals.len();
                    Invalid, Output));
                }
            } else {
                return Err(err!(
                    "The message processor incorrectly identified the \
                    outstanding argument as {}, it should have been '-a'",
                    argrx;
                Invalid, Output));
            }
        } else {
            return Err(err!(
                "The message processor did not detect the incomplete argument: {:?}",
                msgrx.end;
            Invalid, Output));
        }
        Ok(())
    }));

    // p v v
    res!(test_it(filter, &["Msgrx 010", "all", "msgrx"], || {
        let p = Syntax::new("TestSyntax").expected_vals(vec![
            (Kind::Str, fmt!("A string value for TestSyntax.")),
            (Kind::U16, fmt!("A number value for TestSyntax.")),
        ]);
        let p = SyntaxRef::new(p);
        let msgrx = Msg::new(p);

        let msgrx = res!(msgrx.from_str("hello    420    ", None));
        req!(msgrx.vals, vec![dat!("hello"), dat!(420u16)]);  

        let msgrx = res!(msgrx.from_str("  hello  ", None));
        if msgrx.end.vals.len() == 0 {
            return Err(err!(
                "The message processor did not detect the outstanding cmd value.";
            Invalid, Output));
        }

        let msgrx = res!(msgrx.from_str("  ", None));
        if msgrx.end.vals.len() != 2 {
            return Err(err!(
                "The message processor did not detect the outstanding msg values.";
            Invalid, Output));
        }

        Ok(())
    }));

    // p v v a v v
    res!(test_it(filter, &["Msgrx 020", "all", "msgrx"], || {
        let mut p = Syntax::from(SyntaxConfig {
            name:   fmt!("TestSyntax"),
            vals:   vec![
                (Kind::Str, fmt!("A string value for TestSyntax.")),
                (Kind::U8, fmt!("A number value for TestSyntax.")),
            ],
            ..Default::default()
        });
        let a = Arg::from(ArgConfig {
            name:   fmt!("Arg_a"),
            hyph1:  fmt!("a"),
            hyph2:  Some(fmt!("a0")),
            reqd:   false,
            vals:   vec![
                (Kind::Str, fmt!("A string value for Arg_a.")),
                (Kind::I8, fmt!("A number value for Arg_a.")),
            ],
            ..Default::default()
        });
        p = res!(p.add_arg(a));
        let p = SyntaxRef::new(p);
        let msgrx = Msg::new(p);

        let msgrx = res!(msgrx.from_str("hello 42 -a goodbye 1", None));
        req!(msgrx.vals, vec![dat!("hello"), dat!(42u8)]);  
        let argrx_vals = res!(msgrx.get_arg_vals("Arg_a").ok_or(err!(
            "Failed to detect message argument '-a'."), Invalid, Output)));
        req!(argrx_vals.len(), 2);
        req!(argrx_vals[0], dat!("goodbye"));
        req!(argrx_vals[1], dat!(1i8));  

        let msgrx = res!(msgrx.from_str("hello   42 --a0  goodbye ", None));
        if let Some(argrx) = &msgrx.end.arg {
            if argrx == &fmt!("Arg_a") {
                if msgrx.end.vals.len() != 1 {
                    return Err(err!(
                        "The message processor correctly identified the \
                        incomplete argument but found {} outstanding \
                        arg values, when 1 was expected.",
                        msgrx.end.vals.len();
                    Invalid, Output));
                }
            } else {
                return Err(err!(
                    "The message processor incorrectly identified the \
                    outstanding argument as {}, it should have been '-a'.",
                    argrx;
                Invalid, Output));
            }
        } else {
            return Err(err!(
                "The message processor did not detect the incomplete argument: {:?}",
                msgrx.end;
            Invalid, Output));
        }
        Ok(())
    }));

    // p v v a v v c v v
    res!(test_it(filter, &["Msgrx 030", "all", "msgrx"], || {
        let mut p = Syntax::from(SyntaxConfig {
            name:   fmt!("TestSyntax"),
            vals:   vec![
                (Kind::Str, fmt!("A string value for TestSyntax.")),
                (Kind::U8, fmt!("A number value for TestSyntax.")),
            ],
            ..Default::default()
        });
        let a = Arg::from(ArgConfig {
            name:   fmt!("Arg_a"),
            hyph1:  fmt!("a"),
            hyph2:  Some(fmt!("a0")),
            reqd:   false,
            vals:   vec![
                (Kind::Str, fmt!("A string value for Arg_a.")),
                (Kind::I8, fmt!("A number value for Arg_a.")),
            ],
            ..Default::default()
        });
        let c = Cmd::from(CmdConfig {
            name:   fmt!("cmd"),
            vals:   vec![
                (Kind::Str, fmt!("A string value for cmd.")),
                (Kind::I16, fmt!("A number value for cmd.")),
            ],
            ..Default::default()
        });
        p = res!(p.add_arg(a));
        p = res!(p.add_cmd(c));
        let p = SyntaxRef::new(p);
        let msgrx = Msg::new(p);

        let msgrx = res!(msgrx.from_str("hello 42 -a goodbye 1 cmd again -3", None));
        req!(msgrx.vals, vec![dat!("hello"), dat!(42u8)]);  
        let argrx_vals = res!(msgrx.get_arg_vals("Arg_a").ok_or(err!(
            "Failed to detect message argument '-a'."), Invalid, Output)));
        req!(argrx_vals.len(), 2);
        req!(argrx_vals[0], dat!("goodbye"));
        req!(argrx_vals[1], dat!(1i8));  
        let cmdrx = res!(msgrx.get_cmd("cmd").ok_or(err!(
            "Failed to detect command 'cmd'."), Invalid, Output)));
        req!(cmdrx.vals.len(), 2);
        req!(cmdrx.vals[0], dat!("again"));
        req!(cmdrx.vals[1], dat!(-3i16));  

        let msgrx = res!(msgrx.from_str("hello   42 --a0  goodbye 1 cmd again", None));
        if let Some(cmdrx) = &msgrx.end.cmd {
            if cmdrx == &fmt!("cmd") {
                if msgrx.end.vals.len() != 1 {
                    return Err(err!(
                        "The message processor correctly identified the \
                        incomplete command but found {} outstanding \
                        cmd values, when 1 was expected.",
                        msgrx.end.vals.len();
                    Invalid, Output));
                }
            } else {
                return Err(err!(
                    "The message processor incorrectly identified the \
                    outstanding command as {}, it should have been 'cmd'.",
                    cmdrx;
                Invalid, Output));
            }
        } else {
            return Err(err!(
                "The message processor did not detect the incomplete command: {:?}.",
                msgrx.end;
            Invalid, Output));
        }

        let msgrx = res!(msgrx.from_str("hello   42 --a0  goodbye 1 cmd ", None));
        if let Some(cmdrx) = &msgrx.end.cmd {
            if cmdrx == &fmt!("cmd") {
                if msgrx.end.vals.len() != 2 {
                    return Err(err!(
                        "The message processor correctly identified the \
                        incomplete command but found {} outstanding \
                        cmd values, when 2 was expected.",
                        msgrx.end.vals.len();
                    Invalid, Output));
                }
            } else {
                return Err(err!(
                    "The message processor incorrectly identified the \
                    outstanding command as {}, it should have been 'cmd'.",
                    cmdrx;
                Invalid, Output));
            }
        } else {
            return Err(err!(
                "The message processor did not detect the incomplete command: {:?}.",
                msgrx.end;
            Invalid, Output));
        }

        Ok(())
    }));

    // p v v a v v c v v a v v
    res!(test_it(filter, &["Msgrx 040", "all", "msgrx"], || {
        let mut p = Syntax::from(SyntaxConfig {
            name:   fmt!("TestSyntax"),
            vals:   vec![
                (Kind::Str, fmt!("A string value for TestSyntax.")),
                (Kind::I128, fmt!("A number value for TestSyntax.")),
            ],
            ..Default::default()
        });
        let a = Arg::from(ArgConfig {
            name:   fmt!("Arg_a"),
            hyph1:  fmt!("a"),
            hyph2:  Some(fmt!("a0")),
            reqd:   false,
            vals:   vec![
                (Kind::Str, fmt!("A string value for Arg_a.")),
                (Kind::I32, fmt!("A number value for Arg_a.")),
            ],
            ..Default::default()
        });
        p = res!(p.add_arg(a));

        let mut c = Cmd::from(CmdConfig {
            name:   fmt!("cmd"),
            vals:   vec![
                (Kind::Str, fmt!("A string value for cmd.")),
                (Kind::I16, fmt!("A number value for cmd.")),
            ],
            ..Default::default()
        });
        let a = Arg::from(ArgConfig {
            name:   fmt!("Arg_b"),
            hyph1:  fmt!("b"),
            hyph2:  Some(fmt!("b0")),
            reqd:   false,
            vals:   vec![
                (Kind::Str, fmt!("A string value for Arg_b.")),
                (Kind::U8, fmt!("A number value for Arg_b.")),
            ],
            ..Default::default()
        });
        c = res!(c.add_arg(a));
        p = res!(p.add_cmd(c));
        let p = SyntaxRef::new(p);
        let msgrx = Msg::new(p);

        let msgrx = res!(msgrx.from_str("hello 42 -a goodbye 1 cmd again -3 -b dejavu 42", None));
        req!(msgrx.vals, vec![dat!("hello"), dat!(42i128)]);  
        let argrx_vals = res!(msgrx.get_arg_vals("Arg_a").ok_or(err!(
            "Failed to detect message argument '-a'."), Invalid, Output)));
        req!(argrx_vals.len(), 2);
        req!(argrx_vals[0], dat!("goodbye"));
        req!(argrx_vals[1], dat!(1i32));  
        let cmdrx = res!(msgrx.get_cmd("cmd").ok_or(err!(
            "Failed to detect command 'cmd'."), Invalid, Output)));
        req!(cmdrx.vals.len(), 2);
        req!(cmdrx.vals[0], dat!("again"));
        req!(cmdrx.vals[1], dat!(-3i16));  
        let argrx_vals = res!(cmdrx.get_arg_vals("Arg_b").ok_or(err!(
            "Failed to detect command 'cmd' argument '-b'."), Invalid, Output)));
        req!(argrx_vals.len(), 2);
        req!(argrx_vals[0], dat!("dejavu"));
        req!(argrx_vals[1], dat!(42u8));  

        let msgrx = res!(msgrx.from_str("hello   42 --a0  goodbye 1 cmd again -3 -b dejavu  ", None));
        if let Some(argrx) = &msgrx.end.arg {
            if argrx == &fmt!("Arg_b") {
                if msgrx.end.vals.len() != 1 {
                    return Err(err!(
                        "The message processor correctly identified the \
                        incomplete argument but found {} outstanding \
                        arg values, when 1 was expected.",
                        msgrx.end.vals.len();
                    Invalid, Output));
                }
            } else {
                return Err(err!(
                    "The message processor incorrectly identified the \
                    outstanding argument as {}, it should have been '-b'.",
                    argrx;
                Invalid, Output));
            }
        } else {
            return Err(err!(
                "The message processor did not detect the incomplete argument: {:?}.",
                msgrx.end;
            Invalid, Output));
        }

        let msgrx = res!(msgrx.from_str("hello   42 --a0  goodbye 1 cmd again -3 -b", None));
        if let Some(argrx) = &msgrx.end.arg {
            if argrx == &fmt!("Arg_b") {
                if msgrx.end.vals.len() != 2 {
                    return Err(err!(
                        "The message processor correctly identified the \
                        incomplete argument but found {} outstanding \
                        arg values, when 2 was expected.",
                        msgrx.end.vals.len();
                    Invalid, Output));
                }
            } else {
                return Err(err!(
                    "The message processor incorrectly identified the \
                    outstanding argument as {}, it should have been '-b'.",
                    argrx;
                Invalid, Output));
            }
        } else {
            return Err(err!(
                "The message processor did not detect the incomplete argument: {:?}.",
                msgrx.end;
            Invalid, Output));
        }

        Ok(())
    }));

    // test multiple commands
    // p v c v c v v a a v
    res!(test_it(filter, &["Msgrx 100", "all", "msgrx"], || {
        let mut p = Syntax::from(SyntaxConfig {
            name:   fmt!("TestSyntax"),
            vals:   vec![
                (Kind::U8, fmt!("A number value for TestSyntax.")),
            ],
            ..Default::default()
        });
        let c = Cmd::from(CmdConfig {
            name:   fmt!("cmd1"),
            vals:   vec![
                (Kind::Str, fmt!("A string value for cmd1.")),
            ],
            ..Default::default()
        });
        p = res!(p.add_cmd(c));

        let mut c = Cmd::from(CmdConfig {
            name:   fmt!("cmd2"),
            vals:   vec![
                (Kind::Str, fmt!("A string value for cmd2.")),
                (Kind::I8, fmt!("A number value for cmd2.")),
            ],
            ..Default::default()
        });
        let a1 = Arg::from(ArgConfig {
            name:   fmt!("arg1"),
            hyph1:  fmt!("a1"),
            ..Default::default()
        });
        let a2 = Arg::from(ArgConfig {
            name:   fmt!("arg2"),
            hyph1:  fmt!("a2"),
            hyph2:  Some(fmt!("arg2")),
            vals:   vec![
                (Kind::Str, fmt!("A string value for arg2.")),
            ],
            ..Default::default()
        });
        c = res!(c.add_arg(a1));
        c = res!(c.add_arg(a2));
        p = res!(p.add_cmd(c));
        let p = SyntaxRef::new(p);
        let msgrx = Msg::new(p);

        let msgrx = res!(msgrx.from_str("42 cmd1 hello cmd2 goodbye -42 -a1 --arg2 done    ", None));
        req!(msgrx.vals, vec![dat!(42u8)]);  
        let cmdrx = res!(msgrx.get_cmd("cmd1").ok_or(err!(
            "Failed to detect command 'cmd1'."), Invalid, Output)));
        req!(cmdrx.vals.len(), 1);
        req!(cmdrx.vals[0], dat!("hello"));
        let cmdrx = res!(msgrx.get_cmd("cmd2").ok_or(err!(
            "Failed to detect command 'cmd2'."), Invalid, Output)));
        req!(cmdrx.vals.len(), 2);
        req!(cmdrx.vals[0], dat!("goodbye"));
        req!(cmdrx.vals[1], dat!(-42i8));
        assert!(cmdrx.has_arg("arg1"));
        let argrx_vals = res!(cmdrx.get_arg_vals("arg2").ok_or(err!(
            "Failed to detect command 'cmd2' argument '-a2'.";
        Invalid, Output)));
        req!(argrx_vals.len(), 1);
        req!(argrx_vals[0], dat!("done"));

        if msgrx.from_str("hello   42 --a0  goodbye 1 cmd again -3 -b dejavu  ", None).is_ok() {
            return Err(err!(
                "The syntax incorrectly accepted too few values.";
            Invalid, Output));
        }

        if msgrx.from_str("hello   42 --a0  goodbye 1 cmd again -3 -b", None).is_ok() {
            return Err(err!(
                "The syntax incorrectly accepted too few values.";
            Invalid, Output));
        }
        Ok(())
    }));

    res!(test_it(filter, &["Msgrx 110", "all", "msgrx"], || {
        let mut p = Syntax::from(SyntaxConfig {
            name:   fmt!("repl"),
            ver:    SemVer::new(0, 1, 0),
            about:  Some(fmt!("Demonstration REPL")),
            ..Default::default()
        });
        p = res!(p.with_default_help_cmd());

        let c = Cmd::from(CmdConfig {
            name:   fmt!("cd"),
            vals:   vec![
                (Kind::Str, fmt!("A string value for cd.")),
            ],
            help:   Some(fmt!("Change directory")),
            ..Default::default()
        });
        p = res!(p.add_cmd(c));

        let c = Cmd::from(CmdConfig {
            name:   fmt!("pwd"),
            help:   Some(fmt!("Print path of current/working directory")),
            ..Default::default()
        });
        p = res!(p.add_cmd(c));

        let c = Cmd::from(CmdConfig {
            name:   fmt!("who"),
            help:   Some(fmt!("Dump variable names and values")),
            ..Default::default()
        });
        p = res!(p.add_cmd(c));

        let mut c = Cmd::from(CmdConfig {
            name:   fmt!("ls"),
            ..Default::default()
        });
        let a1 = Arg::from(ArgConfig {
            name:   fmt!("sort"),
            hyph1:  fmt!("s"),
            vals:   vec![
                (Kind::Str, fmt!("A string value for sort.")),
            ],
            help:   Some(fmt!("Sort by 'size', 'type' or 'name'")),
            ..Default::default()
        });
        let a2 = Arg::from(ArgConfig {
            name:   fmt!("reverse"),
            hyph1:  fmt!("r"),
            help:   Some(fmt!("Reverse any sort")),
            ..Default::default()
        });
        c = res!(c.add_arg(a1));
        c = res!(c.add_arg(a2));
        p = res!(p.add_cmd(c));
        let p = SyntaxRef::new(p);
        let msgrx = Msg::new(p);

        let msg = "ls -r -s name";
        let msgrx = res!(msgrx.from_str(msg, None));
        if let Some(cmdrx) = msgrx.get_cmd("ls") {
            req!(cmdrx.vals.len(), 0);
            if let Some(argrx_vals) = cmdrx.get_arg_vals("sort") {
                req!(argrx_vals.len(), 1);
                req!(argrx_vals[0], dat!("name"));
            } else {
                return Err(err!(
                    "Expected capture of command 'ls' argument '-s'.";
                Invalid, Output));
            }
            if cmdrx.get_arg_vals("-r").is_some() {
                return Err(err!(
                    "Expected capture of command 'ls' argument '-r' with no values.";
                Invalid, Output));
            }
        } else {
            return Err(err!(
                "Expected capture of command 'ls'.";
            Invalid, Output));
        }

        Ok(())
    }));

    res!(test_it(filter, &["Binary msgrx 010", "all", "msgrx", "binary"], || {
        let mut p = Syntax::from(SyntaxConfig {
            name:   fmt!("TestSyntax"),
            vals:   vec![
                (Kind::I16, fmt!("A number value for TestSyntax.")),
            ],
            ..Default::default()
        });
        let a = Arg::from(ArgConfig {
            name:   fmt!("arg00"),
            hyph1:  fmt!("a00"),
            ..Default::default()
        }).required(true);
        p = res!(p.add_arg(a));

        let mut c = Cmd::from(CmdConfig {
            name:   fmt!("cmd1"),
            vals:   vec![
                (Kind::Str, fmt!("A string value for cmd1.")),
            ],
            ..Default::default()
        });
        let a = Arg::from(ArgConfig {
            name:   fmt!("arg10"),
            hyph1:  fmt!("a10"),
            ..Default::default()
        });
        c = res!(c.add_arg(a));
        p = res!(p.add_cmd(c)); // 0

        let mut c = Cmd::from(CmdConfig {
            name:   fmt!("cmd2"),
            vals:   vec![
                (Kind::Str, fmt!("A string value for cmd2.")),
                (Kind::I32, fmt!("A number value for cmd2.")),
            ],
            ..Default::default()
        });
        let a1 = Arg::from(ArgConfig {
            name:   fmt!("arg20"),
            hyph1:  fmt!("a20"),
            ..Default::default()
        });
        let a2 = Arg::from(ArgConfig {
            name:   fmt!("arg21"),
            hyph1:  fmt!("a21"),
            hyph2:  Some(fmt!("arg21")),
            vals:   vec![
                (Kind::Str, fmt!("A string value for arg21.")),
            ],
            ..Default::default()
        });
        c = res!(c.add_arg(a1)); // 0
        c = res!(c.add_arg(a2)); // 1
        p = res!(p.add_cmd(c)); // 1
        let p = SyntaxRef::new(p);
        let msgrx = Msg::new(p);

        let msg = "(I16|42) -a00 cmd1 hello cmd2 goodbye (I32|-42) -a20 --arg21 done    ";
        let mut msgtx = res!(msgrx.from_str(msg, None));
        // Side check: Msg validator.  In this case from_str validates, but 
        // in theory a binary Msg can be constructed and may need to be validated.
        res!(msgtx.validate());
        let mut buf = Vec::new();
        msgtx.set_encoding(Encoding::Binary);
        buf = res!(msgtx.to_bytes(buf));
        debug!("String: {}", msg);
        debug!("From string: {}", msgtx);
        for line in msgtx.to_lines() {
            debug!("{}", line);
        }
        let msgrx = res!(msgrx.from_bytes(&buf, None));
        debug!("Rx: {}", msgrx);
        req!(msgrx.vals.len(), 1);
        req!(msgtx.vals[0], Dat::I16(42));
        req!(msgrx.args.len(), 1);
        req!(msgrx.cmds.len(), 2);
        match msgrx.get_cmd("cmd1") {
            Some(msgcmd) => {
                req!(msgcmd.vals.len(), 1);
                req!(msgcmd.vals[0], Dat::from("hello"));
                req!(msgcmd.args.len(), 0);
            },
            None => return Err(err!(
                "Expected command 'cmd1'.";
            Input, Invalid)),
        }
        match msgrx.get_cmd("cmd2") {
            Some(msgcmd) => {
                req!(msgcmd.vals.len(), 2);
                req!(msgcmd.vals[0], Dat::from("goodbye"));
                req!(msgcmd.vals[1], Dat::from(-42i32));
                req!(msgcmd.args.len(), 2);
                if msgcmd.get_arg_vals("arg20").is_some() {
                    return Err(err!(
                        "Expected command 'cmd2' to have an argument \
                        '-a20' but no values.";
                    Input, Invalid));
                }
                match msgcmd.get_arg_vals("arg21") {
                    Some(vals) => {
                        req!(vals.len(), 1);
                        req!(vals[0], Dat::from("done"));
                    },
                    None => return Err(err!(
                        "Expected command 'cmd2' to have an argument '-a21'.";
                    Input, Invalid)),
                }
            },
            None => return Err(err!(
                "Expected command 'cmd2'.";
            Input, Invalid)),
        }

        Ok(())
    }));
    
    res!(test_it(filter, &["Msg display 010", "all", "display"], || {
        let mut p = Syntax::from(SyntaxConfig {
            name:   fmt!("TestSyntax"),
            vals:   vec![
                (Kind::I16, fmt!("A number value for TestSyntax.")),
            ],
            ..Default::default()
        });
        let a = Arg::from(ArgConfig {
            name:   fmt!("arg00"),
            hyph1:  fmt!("a00"),
            ..Default::default()
        });
        p = res!(p.add_arg(a));

        let mut c = Cmd::from(CmdConfig {
            name:   fmt!("cmd1"),
            vals:   vec![
                (Kind::Str, fmt!("A string value for cmd1.")),
            ],
            ..Default::default()
        });
        let a = Arg::from(ArgConfig {
            name:   fmt!("arg10"),
            hyph1:  fmt!("a10"),
            ..Default::default()
        });
        c = res!(c.add_arg(a));
        p = res!(p.add_cmd(c));

        let mut c = Cmd::from(CmdConfig {
            name:   fmt!("cmd2"),
            vals:   vec![
                (Kind::Str, fmt!("A string value for cmd2.")),
                (Kind::I32, fmt!("A number value for cmd2.")),
            ],
            ..Default::default()
        });
        let a1 = Arg::from(ArgConfig {
            name:   fmt!("arg20"),
            hyph1:  fmt!("a20"),
            ..Default::default()
        }).required(true);
        let a2 = Arg::from(ArgConfig {
            name:   fmt!("arg21"),
            hyph1:  fmt!("a21"),
            hyph2:  Some(fmt!("arg21")),
            vals:   vec![
                (Kind::Str, fmt!("A string value for arg21.")),
            ],
            ..Default::default()
        });
        c = res!(c.add_arg(a1));
        c = res!(c.add_arg(a2));
        p = res!(p.add_cmd(c));
        let p = SyntaxRef::new(p);
        let msgrx = Msg::new(p);

        let msg = "(I16|42) -a00 cmd1 hello cmd2 goodbye (I32|-42) -a20 --arg21 done    ";

        debug!("Tx: {}", msg);
        let msgrx = res!(msgrx.from_str(
            "(I16|42) -a00 cmd1 hello cmd2 goodbye (I32|-42) -a20 --arg21 done    ", None
        ));
        debug!("Rx: {}", msgrx);
        //for line in msgrx.to_lines() {
        //    debug!("{}", line);
        //}
        let msgrx2 = res!(msgrx.from_str(&msgrx.to_string(), None));
        debug!("Tx: {}", msgrx);
        debug!("Rx: {}", msgrx2);
        //for line in msgrx2.to_lines() {
        //    debug!("{}", line);
        //}

        Ok(())
    }));

    res!(test_it(filter, &["Msg build 010", "all", "build"], || {
        let p = res!(make_test_syntax_01());
        let mut msg = Msg::new(p);
        let expected = "-d (i32|-42) cmd1 (u8|3) -a (str|\"hello\") (i16|42) -b cmd2 -c (str|\"world\")"; 

        msg = res!(msg.add_arg("-d")); // Redundant, just making sure these two work together.
        msg = res!(msg.add_arg_val("-d", Some(dat!(-42i32))));

        let mut cmd2 = res!(msg.new_cmd("cmd2"));
        //cmd2 = res!(cmd2.add_arg("-c"));
        cmd2 = res!(cmd2.add_arg_val("-c", Some(dat!("world"))));
        msg = res!(msg.add_cmd(cmd2));

        let mut cmd1 = res!(msg.new_cmd("cmd1"));
        cmd1 = res!(cmd1.add_cmd_val(dat!(3u8)));
        cmd1 = res!(cmd1.add_arg_val("-a", Some(dat!("hello"))));
        cmd1 = res!(cmd1.add_arg_val("-a", Some(dat!(42i16))));
        cmd1 = res!(cmd1.add_arg("-b"));
        msg = res!(msg.add_cmd(cmd1));

        let result = fmt!("{}", msg);
        debug!("Expected   : {}", expected);
        debug!("Constructed: {}", result);
        req!(&result, expected);

        Ok(())
    }));

    Ok(())
}
