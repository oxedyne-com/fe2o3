//use crate::srv::id;

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_jdat::{
    prelude::*,
    version::SemVer,
};
use oxedize_fe2o3_syntax::{
    self,
    Syntax,
    SyntaxRef,
    arg::{
        Arg,
        ArgConfig,
    },
    cmd::{
        Cmd,
        CmdConfig,
    },
};


pub fn new_shell(
    name:   &str,
    ver:    &SemVer,
    about:  &str,
)
    -> Outcome<SyntaxRef>
{
    let mut s = Syntax::new(name).ver(*ver).about(about);
    s = res!(s.with_default_help_cmd());
    s = res!(oxedize_fe2o3_syntax::apps::file::generic_file_system(s));

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

    // ---------------------------------------------------------------------------------------------
    // Command: shell
    // ---------------------------------------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("shell"),
        help:   Some(fmt!("Start the app shell")),
        cat:    fmt!("Control"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));
    // =============================================================================================

    // ---------------------------------------------------------------------------------------------
    // Command: server
    // ---------------------------------------------------------------------------------------------
    let mut cmd = Cmd::from(CmdConfig {
        name:   fmt!("server"),
        help:   Some(fmt!("Start the app HTTPS server")),
        cat:    fmt!("Control"),
        ..Default::default()
    });
    let a1 = Arg::from(ArgConfig {
        name:   fmt!("dev"),
        hyph1:  fmt!("d"),
        vals:   vec![],
        reqd:   false,
        help:   Some(fmt!("Run server in developer mode.")),
        ..Default::default()
    });
    cmd = res!(cmd.add_arg(a1));
    s = res!(s.add_cmd(cmd));
    // =============================================================================================

    // ---------------------------------------------------------------------------------------------
    // Command: cert
    // ---------------------------------------------------------------------------------------------
    let mut cmd = Cmd::from(CmdConfig {
        name:   fmt!("cert"),
        help:   Some(fmt!("Manage TLS certificates")),
        cat:    fmt!("TLS"),
        ..Default::default()
    });
    let a1 = Arg::from(ArgConfig {
        name:   fmt!("create-dev"),
        hyph1:  fmt!("d"),
        vals:   vec![],
        reqd:   false,
        help:   Some(fmt!("Create self-signed certificates for development.")),
        ..Default::default()
    });
    let a2 = Arg::from(ArgConfig {
        name:   fmt!("create-prod"),
        hyph1:  fmt!("p"),
        vals:   vec![(Kind::Vek, fmt!("Domain name(s), overriding config.jdat."))],
        reqd:   false,
        help:   Some(fmt!("Create production certificates using system Let's Encrypt.")),
        ..Default::default()
    });
    cmd = res!(cmd.add_arg(a1));
    cmd = res!(cmd.add_arg(a2));
    s = res!(s.add_cmd(cmd));
    
    // ┌───────────────────────┐
    // │ WALLET                │
    // └───────────────────────┘
    // ---------------------------------------------------------------------------------------------
    // Command: secrets
    // ---------------------------------------------------------------------------------------------
    let mut cmd = Cmd::from(CmdConfig {
        name:   fmt!("secrets"),
        help:   Some(fmt!("Manage wallet encrypted secrets.")),
        cat:    fmt!("Wallet"),
        ..Default::default()
    });
    let a1 = Arg::from(ArgConfig {
        name:   fmt!("create"),
        hyph1:  fmt!("c"),
        vals:   vec![(Kind::Str, fmt!("Name of secret for indexing"))],
        reqd:   false,
        help:   Some(fmt!("Interactively create a new encrypted secret.")),
        ..Default::default()
    });
    let a2 = Arg::from(ArgConfig {
        name:   fmt!("recover"),
        hyph1:  fmt!("r"),
        vals:   vec![(Kind::Str, fmt!("Name of secret for indexing"))],
        reqd:   false,
        help:   Some(fmt!("Interactively recover an encrypted secret.")),
        ..Default::default()
    });
    cmd = res!(cmd.add_arg(a1));
    cmd = res!(cmd.add_arg(a2));
    s = res!(s.add_cmd(cmd));
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
