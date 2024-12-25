use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_jdat::{
    prelude::*,
    version::SemVer,
};
use oxedize_fe2o3_syntax::{
//    arg::{
//        Arg,
//        ArgConfig,
//    },
    cmd::{
        Cmd,
        CmdConfig,
    },
    core::{
        Syntax,
        SyntaxRef,
    },
};

#[derive(Clone, Debug)]
pub struct WebSocketSyntax;

impl WebSocketSyntax {

    pub fn new(
        name:   &str,
        ver:    &SemVer,
        about:  &str,
    )
        -> Outcome<SyntaxRef>
    {
        let mut s = Syntax::new(name).ver(*ver).about(about);
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
    
        // ┌───────────────────────┐
        // │ DEVELOPMENT           │
        // └───────────────────────┘
        
        // ---------------------------------------------------------------------------------------------
        // Command: dev_connect
        // ---------------------------------------------------------------------------------------------
        let cmd = Cmd::from(CmdConfig {
            name:   fmt!("dev_connect"),
            help:   Some(fmt!("Initialize dev mode refresh connection.")),
            cat:    fmt!("Development"),
            ..Default::default()
        });
        s = res!(s.add_cmd(cmd));
        // ---------------------------------------------------------------------------------------------
        // Command: dev_ping
        // ---------------------------------------------------------------------------------------------
        let cmd = Cmd::from(CmdConfig {
            name:   fmt!("dev_ping"),
            help:   Some(fmt!("Keep dev mode connection alive.")),
            cat:    fmt!("Development"),
            ..Default::default()
        });
        s = res!(s.add_cmd(cmd));

        // ┌───────────────────────┐
        // │ GENERAL IO            │
        // └───────────────────────┘
        
        // ---------------------------------------------------------------------------------------------
        // Command: data
        // ---------------------------------------------------------------------------------------------
        let cmd = Cmd::from(CmdConfig {
            name:   fmt!("data"),
            help:   Some(fmt!("Data retrieved from database.")),
            vals:   vec![(Kind::Unknown, fmt!("Retrieved data"))],
            cat:    fmt!("Database IO"),
            ..Default::default()
        });
        s = res!(s.add_cmd(cmd));
        // ---------------------------------------------------------------------------------------------
        // Command: info
        // ---------------------------------------------------------------------------------------------
        let cmd = Cmd::from(CmdConfig {
            name:   fmt!("info"),
            help:   Some(fmt!("For your information.")),
            vals:   vec![(Kind::Str, fmt!("Information message"))],
            cat:    fmt!("General IO"),
            ..Default::default()
        });
        s = res!(s.add_cmd(cmd));
        // ---------------------------------------------------------------------------------------------
        // Command: error
        // ---------------------------------------------------------------------------------------------
        let cmd = Cmd::from(CmdConfig {
            name:   fmt!("error"),
            help:   Some(fmt!("Error.")),
            vals:   vec![(Kind::Str, fmt!("Error message"))],
            cat:    fmt!("General IO"),
            ..Default::default()
        });
        s = res!(s.add_cmd(cmd));
        // ---------------------------------------------------------------------------------------------
        // Command: echo
        // ---------------------------------------------------------------------------------------------
        let cmd = Cmd::from(CmdConfig {
            name:   fmt!("echo"),
            help:   Some(fmt!("Echo the incoming message.")),
            vals:   vec![(Kind::Str, fmt!("Text to echo"))],
            cat:    fmt!("General IO"),
            ..Default::default()
        });
        s = res!(s.add_cmd(cmd));
        // =============================================================================================
        
        // ┌───────────────────────┐
        // │ DATABASE IO           │
        // └───────────────────────┘
        
        // ---------------------------------------------------------------------------------------------
        // Command: insert
        // ---------------------------------------------------------------------------------------------
        let cmd = Cmd::from(CmdConfig {
            name:   fmt!("insert"),
            help:   Some(fmt!("Insert (key, value) daticles into database.")),
            vals:   vec![(Kind::Tup2, fmt!("(Key, Value)"))],
            cat:    fmt!("Database IO"),
            ..Default::default()
        });
        s = res!(s.add_cmd(cmd));
        // =============================================================================================
    
        // ---------------------------------------------------------------------------------------------
        // Command: get_data
        // ---------------------------------------------------------------------------------------------
        let cmd = Cmd::from(CmdConfig {
            name:   fmt!("get_data"),
            help:   Some(fmt!("Get database value for given key daticle.")),
            vals:   vec![(Kind::Unknown, fmt!("Key"))],
            cat:    fmt!("Database IO"),
            ..Default::default()
        });
        s = res!(s.add_cmd(cmd));
        // =============================================================================================
    
        Ok(SyntaxRef::new(s))
    }
}
