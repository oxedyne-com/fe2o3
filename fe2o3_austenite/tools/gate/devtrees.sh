#!/usr/bin/env bash
# Engine survey of one tree: each main compiled by typst 0.15.1, by Austenite's curated reader (what the
# dev loop's `--watch` runs today) and by Austenite's evaluator (`--eval`, what it would run), with the
# exit status, the page count and the diagnostic counts of each, and nothing of the tree. Generic: it takes
# the tree as arguments and knows no book.
#
#     devtrees.sh <tree-root> <main>...
#
#   <tree-root>  the tree's root, which is `--root` and the parent of `assets/fonts`
#   <main>       an entry file, relative to <tree-root> or absolute; each is compiled from its own
#                directory under its bare file name, as `./dev` does
#
# Environment: AUSTENITE_BIN (the austenite binary under test; required), TYPST_BIN (default
# /home/jason/bin/typst, which must be 0.15.1), DEVTREES_INDEX (digits, default 0: the tree's number in the
# line a caller prints for each tree), DEVTREES_TIMEOUT (seconds an engine may run, default 1800), GATE_CAP
# (the memory cap of each engine, default 3G). Every engine runs under the cap with the account's own fonts
# hidden (`gate_lib.sh`).
#
# Engines, per main:
#   typst      typst compile --root R --font-path R/assets/fonts <main> <pdf>
#   curated    austenite <main> <dir>
#   eval       austenite --eval --strict --diag-summary --root R --font-path R/assets/fonts <main> <dir>
#   eval-lax   the same without --strict, run only after a strict run that failed: the dev loop is not
#              strict, so what it would build and in how many pages is the datum
#
# [bio] by construction. Every PDF lives in a directory made under ~/.cache/austenite/devtrees and removed
# on exit; the stdout of every engine is dropped; the stderr of every engine goes only through an awk that
# keeps two counts (typst: its `error:` and `warning:` lines; curated: its `: error:`, `: warning:` and
# `Error:` lines; eval: the sums of its `diag-summary` lines, which carry no text of the document); the page
# count is read from the PDF by pdfinfo and is a number or `none`; and the whole of stdout and stderr of
# this script passes a final whitelist, so the only lines that can appear are these, with N, M, E, X and Y
# numbers (E is the exit status, 124 when the engine timed out) and P a number or `none`:
#
#   tree N main M typst exit E pages P error X warning Y
#   tree N main M curated exit E pages P error X warning Y
#   tree N main M eval exit E pages P error X warning Y
#   tree N main M eval-lax exit E pages P error X warning Y
#   tree N missing-root
#   tree N main M missing-main
#   devtrees: typst is not 0.15.1
#   devtrees: no austenite binary
#   devtrees: usage
#   devtrees: no scratch
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=gate_lib.sh
. "$HERE/gate_lib.sh"

# A page count: a number or `none`.
pages_of() {
	local n
	n="$(pdfinfo "$1" 2>/dev/null | awk '/^Pages:/ { print $2 }')"
	case "$n" in
		''|*[!0-9]*)	echo none;;
		*)				echo "$n";;
	esac
}

# The counters, each reading an engine's stderr on stdin and printing `<errors> <warnings>`.
count_typst() {
	awk '
		/^error: /		{ e++ }
		/^warning: /	{ w++ }
		END { printf "%d %d\n", e + 0, w + 0 }'
}

count_curated() {
	awk '
		/: error: /		{ e++; next }
		/: warning: /	{ w++; next }
		/^Error: /		{ e++ }
		END { printf "%d %d\n", e + 0, w + 0 }'
}

count_eval() {
	awk '
		/^diag-summary error [a-z-]+ [^ ]+ [0-9]+$/	{ e += $NF; next }
		/^diag-summary warning [a-z-]+ [^ ]+ [0-9]+$/	{ w += $NF }
		END { printf "%d %d\n", e + 0, w + 0 }'
}

# engine <label> <counter> <pdf> <command> <args>...: one engine from the main's directory, then its line.
# The status of the engine is left in RC.
engine() {
	local label="$1" counter="$2" pdf="$3" x=0 y=0
	shift 3
	rm -rf "${W:?}/o"
	mkdir -p "$W/o"
	( cd "$DIR" && gate_hidden timeout -k 10 "$TO" "$@" ) 2>&1 >/dev/null | "count_$counter" > "$W/counts"
	RC="${PIPESTATUS[0]}"
	read -r x y < "$W/counts"
	case "$x" in ''|*[!0-9]*) x=0;; esac
	case "$y" in ''|*[!0-9]*) y=0;; esac
	echo "tree $IDX main $MI $label exit $RC pages $(pages_of "$pdf") error $x warning $y"
	rm -rf "${W:?}/o"
}

main() {
	local base="$HOME/.cache/austenite/devtrees"
	IDX="${DEVTREES_INDEX:-0}"
	TO="${DEVTREES_TIMEOUT:-1800}"
	case "$IDX$TO" in *[!0-9]*) echo "devtrees: usage"; return 2;; esac
	if [ $# -lt 2 ] || [ -z "$IDX" ] || [ -z "$TO" ]; then
		echo "devtrees: usage"
		return 2
	fi
	local typst="${TYPST_BIN:-/home/jason/bin/typst}" aus="${AUSTENITE_BIN:-}"
	if ! timeout 60 "$typst" --version 2>/dev/null | awk '/^typst 0\.15\.1( |$)/ { ok = 1 } END { exit !ok }'; then
		echo "devtrees: typst is not 0.15.1"
		return 2
	fi
	if [ -z "$aus" ] || [ ! -f "$aus" ] || [ ! -x "$aus" ]; then
		echo "devtrees: no austenite binary"
		return 2
	fi
	aus="$(cd "$(dirname "$aus")" && pwd)/$(basename "$aus")"
	local root
	root="$(cd "$1" 2>/dev/null && pwd)" || { echo "tree $IDX missing-root"; return 1; }
	shift
	mkdir -p "$base" || { echo "devtrees: no scratch"; return 2; }
	# Global, not local: the trap runs after this function has returned.
	W="$(mktemp -d "$base/run.XXXXXX")" || { echo "devtrees: no scratch"; return 2; }
	trap 'rm -rf "${W:?}"' EXIT
	GATE_SCRATCH="$W"

	local font="$root/assets/fonts" m path strict
	MI=0
	for m in "$@"; do
		MI=$((MI + 1))
		case "$m" in
			/*)	path="$m";;
			*)	path="$root/$m";;
		esac
		if [ ! -f "$path" ]; then
			echo "tree $IDX main $MI missing-main"
			continue
		fi
		DIR="$(dirname "$path")"
		m="$(basename "$path")"
		engine typst   typst   "$W/o/typst.pdf"           "$typst" compile --root "$root" --font-path "$font" "$m" "$W/o/typst.pdf"
		engine curated curated "$W/o/curated/document.pdf" "$aus" "$m" "$W/o/curated"
		engine eval    eval    "$W/o/eval/document.pdf"   "$aus" --eval --strict --diag-summary --root "$root" --font-path "$font" "$m" "$W/o/eval"
		strict="$RC"
		if [ "$strict" != 0 ]; then
			engine eval-lax eval "$W/o/eval/document.pdf" "$aus" --eval --diag-summary --root "$root" --font-path "$font" "$m" "$W/o/eval"
		fi
	done
	return 0
}

# The final whitelist, over everything main and its children wrote to either stream: the message of a tool
# that failed, a shell's note of a process it lost, anything but these shapes is dropped.
WHITELIST='
	/^tree [0-9]+ main [0-9]+ (typst|curated|eval|eval-lax) exit [0-9]+ pages (none|[0-9]+) error [0-9]+ warning [0-9]+$/ { print; next }
	/^tree [0-9]+ missing-root$/ { print; next }
	/^tree [0-9]+ main [0-9]+ missing-main$/ { print; next }
	/^devtrees: (typst is not 0\.15\.1|no austenite binary|usage|no scratch)$/ { print; next }'
main "$@" 2>&1 | awk "$WHITELIST"
exit "${PIPESTATUS[0]}"
