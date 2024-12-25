use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_jdat::{
    prelude::*,
    version::SemVer,
};
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
    },
};

pub fn new(
    name:   &str,
    ver:    SemVer,
    about:  &str,
)
    -> Outcome<SyntaxRef>
{
    let mut s = Syntax::new(name).ver(ver).about(about);
    s = res!(s.with_default_help_cmd());

    // ┌───────────────────────┐
    // │ CONTROL               │
    // └───────────────────────┘
    
    // ---------------------------------------------------------------------------------------------
    // Command: exit
    // ---------------------------------------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("exit"),
        help:   Some(fmt!("Shutdown the app and exit, or use Ctrl+C, Ctrl+D")),
        cat:    fmt!("Control"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));
    // =============================================================================================

    // =============================================================================================
    
    // ┌───────────────────────┐
    // │ FILE SYSTEM           │
    // └───────────────────────┘
    
    // ---------------------------------------------------------------------------------------------
    // Command: cd
    // ---------------------------------------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("cd"),
        help:   Some(fmt!("Change directory")),
        evals:  vec![Kind::Str],
        cat:    fmt!("File system"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));
    // =============================================================================================

    // ---------------------------------------------------------------------------------------------
    // Command: ls
    // ---------------------------------------------------------------------------------------------
    let mut cmd = Cmd::from(CmdConfig {
        name:   fmt!("ls"),
        help:   Some(fmt!("List directory contents")),
        cat:    fmt!("File system"),
        ..Default::default()
    });
    let a1 = Arg::from(ArgConfig {
        name:   fmt!("sort"),
        hyph1:  fmt!("s"),
        evals:  vec![Kind::Str],
        help:   Some(fmt!("Sort by 'size', 'type' or 'name'")),
        ..Default::default()
    });
    let a2 = Arg::from(ArgConfig {
        name:   fmt!("bytes"),
        hyph1:  fmt!("b"),
        help:   Some(fmt!("Show size in bytes (otherwise humanised)")),
        ..Default::default()
    });
    let a3 = Arg::from(ArgConfig {
        name:   fmt!("reverse"),
        hyph1:  fmt!("r"),
        help:   Some(fmt!("Reverse any sort")),
        ..Default::default()
    });
    cmd = res!(cmd.add_arg(a1));
    cmd = res!(cmd.add_arg(a2));
    cmd = res!(cmd.add_arg(a3));
    s = res!(s.add_cmd(cmd));
    // =============================================================================================

    // ---------------------------------------------------------------------------------------------
    // Command: pwd
    // ---------------------------------------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("pwd"),
        help:   Some(fmt!("Print path of current/working directory")),
        cat:    fmt!("File system"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));
    // =============================================================================================

    // =============================================================================================
    
    // ┌───────────────────────┐
    // │ WORKSPACE             │
    // └───────────────────────┘
    
    // ---------------------------------------------------------------------------------------------
    // Command: vars
    // ---------------------------------------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("vars"),
        help:   Some(fmt!("Display variable names and values")),
        cat:    fmt!("Workspace"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));
    // =============================================================================================

    Ok(SyntaxRef::new(s))
}
