#!/usr/bin/env bash
# The gate on one project: Typst 0.15.1 is the oracle, Austenite is compiled strict through the command line
# and through the wasm door, and the PDFs are compared by G1 to G12 (`g_checks.py`). Generic: it takes the
# project as arguments and knows no book.
#
#     gate.sh <root> <main> <fontdir> <austenite-bin> <pkg>
#
#   <root>          the project root: typst's `--root`, the CLI's `--root` and the door's gather root
#   <main>          the entry file, relative to <root>
#   <fontdir>       the project's font directory, relative to <root> or absolute
#   <austenite-bin> the `austenite` binary built from the tree under test
#   <pkg>           the wasm-pack output directory of the same tree
#
# Environment: GATE_TYPST (the typst binary, default /home/jason/bin/typst), GATE_OUT (the scratch parent,
# default ~/.cache/austenite-o4-probe/out), GATE_EXCLUDE (directories under <root> the door's gather leaves
# out, space separated), GATE_PACKAGES (a directory of packages the door's gather supplies through
# `supplyPackage`, as `tools/bench/lib/packages.mjs` describes; the CLI keeps resolving from Typst's own
# cache), GATE_CAP (the memory cap of each run, default 3G).
#
# What runs, in order, every run under the cap and with the account's own fonts hidden:
#   typst compile; `austenite --eval --strict --diag-summary` twice; `door_pdf.mjs` twice, the first
#   compiling twice on one instance; then G1, G2, G3, G4, G5, G7, G8 and G9 for the CLI's PDF and for the door's
#   against Typst's (the door's only when it is not the CLI's byte for byte), and G12 over the five Austenite
#   PDFs.
#
# [bio] by construction. Every PDF and raster lives in a directory made under GATE_OUT and removed on exit; page
# text is read from a pipe inside g_checks.py and never written; the CLI's stdout (it names the paths) goes
# nowhere; the stderr of every program goes only through an awk filter that keeps counts or a closed
# vocabulary; and the whole of stdout passes a final whitelist, so the only lines that can appear are:
#
#   gate: error <usage|no-root|no-main|no-tool|scratch|typst-failed>
#   gate: typst exit <n>
#   gate: typst diagnostics error <n> warning <n>
#   gate: typst pages <n|none>
#   gate: cli exit <n>
#   gate: cli severity error <n> warning <n>
#   gate: cli diag-summary <error|warning> <kind> <construct|-> <n>
#   gate: cli diag-error <kind> callee:<name> expected:<type> found:<type> file:<main|sibling|other>
#   gate: cli pages <n|none>
#   gate: door <ok|error> pages <n|none> kinds <none|kind:n,...> needs <n>
#   gate: door again <ok|error>
#   gate: door fail <bad-args|bad-pkg|no-root|no-main|no-packages|bad-package|too-large|supply|failed>
#   gate: door pages <n|none>
#   gate: <cli|door> checks skipped
#   gate: door same-as-cli          the door's PDF equals the CLI's byte for byte, so it has the CLI's verdicts
#   gate: <cli|door> g1 .. g9       every line of `g_checks.py` for that check, as its docstring lists them
#   gate: g12 ...                   the lines of `g_checks.py g12`
#   gate: table <cli|door> <g1|g2|g3|g4|g5|g7|g8|g9> <green|red|measured>
#   gate: table g12 <green|red>
#   gate: verdict <green|red>      green when every required check is green and both compiles succeeded
#   gate: suppressed <n>           lines a program printed that the whitelist dropped
#
# G3, G7 and G9 are measured and are `measured` in the table; the others are required. The exit status is 0 on
# `verdict green`, 1 on `verdict red`, 2 on an `error`.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=gate_lib.sh
. "$HERE/gate_lib.sh"

# The lines of a program's stderr, kept as counts or a closed vocabulary (the CLI's).
cli_filter() {
	awk '
		/^diag-summary (error|warning) [a-z_]+ ([#A-Za-z0-9_.:-]+|-) [0-9]+$/ { print "gate: cli " $0; next }
		/^diag-error [a-z_]+ callee:[a-z0-9.-]+ expected:[a-z-]+ found:[a-z-]+ file:(main|sibling|other)$/ { print "gate: cli " $0; next }
		/: error: /   { e++; next }
		/: warning: / { w++; next }
		END { printf "gate: cli severity error %d warning %d\n", e + 0, w + 0 }'
}

# Typst's short-format diagnostics on stdin, as counts.
typst_filter() {
	awk '
		/: error: / || /^error: /     { e++; next }
		/: warning: / || /^warning: / { w++; next }
		END { printf "gate: typst diagnostics error %d warning %d\n", e + 0, w + 0 }'
}

# The door's own lines; anything else is counted.
door_filter() {
	awk '
		/^door (ok|error) pages (none|[0-9]+) kinds (none|[a-z_]+:[0-9]+(,[a-z_]+:[0-9]+)*) needs [0-9]+$/ { print "gate: " $0; next }
		/^door again (ok|error)$/ { print "gate: " $0; next }
		/^door fail (bad-args|bad-pkg|no-root|no-main|no-packages|bad-package|too-large|supply|failed)$/ { print "gate: " $0; next }
		{ n++ }
		END { if (n) printf "gate: suppressed %d\n", n }'
}

pages_of() {
	local n
	n="$(pdfinfo "$1" 2>/dev/null | awk '/^Pages:/ { print $2 }')"
	echo "${n:-none}"
}

# Verdicts of the checks, by "<row> <check>", for the table and the exit status.
declare -A V

# run_g <row> <check> <typst.pdf> <austenite.pdf>: the lines of one check, prefixed with its row.
run_g() {
	local row="$1" chk="$2" out
	out="$(python3 "$HERE/g_checks.py" "$chk" "$3" "$4" 2>/dev/null)"
	printf '%s\n' "$out" | awk -v row="$row" 'NF { print "gate: " row " " $0 }'
	V["$row $chk"]="$(printf '%s\n' "$out" | awk -v c="$chk" '$1 == c && $2 == "verdict" { v = $3 } $1 == c && $2 == "error" { v = "error" } END { print v }')"
}

main() {
	if [ $# -ne 5 ]; then
		echo "gate: error usage"
		return 2
	fi
	local R="$1" M="$2" F="$3" BIN="$4" PKG="$5"
	local TYPST="${GATE_TYPST:-/home/jason/bin/typst}"
	local BASE="${GATE_OUT:-$HOME/.cache/austenite-o4-probe/out}"
	local ROOT
	ROOT="$(cd "$R" 2>/dev/null && pwd)" || { echo "gate: error no-root"; return 2; }
	[ -f "$ROOT/$M" ] || { echo "gate: error no-main"; return 2; }
	[ -x "$BIN" ] && [ -x "$TYPST" ] || { echo "gate: error no-tool"; return 2; }
	mkdir -p "$BASE" || { echo "gate: error scratch"; return 2; }
	# Global, not local: the trap runs after this function has returned.
	W="$(mktemp -d "$BASE/gate.XXXXXX")" || { echo "gate: error scratch"; return 2; }
	trap 'rm -rf "${W:?}"' EXIT
	export GATE_SCRATCH="$W"

	local dex=() d
	for d in ${GATE_EXCLUDE:-}; do
		dex+=(--exclude "$d")
	done
	if [ -n "${GATE_PACKAGES:-}" ]; then
		dex+=(--packages "$GATE_PACKAGES")
	fi

	# Typst, the oracle. Run from the root so a relative <fontdir> and <main> resolve as they do for the CLI.
	local te
	( cd "$ROOT" && gate_typst "$TYPST" compile --diagnostic-format short --root "$ROOT" \
		--font-path "$F" "$M" "$W/typst.pdf" ) 2>&1 >/dev/null | typst_filter
	te=${PIPESTATUS[0]}
	echo "gate: typst exit $te"
	echo "gate: typst pages $(pages_of "$W/typst.pdf")"
	if [ ! -s "$W/typst.pdf" ]; then
		echo "gate: error typst-failed"
		return 2
	fi

	# Austenite's command line, strict, twice. The first run's stderr is the report; the second is for G12.
	local ce
	( cd "$ROOT" && gate_hidden "$BIN" --eval --strict --diag-summary --root "$ROOT" --font-path "$F" \
		"$M" "$W/cli1" ) 2>&1 >/dev/null | cli_filter
	ce=${PIPESTATUS[0]}
	echo "gate: cli exit $ce"
	( cd "$ROOT" && gate_hidden "$BIN" --eval --strict --diag-summary --root "$ROOT" --font-path "$F" \
		"$M" "$W/cli2" ) >/dev/null 2>&1
	echo "gate: cli pages $(pages_of "$W/cli1/document.pdf")"

	# The door, in node: the first process compiles twice on one instance, the second once on another.
	local de
	gate_cap node "$HERE/door_pdf.mjs" "$PKG" "$ROOT" "$M" "$F" "$W/door1.pdf" --again "$W/door2.pdf" \
		"${dex[@]}" 2>&1 | door_filter
	de=${PIPESTATUS[0]}
	gate_cap node "$HERE/door_pdf.mjs" "$PKG" "$ROOT" "$M" "$F" "$W/door3.pdf" "${dex[@]}" >/dev/null 2>&1
	echo "gate: door pages $(pages_of "$W/door1.pdf")"

	local row pdf g
	for row in cli door; do
		if [ "$row" = cli ]; then pdf="$W/cli1/document.pdf"; else pdf="$W/door1.pdf"; fi
		if [ ! -s "$pdf" ]; then
			echo "gate: $row checks skipped"
			continue
		fi
		# The door's PDF, byte for byte the CLI's, has the CLI's verdicts: the checks are not run twice.
		if [ "$row" = door ] && cmp -s "$W/cli1/document.pdf" "$pdf"; then
			echo "gate: door same-as-cli"
			for g in g1 g2 g3 g4 g5 g7 g8 g9; do
				V["door $g"]="${V["cli $g"]:-}"
			done
			continue
		fi
		for g in g1 g2 g3 g4 g5 g7 g8 g9; do
			run_g "$row" "$g" "$W/typst.pdf" "$pdf"
		done
	done
	local out12
	out12="$(python3 "$HERE/g_checks.py" g12 "$W/cli1/document.pdf" "$W/cli2/document.pdf" \
		"$W/door1.pdf" "$W/door2.pdf" "$W/door3.pdf" 2>/dev/null)"
	printf '%s\n' "$out12" | awk 'NF { print "gate: " $0 }'
	V["all g12"]="$(printf '%s\n' "$out12" | awk '$1 == "g12" && $2 == "verdict" { v = $3 } $1 == "g12" && $2 == "error" { v = "error" } END { print v }')"

	# The table. A required check is green on `same` and red on anything else, a missing verdict included.
	local red=0 v
	[ "$ce" -eq 0 ] || red=1
	[ "$de" -eq 0 ] || red=1
	for row in cli door; do
		for g in g1 g2 g3 g4 g5 g7 g8 g9; do
			case "$g" in
				g3|g7|g9)	echo "gate: table $row $g measured" ;;
				*)
					v="${V["$row $g"]:-}"
					if [ "$v" = same ]; then
						echo "gate: table $row $g green"
					else
						echo "gate: table $row $g red"
						red=1
					fi ;;
			esac
		done
	done
	if [ "${V["all g12"]:-}" = same ]; then
		echo "gate: table g12 green"
	else
		echo "gate: table g12 red"
		red=1
	fi
	if [ "$red" -eq 0 ]; then echo "gate: verdict green"; return 0; fi
	echo "gate: verdict red"
	return 1
}

# The final whitelist, over everything main and its children wrote to either stream.
main "$@" 2>&1 | awk '
	function ok(re) { return $0 ~ ("^gate: " re "$") }
	BEGIN {
		N = "[0-9]+"; D2 = "-?[0-9]+\\.[0-9][0-9]"; D = "[0-9]+\\.[0-9][0-9][0-9][0-9]"; S = "[0-9]\\.[0-9][0-9][0-9]"
		R = "(none|[0-9]+(-[0-9]+)?(,[0-9]+(-[0-9]+)?)*)"
		K = "[a-z_]+"
		row = "(cli|door) "
		g[1]  = "g1 pages a " N " b " N
		g[2]  = "g1 mediabox differ " N " pages " R
		g[3]  = "g1 mediabox max-deviation " D
		g[4]  = "g1 verdict (same|differs)"
		g[5]  = "g2 fonts a " N " b " N
		g[6]  = "g2 only-[ab] [A-Za-z0-9+-]+"
		g[7]  = "g2 only-[ab] other"
		g[8]  = "g2 flags a " N " b " N
		g[9]  = "g2 verdict (same|differs)"
		g[10] = "g3 pages a " N " b " N
		g[11] = "g3 differ " N " pages " R
		g[12] = "g3 min-similarity (none|" S ")"
		g[13] = "g3 verdict (same|differs)"
		g[14] = "g4 outline a " N " b " N
		g[15] = "g4 levels (equal|differs) first-diff (none|" N ")"
		g[16] = "g4 titles (equal|differs) unmatched a " N " b " N
		g[17] = "g4 titles position differ " N " of " N
		g[18] = "g4 titles prefix-only " N
		g[19] = "g4 verdict (same|differs)"
		g[20] = "g5 (title|author|subject|keywords) (equal|differs|absent-both)"
		g[21] = "g5 verdict (same|differs)"
		g[22] = "g7 compared " N " of a " N " b " N
		g[23] = "g7 page " N " " D
		g[24] = "g7 max (none|" D ")"
		g[25] = "g7 median (none|" D ")"
		g[26] = "g8 pages a " N " b " N
		g[27] = "g8 out-of-tolerance " N " pages " R
		g[28] = "g8 page " N " a " N " b " N
		g[29] = "g8 max-deviation " D
		g[30] = "g8 empty " N " pages " R
		g[31] = "g8 verdict (same|differs)"
		g[32] = "g[1234578] error (unreadable|usage|scratch)"
		g[33] = "g9 pages differ " N " pages " R
		g[34] = "g9 page " N " (lines|blocks|words|sizes|images|links) a " N " b " N
		g[35] = "g9 page " N " (left|right|top|bottom|pitch|height) a " D2 " b " D2
		g[36] = "g9 line " N " " N " y a " D2 " b " D2 " x a " D2 " b " D2 " w a " D2 " b " D2
		g[37] = "g9 error (unreadable|usage)"
		ng = 37
		h[1] = "g12 (cli-twice|door-one-instance|door-two-instances|cli-door) (equal|differs|absent)"
		h[2] = "g12 size cli (none|" N ") door (none|" N ")"
		h[3] = "g12 verdict (same|differs)"
		h[4] = "g12 error (unreadable|usage)"
		nh = 4
	}
	{
		if (ok("error (usage|no-root|no-main|no-tool|scratch|typst-failed)")) { print; next }
		if (ok("typst exit " N) || ok("typst diagnostics error " N " warning " N) || ok("typst pages (none|" N ")")) { print; next }
		if (ok("cli exit " N) || ok("cli severity error " N " warning " N) || ok("cli pages (none|" N ")")) { print; next }
		if (ok("cli diag-summary (error|warning) " K " ([#A-Za-z0-9_.:-]+|-) " N)) { print; next }
		if (ok("cli diag-error " K " callee:[a-z0-9.-]+ expected:[a-z-]+ found:[a-z-]+ file:(main|sibling|other)")) { print; next }
		if (ok("door (ok|error) pages (none|" N ") kinds (none|" K ":" N "(," K ":" N ")*) needs " N)) { print; next }
		if (ok("door again (ok|error)") || ok("door pages (none|" N ")")) { print; next }
		if (ok("door fail (bad-args|bad-pkg|no-root|no-main|no-packages|bad-package|too-large|supply|failed)")) { print; next }
		if (ok("(cli|door) checks skipped") || ok("door same-as-cli")) { print; next }
		for (i = 1; i <= ng; i++) if (ok(row g[i])) { print; next }
		for (i = 1; i <= nh; i++) if (ok(h[i])) { print; next }
		if (ok("table (cli|door) (g1|g2|g3|g4|g5|g7|g8|g9) (green|red|measured)") || ok("table g12 (green|red)")) { print; next }
		if (ok("verdict (green|red)")) { print; next }
		if (ok("suppressed " N)) { print; next }
		n++
	}
	END { if (n) printf "gate: suppressed %d\n", n }'
exit "${PIPESTATUS[0]}"
