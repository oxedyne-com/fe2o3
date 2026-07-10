# Red — v1 Plan

> Authoritative plan for Red v1, derived from the requirements interview of
> 2026-07-09.  Supersedes the original five-phase design in `plan.md` (kept
> for history).  Cross-references `TODO.md` for the granular backlog.

Red is the unified chat-and-coding workspace interface for Oxedyne, named
after the Oxedyne mascot flame.  It is a web-only agent that talks to
open-weight cloud inference providers, gives each user a sandboxed
server-side workspace, and unifies conversation and coding into a single
interface interacting with that workspace.  It is built as the `fe2o3_red`
crate and served by Steel, dog-fooding the Hematite library throughout.

---

## 0. North star — dissolve the distinction between chat and coding

Red erases the line between a Claude-web-chat-style conversation and a
Claude-Code-style agent.  One interface: web-chat ergonomics (rich rendering,
mobile, anywhere-access, relaxed conversation) sitting on a persistent
workspace the agent can act on at any moment — read/edit files, run commands
— with **no mode switch**, and with every action rendered legibly rather than
as a terminal dump.  This is the through-line for every decision below.

What it elevates from nice-to-have to **core**:

- **Artifacts ≡ files.**  No separate "artifact" object.  A generated
  document or program is just a file in the workspace — rendered inline,
  opened in a tab, edited, and runnable.  The workspace is Red's single
  answer to both web-chat Artifacts and Projects.
- **The workspace is general, not code-only.**  It is "your stuff" — prose,
  notes, books, config, code.  The agent handles a Typst chapter or a
  markdown note as fluently as a Rust file.  Red is a *working environment*
  that can also code, not a coding tool.
- **Rendering is core.**  Markdown, code, tables, math, images, and legible
  inline rendering of agent actions (diffs, previews, command output) are
  what make the coding half feel like web chat, not a console.
- **One thread, conversation and action.**  Pure conversation and agentic
  action live in the same thread; the transition is seamless and every action
  stays reviewable, so agency never ambushes the user.
- **Skills are the shared layer** both halves draw on.

Concrete first-user proof: refining Lucronics/Typst chapters with `/polish`
and `/improve` *and* hacking on fe2o3 in the same interface, over the one
`~/usr` tree — no tool switch.

---

## 0.1 The brief architecture (2026-07-10 pivot)

The chat-centric MVP below (WS-A..J) is the substrate; the product is the
**brief/fold architecture** from `red/README.md`.  The pivot inverts which noun
is primary.  In the MVP the unit of work is the *session/chat*; in the vision
it is the **Focus** — a durable container for a pursuit (a book, a refactor, a
diet, a philosophy) — with chats and agents disposable against it.

The one idea is separating memory from execution.  A chat conflates them and so
it rots; Red splits them:

- The **brief** is the durable, reduced state of a Focus, *and* the command
  surface you steer it from.  A persistent, pre-prompted **brief agent** sits
  behind the input line; you give it instructions, and its reply is usually an
  **action** (edit the brief, dispatch an agent, fold a delta) or **one or more
  errors** shown as an indicator — never a chat transcript.  It cannot rot
  because the brief is its fixed point: past a context threshold it silently
  re-reduces to the brief and continues.
- An **agent** is a re-taskable executor.  It forks from the brief, does a
  bounded job in its own context, produces a **delta**, and folds it back; the
  tile then persists and accepts a new task.
- **Fold** is re-reduction by a *fresh* reducer agent (handed only
  `brief + delta + rules`), reviewed as a diff with veto-on-drop until trusted.
  It replaces the hand-written handover — the brief is the current handover, by
  construction.
- The **log** is per-Focus, append-only, in a **light fe2o3-native store**
  (`jdat` + `hash` under `.red/`), *not* git.  Git appears only in **conducted**
  mode, for worker worktree isolation and file merge on the project's own repo.

The four panels, named once: **Rail** (Foci + Chats) · **Center** (the brief) ·
**Agents** · **Workspace**.  A Focus runs **direct** (one agent) or
**conducted** (many under a conductor — the former "team mode", renamed to avoid
implying human teams).  Conducted mode is designed-for now, built later.

The staged pivot path lives in `TODO.md` (Stages 0–6+).  Stages 1–4 turn the
shipped chat app into the brief/fold loop; 5 is metering polish; 6+ (conducted
mode, the DAG, per-Focus panels) is latent capability the substrate must not
preclude.

---

## 1. Positioning, v1 target, and non-goals

**Positioning: an open-source, self-hosted, web-based agentic coding
workspace, run in a trusted environment.**  Red is a skin over *your own*
compute — like Claude Code, but web-based and self-hostable — that anyone can
run for themselves.  It is **not** a commercial multi-tenant SaaS: hosting a
full agentic workspace for untrusted strangers means owning arbitrary-code
sandboxing, compute cost, and trust — the reasons no such product exists.
Sidestepping all three by being self-hosted-and-trusted is what keeps Red
simple.  Jason runs the first instance and is the first test driver.

**Target: keep it as simple and lightweight as possible while delivering all
the desired functionality.**  Chat plus full agentic coding over a workspace,
provider/model choice, a live context meter, files, sync, skills — with the
security/tenant/commercial scaffolding stripped, not the features.

**Trusted-environment posture.**  Execution runs directly; there is no
adversarial sandbox on the critical path.  A light path-jail keeps the agent
in the workspace by *accident* prevention, not as an *attack* boundary.
Single-user first; a trusted few may share an instance (keyed by user where
that is free), but no isolation walls.  BYOK is just keys in the config.

**Out of scope (not the product):**

- Billing, self-serve signup, spending limits, admin panel, per-tenant
  isolation — the commercial machinery is gone, not deferred.
- Hardened multi-tenant sandboxing (bubblewrap/nsjail/micro-VMs).
- Per-user encrypted key vaults (trusted env: keys live in config).

**First fast-follow (immediately after v1):**

- **MCP host** — spawn/connect MCP servers (stdio + HTTP), surface their
  tools alongside native ones; unlocks a browser tool and the whole MCP
  ecosystem. Generic enough to live in fe2o3.

**Deferred (wanted later):**

- fe2o3-native sync (Syncthing covers v1).
- Tool/script-bearing skills and multi-step workflow skills (v1 skills are
  instruction bundles only).
- In-app code editor (viewing offloads to the browser; editing is the
  agent's job or via the synced local editor).
- Plugins, webhook/API access.

---

## 2. Architecture decisions (decision log)

Each row is a settled decision from the interview.  The rationale is
recorded so future sessions don't relitigate.

| # | Decision | Rationale |
|---|----------|-----------|
| D0 | **Open-source, self-hosted, trusted environment.** Run-it-yourself, not a commercial SaaS. No adversarial sandbox, no billing/tenant machinery. | Dissolves the three hard problems of cloud agentic coding (sandbox security, compute cost, trust) instead of fighting them; keeps Red simple. |
| D1 | **Chat-centric UI.** Conversation is the main surface; files, diffs, command output, and file previews render inline in the stream. Collapsible session/file sidebar. | Best fit for vanilla JS and full mobile parity; builds on the existing UI. |
| D2 | **Full agentic agent.** Read/write/edit files, list/search the tree, run shell commands (build, test, git). | The point of a Claude Code replacement. |
| D3 | **Workspace lives on the host running Red.** A per-user directory (path-jailed as an accident guardrail); the agent executes there directly, no adversarial sandbox. | Trusted env (D0); matches "assigned a workspace"; avoids split-execution complexity. |
| D4 | **Pluggable `Executor`.** The agent never calls `std::process::Command` directly; it calls an `Executor` enum with a `Local` variant now and a `Remote` variant possible later. | Local isn't a security sandbox, just a run location; Remote is the self-hoster's escape hatch to offload heavy builds to a bigger box — a config change, not a rewrite. |
| D5 | **One workspace per user, many sessions, per-session cwd.** Sessions are conversation threads over the shared workspace; each can set a working subdirectory. | Single sync target; matches "assigned a workspace". |
| D6 | **Generic OpenAI-compatible providers.** One client plus a config list of endpoints `{name, base_url, key, models, pricing}`. Adding a provider is a config edit. | Nearly all open-weight providers (Fireworks, Together, DeepInfra, Groq, Novita, Hyperbolic, OpenRouter) speak the same Chat Completions API. |
| D7 | **Syncthing for v1.** Workspace stays a plain directory; sync is external. fe2o3-native sync logged as the dog-food endpoint for later. | Zero build; already in use for `~/usr`. |
| D8 | **Skills are named markdown instruction bundles** in the workspace (`.red/skills/<name>.md`), injected on invocation. | Simple, portable, user-authored; scripts/workflows deferred. |
| D9 | **Skill syntax: `<name args>` … optional `</name>`/`</>`, tolerant.** Only the opening tag is `>`-terminated, so `>` in the payload is safe; missing `>`/close never fatal. Parsed by `fe2o3_syntax`. | Delimits directive from inputs for both the parser and the LLM; angle tags are the most LLM-idiomatic delimiter; dog-foods fe2o3. |
| D10 | **Full mobile parity** is a v1 acceptance criterion. | Explicit requirement; drives responsive/touch-first design from the start. |
| D11 | **File view = new browser tab**, served by Steel with a proper content-type (default `text/plain`). Full file/dir management + upload + download in-UI. | Offloads rendering to the browser; no in-app viewer to build; identical on mobile. |
| D12 | **Live context meter + breakdown** (system / history / files / tool output vs the model's limit) is the v1 signature feature. Richer cost (per-token-class, pre-send prediction, cache-hit) is fast-follow. | The differentiator; existing basic cumulative cost stays. |
| D13 | **Vanilla server-rendered HTML + JS/CSS**, no framework. | Standing preference; the app is already vanilla. |
| D14 | **Reuse elearnity's O3db auth** + login popup (`o3db.js` already shared). | Auth is solved; single-user first, trusted-few optional (no isolation walls). |
| D15 | **MCP is the extensibility spine; web search is native in v1.** Files/shell/web-search are native `RedTool`s now; MCP host (browser + the ecosystem) is the first fast-follow. Tools are curated per session. | One MCP integration inherits hundreds of servers; MCP's arbitrary-code nature fits the trusted env (D0). Web search native = reliable + zero-setup. MCP host is generic → an fe2o3 opportunity (JSON-RPC/stdio/HTTP, likely in `fe2o3_net`). |
| D16 | **The unit of work is a Focus, not a chat.** A Focus (plural Foci) is the durable container for a pursuit — deliverable or not. Chats and agents are disposable against it. | Inverts the MVP's chat-primacy; makes the brief the memory the workflow lives in. "Foci" chosen over Project (implies an endpoint) / Pursuit / Strand for its resonance with the reduce/fold mechanic. |
| D17 | **The brief is a command surface backed by a persistent, pre-prompted brief agent.** Instructions resolve to an action or to one or more errors (an indicator, not a transcript). It self-reduces to the brief when its context fills. | Distinguishes a brief (steer, reply-is-action) from a chat (visible thread). The brief-as-fixed-point is what keeps the persistent agent rot-free. |
| D18 | **Agents are re-taskable, not one-shot runs.** An agent folds its delta and then accepts a new instruction through the same tile. | The task can change, so "agent" not "run"; collapses the run/agent split into one word. |
| D19 | **Fold is done by a fresh reducer agent** handed only `brief + delta + rules`; reviewed diff with veto-on-drop until trusted, then silent. | Semantic re-reduction needs judgement (an LLM), not code; a stateless fresh reducer cannot itself rot. |
| D20 | **Memory (log + brief versions) is a light fe2o3-native store, not git.** `jdat` records + `hash` content-addressing under `.red/`, per Focus. | Avoids git-in-git over a project's own repo; the brief's merge is semantic (the reducer), so git's line-merge buys nothing here. An event log is far simpler than reimplementing git. |
| D21 | **Conducted mode replaces "team mode"; git is load-bearing only there.** Many agents under a thin conductor, each owning a git worktree/branch on the project's own repo; governor caps concurrency; conflicts escalate one decision. | "Team" wrongly implies humans. Worktree isolation + 3-way file merge is exactly git's job; the fold stays a reducer agent, never a git merge. Designed-for now, built later. |
| D22 | **The generic log+reducer core is earmarked for an existing fe2o3 crate** (candidate `fe2o3_data`); built app-local first, lifted once its shape settles. No new crate without express permission. | Event-sourcing is generic (CLAUDE.md: extract generic code upstream); iterate in `fe2o3_red` while the shape moves. |

### OOM: an operator concern, not a product risk

Under self-hosting this dissolves.  A self-hoster runs Red where they have
the RAM and toolchain; running it on an undersized box and having a build
OOM is an ops choice, like running any dev tool on too little hardware — not
a flaw Red must design around.  For Jason's own instance: if karri (1.9 GB,
shared with elearnity/oxegen/mail) strains under real Rust builds, the
`Executor` (D4) offloads execution to a bigger box or the dev machine — a
config change, not a rearchitecture.  Optional cgroup caps remain available
as courtesy guardrails, not as a security boundary.

---

## 3. Workstreams

Ordered roughly by dependency.  Each item maps to `TODO.md` where relevant.

**Status (2026-07-10):** WS-A ✅, WS-C ✅, WS-D ✅ (verified end-to-end vs live
GLM-5.2), WS-E ✅ (skills, 18 tests), WS-F ✅ (file browser), WS-G ✅ (context
meter), WS-I ✅ (mobile), rich rendering ✅ — all committed and pushed.  WS-B
partial: the model picker is now functional (agent uses the session's model);
multi-provider/BYOK config remains.  59 unit tests pass.  Client UI is
syntax-checked and deployed locally but pending Playwright *visual*
verification (the dev browser's X display dropped mid-session) — intended for
manual testing.

**Reframe (2026-07-10):** these workstreams shipped the chat MVP and now serve
as substrate for the brief/fold architecture (§0.1).  The live front-end
already carries the four-panel shell (Rail/Center/Agents/Workspace) wired
chat-first; the pivot's near-term work is Stages 1–4 in `TODO.md` (Focus
container → brief command surface → log+reducer → the fold loop).  WS-B..J below
are re-homed under those stages rather than pursued as standalone chat-app
features.

### WS-A — MVP hardening (finish what exists)
The current crate streams chat but has the P0 gaps from `TODO.md`.

- `max_tokens` on every request (configurable; default e.g. 4096) — stops
  GLM-5.2 reasoning loops.
- Stop/abort button — cancel the in-flight `chat_stream` future via a
  cancellation token checked in the `LineReader` read loop; send `done`.
- WebSocket reconnection — auto-reconnect + re-auth, preserving session and
  history.
- Remove verbose `info!` logging in `llm.rs`/`handler.rs`; move to `debug!`.
- Favicon (Red/Oxedyne); fix the 404.
- Empty state + session timestamps (small UX wins).

### WS-B — Providers as config
- `Provider`/endpoint model in `RedConfig`: a list of OpenAI-compatible
  endpoints `{name, base_url, key, models:[...], pricing:{model:{in,out,cached}}}`,
  with `{env:}`/`{file:}` indirection for keys (Steel already supports this).
- Per-session provider + model selection (UI already has a model picker;
  extend to pick endpoint first, then model).
- Model list + context-window length per model sourced from config (or the
  provider `/v1/models` endpoint as a later refinement).

### WS-C — Workspace and the `Executor`
- Per-user workspace dir under the Red host (e.g. `workspaces/<user>/`),
  path-jailed via `resolve()` — an accident guardrail (agent stays in the
  workspace by default), not an attack boundary (trusted env, D0).
- `Executor` enum (D4); `Local` variant runs commands directly under the Red
  process's user, with `timeout` and *optional* cgroup caps as courtesy
  guardrails. No bubblewrap/nsjail in v1.
- Per-session cwd within the workspace.

### WS-D — Agent tools (native)
The tool-call loop here is the shared substrate for both native tools and
(later) MCP tools — they land in the same flat tool list to the model.

- `RedTool` trait + `ToolRegistry` (`name`, `description`,
  `parameters_schema`, `execute`); provides tool definitions to the LLM.
- File tools: `file_read`, `file_write`, `file_edit`, `file_list`,
  `file_search`, `file_delete` — all via the workspace `resolve()`.
- Shell tool: run commands through the `Executor`; stream stdout/stderr.
- **Web search tool (native, v1):** Exa first, others (Brave/Tavily) as
  config, using the same endpoint-config pattern as providers
  (`{name, base_url, key}`). First-class and zero-setup — the "chat half"
  needs research without any MCP.
- Agent loop: handle `tool_calls` in the LLM response, execute, append
  `ChatMessage::Tool` results, loop until no more calls.
- Per-session tool curation: enable only the tools a session needs (too many
  tools degrade selection accuracy and cost context — the Goose 18-tools
  lesson). Surface tool-calling reliability as a model-picker consideration.
- Inline UI: tool calls (name/args) and results rendered distinctly and
  collapsibly in the chat stream; diffs rendered inline.

### WS-E — Skills
- Skill store: `.red/skills/<name>.md` with frontmatter (`name`,
  `description`); listed for autocomplete.
- Grammar in `fe2o3_syntax`: parse `<name args>` … optional `</name>`/`</>`,
  tolerant per D9; shared server + client so validation matches.
- Invocation: on send, expand a skill tag by injecting the bundle
  (instructions + referenced files) into the turn; autocomplete `<` from the
  skill list.

### WS-F — File management UI
- Full file/dir operations (browse, create, rename, move, delete, mkdir).
- Upload (drag-drop + picker) into the workspace at a chosen path;
  `POST /api/upload`, authed, path-jailed.
- Download files; download a folder as zip; `GET /api/download/...`.
- View: clicking a file opens `GET /workspace/file?path=...` in a new tab,
  served with a sniffed content-type, default `text/plain` (D11), authed,
  path-jailed.

### WS-G — Context meter + cost (v1 slice)
- Server computes per-turn token buckets: system, history, attached files,
  tool outputs.  Actuals from the provider `usage` chunk (`prompt_tokens`,
  `completion_tokens`, `prompt_tokens_details.cached_tokens`); pre-send
  estimate via a heuristic tokeniser (chars/4-class) until a real tokeniser
  is warranted.
- UI: a live meter `used / limit` (e.g. `42k / 1M`) with the breakdown, per
  session, updating each turn.  Keep the existing cumulative cost line.

### WS-H — Sync (Syncthing)
- Install/configure Syncthing on the Red host; one share per user
  workspace.  Document the pairing flow (add the workspace folder on the
  user's chosen device).
- Workspace stays a plain directory — no coupling in Red.

### WS-I — Mobile parity pass
Mostly adaptive CSS; the extra work is a small set of touch interactions with
no direct desktop equivalent, largely because D11 (file view = new browser
tab) already offloads file *viewing* to the browser identically on mobile.

- Responsive layout (the bulk, and cheap): media-query reflow, sidebar
  collapses to a hamburger; chat, file manager, diffs, skills, settings all
  usable at 375 px.
- Touch-specific interaction paths (the only non-CSS part): file-picker
  upload in place of OS drag-drop; long-press or a "⋯" action menu in place
  of right-click (rename/move/delete); tap-to-expand in place of hover
  affordances; drop the draggable sidebar divider on small screens.
- Treat parity as an acceptance gate, tested each workstream, not a final
  bolt-on.

### WS-J — MCP host (first fast-follow, not v1)
Layers on WS-D's tool-call loop; not gating v1.

- MCP client/host: JSON-RPC 2.0 over stdio (subprocess) and HTTP; initialize
  handshake, `tools/list`, `tools/call`; server lifecycle management.
  Generic → build in fe2o3 (`fe2o3_net` module), dog-fooded.
- Surface MCP tools in the same flat tool list as native tools; dispatch
  `tool_calls` to the owning server.
- Config: which MCP servers to run per instance; enable per session (curation).
- Unlocks a browser tool (e.g. Playwright MCP) and the wider ecosystem.
  Browser is Chromium-heavy — run where resources allow.

### Rendering (cross-cutting, core per the north star)
Not a single workstream — a quality bar across WS-A/D/F: web-chat-grade
markdown (tables, code, math, images), syntax highlighting, and legible
inline rendering of agent actions (diffs, previews, command output) rather
than terminal dumps. This is what makes the coding half feel like chat.

### Hardening & non-functional requirements (2026-07-10 sceptical review)

**Threat-model correction.** D0's "trusted environment" covers the user
attacking themselves, *not* third-party content attacking the user through the
agent. A poisoned file / web-search result / repo that the agent reads is prompt
injection, and prompt injection in an agentic tool is **RCE as the run user**.
The following are requirements; `TODO.md` re-stages the near-term work so H1 and
H3 land *before* the LLM-heavy stages (a new "Stage H"), and H2/H4/H5 constrain
Stages 2–4.

- **H1 — Isolate the run-user (security).** Dedicated unprivileged user on a box
  with no wallet/prod secrets; scrubbed `Executor` child env (no `STEEL_*` /
  keys); restricted `PATH`; egress allow-list or logging for the shell tool.
  Single-user until done. The current posture (agent shell as `jason` next to
  the wallet on the prod box) is the highest-severity gap.
- **H2 — Reducer advisory first; folds never block work or lose data.** The fold
  is a non-deterministic LLM mutation of durable state on the critical path.
  Manual/proposed fold before any auto-reducer; always retain the raw delta;
  queue folds and degrade to append-without-reduce on provider outage;
  **serialise folds per Focus.** The append-only log + version snapshots make
  every fold an auditable, revertible transaction — protect that property.
- **H3 — Parse provider JSON with `fe2o3_jdat`; lift HTTP/SSE into `fe2o3_net`.**
  The hand-rolled `extract_json_*` / `LineReader` scanners in `llm.rs` are a
  correctness and DoS surface (documented bugs: `reasoning_content` mis-match,
  whitespace-after-colon). Decode to `Dat` and navigate; move the streaming
  client upstream. This is the correct way to satisfy the fe2o3-first
  requirement — the constraint is fine, the hand-rolled execution is the
  liability.
- **H4 — Log as WAL, single writer, crash-safe.** One per-Focus writer task owns
  `.red/log` (multi-KB JDAT records are not atomic under `O_APPEND`); the log is
  the source of truth and the commit point; the O3db index is a derived,
  rebuildable cache; `fsync` at commit; readers tolerate a torn trailing record.
- **H5 — Escape-by-default frontend; stateless brief agent; `Branch` enum.**
  Every `innerHTML` interpolation escapes and the markdown renderer's output is
  sanitised (model and file content are untrusted → DOM XSS). The brief agent is
  stateless per instruction (reconstructs context from `brief.md`), not
  long-lived server state. Branches are an enum (`Chat | Agent`), not a
  field-flagged god-struct (CLAUDE.md enum preference).

Decisions that survive the review unchanged: Foci + light per-Focus log (not
git-for-memory), git only for conducted-mode worktree isolation, one `Agent`
across three prompts (with the reducer tool-less by construction), chat
transcript as an append-only file + derived index, agent transcripts in-memory
reduced to a delta.

---

## 4. Development workflow (process fix)

The previous work was built and tested against the live karri deployment.
Going forward:

- Develop and run Steel in **local dev mode** with a `red` app under
  `~/usr/code/web/apps/red/`, a local O3db, and a local workspace dir.
- Drive the UI locally with Playwright; verify before any deploy.
- Deploy to karri only once verified locally (scp binary + assets, setcap,
  restart), per the established karri deploy workflow.
- fe2o3 changes committed as coherent units and pushed so the other machine
  can pull.

---

## 5. Open questions / to confirm

- **Pre-send token estimate accuracy.** Heuristic vs a real tokeniser per
  model family — start heuristic, revisit if the meter feels off.
- **In-chat syntax highlighting.** The new-tab view offloads file rendering,
  but in-chat code blocks still want highlighting; decide between a small
  bundled highlighter and deferring.  (No framework either way.)
- **Syncthing on karri.** Confirm it can run alongside prod without resource
  strain, or whether sync targets the eventual larger Red host instead.
- **BYOK timing.** Shared keys for v1; per-user encrypted keys (reusing
  Steel's wrapped-key/wallet patterns) is the first pre-multi-user task.

---

## 6. Immediate next steps

1. Stand up the local dev loop (`~/usr/code/web/apps/red/`, local Steel,
   local workspace) and register a throwaway account to exercise the current
   build end-to-end.
2. WS-A MVP hardening (max_tokens, stop, reconnect, logging, favicon).
3. WS-C `Executor` + workspace scaffold, then WS-D tools — this is the
   coding half and the largest lift.
4. WS-E skills grammar in `fe2o3_syntax` in parallel (independent).
