# S0: Austenite vs Typst speed bench

The harness behind unit S0 of
`~/usr/code/ai/claude/notes/austenite_typesetter_tail_plan_20260923.md`. It compares
native Austenite against `typst compile` (`-j1` and `-j16`, pinned to `~/bin/typst`
0.15.1), and -- where Daimond's vendored wasm copies exist -- Austenite wasm against
typst.ts wasm in node, in three labelled modes (below), plus incremental edit latency.
It writes a JSON report plus a markdown summary and refuses to present numbers taken
while the host was loaded unless told to anyway.

**A withdrawn figure shaped this harness.** An earlier "70x slower" wasm result
compared a cold Austenite compile against a typst.ts run that hit its own comemo
cache, because both engines were run through the same long-lived instance on
unchanged input. That ratio was never a real comparison, and it is withdrawn.
Every wasm leg below is now measured under a mode label that says which cache
state it was taken in, so the two engines are never compared cache-hit-against-
cold-compile again.

## Quick start

```bash
# 1. Build the austenite binary (only through this command -- see the top-level plan):
flock ~/.cache/cargo-targets/claude-rc-3/evslot1.lock ~/usr/code/bash/rc-build 4G -- bash -c \
  'cd ~/.cache/austenite-bench-wt && CARGO_BUILD_JOBS=4 \
   CARGO_TARGET_DIR=~/.cache/cargo-targets/claude-rc-3/bench \
   cargo build --release -p oxedyne_fe2o3_austenite --bin austenite'

# 2. Run the harness (smoke mode by default -- small, fast, proves the plumbing):
tools/bench/run_bench.sh \
  --austenite-bin ~/.cache/cargo-targets/claude-rc-3/bench/release/austenite

# 3. Read the report:
cat tools/bench/out/<timestamp>/bench_report.md
```

For the real baseline, on an idle host:

```bash
tools/bench/run_bench.sh --austenite-bin <path> --full
```

`--full` runs the S0 protocol as specified: 2 warm-ups, 7 measured runs, the 300-page
synthetic document, 50 edits at 5 positions. Smoke mode (the default) uses 1 warm-up, 3
runs, a 20-page synthetic document and 6 edits at 2 positions -- enough to prove every
leg of the harness runs and produces sane numbers, not enough to mean anything as a
baseline.

## What it measures, and how

- **Engines**: `typst compile -j 16` (default), `typst compile -j 1`, native `austenite`,
  Austenite wasm in node, and typst.ts wasm in node -- Daimond's real compile path.
- **Corpora**: `fe2o3_austenite/samples/*.typ` plus one generated synthetic document
  (`gen_synthetic.py`; seeded, no book content, no cetz). This is a narrower corpus than
  the full plan's four-kind/three-size U12 set, which S0 does not depend on and did not
  wait for -- see the brief this harness was built from.
- **Protocol**: for each (doc, engine), `WARMUPS` untimed passes then `RUNS` measured
  passes, run round-robin across every engine for that doc (engine A, B, C, A, B, C,
  ...) rather than all of one engine then all of another, so host load hits every engine
  equally over the course of the run. Each run is capped with `systemd-run --user
  --scope --quiet -p MemoryMax=3G --slice=claude-rc.slice` and timed with
  `/usr/bin/time -v`, which gives wall clock and peak RSS in one pass.
- **Cold vs unchanged recompile (native)**: every native run is its own process --
  a fresh `typst compile` invocation, a fresh `austenite` run -- so the native leg is
  cold by construction. There is nothing to label here.
- **Cold, unchanged and edit (wasm)**: `wasm_bench.mjs` runs one of three modes, named
  as check G11 names them and set with `--mode` (`wasm_bench_runner.sh` runs all three
  by default; narrow with `WASM_MODES`, e.g. `WASM_MODES=edit`):
  - `cold` -- one process per measured run, each loading a fresh wasm instance (new
    linear memory: a new compiler object on an old instance would keep its statics, and
    typst.ts's comemo cache is one of them). The compile is timed; the instance load is
    reported beside it (`load_s`).
  - `unchanged` -- one long-lived instance, every warm-up and measured run against the
    *same unchanged* project. For an engine that memoises unchanged input (typst.ts's
    comemo) this times its cache hit, which is what an idle live view costs.
  - `edit` -- one long-lived instance, warmed on the unchanged project; before each
    measured run one more letter is typed into a word near the middle of the document,
    so every source is new and a memoising engine recompiles only what the edit reaches.

  `aggregate.py` reports `wasm-<engine>-<mode>` as separate rows and computes the
  austenite/typst.ts ratio per mode -- never across modes.
- **Load flagging**: before and after every run, the harness reads `/proc/loadavg` (the
  1-minute average) and `some avg10` from `/proc/pressure/cpu` and `/proc/pressure/
  memory`. A run is flagged (`flagged_under_load: true`) when the load average was
  above `BENCH_LOAD_THRESHOLD` (default 20; the plan's 2 is never met on the 16-thread
  fleet host, and interleaving puts whatever load there is on every engine alike) on
  either side. `aggregate.py` refuses to
  print a report's numbers -- it writes a "REFUSED" json/md pair instead -- when any run
  in the batch was flagged, unless `--force` is passed, in which case the report is
  produced but headed "TAKEN UNDER LOAD" in bold, in the markdown and as `under_load:
  true` in the JSON.
- **Incremental edit latency**: `edit_latency.mjs` keeps one compiler instance alive
  across a scripted run of 50 keystrokes (the default) at five positions in turn, each typing one letter
  onto the end of a word, and times each edit's recompile. The percentiles are nearest-rank
  (`lib/stats.mjs`: p50 of 50 samples is the 25th smallest, p95 the 48th), and Austenite's projects
  carry `strict: true`, as Daimond sends it on every door (the synthetic generator tags
  every paragraph with a stable `EDITTOKn` marker, found with its trailing space so that
  `EDITTOK1` never matches the start of `EDITTOK10`).
  Austenite's leg calls `compileProjectDelta` with the `known`-id cache carried forward
  edit to edit, exactly as the real watch loop does. **typst.ts's leg is a full
  recompile to the `vector` format on every edit, not a call to typst.ts's
  `incr_compile`/`IncrServer` API** -- checked against `www/js/typstwatch.js` and
  `www/js/typst.js` in the Daimond app, neither of which calls that API anywhere; the
  live-view path Daimond actually ships is `compileProjectVector`, a full recompile to
  typst.ts's intermediate format, rendered to SVG by a second wasm module
  (`typst_ts_renderer_bg.wasm`) through `render_svg(session, domElement)`, which needs a
  live DOM. Timing the API Daimond does not call would not be the number a Daimond user
  feels, so this harness times what Daimond actually does and excludes the DOM-render
  step, which needs a browser; see "What is not measured" below.
- **Phase attribution**: `austenite --timings` is a U9 addendum that has not landed
  (checked by `grep -q -- '--timings' src/bin/austenite.rs` in `speed_bench.sh`, not by
  probing the binary -- `austenite`'s argument parser treats an unrecognised flag as a
  positional path, so guessing wrong would silently corrupt the invocation rather than
  fail loudly). Until it lands, `speed_bench.sh` runs without it and says so on stderr;
  once it lands, the same check turns it on with no harness change needed. Typst's own
  `--timings` and a `perf record -g` cross-check are follow-on work this unit does not
  attempt.
- **Peak RSS**: `/usr/bin/time -v`'s "Maximum resident set size", per run for the native
  and wasm-compile legs; for edit latency, once per whole edit session (a per-edit
  figure inside one warm process is not a meaningful peak).

## What is not measured

- The final DOM-diff/SVG-string step of typst.ts's live view (`render_svg`), because it
  needs an `HTMLElement`, i.e. a browser, not node. It runs on typst.ts's SIR output
  after the compile this harness times, and by the numbers in
  `www/js/typstwatch.js`'s own comments is the cheaper of the two steps, but it is not
  zero and this harness does not claim to have measured it. A follow-on that drives a
  real (headless) browser closes this gap.
- Typst's own `--timings` trace and the `perf record -g` cross-check the plan's §S0
  names for phase attribution.
- Anything beyond `fe2o3_austenite/samples/*.typ` plus the one generated document --
  no oxeweb TechSpec/Overview, no cetz examples, no 50/1000-page variants. Point
  `--docs` at a wider glob once U12's corpora exist.

## Files

| File | Role |
|---|---|
| `run_bench.sh` | top-level driver: generates the synthetic doc, runs every leg, aggregates |
| `speed_bench.sh` | native leg: typst -j16/-j1 vs austenite, interleaved, capped, timed |
| `wasm_bench.mjs` / `wasm_bench_runner.sh` | wasm full-compile leg (node process per engine per doc per mode, capped by the runner; modes: `cold`, `unchanged`, `edit`) |
| `edit_latency.mjs` / `edit_latency_runner.sh` | incremental edit-latency leg |
| `gen_synthetic.py` | deterministic synthetic `.typ` generator, any page count |
| `aggregate.py` | merges the three legs' JSONL into `bench_report.json` + `bench_report.md`, and the load gate |
| `typing_sessions.sh` | the typing test (D-20261001-08): rounds of one `edit_latency.mjs` session per engine, the starting engine alternating, each under both Austenite build-slot locks, host readings at each end, a flagged session run once more |
| `cost_runs.sh` | cold and unchanged wasm compiles, one `wasm_bench.mjs` process per run, the engines interleaved, each under both slot locks |
| `bench_verdict.mjs` | `typing`: per-session p50/p95, medians, T = max(1.25 x typst.ts, typst.ts + 16 ms), the verdict; `cost`: medians and the ratio against the 1.5 limit |
| `percentile_check.mjs` | node check that nearest-rank over 1..50 gives p50 = 25 and p95 = 48, and the threshold arithmetic; red when `lib/stats.mjs` indexes with a floor |
| `lib/stats.mjs` | nearest-rank percentile, median, the typing threshold |
| `lib/quiet_host.sh` | `with_quiet_host`: runs a command holding both Austenite build-slot locks (append-open, never replaced) |
| `lib/host.sh` | bash: `/proc/loadavg` + PSI reading, the capped-and-timed single-run helper |
| `lib/wasm_common.mjs` | node: loads both vendored wasm compilers the way `www/js/typst.js` does, minus its use of `fetch` (Node does not resolve `file://` through `fetch`; confirmed on this host, Node v20.20.2 -- everything here reads with `fs` instead) |

## Wasm legs are best-effort

The wasm legs run the tree's own build when `--aust-vendor DIR` (or `AUST_VENDOR`) names a
`wasm-pack build fe2o3_austenite --release --target web --features wasm` output, and otherwise
Daimond's vendored copies, which are whatever was last shipped. They run only when the copies
are present:
`~/usr/code/web/apps/oxedyne/daimond/www/vendor/austenite/` and `.../vendor/typst/`.
Both are hand-shipped (not built by any `cargo`/`npm` step in this repo -- see the
Daimond integration plan's decision on `vendor/austenite`), so their absence on a
fresh checkout is normal, not an error: `run_bench.sh` says so on stderr and reports
the native leg alone. Pass `--skip-wasm` to skip them even when present.
