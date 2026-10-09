#!/usr/bin/env bash
# The whole-corpus sweep (D-20261009-10): every document in real_corpus.txt is compiled by Austenite and by
# typst 0.15.1, one at a time, each run under its own wall-time and memory cap, and one JSONL record is
# written per document (rec.py). A crash, hang or out-of-memory is recorded and the sweep moves on. Each
# document's outputs are deleted once measured; only the JSONL is kept. The sweep resumes: a document already
# in the JSONL is skipped, so a killed sweep is simply started again.
#
# Run it as ONE heavy job, never beside another document run:
#   ~/usr/code/bash/rc-build 3G --io -- tools/bench/corpus/sweep.sh
#
# Per document: the first run of each engine is a warm-up of the disk cache and, for a Universe template,
# its download (`typst init`, then one typst compile that fetches its imports). Then RUNS alternating
# Austenite/typst runs (typst `-j 1 --ignore-system-fonts`, the same fonts on both sides), one typst run under
# `--timings` for its stage split (skipped above TRACE_MAX_S), and for a document (not a fixture) the warm
# one-letter edit through watch_edit.sh. Fixtures and the crate's own tree files get one run each and no edit.
# Before each document the 1-minute load average must be under LOADMAX; the sweep waits up to LOADWAIT seconds
# for it, then measures anyway and records the load, so report.py can name the runs to repeat.
#
# Environment (defaults): W (~/.cache/austenite/claude-rc-2/spd/corp), BIN (~/.local/bin/austenite), TYPST
# (~/bin/typst), LIST (real_corpus.txt beside this script), OUT ($W/sweep.jsonl), RUNS (3), TMO (120 s per
# run), MEM (3G per run), LOADMAX (6), LOADWAIT (120), TRACE_MAX_S (20), TIERS (all; e.g. "universe own"),
# ONLY (a file of ids to run, one per line; such a run ignores the resume skip), AUST_INV_ROOT
# (~/usr/books/ontheism), AUST_HEM_DIR (~/usr/complement/projects/oxedyne/doc/Hematite).
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE="$(cd "$HERE/../../.." && pwd)"
REPO="$(cd "$CRATE/.." && pwd)"
W="${W:-$HOME/.cache/austenite/claude-rc-2/spd/corp}"
BIN="${BIN:-$HOME/.local/bin/austenite}"
TYPST="${TYPST:-$HOME/bin/typst}"
LIST="${LIST:-$HERE/real_corpus.txt}"
OUT="${OUT:-$W/sweep.jsonl}"
RUNS="${RUNS:-3}"
TMO="${TMO:-120}"
MEM="${MEM:-3G}"
LOADMAX="${LOADMAX:-6}"
LOADWAIT="${LOADWAIT:-120}"
TRACE_MAX_S="${TRACE_MAX_S:-20}"
TIERS="${TIERS:-universe own syn tree fixture}"
ONLY="${ONLY:-}"
AUST_INV_ROOT="${AUST_INV_ROOT:-$HOME/usr/books/ontheism}"
AUST_HEM_DIR="${AUST_HEM_DIR:-$HOME/usr/complement/projects/oxedyne/doc/Hematite}"
export TYPST_PACKAGE_CACHE_PATH="${TYPST_PACKAGE_CACHE_PATH:-$W/pkgs}"

mkdir -p "$W/src" "$W/run" "$TYPST_PACKAGE_CACHE_PATH"
touch "$OUT"
LOG="$W/sweep.log"

log() { printf '%s %s\n' "$(date +%T)" "$*" >> "$LOG"; }

load1() { cut -d' ' -f1 /proc/loadavg; }

wait_load() {
	local t=0
	while awk -v l="$(load1)" -v m="$LOADMAX" 'BEGIN { exit !(l > m) }'; do
		[ "$t" -ge "$LOADWAIT" ] && return 0
		sleep 10
		t=$((t + 10))
	done
}

# capped <prefix> <command...>: one run under its own memory scope and wall-time cap. Writes <prefix>.rc,
# .wall (seconds, from tick.sh), .time (wall user sys maxrss_kb; /usr/bin/time wraps the timeout, inside the memory cap), .out, .err and .load.
capped() {
	local pfx="$1"
	shift
	load1 > "$pfx.load"
	systemd-run --user --scope --quiet -p MemoryMax="$MEM" -p MemorySwapMax=0 --slice=claude-rc.slice \
		/usr/bin/time -o "$pfx.time" -f '%e %U %S %M' timeout -k 5 "$TMO" "$HERE/tick.sh" "$pfx.wall" "$@" > "$pfx.out" 2> "$pfx.err"
	echo $? > "$pfx.rc"
}

has_fonts() { [ -n "$(find "$1" -maxdepth 3 \( -iname '*.ttf' -o -iname '*.otf' -o -iname '*.ttc' \) -print -quit 2>/dev/null)" ]; }

# stage <tier> <id> <spec> <extra>: sets SRC, ROOT, FONTS (may be empty) and EDIT (1 when the warm edit
# runs, on a copy this script owns). Returns non-zero, with STAGE_ERR, when the document cannot be had.
stage() {
	local tier="$1" id="$2" spec="$3" extra="$4" d
	SRC="" ROOT="" FONTS="" EDIT=0 STAGE_ERR=""
	case "$tier" in
		universe)
			d="$W/src/universe/${id/:/-}"
			if [ ! -f "$d/.staged" ]; then
				rm -rf "${d:?}"
				mkdir -p "$W/src/universe"
				if ! timeout 120 "$TYPST" init "@preview/$id" "$d" > /dev/null 2> "$R/fetch.err"; then
					STAGE_ERR="typst init: $(tail -c 300 "$R/fetch.err" | tr '\n' ' ')"
					rm -rf "${d:?}"
					return 1
				fi
				touch "$d/.staged"
			fi
			SRC="$d/$spec" ROOT="$d" EDIT=1
			has_fonts "$d" && FONTS="$d"
			;;
		syn)
			d="$W/src/syn"
			mkdir -p "$d"
			SRC="$d/$id.typ" ROOT="$d" EDIT=1
			if [ ! -s "$SRC" ]; then
				case "$spec" in
					gen:*)	python3 -I "$CRATE/tools/bench/gen_synthetic.py" "${spec#gen:}" "$SRC" > /dev/null ;;
					heap:*)	node "$CRATE/tools/heap_probe.mjs" --dump "${spec#heap:}" > "$SRC" ;;
				esac
			fi
			[ -s "$SRC" ] || { STAGE_ERR="cannot generate $spec"; return 1; }
			;;
		own)
			d="$W/src/own"
			case "$spec" in
				invitation)
					[ -d "$AUST_INV_ROOT/TheOrder/Invitation" ] || { STAGE_ERR="no $AUST_INV_ROOT"; return 1; }
					if [ ! -f "$d/ontheism/.staged" ]; then
						mkdir -p "$d/ontheism/TheOrder" "$d/ontheism/assets"
						rsync -a --exclude '*.pdf' "$AUST_INV_ROOT/TheOrder/Invitation" "$d/ontheism/TheOrder/"
						rsync -a "$AUST_INV_ROOT/assets/fonts" "$d/ontheism/assets/"
						touch "$d/ontheism/.staged"
					fi
					SRC="$d/ontheism/TheOrder/Invitation/invitation.typ" ROOT="$d/ontheism"
					FONTS="$d/ontheism/assets/fonts" EDIT=1
					;;
				hematite)
					[ -d "$AUST_HEM_DIR" ] || { STAGE_ERR="no $AUST_HEM_DIR"; return 1; }
					if [ ! -f "$d/Hematite/.staged" ]; then
						mkdir -p "$d"
						rsync -aL --exclude '*.pdf' --exclude archive --exclude dev "$AUST_HEM_DIR" "$d/"
						touch "$d/Hematite/.staged"
					fi
					SRC="$d/Hematite/hematite.typ" ROOT="$d/Hematite" EDIT=1
					;;
			esac
			;;
		tree|fixture)
			SRC="$REPO/$spec" ROOT="$(dirname "$SRC")"
			;;
	esac
	[ -f "$SRC" ] || { STAGE_ERR="no source $SRC"; return 1; }
}

aus_cmd() {
	local o="$1" j="$2"
	AUS=("$BIN" --eval --timings "$j" --root "$ROOT")
	[ -n "$FONTS" ] && AUS+=(--font-path "$FONTS")
	AUS+=("$SRC" "$o")
}

typ_cmd() {
	local o="$1"
	shift
	TYP=("$TYPST" compile -j 1 --ignore-system-fonts --root "$ROOT")
	[ -n "$FONTS" ] && TYP+=(--font-path "$FONTS")
	TYP+=("$@" "$SRC" "$o")
}

measure() {
	local tier="$1" id="$2" spec="$3" extra="$4" runs="$RUNS" i
	case "$tier" in tree|fixture) runs=1 ;; esac
	if [ "$tier" = universe ]; then
		# The warm-up compile fetches the template's imports; typst may need the network for it.
		typ_cmd "$R/warm.pdf"
		capped "$R/fetch" "${TYP[@]}"
		rm -f "$R/warm.pdf"
	fi
	# Warm the disk cache for both engines (not recorded).
	aus_cmd "$R/aw" "$R/aw.json"
	capped "$R/aw" "${AUS[@]}"
	rm -rf "${R:?}/aw" "$R/aw.json"
	for i in $(seq 1 "$runs"); do
		aus_cmd "$R/ao" "$R/a$i.json"
		capped "$R/a$i" "${AUS[@]}"
		rm -rf "${R:?}/ao"
		typ_cmd "$R/t.pdf"
		capped "$R/t$i" "${TYP[@]}"
		if [ "$i" = "$runs" ] && [ -f "$R/t.pdf" ]; then
			pdfinfo "$R/t.pdf" 2>/dev/null | awk '/^Pages:/ { print $2 }' > "$R/t.pages"
		fi
		rm -f "$R/t.pdf"
	done
	case "$tier" in tree|fixture) return 0 ;; esac
	if [ "$(cat "$R/t1.rc" 2>/dev/null)" = 0 ] && awk -v w="$(cut -d' ' -f1 "$R/t1.time")" -v m="$TRACE_MAX_S" 'BEGIN { exit !(w <= m) }'; then
		typ_cmd "$R/t.pdf" --timings "$R/tt.json"
		capped "$R/tt" "${TYP[@]}"
		rm -f "$R/t.pdf"
	fi
	if [ "$EDIT" = 1 ] && [ "$(cat "$R/a1.rc" 2>/dev/null)" = 0 ]; then
		local set=(--set "root=$ROOT")
		[ -n "$FONTS" ] && set+=(--set "fonts=$FONTS")
		# Three waits of WAIT_S inside one cap, so the watch is never cut off with its child alive.
		SCRATCH="$R/w" WAIT_S=60 TMO=220 capped "$R/w" "$CRATE/tools/bench/watch_edit.sh" "$BIN" "$SRC" "${set[@]}"
		rm -rf "${R:?}/w"
	fi
}

done_ids() { python3 -I -c '
import json, sys
for l in open(sys.argv[1]):
	try: print(json.loads(l)["id"])
	except Exception: pass' "$OUT"; }

declare -A SKIP=()
declare -A WANT=()
if [ -n "$ONLY" ]; then
	while read -r x; do [ -n "$x" ] && WANT["$x"]=1; done < "$ONLY"
else
	while read -r x; do SKIP["$x"]=1; done < <(done_ids)
fi

log "sweep start: list $LIST, out $OUT, runs $RUNS, bin $BIN ($(sha256sum "$BIN" | cut -c1-12)), $(${TYPST} --version)"
n=0
while IFS=$'\t' read -r tier id spec extra rest; do
	case "$tier" in ''|'#'*) continue ;; esac
	case " $TIERS " in *" $tier "*) ;; *) continue ;; esac
	if [ -n "$ONLY" ]; then
		[ -n "${WANT[$id]:-}" ] || continue
	else
		[ -n "${SKIP[$id]:-}" ] && continue
	fi
	n=$((n + 1))
	R="$W/run/cur"
	rm -rf "${R:?}"
	mkdir -p "$R"
	wait_load
	t0=$EPOCHREALTIME
	meta="$(python3 -I -c 'import json,sys; print(json.dumps({"id":sys.argv[1],"tier":sys.argv[2],"spec":sys.argv[3],"extra":sys.argv[4],"licence":sys.argv[5]}))' "$id" "$tier" "$spec" "${extra:-}" "${rest:-}")"
	if stage "$tier" "$id" "$spec" "${extra:-}"; then
		measure "$tier" "$id" "$spec" "${extra:-}"
		runs="$RUNS"
		case "$tier" in tree|fixture) runs=1 ;; esac
		python3 -I "$HERE/rec.py" "$R" "$runs" "$meta" >> "$OUT" || log "rec failed: $id"
	else
		python3 -I -c 'import json,sys; m=json.loads(sys.argv[1]); m["stage_err"]=sys.argv[2]; print(json.dumps(m))' "$meta" "$STAGE_ERR" >> "$OUT"
	fi
	log "$n $tier $id $(awk -v s="$t0" -v e="$EPOCHREALTIME" 'BEGIN { printf "%.1fs", e - s }')"
	rm -rf "${R:?}"
done < "$LIST"
log "sweep end: $n documents"
