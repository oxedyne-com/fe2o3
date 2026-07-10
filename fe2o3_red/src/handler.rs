//! Red WS handler — chat protocol over WebSocket.
//!
//! Connected via Steel's path-based WS dispatch in https.rs when the
//! path is `/chat`.  Handles the full chat lifecycle:
//!
//! - Receives user messages (syntax commands from o3db.js)
//! - Runs the agent loop
//! - Streams LLM response tokens back as binary WS messages

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    id::NumIdDat,
};
use oxedyne_fe2o3_net::ws::core::{WebSocket, WebSocketMessage};
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_syntax::{
    SyntaxRef,
    msg::{Msg, MsgCmd},
};

use std::sync::{Arc, RwLock};

use crate::agent::Agent;
use crate::executor::Executor;
use crate::protocol::{AgentEvent, Session};
use crate::session::SessionStore;
use crate::tools::{Tool, ToolContext, ToolRegistry};
use crate::workspace::Workspace;
use crate::llm::datmap_to_json;
use std::path::PathBuf;
use std::pin::pin;


// ┌───────────────────────────────────────────────────────────────┐
// │ RedHandler                                                     │
// └───────────────────────────────────────────────────────────────┘

/// Per-vhost configuration for Red, parsed from Steel's config.jdat.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedConfig {
    pub llm_host:       String,
    pub llm_port:       u16,
    pub llm_path:       String,
    pub llm_key:        String,
    pub llm_model:      String,
    pub max_tokens:     u32,
    pub system_prompt:  String,
    /// Base directory (relative to the app root, or absolute) under
    /// which each user's workspace lives as `<workspace_root>/<user>`.
    pub workspace_root: String,
}

impl RedConfig {

    pub fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
        let llm_host = match m.get(&dat!("llm_host")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => "api.fireworks.ai".to_string(),
        };
        let llm_port = match m.get(&dat!("llm_port")) {
            Some(Dat::U16(n)) => *n,
            Some(Dat::U64(n)) => *n as u16,
            _ => 443,
        };
        let llm_path = match m.get(&dat!("llm_path")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => "/inference/v1/chat/completions".to_string(),
        };
        let llm_key = match m.get(&dat!("llm_key")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => return Err(err!("RedConfig: 'llm_key' is required."; Invalid, Input, Missing)),
        };
        let llm_model = match m.get(&dat!("llm_model")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => "accounts/fireworks/models/glm-5p2".to_string(),
        };
        let max_tokens = match m.get(&dat!("max_tokens")) {
            Some(Dat::U32(n)) => *n,
            Some(Dat::U64(n)) => *n as u32,
            Some(Dat::U16(n)) => *n as u32,
            _ => 4096, // sensible default; caps runaway reasoning loops
        };
        let system_prompt = match m.get(&dat!("system_prompt")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => "You are Red, an AI assistant.".to_string(),
        };
        let workspace_root = match m.get(&dat!("workspace_root")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => "workspaces".to_string(),
        };
        Ok(Self {
            llm_host, llm_port, llm_path, llm_key, llm_model, max_tokens,
            system_prompt, workspace_root,
        })
    }
}


/// State for the Red WS handler, shared across connections on a vhost.
#[derive(Clone)]
pub struct RedState<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + Clone + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    DB:     Database<UIDL, UID, ENC, KH> + 'static,
> {
    pub agent:         Agent,
    pub session_store: SessionStore<UIDL, UID, ENC, KH, DB>,
    /// Absolute base directory under which per-user workspaces live.
    pub workspace_base: PathBuf,
    /// Command-execution backend for the shell tool.
    pub executor:      Executor,
}

// ┌───────────────────────────────────────────────────────────────┐
// │ Chat WS handler                                                │
// └───────────────────────────────────────────────────────────────┘

/// Handle a chat WebSocket connection.
///
/// This is a path-based handler (like the terminal bridge) that
/// manages its own WS read/write loop.  It receives syntax commands
/// from the client, executes session/chat operations, and streams
/// agent responses back.
///
/// The connection uses Steel's existing session cookie for auth —
/// the `sid` parameter identifies the user's session.
pub async fn handle_chat_websocket<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + Clone + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    DB:     Database<UIDL, UID, ENC, KH> + 'static,
    S:      tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
>(
    mut stream:     S,
    state:          RedState<UIDL, UID, ENC, KH, DB>,
    sid:            Option<String>,
    vhost_db:       Option<(Arc<RwLock<DB>>, UID)>,
    request:        oxedyne_fe2o3_net::http::msg::HttpMessage,
    id:             &String,
)
    -> Outcome<()>
{
    // ── WebSocket handshake ───────────────────────────────────
    let chunk_size = 65536;
    let chunk_thresh = 32768;
    let mut ws: WebSocket<
        '_,
        UIDL, UID, ENC, KH, DB,
        S,
        oxedyne_fe2o3_net::ws::handler::WebSocketEchoHandler,
    > = WebSocket::new_server(
        &mut stream,
        oxedyne_fe2o3_net::ws::handler::WebSocketEchoHandler,
        chunk_size,
        chunk_thresh,
    );
    match ws.connect_as_server(request).await {
        Ok(()) => {
            debug!("{}: Red WS handshake completed.", id);
        }
        Err(e) => return Err(err!(e,
            "{}: Red WS handshake failed.", id;
            IO, Network, Wire)),
    }

    // ── Build Red's own syntax ────────────────────────────────
    let syntax = match crate::syntax::build_syntax() {
        Ok(s) => s,
        Err(e) => {
            error!(e, "{}: Red: failed to build syntax.", id);
            return Ok(());
        }
    };

    // ── Determine the authenticated username ──────────────────
    let username = match get_username(&vhost_db, &sid) {
        Some(u) => {
             info!("{}: Red WS authenticated as '{}'.", id, u);
            u
        }
        None => {
             info!("{}: Red WS not authenticated (sid={:?}).", id, sid);
            let _ = ws.send(&text_msg(syntax.clone(), "error", vec![
                dat!("Not authenticated.  Please log in first."),
            ])).await;
            return Ok(());
        }
    };

    // ── Chat loop ─────────────────────────────────────────────
    //
    // We read WS messages, parse them as syntax commands, and
    // dispatch:
    //   session_new   → create a new session
    //   session_list  → list user's sessions
    //   session_switch → set current session
    //   session_close → delete a session
    //   chat          → run agent turn (streams response)
    //   file_list     → list sandbox files (Phase 2)

    let mut current_session_id: Option<String> = None;
    let mut current_session: Option<Session> = None;

    loop {
        let msg = match ws.read().await {
            Ok(Some(WebSocketMessage::Text(txt))) => {
                debug!("{}: Red WS received text: '{}'", id, txt);
                txt
            }
            Ok(Some(WebSocketMessage::Binary(_))) => continue,
            Ok(Some(WebSocketMessage::Close(_, _))) => {
                info!("{}: Red WS client closed.", id);
                break;
            }
            Ok(None) => {
                info!("{}: Red WS client disconnected.", id);
                break;
            }
            Ok(_) => continue,
            Err(e) => {
                warn!("{}: Red WS read error: {}", id, e);
                break;
            }
        };

        // Parse syntax command.
        let msgrx = Msg::new(syntax.clone());
        let msgrx = match msgrx.from_str(&msg, None) {
            Ok(m) => m,
            Err(e) => {
                let _ = ws.send(&text_msg(syntax.clone(), "error", vec![
                    dat!(e.to_string()),
                ])).await;
                continue;
            }
        };
        if msgrx.cmds.len() != 1 {
            let _ = ws.send(&text_msg(syntax.clone(), "error", vec![
                dat!("Expected one command."),
            ])).await;
            continue;
        }
        let (cmd_name, cmdrx) = match msgrx.cmds.into_iter().next() {
            Some(v) => v,
            None    => continue, // length checked == 1 above
        };

        debug!("{}: Red WS command '{}'.", id, cmd_name);

        match cmd_name.as_str() {
            "session_new" => {
                let name = match cmdrx.vals.get(0) {
                    Some(Dat::Str(s)) => s.clone(),
                    _ => fmt!("Session"),
                };
                let model = match cmdrx.vals.get(1) {
                    Some(Dat::Str(s)) => s.clone(),
                    _ => state.agent.llm.model.clone(),
                };
                match state.session_store.create_session(
                    &username, &name, &model,
                ) {
                    Ok(session) => {
                        current_session_id = Some(session.id.clone());
                        current_session = Some(session.clone());
                        let _ = ws.send(&text_msg(syntax.clone(), "data", vec![
                            dat!(datmap_to_json(&session.to_meta_datmap())),
                        ])).await;
                    }
                    Err(e) => {
                        let _ = ws.send(&text_msg(syntax.clone(), "error", vec![
                            dat!(e.to_string()),
                        ])).await;
                    }
                }
            }
            "session_list" => {
                match state.session_store.list_sessions(&username) {
                    Ok(sessions) => {
                        let list: Vec<Dat> = sessions.iter()
                            .map(|s| Dat::Map(s.to_meta_datmap()))
                            .collect();
                        let mut m = DaticleMap::new();
                        m.insert(dat!("sessions"), Dat::List(list));
                        let json = crate::llm::datmap_to_json(&m);
                        let _ = ws.send(&text_msg(syntax.clone(), "data", vec![
                            dat!(json),
                        ])).await;
                    }
                    Err(e) => {
                        let _ = ws.send(&text_msg(syntax.clone(), "error", vec![
                            dat!(e.to_string()),
                        ])).await;
                    }
                }
            }
            "session_switch" => {
                let session_id = match cmdrx.vals.get(0) {
                    Some(Dat::Str(s)) => s.clone(),
                    _ => {
                        let _ = ws.send(&text_msg(syntax.clone(), "error", vec![
                            dat!("session_switch: missing session id."),
                        ])).await;
                        continue;
                    }
                };
                match state.session_store.get_session(&session_id) {
                    Ok(session) => {
                        current_session_id = Some(session.id.clone());
                        current_session = Some(session.clone());
                        let json = crate::llm::datmap_to_json(&session.to_datmap());
                        let _ = ws.send(&text_msg(syntax.clone(), "data", vec![
                            dat!(json),
                        ])).await;
                    }
                    Err(e) => {
                        let _ = ws.send(&text_msg(syntax.clone(), "error", vec![
                            dat!(e.to_string()),
                        ])).await;
                    }
                }
            }
            "session_close" => {
                let session_id = match cmdrx.vals.get(0) {
                    Some(Dat::Str(s)) => s.clone(),
                    _ => {
                        let _ = ws.send(&text_msg(syntax.clone(), "error", vec![
                            dat!("session_close: missing session id."),
                        ])).await;
                        continue;
                    }
                };
                match state.session_store.delete_session(&username, &session_id) {
                    Ok(()) => {
                        if current_session_id.as_deref() == Some(&session_id) {
                            current_session_id = None;
                            current_session = None;
                        }
                        let _ = ws.send(&text_msg(syntax.clone(), "info", vec![
                            dat!(fmt!("Session '{}' closed.", session_id)),
                        ])).await;
                    }
                    Err(e) => {
                        let _ = ws.send(&text_msg(syntax.clone(), "error", vec![
                            dat!(e.to_string()),
                        ])).await;
                    }
                }
            }
            "session_rename" => {
                let session_id = match cmdrx.vals.get(0) {
                    Some(Dat::Str(s)) => s.clone(),
                    _ => {
                        let _ = ws.send(&text_msg(syntax.clone(), "error", vec![
                            dat!("session_rename: missing session id."),
                        ])).await;
                        continue;
                    }
                };
                let new_name = match cmdrx.vals.get(1) {
                    Some(Dat::Str(s)) => s.clone(),
                    _ => {
                        let _ = ws.send(&text_msg(syntax.clone(), "error", vec![
                            dat!("session_rename: missing new name."),
                        ])).await;
                        continue;
                    }
                };
                match state.session_store.rename_session(&session_id, &new_name) {
                    Ok(()) => {
                        if current_session_id.as_deref() == Some(&session_id) {
                            if let Some(ref mut s) = current_session {
                                s.name = new_name.clone();
                            }
                        }
                        let _ = ws.send(&text_msg(syntax.clone(), "info", vec![
                            dat!(fmt!("Session renamed to '{}'.", new_name)),
                        ])).await;
                    }
                    Err(e) => {
                        let _ = ws.send(&text_msg(syntax.clone(), "error", vec![
                            dat!(e.to_string()),
                        ])).await;
                    }
                }
            }
            "chat" => {
                let content = match cmdrx.vals.get(0) {
                    Some(Dat::Str(s)) => s.clone(),
                    _ => {
                        let _ = ws.send(&text_msg(syntax.clone(), "error", vec![
                            dat!("chat: missing content."),
                        ])).await;
                        continue;
                    }
                };

                // Ensure we have a current session.
                if current_session.is_none() {
                    // Auto-create a session.
                    match state.session_store.create_session(
                        &username, "Untitled", &state.agent.llm.model,
                    ) {
                        Ok(s) => {
                            current_session_id = Some(s.id.clone());
                            current_session = Some(s);
                        }
                        Err(e) => {
                            let _ = ws.send(&text_msg(syntax.clone(), "error", vec![
                                dat!(e.to_string()),
                            ])).await;
                            continue;
                        }
                    }
                }

                // Run the agent turn with streaming events.
                //
                // The on_event callback is synchronous (FnMut), but
                // we need to send WS messages asynchronously.  We
                // use an mpsc channel and pin the agent future so
                // we can concurrently drain events while the agent
                // turn runs — giving the user true incremental
                // streaming.
                //
                // The session is taken out of current_session so
                // the pinned future can hold &mut without
                // conflicting with the save after.  The future is
                // scoped inside a block so the borrow ends before
                // we put the session back.
                let mut session = match current_session.take() {
                    Some(s) => s,
                    None    => continue, // presence checked above
                };
                let agent = state.agent.clone();
                let syntax_ref = syntax.clone();

                // Build the per-user tool registry: a workspace jailed
                // to <workspace_base>/<user>, plus the executor.
                let registry = {
                    let ws_dir = state.workspace_base.join(&username);
                    match Workspace::new(ws_dir) {
                        Ok(ws) => {
                            let ctx = ToolContext {
                                workspace: ws,
                                executor:  state.executor.clone(),
                                cwd:       String::new(),
                            };
                            ToolRegistry::new(Tool::defaults(), ctx)
                        }
                        Err(e) => {
                            let _ = ws.send(&text_msg(syntax.clone(), "error", vec![
                                dat!(fmt!("Workspace unavailable: {}", e)),
                            ])).await;
                            current_session = Some(session);
                            continue;
                        }
                    }
                };

                // Expand any skill invocation (<name ...>) in the message
                // by injecting the matching skill's instructions.
                let content = match crate::skills::expand(&content, &registry.ctx.workspace) {
                    Ok(c) => c,
                    Err(_) => content,
                };

                let result = {
                    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();

                    let mut on_event = |ev| { let _ = tx.send(ev); };
                    let mut agent_fut = pin!(agent.run_turn(
                        &mut session,
                        content,
                        &registry,
                        &mut on_event,
                    ));

                    let mut result = Ok(());
                    loop {
                        tokio::select! {
                            biased;
                            r = &mut agent_fut => {
                                result = r;
                                // Drain any remaining events.
                                while let Ok(ev) = rx.try_recv() {
                                    let (cmd_name, vals) = event_to_ws(&ev);
                                    let _ = ws.send(&text_msg(
                                        syntax_ref.clone(), cmd_name, vals,
                                    )).await;
                                }
                                break;
                            }
                            ev = rx.recv() => {
                                match ev {
                                    Some(ev) => {
                                        let (cmd_name, vals) = event_to_ws(&ev);
                                        let _ = ws.send(&text_msg(
                                            syntax_ref.clone(), cmd_name, vals,
                                        )).await;
                                    }
                                    None => break,
                                }
                            }
                        }
                    }
                    result
                };

                // Put the session back and save.
                current_session = Some(session);
                if let Some(ref s) = current_session {
                    let _ = state.session_store.save_session(s);
                }

                if let Err(e) = result {
                    warn!("{}: Agent turn error: {}", id, e);
                }
            }
            "fs_list" => {
                let path = match cmdrx.vals.get(0) {
                    Some(Dat::Str(s)) => s.clone(),
                    _ => ".".to_string(),
                };
                match user_ws(&state.workspace_base, &username)
                    .and_then(|w| fs_list_json(&w, &path))
                {
                    Ok(json) => { let _ = ws.send(&text_msg(syntax.clone(), "fs_tree", vec![dat!(json)])).await; }
                    Err(e)   => { let _ = ws.send(&text_msg(syntax.clone(), "error", vec![dat!(e.to_string())])).await; }
                }
            }
            "fs_read" => {
                let path = match cmdrx.vals.get(0) {
                    Some(Dat::Str(s)) => s.clone(),
                    _ => { let _ = ws.send(&text_msg(syntax.clone(), "error", vec![dat!("fs_read: missing path.")])).await; continue; }
                };
                match user_ws(&state.workspace_base, &username)
                    .and_then(|w| fs_read_file(&w, &path))
                {
                    Ok(content) => { let _ = ws.send(&text_msg(syntax.clone(), "fs_content", vec![dat!(path.clone()), dat!(content)])).await; }
                    Err(e)      => { let _ = ws.send(&text_msg(syntax.clone(), "error", vec![dat!(e.to_string())])).await; }
                }
            }
            "fs_delete" => {
                let path = match cmdrx.vals.get(0) {
                    Some(Dat::Str(s)) => s.clone(),
                    _ => { let _ = ws.send(&text_msg(syntax.clone(), "error", vec![dat!("fs_delete: missing path.")])).await; continue; }
                };
                match user_ws(&state.workspace_base, &username)
                    .and_then(|w| fs_delete_file(&w, &path))
                {
                    Ok(())  => { let _ = ws.send(&text_msg(syntax.clone(), "info", vec![dat!(fmt!("Deleted {}.", path))])).await; }
                    Err(e)  => { let _ = ws.send(&text_msg(syntax.clone(), "error", vec![dat!(e.to_string())])).await; }
                }
            }
            "fs_write" => {
                let path = match cmdrx.vals.get(0) {
                    Some(Dat::Str(s)) => s.clone(),
                    _ => { let _ = ws.send(&text_msg(syntax.clone(), "error", vec![dat!("fs_write: missing path.")])).await; continue; }
                };
                let content = match cmdrx.vals.get(1) {
                    Some(Dat::Str(s)) => s.clone(),
                    _ => String::new(),
                };
                match user_ws(&state.workspace_base, &username)
                    .and_then(|w| fs_write_file(&w, &path, &content))
                {
                    Ok(())  => { let _ = ws.send(&text_msg(syntax.clone(), "info", vec![dat!(fmt!("Wrote {}.", path))])).await; }
                    Err(e)  => { let _ = ws.send(&text_msg(syntax.clone(), "error", vec![dat!(e.to_string())])).await; }
                }
            }
            _ => {
                let _ = ws.send(&text_msg(syntax.clone(), "error", vec![
                    dat!(fmt!("Unknown command '{}'.", cmd_name)),
                ])).await;
            }
        }
    }

     info!("{}: Red WS closed.", id);
    Ok(())
}


// ┌───────────────────────────────────────────────────────────────┐
// │ Helpers                                                        │
// └───────────────────────────────────────────────────────────────┘

/// Map an `AgentEvent` to a WS command name and value list.
/// Open a user's workspace under the shared base directory.
fn user_ws(base: &PathBuf, user: &str) -> Outcome<Workspace> {
    Workspace::new(base.join(user))
}

/// Build a JSON directory listing: `{"path":..,"entries":[{name,dir,size}]}`.
fn fs_list_json(ws: &Workspace, path: &str) -> Outcome<String> {
    let abs = res!(ws.resolve(path));
    let rd = res!(std::fs::read_dir(&abs)
        .map_err(|e| err!(e, "fs_list: cannot read directory '{}'.", path; IO, File, Read)));
    let mut entries: Vec<(bool, String, u64)> = rd
        .filter_map(|e| e.ok())
        .map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let is_dir = e.path().is_dir();
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            (is_dir, name, size)
        })
        .collect();
    entries.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let disp = if path == "." { String::new() } else { path.to_string() };
    let mut out = fmt!("{{\"path\":\"{}\",\"entries\":[", crate::llm::json_escape(&disp));
    for (i, (is_dir, name, size)) in entries.iter().enumerate() {
        if i > 0 { out.push(','); }
        out.push_str(&fmt!("{{\"name\":\"{}\",\"dir\":{},\"size\":{}}}",
            crate::llm::json_escape(name), is_dir, size));
    }
    out.push_str("]}");
    Ok(out)
}

/// Read a workspace text file.
fn fs_read_file(ws: &Workspace, path: &str) -> Outcome<String> {
    let abs = res!(ws.resolve(path));
    let data = res!(std::fs::read(&abs)
        .map_err(|e| err!(e, "fs_read: cannot read '{}'.", path; IO, File, Read)));
    Ok(String::from_utf8_lossy(&data).to_string())
}

/// Delete a workspace file.
fn fs_delete_file(ws: &Workspace, path: &str) -> Outcome<()> {
    let abs = res!(ws.resolve(path));
    res!(std::fs::remove_file(&abs)
        .map_err(|e| err!(e, "fs_delete: cannot delete '{}'.", path; IO, File)));
    Ok(())
}

/// Create or overwrite a workspace file.
fn fs_write_file(ws: &Workspace, path: &str, content: &str) -> Outcome<()> {
    let abs = res!(ws.resolve(path));
    if let Some(parent) = abs.parent() {
        res!(std::fs::create_dir_all(parent)
            .map_err(|e| err!(e, "fs_write: cannot create parent dirs for '{}'.", path; IO, File)));
    }
    res!(std::fs::write(&abs, content.as_bytes())
        .map_err(|e| err!(e, "fs_write: cannot write '{}'.", path; IO, File, Write)));
    Ok(())
}

fn event_to_ws(ev: &AgentEvent) -> (&'static str, Vec<Dat>) {
    match ev {
        AgentEvent::Text(t)   => ("text",  vec![dat!(t.clone())]),
        AgentEvent::ToolCall { name, args } =>
            ("tool_call", vec![dat!(name.clone()), dat!(args.clone())]),
        AgentEvent::ToolResult { name, result } =>
            ("tool_result", vec![dat!(name.clone()), dat!(result.clone())]),
        AgentEvent::Done      => ("done",  vec![]),
        AgentEvent::Error(msg) => ("error", vec![dat!(msg.clone())]),
    }
}

fn text_msg(syntax: SyntaxRef, cmd: &str, vals: Vec<Dat>)
    -> WebSocketMessage
{
    let mut response = match MsgCmd::new(syntax.clone(), cmd) {
        Ok(r) => r,
        Err(e) => {
            debug!("text_msg: MsgCmd::new('{}') failed: {}", cmd, e);
            return WebSocketMessage::Text("error \"internal\"".to_string());
        }
    };
    for val in &vals {
        response = match response.add_cmd_val(val.clone()) {
            Ok(r) => r,
            Err(e) => {
                debug!("text_msg: add_cmd_val for '{}' failed: {} (val kind={:?})", cmd, e, val.kind());
                return WebSocketMessage::Text("error \"internal\"".to_string());
            }
        }
    }
    WebSocketMessage::Text(response.to_string())
}

/// Look up the authenticated username from the session metadata
/// in O3db.
fn get_username<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + Clone,
    ENC:    Encrypter,
    KH:     Hasher,
    DB:     Database<UIDL, UID, ENC, KH>,
>(
    db: &Option<(Arc<RwLock<DB>>, UID)>,
    sid: &Option<String>,
) -> Option<String> {
    let sid = match sid.as_ref() {
        Some(s) => s,
        None    => return None,
    };
    let (db, _uid) = match db.as_ref() {
        Some(v) => v,
        None    => return None,
    };
    let db_r = match db.read() {
        Ok(v)  => v,
        Err(_) => return None,
    };
    let meta_key = Dat::Str(fmt!("sess_meta:{}", sid));
    match db_r.get(&meta_key, None) {
        Ok(Some((data, _))) => {
            if let Dat::Map(m) = &data {
                if let Some(Dat::Str(user)) = m.get(&dat!("user")) {
                    if !user.is_empty() {
                        return Some(user.clone());
                    }
                }
            }
            None
        }
        _ => None,
    }
}
