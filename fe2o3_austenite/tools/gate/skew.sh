#!/usr/bin/env bash
# Version skew, metrics only: one project compiled by typst.ts 0.14.2 (Daimond's compiler, in node) and by
# typst 0.15.1, and the two PDFs compared page by page (G3, `g_checks.py`). Generic: it takes the project
# as arguments and knows no book.
#
#     skew.sh <root> <main> <fontdir> <typst-vendor>
#
#   <root>         the project root, which is typst's `--root` and the root of the typst.ts gather
#   <main>         the entry file, relative to <root>
#   <fontdir>      the project's font directory, relative to <root> or absolute
#   <typst-vendor> Daimond's `www/vendor/typst` (the compiler and its `fonts/`)
#
# Environment: SKEW_TYPST (the typst binary, default /home/jason/bin/typst), SKEW_OUT (the scratch parent,
# default ~/.cache/austenite-o4-probe/out), SKEW_EXCLUDE (directories under <root> the typst.ts gather
# leaves out, space separated), SKEW_PACKAGES (a directory of packages for the typst.ts side only, as
# `tools/bench/lib/packages.mjs` describes: `<ns>/<name>/<version>/` or Daimond's `<version>.pack`, for
# instance `www/assets/typst/packs`). Typst 0.15.1 keeps resolving packages from its own cache.
#
# Two rows, each typst.ts against a different 0.15.1 run:
#   a  as each engine runs in its own setting: typst.ts with the vendor fonts and <fontdir>; 0.15.1 with
#      `--font-path <fontdir>`, its own embedded fonts and the system's, the account's own hidden
#   b  the same font files for both: 0.15.1 with --ignore-system-fonts --ignore-embedded-fonts
#      --font-path <vendor>/fonts --font-path <fontdir>; this row separates version skew from font skew
#
# [bio] by construction. Every PDF lives in a directory made under SKEW_OUT and removed on exit; the page
# text is read from a pipe inside g_checks.py and never written; the stderr of every program goes only
# through an awk filter that keeps counts or a closed vocabulary; and the whole of stdout passes a final
# whitelist, so the only lines that can appear are:
#
#   skew: typstts gathered sources <n> assets <n> fonts <n> packages <n> missing <n>
#   skew: typstts package preview <name> <version> <supplied|missing>   (the public registry only)
#   skew: typstts compile <ok|failed>
#   skew: typstts error <bad-args|no-root|no-main|no-packages|bad-package|too-large|failed>
#   skew: row <a|b> typst exit <n>
#   skew: row <a|b> typst diagnostics error <n> warning <n>
#   skew: row <a|b> pages typstts <n|none> typst <n|none>
#   skew: row <a|b> differ <n> pages <none|ranges such as 3-5,9>
#   skew: row <a|b> min-similarity <none|d.ddd>
#   skew: row <a|b> verdict <same|differs|incomparable>
#   skew: error <usage|no-root|scratch>
#   skew: suppressed <n>      (lines a program printed that the whitelist dropped)
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=gate_lib.sh
. "$HERE/gate_lib.sh"

main() {
	if [ $# -ne 4 ]; then
		echo "skew: error usage"
		return 2
	fi
	local R="$1" M="$2" F="$3" V="$4"
	local TYPST="${SKEW_TYPST:-/home/jason/bin/typst}"
	local BASE="${SKEW_OUT:-$HOME/.cache/austenite-o4-probe/out}"
	local ROOT
	ROOT="$(cd "$R" 2>/dev/null && pwd)" || { echo "skew: error no-root"; return 2; }
	if [ ! -f "$V/typst_ts_web_compiler.mjs" ] || [ ! -d "$V/fonts" ]; then
		echo "skew: error usage"
		return 2
	fi
	mkdir -p "$BASE" || { echo "skew: error scratch"; return 2; }
	# Global, not local: the trap runs after this function has returned.
	W="$(mktemp -d "$BASE/skew.XXXXXX")" || { echo "skew: error scratch"; return 2; }
	trap 'rm -rf "${W:?}"' EXIT
	GATE_SCRATCH="$W"

	local ex=() d
	for d in ${SKEW_EXCLUDE:-}; do
		ex+=(--exclude "$d")
	done
	if [ -n "${SKEW_PACKAGES:-}" ]; then
		ex+=(--packages "$SKEW_PACKAGES")
	fi

	# typst.ts, in node.
	gate_cap node "$HERE/skew_ts.mjs" --root "$ROOT" --main "$M" --vendor "$V" --out "$W/ts.pdf" \
		--font-dir "$F" "${ex[@]}" 2>&1 | awk '
		/^typstts: gathered sources [0-9]+ assets [0-9]+ fonts [0-9]+ packages [0-9]+ missing [0-9]+$/ { sub(/^typstts: /, "typstts "); print "skew: " $0; next }
		/^typstts: package preview [A-Za-z0-9_-]+ [0-9]+\.[0-9]+\.[0-9]+ (supplied|missing)$/ { sub(/^typstts: /, "typstts "); print "skew: " $0; next }
		/^typstts: compile (ok|failed)$/ { sub(/^typstts: /, "typstts "); print "skew: " $0; next }
		/^typstts: error (bad-args|no-root|no-main|no-packages|bad-package|too-large|failed)$/ { sub(/^typstts: /, "typstts "); print "skew: " $0; next }
		{ n++ }
		END { if (n) printf "skew: suppressed %d\n", n }'

	# The two 0.15.1 runs, each from the project root so that a relative <fontdir> and <main> resolve as
	# they do for `probe.sh`.
	local te
	( cd "$ROOT" && gate_typst "$TYPST" compile --diagnostic-format short --root "$ROOT" \
		--font-path "$F" "$M" "$W/a.pdf" ) 2>&1 >/dev/null | row_diag a
	te=${PIPESTATUS[0]}
	echo "skew: row a typst exit $te"
	( cd "$ROOT" && gate_typst "$TYPST" compile --diagnostic-format short --root "$ROOT" \
		--ignore-system-fonts --ignore-embedded-fonts --font-path "$V/fonts" --font-path "$F" \
		"$M" "$W/b.pdf" ) 2>&1 >/dev/null | row_diag b
	te=${PIPESTATUS[0]}
	echo "skew: row b typst exit $te"

	row_cmp a "$W/ts.pdf" "$W/a.pdf"
	row_cmp b "$W/ts.pdf" "$W/b.pdf"
	return 0
}

# Counts typst's short-format diagnostics on stdin; prints only the two counts.
row_diag() {
	awk -v row="$1" '
		/: error: / || /^error: /     { e++; next }
		/: warning: / || /^warning: / { w++; next }
		END { printf "skew: row %s typst diagnostics error %d warning %d\n", row, e + 0, w + 0 }'
}

pages_of() {
	local n
	n="$(pdfinfo "$1" 2>/dev/null | awk '/^Pages:/ { print $2 }')"
	echo "${n:-none}"
}

# row_cmp <a|b> <typst.ts pdf> <typst pdf>
row_cmp() {
	echo "skew: row $1 pages typstts $(pages_of "$2") typst $(pages_of "$3")"
	if [ ! -s "$2" ] || [ ! -s "$3" ]; then
		echo "skew: row $1 verdict incomparable"
		return
	fi
	python3 "$HERE/g_checks.py" compare "$2" "$3" 2>/dev/null | awk -v row="$1" '
		/^g3 pages / { next }
		/^g3 differ [0-9]+ pages (none|[0-9]+(-[0-9]+)?(,[0-9]+(-[0-9]+)?)*)$/ { sub(/^g3 /, ""); print "skew: row " row " " $0; next }
		/^g3 min-similarity (none|[0-9]\.[0-9][0-9][0-9])$/ { sub(/^g3 /, ""); print "skew: row " row " " $0; next }
		/^g3 verdict (same|differs)$/ { sub(/^g3 /, ""); print "skew: row " row " " $0; next }
		/^g3 error (unreadable|usage)$/ { print "skew: row " row " verdict incomparable"; next }
		{ n++ }
		END { if (n) printf "skew: suppressed %d\n", n }'
}

# The final whitelist, over everything main and its children wrote to either stream.
main "$@" 2>&1 | awk '
	/^skew: typstts gathered sources [0-9]+ assets [0-9]+ fonts [0-9]+ packages [0-9]+ missing [0-9]+$/ { print; next }
	/^skew: typstts package preview [A-Za-z0-9_-]+ [0-9]+\.[0-9]+\.[0-9]+ (supplied|missing)$/ { print; next }
	/^skew: typstts compile (ok|failed)$/ { print; next }
	/^skew: typstts error (bad-args|no-root|no-main|no-packages|bad-package|too-large|failed)$/ { print; next }
	/^skew: row [ab] typst exit [0-9]+$/ { print; next }
	/^skew: row [ab] typst diagnostics error [0-9]+ warning [0-9]+$/ { print; next }
	/^skew: row [ab] pages typstts (none|[0-9]+) typst (none|[0-9]+)$/ { print; next }
	/^skew: row [ab] differ [0-9]+ pages (none|[0-9]+(-[0-9]+)?(,[0-9]+(-[0-9]+)?)*)$/ { print; next }
	/^skew: row [ab] min-similarity (none|[0-9]\.[0-9][0-9][0-9])$/ { print; next }
	/^skew: row [ab] verdict (same|differs|incomparable)$/ { print; next }
	/^skew: error (usage|no-root|scratch)$/ { print; next }
	/^skew: suppressed [0-9]+$/ { print; next }
	{ n++ }
	END { if (n) printf "skew: suppressed %d\n", n }'
exit "${PIPESTATUS[0]}"
