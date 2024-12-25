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

/// Works with the `oxedize_fe2o3_iop_db::Database` trait.
pub fn generic_database(
    mut s:      Syntax,
    mcid_kind:  Kind,
    uid_kind:   Kind,
)
    -> Outcome<Syntax>
{
    // ┌───────────────────────┐
    // │ DATABASE              │
    // └───────────────────────┘
    
    let arg_msg_cmd_id = Arg::from(ArgConfig {
        name:   fmt!("msg_cmd_id"),
        hyph1:  fmt!("mcid"),
        vals:   vec![(mcid_kind, fmt!("Message command identifier value"))],
        reqd:   true,
        help:   Some(fmt!("Message command identifier")),
        ..Default::default()
    });
    let arg_usr_id = Arg::from(ArgConfig {
        name:   fmt!("usr_id"),
        hyph1:  fmt!("uid"),
        vals:   vec![(uid_kind, fmt!("User identifier value"))],
        reqd:   true,
        help:   Some(fmt!("User identifier")),
        ..Default::default()
    });

    // ---------------------------------------------------------------------------------------------
    // Command: insert
    // ---------------------------------------------------------------------------------------------
    let mut cmd = Cmd::from(CmdConfig {
        name:   fmt!("insert"),
        help:   Some(fmt!("Insert (key, value) daticles into database")),
        vals:   vec![
            (Kind::Unknown, fmt!("Key")),
            (Kind::Unknown, fmt!("Value")),
        ],
        cat:    fmt!("Database"),
        ..Default::default()
    });
    cmd = res!(cmd.add_arg(arg_msg_cmd_id.clone()));
    cmd = res!(cmd.add_arg(arg_usr_id.clone()));
    s = res!(s.add_cmd(cmd));
    // =============================================================================================

    // ---------------------------------------------------------------------------------------------
    // Command: get
    // ---------------------------------------------------------------------------------------------
    let mut cmd = Cmd::from(CmdConfig {
        name:   fmt!("get"),
        help:   Some(fmt!("Get database value for given key daticle")),
        vals:   vec![(Kind::Unknown, fmt!("Key"))],
        cat:    fmt!("Database"),
        ..Default::default()
    });
    cmd = res!(cmd.add_arg(arg_msg_cmd_id.clone()));
    s = res!(s.add_cmd(cmd));
    // =============================================================================================

    // ---------------------------------------------------------------------------------------------
    // Command: delete
    // ---------------------------------------------------------------------------------------------
    let mut cmd = Cmd::from(CmdConfig {
        name:   fmt!("delete"),
        help:   Some(fmt!("Delete given key from database")),
        vals:   vec![(Kind::Unknown, fmt!("Key"))],
        cat:    fmt!("Database"),
        ..Default::default()
    });
    cmd = res!(cmd.add_arg(arg_msg_cmd_id.clone()));
    cmd = res!(cmd.add_arg(arg_usr_id.clone()));
    s = res!(s.add_cmd(cmd));
    // =============================================================================================
    
    Ok(s)
}
