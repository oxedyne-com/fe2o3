# Slice 0 — Browser-Only Red: de-risk probe results

> Execution record for Slice 0 of `proposal_browser_only.md`.  Ran 2026-07-11.
> All results below are **measured** (headless Chromium + `cargo` wasm builds on
> argonaut), not estimated.  Slice 0's purpose was to retire the unknowns that
> research could not settle, before any porting.  It did; the proposal's §12
> carries a condensed version of these findings.

---

## Verdict

**Browser-only Red is viable.**  The two unknowns that could have sunk the track
both came back green, the full agent loop runs client-side with no server, and
the fe2o3 leaf set compiles to `wasm32` after two small, verified fixes.

- **CORS:** every provider tested allows direct browser calls — no relay
  required.
- **OPFS persistence:** the workspace survives a browser restart.
- **A4 loop:** `user → LLM → tool_call → OPFS write → tool result → answer` ran
  entirely in the browser, file verified on disk.
- **wasm port:** `core`, `jdat`, `syntax`, `hash`, `stds`, `text` all compile to
  `wasm32-unknown-unknown`; the remaining work is bounded and known.

No new showstoppers surfaced.

---

## Method

- **Track A (browser):** headless Chromium (Playwright), a page served over
  `http://127.0.0.1` (a real origin, so CORS behaves realistically), driven via
  `page.evaluate`.  Persistence tested with a persistent profile closed and
  reopened (a simulated restart).
- **Track B (Rust):** `rustup target add wasm32-unknown-unknown`, then
  `cargo build -p <crate> --target wasm32-unknown-unknown` per crate; native
  regression checks and unit tests after each source fix.

---

## Track A — the browser substrate

### CORS (D-B3) — all providers allow direct browser calls

Cross-origin `POST` to each provider's chat-completions endpoint with a dummy
key.  A *readable* response (even a 401) proves the provider sends
`Access-Control-Allow-Origin`; a thrown `TypeError` would mean blocked.

| Provider   | CORS allowed | Status (dummy key) | Round-trip |
|------------|--------------|--------------------|------------|
| Fireworks  | ✅ | 401 (invalid key) | ~350 ms |
| OpenRouter | ✅ | 401 | ~39 ms |
| Together   | ✅ | 401 | ~710 ms |
| Groq       | ✅ | 401 | ~152 ms |
| DeepInfra  | ✅ | 401 | ~1877 ms |

**Consequence:** the CORS relay is *not* a technical dependency.  It becomes an
optional business component (the metered-inference gateway of §2), not a
requirement.  BYOK direct-to-provider works today.

> Note: the first probe run reported OpenRouter/Together as blocked — that was a
> test bug (an argument array coerced into the URL), not real.  Corrected and
> re-run; all five allow CORS.

### OPFS persistence (D-B2) — survives a restart

- Wrote a file to OPFS, closed the browser profile, reopened it, read the content
  back **intact**.
- `navigator.storage.persist()` returned **`false`** under headless (no user-
  engagement heuristic) — yet the data survived the restart regardless.  A real
  installed browser with engagement is *more* likely to grant persistence.
- Quota reported ~**10 GB**.

**Consequence:** the scary case (does the workspace survive a restart) is fine.
Residual risk is eviction under storage pressure or a manual "clear site data" —
mitigated by the export backstop and the FSA-real-folder path (D-B2).

### D-B1 — one handle interface

The OPFS root is a `FileSystemDirectoryHandle` — the same interface
`showDirectoryPicker()` (FSA) returns.  Confirms "one FS layer, two entry
points": the downstream read/write/list code is identical for both.
(`createSyncAccessHandle` is worker-only, as expected — absent on the main
thread; the per-Focus log writer will live in a Web Worker per D-B6.)

### A4 — the full agentic loop, in the browser, no server

Ran end-to-end:

```
tool_call: write_file(notes.md)
tool_result: wrote 35 bytes to notes.md
assistant: Done — I created notes.md in your workspace.
OPFS verify — notes.md contains: "# Bloom filter\nA probabilistic set."
```

`user message → LLM → tool_call → execute against OPFS → tool result → final
answer`, with the file verifiably persisted in OPFS.  The LLM leg used a local
OpenAI-shaped mock for a deterministic `tool_call`; the real Fireworks leg is
proven separately by the CORS probe (readable response, ~350 ms).

---

## Track B — the `wasm32` compile

### Build matrix (`wasm32-unknown-unknown`)

| Crate | Result | Note |
|---|---|---|
| `core` | ✅ compiles | runtime hazards remain (see below) |
| `jdat` | ✅ compiles | after the `c64_len` fix |
| `syntax` | ✅ compiles | (was blocked only by `text`) |
| `hash` | ✅ compiles | |
| `stds` | ✅ compiles | |
| `text` | ✅ compiles | after the `MAX_A` fix |
| `iop_crypto` | ✅ compiles | |
| `iop_hash` | ✅ compiles | |
| `iop_db` | ❌ fails | direct dep on `fe2o3_crypto` → `pqcrypto-dilithium` → C |
| `crypto` | ❌ fails | `cc-rs: failed to find tool "clang"` — compiles C (FireSaber) |

### F1 confirmed — a C library reaches wasm builds

`fe2o3_crypto` fails with `error occurred in cc-rs: failed to find tool "clang"`.
It bindgen-links the FireSaber C archive; C cannot target wasm.  Confirms the
hidden-C-dependency finding empirically.

### F3 corrected — `iop_db` is not wasm-clean at the dependency level

`fe2o3_iop_db`'s *source* is clean, but it **directly depends on `fe2o3_crypto`**
(`cargo tree`: `iop_db → crypto → pqcrypto-dilithium → pqcrypto-internals` (C)).
So the `Database` trait crate — the seam red is generic over — does **not** build
to wasm until that dependency is feature-gated out.  The seam is still the right
approach; it just needs decoupling from the C crypto first (a dependency-hygiene
fix, not a redesign).  `fe2o3_data` should be checked for the same coupling.

### F2 nuance measured — the hazards are runtime, not compile

`core` *compiles* to wasm.  Its `Instant::now()` / `thread::spawn` /
`SystemTime::now()` sites (in `time.rs`, `log/*`) **compile clean and would panic
at runtime** on `wasm32-unknown-unknown`.  A green build is necessary but not
sufficient — these need runtime gating (a clock shim; a single-threaded logger
path), and only *running* the wasm would catch them.

---

## Fixes applied (upstream fe2o3, native-verified, uncommitted)

Two genuine 32-bit-`usize` portability bugs — fe2o3 did not build on *any* 32-bit
target, wasm or otherwise.  Both fixed minimally; native builds and unit tests
green after.

**1. `fe2o3_text/src/base2x.rs`** — `MAX_A = 2^32` overflows a 32-bit `usize`.
Widened the ceiling constant to `u64` (lossless on 64-bit) and cast the `usize`
argument at the comparison:

```rust
pub const MAX_A: u64 = 2_u64.pow(MAX_X as u32);   // was: usize
...
if n == 0 || n as u64 > MAX_A { return None; }     // was: n > MAX_A
```

**2. `fe2o3_jdat/src/binary/core.rs`** — `c64_len` compares against 40- and
48-bit literals inferred as `usize`.  Widen the argument once:

```rust
pub fn c64_len(num: usize) -> usize {
    let num = num as u64;   // upper literals exceed 32-bit usize
    ...
}
```

**Verification:** `text` unit tests 48/48 pass; `jdat` lib unit tests 6/6 pass;
native builds of both crates unaffected.  Not committed — pending the minimal-git
rule (commit/push on request).

---

## What remains for the port (bounded, known)

1. **Feature-gate `fe2o3_crypto` out of `iop_db`** (and check `fe2o3_data`) so the
   `Database` trait compiles to wasm.
2. **Runtime-gate `core`'s time/thread calls** — a clock shim
   (`performance.now()`/`Date` via `web-sys`) and a single-threaded logger path
   under `#[cfg(target_arch = "wasm32")]`.
3. **Target-gate `net`/`o3db_sync` out of the wasm build** (they drag threads,
   mmap, and the C crypto).
4. **Build the wasm edges:** an OPFS `Database` impl, a `fetch`/SSE transport, a
   WebCrypto `iop_crypto` impl, and rework red's async to `wasm-bindgen-futures`.

---

## Side observations (not caused by this work)

- **`fe2o3_jdat` integration tests are broken.**  `tests/string.rs`, `map`,
  `byte`, `daticle` fail to compile with unclosed-delimiter syntax errors,
  pre-existing and untouched by Slice 0.  `cargo test -p oxedyne_fe2o3_jdat` is
  therefore red independently of this work (the *lib* tests pass; the integration
  test files don't parse).  Worth a separate fix.

---

## Reproduction

Probes were headless and disposable:

- **CORS:** serve a blank page on `http://127.0.0.1`, then from the page
  `fetch(<provider>/chat/completions, {method:POST, headers:{Authorization:
  'Bearer dummy', 'Content-Type':'application/json'}, body:{model, messages,
  max_tokens:1}})`; a readable response = CORS allowed.
- **OPFS persistence:** `launchPersistentContext(userDataDir)` → write via
  `navigator.storage.getDirectory()` → close → reopen same `userDataDir` → read.
- **A4 loop:** local mock returns a `write_file` tool_call then a final message;
  the page runs the tool loop with OPFS as the executor.
- **wasm:** `cargo build -p <crate> --target wasm32-unknown-unknown`.
