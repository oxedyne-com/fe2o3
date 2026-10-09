#!/usr/bin/env bash
# The edit-to-view figures of `austenite watch` (F4): one warm compile after one letter is appended to the
# document. The watch runs with the viewer off and its PDF in a scratch directory; after the first PDF the
# script appends a letter, waits for the second status line, and reports from it the warm compile's
# seconds, its passes and whether it ran warm, and the save-to-view time: the wall time from just before
# the append to the moment the new PDF is seen in place, which holds the poll and the swap as well as the
# compile. Both ends are read from the shell's own clock ($EPOCHREALTIME) and never from file times, which
# on a loaded host are stamped from a coarse clock that can lag by more than a second.
# The document is restored on exit, byte for byte and with its time.
#
# Usage: watch_edit.sh <austenite-binary> <doc.typ> [extra austenite arguments, e.g. --set k=v]...
# Environment: SCRATCH (default: a new directory under ~/.cache/austenite/claude-rc-2/spd), WAIT_S (per
# wait, default 240).
set -uo pipefail

BIN="${1:?usage: watch_edit.sh <austenite-binary> <doc.typ> [extra arguments]...}"
DOC="${2:?usage: watch_edit.sh <austenite-binary> <doc.typ> [extra arguments]...}"
shift 2
WAIT_S="${WAIT_S:-240}"
DOC="$(readlink -f "$DOC")"

OWN=0
if [ -z "${SCRATCH:-}" ]; then
	mkdir -p "$HOME/.cache/austenite/claude-rc-2/spd"
	SCRATCH="$(mktemp -d "$HOME/.cache/austenite/claude-rc-2/spd/watch_edit.XXXXXX")"
	OWN=1
fi
mkdir -p "$SCRATCH"
BACK="$SCRATCH/doc.orig"
OUT="$SCRATCH/out.pdf"
LOG="$SCRATCH/watch.log"
cp -p "$DOC" "$BACK"
PID=""

cleanup() {
	if [ -n "$PID" ]; then
		kill "$PID" 2>/dev/null
		wait "$PID" 2>/dev/null
	fi
	cp -p "$BACK" "$DOC"
	if [ "$OWN" = 1 ]; then
		rm -rf "${SCRATCH:?}"
	fi
}
trap cleanup EXIT

# Waits until the log holds $1 status lines (those that end a compile), or WAIT_S passes.
status_lines() { grep -c 'page(s)' "$LOG" 2>/dev/null || true; }
wait_for() {
	local want="$1" t=0
	while [ "$(status_lines)" -lt "$want" ]; do
		if ! kill -0 "$PID" 2>/dev/null; then
			echo "watch_edit: the watch ended before status line $want" >&2
			tail -5 "$LOG" >&2
			return 1
		fi
		if [ "$t" -ge $((WAIT_S * 20)) ]; then
			echo "watch_edit: no status line $want within ${WAIT_S}s" >&2
			return 1
		fi
		sleep 0.05
		t=$((t + 1))
	done
}

"$BIN" watch "$DOC" --set view.open=false --set "output=$OUT" "$@" > "$LOG" 2>&1 &
PID=$!
wait_for 1 || exit 1
# Appended after a pause, so that the document's time differs from the build's reads. The first PDF is
# kept as the reference for "a new PDF is in place": a later file time than it is the swap.
sleep 1
cp -p "$OUT" "$SCRATCH/first.pdf"
SAVED=$EPOCHREALTIME
printf 'x' >> "$DOC"
t=0
until [[ "$OUT" -nt "$SCRATCH/first.pdf" ]]; do
	if [ "$t" -ge $((WAIT_S * 200)) ]; then
		echo "watch_edit: no new PDF within ${WAIT_S}s" >&2
		exit 1
	fi
	sleep 0.005
	t=$((t + 1))
done
SEEN=$EPOCHREALTIME
wait_for 2 || exit 1
sleep 0.2

LINE="$(grep 'page(s)' "$LOG" | sed -n 2p)"
VIEW="$(awk -v s="$SAVED" -v e="$SEEN" 'BEGIN { printf "%.3f", e - s }')"
SECS="$(echo "$LINE" | sed -n 's/.* page(s), \([0-9.]*\)s,.*/\1/p')"
PASSES="$(echo "$LINE" | sed -n 's/.*, \([0-9]*\) pass.*/\1/p')"
MODE="$(echo "$LINE" | sed -n 's/.*s, \(warm\|cold\), .*/\1/p')"
echo "mode=$MODE compile_s=$SECS passes=$PASSES save_to_view_s=$VIEW"
echo "line: $LINE"
