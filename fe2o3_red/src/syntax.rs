//! Red chat protocol syntax — command definitions for the
//! `/chat` WebSocket endpoint.
//!
//! Red owns its syntax independently of Steel.  Steel routes
//! WebSocket connections with path `/chat` to the Red handler;
//! Red then creates and uses its own `SyntaxRef` for parsing
//! incoming commands and constructing outgoing responses.
//!
//! Command categories:
//!
//! - **Red** — session management and chat (`session_new`, `chat`, …)
//! - **Response** — server-to-client responses (`data`, `text`, `done`, …)
//!
//! The `data`, `error`, and `info` commands are general-purpose
//! response commands also found in Steel's syntax.  They are
//! duplicated here so Red is self-contained and does not depend
//! on Steel's syntax definitions.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    version::SemVer,
};
use oxedyne_fe2o3_syntax::{
    cmd::{
        Cmd,
        CmdConfig,
    },
    core::{
        Syntax,
        SyntaxRef,
    },
};

/// Build the Red chat protocol syntax.
///
/// Defines all commands the Red WS handler can receive from the
/// client and all commands it can send back.  The version should
/// be bumped when the protocol changes incompatibly.
pub fn build_syntax()
    -> Outcome<SyntaxRef>
{
    let mut s = Syntax::new("red-chat")
        .ver(SemVer::new(0, 1, 0))
        .about("Red AI agent chat protocol");
    s = res!(s.with_default_help_cmd());

    // ┌───────────────────────┐
    // │ RED — SESSION MGMT    │
    // └───────────────────────┘

    // ----------------------------------------------------------------
    // Command: session_new
    // ----------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("session_new"),
        help:   Some(fmt!("Create a new chat session.")),
        vals:   vec![
            (Kind::Str, fmt!("Session name (optional)")),
            (Kind::Str, fmt!("Model (optional)")),
        ],
        cat:    fmt!("Red"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));

    // ----------------------------------------------------------------
    // Command: session_list
    // ----------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("session_list"),
        help:   Some(fmt!("List all chat sessions for the current user.")),
        cat:    fmt!("Red"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));

    // ----------------------------------------------------------------
    // Command: session_switch
    // ----------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("session_switch"),
        help:   Some(fmt!("Switch to a session by ID.")),
        vals:   vec![(Kind::Str, fmt!("Session ID"))],
        cat:    fmt!("Red"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));

    // ----------------------------------------------------------------
    // Command: session_close
    // ----------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("session_close"),
        help:   Some(fmt!("Delete a session by ID.")),
        vals:   vec![(Kind::Str, fmt!("Session ID"))],
        cat:    fmt!("Red"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));

    // ----------------------------------------------------------------
    // Command: session_rename
    // ----------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("session_rename"),
        help:   Some(fmt!("Rename a session.")),
        vals:   vec![
            (Kind::Str, fmt!("Session ID")),
            (Kind::Str, fmt!("New name")),
        ],
        cat:    fmt!("Red"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));

    // ----------------------------------------------------------------
    // Command: chat
    // ----------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("chat"),
        help:   Some(fmt!("Send a message to the current session's agent.")),
        vals:   vec![(Kind::Str, fmt!("Message content"))],
        cat:    fmt!("Red"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));

    // ----------------------------------------------------------------
    // Commands: fs_list / fs_read / fs_delete / fs_write (file browser)
    // ----------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("fs_list"),
        help:   Some(fmt!("List a workspace directory.")),
        vals:   vec![(Kind::Str, fmt!("Directory path"))],
        cat:    fmt!("Red"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));

    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("fs_read"),
        help:   Some(fmt!("Read a workspace text file.")),
        vals:   vec![(Kind::Str, fmt!("File path"))],
        cat:    fmt!("Red"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));

    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("fs_delete"),
        help:   Some(fmt!("Delete a workspace file.")),
        vals:   vec![(Kind::Str, fmt!("File path"))],
        cat:    fmt!("Red"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));

    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("fs_write"),
        help:   Some(fmt!("Create or overwrite a workspace file.")),
        vals:   vec![
            (Kind::Str, fmt!("File path")),
            (Kind::Str, fmt!("Content")),
        ],
        cat:    fmt!("Red"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));

    // ┌───────────────────────┐
    // │ RESPONSE              │
    // └───────────────────────┘

    // ----------------------------------------------------------------
    // Command: data  (server → client: structured data)
    // ----------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("data"),
        help:   Some(fmt!("Data payload (JSON or structured).")),
        vals:   vec![(Kind::Unknown, fmt!("Data"))],
        cat:    fmt!("Response"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));

    // ----------------------------------------------------------------
    // Command: text  (server → client: streamed LLM token)
    // ----------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("text"),
        help:   Some(fmt!("Streamed text token from the LLM.")),
        vals:   vec![(Kind::Str, fmt!("Text content"))],
        cat:    fmt!("Response"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));

    // ----------------------------------------------------------------
    // Command: tool_call  (server → client: agent is invoking a tool)
    // ----------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("tool_call"),
        help:   Some(fmt!("The agent is invoking a tool.")),
        vals:   vec![
            (Kind::Str, fmt!("Tool name")),
            (Kind::Str, fmt!("JSON arguments")),
        ],
        cat:    fmt!("Response"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));

    // ----------------------------------------------------------------
    // Command: tool_result  (server → client: a tool returned)
    // ----------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("tool_result"),
        help:   Some(fmt!("A tool returned its result.")),
        vals:   vec![
            (Kind::Str, fmt!("Tool name")),
            (Kind::Str, fmt!("Result text")),
        ],
        cat:    fmt!("Response"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));

    // ----------------------------------------------------------------
    // Command: fs_tree  (server → client: directory listing JSON)
    // ----------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("fs_tree"),
        help:   Some(fmt!("Workspace directory listing (JSON).")),
        vals:   vec![(Kind::Str, fmt!("JSON entries"))],
        cat:    fmt!("Response"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));

    // ----------------------------------------------------------------
    // Command: fs_content  (server → client: file contents)
    // ----------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("fs_content"),
        help:   Some(fmt!("Workspace file contents.")),
        vals:   vec![
            (Kind::Str, fmt!("File path")),
            (Kind::Str, fmt!("Content")),
        ],
        cat:    fmt!("Response"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));

    // ----------------------------------------------------------------
    // Command: done  (server → client: agent turn complete)
    // ----------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("done"),
        help:   Some(fmt!("Agent turn complete.")),
        cat:    fmt!("Response"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));

    // ----------------------------------------------------------------
    // Command: info  (server → client: informational message)
    // ----------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("info"),
        help:   Some(fmt!("Informational message.")),
        vals:   vec![(Kind::Str, fmt!("Information message"))],
        cat:    fmt!("Response"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));

    // ----------------------------------------------------------------
    // Command: error  (server → client: error message)
    // ----------------------------------------------------------------
    let cmd = Cmd::from(CmdConfig {
        name:   fmt!("error"),
        help:   Some(fmt!("Error message.")),
        vals:   vec![(Kind::Str, fmt!("Error message"))],
        cat:    fmt!("Response"),
        ..Default::default()
    });
    s = res!(s.add_cmd(cmd));

    Ok(SyntaxRef::new(s))
}
