#!/usr/bin/env bash
# Proves the books' `./dev` script hands a tree to `austenite watch` when it should, and leaves the typst and
# curated paths as they were.
#
#     dev_dispatch.sh <dev> [<dev-orig>]
#
#   <dev>       the script under test
#   <dev-orig>  the script it replaces; with it, the typst branch of `start_typst` is compared line for line
#
# Everything is a stub. `austenite` and `typst` are scripts that write their argument line to a log and make the
# file or directory the real one would have made, then stay a moment as a watch does; `papers`, `evince` and `xdg-open` only note that they were
# asked; and `python3` fails `import pikepdf`. The PATH holds the stubs and a symlink to each plain tool the
# script uses, so `gs` is absent too: a hand-over with `--cmyk --scrub` that still runs proves the watch needs
# neither. Trees are a few files under ~/.cache/austenite/dev_dispatch, made here and removed on exit. No book
# is read or written, no real austenite or typst runs, and no viewer opens.
#
# Cases, each checked for the exit status, the austenite argument line, the typst argument line, whether a viewer
# was asked for, and a message where one is due:
#   an austenite.jdat tree, with and without --cmyk; USE_EVAL=1 with --cmyk, --grey, --scrub and --cmyk --scrub,
#   from the environment and from a dev.conf; USE_TYPST=1 in the environment and in a dev.conf, --typst, and
#   USE_TYPST=1 beside USE_EVAL=1; a binary whose usage has no watch command; a missing binary; the curated
#   path; and the Ghostscript and pikepdf refusals, which the curated path keeps. Then a book with its own
#   austenite.jdat below the tree root, reached through the relative dev symlink a book directory holds: run
#   in the book, run in a directory below it, with --cmyk, with USE_TYPST=1 in the environment, beside a
#   dev.conf that sets USE_TYPST=1 for every other directory of the tree, and in that other directory; and a
#   settings file above the tree root, which does not count.
# Then the plant: the script with its USE_TYPST line removed, which the same cases must find red.
set -u

DEV="${1:-}"
ORIG="${2:-}"
[ -n "$DEV" ] && [ -f "$DEV" ] || { echo "usage: dev_dispatch.sh <dev> [<dev-orig>]"; exit 2; }
DEV="$(cd "$(dirname "$DEV")" && pwd -P)/$(basename "$DEV")"
if [ -n "$ORIG" ]; then
	[ -f "$ORIG" ] || { echo "dev_dispatch: no such file: $ORIG"; exit 2; }
	ORIG="$(cd "$(dirname "$ORIG")" && pwd -P)/$(basename "$ORIG")"
fi

BASE="$HOME/.cache/austenite/dev_dispatch"
mkdir -p "$BASE" || exit 2
S="$(mktemp -d "$BASE/run.XXXXXX")" || exit 2
S="$(cd "$S" && pwd -P)"
trap 'rm -rf "${S:?}"' EXIT

BIN="$S/bin"; LOGS="$S/log"; TREES="$S/trees"
mkdir -p "$BIN" "$LOGS" "$TREES" "$S/home"

# A symlink to each plain tool the script uses, and nothing else.
for t in bash dirname readlink grep sort timeout stat cp sleep md5sum cut head mkdir rm cat touch; do
	p="$(type -P "$t")" || { echo "dev_dispatch: no $t on this machine"; exit 2; }
	ln -s "$p" "$BIN/$t"
done

cat > "$BIN/austenite" <<'EOF'
#!/bin/bash
# No arguments: the usage, from the file the case names. Otherwise log the line and make what the real one makes.
if [ $# -eq 0 ]; then cat "$STUB_USAGE"; exit 0; fi
echo "$*" >> "$STUB_LOG/austenite.argv"
if [ "$1" = "--watch" ]; then
	out="${@: -1}"
	mkdir -p "$out"
	: > "$out/document.pdf"
	sleep 3
fi
exit 0
EOF
cat > "$BIN/typst" <<'EOF'
#!/bin/bash
echo "$*" >> "$STUB_LOG/typst.argv"
: > "${@: -1}"
sleep 3
exit 0
EOF
for v in papers evince xdg-open; do
	printf '#!/bin/bash\necho %s >> "$STUB_LOG/viewer.log"\nexit 0\n' "$v" > "$BIN/$v"
done
printf '#!/bin/bash\nexit 1\n' > "$BIN/python3"
chmod +x "$BIN"/austenite "$BIN"/typst "$BIN"/papers "$BIN"/evince "$BIN"/xdg-open "$BIN"/python3

echo "Usage: austenite watch|build ... or austenite [--watch] [--eval] <SOURCE.typ> [OUTPUT_DIR]" > "$S/usage_new"
echo "Usage: austenite [--watch] [--eval] <SOURCE.typ> [OUTPUT_DIR]"                               > "$S/usage_old"

# mk_tree <name> [jdat] [conf=<text>]: a document and what stands beside it. `.root` marks the tree root,
# where `go` puts the link to the script under test.
mk_tree() {
	local name="$1"; shift
	mkdir -p "$TREES/$name"
	: > "$TREES/$name/.root"
	printf '= Title\n\nBody.\n' > "$TREES/$name/x.typ"
	local a
	for a in "$@"; do
		case "$a" in
			jdat)	echo '{"document": "x.typ"}' > "$TREES/$name/austenite.jdat" ;;
			conf=*)	printf '%s\n' "${a#conf=}" > "$TREES/$name/dev.conf" ;;
		esac
	done
}

# mk_sub <tree> <sub> [jdat]: a directory below a tree root with a document, its own dev linked relatively
# to the one above it as a book directory's is, and optionally its own settings file.
mk_sub() {
	local d="$TREES/$1/$2"
	mkdir -p "$d"
	printf '= Title\n\nBody.\n' > "$d/x.typ"
	ln -sfn ../dev "$d/dev"
	[ "${3:-}" = "jdat" ] && echo '{"document": "x.typ"}' > "$d/austenite.jdat"
	return 0
}

FAILS=0
CASES=0

# go <dev> <name> <tree> <usage> [VAR=value...] -- [flags...]: run the script once, leaving $OUT and $RC.
go() {
	local dev="$1" name="$2" tree="$3" usage="$4"; shift 4
	local envs=()
	while [ $# -gt 0 ] && [ "$1" != "--" ]; do envs+=("$1"); shift; done
	[ $# -gt 0 ] && shift
	local log="$LOGS/$name"
	rm -rf "$log"; mkdir -p "$log"
	local t="$TREES/$tree"
	# Each run starts from the tree as made, without the last run's outputs.
	rm -rf "$t/.x.aus-out" "$t/x.pdf" "$t/.x.source.pdf" "$t/.x.raw.pdf"
	local r="$t"
	while [ ! -f "$r/.root" ]; do r="$(dirname "$r")"; done
	ln -sfn "$dev" "$r/dev"
	OUT="$(cd "$t" && timeout 30 /usr/bin/env -i PATH="$BIN" HOME="$S/home" \
		AUSTENITE_BIN="$BIN/austenite" STUB_LOG="$log" STUB_USAGE="$S/$usage" \
		"${envs[@]}" ./dev x.typ "$@" 2>&1)"
	RC=$?
}

# expect <name> <rc> <aus-line|-> <typ-line|-> <viewer yes|no> <message|->: judge the last run.
expect() {
	local name="$1" rc="$2" aus="$3" typ="$4" view="$5" msg="$6"
	local log="$LOGS/$name" why="" got
	CASES=$((CASES + 1))
	[ "$RC" = "$rc" ] || why="$why exit $RC not $rc;"
	got="$(cat "$log/austenite.argv" 2>/dev/null)"; [ -f "$log/austenite.argv" ] || got="-"
	[ "$got" = "$aus" ] || why="$why austenite got [$got] not [$aus];"
	got="$(cat "$log/typst.argv" 2>/dev/null)"; [ -f "$log/typst.argv" ] || got="-"
	[ "$got" = "$typ" ] || why="$why typst got [$got] not [$typ];"
	if [ "$view" = "yes" ]; then
		[ -f "$log/viewer.log" ] || why="$why no viewer asked for;"
	else
		[ ! -f "$log/viewer.log" ] || why="$why a viewer was asked for;"
	fi
	if [ "$msg" != "-" ]; then
		case "$OUT" in *"$msg"*) ;; *) why="$why output lacks [$msg];" ;; esac
	fi
	if [ -z "$why" ]; then echo "ok    $name"; else echo "FAIL  $name:$why"; FAILS=$((FAILS + 1)); fi
}

# run_cases <dev>: every case against one copy of the script.
run_cases() {
	local dev="$1" T
	rm -rf "$TREES"; mkdir -p "$TREES"
	mk_tree jdat jdat
	mk_tree evalbare
	mk_tree evalconf  conf='DEV_FLAGS="--cmyk --scrub"
USE_EVAL=1'
	mk_tree typstconf conf='USE_TYPST=1'
	mk_tree plain
	T="$(cd "$TREES" && pwd -P)"
	local tf="watch --root $T/typstconf --font-path $T/typstconf/assets/fonts x.typ x.pdf"
	local jf="watch --root $T/jdat --font-path $T/jdat/assets/fonts x.typ x.pdf"
	local ef="watch --root $T/evalbare --font-path $T/evalbare/assets/fonts x.typ x.pdf"
	local rf="--set root=$T/evalbare --set fonts=$T/evalbare/assets/fonts"

	go "$dev" jdat_plain        jdat      usage_new --
	expect jdat_plain        0 "watch x.typ" - no "dev:"
	go "$dev" jdat_cmyk         jdat      usage_new -- --cmyk
	expect jdat_cmyk         0 "watch x.typ --set colour.space=cmyk" - no -
	go "$dev" eval_cmyk         evalbare  usage_new USE_EVAL=1 -- --cmyk
	expect eval_cmyk         0 "watch x.typ --set colour.space=cmyk $rf" - no -
	go "$dev" eval_grey         evalbare  usage_new USE_EVAL=1 -- --grey
	expect eval_grey         0 "watch x.typ --set colour.space=grey $rf" - no -
	go "$dev" eval_cmyk_scrub   evalbare  usage_new USE_EVAL=1 -- --cmyk --scrub
	expect eval_cmyk_scrub   0 "watch x.typ --set colour.space=cmyk $rf" - no -
	go "$dev" eval_scrub        evalbare  usage_new USE_EVAL=1 -- --scrub
	expect eval_scrub        0 "watch x.typ $rf" - no -
	go "$dev" eval_conf         evalconf  usage_new --
	expect eval_conf         0 "watch x.typ --set colour.space=cmyk --set root=$T/evalconf --set fonts=$T/evalconf/assets/fonts" - no -
	go "$dev" typst_env         jdat      usage_new USE_TYPST=1 --
	expect typst_env         0 - "$jf" yes -
	go "$dev" typst_conf        typstconf usage_new --
	expect typst_conf        0 - "$tf" yes -
	go "$dev" typst_flag        jdat      usage_new -- --typst
	expect typst_flag        0 - "$jf" yes -
	go "$dev" typst_beats_eval  evalbare  usage_new USE_EVAL=1 USE_TYPST=1 --
	expect typst_beats_eval  0 - "$ef" yes -
	go "$dev" old_binary        evalbare  usage_old USE_EVAL=1 --
	expect old_binary        0 - "$ef" yes "has no watch command"
	go "$dev" no_binary         jdat      usage_new AUSTENITE_BIN="$S/none" --
	expect no_binary         0 - "$jf" yes "austenite not found"
	go "$dev" curated           plain     usage_new --
	expect curated           0 "--watch x.typ .x.aus-out" - yes -
	go "$dev" curated_cmyk      plain     usage_new -- --cmyk
	expect curated_cmyk      1 - - no "needs Ghostscript"
	go "$dev" curated_scrub     plain     usage_new -- --scrub
	expect curated_scrub     1 - - no "needs pikepdf"

	# A book with its own settings file below the tree root, as the Invitation has.
	mk_tree shelf conf='case "$PWD" in
	*/inv|*/inv/*) ;;
	*) USE_TYPST=1 ;;
esac'
	mk_sub shelf inv jdat
	mk_sub shelf inv/part
	mk_sub shelf other
	mk_tree bare_shelf
	mk_sub bare_shelf inv jdat
	mkdir -p "$TREES/outer"
	echo '{"document": "x.typ"}' > "$TREES/outer/austenite.jdat"
	mk_tree outer/inner
	local sf="watch --root $T/shelf --font-path $T/shelf/assets/fonts x.typ x.pdf"
	local bf="watch --root $T/bare_shelf --font-path $T/bare_shelf/assets/fonts x.typ x.pdf"

	go "$dev" book_jdat         shelf/inv       usage_new --
	expect book_jdat         0 "watch x.typ" - no "dev:"
	go "$dev" book_jdat_below   shelf/inv/part  usage_new --
	expect book_jdat_below   0 "watch x.typ" - no -
	go "$dev" book_jdat_cmyk    bare_shelf/inv  usage_new -- --cmyk
	expect book_jdat_cmyk    0 "watch x.typ --set colour.space=cmyk" - no -
	go "$dev" book_jdat_eval    bare_shelf/inv  usage_new USE_EVAL=1 --
	expect book_jdat_eval    0 "watch x.typ" - no -
	go "$dev" book_typst_env    bare_shelf/inv  usage_new USE_TYPST=1 --
	expect book_typst_env    0 - "$bf" yes -
	go "$dev" book_typst_flag   bare_shelf/inv  usage_new -- --typst
	expect book_typst_flag   0 - "$bf" yes -
	go "$dev" book_other_typst  shelf/other     usage_new --
	expect book_other_typst  0 - "$sf" yes -
	go "$dev" above_root        outer/inner     usage_new --
	expect above_root        0 "--watch x.typ .x.aus-out" - yes -
}

# The typst branch of start_typst, from its opening line to the first four-space fi.
typst_branch() {
	awk '/^start_typst\(\) \{/ { f = 1 } f { print } f && /^    fi$/ { exit }' "$1"
}

echo "== the script"
run_cases "$DEV"
REAL_FAILS=$FAILS

if [ -n "$ORIG" ]; then
	CASES=$((CASES + 1))
	a="$(typst_branch "$DEV")"
	b="$(typst_branch "$ORIG")"
	if [ -n "$a" ] && [ "$a" = "$b" ]; then
		echo "ok    typst_branch_unchanged ($(printf '%s\n' "$a" | wc -l) lines)"
	else
		echo "FAIL  typst_branch_unchanged: the typst branch of start_typst differs from $ORIG"
		REAL_FAILS=$((REAL_FAILS + 1))
	fi
fi

echo "== the plant: the USE_TYPST line removed"
PLANTED="$S/dev.planted"
grep -vxF '[ "${USE_TYPST:-}" = "1" ] && engine="typst"' "$DEV" > "$PLANTED"
if cmp -s "$DEV" "$PLANTED"; then
	echo "FAIL  plant: no USE_TYPST line found to remove"
	exit 1
fi
chmod +x "$PLANTED"
REAL_CASES=$CASES
FAILS=0
run_cases "$PLANTED" > "$S/plant.out"
grep -v '^ok ' "$S/plant.out"
PLANT_FAILS=$FAILS

echo "== summary"
echo "script: $REAL_FAILS failed of $REAL_CASES cases"
echo "plant:  $PLANT_FAILS cases red"
if [ "$REAL_FAILS" -ne 0 ]; then echo "dev_dispatch: RED"; exit 1; fi
if [ "$PLANT_FAILS" -eq 0 ]; then echo "dev_dispatch: the plant stayed green, so the test proves nothing"; exit 1; fi
echo "dev_dispatch: green, and the plant is caught"
