# Red — Browser-Only Track

> Proposal and brief for the agent responsible for a browser-only Red.
> Companion to `plan_v1.md` (the authoritative server-hosted plan); this
> document proposes a second execution/storage substrate for the *same* north
> star and the *same* brief/fold architecture (`plan_v1.md` §0, §0.1).  It does
> not supersede `plan_v1.md`; it forks the runtime, not the vision.

---

## 1. Mandate

Build Red so that a user can open a URL, be handed a **workspace that never
leaves their machine**, and drive the full brief/fold loop — chat, agentic file
work, running code — with **no server holding their files and nothing to
install**.  The workspace lives in the browser (OPFS or a real local folder via
the File System Access API); the LLM is called directly with the user's own key;
tool execution runs *inside the browser's wasm sandbox*.

The thesis, established over the design conversation this proposal derives from:

- **Files and inference can be fully browser-local today.**  Storage is
  OPFS/File System Access; inference is BYOK `fetch` to any OpenAI-compatible
  endpoint.  No Red server sits in the middle.
- **Execution splits at one clean line.**  Anything that is pure computation
  compiled to **wasm/WASI** runs client-side — file ops, search, Python, JS,
  git, SQL, even C and Typst.  The single thing that does *not* is **native
  compiled-language builds, chiefly Rust/`cargo`** (the `wasm32` 4 GB address
  ceiling, plus proc-macro/`build.rs` executing native code at compile time,
  make an in-browser `rustc` a heavy opt-in that still can't build a real
  workspace).  That one case escalates to a host executor.
- **This is not a pivot away from `plan_v1.md` — it completes its arc.**  D0
  said "self-hosted in a trusted environment to dissolve sandbox/cost/trust".
  Browser-only is the purer form: *no server at all, the trusted environment is
  the user's own browser*, and all three hard problems evaporate more completely
  than self-hosting managed.  D4 already made the `Executor` pluggable "so
  offloading execution is a config change, not a rewrite" — browser-only is that
  seam taken to its conclusion.

Deliver browser-only as the **default product**; keep host execution as an
**opt-in escalation** behind the trust gate — not a co-equal path.  One codebase
must not serve two masters evenly.

---

## 2. Why browser-only — strategy and the business case

This section records the project owner's reasoning, because it is what decides
the track and it must not be relitigated.

**The decision hinges on one question: is Red a personal tool, or a product
others use?**  If purely personal — the owner compiling Typst and building steel
for themselves — server-native already works, the owner is the trusted operator
on their own box, and browser-only is effort for privacy/distribution the owner
does not personally need.  But Red is intended as a product people run, and the
owner cares about its having a viable model.  That answers it, and for a product
the two substrates are not close.

**Server-native structurally traps Red in "open-source release, no revenue" —
and the owner had already resigned to exactly that.**  It is not a failure of
nerve; it is the shape of the thing.  Hosting a full agentic workspace for
strangers means owning the three hard problems D0 names — arbitrary-code
sandboxing, compute cost, and trust.  You cannot run server-native as a SaaS
without solving all three, which is precisely what Red was designed to avoid.
So server-native collapses to "self-host it yourself", and self-hosted server
software has no clean revenue path.  The dead end was reasoned to correctly.

**Browser-only escapes the corner because the user brings the compute.**  Files,
execution, and inference all run on the user's machine with the user's key.  The
marginal cost of one more user is ~zero — you ship static wasm/JS/HTML from a
CDN.  That is the local-first / BYOK model, and it has proven revenue paths that
require solving **none** of the three hard problems, because the default workload
runs in the user's own browser sandbox, not yours:

| Model | Mechanic | Cost to you | Notes |
|---|---|---|---|
| **Managed inference** (biggest lever) | Default is BYOK (free, costs nothing). Optional "just works" tier: you hold provider keys, meter tokens, charge a markup. The CORS relay you need anyway *becomes* the metered gateway. | You front inference spend, recover it + margin; use prepaid credits/quotas against abuse. | Most non-technical users won't BYOK — this is what they pay for. The Cursor model, minus any sandboxing. |
| **E2E-encrypted sync** (on-brand recurring) | Cross-device sync is the one thing browser-only can't do locally — so sell it. Store encrypted blobs you can't read. | Cheap sync infra; you never see content. | The Obsidian Sync / Ente model; dovetails with the owner's Oxeverse/Ozone privacy instincts. |
| **Hosted remote build — "Red Build"** (niche upsell) | A hosted `Executor::Remote` box for users who need native builds without a local companion. | Real compute; reintroduces sandboxing → price to cover, gate it. | The owner's own niche (compiling/deploying steel) becomes sellable. Smaller market, higher price. |
| **Open core** (base) | Whole app open source (the owner's instinct preserved); monetise only the hosted convenience layer above. | — | What local-first companies actually do; self-hosters get everything free. |

**The clincher: browser-only dominates whether or not you monetise.**  Even with
no price tag it is the better product — privacy, zero-install distribution, the
H1 security win (§9), and the owner's standing preference for web.  And *unlike*
server-native it leaves the revenue door open if the owner ever wants it.  So it
wins on the "just a good product" axis and the "could earn" axis at once.

**The honest costs, recorded so they are not a surprise:**

- The `fe2o3` → `wasm32` port is real work of unknown depth (findings in §12,
  scoped further in Slice 0, §13).  It is partly repaid as `fe2o3` wasm
  hardening — the stated reason
  downstream apps exist (CLAUDE.md).
- Any *monetised* path adds operational burden — billing, abuse, support — that
  OSS-only avoids.  If the owner wants zero operational load, don't monetise:
  browser-only is *still* the better product, just without the tiers above.

---

## 3. What carries over, what relocates, what is new

| Plan_v1 element | In browser-only |
|---|---|
| North star §0 (artifacts≡files, general workspace, rendering core, one thread) | **Unchanged.** All client-side-able; rendering (`render.js`) reused verbatim. |
| Brief/fold architecture §0.1 (Foci, brief, fold, log, conducted mode) | **Carries over**, re-homed on browser storage + wasm execution. See §8. |
| D0 trusted environment | **Strengthened** — the trust boundary is the user's own browser; the wasm sandbox is the isolation. |
| D3 workspace on the host / D4 pluggable `Executor` | **Relocated + extended.** Workspace → OPFS/FSA; add `Executor::Wasm` as the default, `Local`/`Remote` become the escalation. |
| D6 OpenAI-compatible providers, BYOK in config | **Relocated to the client.** Keys wrapped in the browser (§10 D-B4); direct `fetch`; thin relay only where CORS blocks. |
| D7/WS-H Syncthing sync | **Changed** — the browser can't run Syncthing. Sync via FSA over a host-synced folder, or fe2o3-native sync later (§10 D-B7). |
| D14 O3db server auth | **Changed** — no server account. A local passphrase unlocks the wrapped keys; the "account" is the browser profile. |
| H1 isolate the run-user (highest-severity gap) | **Largely dissolved** for the wasm tier — see §9. This is browser-only's biggest single win. |
| H3 hand-rolled HTTP/SSE/JSON scanners in `llm.rs` | **Mostly dissolved** — the browser's `fetch` + `ReadableStream` replace the hand-rolled TLS/chunk/`LineReader` code entirely; only JDAT decoding remains. |
| H4 WAL / crash-safe log, `fsync` | **Hardest to honour in the browser** — OPFS durability is weaker than `fsync`. The real tension; mitigated, not eliminated (§8, §10 D-B6). |
| H5 escape-by-default frontend, DOM XSS, `Branch` enum | **Unchanged and more important** — the frontend is now the whole app; untrusted model/file content → DOM XSS is critical. |
| WS-J MCP host (stdio) | **Partial** — stdio MCP servers can't be spawned from the browser; HTTP/WebSocket MCP servers that are CORS-friendly work. stdio MCP escalates to the companion. |

The engineering shape that makes this a **port, not a rewrite**: `fe2o3_red`
already separates protocol / agent loop / tool registry / workspace / skills
from the transport.  Browser-only **compiles that core to `wasm32`** and swaps
**three edge implementations** — the executor, the filesystem, and the LLM
transport — behind their existing seams.  The Steel WebSocket `handler.rs`
round-trip largely disappears: the UI calls the wasm agent in-process.

---

## 4. The three planes

**Storage (client-local).**  A single `WorkspaceFs` trait with two backends:
File System Access API (a *real* local folder, Chromium) and OPFS (a virtual,
origin-private FS, all browsers).  Mirrors the existing `fs_*` command surface so
the file browser UI is unchanged.  Import/export for OPFS; live folder for FSA.

**Inference (client-direct, BYOK).**  `fetch` to any OpenAI-compatible endpoint
(D6 preserved) with the user's key, streamed via `ReadableStream`.  Where a
provider blocks browser CORS, a **stateless relay** — a tiny optional companion
that forwards and stores nothing — bridges it (§10 D-B3).  This *replaces* the
hand-rolled TLS/HTTP/SSE client in `llm.rs`, closing the H3 correctness/DoS
surface for the browser build.

**Execution (in-sandbox by default).**  `Executor::Wasm` runs tools against the
wasm runtime and the OPFS-backed FS.  The trust-gated `Executor::Local` /
`Executor::Remote` escalation (a localhost companion, or a bigger box) handles
only what wasm cannot — native builds.

---

## 5. The `Executor::Wasm` tier and the escalation model

Extend D4's enum:

```
Executor::Wasm     // default — in-browser WASI runtime, OPFS FS, no host authority
Executor::Local    // opt-in escalation — a localhost companion (Steel/Red in
                   //   executor mode) the browser reaches over WebSocket
Executor::Remote   // opt-in escalation — a bigger box, per D4 (and "Red Build", §2)
```

The default is `Wasm`.  A tool call that needs native execution (a `cargo
build`) does not fail silently — it surfaces a **trust-gated escalation prompt**:
"this needs to run on your machine outside the sandbox — allow for this
workspace?"  Consent selects the tier; the jail still applies to everything else.
This is the mechanism that makes hitting the native-build wall a *bounded,
opt-in* event, never a rewrite: the escalation is additive, and the wasm default
stands untouched.

---

## 6. The wasm toolkit map

What the agent's tool tier can run **client-side** (integration of existing wasm
artefacts, not novel work):

| Capability | wasm artefact |
|---|---|
| File read/write/edit/list | OPFS / FSA — plain JS, no wasm needed |
| Search / grep | `ripgrep`/`fd` → wasm/WASI |
| **Typst compile** | **`typst` → wasm** (the compiler is Rust; this is how typst.app compiles in-browser) |
| Python (incl. numpy/pandas) | **Pyodide** |
| JS execution | QuickJS-wasm, or WebContainers (note: StackBlitz-proprietary — licence) |
| git | `isomorphic-git` (pure JS) or `wasm-git` (libgit2) |
| SQL / analytics | `sqlite-wasm`, `duckdb-wasm`, PGlite (Postgres) |
| C compile | `clang`/`lld` → wasm (binji/wasm-clang, Wasmer) |
| TS / bundlers | `tsc`, `esbuild-wasm`, `swc-wasm` |
| Ruby / PHP / Lua | `ruby.wasm`, `php-wasm`, `wasmoon` |

The unifying substrate is a **WASI shim in the browser**
(`@bjorn3/browser_wasi_shim`, `wasmer-js`): any WASI-compiled tool runs against
the OPFS-backed FS.

**The one wall:** native `rustc`/`cargo` (and heavy native C/C++ builds).  LLVM
*has* been compiled to wasm, so this is "big-and-unbuilt + a memory ceiling",
not impossible — but it does not build a real workspace in-browser and must
escalate (§5).  Do not spend effort trying to make fe2o3-scale Rust builds run
client-side; that is explicitly out of scope for the wasm tier.

---

## 7. A real fe2o3-dev day (the owner's own workflow)

The owner's two representative tasks are **compiling Typst chapters** (the
Lucronics/books work) and **building `fe2o3_steel`** (the web-app backends).
They land on opposite sides of the one wall, and the split is the most
persuasive concrete illustration of the whole design.

**"VM memory" clarified — it depends on the storage mode.**  OPFS is a sandboxed
store: on disk, but walled off, so a native `cargo` on the machine cannot reach
into it and you *cannot* "jump out" and compile its contents.  FSA is the
opposite: the browser operates on a **real folder on real disk** (e.g. the actual
`/home/jason/usr/code/rust/fe2o3`), so the agent's edits *are* the real files and
you can drop to a terminal and build them yourself.  **For fe2o3 dev, use FSA on
the real tree** — the sandbox is then around the running *tools*, not around your
*files*.  (FSA is Chromium-only, so use Chromium for dev.)

- **Compiling a Typst chapter → fully in-browser, no escalation.**  `typst` has a
  wasm build; "agent, compile this chapter" runs it client-side and the PDF
  renders inline.  The whole `/polish` → `/improve` → compile → read-the-PDF loop
  is browser-local, any device, nothing installed, nothing leaves the machine.
- **Building `fe2o3_steel` → the native wall, via the companion.**  With FSA on
  the real tree, the agent edits the real `fe2o3_steel/src/...`.  When it needs
  `cargo build`, it dispatches the command to a **small localhost companion**
  (the headless Red/Steel binary in executor mode) that runs the real native
  cargo on those same files and streams stdout/stderr back into the browser,
  rendered inline like any tool result.  Deploy (scp to karri) lives in the
  companion too — it has shell and network.  From the owner's chair this is
  identical to server-native Red; the difference is one small daemon instead of
  the full Steel-plus-wallet-plus-o3db server, and the owner can always
  `cargo build` the same tree in their own terminal.

**Why steel *inherently* belongs on the companion:** it is a native Linux binary
built to deploy to karri.  Native toolchain + native artefact + deploy is a host
job by definition — the browser is the interface, the companion is the compiler.
This was never the browser's job, browser-only or not.

Net for the owner: **Typst goes fully browser-local (a real upgrade); steel
builds run through one small local daemon (lighter than today).**  Nothing the
owner actually needs is lost, and dev is never trapped in unreachable memory
because FSA *is* the real disk.

---

## 8. Brief/fold in the browser

Map `plan_v1.md` §0.1 / D16–D22 onto the client substrate:

- **Brief** (durable reduced state + command surface) → `brief.md` in the
  workspace FS.  The **brief agent** (D17) is a stateless-per-instruction wasm
  agent that reconstructs context from `brief.md` — which matches H5's
  "stateless brief agent" requirement *by construction* in this substrate.
- **Agent** (D18, re-taskable executor) → a wasm agent loop running the
  `RedTool` registry against `Executor::Wasm`.
- **Fold** (D19, fresh reducer agent, `brief + delta + rules`) → a BYOK LLM
  call; advisory/diff-reviewed first per **H2**.  H2's "never lose data, always
  retain the raw delta, serialise folds per Focus" becomes *more* load-bearing
  here because browser storage can be evicted — reinforcing §10 D-B2/D-B6.
- **Log** (D20, per-Focus append-only `jdat` + `hash` under `.red/`) → OPFS,
  written by a **single dedicated Web Worker** using `createSyncAccessHandle`.
  This is where **H4 bites hardest**: OPFS flush semantics are weaker than
  `fsync`, so the "log is the crash-safe commit point" guarantee is softened.
  Honour the *shape* (single writer, append-only, snapshot + rebuildable index,
  readers tolerate a torn trailing record) and treat **export as the real
  backup** (§10 D-B6).
- **Conducted mode** (D21, many agents, git worktrees) → git *file* operations
  work via `isomorphic-git`, but worker worktrees that run native builds hit the
  execution wall and escalate.  Designed-for now, built later — same as
  `plan_v1.md`.  In the browser, conducted mode's parallelism is Web Workers.
- **Panels** (Rail / Center / Agents / Workspace) → unchanged; the live
  front-end already carries the four-panel shell.

---

## 9. Security reframe — the browser-only H1 win

`plan_v1.md`'s hardening section makes the sharpest point in the whole plan:
under an agentic tool, **prompt injection is RCE as the run user**, and H1
(isolate the run-user; the agent currently shells as `jason` next to the wallet)
is the highest-severity gap.

**Browser-only dissolves H1 for the wasm tier.**  A poisoned file or web-search
result that makes the agent run a shell command can only run it *inside the wasm
sandbox against the OPFS workspace* — no ambient host authority, no wallet, no
`STEEL_*` env, no filesystem beyond the granted workspace.  The
"injection → RCE as a host user" chain is broken by construction; the worst case
is "injection → mischief inside a sandbox with the workspace's own data", which
is the workspace attacking itself — squarely inside D0's actual trust model.

Residual requirements:

- **H1 re-applies only to the escalation tier.**  The moment a user allows
  `Executor::Local` for native builds, host-RCE is back on the table for that
  workspace.  So the companion must still run as a dedicated unprivileged user
  with a scrubbed env and no secrets — H1 in full, but now gating a rare opt-in
  rather than the default path.
- **H5 is unchanged and more critical.**  The frontend is the whole app; model
  and file content are untrusted → every `innerHTML` interpolation escapes and
  the markdown renderer's output is sanitised.  DOM XSS is now the primary
  attack surface.
- **H2/H4** as in §8.

Net: browser-only turns the plan's worst security gap into its strongest
security property, at the cost of making frontend XSS (H5) and browser
durability (H4) the two things to get right.

---

## 10. Decisions required before build

Resolve these up front — they are the "could this be too much of a compromise"
points, and each has a recommended default with its tradeoff.

- **D-B1 Filesystem substrate.**  *Default:* **one `WorkspaceFs` layer, two entry
  points.**  FSA and OPFS expose the *same* `FileSystemDirectoryHandle`; only how
  you obtain the root differs — `showDirectoryPicker()` (real folder) versus
  `navigator.storage.getDirectory()` (OPFS) — after which the code is shared
  (~90%).  So this is **not two backends**; it is one handle-based FS with two
  front doors, and the *user* picks which per workspace at creation ("open a
  folder" → FSA, real tree, durable, Chromium; "new private workspace" → OPFS,
  universal, evictable).  *Tradeoff:* FSA real-folder is Chromium-only
  (Firefox/Safari get OPFS).

- **D-B2 Durability.**  *Default:* **FSA-real-folder is the durable path**; OPFS
  mode requires `navigator.storage.persist()` + periodic export + a loud "this
  lives in your browser, export to be safe" warning.  *Tradeoff:* OPFS can still
  be evicted; export is the honest backstop.  This is the single most important
  decision for a product literally called a *workspace* — a workspace you can
  lose by clearing browser data is a serious flaw.

- **D-B3 Provider CORS.**  *Default:* **direct `fetch` first, optional stateless
  relay** (a ~50-line companion that forwards and stores nothing) for providers
  that block browser calls.  *Tradeoff:* the relay dents "pure browser"
  slightly, but holds no data and is opt-in.  Doubles as the metered inference
  gateway (§2).

- **D-B4 Key storage.**  *Default:* **passphrase-wrapped keys in IndexedDB**,
  unlocked by a local passphrase.  No server account (replaces D14 for this
  track).  **Correction (see §12 F1):** *not* via `fe2o3_crypto` — that crate
  bindgen-links a C library and cannot compile to wasm.  Use **WebCrypto
  (`SubtleCrypto` via `web-sys`)** or pure-Rust RustCrypto (AES-GCM + Argon2),
  behind the wasm-clean `iop_crypto` trait.  *Tradeoff:* a passphrase prompt on
  open.  *Rejected:* plaintext `localStorage` keys; `fe2o3_crypto` on wasm.

- **D-B5 Native-build escalation transport.**  *Default:* **an optional
  localhost companion** (Steel/Red in executor mode) the browser reaches over
  WebSocket, exposing `Executor::Local`; `Remote` for a bigger box per D4.
  *Tradeoff:* running native builds means running the companion — but only users
  who want native builds ever start it.

- **D-B6 Log crash-safety under OPFS (H4).**  *Default:* **single-writer Worker +
  `createSyncAccessHandle` + snapshot/export**, accepting weaker-than-`fsync`
  durability and treating export as the real backup.  *Tradeoff:* the "log is
  the crash-safe commit point" guarantee is softened versus the server build.

- **D-B7 Cross-device sync.**  *Default:* **FSA over a host-synced folder**
  (Syncthing/Dropbox/iCloud runs on the OS; the browser just edits the folder),
  with fe2o3-native browser sync as a later dog-food *and* the paid E2E-sync
  tier (§2).  *Tradeoff:* pure-browser cross-device sync isn't solved in v1; the
  power-user answer (FSA over a synced folder) works today and needs no new code.

---

## 11. Engineering thesis and fe2o3 opportunities

**Port the core, swap three edges** — sharpened by the empirical findings in
§12.  Compile `fe2o3_red`'s core (protocol, agent loop, `ToolRegistry`,
`Workspace`, `skills`, `syntax`) to `wasm32`.  Replace: (1) the executor with
`Executor::Wasm`; (2) the filesystem with `WorkspaceFs` (OPFS/FSA); (3) the LLM
transport with browser `fetch` + `ReadableStream`.  Everything else — the
brief/fold logic, the tool schemas, the skill grammar, the rendering — is shared.
The findings sharpen the *ordering*: **make `fe2o3_core` wasm-clean first**
(it's the universal dependency and it isn't), **sever `net`/`o3db_sync`/`crypto`
from the wasm target** (they transitively drag in a C library — §12 F1), and
**swap concrete stores behind the already-wasm-clean `iop_*` traits** rather than
porting the native crates.

**fe2o3 is the exercise ground (CLAUDE.md).**  The port surfaces real library
work, all as **extensions to existing crates — no new crate without express
permission** (D22, `~/usr/CLAUDE.md`):

- **`wasm32` target support** across `fe2o3_core`, `fe2o3_jdat`, `fe2o3_syntax`,
  `fe2o3_hash`, `fe2o3_stds`, `fe2o3_text` (*not* `fe2o3_crypto` — it links C,
  §12 F1) — compiling this subset to wasm will find and fix `std`/time/threading
  assumptions.  A generically valuable hardening pass that a second downstream
  caller benefits from.
- **A browser `fetch`/SSE transport** behind a feature in `fe2o3_net`, replacing
  the hand-rolled `llm.rs` scanners (this is the *correct* way to satisfy H3 —
  the constraint is fine, the hand-rolled execution is the liability).
- **The log + reducer store** (D22, candidate `fe2o3_data`) built app-local
  first against OPFS, lifted once its shape settles.

Keep the error-handling style throughout: `res!`/`ok!`/`catch!`/`err!`, no
`unwrap`/`?`/`unsafe`, enums over `dyn`, British spelling, doc comments.

---

## 12. fe2o3 → wasm: findings and constraints (research, 2026-07-10)

Empirical audit of the actual dependency graph and source (grep of
`fe2o3_red`'s crates for wasm-hostile `std` patterns, native build scripts, and
concurrency).  These findings supersede any looser statement above.

**F1 — A C library reaches red transitively (hard blocker, invisible from red's
manifest).**  `fe2o3_crypto` is not pure Rust: its `build.rs` uses `bindgen` to
FFI-link a **C static archive** (`libfiresaber.a`, the Saber PQ KEM) plus
`libc`.  A native `.a` cannot link into `wasm32`, and you cannot `cfg`-gate a
linked archive away.  red does not depend on `fe2o3_crypto` directly — but both
`fe2o3_net` and `fe2o3_o3db_sync` do, so red drags it in two levels down.  This
is only visible at wasm link-time.  *Consequence:* the wasm build must not
compile `net`, `o3db_sync`, or `crypto` at all; severing `net`/`o3db_sync`
incidentally cuts `crypto`, `ring`, and `libc`.  Also corrects **D-B4**: browser
key-wrapping uses WebCrypto/RustCrypto behind `iop_crypto`, never `fe2o3_crypto`.

**F2 — `fe2o3_core` is not wasm-clean, and it is the universal dependency.**
`Instant::now()` in `time.rs`; the logger **spawns threads** (`log/base.rs`,
`log/console.rs`) and calls `SystemTime::now()`; plus `getrandom`, `fs`, and
`tokio`.  Every crate depends on core, so **core must be made portable first**,
via `cfg`-gating: a wasm clock shim (`performance.now()`/`Date` through
`web-sys`), a single-threaded logger path on wasm (real wasm threads need
`SharedArrayBuffer` + COOP/COEP — avoid), the `getrandom` `js` backend.  The
`Instant`/`SystemTime` calls **compile clean and panic at runtime** on
`wasm32-unknown-unknown` — insidious; they pass CI and fail only when exercised.

**F3 — The `iop_*` trait seam is (mostly) the salvation — with one correction
the compile forced.**  `fe2o3_iop_crypto`, `fe2o3_iop_hash` and `fe2o3_syntax`
build to wasm as-is.  red is **generic over the `iop_db::Database` trait**
(`handler.rs`, `SessionStore<…, DB>`), not the concrete store — so the port
swaps *concrete implementations behind the interop traits* (an OPFS `Database`, a
WebCrypto `iop_crypto`, a `fetch` transport) while the native crates aren't
compiled for wasm.  **Correction from Slice 0:** `fe2o3_iop_db`'s *source* is
clean, but it **directly depends on `fe2o3_crypto`** (→ `pqcrypto-dilithium` → C),
so the `Database` trait crate itself does **not** build to wasm until that
dependency is feature-gated out.  The seam is real, but it must first be
decoupled from the C crypto — a dependency-hygiene fix, not a redesign.

**F4 — o3db is a threaded, mmap'd actor database — not portable, and it stays
native untouched.**  `fe2o3_o3db_sync` is a **bot/actor thread-pool**
architecture (`WriterBot`/`CacheBot`/`ZoneBot`/`SuperBot` on `fe2o3_bot`,
`thread::Builder::spawn`, `crossbeam` `WaitGroup`, zone sharding) over mmap + fs
— ~530 concurrency sites.  mmap alone has no browser equivalent.  Because red
reaches it through the `Database` trait (F3), **native o3db needs zero changes
and loses nothing** — it is severed by the trait boundary, not ported, and keeps
every thread, bot, and mmap.  The browser gets a light OPFS `Database` impl,
which is exactly the D20 design ("a light `jdat`+`hash` store under `.red/`, not
o3db") — so nothing is lost that the plan wanted in the browser.

**F5 — Trait-genericity is necessary but not sufficient; gate the deps out of
the wasm target.**  Cargo compiles every listed dependency regardless of whether
red's code uses it, so an un-gated `o3db_sync`/`net` dependency still breaks the
wasm link through its threads/mmap/C-FFI *even though red only calls the trait*.
Make `o3db_sync` and `net` **target-gated dependencies**
(`[target.'cfg(not(target_arch="wasm32"))'.dependencies]` or a `native`
feature); the wasm build pulls the OPFS/fetch impls instead.

**F6 — Scope wasm-only features by target, or you break the native build.**  The
`getrandom` `js` feature (and any wasm dep) must live under
`[target.'cfg(target_arch="wasm32")'.dependencies]`, **not** a plain `[features]`
entry — Cargo feature-unification would otherwise leak the `js` backend into the
native build and fail it.  The one way this work can regress native fe2o3 is
mis-scoped features; get the scoping right and native is untouched.

**Portability tiers (the audit, at a glance):**

| Tier | Crates | Action |
|---|---|---|
| Compiles to wasm (verified Slice 0) | `core`, `jdat`, `syntax`, `hash`, `stds`, `text`, `iop_crypto`, `iop_hash` | build clean (2 fixes applied — see below) |
| Needs decoupling first | `iop_db` | feature-gate its `fe2o3_crypto` dep out (F3) |
| Runtime-gate (compiles, panics live) | `core` time/thread calls | clock shim, single-thread logger — not caught by compile |
| Severed from wasm (stay fully native) | `net`, `o3db_sync`, `crypto`, `data`?, `steel`, `mail`, `social`, `shield`, `sys`, `tui` | target-gate out; swap impls behind `iop_*` |

**Slice 0 — measured results (2026-07-11, verified, not estimated):**

*Track A — browser (headless Chromium):*
- **CORS (D-B3): all five providers allow direct browser calls** — Fireworks,
  OpenRouter, Together, Groq, DeepInfra each returned a *readable* cross-origin
  response (401 on a dummy key), 39–1877 ms.  **No relay is technically
  required**; it becomes a business choice, not a dependency.
- **OPFS persistence (D-B2): survives a browser restart.**  Data written, profile
  closed and reopened, content read back intact — even though `persist()`
  returned `false` under headless (no engagement heuristic; a real installed
  browser is likelier to grant).  Quota ~10 GB.  Residual risk is eviction under
  storage pressure / manual clear → mitigated by export + FSA-real-folder.
- **D-B1:** the OPFS root *is* a `FileSystemDirectoryHandle` — same interface FSA
  returns; the "one layer, two entry points" claim holds.
- **A4 (does it sing): the full agentic loop ran in-browser, no server** —
  `user → LLM → tool_call → OPFS write → tool result → final answer`, and the
  file verifiably persisted in OPFS.  (LLM leg mocked for a deterministic
  `tool_call`; the real Fireworks leg proven by the CORS probe.)

*Track B — `wasm32-unknown-unknown` compile:*
- The whole red-relevant leaf set — **`core`, `jdat`, `syntax`, `hash`, `stds`,
  `text` — compiles to wasm**, after **two small fixes applied upstream and
  verified against native tests** (48 `text` tests green): `text/base2x.rs`
  `MAX_A` (`2^32` overflows 32-bit `usize`) widened to `u64`; `jdat/binary/core.rs`
  `c64_len` compares in `u64` (48-bit literals).  Both are genuine 32-bit
  portability bugs — fe2o3 didn't build on *any* 32-bit target.
- **F1 confirmed empirically:** `fe2o3_crypto` fails wasm with
  `cc-rs: failed to find tool "clang"` — it compiles C (FireSaber).
- **F3 corrected:** `iop_db` fails wasm via a direct `fe2o3_crypto` dependency
  (`pqcrypto-dilithium` → C).  Must be feature-gated.
- **F2 nuance measured:** `core` *compiles*; its `Instant`/`thread`/`SystemTime`
  hazards are **runtime**, invisible to the compiler — confirming they need
  runtime gating, not a build fix.

**Verdict:** the two sink-the-track unknowns came back green (CORS everywhere;
OPFS survives restart), the browser loop works end-to-end, and the wasm port is
tractable — the leaf set already compiles.  Remaining port work is bounded and
known: feature-gate `fe2o3_crypto` out of `iop_db`/`data`, runtime-gate `core`'s
time/thread, then build the OPFS `Database` impl + `fetch` transport + async
rework.  No new showstoppers surfaced.

**Native impact (what the rest of fe2o3 loses): nothing functional.**
`cfg`-gating leaves every native path unchanged; the `iop` swaps *add* backends
rather than removing them; the C PQ crypto, threaded logger, tokio, and mmap
o3db all stay.  The real cost is two standing taxes, **contained to the
foundational crates**: a doubled (native + wasm) build/test matrix, and a rule
that `core`/`jdat`/`syntax`/`hash`/`stds`/`text` *stay* `std`-service-free unless
`cfg`-gated.  The heavy native machinery (`net`, `o3db_sync`, `crypto`, the
servers) is entirely outside the blast radius.

---

## 13. Staged plan

Wave-gated; snapshot at each gate; the server build stays intact behind you so
nothing is bet before it is earned.

- **Slice 0 — throwaway de-risk probes (buy information, not code).**  Disposable
  spikes that retire the sink-the-track unknowns before any porting; the server
  path stays untouched, and nothing here becomes production code.  Two
  independent tracks:
  - *Browser-capability spike (vanilla JS, no Rust):* OPFS + FSA behind one
    handle interface (D-B1); a persistence torture test — `persist()`, then try
    to evict; FSA handle survival across reload (D-B2); a real BYOK `fetch` to
    each provider to see who CORS-blocks, relay where needed (D-B3); one full
    agent turn end-to-end (user → LLM → `tool_call` → OPFS/Pyodide → result →
    answer), no server.
  - *fe2o3 wasm-compile spike (Rust, compilation only):* `cargo build --target
    wasm32-unknown-unknown` against `jdat`+`syntax`, then add `core` and
    catalogue exactly what breaks — turning F2's depth from estimate to
    measurement.
  *Gate:* does the agent loop sing, and are D-B1/D-B2/D-B3 and the `core`-cleanup
  size answered?  Front-load the two "too much of a compromise" probes —
  eviction (D-B2) and CORS (D-B3) — so a bad answer surfaces before anything is
  bet.  If it doesn't hold up, stop; the probes are disposable and the server
  path is intact.

- **Stage 1 — core to wasm.**  Compile `fe2o3_red` + fe2o3 deps to `wasm32`;
  `Executor::Wasm`; `WorkspaceFs`; LLM via `fetch`.  Chat + files + Python +
  Typst, client-side, no server.

- **Stage 2 — the wasm tool tier.**  `isomorphic-git`, `ripgrep-wasm`,
  `sqlite`/`duckdb`-wasm, QuickJS; the `RedTool` registry over `Executor::Wasm`;
  skills grammar (already in `fe2o3_syntax`); reuse `render.js`.

- **Stage 3 — brief/fold client-side.**  `brief.md` + brief agent (stateless per
  instruction), re-taskable agents, fold reducer (advisory/diff per H2), the
  `.red/` log via the single-writer Worker (H4-shaped per D-B6).  Rail / Center /
  Agents / Workspace.

- **Stage 4 — escalation + sync.**  Optional localhost companion →
  `Executor::Local` for native Rust builds behind the trust-gated prompt; the
  CORS relay; cross-device via FSA-over-synced-folder (D-B7).

- **Stage 5 — hardening + polish.**  H5 escape/sanitise pass; H2 fold-advisory
  guarantees; H4 WAL-in-Worker; key wrapping (D-B4); PWA/offline; mobile parity
  (D10) as an acceptance gate throughout, not a bolt-on.

- **Stage 6 — the business layer (only if monetising, §2).**  Metered inference
  gateway on top of the relay; E2E-encrypted sync service; optional hosted
  `Executor::Remote` ("Red Build").  Open-core base; self-hosters get all of the
  above free.  None of this gates the free product.

---

## 14. Acceptance criteria and non-goals

**Acceptance.**  A first-time user opens the URL, grants a folder (or accepts
OPFS), sets a provider key behind a passphrase, and runs a full brief/fold
Focus — chat, edit files, compile a Typst chapter, run a Python/JS/SQL/git
tool, fold a delta — with **no server involved and the workspace never leaving
the machine**.  Native Rust builds are reachable *only* via an explicit,
trust-gated escalation.  Mobile parity holds.  The frontend passes an XSS review
(H5).

**Non-goals (this track).**  In-browser fe2o3-scale Rust builds; pure-browser
cross-device sync in v1; multi-tenant isolation (D0 unchanged — it's your own
browser); spawning stdio MCP servers from the browser (escalates to the
companion).

---

## 15. Open questions

- **Does the server build survive as the escalation companion, or diverge?**
  Ideally the same `fe2o3_red` binary runs headless as `Executor::Local`, so
  browser-only and server-hosted are two front-ends over one core — "one core,
  two faces".  Confirm the core factors cleanly enough for this.
- **fe2o3 `wasm32` blast radius.**  How much of the dependency graph assumes
  `std`/threads/time?  Scope the port in Slice 0 before committing to Stage 1.
- **Which providers allow browser CORS** (Fireworks in particular) — determines
  how often the relay is needed, and how much the metered-inference tier leans on
  it.  Answer empirically in Slice 0.
- **OPFS durability in practice** — how aggressively do current browsers evict
  `persist()`-ed origins?  Decides how loudly D-B2's export warning must shout.
- **Business commitment.**  The tiers in §2 are latent, not required.  Confirm
  whether v1 ships pure open-core (zero operational burden) or wires the metered
  gateway from the start.
