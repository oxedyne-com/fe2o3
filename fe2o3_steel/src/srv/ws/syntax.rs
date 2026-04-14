use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    version::SemVer,
};
use oxedyne_fe2o3_syntax::{
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
            help:   Some(fmt!("Insert a key-value pair into the database.")),
            vals:   vec![
                (Kind::Unknown, fmt!("Key")),
                (Kind::Unknown, fmt!("Value")),
            ],
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

        // ┌───────────────────────┐
        // │ SESSION IO            │
        // └───────────────────────┘

        // ---------------------------------------------------------------------------------------------
        // Command: sess_get
        // ---------------------------------------------------------------------------------------------
        // Reads a value from the caller's session-scoped keyspace. The
        // server automatically prefixes the caller's session id (taken from
        // the HttpOnly session cookie attached at the WebSocket upgrade),
        // so the client cannot cross into another user's namespace.
        let cmd = Cmd::from(CmdConfig {
            name:   fmt!("sess_get"),
            help:   Some(fmt!("Read a value from the caller's session-scoped storage.")),
            vals:   vec![(Kind::Str, fmt!("Key (string)"))],
            cat:    fmt!("Session IO"),
            ..Default::default()
        });
        s = res!(s.add_cmd(cmd));

        // ---------------------------------------------------------------------------------------------
        // Command: sess_put
        // ---------------------------------------------------------------------------------------------
        // Writes a (key, value) pair into the caller's session-scoped
        // keyspace. The key must be a string; the value can be any Dat.
        let cmd = Cmd::from(CmdConfig {
            name:   fmt!("sess_put"),
            help:   Some(fmt!("Write a (key, value) pair into session-scoped storage.")),
            vals:   vec![
                (Kind::Str, fmt!("Key")),
                (Kind::Unknown, fmt!("Value")),
            ],
            cat:    fmt!("Session IO"),
            ..Default::default()
        });
        s = res!(s.add_cmd(cmd));
        // =============================================================================================

        // ┌───────────────────────┐
        // │ USER IO               │
        // └───────────────────────┘

        // ---------------------------------------------------------------------------------------------
        // Command: user_get
        // ---------------------------------------------------------------------------------------------
        // Reads a value from the authenticated user's keyspace. The server
        // looks up the session's bound user via `sess_meta:<sid>` and
        // prefixes the request with `user:<username>:`. Rejects when the
        // session is not authenticated.
        let cmd = Cmd::from(CmdConfig {
            name:   fmt!("user_get"),
            help:   Some(fmt!("Read a value from the authenticated user's storage.")),
            vals:   vec![(Kind::Str, fmt!("Key (string)"))],
            cat:    fmt!("User IO"),
            ..Default::default()
        });
        s = res!(s.add_cmd(cmd));

        // ---------------------------------------------------------------------------------------------
        // Command: user_put
        // ---------------------------------------------------------------------------------------------
        // Writes a (key, value) pair into the authenticated user's
        // keyspace. The server scopes the key by the session-bound user
        // and rejects if the session is not authenticated.
        let cmd = Cmd::from(CmdConfig {
            name:   fmt!("user_put"),
            help:   Some(fmt!("Write a (key, value) pair into user-scoped storage.")),
            vals:   vec![
                (Kind::Str, fmt!("Key")),
                (Kind::Unknown, fmt!("Value")),
            ],
            cat:    fmt!("User IO"),
            ..Default::default()
        });
        s = res!(s.add_cmd(cmd));
        // =============================================================================================

        // ┌───────────────────────┐
        // │ AUTH                  │
        // └───────────────────────┘

        // ---------------------------------------------------------------------------------------------
        // Command: register
        // ---------------------------------------------------------------------------------------------
        // Create a new user record. The passphrase is hashed with Argon2id
        // before storage; the plain passphrase is never written to disk.
        let cmd = Cmd::from(CmdConfig {
            name:   fmt!("register"),
            help:   Some(fmt!("Create a new user record keyed by username.")),
            vals:   vec![
                (Kind::Str, fmt!("Username")),
                (Kind::Str, fmt!("Passphrase")),
            ],
            cat:    fmt!("Auth"),
            ..Default::default()
        });
        s = res!(s.add_cmd(cmd));

        // ---------------------------------------------------------------------------------------------
        // Command: login
        // ---------------------------------------------------------------------------------------------
        // Verify credentials and bind the current session to a user. A
        // successful login writes a `sess_meta:<sid>` record recording
        // which user the session is now acting as.
        let cmd = Cmd::from(CmdConfig {
            name:   fmt!("login"),
            help:   Some(fmt!("Verify credentials and bind the current session to the user.")),
            vals:   vec![
                (Kind::Str, fmt!("Username")),
                (Kind::Str, fmt!("Passphrase")),
            ],
            cat:    fmt!("Auth"),
            ..Default::default()
        });
        s = res!(s.add_cmd(cmd));

        // ---------------------------------------------------------------------------------------------
        // Command: logout
        // ---------------------------------------------------------------------------------------------
        // Clear the session-to-user binding for the caller's session. The
        // underlying HttpOnly cookie is not touched; the session remains
        // anonymous until the next successful login.
        let cmd = Cmd::from(CmdConfig {
            name:   fmt!("logout"),
            help:   Some(fmt!("Clear the session-to-user binding for this session.")),
            cat:    fmt!("Auth"),
            ..Default::default()
        });
        s = res!(s.add_cmd(cmd));

        // ---------------------------------------------------------------------------------------------
        // Command: whoami
        // ---------------------------------------------------------------------------------------------
        // Report whether the caller's session is authenticated and, if so,
        // under which username. The session id itself is never returned.
        let cmd = Cmd::from(CmdConfig {
            name:   fmt!("whoami"),
            help:   Some(fmt!("Report the authenticated user for this session, if any.")),
            cat:    fmt!("Auth"),
            ..Default::default()
        });
        s = res!(s.add_cmd(cmd));
        // =============================================================================================

        Ok(SyntaxRef::new(s))
    }
}
