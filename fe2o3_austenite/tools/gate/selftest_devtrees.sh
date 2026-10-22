#!/usr/bin/env bash
# Proves devtrees.sh on synthetic trees, and proves the leak test can fail.
#
#     selftest_devtrees.sh <austenite-bin>
#
# <austenite-bin> is the austenite binary under test (or AUSTENITE_BIN). Scratch is a child of
# ~/.cache/austenite/devtrees, made here and removed on exit. Everything is synthetic: no path to a real tree
# is taken or needed.
#
# It checks, in order:
#   1. a clean tree (a relative import, an image and an include by a root-absolute path, a supplied font), two
#      mains, the second in a subdirectory, and a third that is not there: typst and the evaluator both exit 0
#      and agree on the pages, the curated reader gives a line, and the missing main is named;
#   2. a syntax error: typst and the evaluator exit non-zero, and the strict run's failure is followed by the
#      lax run;
#   3. a font family nobody has: typst warns and exits 0; the evaluator's strict run exits non-zero and its lax
#      run exits 0 with a page;
#   4. a missing root, a binary that is not typst 0.15.1, no austenite binary, no arguments, an engine that
#      times out (124), and the tree's number;
#   5. every line printed anywhere above is one of the whitelist's shapes, and no scratch is left behind;
#   6. the marker word, ZZLEAKMARKERZZ, in a tree's directory and file names, its contents, its errors and the
#      family a font is asked for: the engines' own stderr quotes it (the premise), and nothing of it is
#      printed; a tool of the script's own that fails and quotes a path (mktemp, with a HOME that cannot hold the
#      scratch) is dropped as well; and ten lookalike lines go through the script's own whitelist, one result
#      among them, which is the only one to pass;
#   7. the plant: the script's final whitelist made `cat`, and the stray test goes red.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE="$(cd "$HERE/../.." && pwd)"
# shellcheck source=gate_lib.sh
. "$HERE/gate_lib.sh"

AUS="${1:-${AUSTENITE_BIN:-}}"
[ -n "$AUS" ] && [ -x "$AUS" ] || { echo "usage: selftest_devtrees.sh <austenite-bin>"; exit 2; }
AUS="$(cd "$(dirname "$AUS")" && pwd)/$(basename "$AUS")"
TYPST="${TYPST_BIN:-/home/jason/bin/typst}"
DT="$HERE/devtrees.sh"
PARENT="$HOME/.cache/austenite/devtrees"
mkdir -p "$PARENT" || exit 2
S="$(mktemp -d "$PARENT/selftest.XXXXXX")" || exit 2
trap 'rm -rf "${S:?}"' EXIT
GATE_SCRATCH="$S"
MARK=ZZLEAKMARKERZZ

bad=0
ok()   { echo "selftest: ok   $1"; }
fail() { echo "selftest: FAIL $1"; bad=$((bad + 1)); }
expect() { # expect <label> <text> <fixed line the text must contain>
	if printf '%s\n' "$2" | grep -qxF -- "$3"; then ok "$1"; else fail "$1 (wanted: $3)"; fi
}
expect_re() { # expect_re <label> <text> <extended regex a line must match whole>
	if printf '%s\n' "$2" | grep -qE -- "^($3)\$"; then ok "$1"; else fail "$1 (wanted a line like: $3)"; fi
}
lack() { # lack <label> <text> <fixed string the text must not contain>
	if printf '%s\n' "$2" | grep -qF -- "$3"; then fail "$1 (found: $3)"; else ok "$1"; fi
}

# run <script> <index> <args>...: the survey with both streams, as a caller sees them.
run() {
	local script="$1" idx="$2"
	shift 2
	DEVTREES_INDEX="$idx" AUSTENITE_BIN="$AUS" TYPST_BIN="$TYPST" "$script" "$@" 2>&1
}

# ── the synthetic trees ─────────────────────────────────────────────────────
mktree() { # mktree <dir>
	local t="$1"
	mkdir -p "$t/chap" "$t/img" "$t/sub" "$t/assets/fonts"
	cp "$CRATE/fonts/LibertinusMono-Regular.otf" "$t/assets/fonts/"
	python3 - "$t/img/mark.png" <<'PY'
import sys
from PIL import Image
Image.new('RGB', (8, 8), (200, 40, 40)).save(sys.argv[1])
PY
	printf '#let one = [One: some words on the first page.]\n' > "$t/chap/one.typ"
	printf 'Two: the included page.\n' > "$t/chap/two.typ"
	cat > "$t/main.typ" <<'TYP'
#set page(width: 200pt, height: 120pt, margin: 12pt)
#set text(size: 10pt)
#import "chap/one.typ": one
= Clean
#image("/img/mark.png", width: 24pt)
#text(font: "Libertinus Mono")[mono face]
#one
#pagebreak()
#include "/chap/two.typ"
TYP
	cat > "$t/sub/page.typ" <<'TYP'
#set page(width: 200pt, height: 120pt, margin: 12pt)
#import "../chap/one.typ": one
#one
TYP
}
mktree "$S/clean"

mkdir -p "$S/bad"
printf '#set page(width: 200pt, height: 120pt)\n= Broken\n#let x = (1, 2\n' > "$S/bad/main.typ"

mkdir -p "$S/font"
printf '#set page(width: 200pt, height: 120pt)\n#set text(font: "ZZNoSuchFamilyZZ")\nA family nobody has.\n' > "$S/font/main.typ"

# The marker in a directory name, a file name, a line of source, an error and a font family.
MK="$S/root-$MARK"
mkdir -p "$MK/dir-$MARK" "$MK/assets/fonts"
printf '#let broken = (1, 2\n// %s in a comment\n' "$MARK" > "$MK/dir-$MARK/lib-$MARK.typ"
cat > "$MK/dir-$MARK/main-$MARK.typ" <<TYP
#set page(width: 200pt, height: 120pt)
#set text(font: "family-$MARK")
#import "lib-$MARK.typ": *
Text #$MARK more.
TYP

# ── 1. a clean tree ─────────────────────────────────────────────────────────
r="$(run "$DT" 0 "$S/clean" main.typ sub/page.typ gone.typ)"
echo "$r" | sed 's/^/    /'
ALL="$r"
expect "clean: typst compiles main 1 in 2 pages" "$r" "tree 0 main 1 typst exit 0 pages 2 error 0 warning 0"
expect "clean: the evaluator compiles it in 2 pages, strict" "$r" "tree 0 main 1 eval exit 0 pages 2 error 0 warning 0"
expect_re "clean: the curated reader gives its line" "$r" "tree 0 main 1 curated exit [0-9]+ pages (none|[0-9]+) error [0-9]+ warning [0-9]+"
expect "clean: main 2 in a subdirectory, run from there, reaches ../chap in typst" "$r" "tree 0 main 2 typst exit 0 pages 1 error 0 warning 0"
expect "clean: and in the evaluator" "$r" "tree 0 main 2 eval exit 0 pages 1 error 0 warning 0"
expect "clean: a main that is not there is named" "$r" "tree 0 main 3 missing-main"
lack "clean: a strict run that passed has no lax run" "$r" "tree 0 main 1 eval-lax"

# ── 2. a syntax error ───────────────────────────────────────────────────────
r="$(run "$DT" 1 "$S/bad" main.typ)"
echo "$r" | sed 's/^/    /'
ALL="$ALL
$r"
expect_re "syntax error: typst exits non-zero, with no pages and an error" "$r" "tree 1 main 1 typst exit [1-9][0-9]* pages none error [1-9][0-9]* warning [0-9]+"
expect_re "syntax error: the evaluator exits non-zero, with no pages" "$r" "tree 1 main 1 eval exit [1-9][0-9]* pages none error [0-9]+ warning [0-9]+"
expect_re "syntax error: the strict failure is followed by the lax run" "$r" "tree 1 main 1 eval-lax exit [1-9][0-9]* pages none error [0-9]+ warning [0-9]+"

# ── 3. a font family nobody has ─────────────────────────────────────────────
r="$(run "$DT" 2 "$S/font" main.typ)"
echo "$r" | sed 's/^/    /'
ALL="$ALL
$r"
expect "font: typst warns and exits 0" "$r" "tree 2 main 1 typst exit 0 pages 1 error 0 warning 1"
expect_re "font: the evaluator, strict, exits non-zero" "$r" "tree 2 main 1 eval exit [1-9][0-9]* pages none error [0-9]+ warning [0-9]+"
expect_re "font: and lax exits 0 in 1 page" "$r" "tree 2 main 1 eval-lax exit 0 pages 1 error 0 warning [0-9]+"

# ── 4. the guards ───────────────────────────────────────────────────────────
r="$(run "$DT" 3 "$S/nowhere" main.typ)"
ALL="$ALL
$r"
expect "guard: a root that is not there" "$r" "tree 3 missing-root"
printf '#!/bin/sh\necho "typst 0.14.2 (stub)"\n' > "$S/typst-old"
chmod +x "$S/typst-old"
r="$(TYPST_BIN="$S/typst-old" DEVTREES_INDEX=0 AUSTENITE_BIN="$AUS" "$DT" "$S/clean" main.typ 2>&1)"
ALL="$ALL
$r"
expect "guard: a typst that is not 0.15.1" "$r" "devtrees: typst is not 0.15.1"
r="$(AUSTENITE_BIN="$S/none" TYPST_BIN="$TYPST" "$DT" "$S/clean" main.typ 2>&1)"
ALL="$ALL
$r"
expect "guard: no austenite binary" "$r" "devtrees: no austenite binary"
r="$("$DT" 2>&1)"
ALL="$ALL
$r"
expect "guard: no arguments" "$r" "devtrees: usage"
r="$(DEVTREES_INDEX=x AUSTENITE_BIN="$AUS" "$DT" "$S/clean" main.typ 2>&1)"
ALL="$ALL
$r"
expect "guard: a tree number that is not digits" "$r" "devtrees: usage"
printf '#!/bin/sh\n[ "$1" = --version ] && { echo "typst 0.15.1 (stub)"; exit 0; }\nexec sleep 30\n' > "$S/typst-slow"
chmod +x "$S/typst-slow"
r="$(TYPST_BIN="$S/typst-slow" DEVTREES_TIMEOUT=1 DEVTREES_INDEX=7 AUSTENITE_BIN="$AUS" "$DT" "$S/clean" sub/page.typ 2>&1)"
ALL="$ALL
$r"
expect "guard: an engine that runs past the timeout exits 124" "$r" "tree 7 main 1 typst exit 124 pages none error 0 warning 0"
expect_re "guard: the tree's number is the caller's" "$r" "tree 7 main 1 curated exit [0-9]+ pages (none|[0-9]+) error [0-9]+ warning [0-9]+"

# ── 5. the vocabulary ───────────────────────────────────────────────────────
WL='tree [0-9]+ main [0-9]+ (typst|curated|eval|eval-lax) exit [0-9]+ pages (none|[0-9]+) error [0-9]+ warning [0-9]+'
WL="$WL|tree [0-9]+ missing-root|tree [0-9]+ main [0-9]+ missing-main"
WL="$WL|devtrees: (typst is not 0\\.15\\.1|no austenite binary|usage|no scratch)"
n="$(printf '%s\n' "$ALL" | grep -vcE "^($WL)\$")"
[ "$n" -eq 0 ] && ok "whitelist: every line printed so far is one of the closed shapes" || fail "whitelist: $n line(s) outside the closed shapes"
left="$(ls -d "$PARENT"/run.* 2>/dev/null | wc -l)"
[ "$left" -eq 0 ] && ok "scratch: no run directory is left behind" || fail "scratch: $left run directory(ies) left under $PARENT"

# ── 6. the marker ───────────────────────────────────────────────────────────
raw="$(cd "$MK/dir-$MARK" && gate_typst "$TYPST" compile --root "$MK" --font-path "$MK/assets/fonts" "main-$MARK.typ" "$S/raw.pdf" 2>&1)"
printf '%s\n' "$raw" | grep -qF "$MARK" && ok "leak premise: typst's own stderr quotes the tree's text" \
	|| fail "leak premise: typst did not quote the marker, so the next check proves nothing"
raw="$(cd "$MK/dir-$MARK" && gate_hidden "$AUS" --eval --diag-summary --root "$MK" --font-path "$MK/assets/fonts" "main-$MARK.typ" "$S/rawout" 2>&1)"
printf '%s\n' "$raw" | grep -qF "$MARK" && ok "leak premise: the evaluator's own stderr quotes it too" \
	|| fail "leak premise: the evaluator did not quote the marker"
r="$(run "$DT" 4 "$MK" "dir-$MARK/main-$MARK.typ" "gone-$MARK.typ")"
echo "$r" | sed 's/^/    /'
lack "marker: nothing of a tree's names, text or errors is printed" "$r" "$MARK"
expect_re "marker: typst's failure is counted" "$r" "tree 4 main 1 typst exit [1-9][0-9]* pages none error [1-9][0-9]* warning [0-9]+"
expect "marker: a missing main is a closed word" "$r" "tree 4 main 2 missing-main"
n="$(printf '%s\n' "$r" | grep -vcE "^($WL)\$")"
[ "$n" -eq 0 ] && ok "marker: and every line is one of the closed shapes" || fail "marker: $n stray line(s)"

# A stray from the script's own tools: HOME names a directory with the marker in its name, where the scratch
# directory is made and cannot be, so that mktemp's own message quotes the path. The whitelist must drop it.
mkdir -p "$S/home-$MARK/.cache/austenite/devtrees"
chmod 555 "$S/home-$MARK/.cache/austenite/devtrees"
stray() { # stray <script>
	HOME="$S/home-$MARK" DEVTREES_INDEX=0 AUSTENITE_BIN="$AUS" TYPST_BIN="$TYPST" "$1" "$S/clean" main.typ 2>&1
}
raw="$(mktemp -d "$S/home-$MARK/.cache/austenite/devtrees/run.XXXXXX" 2>&1)"
printf '%s\n' "$raw" | grep -qF "$MARK" && ok "leak premise: mktemp's own message quotes the path" \
	|| fail "leak premise: mktemp did not quote the marker, so the stray check proves nothing"
r="$(stray "$DT")"
lack "stray: a tool's message that quotes a path is dropped" "$r" "$MARK"
expect "stray: and the script says so in a closed word" "$r" "devtrees: no scratch"

# Lines that look like results but are not, through the script's own whitelist.
prog="$(sed -n "/^WHITELIST='\$/,/'\$/p" "$DT" | sed "1d; \$s/'\$//")"
if [ -z "$prog" ]; then
	fail "whitelist: the program was not found in devtrees.sh"
else
	lookalikes="tree 0 main 1 typst exit 0 pages 1 error 0 warning 0
tree 0 main 1 typst exit 0 pages 1 error 0 warning 0 $MARK
tree $MARK main 1 typst exit 0 pages 1 error 0 warning 0
 tree 0 main 1 typst exit 0 pages 1 error 0 warning 0
tree 0 main 1 typst exit 0 pages $MARK error 0 warning 0
tree 0 main 1 pdf exit 0 pages 1 error 0 warning 0
tree 0 main $MARK missing-main
tree 0 missing-root $MARK
devtrees: usage $MARK
devtrees: $MARK
$MARK"
	out="$(printf '%s\n' "$lookalikes" | awk "$prog")"
	[ "$out" = "tree 0 main 1 typst exit 0 pages 1 error 0 warning 0" ] \
		&& ok "whitelist: of ten lines only the one result passes; each lookalike is dropped" \
		|| fail "whitelist: the lookalikes were not all dropped"
fi

# ── 7. the plant ────────────────────────────────────────────────────────────
mkdir -p "$S/plant"
cp "$HERE/gate_lib.sh" "$S/plant/"
sed 's/| awk "\$WHITELIST"$/| cat/' "$DT" > "$S/plant/devtrees.sh"
chmod +x "$S/plant/devtrees.sh"
if cmp -s "$DT" "$S/plant/devtrees.sh" || ! grep -q '2>&1 | cat$' "$S/plant/devtrees.sh"; then
	fail "plant: the sed did not change devtrees.sh (its final filter moved)"
else
	r="$(stray "$S/plant/devtrees.sh")"
	if printf '%s\n' "$r" | grep -qF "$MARK"; then
		ok "plant: with the final whitelist dropped the marker is printed, so the stray test is red"
	else
		fail "plant: the stray test stayed green with the final whitelist dropped"
	fi
fi

echo "selftest: $bad failure(s)"
[ "$bad" -eq 0 ]
