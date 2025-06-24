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
    },
    key::Key,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    map::MapRec,
    test::test_it,
};
use oxedyne_fe2o3_jdat::{
    prelude::*,
    version::SemVer,
};

pub fn test_core(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["Arg value", "all", "arg"], || {
        let arg = Arg::from(ArgConfig {
            name:   fmt!("test"),
            vals:   vec![(Kind::C64, fmt!("A test kind."))],
            ..Default::default()
        });
        req!(arg.config().vals.len(), 1);
        req!(arg.config().vals[0].0, Kind::C64);
        Ok(())
    }));

    res!(test_it(filter, &["Set retrieve", "all", "arg"], || {
        let mut p = Syntax::from(SyntaxConfig {
            name:   fmt!("TestSyntax"),
            ver:    SemVer::new(0, 1, 0),
            about:  Some(fmt!("Testing")),
            ..Default::default()
        });
        let mut c = Cmd::from(CmdConfig {
            name:   fmt!("print"),
            help:   Some(fmt!("Execute printing")),
            ..Default::default()
        });
        let a = Arg::from(ArgConfig {
            name:   fmt!("colour"),
            hyph1:  fmt!("c"),
            hyph2:  Some(fmt!("c")),
            reqd:   false,
            help:   Some(fmt!("Use colour")),
            ..Default::default()
        });
        c = res!(c.add_arg(a));
        p = res!(p.add_cmd(c));
            
        req!(p.config().name, "TestSyntax".to_string());
        req!(p.config().ver, SemVer::new(0, 1, 0));
        req!(p.config().about, Some("Testing".to_string()));

        if let Some(cmd) = p.cmds.get_recursive(&Key::from("print")) {
            if let Some(arg) = cmd.args.get_recursive(&Key::from("colour")) {
                req!(arg.config().hyph1, fmt!("c"));
            } else {
                panic!("Could not find the argument 'colour'");
            }
        } else {
            panic!("Could not find the command 'print'");
        }
        Ok(())
    }));

    Ok(())
}
