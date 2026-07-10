# Red — project outline

A unified interface for local agentic work and chat. One surface for long-horizon projects (a Typst book, a Rust codebase) and for open-ended thinking, built so that the context-window limit stops governing the workflow.

This document is the executor brief. It states the model, the interface, the data, the agent lifecycle, and the teamwork layer in enough detail to build from, and ends with the decisions still open. The accompanying `index.html` + `assets/` is a clickable reference mockup of the target layout and interactions.

---

## 1. The reframe

The failure mode of a normal chat is that one session is both the memory and the execution. It accumulates because it is the memory, it burns because it is the execution, and past ~60–70% of the window the execution poisons the memory (context rot). The manual fix people use — ask for a handover doc, restart, re-read it — is lossy compaction done by hand under duress.

Red separates the two up front and makes that separation the whole loop:

- The **brief** is durable, curated, small. It is the reduced state of a project. You live here.
- A **run/agent** is disposable execution. It forks from the brief, does one bounded job in its own context, produces a delta, folds the delta back, and dies.
- **Folding** is re-reduction: the brief is recomputed to absorb the delta and drop what is now superseded. Folding replaces the handover — the brief is always the current handover, by construction.

Restarting a session stops being an event. The brief was the memory all along; the transcript was disposable.

---

## 2. The substrate (why this is a DAG)

Store a project as an append-only event log plus a reducible state. Then:

- the brief is the reducer output (current `HEAD`),
- a run is a branch that forks from a brief version,
- a fold is a merge that appends to the log and bumps the brief to its next version,
- a chat is an ephemeral branch: folding it is a merge, disposing it is pruning a dangling branch.

The visible interface is a projection over this substrate. Two projections are offered: a linear chat view (a chat), and a state-document view (a brief). Same underlying graph, different lens.

The everyday flow uses the simple slice — trunk plus branches that merge straight back. The full DAG is latent and gives, for free: runs that depend on a prior run's fold (a chain), speculative sibling branches off one version ("try it two ways, keep the winner"), a heavy run that spawns its own children (a subtree), and replay (reopen any past brief version, change one instruction, re-run forward).

---

## 3. Layout — four panels

```
┌──────────────────────────────────────────────────────────────────────┐
│ Red · session/fleet meter (context % + cost, always visible)           │
├────────────┬───────────────────────────────┬───────────────┬──────────┤
│ RAIL       │ CENTER PANEL                  │ AGENTS        │ WORKSPACE │
│ Projects + │ the brief (or a chat)         │ metered tiles │ file tree │
│ Chats +    │ AI-curated, html/js/css       │ one per run   │ = source  │
│            │ steering box at the bottom    │               │ of truth  │
└────────────┴───────────────────────────────┴───────────────┴──────────┘
```

### Rail (left)
- Two headers, `Projects` and `Chats`, each with a `+` button.
- Projects are persistent; each shows its own context % and cost. Clicking a project selects it and loads its brief into the center. Clicking a project name renames it.
- Chats are ephemeral and selectable. The list is meant to stay short: a chat is either folded into a project or disposed. A chat is deletable only from the center panel (see below), not from the rail.

### Center panel (the brief)
- The panel is the project's brief. In normal use the AI curates it; the user may edit it directly.
- The panel is itself `html/js/css` living in the project's workspace directory, copied at project creation from a default template parked higher in the tree, so each project can diverge. The default template provides an editable panel title (independent of the rail names) and can host widgets (pulldowns, toggles) whose settings the AI reads back as parameters.
- Header carries: the editable title, a **Chat** button, and this panel's own context/cost meter (a thin conductor stays low here).
- The **Chat** button starts a new chat, inserts it at the top of the Chats list, and loads the current brief as that chat's context. Use it to explore an idea against a project without polluting the brief.
- A steering input sits at the bottom. Plain text steers; slash commands dispatch (e.g. `/polish ch 3`, `/plan the www migration`).
- Center has two view states: the **brief view** (default per project) and a **chat view** (when a chat is selected). The chat view is a linear thread; its header shows which brief is loaded as context and carries the **Delete** control.

### Agents rail
- One tile per run, showing name, kind badge (light / heavy / research), an optional branch/worktree, a context bar, context %, and cost.
- A finished run shows its result and a **Fold in** button; folding drops the delta into the brief and removes the tile.
- Metering here is load-bearing, not decoration: it is how a bloating brief or a runaway heavy agent becomes visible before it costs anything.

### Workspace
- The file tree is the source of truth. The brief lives here as a content file (`brief.md`); `panel/` holds the per-project panel; shared assets (`app.css`, `brief.js`) sit higher in the tree. `.red/log` is the append-only event log.
- In team mode, directories carry an owner badge (which agent owns which subtree) and contested files carry a warning.
- User and AI both read and write here; each user's workspace is private.

---

## 4. Keeping the brief brief

If the brief bloats, context rot has simply moved up one layer. The discipline that prevents it:

- **Fold is re-reduction, not append.** On fold, the AI recomputes state and drops what the delta supersedes; it does not just add a line.
- **Content / presentation split.** The AI curates a structured content file (`brief.md` — plain, diffable). A fixed template (`panel/`, using shared css/js) renders it. The AI never rewrites the HTML shell, so a bad fold is a bad line, not a broken page, and history is diffs of readable content.
- **Detail lives in the workspace.** The brief points at files rather than containing them, so it stays short.
- **Visible budget.** The brief has a soft cap; exceeding it signals re-reduction or archival. Superseded material is not deleted — it drops to `.red/log`, out of view but queryable on replay.
- **Veto on drop.** The fold step surfaces what it is about to drop and lets the user veto — brevity as a steerable operation.

---

## 5. Agents — lifecycle and vocabulary

An agent is created when a piece of work should leave the brief and run in its own session, so the noise (file edits, compiles, searches) never clogs the brief. It forks from the current brief state, does its bounded job, folds its delta back, and dies. Agents are cheap and disposable; the brief persists.

Two independent axes describe an agent:

**Context policy — chosen at creation, a property of the task:**
- **light** — does its bit, externalizes to the brief, stays thin. In and out. Most agents.
- **heavy** — holds an entangled task fully in one window because it cannot be checkpointed piecewise (restructure across chapters, refactor a shared module). It accumulates, grinds, folds once, and is discarded. Heavy means *holds more, lives shorter*. It is chosen when the work is too tangled to split — the opposite of fanning out.

**Runtime state — what happens to the agent while it runs:**
- **running / done / folded** — the normal path.
- **queued** — ready but not running, because a governor caps how many agents (especially heavy) run at once. Waits for a slot. No user action.
- **conflict** — two agents wrote the same thing and the system will not guess which is right, so it pauses them and escalates one decision to the user. Team mode only, and only when ownership was not clean.

From the user's chair, the words are guidance, not controls: light/heavy tell you whether to wait a moment or step away; queued resolves itself; conflict is the one moment that needs a decision. The user only ever does two things to a tile: fold a finished one, and occasionally settle a conflict.

Promotion: a light agent that discovers its task is entangled can go heavy (a bounded deep dive) or spawn its own workers. The governor throttles the result.

---

## 6. Metering (the "Red" requirement)

Context % and cost are shown for every agent, everywhere: a session/fleet total in the top bar, a readout per project, the center panel's own meter, each agent tile, and each chat. A heavy-agent governor count sits in the top bar in team mode. Consistent metering is what makes cost legible before it hurts.

---

## 7. Teamwork layer

Several agents against one brief is the single-user loop with N turned up; the structure does not change.

- **Thin conductor.** In team mode the center panel is a conductor agent. It holds the plan and pointers, decomposes the brief's open threads into a claimable task board, dispatches workers, keeps the brief coherent as deltas fold in, and stays thin so it never burns out. It reads the panel's config widgets (strategy, governor) as parameters.
- **Coordinate through state, not conversation.** Workers never narrate to each other. They read and write the brief, the workspace, and `.red/log`. A worker re-reads current state before committing, so it sees prior folds.
- **Ownership / partitioning.** Each worker owns a directory / module / worktree branch, so writes do not collide. The small interface between fronts (e.g. an identity API contract) is what keeps the fronts weakly coupled. Partition quality is the whole ballgame: clean ownership makes conflicts rare; each leak costs a human interrupt.
- **Governor.** A cap on concurrent (especially heavy) agents; the rest queue.
- **Escalation.** When ownership leaks and an auto-merge is unsafe, the conductor escalates one decision to the user. That is the user's role in team mode — adjudicator, not operator.

Empirical note to respect: the fan-out (many disposable subagents) approach is resilient but shallow; the accumulate-in-one-context (heavy) approach deliberates harder but is less stable. Choose per task.

Threshold: teamwork pays only when the work has multiple sustained, weakly-coupled fronts. Below that, one dispatched run wins and the conductor is pure overhead.

---

## 8. Data model (sketch)

Workspace, per project directory:
```
shared/                 # assets parked up the tree
  panel-default/        # default center-panel html/js/css, copied on project create
<project>/
  brief.md              # curated content (the reduced state) — AI writes, user may edit
  panel/                # this project's copy of the template (customisable)
  .red/
    log                 # append-only event log (folds, dispatches, decisions)
  <owned subtrees>/     # e.g. fe2o3/ [w1], www/ [w3]
```

Event (log) record:
```
{ id, ts, kind: "dispatch"|"fold"|"decision"|"prune",
  agent, task, parent_brief_version, brief_version,
  delta_ref, note }
```

Agent record:
```
{ id, project, task, policy: "light"|"heavy",
  state: "queued"|"running"|"conflict"|"done"|"folded",
  owns: [path...], branch, context_pct, cost, live_line }
```

Task (board) record:
```
{ id, text, owner_agent|null, status: "unclaimed"|"claimed"|"running"|"blocked"|"folded",
  blocked_on: [task_id...] }
```

Brief content (`brief.md`) is structured but plain: goal line, status/board, decisions log, open threads. The panel template renders it; the AI edits only the content.

---

## 9. Interactions (acceptance list)

- `+` next to Projects creates a project (its directory, `brief.md`, and a `panel/` copy) and selects it.
- `+` next to Chats creates an empty chat at the top of the list and opens it in the center.
- Clicking a project selects it; the center loads its brief, the agents rail and workspace swap to it.
- The center **Chat** button creates a chat titled from the project, inserts it at the top of the Chats list with the brief loaded as context, and opens it.
- Selecting a chat opens the chat view in the center; the chat is deletable only from that view's header.
- A finished agent's **Fold in** drops its delta into the brief and removes the tile.
- Steering text steers the brief; slash commands dispatch agents.
- Team mode: the governor queues agents past the cap; a conflict raises a banner in the center; resolving it merges the pair and unblocks the dependent task.

---

## 10. Build notes

Bias: lean, terminal-first, Rust-shaped.
- The **event log is the source of truth**; the brief is a projection. Build the reducer and the log first; the UI is a lens over them.
- **Git as the coordination substrate**: worker per branch / worktree, conductor merges; a diff is a worker's delta.
- Per-project runners defined by a **profile**: what a run may touch, how it builds, how it tests, what tools it has. Cheap Thinking → Typst dir, compile script → PDF, PDF read; Oxegen → `fe2o3` + `www`, cargo build, `cargo test` + headless-browser pass. Every profile has agentic web search (e.g. Exa). Adding a project is declaring a profile.
- Prior art worth studying rather than reinventing: spec-driven development (Kiro, GitHub Spec Kit, OpenSpec) for the durable-artifact mechanics and the "kept vs. discarded plan" distinction; git-worktree multi-agent runners; Goose subagents and worktree/handoff patterns for the fan-out layer; server-side compaction as a primitive the brief can drive.

---

## 11. Open decisions

1. **Brief authorship.** AI-maintained with user correction (default), vs. user-authored structure the AI fills. Governs how aggressive the reducer may be.
2. **Dispatch control.** Is a run started by a button on an open thread, by a slash command, or by the AI offering to spin one up from typed intent — or all three.
3. **Reducer rules.** The precise fold logic that keeps the brief brief: what counts as superseded, what always survives, what drops to the log, budget thresholds.
4. **Conductor policy.** How the conductor turns open threads into cleanly-owned tasks, and the exact rule for auto-merge vs. escalate.
5. **Chat → project graduation.** The concrete gesture and what it carries when a chat folds into a project (new open thread vs. seed of a new brief).
