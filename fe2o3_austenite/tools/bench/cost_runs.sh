#!/usr/bin/env bash
# Cold and unchanged wasm compiles (G11 (a) and (b)), the engines interleaved run for run: ROUNDS
# rounds, each one wasm_bench.mjs process per engine, the engine that goes first alternating from
# round to round (round 0 Austenite first). One process is one measured run:
#   cold       a fresh wasm instance in a fresh process, no warm-up (the instance is the cold part);
#   unchanged  one warm-up compile of the project, then one measured compile of the same project.
# Each process holds both Austenite build-slot locks while it runs (lib/quiet_host.sh) and
# releases them, with a pause, before the next. cpu PSI avg10 and the 1-minute load are read at
# each process's start and end and kept in the JSONL; a run over PSI_LIMIT at either end is
# marked flagged. bench_verdict.mjs cost takes the medians.
#
# Usage: AUST_VENDOR=DIR TYPST_VENDOR=DIR OUT=FILE.jsonl [FMT=pdf] [ROUNDS=5] [MODES="cold unchanged"] \
#        cost_runs.sh <doc.typ>
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/host.sh
source "$HERE/lib/host.sh"
# shellcheck source=lib/quiet_host.sh
source "$HERE/lib/quiet_host.sh"

AUST_VENDOR="${AUST_VENDOR:?set AUST_VENDOR to the Austenite package dir}"
TYPST_VENDOR="${TYPST_VENDOR:?set TYPST_VENDOR to the typst.ts vendor dir}"
OUT="${OUT:?set OUT to the JSONL file}"
FMT="${FMT:-pdf}"
ROUNDS="${ROUNDS:-5}"
MODES="${MODES:-cold unchanged}"
PSI_LIMIT="${PSI_LIMIT:-20}"
GAP_S="${GAP_S:-12}"
DOC="${1:?usage: cost_runs.sh <doc.typ>}"

DOCNAME="$(basename "$DOC" .typ)"
DOC_SHA="$(sha256sum "$DOC" | awk '{print $1}')"
mkdir -p "$(dirname "$OUT")"

# One process under the locks. Globals: S_ENGINE, S_VENDOR, S_MODE, S_ROUND.
run_body() {
	local stdout_f time_f warm=1
	[[ "$S_MODE" == cold ]] && warm=0
	stdout_f="$(mktemp)"; time_f="$(mktemp)"
	local load_before psi_before load_after psi_after
	load_before="$(loadavg1)"; psi_before="$(psi_avg10 cpu)"
	systemd-run --user --scope --quiet -p MemoryMax=3G --slice=claude-rc.slice -- \
		/usr/bin/time -v -o "$time_f" -- \
		node "$HERE/wasm_bench.mjs" "--engine=$S_ENGINE" "--vendor=$S_VENDOR" "--doc=$DOC" \
			"--mode=$S_MODE" "--warmups=$warm" "--runs=1" "--fmt=$FMT" \
		> "$stdout_f" 2> /dev/null
	local exit_code=$?
	load_after="$(loadavg1)"; psi_after="$(psi_avg10 cpu)"
	local rss_kb=0
	[[ -f "$time_f" ]] && rss_kb="$(awk -F': ' '/Maximum resident set size/{print $2}' "$time_f")"
	[[ -z "$rss_kb" ]] && rss_kb=0
	local inner; inner="$(tail -n1 "$stdout_f" 2>/dev/null)"
	[[ -z "$inner" ]] && inner='{"ok":false,"error":"no stdout from wasm_bench.mjs"}'
	local flagged=false
	if awk -v a="$psi_before" -v b="$psi_after" -v t="$PSI_LIMIT" 'BEGIN{exit !(a>t || b>t)}'; then
		flagged=true
	fi
	jq -nc \
		--arg doc "$DOCNAME" --arg sha "$DOC_SHA" --arg engine "$S_ENGINE" --argjson round "$S_ROUND" \
		--argjson rss_kb "${rss_kb:-0}" --argjson exit_code "$exit_code" \
		--argjson load_before "$load_before" --argjson load_after "$load_after" \
		--argjson psi_before "$psi_before" --argjson psi_after "$psi_after" \
		--argjson flagged "$flagged" --arg ts "$(date -u +%FT%TZ)" --argjson inner "$inner" \
		'$inner * {ts:$ts, doc:$doc, doc_sha256:$sha, engine:$engine, round:$round, rss_kb:$rss_kb,
		  exit_code:$exit_code, load_before:$load_before, load_after:$load_after,
		  psi_before:$psi_before, psi_after:$psi_after, flagged:$flagged}' \
		>> "$OUT"
	rm -f "$stdout_f" "$time_f"
}

run_one() {
	S_ENGINE="$1"; S_VENDOR="$2"; S_MODE="$3"; S_ROUND="$4"
	with_quiet_host run_body
	echo "[cost] $DOCNAME $FMT $S_MODE round $S_ROUND $S_ENGINE: $(tail -n1 "$OUT" | jq -c '{ok,runs,load_s,psi_before,psi_after,flagged}')" >&2
	sleep "$GAP_S"
}

for mode in $MODES; do
	for ((r = 0; r < ROUNDS; r++)); do
		if ((r % 2 == 0)); then
			run_one austenite "$AUST_VENDOR" "$mode" "$r"
			run_one typstts "$TYPST_VENDOR" "$mode" "$r"
		else
			run_one typstts "$TYPST_VENDOR" "$mode" "$r"
			run_one austenite "$AUST_VENDOR" "$mode" "$r"
		fi
	done
done
echo "$OUT"
