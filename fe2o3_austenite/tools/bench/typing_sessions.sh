#!/usr/bin/env bash
# The typing test's sessions (D-20261001-08): ROUNDS rounds, each one session per engine, the
# engine that starts alternating from round to round (round 0 Austenite first). A session is one
# fresh node process (edit_latency.mjs), capped like every other run, holding both Austenite
# build-slot locks while it runs (lib/quiet_host.sh) and releasing them, with a pause, before the
# next, so the lane's builds run in the gaps.
#
# At each session's start and end the cpu PSI `some avg10` and the 1-minute load are read. A
# session with avg10 above PSI_LIMIT at either end is run once more, and a second flagged run
# is reported flagged. A session whose result is invalid (an error, a changed page count) is not
# repeated. Every attempt is appended to the JSONL, with its round and attempt number; the verdict
# script (bench_verdict.mjs typing) takes each round's last attempt.
#
# Usage: AUST_VENDOR=DIR TYPST_VENDOR=DIR OUT=FILE.jsonl typing_sessions.sh <doc.typ> [rounds]
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/host.sh
source "$HERE/lib/host.sh"
# shellcheck source=lib/quiet_host.sh
source "$HERE/lib/quiet_host.sh"

AUST_VENDOR="${AUST_VENDOR:?set AUST_VENDOR to the Austenite package dir}"
TYPST_VENDOR="${TYPST_VENDOR:?set TYPST_VENDOR to the typst.ts vendor dir}"
OUT="${OUT:?set OUT to the JSONL file}"
EDITS="${EDITS:-50}"
POSITIONS="${POSITIONS:-5}"
PSI_LIMIT="${PSI_LIMIT:-20}"
GAP_S="${GAP_S:-25}"	# a gap longer than the slot helper's 10 s poll, so a waiting build gets in
DOC="${1:?usage: typing_sessions.sh <doc.typ> [rounds]}"
ROUNDS="${2:-3}"

DOCNAME="$(basename "$DOC" .typ)"
DOC_SHA="$(sha256sum "$DOC" | awk '{print $1}')"
mkdir -p "$(dirname "$OUT")"

# One attempt at one session, run inside the lock-holding subshell. Globals set by the caller:
# S_ENGINE, S_VENDOR, S_ROUND, S_ATTEMPT, S_T0 (epoch seconds when the lock was asked for).
session_body() {
	local locked_s stdout_f time_f
	locked_s="$(awk -v a="$S_T0" -v b="$(date +%s.%N)" 'BEGIN{printf "%.2f", b-a}')"
	stdout_f="$(mktemp)"; time_f="$(mktemp)"

	local load_before psi_before load_after psi_after
	load_before="$(loadavg1)"; psi_before="$(psi_avg10 cpu)"
	local t0 t1
	t0=$(date +%s.%N)
	systemd-run --user --scope --quiet -p MemoryMax=3G --slice=claude-rc.slice -- \
		/usr/bin/time -v -o "$time_f" -- \
		node "$HERE/edit_latency.mjs" "--engine=$S_ENGINE" "--vendor=$S_VENDOR" "--doc=$DOC" \
			"--edits=$EDITS" "--positions=$POSITIONS" \
		> "$stdout_f" 2> "$stdout_f.err"
	local exit_code=$?
	t1=$(date +%s.%N)
	load_after="$(loadavg1)"; psi_after="$(psi_avg10 cpu)"

	local rss_kb=0
	[[ -f "$time_f" ]] && rss_kb="$(awk -F': ' '/Maximum resident set size/{print $2}' "$time_f")"
	[[ -z "$rss_kb" ]] && rss_kb=0
	local inner; inner="$(tail -n1 "$stdout_f" 2>/dev/null)"
	[[ -z "$inner" ]] && inner='{"ok":false,"error":"no stdout from edit_latency.mjs"}'

	local flagged=false
	if awk -v a="$psi_before" -v b="$psi_after" -v t="$PSI_LIMIT" 'BEGIN{exit !(a>t || b>t)}'; then
		flagged=true
	fi

	jq -nc \
		--arg doc "$DOCNAME" --arg sha "$DOC_SHA" --arg engine "$S_ENGINE" \
		--argjson round "$S_ROUND" --argjson attempt "$S_ATTEMPT" \
		--argjson rss_kb "${rss_kb:-0}" --argjson exit_code "$exit_code" \
		--argjson load_before "$load_before" --argjson load_after "$load_after" \
		--argjson psi_before "$psi_before" --argjson psi_after "$psi_after" \
		--argjson flagged "$flagged" --argjson lock_wait_s "$locked_s" \
		--argjson wall_s "$(awk -v a="$t0" -v b="$t1" 'BEGIN{printf "%.2f", b-a}')" \
		--arg ts "$(date -u +%FT%TZ)" --argjson inner "$inner" \
		'$inner * {ts:$ts, doc:$doc, doc_sha256:$sha, engine:$engine, round:$round, attempt:$attempt,
		  rss_kb:$rss_kb, exit_code:$exit_code, load_before:$load_before, load_after:$load_after,
		  psi_before:$psi_before, psi_after:$psi_after, flagged:$flagged,
		  lock_wait_s:$lock_wait_s, wall_s:$wall_s}' \
		>> "$OUT"
	rm -f "$stdout_f" "$stdout_f.err" "$time_f"
	[[ "$flagged" == true ]] && return 3
	return 0
}

run_session() {
	S_ENGINE="$1"; S_VENDOR="$2"; S_ROUND="$3"
	S_ATTEMPT=1
	local rc
	while :; do
		S_T0="$(date +%s.%N)"
		with_quiet_host session_body
		rc=$?
		echo "[typing] $DOCNAME round $S_ROUND $S_ENGINE attempt $S_ATTEMPT: $(tail -n1 "$OUT" | jq -c '{ok,p50_s,p95_s,psi_before,psi_after,load_before,load_after,flagged,lock_wait_s,wall_s}')" >&2
		if [[ $rc -eq 3 && $S_ATTEMPT -lt 2 ]]; then
			S_ATTEMPT=$((S_ATTEMPT + 1))
			sleep "$GAP_S"
			continue
		fi
		break
	done
	sleep "$GAP_S"
}

for ((r = 0; r < ROUNDS; r++)); do
	if ((r % 2 == 0)); then
		run_session austenite "$AUST_VENDOR" "$r"
		run_session typstts "$TYPST_VENDOR" "$r"
	else
		run_session typstts "$TYPST_VENDOR" "$r"
		run_session austenite "$AUST_VENDOR" "$r"
	fi
done
echo "$OUT"
