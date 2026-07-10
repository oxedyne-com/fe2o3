//! The browser agent surface — a `#[wasm_bindgen]` [`RedApp`] that runs a
//! real [`Agent`] turn and streams [`AgentEvent`]s to a JS callback.
//!
//! This is the Stage 3 completion: the agent loop itself running in the
//! browser, not merely a transport probe.  A [`RedApp`] owns a
//! [`Session`], an [`Agent`] (built on the wasm [`LlmClient`]), and a
//! [`ToolRegistry`].  [`RedApp::run_turn`] drives
//! [`Agent::run_turn`](crate::agent::Agent::run_turn), forwarding each
//! streamed event to the supplied `on_event` function as a plain JS
//! object.
//!
//! With tools disabled the turn takes the pure-streaming path (SSE token
//! deltas); with tools enabled it takes the agentic tool loop, whose file
//! tools are backed by the OPFS edge (see [`crate::tools`]).

use crate::agent::Agent;
use crate::llm::LlmClient;
use crate::protocol::{AgentEvent, Session, generate_session_id};
use crate::tools::{Tool, ToolContext, ToolRegistry};
use crate::executor::Executor;
use crate::workspace::Workspace;
use crate::wasm::{focus, to_js_err};

use oxedyne_fe2o3_core::prelude::*;

use std::path::PathBuf;

use wasm_bindgen::prelude::*;


/// The browser-side Red application: one session driven by the agent
/// loop over the wasm transport.
#[wasm_bindgen]
pub struct RedApp {
    agent:    Agent,
    session:  Session,
    registry: ToolRegistry,
}

#[wasm_bindgen]
impl RedApp {

    /// Construct a [`RedApp`].
    ///
    /// `base_url` is the full chat-completions endpoint, e.g.
    /// `https://api.provider.com/v1/chat/completions` or, for a local
    /// mock, `http://127.0.0.1:8081/v1/chat/completions`; the scheme
    /// selects the transport's `secure` flag.  When `enable_tools` is
    /// set, the OPFS-backed file tools (`file_write`, `file_read`) are
    /// registered and the turn runs the agentic tool loop.
    #[wasm_bindgen(constructor)]
    pub fn new(
        base_url:      String,
        api_key:       String,
        model:         String,
        max_tokens:    u32,
        system_prompt: String,
        enable_tools:  bool,
    )
        -> Result<RedApp, JsValue>
    {
        Self::build(&base_url, &api_key, &model, max_tokens, &system_prompt, enable_tools)
            .map_err(to_js_err)
    }

    /// Inner constructor returning an [`Outcome`], so the URL parse and
    /// client build use the error macros; the `#[wasm_bindgen]` wrapper
    /// maps the result to the JS boundary.
    fn build(
        base_url:      &str,
        api_key:       &str,
        model:         &str,
        max_tokens:    u32,
        system_prompt: &str,
        enable_tools:  bool,
    )
        -> Outcome<RedApp>
    {
        let (secure, host, port, path) = res!(parse_base_url(base_url));
        let llm = LlmClient::new_with_scheme(&host, port, &path, api_key, model, max_tokens, secure);
        let agent = Agent::new(llm, system_prompt);

        let session = Session::new(
            crate::protocol::generate_session_id(),
            "browser".to_string(),
            model.to_string(),
        );

        // The OPFS edge does its own path jailing, so the workspace root
        // is nominal; `Executor::Wasm` escalates any shell attempt.
        let ctx = ToolContext {
            workspace:   Workspace::unchecked(PathBuf::from("/")),
            executor:    Executor::Wasm,
            cwd:         String::new(),
            path_prefix: String::new(),
        };
        // The whole file toolset is OPFS-backed in the browser; only the
        // shell tool has no in-browser executor, so it is left out.
        let tools = if enable_tools {
            vec![
                Tool::FileRead,
                Tool::FileWrite,
                Tool::FileEdit,
                Tool::FileList,
                Tool::FileSearch,
                Tool::FileDelete,
            ]
        } else {
            Vec::new()
        };
        let registry = ToolRegistry::new(tools, ctx);

        Ok(RedApp { agent, session, registry })
    }

    /// Run one agent turn for `user_msg`, invoking `on_event` once per
    /// streamed [`AgentEvent`] with a plain JS object (see
    /// [`event_to_js`]).  Resolves when the turn completes; rejects with
    /// the stringified error on failure.
    pub async fn run_turn(
        &mut self,
        user_msg: String,
        on_event: js_sys::Function,
    )
        -> Result<(), JsValue>
    {
        let mut sink = |ev: AgentEvent| {
            let js = event_to_js(&ev);
            // A callback that throws must not abort the turn; ignore the
            // JS-side result deliberately.
            let _ = on_event.call1(&JsValue::NULL, &js);
        };
        self.agent
            .run_turn(&mut self.session, user_msg, &self.registry, &mut sink)
            .await
            .map_err(to_js_err)
    }

    /// Invoke a single tool directly by wire name with a raw-JSON argument
    /// object, returning its result text — the same path the agent loop
    /// takes, without an LLM turn.  This backs UI affordances such as a
    /// file-browser panel (list/read/delete) that act on OPFS directly.
    /// Tool errors are returned as `Error: …` text (never a rejection), so
    /// the browser can surface them inline.
    pub async fn run_tool(&self, name: String, args_json: String) -> String {
        self.registry.dispatch(&name, &args_json).await
    }

    // ── Focus / brief / fold surface ─────────────────────────────────

    /// Create a Focus named `name`, returning its id.  Creates the Focus
    /// directory, an empty `brief.md`, version `0`, a `meta.json`, and a
    /// `create` log record.
    pub async fn create_focus(&self, name: String) -> Result<String, JsValue> {
        focus::create(&name).await.map_err(to_js_err)
    }

    /// List every Focus as a JSON array of
    /// `{ id, name, brief_version, updated }`, most-recently updated first.
    pub async fn list_foci(&self) -> Result<String, JsValue> {
        focus::list().await.map_err(to_js_err)
    }

    /// Read a Focus's current brief markdown.
    pub async fn read_brief(&self, id: String) -> Result<String, JsValue> {
        focus::read_brief(&id).await.map_err(to_js_err)
    }

    /// Apply a user hand-edit to a Focus's brief: snapshots a new version
    /// and logs an `edit` record.
    pub async fn write_brief(&self, id: String, md: String) -> Result<(), JsValue> {
        focus::write_brief(&id, &md).await.map_err(to_js_err)
    }

    /// Read a Focus's append-only log as a JSON array of records.
    pub async fn log_read(&self, id: String) -> Result<String, JsValue> {
        focus::log_read(&id).await.map_err(to_js_err)
    }

    /// Steer a Focus's brief: run one brief-agent turn for `instruction`,
    /// streaming [`AgentEvent`]s to `on_event`.  The agent's file tools
    /// are scoped to `foci/<id>/`, so `file_read` / `file_write` on
    /// `brief.md` address the Focus's brief; it is stateless per
    /// instruction, reconstructing context from the current brief passed
    /// in its system prompt.  When the turn leaves `brief.md` changed, a
    /// new version is snapshotted and an `edit` record logged.
    pub async fn steer_brief(
        &self,
        id:          String,
        instruction: String,
        on_event:    js_sys::Function,
    )
        -> Result<(), JsValue>
    {
        self.steer_inner(&id, instruction, on_event).await.map_err(to_js_err)
    }

    /// Propose a fold: run a fresh reducer over the current brief plus one
    /// `delta`, returning the PROPOSED new brief markdown.  Writes
    /// nothing — the advisory half of the fold (H2); the delta is applied
    /// only on explicit confirm via [`RedApp::fold_apply`].
    pub async fn fold_propose(&self, id: String, delta: String) -> Result<String, JsValue> {
        self.fold_propose_inner(&id, &delta).await.map_err(to_js_err)
    }

    /// Apply a confirmed fold: write the accepted `new_brief`, snapshot a
    /// version, retain the raw `delta` under `.red/deltas/`, and append a
    /// `fold` record referencing it.  Called only after the user accepts
    /// the proposed diff, so a fold never auto-applies and never discards
    /// the raw delta.
    pub async fn fold_apply(
        &self,
        id:        String,
        new_brief: String,
        delta:     String,
        note:      String,
    )
        -> Result<(), JsValue>
    {
        focus::fold_apply(&id, &new_brief, &delta, &note).await.map_err(to_js_err)
    }

    /// Cumulative prompt tokens billed to this session.
    #[wasm_bindgen(getter)]
    pub fn prompt_tokens(&self) -> f64 {
        self.session.prompt_tokens as f64
    }

    /// Cumulative completion tokens billed to this session.
    #[wasm_bindgen(getter)]
    pub fn completion_tokens(&self) -> f64 {
        self.session.completion_tokens as f64
    }
}

/// Inner helpers for the brief and reducer turns.  Kept in a plain
/// `impl` (not `#[wasm_bindgen]`) so they can take Rust-only types and
/// return [`Outcome`], using the error macros throughout; the exported
/// wrappers above map the result to the JS boundary.
impl RedApp {

    /// Drive the brief agent for one instruction (see
    /// [`RedApp::steer_brief`]).
    async fn steer_inner(
        &self,
        id:          &str,
        instruction: String,
        on_event:    js_sys::Function,
    )
        -> Outcome<()>
    {
        // Stateless per instruction: reconstruct context from the brief.
        let before = focus::read_brief(id).await.unwrap_or_default();
        let mut system = focus::BRIEF_AGENT_PROMPT.to_string();
        system.push_str("\n\nCurrent brief.md:\n");
        system.push_str(&before);

        // File tools scoped to this Focus's directory.
        let ctx = ToolContext {
            workspace:   Workspace::unchecked(PathBuf::from("/")),
            executor:    Executor::Wasm,
            cwd:         String::new(),
            path_prefix: focus::focus_dir(id),
        };
        let registry = ToolRegistry::new(
            vec![
                Tool::FileRead,
                Tool::FileWrite,
                Tool::FileEdit,
                Tool::FileList,
                Tool::FileSearch,
                Tool::FileDelete,
            ],
            ctx,
        );
        let agent = Agent::new(self.agent.llm.clone(), &system);
        let mut session = Session::new(
            generate_session_id(),
            fmt!("brief:{}", id),
            self.session.model.clone(),
        );
        let mut sink = |ev: AgentEvent| {
            let js = event_to_js(&ev);
            let _ = on_event.call1(&JsValue::NULL, &js);
        };
        res!(agent.run_turn(&mut session, instruction.clone(), &registry, &mut sink).await);

        // If the brief changed, snapshot a version and log the edit so
        // every brief mutation stays versioned and auditable.
        let after = focus::read_brief(id).await.unwrap_or_default();
        if after != before {
            res!(focus::record_steer(id, &after, &instruction).await);
        }
        Ok(())
    }

    /// Drive the reducer for one delta, returning the proposed brief (see
    /// [`RedApp::fold_propose`]).
    async fn fold_propose_inner(&self, id: &str, delta: &str) -> Outcome<String> {
        let brief = res!(focus::read_brief(id).await);
        let user_msg = fmt!(
            "Current brief:\n{}\n\n---\nDelta to fold in:\n{}",
            brief, delta,
        );
        // The reducer only emits text — no tools, so it cannot write.
        let ctx = ToolContext {
            workspace:   Workspace::unchecked(PathBuf::from("/")),
            executor:    Executor::Wasm,
            cwd:         String::new(),
            path_prefix: String::new(),
        };
        let registry = ToolRegistry::new(Vec::new(), ctx);
        let agent = Agent::new(self.agent.llm.clone(), focus::REDUCER_PROMPT);
        let mut session = Session::new(
            generate_session_id(),
            fmt!("reducer:{}", id),
            self.session.model.clone(),
        );
        let mut out = String::new();
        {
            let mut sink = |ev: AgentEvent| {
                if let AgentEvent::Text(t) = &ev {
                    out.push_str(t);
                }
            };
            res!(agent.run_turn(&mut session, user_msg, &registry, &mut sink).await);
        }
        Ok(out)
    }
}

/// Convert an [`AgentEvent`] to a plain JS object mirroring
/// [`AgentEvent::to_datmap`]: a `type` discriminator plus the variant's
/// fields.  Built directly with `Reflect::set` so the JS side receives a
/// structured object, not a string it must re-parse.
fn event_to_js(ev: &AgentEvent) -> JsValue {
    let obj = js_sys::Object::new();
    let set = |k: &str, v: &JsValue| {
        // `Reflect::set` on a fresh object cannot fail; ignore the result.
        let _ = js_sys::Reflect::set(&obj, &JsValue::from_str(k), v);
    };
    match ev {
        AgentEvent::Text(text) => {
            set("type", &JsValue::from_str("text"));
            set("content", &JsValue::from_str(text));
        }
        AgentEvent::ToolCall { name, args } => {
            set("type", &JsValue::from_str("tool_call"));
            set("name", &JsValue::from_str(name));
            set("args", &JsValue::from_str(args));
        }
        AgentEvent::ToolResult { name, result } => {
            set("type", &JsValue::from_str("tool_result"));
            set("name", &JsValue::from_str(name));
            set("content", &JsValue::from_str(result));
        }
        AgentEvent::Done => {
            set("type", &JsValue::from_str("done"));
        }
        AgentEvent::Error(msg) => {
            set("type", &JsValue::from_str("error"));
            set("content", &JsValue::from_str(msg));
        }
    }
    obj.into()
}

/// Split a full `scheme://host[:port]/path` base URL into
/// `(secure, host, port, path)`.
///
/// `https` and `http` are both accepted — the former for real providers,
/// the latter for a local mock over `127.0.0.1`.  The port defaults to
/// the scheme default (443 / 80) when absent; the path defaults to `/`.
fn parse_base_url(url: &str) -> Outcome<(bool, String, u16, String)> {
    let (secure, default_port, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, 443u16, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, 80u16, r)
    } else {
        return Err(err!(
            "RedApp: base URL '{}' must start with http:// or https://.", url;
            Invalid, Input));
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None    => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => {
            let port = res!(p.parse::<u16>()
                .map_err(|e| err!(e, "RedApp: bad port in '{}'.", url; Invalid, Input)));
            (h.to_string(), port)
        }
        None => (authority.to_string(), default_port),
    };
    if host.is_empty() {
        return Err(err!("RedApp: empty host in '{}'.", url; Invalid, Input));
    }
    Ok((secure, host, port, path.to_string()))
}
