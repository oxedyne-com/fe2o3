//! Terminal bridge — manages tmux sessions and bridges PTY I/O to
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
//!    `/term/<session>`):  creates a PTY pair, spawns `tmux attach`
//!    with the slave end as stdin/stdout/stderr, and bridges bytes
//!    between the PTY master and the WebSocket.  The client sends
//!    keystrokes as binary WS frames; the server pushes terminal
//!    output as binary WS frames.
//!
//! tmux manages the PTY for the child process (e.g. Goose).  We
//! create our own PTY for the `tmux attach` process so it sees a
//! real terminal.  The tmux session persists after the WebSocket
//! disconnects; reconnecting reattaches.

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
    fs::File,
    os::fd::AsFd,
    process::{Command, Stdio},
};

use tokio::{
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
            arr.push(Dat::Map(m));
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
            Err(_) => return Ok(Vec::new()),
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
// │ Terminal I/O bridge — PTY ↔ WebSocket                         │
// └───────────────────────────────────────────────────────────────┘

/// Bridge a WebSocket connection to a tmux session via a PTY.
///
/// Creates a PTY pair, spawns `tmux attach -t <session>` with the
/// slave end as stdin/stdout/stderr (so tmux sees a real terminal),
/// and bridges bytes between the PTY master and the WebSocket:
///
/// - Client keystrokes (binary WS) → PTY master write
/// - PTY master read → terminal output (binary WS)
///
/// The tmux session persists after the WebSocket disconnects;
/// reconnecting with the same session name reattaches.
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

    // ── Create PTY ────────────────────────────────────────────
    let winsize = nix::pty::Winsize {
        ws_row:    24,
        ws_col:    80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let pty = match nix::pty::openpty(&Some(winsize), &None) {
        Ok(p) => p,
        Err(e) => {
            let err_msg = WebSocketMessage::Text(
                fmt!("error \"PTY creation failed: {}\"", e));
            let _ = ws.send(&err_msg).await;
            return Err(err!(e,
                "{}: Failed to create PTY for '{}'.", id, session_name;
                IO, System));
        }
    };

    // pty.master and pty.slave are OwnedFd (safe FD wrappers).

    // Dup the slave for each stdio stream using nix's safe API.
    let slave_stdin  = match nix::unistd::dup(pty.slave.as_fd()) {
        Ok(f) => f,
        Err(e) => return Err(err!(e, "{}: dup slave stdin failed.", id; IO, System)),
    };
    let slave_stdout = match nix::unistd::dup(pty.slave.as_fd()) {
        Ok(f) => f,
        Err(e) => return Err(err!(e, "{}: dup slave stdout failed.", id; IO, System)),
    };
    let slave_stderr = match nix::unistd::dup(pty.slave.as_fd()) {
        Ok(f) => f,
        Err(e) => return Err(err!(e, "{}: dup slave stderr failed.", id; IO, System)),
    };

    // ── Spawn tmux attach with the PTY slave ──────────────────
    let mut cmd = TokioCommand::new("tmux");
    cmd.arg("attach-session")
        .arg("-t")
        .arg(&session_name)
        .stdin(Stdio::from(slave_stdin))
        .stdout(Stdio::from(slave_stdout))
        .stderr(Stdio::from(slave_stderr))
        .env("TERM", "xterm-256color");

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let err_msg = WebSocketMessage::Text(
                fmt!("error \"tmux attach failed: {}\"", e));
            let _ = ws.send(&err_msg).await;
            return Err(err!(e,
                "{}: Failed to spawn tmux attach for '{}'.", id, session_name;
                IO, System));
        }
    };

    // ── Set master to non-blocking ────────────────────────────
    //
    // We must do this before wrapping the master in AsyncFd.
    // nix::fcntl::fcntl takes AsFd, so we pass the OwnedFd
    // before it is consumed by the File conversion below.
    if let Err(e) = nix::fcntl::fcntl(
        &pty.master,
        nix::fcntl::FcntlArg::F_SETFL(nix::fcntl::OFlag::O_NONBLOCK),
    ) {
        warn!("{}: Failed to set master non-blocking: {}", id, e);
    }

    // ── Wrap the master for async I/O ─────────────────────────
    //
    // Convert the master OwnedFd to a File for AsyncFd.  The
    // OwnedFd is consumed and the File takes ownership of the FD.
    let master_file: File = pty.master.into();
    let master_async = match tokio::io::unix::AsyncFd::new(master_file) {
        Ok(a) => a,
        Err(e) => {
            let _ = child.kill().await;
            return Err(err!(e,
                "{}: Failed to create AsyncFd for PTY master.", id;
                IO, System));
        }
    };

    // ── Bidirectional pipe loop ───────────────────────────────
    let mut buf = vec![0u8; 16384];

    loop {
        tokio::select! {
            // ── Read from WebSocket → write to PTY master ──────
            ws_read = ws.read() => {
                match ws_read {
                    Ok(Some(WebSocketMessage::Binary(byts))) => {
                        use std::io::Write;
                        if let Err(e) = (&*master_async.get_ref()).write(&byts) {
                            if e.kind() != std::io::ErrorKind::WouldBlock {
                                warn!("{}: PTY write error: {}", id, e);
                                break;
                            }
                        }
                    }
                    Ok(Some(WebSocketMessage::Text(txt))) => {
                        use std::io::Write;
                        if let Err(e) = (&*master_async.get_ref()).write(txt.as_bytes()) {
                            if e.kind() != std::io::ErrorKind::WouldBlock {
                                warn!("{}: PTY write error: {}", id, e);
                                break;
                            }
                        }
                    }
                    Ok(Some(WebSocketMessage::Close(_, _))) => {
                        debug!("{}: Terminal WS client closed.", id);
                        break;
                    }
                    Ok(None) => {
                        debug!("{}: Terminal WS client disconnected.", id);
                        break;
                    }
                    Ok(_) => (),
                    Err(e) => {
                        warn!("{}: Terminal WS read error: {}", id, e);
                        break;
                    }
                }
            }
            // ── Read from PTY master → send as binary WS ───────
            readable = master_async.readable() => {
                let mut guard = match readable {
                    Ok(g) => g,
                    Err(e) => {
                        warn!("{}: PTY readable error: {}", id, e);
                        break;
                    }
                };
                use std::io::Read;
                match guard.try_io(|afd| afd.get_ref().read(&mut buf)) {
                    Ok(Ok(0)) => {
                        debug!("{}: PTY EOF for '{}'.", id, session_name);
                        break;
                    }
                    Ok(Ok(n)) => {
                        let msg = WebSocketMessage::Binary(buf[..n].to_vec());
                        if let Err(e) = ws.send(&msg).await {
                            warn!("{}: WS send error: {}", id, e);
                            break;
                        }
                    }
                    Ok(Err(ref e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        guard.clear_ready();
                    }
                    Ok(Err(e)) => {
                        warn!("{}: PTY read error: {}", id, e);
                        break;
                    }
                    Err(_) => {
                        guard.clear_ready();
                    }
                }
            }
        }
    }

    // ── Cleanup ───────────────────────────────────────────────
    // Kill the tmux attach process.  The tmux session itself
    // persists for reconnection.
    let _ = child.kill().await;
    let _ = child.wait().await;
    let _ = ws.send(&WebSocketMessage::Close(
        None, Some("session ended".to_string()))).await;
    debug!("{}: Terminal bridge closed for '{}'.", id, session_name);
    Ok(())
}
