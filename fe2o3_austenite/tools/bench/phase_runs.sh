#!/usr/bin/env bash
# The native phase table's runs: RUNS runs of `austenite --eval --timings` on each document, one
# process a run, each under both Austenite build-slot locks (lib/quiet_host.sh) and released, with a
# pause, before the next. cpu PSI avg10 and the 1-minute load are read at the start and end of
# each run. A run with PSI above PSI_LIMIT at either end is run once more, and a second flagged
# run is kept flagged. Every attempt is one JSONL line carrying the engine's own timings record,
# the CPU and peak resident set from /usr/bin/time -v, and the host readings; phase_table.mjs
# takes each run's last attempt.
#
# SCRATCH is where each run's PDF and record go: a RAM disk keeps the write phase from timing the disk
# (a busy disk made a 1 MB write take a second); the files are a megabyte at most.
#
# Usage: BIN=PATH/austenite OUT=FILE.jsonl [SCRATCH=DIR] [RUNS=3] phase_runs.sh <doc.typ>...
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/host.sh
source "$HERE/lib/host.sh"
# shellcheck source=lib/quiet_host.sh
source "$HERE/lib/quiet_host.sh"

BIN="${BIN:?set BIN to the austenite binary}"
OUT="${OUT:?set OUT to the JSONL file}"
RUNS="${RUNS:-3}"
PSI_LIMIT="${PSI_LIMIT:-20}"
GAP_S="${GAP_S:-12}"	# longer than the slot helper's poll, so a waiting build gets in
SCRATCH="${SCRATCH:-$(mktemp -d)}"
mkdir -p "$(dirname "$OUT")" "$SCRATCH"

# One attempt, inside the lock-holding subshell. Globals: S_DOC, S_RUN, S_ATTEMPT.
body() {
	local name time_f rec_f out_d
	name="$(basename "$S_DOC" .typ)"
	time_f="$SCRATCH/$name.time"; rec_f="$SCRATCH/$name.json"; out_d="$SCRATCH/$name.out"
	local load_before psi_before load_after psi_after io_before io_after
	load_before="$(loadavg1)"; psi_before="$(psi_avg10 cpu)"; io_before="$(psi_avg10 io)"
	systemd-run --user --scope --quiet -p MemoryMax=3G --slice=claude-rc.slice -- \
		/usr/bin/time -v -o "$time_f" -- \
		"$BIN" --eval --timings "$rec_f" "$S_DOC" "$out_d" > /dev/null 2> "$SCRATCH/$name.err"
	local exit_code=$?
	load_after="$(loadavg1)"; psi_after="$(psi_avg10 cpu)"; io_after="$(psi_avg10 io)"
	local user_s sys_s rss_kb wall_s
	user_s="$(awk -F': ' '/User time/{print $2}' "$time_f")"
	sys_s="$(awk -F': ' '/System time/{print $2}' "$time_f")"
	rss_kb="$(awk -F': ' '/Maximum resident set size/{print $2}' "$time_f")"
	wall_s="$(awk -F': ' '/Elapsed/{n=split($2,a,":"); s=0; for(i=1;i<=n;i++) s=s*60+a[i]; print s}' "$time_f")"
	local rec='null'
	[[ -s "$rec_f" ]] && rec="$(cat "$rec_f")"
	local flagged=false
	if awk -v a="$psi_before" -v b="$psi_after" -v t="$PSI_LIMIT" 'BEGIN{exit !(a>t || b>t)}'; then
		flagged=true
	fi
	jq -nc \
		--arg doc "$name" --arg sha "$(sha256sum "$S_DOC" | awk '{print $1}')" \
		--argjson run "$S_RUN" --argjson attempt "$S_ATTEMPT" --argjson exit_code "$exit_code" \
		--argjson user_s "${user_s:-0}" --argjson sys_s "${sys_s:-0}" --argjson rss_kb "${rss_kb:-0}" \
		--argjson wall_s "${wall_s:-0}" \
		--argjson load_before "$load_before" --argjson load_after "$load_after" \
		--argjson psi_before "$psi_before" --argjson psi_after "$psi_after" \
		--argjson io_before "$io_before" --argjson io_after "$io_after" \
		--argjson flagged "$flagged" --argjson timings "$rec" --arg ts "$(date -u +%FT%TZ)" \
		'{ts:$ts, doc:$doc, doc_sha256:$sha, run:$run, attempt:$attempt, exit_code:$exit_code,
		  user_s:$user_s, sys_s:$sys_s, wall_s:$wall_s, rss_kb:$rss_kb,
		  load_before:$load_before, load_after:$load_after, psi_before:$psi_before,
		  psi_after:$psi_after, io_before:$io_before, io_after:$io_after, flagged:$flagged, timings:$timings}' >> "$OUT"
	[[ "$flagged" == true ]] && return 3
	return 0
}

for S_DOC in "$@"; do
	for ((S_RUN = 1; S_RUN <= RUNS; S_RUN++)); do
		S_ATTEMPT=1
		while :; do
			with_quiet_host body
			rc=$?
			echo "[phases] $(basename "$S_DOC") run $S_RUN attempt $S_ATTEMPT: $(tail -n1 "$OUT" | jq -c '{exit_code,wall_s,psi_before,psi_after,flagged}')" >&2
			if [[ $rc -eq 3 && $S_ATTEMPT -lt 2 ]]; then
				S_ATTEMPT=$((S_ATTEMPT + 1)); sleep "$GAP_S"; continue
			fi
			break
		done
		sleep "$GAP_S"
	done
done
echo "$OUT"
