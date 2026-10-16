#!/usr/bin/env bash
# CPU-profiled wasm runs (cpuprof_runs.mjs), one fresh node process a run, each under both Austenite
# build-slot locks (lib/quiet_host.sh) and released, with a pause, before the next. cpu and io PSI avg10
# and the 1-minute load are read at each end; a run over PSI_LIMIT in cpu at either end is run once more
# and a second flagged run is kept flagged. The profile of each run is OUT_DIR/<doc>-<mode>.cpuprofile
# (the last attempt's) and one JSONL line carries the run's own report and the host readings.
#
# Usage: AUST_VENDOR=DIR OUT_DIR=DIR [EDITS=10] [REPS=1] cpuprof_sessions.sh <doc.typ> <mode>...
#        mode is cold, warm, edits or unchanged (see cpuprof_runs.mjs)
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/host.sh
source "$HERE/lib/host.sh"
# shellcheck source=lib/quiet_host.sh
source "$HERE/lib/quiet_host.sh"

AUST_VENDOR="${AUST_VENDOR:?set AUST_VENDOR to the Austenite package dir}"
OUT_DIR="${OUT_DIR:?set OUT_DIR to the output directory}"
EDITS="${EDITS:-10}"
REPS="${REPS:-1}"
PSI_LIMIT="${PSI_LIMIT:-20}"
GAP_S="${GAP_S:-12}"
DOC="${1:?usage: cpuprof_sessions.sh <doc.typ> <mode>...}"
shift
DOCNAME="$(basename "$DOC" .typ)"
mkdir -p "$OUT_DIR"

# One attempt inside the lock-holding subshell. Globals: S_MODE, S_ATTEMPT.
body() {
	local prof="$OUT_DIR/$DOCNAME-$S_MODE.cpuprofile" out_f="$OUT_DIR/$DOCNAME-$S_MODE.stdout" time_f="$OUT_DIR/$DOCNAME-$S_MODE.time"
	local load_before psi_before load_after psi_after io_before io_after
	load_before="$(loadavg1)"; psi_before="$(psi_avg10 cpu)"; io_before="$(psi_avg10 io)"
	systemd-run --user --scope --quiet -p MemoryMax=3G --slice=claude-rc.slice -- \
		/usr/bin/time -v -o "$time_f" -- \
		node "$HERE/cpuprof_runs.mjs" "--vendor=$AUST_VENDOR" "--doc=$DOC" "--mode=$S_MODE" \
			"--out=$prof" "--edits=$EDITS" "--reps=$REPS" > "$out_f" 2> "$out_f.err"
	local exit_code=$?
	load_after="$(loadavg1)"; psi_after="$(psi_avg10 cpu)"; io_after="$(psi_avg10 io)"
	local rss_kb
	rss_kb="$(awk -F': ' '/Maximum resident set size/{print $2}' "$time_f")"
	local inner; inner="$(tail -n1 "$out_f" 2>/dev/null)"
	[[ -z "$inner" ]] && inner='{"ok":false,"error":"no stdout from cpuprof_runs.mjs"}'
	local flagged=false
	if awk -v a="$psi_before" -v b="$psi_after" -v t="$PSI_LIMIT" 'BEGIN{exit !(a>t || b>t)}'; then
		flagged=true
	fi
	jq -nc --arg doc "$DOCNAME" --arg mode "$S_MODE" --argjson attempt "$S_ATTEMPT" \
		--argjson exit_code "$exit_code" --argjson rss_kb "${rss_kb:-0}" \
		--argjson load_before "$load_before" --argjson load_after "$load_after" \
		--argjson psi_before "$psi_before" --argjson psi_after "$psi_after" \
		--argjson io_before "$io_before" --argjson io_after "$io_after" \
		--argjson flagged "$flagged" --arg ts "$(date -u +%FT%TZ)" --argjson inner "$inner" \
		'$inner * {ts:$ts, doc:$doc, mode:$mode, attempt:$attempt, exit_code:$exit_code, rss_kb:$rss_kb,
		  load_before:$load_before, load_after:$load_after, psi_before:$psi_before, psi_after:$psi_after,
		  io_before:$io_before, io_after:$io_after, flagged:$flagged}' >> "$OUT_DIR/runs.jsonl"
	[[ "$flagged" == true ]] && return 3
	return 0
}

for S_MODE in "$@"; do
	S_ATTEMPT=1
	while :; do
		with_quiet_host body
		rc=$?
		echo "[cpuprof] $DOCNAME $S_MODE attempt $S_ATTEMPT: $(tail -n1 "$OUT_DIR/runs.jsonl" | jq -c '{ok,exit_code,calls_s,psi_before,psi_after,flagged}')" >&2
		if [[ $rc -eq 3 && $S_ATTEMPT -lt 2 ]]; then
			S_ATTEMPT=$((S_ATTEMPT + 1)); sleep "$GAP_S"; continue
		fi
		break
	done
	sleep "$GAP_S"
done
