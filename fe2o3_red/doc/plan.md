# Red — AI Agent and Web Chatbot

> Status: Planning
> Date: 2026-07-09
> Author: Jason Hoogland

## Purpose

Red is a web-based AI agent that provides a clean chat interface to an LLM
(with tool-calling capability) over Steel's WebSocket infrastructure.  It
replaces the tmux/PTY terminal bridge approach used previously, which suffered
from garbled output caused by terminal escape sequences in Goose's CLI TUI.

Red is a new crate in the fe2o3 (Hematite) library.  It depends on
`fe2o3_steel`, `fe2o3_net`, `fe2o3_o3db_sync`, and `fe2o3_jdat`.  It does not
depend on Goose, ttyd, xterm.js, or any external agent framework.

## Goals

1. **Chatbot with file I/O** — users upload files, the AI reads/modifies/creates
   files, users download results.  Files are scoped to sessions.

2. **Goose-equivalent functionality, sandboxed** — the agent can run shell
   commands, read/write files, and use tools, but operates inside a per-session
   sandbox directory rather than directly on the host filesystem.

3. **Extensible** — a trait-based tool system allows adding new capabilities
   without modifying the agent loop.  Future phases add MCP bridging, custom
   system prompts, and additional tool types.

## Design Principles

- **Simple** — the core agent loop is a single function.  Each component (LLM
  client, tool executor, session store, WS handler) is independently testable.

- **Extensible** — tools implement a trait.  The agent loop knows nothing about
  specific tools; it passes tool-call requests from the LLM to the tool registry
  and returns results.

- **Performant** — streaming LLM responses via SSE, async throughout, no
  buffering of full responses before sending to the client.  The agent streams
  tokens to the browser as they arrive.

- **fe2o3-native** — uses `fe2o3_net` for HTTP (LLM API), `fe2o3_o3db_sync` for
  session/conversation storage, `fe2o3_jdat` for serialisation, `fe2o3_steel`
  for the WS handler trait and server infrastructure.  No `reqwest`, `serde`,
  or other heavy external crates where fe2o3 equivalents exist.


## Architecture

```
Browser (red.oxedyne.com)
  │
  ├── HTTP GET /  →  Steel static file server (HTML/CSS/JS)
  ├── HTTP POST /api/upload   →  file upload (multipart)
  ├── HTTP GET /api/download  →  file download
  │
  └── WS /  →  Steel WS handler (existing auth: login/register/whoami/change_pass)
               + Red WS handler (agent protocol)
```

### Data flow

```
User types message
  → WS sends {"type":"message","content":"..."}
  → Red agent appends to conversation history (O3db)
  → Red agent POSTs to LLM API (Fireworks) with tools + history
  → LLM streams response tokens back (SSE)
  → Each token → WS sends {"type":"text","content":"..."} (streaming)
  → If LLM requests tool call:
      → WS sends {"type":"tool_call","name":"...","args":...}
      → Red executes tool in sandbox
      → WS sends {"type":"tool_result","name":"...","result":...}
      → Red sends tool result back to LLM → continue streaming
  → When LLM finishes:
      → WS sends {"type":"done"}
      → Full exchange stored in O3db
```

### Crate structure

```
fe2o3_red/
  Cargo.toml
  doc/
    plan.md           ← this file
  src/
    lib.rs            ← crate root, public exports
    agent.rs          ← agent loop: message → LLM → tools → response
    llm.rs            ← OpenAI-compatible API client with SSE streaming
    tool.rs           ← RedTool trait + ToolRegistry
    tools/
      mod.rs
      shell.rs         ← execute shell commands in sandbox dir
      file.rs          ← read/write/list/delete files in sandbox dir
    sandbox.rs         ← per-session working directory management
    session.rs         ← session CRUD + conversation history (O3db)
    protocol.rs        ← WS message types (serialised as JDAT)
    handler.rs         ← WebSocketHandler impl for Steel integration
  www/
    index.html
    css/
      variables.css
      app.css
    js/
      o3db.js          ← Steel WS auth client (reused from elearnity)
      app.js           ← chat UI logic
      marked.min.js    ← markdown rendering (bundled, ~40KB)
  tests/
    agent.rs           ← agent loop tests (mock LLM)
    tool.rs            ← tool execution tests
    session.rs         ← session storage tests
```

### Dependencies

```toml
[dependencies]
oxedyne_fe2o3_core      = { path = "../fe2o3_core" }
oxedyne_fe2o3_net       = { path = "../fe2o3_net" }       # HTTP client, WS
oxedyne_fe2o3_jdat      = { path = "../fe2o3_jdat" }      # serialisation
oxedyne_fe2o3_o3db_sync = { path = "../fe2o3_o3db_sync" } # session storage
oxedyne_fe2o3_hash      = { path = "../fe2o3_hash" }      # session IDs
oxedyne_fe2o3_iop_db    = { path = "../fe2o3_iop_db" }    # DB trait
oxedyne_fe2o3_iop_crypto = { path = "../fe2o3_iop_crypto" }
oxedyne_fe2o3_iop_hash   = { path = "../fe2o3_iop_hash" }
oxedyne_fe2o3_syntax    = { path = "../fe2o3_syntax" }    # WS syntax
oxedyne_fe2o3_stds      = { path = "../fe2o3_stds" }
oxedyne_fe2o3_text      = { path = "../fe2o3_text" }

tokio = { version = "1.35", features = ["full"] }
tokio-rustls = "0.26"
```

No `serde`, no `reqwest`, no `serde_json`.  All serialisation uses `fe2o3_jdat`.
LLM API requests/responses are parsed manually from JDAT maps — the
OpenAI-compatible JSON format maps cleanly to JDAT's `DaticleMap`.


## Component Design

### 1. LLM Client (`llm.rs`)

An async client for OpenAI-compatible chat completion APIs with streaming.

```rust
pub struct LlmClient {
    host:       String,      // e.g. "api.fireworks.ai"
    port:       u16,         // 443
    path:       String,      // "/inference/v1/chat/completions"
    api_key:    String,
    model:      String,      // "accounts/fireworks/models/glm-5p2"
    tls_config: TlsClientConfig,
}

impl LlmClient {
    /// Send a chat completion request with streaming.
    /// Calls `on_token` for each text chunk as it arrives.
    /// Calls `on_tool_call` if the LLM requests a tool.
    /// Returns the full response when the stream completes.
    pub async fn chat_stream(
        &self,
        messages:   &[ChatMessage],
        tools:      &[ToolDef],
        on_token:   impl FnMut(&str),
        on_tool:    impl FnMut(&ToolCall),
    ) -> Outcome<ChatResponse>;
}
```

**SSE streaming:** The LLM API returns `text/event-stream` with `data: {...}`
lines.  Each line is a JSON object with a `delta` containing either `content`
(text token) or `tool_calls` (tool invocation).  The client reads the HTTPS
response body incrementally using `fe2o3_net`'s `HttpMessageReader`, parses
each `data:` line, and calls the appropriate callback.

**Why not use `fe2o3_net::http::https_request`?** That function reads a
complete `HttpMessage` (headers + body) before returning.  For SSE streaming
we need to read the body incrementally as chunks arrive.  We'll use the
underlying `tokio_rustls` connection directly, write the request, read headers
with `HttpMessage::read`, then continue reading the body stream line-by-line.

### 2. Tool System (`tool.rs`)

```rust
pub trait RedTool: Send + Sync {
    /// Tool name as seen by the LLM (e.g. "shell", "file_read").
    fn name(&self) -> &str;

    /// Description for the LLM's system prompt.
    fn description(&self) -> &str;

    /// JSON schema for the tool's parameters (OpenAI function-calling format).
    /// Returned as a JDAT map so it can be serialised into the API request.
    fn parameters_schema(&self) -> DaticleMap;

    /// Execute the tool with the given arguments (JDAT map) inside
    /// the given sandbox directory.  Returns the result as a string
    /// (which the LLM sees as the tool response).
    fn execute(
        &self,
        args:       &DaticleMap,
        sandbox:    &Sandbox,
    ) -> Outcome<String>;
}

pub struct ToolRegistry {
    tools: Vec<Box<dyn RedTool>>,
}

impl ToolRegistry {
    pub fn new() -> Self;
    pub fn register(&mut self, tool: Box<dyn RedTool>);
    pub fn get(&self, name: &str) -> Option<&dyn RedTool>;
    pub fn definitions(&self) -> Vec<ToolDef>;  // for LLM API request
}
```

**Built-in tools:**

| Tool | Description |
|---|---|
| `shell` | Execute a shell command in the sandbox directory.  Returns stdout+stderr. |
| `file_read` | Read a file from the sandbox directory. |
| `file_write` | Write/create a file in the sandbox directory. |
| `file_list` | List files in the sandbox directory. |
| `file_delete` | Delete a file from the sandbox directory. |

All tools are confined to the session's sandbox directory — they cannot access
files outside it.  The `shell` tool uses `std::process::Command` with
`current_dir()` set to the sandbox path and a restricted environment.

### 3. Sandbox (`sandbox.rs`)

```rust
pub struct Sandbox {
    /// Root directory for this session's files.
    /// e.g. /home/jason/usr/steel-prod/sessions/<user>/<session_id>/
    root: PathBuf,
}

impl Sandbox {
    pub fn new(root: PathBuf) -> Self;

    /// Resolve a filename to a safe path within the sandbox.
    /// Rejects path traversal (../) attempts.
    pub fn resolve(&self, filename: &str) -> Outcome<PathBuf>;

    /// Execute a shell command in the sandbox directory.
    pub fn shell(&self, command: &str) -> Outcome<String>;

    /// Read a file.
    pub fn read(&self, filename: &str) -> Outcome<String>;

    /// Write a file.
    pub fn write(&self, filename: &str, content: &str) -> Outcome<()>;

    /// List files.
    pub fn list(&self) -> Outcome<Vec<String>>;

    /// Delete a file.
    pub fn delete(&self, filename: &str) -> Outcome<()>;
}
```

**Sandboxing levels (progressive):**

- **Phase 1:** Directory confinement — tools use `current_dir(sandbox.root)`,
  path traversal blocked by `resolve()`.  Shell commands run as the Steel
  process user but in the sandbox directory.

- **Phase 2:** Restricted shell — set `PATH` to a whitelist, unset dangerous
  env vars, use `chroot` if running as root, or `bubblewrap`/`firejail` if
  available.

- **Phase 3:** Linux namespaces — `unshare --mount --pid --net` for full
  isolation.  The sandbox directory is bind-mounted as `/` in the new
  namespace.

### 4. Session Management (`session.rs`)

Sessions are stored in O3db, keyed by user:

```
user:<username>:sessions       → List of session IDs
session:<id>:meta              → { name, created_at, model }
session:<id>:messages          → List of chat messages (conversation history)
session:<id>:sandbox_dir       → Path to sandbox directory
```

```rust
pub struct Session {
    pub id:         String,
    pub name:       String,
    pub created_at: u64,
    pub messages:   Vec<ChatMessage>,
}

pub struct SessionStore {
    // O3db handle (cloned from Steel's per-vhost database)
    db: Arc<RwLock<DB>>,
    uid: UID,
}

impl SessionStore {
    pub fn create(&self, user: &str, name: &str) -> Outcome<Session>;
    pub fn list(&self, user: &str) -> Outcome<Vec<Session>>;
    pub fn get(&self, id: &str) -> Outcome<Session>;
    pub fn append_message(&self, id: &str, msg: &ChatMessage) -> Outcome<()>;
    pub fn delete(&self, id: &str) -> Outcome<()>;
}
```

**ChatMessage** mirrors the OpenAI API format:

```rust
pub enum ChatMessage {
    System { content: String },
    User { content: String },
    Assistant { content: String, tool_calls: Vec<ToolCall> },
    Tool { tool_call_id: String, content: String },
}
```

### 5. Agent Loop (`agent.rs`)

```rust
pub async fn run_agent_turn(
    llm:        &LlmClient,
    tools:      &ToolRegistry,
    sandbox:    &Sandbox,
    session:    &mut Session,
    user_msg:   String,
    on_event:   impl FnMut(AgentEvent),
) -> Outcome<()>
```

`AgentEvent` is what gets sent to the client over WS:

```rust
pub enum AgentEvent {
    Text(String),              // LLM response token (streamed)
    ToolCall { name, args },   // LLM requested a tool
    ToolResult { name, result }, // Tool finished
    Done,                      // Turn complete
    Error(String),             // Error
}
```

The loop:

1. Append `ChatMessage::User { user_msg }` to session.
2. Build LLM request: `session.messages` + `tools.definitions()`.
3. Call `llm.chat_stream()` with callbacks:
   - `on_token(text)` → `on_event(AgentEvent::Text(text))`
   - `on_tool(call)` → `on_event(AgentEvent::ToolCall { ... })`
4. If tool calls were made:
   - For each tool call: execute via `tools.get(name).execute(args, sandbox)`
   - `on_event(AgentEvent::ToolResult { ... })`
   - Append `ChatMessage::Tool { ... }` to session
   - Go to step 2 (send tool results back to LLM)
5. If no tool calls: append `ChatMessage::Assistant { ... }` to session
6. `on_event(AgentEvent::Done)`

### 6. WebSocket Protocol (`protocol.rs` + `handler.rs`)

Red implements `WebSocketHandler` for Steel.  The existing Steel WS commands
(login, register, whoami, change_pass) remain unchanged — Red adds new
commands for the agent protocol.

**Client → Server (syntax commands):**

| Command | Args | Description |
|---|---|---|
| `session_new` | `name` (optional) | Create a new chat session |
| `session_list` | — | List user's sessions |
| `session_switch` | `id` | Switch to a session |
| `session_close` | `id` | Delete a session |
| `session_rename` | `id`, `name` | Rename a session |
| `chat` | `content` | Send a message to the current session's agent |
| `file_list` | — | List files in the current session's sandbox |
| `file_read` | `filename` | Read a file (returns content as data) |
| `file_delete` | `filename` | Delete a file |

**Server → Client (syntax responses):**

| Command | Args | Description |
|---|---|---|
| `data` | session list / file list / file content | Response to query commands |
| `info` | confirmation message | Response to action commands |
| `error` | error message | Error response |
| `text` | `content` | Streamed LLM response token |
| `tool_call` | `name`, `args` | Agent is calling a tool |
| `tool_result` | `name`, `result` | Tool execution result |
| `done` | — | Agent turn complete |

**Why syntax commands instead of raw JSON?** Steel's WS handler already uses
the syntax protocol with JDAT serialisation.  Using the same protocol means
auth commands (login, whoami) work unchanged, and we reuse `o3db.js` on the
client side with its existing `send()` method.

### 7. File Upload/Download

Files are handled via HTTP routes (not WS) for efficiency:

- **Upload:** `POST /api/upload` with multipart form data.  The file is stored
  in the current session's sandbox directory.  Requires authentication (session
  cookie).  The `api_routes` mechanism in Steel's `VhostConfig` routes this to
  a Red API handler.

- **Download:** `GET /api/download/<filename>`.  Serves a file from the current
  session's sandbox directory.  Requires authentication.

These use Steel's existing `ApiRoute` with in-process handlers, similar to how
elearnity_app handles `/api/stripe/checkout`.

### 8. Web UI (`www/`)

The frontend is a single-page app served as static files by Steel:

- **Login screen** — username/password via `o3db.js` (existing Steel auth)
- **Sidebar** — session list with new/close/rename, user info, settings
- **Chat area** — scrollable output area (markdown rendered), input box at
  bottom (Enter to send, Shift+Enter for newline)
- **File panel** — list of files in the current session's sandbox, upload
  button, download links, file preview
- **Theme toggle** — dark/light mode (existing, persisted in localStorage)
- **Settings modal** — change password (existing)

**Dependencies:**
- `o3db.js` — Steel WS auth client (138 lines, reused)
- `marked.min.js` — markdown to HTML renderer (~40KB, bundled)
- No xterm.js, no terminal emulation, no ANSI parsing

**Sidebar resize:** A draggable split handle between sidebar and chat area.
Click to toggle, drag to resize.  Width persisted in localStorage.


## Steel Integration

Red is wired into Steel via `VhostConfig`:

```jdat
{
    "hostnames": ["red.oxedyne.com"],
    "public_dir_rel": "www/red",
    "db_dir_rel": "o3db/red",
    "api_routes": [
        { "path": "/api/upload",   "handler": "red_upload" },
        { "path": "/api/download", "handler": "red_download" }
    ],
    "red_config": {
        "llm_host":      "api.fireworks.ai",
        "llm_port":      443,
        "llm_path":      "/inference/v1/chat/completions",
        "llm_key":       "{file:/path/to/fireworks-key}",
        "llm_model":     "accounts/fireworks/models/glm-5p2",
        "sandbox_root":  "sessions",
        "system_prompt": "You are Red, an AI coding assistant..."
    }
}
```

A new `RedConfig` struct in `cfg.rs` (alongside `TermConfig`) holds the LLM
configuration.  The `RedHandler` (implementing `WebSocketHandler`) is attached
to the vhost's WS handler at server startup, similar to how
`TerminalManager` was attached via `with_term_manager`.

The `AppExtension` trait is used to register the API handlers (`red_upload`,
`red_download`) with Steel's API handler registry.


## Phasing

### Phase 1: Core agent (MVP)

Goal: a working chatbot that can answer questions via streaming LLM responses.

- [ ] Create `fe2o3_red` crate with `Cargo.toml` and `lib.rs`
- [ ] `llm.rs` — LLM client with SSE streaming (Fireworks GLM-5.2)
- [ ] `agent.rs` — agent loop (message → LLM → streamed response, no tools yet)
- [ ] `session.rs` — session CRUD + conversation history in O3db
- [ ] `protocol.rs` — WS message types (session_new, session_list, chat, text, done)
- [ ] `handler.rs` — `WebSocketHandler` impl with session/chat commands
- [ ] `cfg.rs` — `RedConfig` struct + `VhostConfig` integration
- [ ] Update Steel `server.rs` to wire `RedHandler` per vhost
- [ ] Web UI: chat interface with markdown rendering, session sidebar
- [ ] Deploy to karri, test end-to-end

**Deliverable:** user can log in, create sessions, chat with GLM-5.2, see
streamed markdown responses, switch between sessions.

### Phase 2: Tools and sandbox

Goal: the agent can execute tools (shell, file ops) in a sandboxed directory.

- [ ] `tool.rs` — `RedTool` trait + `ToolRegistry`
- [ ] `tools/shell.rs` — execute commands in sandbox directory
- [ ] `tools/file.rs` — read/write/list/delete files in sandbox
- [ ] `sandbox.rs` — per-session working directory, path traversal protection
- [ ] Update `agent.rs` — handle tool calls from LLM, execute, loop
- [ ] Update `protocol.rs` — tool_call, tool_result messages
- [ ] Update Web UI — show tool calls and results in chat
- [ ] Test: agent can run `ls`, create files, read them back

**Deliverable:** user asks "create a Python script that prints hello world and
run it" → agent uses shell + file_write tools, shows results in chat.

### Phase 3: File upload/download

Goal: users can upload files for the agent to work on and download results.

- [ ] API handler: `POST /api/upload` (multipart, stores in sandbox)
- [ ] API handler: `GET /api/download/<filename>` (serves from sandbox)
- [ ] `file_list` / `file_read` WS commands for the agent
- [ ] Web UI: file panel with upload button, file list, download links
- [ ] Test: upload a CSV, ask agent to analyse it, download the result

**Deliverable:** user uploads a file, asks the agent to process it, downloads
the output.

### Phase 4: Sandboxing hardening

Goal: stronger isolation for shell command execution.

- [ ] Restricted PATH and environment for shell tool
- [ ] Optional `bubblewrap` or `firejail` integration
- [ ] Resource limits (CPU time, memory, disk per session)
- [ ] Session cleanup (auto-delete sandbox after configurable period)

### Phase 5: Extensibility

Goal: extensible tool system and MCP bridge.

- [ ] Configurable tools via `RedConfig` (enable/disable, custom params)
- [ ] Custom system prompts per session
- [ ] MCP bridge — spawn MCP servers as subprocesses, bridge stdio to tool calls
- [ ] Conversation export/import (download as JSON or markdown)
- [ ] Multi-model support (switch between LLM providers per session)
- [ ] Context window management (summarise old messages when approaching limit)


## Relationship to Existing Code

| Existing | Status in Red |
|---|---|
| `fe2o3_steel/src/srv/ws/term.rs` | Removed — replaced by Red's `handler.rs` |
| `TermConfig` in `cfg.rs` | Removed — replaced by `RedConfig` |
| `TerminalManager` | Removed — replaced by `SessionStore` + `LlmClient` |
| PTY bridge (`handle_terminal_websocket`) | Removed — replaced by structured WS protocol |
| tmux dependency | Removed — no terminal sessions |
| Goose CLI | Not used — Red has its own agent loop |
| `o3db.js` | Reused — auth commands unchanged |
| Steel WS auth (login/register/whoami/change_pass) | Reused — unchanged |
| `VhostConfig` / `VhostRuntime` | Extended — `red_config` field replaces `term_config` |
| Web UI (HTML/CSS) | Replaced — chat UI instead of terminal, no xterm.js |
| `nix` dependency | Removed — no PTY needed |
| Accept loop fix (`ae4cbb1`) | Kept — unrelated to Red, benefits all vhosts |
| ProxyRoute (`2cf0754`) | Kept — unrelated to Red, used by chat.oxegen.io vhost |


## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| LLM API format changes | `llm.rs` is isolated — only one file needs updating |
| Tool execution security | Phase 1 has no tools. Phase 2 uses directory confinement. Phase 4 adds proper sandboxing. |
| O3db performance with large conversations | Messages stored as JDAT lists. Add summarisation in Phase 5. |
| SSE parsing complexity | Manual line-by-line parsing of `data:` lines — simple, well-understood format. |
| No `serde` — manual JSON parsing | OpenAI API JSON is simple and stable. JDAT maps parse cleanly. Add a thin JSON→JDAT converter if needed. |


## Naming

- **Red** — the project name.  Short, memorable, matches the `red.oxedyne.com`
  domain and the Oxedyne red logo (`#f33c57`).
- **Red agent** — the agent loop that drives conversations.
- **Red session** — a conversation with associated files and sandbox.
- **Red tool** — a capability the agent can use (shell, file, etc.).
