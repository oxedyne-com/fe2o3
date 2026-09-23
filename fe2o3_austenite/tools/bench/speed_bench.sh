#!/usr/bin/env bash
# S0 native speed bench: typst compile -j16, typst compile -j1 and the native
# austenite binary, interleaved run-for-run across engines so host load hits
# every engine equally, each run capped at 3G and timed under
# `/usr/bin/time -v`. See tools/bench/README.md for the full protocol and the
# owner-decision pass line this feeds.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/host.sh
source "$HERE/lib/host.sh"

AUSTENITE_BIN="${AUSTENITE_BIN:?set AUSTENITE_BIN to the built austenite binary}"
TYPST_BIN="${TYPST_BIN:-typst}"
WARMUPS="${WARMUPS:-1}"
RUNS="${RUNS:-3}"
OUT_DIR="${OUT_DIR:?set OUT_DIR}"
DOCS=("$@")

if [[ ${#DOCS[@]} -eq 0 ]]; then
	echo "usage: OUT_DIR=... AUSTENITE_BIN=... speed_bench.sh <doc.typ>..." >&2
	exit 2
fi

mkdir -p "$OUT_DIR"
JSONL="$OUT_DIR/native_runs.jsonl"
: > "$JSONL"

# The --timings flag is a U9 addendum not yet landed -- detect it from the
# source rather than probing the binary, because austenite's argument parser
# treats an unrecognised flag as a positional path (see bin/austenite.rs),
# which would silently break the invocation rather than error cleanly.
AUSTENITE_SRC="$HERE/../../src/bin/austenite.rs"
HAS_TIMINGS=false
if [[ -f "$AUSTENITE_SRC" ]] && grep -q -- '--timings' "$AUSTENITE_SRC"; then
	HAS_TIMINGS=true
fi
echo "[speed_bench] austenite --timings: $HAS_TIMINGS (U9 addendum)" >&2

engines_for_doc() {
	local doc="$1" scratch="$2"
	# name|jobs|command...
	echo "typst-j16|16|$TYPST_BIN compile -j 16 $doc $scratch/typst-j16.pdf"
	echo "typst-j1|1|$TYPST_BIN compile -j 1 $doc $scratch/typst-j1.pdf"
	local aust_cmd=("$AUSTENITE_BIN")
	if $HAS_TIMINGS; then
		aust_cmd+=(--timings)
	fi
	aust_cmd+=("$doc" "$scratch/austenite-out")
	echo "austenite-native|1|${aust_cmd[*]}"
}

for doc in "${DOCS[@]}"; do
	docname="$(basename "$doc" .typ)"
	scratch="$OUT_DIR/scratch/$docname"
	mkdir -p "$scratch"
	mapfile -t engines < <(engines_for_doc "$doc" "$scratch")

	# Warm-ups: one pass per engine, not recorded.
	for ((w = 0; w < WARMUPS; w++)); do
		for e in "${engines[@]}"; do
			IFS='|' read -r name jobs cmd <<< "$e"
			# shellcheck disable=SC2086
			systemd-run --user --scope --quiet -p MemoryMax=3G --slice=claude-rc.slice -- \
				bash -c "$cmd" > /dev/null 2>&1
		done
	done

	# Measured runs, interleaved ABC-ABC-... across engines.
	for ((r = 0; r < RUNS; r++)); do
		for e in "${engines[@]}"; do
			IFS='|' read -r name jobs cmd <<< "$e"
			run_timed "$docname" "$name" "$jobs" "$JSONL" -- bash -c "$cmd"
		done
	done
	echo "[speed_bench] $docname: $((WARMUPS)) warm-up(s), $RUNS measured run(s) x ${#engines[@]} engines" >&2
done

echo "$JSONL"
