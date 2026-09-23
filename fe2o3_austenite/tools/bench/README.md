# S0: Austenite vs Typst speed bench

The harness behind unit S0 of
`~/usr/code/ai/claude/notes/austenite_typesetter_tail_plan_20260923.md`. It compares
native Austenite against `typst compile` (`-j1` and `-j16`), and -- where Daimond's
vendored wasm copies exist -- Austenite wasm against typst.ts wasm in node, including
incremental edit latency. It writes a JSON report plus a markdown summary and refuses
to present numbers taken while the host was loaded unless told to anyway.

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
- **Load flagging**: before and after every run, the harness reads `/proc/loadavg` (the
  1-minute average) and `some avg10` from `/proc/pressure/cpu` and `/proc/pressure/
  memory`. A run is flagged (`flagged_under_load: true`) when the load average was
  above `BENCH_LOAD_THRESHOLD` (default 2) on either side. `aggregate.py` refuses to
  print a report's numbers -- it writes a "REFUSED" json/md pair instead -- when any run
  in the batch was flagged, unless `--force` is passed, in which case the report is
  produced but headed "TAKEN UNDER LOAD" in bold, in the markdown and as `under_load:
  true` in the JSON.
- **Incremental edit latency**: `edit_latency.mjs` keeps one compiler instance alive
  across a scripted run of one-character edits to five positions in the document (the
  synthetic generator tags every paragraph with a stable `EDITTOKn` marker so a
  position can be found and mutated by string search), timing each edit's recompile.
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
| `wasm_bench.mjs` / `wasm_bench_runner.sh` | wasm full-compile leg (node process per engine per doc, capped by the runner) |
| `edit_latency.mjs` / `edit_latency_runner.sh` | incremental edit-latency leg |
| `gen_synthetic.py` | deterministic synthetic `.typ` generator, any page count |
| `aggregate.py` | merges the three legs' JSONL into `bench_report.json` + `bench_report.md`, and the load gate |
| `lib/host.sh` | bash: `/proc/loadavg` + PSI reading, the capped-and-timed single-run helper |
| `lib/wasm_common.mjs` | node: loads both vendored wasm compilers the way `www/js/typst.js` does, minus its use of `fetch` (Node does not resolve `file://` through `fetch`; confirmed on this host, Node v20.20.2 -- everything here reads with `fs` instead) |

## Wasm legs are best-effort

The wasm legs run only when Daimond's vendored copies are present:
`~/usr/code/web/apps/oxedyne/daimond/www/vendor/austenite/` and `.../vendor/typst/`.
Both are hand-shipped (not built by any `cargo`/`npm` step in this repo -- see the
Daimond integration plan's decision on `vendor/austenite`), so their absence on a
fresh checkout is normal, not an error: `run_bench.sh` says so on stderr and reports
the native leg alone. Pass `--skip-wasm` to skip them even when present.
