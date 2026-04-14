//use crate::srv::id;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    version::SemVer,
};
use oxedyne_fe2o3_syntax::{
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


/// Build the shell Syntax tree wrapped in a `SyntaxRef`.
///
/// Most callers want this. App binaries that need to inject their
/// own commands through `AppExtension::extend_syntax` go through
/// `new_shell_raw` instead so they can mutate the tree before it is
/// shared.
pub fn new_shell(
    name:   &str,
    ver:    &SemVer,
    about:  &str,
)
    -> Outcome<SyntaxRef>
{
    let s = res!(new_shell_raw(name, ver, about));
    Ok(SyntaxRef::new(s))
}

/// Build the shell Syntax tree without wrapping it in a `SyntaxRef`,
/// so the caller can hand it to `AppExtension::extend_syntax` for
/// further population before sharing.
pub fn new_shell_raw(
    name:   &str,
    ver:    &SemVer,
    about:  &str,
)
    -> Outcome<Syntax>
{
    let mut s = Syntax::new(name).ver(*ver).about(about);
    s = res!(s.with_default_help_cmd());
    s = res!(oxedyne_fe2o3_syntax::apps::file::generic_file_system(s));

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
    cmd = res!(cmd.add_arg(a1));
    s = res!(s.add_cmd(cmd));
    // =============================================================================================

    // ---------------------------------------------------------------------------------------------
    // Command: acme
    // ---------------------------------------------------------------------------------------------
    let mut cmd = Cmd::from(CmdConfig {
        name:   fmt!("acme"),
        help:   Some(fmt!("Manage ACME (Let's Encrypt) certificate state")),
        cat:    fmt!("TLS"),
        ..Default::default()
    });
    let a1 = Arg::from(ArgConfig {
        name:   fmt!("status"),
        hyph1:  fmt!("s"),
        vals:   vec![],
        reqd:   false,
        help:   Some(fmt!("Print the configured ACME state and vhost hostnames.")),
        ..Default::default()
    });
    let a2 = Arg::from(ArgConfig {
        name:   fmt!("renew"),
        hyph1:  fmt!("r"),
        vals:   vec![],
        reqd:   false,
        help:   Some(fmt!("Clear the ACME cache so certs are re-issued on next start-up.")),
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
    
    // ---------------------------------------------------------------------------------------------
    // Command: wallet
    // ---------------------------------------------------------------------------------------------
    let mut cmd = Cmd::from(CmdConfig {
        name:   fmt!("wallet"),
        help:   Some(fmt!("Manage the wallet file.")),
        cat:    fmt!("Wallet"),
        ..Default::default()
    });
    let a1 = Arg::from(ArgConfig {
        name:   fmt!("migrate"),
        hyph1:  fmt!("m"),
        hyph2:  Some(fmt!("migrate")),
        vals:   vec![],
        reqd:   false,
        help:   Some(fmt!("One-shot migrate a pre-admin-user wallet (the legacy \
            app_hashes / wallet_pass_hashes layout) into the multi-admin layout, \
            preserving the existing master key so no Ozone re-encryption is \
            required.")),
        ..Default::default()
    });
    cmd = res!(cmd.add_arg(a1));
    s = res!(s.add_cmd(cmd));
    // =============================================================================================

    // ---------------------------------------------------------------------------------------------
    // Command: admin
    // ---------------------------------------------------------------------------------------------
    let mut cmd = Cmd::from(CmdConfig {
        name:   fmt!("admin"),
        help:   Some(fmt!("Manage wallet admin entries (add, remove, list).")),
        cat:    fmt!("Wallet"),
        ..Default::default()
    });
    let a1 = Arg::from(ArgConfig {
        name:   fmt!("add"),
        hyph1:  fmt!("a"),
        hyph2:  Some(fmt!("add")),
        vals:   vec![(Kind::Str, fmt!("New admin name"))],
        reqd:   false,
        help:   Some(fmt!("Add a new admin entry wrapping the wallet master \
            key under a freshly prompted password. The running session must \
            have been unlocked by an admin holding the 'admin' scope.")),
        ..Default::default()
    });
    let a2 = Arg::from(ArgConfig {
        name:   fmt!("remove"),
        hyph1:  fmt!("r"),
        hyph2:  Some(fmt!("remove")),
        vals:   vec![(Kind::Str, fmt!("Admin name to remove"))],
        reqd:   false,
        help:   Some(fmt!("Remove an existing admin entry by name. The last \
            remaining admin cannot be removed.")),
        ..Default::default()
    });
    let a3 = Arg::from(ArgConfig {
        name:   fmt!("list"),
        hyph1:  fmt!("l"),
        hyph2:  Some(fmt!("list")),
        vals:   vec![],
        reqd:   false,
        help:   Some(fmt!("List the names, scopes and expiry of every admin \
            entry in the wallet.")),
        ..Default::default()
    });
    let a4 = Arg::from(ArgConfig {
        name:   fmt!("scopes"),
        hyph1:  fmt!("s"),
        hyph2:  Some(fmt!("scopes")),
        vals:   vec![(Kind::Str, fmt!("Comma-separated verb list"))],
        reqd:   false,
        help:   Some(fmt!("Scopes for a new admin, comma-separated (e.g. \
            'restart,log,probe'). Use '*' for operator-level access. Defaults \
            to an empty scope list, which can unlock the wallet but cannot \
            invoke any verb.")),
        ..Default::default()
    });
    let a5 = Arg::from(ArgConfig {
        name:   fmt!("expires-in"),
        hyph1:  fmt!("e"),
        hyph2:  Some(fmt!("expires-in")),
        vals:   vec![(Kind::U64, fmt!("Seconds until expiry"))],
        reqd:   false,
        help:   Some(fmt!("Expire the new admin after N seconds from now. \
            A value of 0 (the default) means 'never expires'.")),
        ..Default::default()
    });
    let a6 = Arg::from(ArgConfig {
        name:   fmt!("passwd"),
        hyph1:  fmt!("p"),
        hyph2:  Some(fmt!("passwd")),
        vals:   vec![],
        reqd:   false,
        help:   Some(fmt!("Change the caller's own admin password in place. \
            The caller is whichever admin unlocked the wallet at start-up. \
            Scopes and expiry are preserved; only the wrap is replaced. \
            Prompts for the new password twice.")),
        ..Default::default()
    });
    cmd = res!(cmd.add_arg(a1));
    cmd = res!(cmd.add_arg(a2));
    cmd = res!(cmd.add_arg(a3));
    cmd = res!(cmd.add_arg(a4));
    cmd = res!(cmd.add_arg(a5));
    cmd = res!(cmd.add_arg(a6));
    s = res!(s.add_cmd(cmd));
    // =============================================================================================

    // ┌───────────────────────┐
    // │ MAIL                  │
    // └───────────────────────┘
    // ---------------------------------------------------------------------------------------------
    // Command: mailpass
    // ---------------------------------------------------------------------------------------------
    let mut cmd = Cmd::from(CmdConfig {
        name:   fmt!("mailpass"),
        help:   Some(fmt!("Hash an interactively-prompted password with Argon2id, ready \
            to paste into the mail users file.")),
        cat:    fmt!("Mail"),
        ..Default::default()
    });
    let a1 = Arg::from(ArgConfig {
        name:   fmt!("address"),
        hyph1:  fmt!("a"),
        vals:   vec![(Kind::Str, fmt!("Email address"))],
        reqd:   true,
        help:   Some(fmt!("Email address (local@domain) the password belongs to.")),
        ..Default::default()
    });
    let a2 = Arg::from(ArgConfig {
        name:   fmt!("delivery-dir"),
        hyph1:  fmt!("d"),
        vals:   vec![(Kind::Str, fmt!("Relative directory under maildir_root"))],
        reqd:   true,
        help:   Some(fmt!("Per-user mailbox directory, relative to maildir_root.")),
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

    Ok(s)
}
