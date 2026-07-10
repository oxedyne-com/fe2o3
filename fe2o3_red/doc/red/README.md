# Red — project outline

A unified interface for local agentic work and chat. One surface for
long-horizon pursuits (a Typst book, a Rust codebase, a diet, a philosophy) and
for open-ended thinking, built so that the context-window limit stops governing
the workflow.

This document is the executor brief. It states the model, the interface, the
data, the agent lifecycle, and the coordination layer in enough detail to build
from, and ends with the decisions still open. The accompanying `index.html` +
`assets/` is a clickable reference mockup of the target layout and interactions.

> **Vocabulary (settled 2026-07-10).** A **Focus** (plural **Foci**) is the
> durable container for a pursuit — deliverable or not. Its reduced state is the
> **brief**. An **agent** is a re-taskable executor. **Fold** is re-reduction.
> The **log** is per-Focus and append-only. A Focus runs **direct** (one agent)
> or **conducted** (many under a conductor).

---

## 1. The reframe

The failure mode of a normal chat is that one session is both the memory and the
execution. It accumulates because it is the memory, it burns because it is the
execution, and past ~60–70% of the window the execution poisons the memory
(context rot). The manual fix people use — ask for a handover doc, restart,
re-read it — is lossy compaction done by hand under duress.

Red separates the two up front and makes that separation the whole loop:

- The **brief** is durable, curated, small. It is the reduced state of a Focus.
  You live here.
- An **agent** is disposable execution. It forks from the brief, does one
  bounded job in its own context, produces a delta, folds the delta back, and is
  re-tasked or discarded.
- **Folding** is re-reduction: the brief is recomputed to absorb the delta and
  drop what is now superseded. Folding replaces the handover — the brief is
  always the current handover, by construction.

Restarting a session stops being an event. The brief was the memory all along;
the transcript was disposable.

---

## 2. The substrate (why this is a DAG)

Store a Focus as an append-only event log plus a reducible state. Then:

- the brief is the reducer output (current `HEAD`),
- an agent's work is a branch that forks from a brief version,
- a fold is a merge that appends to the log and bumps the brief to its next
  version,
- a chat is an ephemeral branch: folding it is a merge, disposing it is pruning
  a dangling branch.

There is **one log per Focus**, not a global log — a Focus is self-contained, so
copying its directory copies its whole history. The visible interface is a
projection over this substrate. Two projections are offered: a linear chat view
(a chat), and a state-document view (a brief). Same underlying graph, different
lens.

The everyday flow uses the simple slice — trunk plus branches that merge
straight back. The full DAG is latent and gives, for free: agents that depend on
a prior agent's fold (a chain), speculative sibling branches off one version
("try it two ways, keep the winner"), a heavy agent that spawns its own children
(a subtree), and replay (reopen any past brief version, change one instruction,
re-run forward).

The log and the brief history are held in a **light fe2o3-native store**, not
git. Git's headline feature — line-based merge — is the wrong tool for the
brief, whose merge is *semantic* (done by a reducer agent, §4), and a git repo
per Focus would nest badly inside a project that already has its own VCS. Git
earns its place only in **conducted** mode (§7), where worker agents edit real
code in parallel and want worktree isolation and file merge.

---

## 3. Layout — four panels

```
┌──────────────────────────────────────────────────────────────────────┐
│ Red · session/fleet meter (context % + cost, always visible)           │
├────────────┬───────────────────────────────┬───────────────┬──────────┤
│ RAIL       │ CENTER                        │ AGENTS        │ WORKSPACE │
│ Foci +     │ the brief (or a chat)         │ metered tiles │ file tree │
│ Chats      │ command surface, html/js/css  │ one per agent │ = source  │
│            │ steering box at the bottom    │               │ of truth  │
└────────────┴───────────────────────────────┴───────────────┴──────────┘
```

The four panels, named once for communication: **Rail** (Foci + Chats) ·
**Center** (the brief) · **Agents** · **Workspace**. On mobile they swap rather
than tile, so each must stand alone.

### Rail (left)
- Two headers, `Foci` and `Chats`, each with a `+` button.
- Foci are persistent; each shows its own context % and cost. Clicking a Focus
  selects it and loads its brief into the Center. Clicking a Focus name renames
  it.
- Chats are ephemeral and selectable. The list is meant to stay short: a chat is
  either folded into a Focus or disposed. A chat is deletable only from the
  Center (see below), not from the rail.

### Center panel (the brief)
- The panel is the Focus's brief, and it is a **command surface**, not a passive
  document. Behind it sits a persistent **brief agent**, pre-prompted with its
  role, that you steer from the input line at the bottom.
- **You give it instructions and commands; its reply is usually an action** —
  edit the brief, dispatch an agent, fold a delta, close a thread. It differs
  from a chat, which is a visible back-and-forth you live in and read as a
  thread.
- **Response protocol.** Every instruction resolves to either **an action** or
  **one or more errors**. Errors surface as a compact status indicator (an icon
  or lights), never as a chat transcript, so the brief surface stays quiet. A
  quick one-shot question gets a one-shot answer that does not accumulate.
- **Self-reduction.** The brief agent is persistent but cannot rot, because the
  brief is its fixed point: when its working context swells past a threshold it
  silently re-reduces back to the brief and carries on from that reduced state.
  The brief is simultaneously the artifact you read and the checkpoint its own
  agent resets to.
- The panel is itself `html/js/css` living in the Focus's workspace directory,
  copied at Focus creation from a default template parked higher in the tree, so
  each Focus can diverge. (Deferred; until then one fixed renderer, many
  `brief.md`.) The template provides an editable panel title and can host
  widgets whose settings the agent reads back as parameters.
- Header carries: the editable title, a **Chat** button, and this panel's own
  context/cost meter.
- The **Chat** button starts a new chat, inserts it at the top of the Chats
  list, and loads the current brief as that chat's context. Use it to explore an
  idea against a Focus without polluting the brief.
- Boundary in use: **steer or ask-once → the brief line; sustained visible
  dialogue → a chat.**
- Center has two view states: the **brief view** (default per Focus) and a
  **chat view** (when a chat is selected). The chat view is a linear thread; its
  header shows which brief is loaded as context and carries the **Delete**
  control.

### Agents rail
- One tile per agent, showing name, kind badge (light / heavy / research), an
  optional branch/worktree, a context bar, context %, and cost.
- A finished agent shows its result and a **Fold in** button; folding drops the
  delta into the brief. The tile then persists and can be **re-tasked** with a
  new instruction — the agent does not die, the task rotates through it.
- Metering here is load-bearing, not decoration: it is how a bloating brief or a
  runaway heavy agent becomes visible before it costs anything.

### Workspace
- The file tree is the source of truth. The brief lives here as a content file
  (`brief.md`); `panel/` holds the per-Focus panel; shared assets (`app.css`,
  `brief.js`) sit higher in the tree. `.red/log` is the append-only event log.
- In conducted mode, directories carry an owner badge (which agent owns which
  subtree) and contested files carry a warning.
- User and agent both read and write here; each user's workspace is private.

---

## 4. Keeping the brief brief

If the brief bloats, context rot has simply moved up one layer. The discipline
that prevents it:

- **Fold is re-reduction, not append, and it is done by a fresh agent.** Each
  fold spawns an ephemeral **reducer** handed only the current brief, the one
  delta, and the reduction rules. It recomputes the state, drops what the delta
  supersedes, and dies. Holding no history, the reducer cannot itself rot — the
  one process whose job is to prevent rot is structurally immune to it.
- **Content / presentation split.** The reducer curates a structured content
  file (`brief.md` — plain, diffable). A fixed template renders it. The agent
  never rewrites the HTML shell, so a bad fold is a bad line, not a broken page,
  and history is diffs of readable content.
- **Detail lives in the workspace and the log.** The brief points at files
  rather than containing them, so it stays short; superseded detail drops to
  `.red/log`, out of view but queryable on replay.
- **Visible budget.** The brief has a soft cap; exceeding it signals
  re-reduction or archival.
- **Veto on drop.** The fold surfaces what it is about to drop and lets the user
  veto — brevity as a steerable operation. Folds are reviewed diffs at first and
  become silent only once the reducer has earned trust.

---

## 5. Agents — lifecycle and vocabulary

An agent is created when a piece of work should leave the brief and run in its
own session, so the noise (file edits, compiles, searches) never clogs the
brief. It forks from the current brief state, does its bounded job, folds its
delta back, and is then re-tasked or discarded. Agents are cheap; the brief
persists.

Two independent axes describe an agent:

**Context policy — chosen at creation, a property of the task:**
- **light** — does its bit, externalises to the brief, stays thin. In and out.
  Most agents.
- **heavy** — holds an entangled task fully in one window because it cannot be
  checkpointed piecewise (restructure across chapters, refactor a shared
  module). It accumulates, grinds, folds once, and is discarded. Heavy means
  *holds more, lives shorter*. It is chosen when the work is too tangled to
  split — the opposite of fanning out.

**Runtime state — what happens to the agent while it runs:**
- **running / done / folded** — the normal path. After a fold, done work can be
  re-tasked rather than discarded.
- **queued** — ready but not running, because a governor caps how many agents
  (especially heavy) run at once. Waits for a slot. No user action.
- **conflict** — two agents wrote the same thing and the system will not guess
  which is right, so it pauses them and escalates one decision to the user.
  Conducted mode only, and only when ownership was not clean.

From the user's chair, the words are guidance, not controls: light/heavy tell
you whether to wait a moment or step away; queued resolves itself; conflict is
the one moment that needs a decision. The user only ever does three things to a
tile: fold a finished one, re-task it, and occasionally settle a conflict.

Promotion: a light agent that discovers its task is entangled can go heavy (a
bounded deep dive) or spawn its own workers. The governor throttles the result.

---

## 6. Metering (the "Red" requirement)

Context % and cost are shown for every agent, everywhere: a session/fleet total
in the top bar, a readout per Focus, the Center's own meter, each agent tile,
and each chat. In **direct** mode the top bar is a session meter; in
**conducted** mode it becomes a fleet meter with the agent count and the
governor's heavy cap. Consistent metering is what makes cost legible before it
hurts.

---

## 7. Conducted mode (the coordination layer)

Several agents against one brief is the single-user loop with N turned up; the
structure does not change. A Focus is either **direct** (one agent) or
**conducted** (many under a conductor).

- **Thin conductor.** In conducted mode the Center is a conductor agent. It
  holds the plan and pointers, decomposes the brief's open threads into a
  claimable task board, dispatches workers, keeps the brief coherent as deltas
  fold in, and stays thin (self-reducing) so it never burns out. It reads the
  panel's config widgets (strategy, governor) as parameters.
- **Coordinate through state, not conversation.** Workers never narrate to each
  other. They read and write the brief, the workspace, and `.red/log`. A worker
  re-reads current state before committing, so it sees prior folds.
- **Ownership / partitioning.** Each worker owns a directory / module / git
  worktree branch, so writes do not collide. Here — and only here — real git is
  load-bearing: worktree isolation plus 3-way file merge, driven by agents
  through the executor, operating on the project's *own* repo. The small
  interface between fronts (e.g. an identity API contract) is what keeps them
  weakly coupled. Partition quality is the whole ballgame: clean ownership makes
  conflicts rare; each leak costs a human interrupt.
- **Governor.** A cap on concurrent (especially heavy) agents; the rest queue.
- **Escalation.** When ownership leaks and an auto-merge is unsafe, the
  conductor escalates one decision to the user. That is the user's role in
  conducted mode — adjudicator, not operator.

Empirical note to respect: the fan-out (many disposable workers) approach is
resilient but shallow; the accumulate-in-one-context (heavy) approach
deliberates harder but is less stable. Choose per task.

Threshold: conducted mode pays only when the work has multiple sustained,
weakly-coupled fronts. Below that, one dispatched agent wins and the conductor
is pure overhead. Conducted mode is designed-for now and built later; the
substrate (agents as branches, ownership as directory partitions) must not
preclude it.

---

## 8. Data model (sketch)

Workspace, per Focus directory:
```
shared/                 # assets parked up the tree
  panel-default/        # default center-panel html/js/css, copied on create
<focus>/
  brief.md              # curated content (the reduced state) — agent writes, user may edit
  panel/                # this Focus's copy of the template (customisable, deferred)
  .red/
    log                 # append-only event log (folds, dispatches, decisions)
    versions/           # content-addressed brief snapshots (light store, not git)
  <owned subtrees>/     # e.g. fe2o3/ [w1], www/ [w3]
```

Event (log) record:
```
{ id, ts, kind: "dispatch"|"fold"|"edit"|"decision"|"prune",
  agent, task, parent_brief_version, brief_version,
  delta_ref, note }
```

Agent record:
```
{ id, focus, task, policy: "light"|"heavy",
  state: "queued"|"running"|"conflict"|"done"|"folded",
  owns: [path...], branch, context_pct, cost, live_line }
```

Task (board) record:
```
{ id, text, owner_agent|null, status: "unclaimed"|"claimed"|"running"|"blocked"|"folded",
  blocked_on: [task_id...] }
```

Brief content (`brief.md`) is structured but plain: goal line, status/board,
decisions log, open threads. The panel template renders it; the agent edits only
the content.

---

## 9. Interactions (acceptance list)

- `+` next to Foci creates a Focus (its directory, `brief.md`, and a `panel/`
  copy) and selects it.
- `+` next to Chats creates an empty chat at the top of the list and opens it in
  the Center.
- Clicking a Focus selects it; the Center loads its brief, the Agents rail and
  Workspace swap to it.
- The Center **Chat** button creates a chat titled from the Focus, inserts it at
  the top of the Chats list with the brief loaded as context, and opens it.
- Selecting a chat opens the chat view in the Center; the chat is deletable only
  from that view's header.
- The brief line resolves an instruction to an action or to one or more errors
  (shown as an indicator), never to a chat message.
- A finished agent's **Fold in** drops its delta into the brief; the tile then
  accepts a re-task.
- Steering text steers the brief; slash commands dispatch agents.
- Conducted mode: the governor queues agents past the cap; a conflict raises a
  banner in the Center; resolving it merges the pair and unblocks the dependent
  task.

---

## 10. Build notes

Bias: lean, terminal-first, Rust-shaped.
- The **event log is the source of truth**; the brief is a projection. Build the
  reducer and the log first; the UI is a lens over them.
- **Two stores, two mechanisms.** Red's memory (the log + brief versions) is a
  **light fe2o3-native store** (`jdat` records + `hash` content-addressing under
  `.red/`), *not* git. The **fold** is always a reducer *agent* (semantic
  re-reduction), never a git merge. Real **git** appears only in conducted mode,
  for worker worktree isolation and file merge on the project's own repo. This
  is how Goose uses git — worktrees for isolation, not as a memory engine; Red
  matches it there and goes further only at the memory layer.
- The generic log+reducer core is a candidate for extraction into an existing
  fe2o3 crate (e.g. `fe2o3_data`); build app-local first, lift once its shape
  settles. No new crate without express permission.
- Per-Focus runners defined by a **profile**: what an agent may touch, how it
  builds, how it tests, what tools it has. Cheap Thinking → Typst dir, compile
  script → PDF, PDF read; Oxegen → `fe2o3` + `www`, cargo build, `cargo test` +
  headless-browser pass. Every profile has agentic web search. Adding a Focus is
  declaring a profile.
- Prior art worth studying rather than reinventing: spec-driven development
  (Kiro, GitHub Spec Kit, OpenSpec) for the durable-artifact mechanics and the
  "kept vs. discarded plan" distinction; git-worktree multi-agent runners; Goose
  subagents and worktree/handoff patterns for the fan-out layer; server-side
  compaction as a primitive the brief can drive.

---

## 11. Open decisions

1. **Reducer rules.** The precise fold logic that keeps the brief brief: what
   counts as superseded, what always survives (goal, open decisions, live
   threads), what drops to the log, budget thresholds. This is the deepest risk
   — start conservative (promote status changes and resolved-thread removals;
   leave the rest in the log) and grow the aggression as trust builds.
2. **Dispatch control.** Is an agent started by a button on an open thread, by a
   slash command, or by the brief agent offering to spin one up from typed
   intent — or all three.
3. **Conductor policy.** How the conductor turns open threads into cleanly-owned
   tasks, and the exact rule for auto-merge vs. escalate.
4. **Chat → Focus graduation.** The concrete gesture and what it carries when a
   chat folds into a Focus (new open thread vs. seed of a new brief).
5. **Auto-fold threshold.** The context % at which an agent self-reduces, and
   whether it differs for the brief agent vs. workers.
