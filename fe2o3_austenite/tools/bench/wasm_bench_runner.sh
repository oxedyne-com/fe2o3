#!/usr/bin/env bash
# Runs wasm_bench.mjs for both wasm engines on each doc, interleaved (Austenite
# then typst.ts per doc, round for round -- there is only one round per doc here
# since wasm_bench.mjs itself runs the warm-ups and measured runs in one long-
# lived process), each capped and timed exactly as the native runs are.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/host.sh
source "$HERE/lib/host.sh"

AUST_VENDOR="${AUST_VENDOR:?set AUST_VENDOR to the daimond austenite vendor dir}"
TYPST_VENDOR="${TYPST_VENDOR:?set TYPST_VENDOR to the daimond typst.ts vendor dir}"
WARMUPS="${WARMUPS:-1}"
RUNS="${RUNS:-3}"
FMT="${FMT:-pdf}"
OUT_DIR="${OUT_DIR:?set OUT_DIR}"
DOCS=("$@")

if [[ ${#DOCS[@]} -eq 0 ]]; then
	echo "usage: OUT_DIR=... AUST_VENDOR=... TYPST_VENDOR=... wasm_bench_runner.sh <doc.typ>..." >&2
	exit 2
fi

mkdir -p "$OUT_DIR"
JSONL="$OUT_DIR/wasm_runs.jsonl"
: > "$JSONL"

run_engine() {
	local doc="$1" engine="$2" vendor="$3"
	local docname; docname="$(basename "$doc" .typ)"
	local stdout_f time_f
	stdout_f="$(mktemp)"; time_f="$(mktemp)"

	local load_before psi_cpu_before psi_mem_before
	load_before="$(loadavg1)"; psi_cpu_before="$(psi_avg10 cpu)"; psi_mem_before="$(psi_avg10 memory)"

	local t0 t1
	t0=$(date +%s.%N)
	systemd-run --user --scope --quiet -p MemoryMax=3G --slice=claude-rc.slice -- \
		/usr/bin/time -v -o "$time_f" -- \
		node "$HERE/wasm_bench.mjs" "--engine=$engine" "--vendor=$vendor" "--doc=$doc" \
			"--warmups=$WARMUPS" "--runs=$RUNS" "--fmt=$FMT" \
		> "$stdout_f" 2> /dev/null
	local exit_code=$?
	t1=$(date +%s.%N)

	local load_after psi_cpu_after psi_mem_after
	load_after="$(loadavg1)"; psi_cpu_after="$(psi_avg10 cpu)"; psi_mem_after="$(psi_avg10 memory)"

	local rss_kb=0
	[[ -f "$time_f" ]] && rss_kb="$(awk -F': ' '/Maximum resident set size/{print $2}' "$time_f")"
	[[ -z "$rss_kb" ]] && rss_kb=0

	local inner; inner="$(tail -n1 "$stdout_f" 2>/dev/null)"
	[[ -z "$inner" ]] && inner='{"ok":false,"error":"no stdout from wasm_bench.mjs"}'

	local flagged=false
	if awk -v a="$load_before" -v b="$load_after" -v t="$BENCH_LOAD_THRESHOLD" \
		'BEGIN{exit !(a>t || b>t)}'; then
		flagged=true
	fi

	jq -nc \
		--arg doc "$docname" --arg engine "$engine" --argjson rss_kb "${rss_kb:-0}" \
		--argjson exit_code "$exit_code" \
		--argjson load_before "$load_before" --argjson load_after "$load_after" \
		--argjson psi_cpu_before "$psi_cpu_before" --argjson psi_cpu_after "$psi_cpu_after" \
		--argjson psi_mem_before "$psi_mem_before" --argjson psi_mem_after "$psi_mem_after" \
		--argjson flagged_under_load "$flagged" --arg ts "$(date -u +%FT%TZ)" \
		--argjson inner "$inner" \
		'$inner * {ts:$ts, doc:$doc, engine:("wasm-"+$engine), rss_kb:$rss_kb, exit_code:$exit_code,
		  load_before:$load_before, load_after:$load_after,
		  psi_cpu_before:$psi_cpu_before, psi_cpu_after:$psi_cpu_after,
		  psi_mem_before:$psi_mem_before, psi_mem_after:$psi_mem_after,
		  flagged_under_load:$flagged_under_load}' \
		>> "$JSONL"

	rm -f "$stdout_f" "$time_f"
}

for doc in "${DOCS[@]}"; do
	run_engine "$doc" austenite "$AUST_VENDOR"
	run_engine "$doc" typstts "$TYPST_VENDOR"
	echo "[wasm_bench] $(basename "$doc" .typ): austenite + typstts done" >&2
done

echo "$JSONL"
