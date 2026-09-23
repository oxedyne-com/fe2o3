#!/usr/bin/env bash
# S0 bench harness top-level driver. Runs the native comparison
# (typst -j16 / -j1 vs the austenite binary) and, when the vendored wasm is
# present, the wasm comparison (Austenite wasm vs typst.ts wasm in node) and
# the incremental edit-latency bench, on the sample docs plus a generated
# 300-page synthetic document, then aggregates everything into one JSON
# report and one markdown summary.
#
# Defaults to SMOKE mode: 1 warm-up, 3 measured runs, a small synthetic doc,
# few edits. Pass --full for the real S0 protocol (2 warm-ups, 7 runs, the
# 300-page doc, 50 edits at 5 positions) -- run that only when the host is
# idle; see aggregate.py for the load gate this cannot be talked past.
#
# Usage: run_bench.sh --austenite-bin PATH [--full] [--force]
#                      [--out DIR] [--docs 'glob'] [--skip-wasm]
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AUSTENITE_ROOT="$(cd "$HERE/../.." && pwd)"   # fe2o3_austenite/
# shellcheck source=lib/host.sh
source "$HERE/lib/host.sh"

AUSTENITE_BIN=""
FULL=false
FORCE=false
OUT_DIR="$HERE/out/$(date -u +%Y%m%dT%H%M%SZ)"
SKIP_WASM=false
DOCS_GLOB=""

# Daimond's vendored wasm -- see tools/bench/README.md for how these paths
# were found; both are hand-shipped, not part of any build, so their absence
# is normal on a fresh checkout and just means the wasm legs are skipped.
DAIMOND_WWW="$HOME/usr/code/web/apps/oxedyne/daimond/www"
AUST_VENDOR="$DAIMOND_WWW/vendor/austenite"
TYPST_VENDOR="$DAIMOND_WWW/vendor/typst"

while [[ $# -gt 0 ]]; do
	case "$1" in
		--austenite-bin) AUSTENITE_BIN="$2"; shift 2 ;;
		--full) FULL=true; shift ;;
		--force) FORCE=true; shift ;;
		--out) OUT_DIR="$2"; shift 2 ;;
		--docs) DOCS_GLOB="$2"; shift 2 ;;
		--skip-wasm) SKIP_WASM=true; shift ;;
		*) echo "run_bench.sh: unknown argument $1" >&2; exit 2 ;;
	esac
done

if [[ -z "$AUSTENITE_BIN" || ! -x "$AUSTENITE_BIN" ]]; then
	echo "run_bench.sh: --austenite-bin must point at a built, executable austenite binary" >&2
	exit 2
fi

mkdir -p "$OUT_DIR"
export OUT_DIR AUSTENITE_BIN AUST_VENDOR TYPST_VENDOR

if $FULL; then
	export WARMUPS=2 RUNS=7 EDITS=50 POSITIONS=5
	SYN_PAGES=300
else
	export WARMUPS=1 RUNS=3 EDITS=6 POSITIONS=2
	SYN_PAGES=20
fi

# Under-load notice up front -- the harness still runs (so a smoke test can
# prove the plumbing works), but aggregate.py refuses to print the numbers
# as a baseline unless --force is given, and every markdown/json output says so.
if host_is_loaded; then
	echo "[run_bench] HOST IS LOADED (1-min load average $(loadavg1) > $BENCH_LOAD_THRESHOLD)." >&2
	echo "[run_bench] proceeding (this run is for plumbing, not a baseline) -- runs will be flagged." >&2
fi

# Docs: the fixed samples plus one generated synthetic doc.
DOCS=()
if [[ -n "$DOCS_GLOB" ]]; then
	# shellcheck disable=SC2086
	for f in $DOCS_GLOB; do DOCS+=("$f"); done
else
	for f in "$AUSTENITE_ROOT"/samples/*.typ; do DOCS+=("$f"); done
	SYN="$OUT_DIR/synthetic_${SYN_PAGES}.typ"
	python3 "$HERE/gen_synthetic.py" "$SYN_PAGES" "$SYN"
	DOCS+=("$SYN")
fi
echo "[run_bench] docs: ${DOCS[*]}" >&2

# Native leg.
bash "$HERE/speed_bench.sh" "${DOCS[@]}"

# Wasm legs, only where the vendored copies exist (they are hand-shipped, not
# built by this repo -- see tools/bench/README.md).
if ! $SKIP_WASM && [[ -f "$AUST_VENDOR/oxedyne_fe2o3_austenite_bg.wasm" && \
                       -f "$TYPST_VENDOR/typst_ts_web_compiler_bg.wasm" ]]; then
	bash "$HERE/wasm_bench_runner.sh" "${DOCS[@]}"
	bash "$HERE/edit_latency_runner.sh" "${DOCS[@]}"
else
	echo "[run_bench] skipping wasm legs: vendored wasm not found under $DAIMOND_WWW/vendor" >&2
fi

LABEL="mode=$($FULL && echo full || echo smoke); host load 1-min before finish: $(loadavg1); $(date -u +%FT%TZ)"
FORCE_FLAG=()
$FORCE && FORCE_FLAG=(--force)
python3 "$HERE/aggregate.py" --in "$OUT_DIR" \
	--out-json "$OUT_DIR/bench_report.json" --out-md "$OUT_DIR/bench_report.md" \
	--label "$LABEL" "${FORCE_FLAG[@]}"

echo "[run_bench] report: $OUT_DIR/bench_report.md" >&2
echo "$OUT_DIR"
