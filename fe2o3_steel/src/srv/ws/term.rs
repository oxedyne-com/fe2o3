//! Terminal bridge — manages tmux sessions and bridges I/O to
//! WebSocket binary frames.
//!
//! Two layers:
//!
//! 1. **Management commands** (sync, via the text WS handler):
//!    `term_new`, `term_list`, `term_close`, `term_set_name`.
//!    These are thin wrappers around the `tmux` CLI and return
//!    syntax-protocol responses.
//!
//! 2. **Terminal I/O bridge** (async, via a separate WS path
//!    `/term/<session>`):  spawns `tmux attach` as a child process
//!    with piped stdin/stdout and bidirectionally pipes bytes
//!    between the child and the WebSocket.  The client sends
//!    keystrokes as binary WS frames; the server pushes terminal
//!    output as binary WS frames.
//!
//! No PTY is needed on our side — tmux already manages the PTY for
//! the child process (e.g. Goose).  When we `tmux attach`, tmux
//! connects us to that terminal via its own protocol over
//! stdin/stdout pipes.  We simply bridge bytes.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    id::NumIdDat,
};
use oxedyne_fe2o3_net::ws::core::{WebSocket, WebSocketMessage};

use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;

use std::{
    process::Command,
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command as TokioCommand,
};


// ┌───────────────────────────────────────────────────────────────┐
// │ TerminalManager                                                │
// └───────────────────────────────────────────────────────────────┘

/// Manages terminal sessions via tmux.
///
/// All management methods are synchronous because they are called
/// from the sync `handle_text` WS handler.  The tmux CLI calls are
/// quick (list/create/kill sessions) so blocking briefly is
/// acceptable.
///
/// The terminal I/O bridge (`handle_terminal_websocket`) is async
/// and runs in a separate WS connection.
#[derive(Clone, Debug)]
pub struct TerminalManager {
    /// Prefix for tmux session names (e.g. "goose-").
    session_prefix:   String,
    /// Command to launch in new sessions (e.g. "goose session").
    launch_command:   String,
}

impl TerminalManager {

    pub fn new(session_prefix: &str, launch_command: &str) -> Self {
        Self {
            session_prefix:   session_prefix.to_string(),
            launch_command:   launch_command.to_string(),
        }
    }

    /// Create a new tmux session running the launch command.
    /// Returns the session name (e.g. "goose-3").
    pub fn new_session(&self) -> Outcome<String> {
        let max = self.list_session_nums()?.iter().max().copied().unwrap_or(0);
        let name = fmt!("{}{}", self.session_prefix, max + 1);
        let status = res!(Command::new("tmux")
            .arg("new-session")
            .arg("-d")
            .arg("-s")
            .arg(&name)
            .arg(&self.launch_command)
            .status());
        if !status.success() {
            return Err(err!(
                "tmux new-session failed for '{}'.", name;
                IO, System));
        }
        Ok(name)
    }

    /// List active tmux sessions with our prefix.
    /// Returns a JDAT map: { "sessions": [{ "name": "goose-1" }, ...] }
    pub fn list_sessions_dat(&self) -> Outcome<Dat> {
        let names = res!(self.list_session_names());
        let mut arr = Vec::new();
        for n in names {
            let mut m = DaticleMap::new();
            m.insert(dat!("name"), dat!(n));
            arr.push(Dat::List(vec![Dat::Map(m)]));
        }
        let mut out = DaticleMap::new();
        out.insert(dat!("sessions"), Dat::List(arr));
        Ok(Dat::Map(out))
    }

    /// Close (kill) a tmux session by name.
    pub fn close_session(&self, name: &str) -> Outcome<()> {
        let status = res!(Command::new("tmux")
            .arg("kill-session")
            .arg("-t")
            .arg(name)
            .status());
        if !status.success() {
            return Err(err!(
                "tmux kill-session failed for '{}'.", name;
                IO, System));
        }
        Ok(())
    }

    /// Rename a tmux session.
    pub fn set_session_name(&self, old: &str, new: &str) -> Outcome<()> {
        let status = res!(Command::new("tmux")
            .arg("rename-session")
            .arg("-t")
            .arg(old)
            .arg(new)
            .status());
        if !status.success() {
            return Err(err!(
                "tmux rename-session failed '{}' -> '{}'.", old, new;
                IO, System));
        }
        Ok(())
    }

    // ── Internal helpers ──────────────────────────────────────

    /// List tmux session names matching our prefix.
    fn list_session_names(&self) -> Outcome<Vec<String>> {
        let output = match Command::new("tmux")
            .arg("list-sessions")
            .arg("-F")
            .arg("#{session_name}")
            .output()
        {
            Ok(o) => o,
            Err(_) => {
                // tmux not running or no sessions — return empty.
                return Ok(Vec::new());
            }
        };
        if !output.status.success() {
            return Ok(Vec::new());
        }
        let names: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|l| l.starts_with(&self.session_prefix))
            .map(|l| l.to_string())
            .collect();
        Ok(names)
    }

    /// List session numbers (e.g. [1, 2, 3] for goose-1, goose-2, goose-3).
    fn list_session_nums(&self) -> Outcome<Vec<u32>> {
        let names = res!(self.list_session_names());
        let nums: Vec<u32> = names
            .iter()
            .filter_map(|n| {
                n.strip_prefix(&self.session_prefix)
                    .and_then(|s| s.parse().ok())
            })
            .collect();
        Ok(nums)
    }
}


// ┌───────────────────────────────────────────────────────────────┐
// │ Terminal I/O bridge — piped child ↔ WebSocket                 │
// └───────────────────────────────────────────────────────────────┘

/// Bridge a WebSocket connection to a tmux session via piped I/O.
///
/// Spawns `tmux attach -t <session>` as a child process with piped
/// stdin/stdout/stderr.  Bytes from the child's stdout/stderr are
/// forwarded as binary WS messages to the client; binary WS
/// messages from the client are written to the child's stdin as
/// keystrokes.
///
/// tmux manages the PTY for the child process (e.g. Goose) — we
/// only bridge bytes between the WebSocket and the tmux attach
/// process.  The tmux session persists after the WebSocket
/// disconnects; reconnecting with the same session name
/// reattaches.
pub async fn handle_terminal_websocket<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    DB:     Database<UIDL, UID, ENC, KH> + 'static,
    S:      tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
>(
    mut stream:     S,
    session_name:   String,
    request:        oxedyne_fe2o3_net::http::msg::HttpMessage,
    id:             &String,
)
    -> Outcome<()>
{
    // ── WebSocket handshake ───────────────────────────────────
    let mut ws: WebSocket<
        '_,
        UIDL, UID, ENC, KH, DB,
        S,
        oxedyne_fe2o3_net::ws::handler::WebSocketEchoHandler,
    > = WebSocket::new_server(
        &mut stream,
        oxedyne_fe2o3_net::ws::handler::WebSocketEchoHandler,
        crate::srv::constant::WEBSOCKET_CHUNK_SIZE,
        crate::srv::constant::WEBSOCKET_CHUNKING_THRESHOLD,
    );
    match ws.connect_as_server(request).await {
        Ok(()) => (),
        Err(e) => return Err(err!(e,
            "{}: Terminal WS handshake failed.", id;
            IO, Network, Wire)),
    }

    // ── Spawn tmux attach with piped I/O ──────────────────────
    let mut child = match TokioCommand::new("tmux")
        .arg("attach-session")
        .arg("-t")
        .arg(&session_name)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env("TERM", "xterm-256color")
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            // Send an error message before closing.
            let err_msg = WebSocketMessage::Text(
                fmt!("error \"tmux attach failed: {}\"", e));
            let _ = ws.send(&err_msg).await;
            return Err(err!(e,
                "{}: Failed to spawn tmux attach for '{}'.", id, session_name;
                IO, System));
        }
    };

    let mut child_stdin = child.stdin.take()
        .ok_or_else(|| err!(
            "{}: Failed to capture tmux stdin.", id;
            IO, System))?;
    let mut child_stdout = child.stdout.take()
        .ok_or_else(|| err!(
            "{}: Failed to capture tmux stdout.", id;
            IO, System))?;

    // ── Bidirectional pipe loop ───────────────────────────────
    let mut pty_buf = vec![0u8; 16384];

    loop {
        tokio::select! {
            // ── Read from WebSocket → write to tmux stdin ──────
            ws_read = ws.read() => {
                match ws_read {
                    Ok(Some(WebSocketMessage::Binary(byts))) => {
                        if let Err(e) = child_stdin.write_all(&byts).await {
                            warn!("{}: tmux stdin write error: {}", id, e);
                            break;
                        }
                        let _ = child_stdin.flush().await;
                    }
                    Ok(Some(WebSocketMessage::Text(txt))) => {
                        // Allow text messages as keystrokes too
                        // (some clients send text instead of binary).
                        if let Err(e) = child_stdin.write_all(txt.as_bytes()).await {
                            warn!("{}: tmux stdin write error: {}", id, e);
                            break;
                        }
                        let _ = child_stdin.flush().await;
                    }
                    Ok(Some(WebSocketMessage::Close(_, _))) => {
                        debug!("{}: Terminal WS client closed.", id);
                        break;
                    }
                    Ok(None) => {
                        debug!("{}: Terminal WS client disconnected.", id);
                        break;
                    }
                    Ok(_) => (), // Ping/Pong handled by read().
                    Err(e) => {
                        warn!("{}: Terminal WS read error: {}", id, e);
                        break;
                    }
                }
            }
            // ── Read from tmux stdout → send as binary WS ──────
            n = child_stdout.read(&mut pty_buf) => {
                match n {
                    Ok(0) => {
                        // tmux attach exited — session detached or ended.
                        debug!("{}: tmux stdout EOF for '{}'.", id, session_name);
                        break;
                    }
                    Ok(n) => {
                        let msg = WebSocketMessage::Binary(pty_buf[..n].to_vec());
                        if let Err(e) = ws.send(&msg).await {
                            warn!("{}: WS send error: {}", id, e);
                            break;
                        }
                    }
                    Err(e) => {
                        warn!("{}: tmux stdout read error: {}", id, e);
                        break;
                    }
                }
            }
        }
    }

    // ── Cleanup ───────────────────────────────────────────────
    // Kill the tmux attach process if still running.  The tmux
    // session itself persists for reconnection.
    let _ = child.kill().await;
    let _ = child.wait().await;
    let _ = ws.send(&WebSocketMessage::Close(None, Some("session ended".to_string()))).await;
    debug!("{}: Terminal bridge closed for '{}'.", id, session_name);
    Ok(())
}
