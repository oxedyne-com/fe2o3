use crate::{
    Syntax,
    arg::{
        Arg,
        ArgConfig,
    },
    cmd::{
        Cmd,
        CmdConfig,
    },
};

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_jdat::prelude::*;

pub fn generic_file_system(
    mut s: Syntax,
)
    -> Outcome<Syntax>
{
    // ┌───────────────────────┐
    // │ FILE SYSTEM           │
    // └───────────────────────┘
    
    // ---------------------------------------------------------------------------------------------
    // Command: cd
    // ---------------------------------------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("cd"),
        help:   Some(fmt!("Change directory")),
        vals:   vec![(Kind::Str, fmt!("New directory path"))],
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
        vals:   vec![(Kind::Str, fmt!("Sort directive 'size', 'type' or 'name'"))],
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
    
    Ok(s)
}
