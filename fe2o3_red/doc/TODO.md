# Red — TODO

> Last updated: 2026-07-10
> Reframed around the **brief/fold architecture** (see `red/README.md` and
> `plan_v1.md` §0.1). The unit of work is a **Focus**, not a chat/session.
> Stages are roughly sequential; each one leaves a working app. Priority is
> within a stage.

## Vocabulary (settled 2026-07-10)

- **Focus / Foci** — the durable container for an ongoing pursuit (a book, a
  refactor, a diet, a philosophy). Holds a brief, a log, and owned files.
  Replaces "Projects".
- **brief** — the reduced, curated state of a Focus, and the command surface
  you steer it from. Backed by a persistent, pre-prompted **brief agent**.
- **agent** — a re-taskable executor. The centre one maintains the brief;
  worker ones do bounded jobs and fold their deltas back. Replaces "run".
- **fold** — re-reduction: a fresh reducer agent folds a delta into the brief
  and bumps its version. Replaces the hand-written handover.
- **log** — a per-Focus, append-only event record. Light fe2o3-native store,
  **not git**.
- **direct / conducted** — one agent on a Focus, or many under a conductor.
  Replaces "team mode".

---

## Hardening & non-functional requirements (2026-07-10 sceptical review)

Threat-model correction: "trusted environment" (plan D0) covers the user
attacking themselves, **not** third-party content attacking the user *through*
the agent. A poisoned file, web-search result, or repo that the agent reads is
prompt injection, and prompt injection in an agentic tool is **RCE as the run
user**. These are requirements, not nice-to-haves; each note says which stage it
constrains.

- **H1 · Isolate the run-user (security; do early).** Run the Red backend as a
  dedicated unprivileged user on a box with no wallet/prod secrets; scrub the
  `Executor` child env (no `STEEL_*` / keys); restricted `PATH`; egress
  allow-list or at least logging for the shell tool. Single-user until done.
  → **Stage H.**
- **H2 · Reducer advisory first; folds never block work or lose data
  (safety / resilience).** Ship manual/proposed fold before any auto-reducer;
  always retain the raw delta so a fold can be redone; queue folds and degrade
  to append-without-reduce when the provider is down; **serialise folds per
  Focus** (no concurrent reduction against one brief HEAD). → constrains
  **Stage 4.**
- **H3 · Parse provider JSON with `fe2o3_jdat`'s decoder; lift the HTTP/SSE
  client into `fe2o3_net` (safety / maintainability).** Replace the hand-rolled
  `extract_json_*` / `LineReader` string-scanners in `llm.rs` (documented bug
  class: `reasoning_content` mis-match, whitespace-after-colon) with jdat
  decode-to-`Dat` + `Dat` navigation; move the streaming HTTP client upstream.
  This is how the fe2o3-first requirement is meant to be met. → **Stage H**
  (before the LLM-heavy stages).
- **H4 · Log as WAL + single writer + crash-safety (correctness / durability).**
  One per-Focus writer task owns `.red/log` (no reliance on `O_APPEND` atomicity
  for multi-KB records); the log is the source of truth and the commit point;
  the O3db index is a derived, rebuildable cache; `fsync` at commit; readers
  tolerate a torn trailing record. → constrains **Stage 3.**
- **H5 · Escape-by-default frontend; stateless brief agent; `Branch` enum
  (security / maintainability).** Every `innerHTML` interpolation escapes;
  sanitise the markdown renderer's output (model and file content are
  untrusted). The brief agent is **stateless per instruction** (reconstructs
  context from `brief.md`), not long-lived server state. Model branches as an
  enum (`Chat | Agent`), not a god-struct — per CLAUDE.md's enum preference.
  → constrains **Stages 2, 4.**

---

## Stage 0 — Name & concept lock (doc-only)

- [x] Settle vocabulary (above) and the four panel names: **Rail** (Foci +
  Chats) · **Center** (the brief) · **Agents** · **Workspace**.
- [~] Thread the vocabulary and the brief-agent model through `plan_v1.md` and
  `red/README.md`.
- [ ] Rename "Projects" → "Foci" in the UI copy and the client code.

## Stage 1 — Focus as a first-class container (the structural inversion)

Today `Session` is the top-level unit; a Focus must sit above it.

- [ ] `Focus` type + O3db store, keyed per user, sitting above `Session`.
- [ ] Focus directory layout under the workspace: `<ws>/<focus>/` holding
  `brief.md`, `.red/log`, and owned subtrees.
- [ ] A default **Scratch** Focus; migrate the current single workspace into
  it so nothing breaks and briefless chats keep working.
- [ ] Rail: the **Foci** section lists Foci (each with its own context/cost
  meter); selecting one scopes the Center, Agents, and Workspace panels to it.
- [ ] Chats belong to a Focus (the Chats section is per-Focus).

## Stage H — Hardening foundations (before the fold loop, Stages 2–4)

Pulled ahead of the LLM-heavy stages because they underpin them.

- [ ] **H1** — run-user isolation: dedicated unprivileged user, off the
  wallet/prod box; scrub the `Executor` child env; restricted `PATH`; egress
  logging for the shell tool.
- [ ] **H3** — swap the hand-rolled JSON scanning in `llm.rs` for `fe2o3_jdat`
  decode-to-`Dat`; move the streaming HTTP/SSE client into `fe2o3_net` (fill the
  gap upstream if it is missing there).

## Stage 2 — The brief as the Center's command surface

- [ ] **Brief view**: render `brief.md` (reuse `render.js`) when a Focus is
  selected and no chat is open; header carries the editable title, a **Chat**
  button, and the Center's own thin meter.
- [ ] **Brief agent**: persistent, pre-prompted with its role; the input line
  takes instructions and commands (not conversation).
- [ ] **Response protocol**: the brief agent either **acts** (edit the brief,
  dispatch an agent, fold a delta, close a thread) or returns **one or more
  errors**, surfaced as a compact status indicator (icon / lights), never as a
  chat transcript. Quick one-shot questions get a one-shot answer that does not
  accumulate.
- [ ] Direct editing of `brief.md` with write-back (user may always edit).
- [ ] Boundary in the UX: steer or ask-once → the brief line; sustained
  visible dialogue → spin a chat.

## Stage 3 — Log + brief versions (the substrate)

- [ ] Per-Focus append-only `.red/log` of event records (`dispatch`, `fold`,
  `edit`, `drop`, `decision`), one per line.
- [ ] Brief versioning: each fold bumps a version; snapshots content-addressed;
  restore and diff any past version.
- [ ] **Light fe2o3-native store**, not git: `jdat` records + `hash`
  content-addressing, all under `.red/` and independent of any project's own
  VCS (so a Focus over `~/usr` never has Red committing to fe2o3).
- [ ] Earmark the generic log+reducer core for extraction into an existing
  fe2o3 crate (candidate: `fe2o3_data`); build app-local first, lift once the
  shape settles. **No new crate without express permission.**

## Stage 4 — Agents as folding branches (the fold loop — the heart)

- [ ] Refactor `Session` → a **branch** record with `kind: chat | agent` and
  `parent_brief_version`; a chat and a worker agent are one primitive seen from
  two chairs.
- [ ] Dispatch an agent from the brief (slash command or a button on an open
  thread); it runs the existing tool loop in its own context.
- [ ] On completion the agent produces a compact **delta** and offers **Fold
  in**.
- [ ] **Fold** = a fresh **reducer** agent handed only `brief + delta + rules`;
  it emits the new brief, shows the diff, lets the user **veto drops**, appends
  to the log, and bumps the version. The reducer holds no history, so it cannot
  rot.
- [ ] **Auto-fold trigger**: when any agent's context crosses a threshold
  (~65%) it self-reduces to the brief and continues fresh — the brief is the
  fixed point its own agent resets to. Reviewed at first; silent once trusted.
- [ ] **Re-taskable agent**: after a fold the tile persists and accepts a new
  instruction (the task rotates through the agent; the agent does not die).
- [ ] Chats fold or dispose the same way (fold = merge, dispose = prune).
- [ ] Inline rendering of agent actions — diffs, previews, command output —
  rather than terminal dumps.

## Stage 5 — Metering everywhere (the "Red" requirement)

- [ ] Per-agent tile meter (context % + cost) from the token counts already
  tracked.
- [ ] Per-Focus rollup, and a top-bar total: the session meter when **direct**,
  a fleet-style meter (agent count · budget · governor) when **conducted**.
- [ ] The brief's own thin meter in the Center header.

## Stage 6+ — Deferred (design only; the substrate must not preclude these)

- [ ] **Conducted mode**: the Center becomes a thin conductor that decomposes
  the brief's open threads into a task board and dispatches workers; each worker
  owns a **git worktree/branch** on the project's *own* repo; ownership /
  partitioning keeps writes apart; a **governor** caps concurrent agents; a
  **conflict** escalates one decision to the user. Real git here (worktrees +
  3-way file merge); the fold stays semantic (agent), never a git line-merge.
- [ ] **DAG surfaces**: speculative sibling agents off one brief version, chains
  (an agent depending on a prior fold), subtrees (a heavy agent spawning
  workers), and **replay** (reopen a past brief version, change one instruction,
  re-run forward — the log branches for free).
- [ ] **Per-Focus panels**: the Center as customisable `html/js/css` copied from
  a template, hosting widgets the agent reads back as parameters. Until then:
  one fixed renderer, many `brief.md`.
- [ ] **Per-Focus profiles/runners**: what a Focus may touch, how it builds and
  tests, which tools it has (declaring a profile is how you add a Focus).
- [ ] **MCP host** (first fast-follow): JSON-RPC over stdio + HTTP, tools in the
  same flat list; unlocks a browser tool and the ecosystem. Generic → fe2o3.

---

## Carried over from the chat MVP (still needed — now serving the fold loop)

These shipped as the chat app; they are not discarded, they become substrate.

- [x] Streaming, `max_tokens`, stop/abort, WS reconnect, favicon, empty state,
  timestamps (was WS-A).
- [x] Tools (file read/write/edit/list/search/delete, shell) + skills — now the
  agent's hands.
- [x] File management UI + mobile parity — now the Workspace panel; keep parity
  an acceptance gate each stage (four panels swap on mobile via `#mnav`).
- [x] Context meter — now decomposed per agent and per Focus.

## Cut (commercial machinery — not the product, per plan_v1 D0)

- Billing, per-user usage aggregation, spending limits, admin panel.
- Hardened multi-tenant sandboxing (trusted environment).
- Per-user encrypted key vaults (keys live in config).
- Multi-provider / BYOK stays wanted but drops to Stage 6+ priority.
