#!/usr/bin/env bash
# Proves the O2 tools on synthetic projects, and proves each proof can fail.
#
#     selftest_gate.sh <austenite-bin> <pkg> <scratch>
#
# <austenite-bin> is the `austenite` binary and <pkg> the wasm-pack output directory, both built from the tree
# under test; <scratch> is a directory this script makes a child of and removes on exit. Everything here is
# synthetic: no path to a real project is taken or needed.
#
# It checks, in order:
#   1. g_checks.py, G1 G2 G4 G5 G7 G8 and G12: for each, a pair that agrees reads `same` and a pair that
#      differs the way the check is about reads red on the line that names the difference; then a copy of
#      g_checks.py with one fault planted is shown to miss it, so the expectation on that copy is red;
#   2. door_pdf.mjs on synthetic projects: a pass, a strict refusal, a package supplied and a package missing,
#      the same plants on a copy of the tools, and no word of the project's own text on the stream;
#   3. gate.sh end to end: a project that agrees is green, one that strict refuses is red, a fault in a required
#      check turns the verdict red, and nothing of the project's text is printed, with its two plants.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE="$(cd "$HERE/../.." && pwd)"
# shellcheck source=gate_lib.sh
. "$HERE/gate_lib.sh"

BIN="${1:?usage: selftest_gate.sh <austenite-bin> <pkg> <scratch>}"
PKG="${2:?usage: selftest_gate.sh <austenite-bin> <pkg> <scratch>}"
PARENT="${3:?usage: selftest_gate.sh <austenite-bin> <pkg> <scratch>}"
TYPST="${GATE_TYPST:-/home/jason/bin/typst}"
mkdir -p "$PARENT" || exit 2
S="$(mktemp -d "$PARENT/o2.XXXXXX")" || exit 2
trap 'rm -rf "${S:?}"' EXIT
export GATE_SCRATCH="$S"

bad=0
ok()   { echo "selftest: ok   $1"; }
fail() { echo "selftest: FAIL $1"; bad=$((bad + 1)); }
expect() { # expect <label> <text> <fixed line the text must contain>
	if printf '%s\n' "$2" | grep -qxF -- "$3"; then ok "$1"; else fail "$1 (wanted: $3)"; fi
}
lack() { # lack <label> <text> <fixed line the text must not contain>
	if printf '%s\n' "$2" | grep -qxF -- "$3"; then fail "$1 (found: $3)"; else ok "$1"; fi
}
lackany() { # lackany <label> <text> <fixed string the text must not contain anywhere>
	if printf '%s\n' "$2" | grep -qiF -- "$3"; then fail "$1 (found: $3)"; else ok "$1"; fi
}

G="$HERE/g_checks.py"

# ── typst documents ─────────────────────────────────────────────────────────
mkpdf() { # mkpdf <name> <typst source>: a PDF of a 200x80 page document
	printf '#set page(width: 200pt, height: 80pt, margin: 10pt)\n%s\n' "$2" > "$S/$1.typ"
	gate_typst "$TYPST" compile "$S/$1.typ" "$S/$1.pdf" > /dev/null 2>&1 || fail "fixture $1 did not compile"
}
npages() { pdfinfo "$1" 2>/dev/null | awk '/^Pages:/ { print $2 }'; }

# subst <src> <dst> <old> <new>: `dst` is `src` with `old` replaced, which must occur exactly once.
subst() {
	python3 - "$@" <<'PY'
import sys
src, dst, old, new = sys.argv[1:]
text = open(src).read()
if text.count(old) != 1:
    sys.exit(1)
open(dst, 'w').write(text.replace(old, new))
PY
}

# plant <label> <old text> <new text> <line> <check> <pdf>...
# The copy is g_checks.py with `old` replaced once by `new`; <line> is one the real file prints for this pair, so a
# copy that still prints it did not lose the check, and the proof is red.
plant() {
	local label="$1" old="$2" new="$3" want="$4" r
	shift 4
	if ! subst "$G" "$S/plant.py" "$old" "$new"; then
		fail "plant $label: the text it replaces has moved or repeats in g_checks.py"
		return
	fi
	r="$(python3 "$S/plant.py" "$@" 2>&1)"
	if printf '%s\n' "$r" | grep -qxF -- "$want"; then
		fail "plant $label: the planted copy still prints: $want"
	else
		ok "plant $label: the planted copy no longer prints the line, so the check is red"
	fi
}

# ── 1a. G1 ──────────────────────────────────────────────────────────────────
mkpdf p3 $'Alpha one\n#pagebreak()\nBeta two\n#pagebreak()\nGamma three'
mkpdf p3tall $'Alpha one\n#pagebreak()\n#set page(height: 100pt)\nBeta two\n#pagebreak()\n#set page(height: 80pt)\nGamma three'
mkpdf p2 $'Alpha one\n#pagebreak()\nBeta two'
r="$(python3 "$G" g1 "$S/p3.pdf" "$S/p3.pdf")"
expect "g1: a PDF agrees with itself" "$r" "g1 verdict same"
r="$(python3 "$G" g1 "$S/p3.pdf" "$S/p3tall.pdf")"
expect "g1: a page of another size goes red, on that page" "$r" "g1 mediabox differ 1 pages 2"
expect "g1: and the deviation is its 20 pt" "$r" "g1 mediabox max-deviation 20.0000"
expect "g1: and the verdict is differs" "$r" "g1 verdict differs"
r="$(python3 "$G" g1 "$S/p3.pdf" "$S/p2.pdf")"
expect "g1: a page that is missing goes red" "$r" "g1 pages a 3 b 2"
expect "g1: and the verdict is differs" "$r" "g1 verdict differs"
plant "g1 sizes ignored" 'd = max(abs(x - y) for x, y in zip(ma[i], mb[i]))' 'd = 0.0' "g1 mediabox differ 1 pages 2" g1 "$S/p3.pdf" "$S/p3tall.pdf"
plant "g1 count ignored" 'same = len(ma) == len(mb) and not differ' 'same = not differ' "g1 verdict differs" g1 "$S/p3.pdf" "$S/p2.pdf"

# ── 1b. G2 ──────────────────────────────────────────────────────────────────
mkpdf f_plain 'Alpha one two'
mkpdf f_other 'Beta three four five'
mkpdf f_mono $'Alpha one two\n#text(font: "DejaVu Sans Mono")[mono]'
r="$(python3 "$G" g2 "$S/f_plain.pdf" "$S/f_other.pdf")"
expect "g2: the same fonts with other glyphs agree (the subset tag is stripped)" "$r" "g2 verdict same"
r="$(python3 "$G" g2 "$S/f_plain.pdf" "$S/f_mono.pdf")"
expect "g2: a font that only b embeds is named" "$r" "g2 only-b DejaVuSansMono"
expect "g2: and the verdict is differs" "$r" "g2 verdict differs"
plant "g2 subset tag kept" "FONT_TAG = re.compile(r'^[A-Z]{6}\\+')" "FONT_TAG = re.compile(r'^(?!)')" "g2 verdict same" g2 "$S/f_plain.pdf" "$S/f_other.pdf"
plant "g2 font sets not compared" 'same = sa == sb and flags_a == 0 and flags_b == 0' 'same = flags_a == 0 and flags_b == 0' "g2 verdict differs" g2 "$S/f_plain.pdf" "$S/f_mono.pdf"
# A font that is not embedded and has no Unicode map: one is written by hand.
python3 - "$S/noemb.pdf" <<'PY'
import sys
import pikepdf
pdf = pikepdf.Pdf.new()
font = pdf.make_indirect(pikepdf.Dictionary(Type=pikepdf.Name.Font, Subtype=pikepdf.Name.Type1, BaseFont=pikepdf.Name.Helvetica))
page = pdf.add_blank_page(page_size=(200, 80))
page.Resources = pikepdf.Dictionary(Font=pikepdf.Dictionary(F1=font))
page.Contents = pdf.make_stream(b'BT /F1 12 Tf 10 40 Td (Hello) Tj ET')
pdf.save(sys.argv[1])
PY
r="$(python3 "$G" g2 "$S/noemb.pdf" "$S/noemb.pdf")"
expect "g2: a font that is not embedded and has no Unicode map is flagged on both sides" "$r" "g2 flags a 1 b 1"
expect "g2: and the verdict is differs although the sets are equal" "$r" "g2 verdict differs"
plant "g2 flags ignored" 'flags_a = sum(1 for _, ok in fa if not ok)' 'flags_a = 0' "g2 flags a 1 b 1" g2 "$S/noemb.pdf" "$S/noemb.pdf"

# ── 1c. G4 ──────────────────────────────────────────────────────────────────
mkpdf h_num $'#set heading(numbering: "1.1")\n= One\n== Under\n= Two'
mkpdf h_plain $'= One\n== Under\n= Two'
mkpdf h_flat $'= One\n= Under\n= Two'
mkpdf h_short $'= One\n== Under'
mkpdf h_ab $'= Alpha\n= Beta'
mkpdf h_ba $'= Beta\n= Alpha'
r="$(python3 "$G" g4 "$S/h_plain.pdf" "$S/h_plain.pdf")"
expect "g4: an outline agrees with itself" "$r" "g4 verdict same"
r="$(python3 "$G" g4 "$S/h_num.pdf" "$S/h_plain.pdf")"
expect "g4: a numbered title and a plain one differ, with the levels equal" "$r" "g4 levels equal first-diff none"
expect "g4: all 3 titles are unmatched" "$r" "g4 titles differs unmatched a 3 b 3"
expect "g4: and all 3 differ only by a leading number" "$r" "g4 titles prefix-only 3"
expect "g4: and the verdict is differs" "$r" "g4 verdict differs"
r="$(python3 "$G" g4 "$S/h_plain.pdf" "$S/h_flat.pdf")"
expect "g4: a title at another level goes red, at its place" "$r" "g4 levels differs first-diff 2"
r="$(python3 "$G" g4 "$S/h_plain.pdf" "$S/h_short.pdf")"
expect "g4: a missing entry is counted" "$r" "g4 outline a 3 b 2"
expect "g4: and the levels differ where it is missing" "$r" "g4 levels differs first-diff 3"
r="$(python3 "$G" g4 "$S/h_ab.pdf" "$S/h_ba.pdf")"
expect "g4: the same titles in another order are one multiset" "$r" "g4 titles equal unmatched a 0 b 0"
expect "g4: but two are out of place" "$r" "g4 titles position differ 2 of 2"
expect "g4: and the verdict is differs" "$r" "g4 verdict differs"
plant "g4 titles ignored" 'titles_equal = miss_a == 0 and miss_b == 0' 'titles_equal = True' "g4 titles differs unmatched a 3 b 3" g4 "$S/h_num.pdf" "$S/h_plain.pdf"
plant "g4 levels ignored" 'if la[i] != lb[i]:' 'if False:' "g4 levels differs first-diff 2" g4 "$S/h_plain.pdf" "$S/h_flat.pdf"
plant "g4 order ignored" 'pos += 1' 'pos += 0' "g4 verdict differs" g4 "$S/h_ab.pdf" "$S/h_ba.pdf"

# ── 1d. G5 ──────────────────────────────────────────────────────────────────
mkpdf i_full $'#set document(title: "T", author: "Au", description: "Sub")\nbody'
mkpdf i_part $'#set document(title: "T", author: "Other")\nbody'
r="$(python3 "$G" g5 "$S/i_full.pdf" "$S/i_full.pdf")"
expect "g5: Info agrees with itself" "$r" "g5 verdict same"
expect "g5: an entry that neither carries is absent-both" "$r" "g5 keywords absent-both"
r="$(python3 "$G" g5 "$S/i_full.pdf" "$S/i_part.pdf")"
expect "g5: an equal title" "$r" "g5 title equal"
expect "g5: a different author" "$r" "g5 author differs"
expect "g5: an entry only a carries differs" "$r" "g5 subject differs"
expect "g5: and the verdict is differs" "$r" "g5 verdict differs"
plant "g5 values ignored" 'elif x == y:' 'elif True:' "g5 author differs" g5 "$S/i_full.pdf" "$S/i_part.pdf"
plant "g5 absence ignored" 'if x is None and y is None:' 'if x is None or y is None:' "g5 subject differs" g5 "$S/i_full.pdf" "$S/i_part.pdf"

# ── 1e. G7 ──────────────────────────────────────────────────────────────────
mkpdf r_plain $'Alpha one\n#pagebreak()\nBeta two'
mkpdf r_red $'Alpha one\n#pagebreak()\nBeta two\n#place(top + left, dx: -10pt, dy: -10pt, rect(width: 200pt, height: 40pt, fill: red))'
r="$(python3 "$G" g7 "$S/r_plain.pdf" "$S/r_red.pdf")"
expect "g7: the page that is the same reads 0" "$r" "g7 page 1 0.0000"
p2="$(printf '%s\n' "$r" | awk '$1 == "g7" && $2 == "page" && $3 == 2 { print $4 }')"
if [ -n "$p2" ] && awk -v f="$p2" 'BEGIN { exit !(f >= 0.4 && f <= 0.6) }'; then
	ok "g7: the page with half its area red reads about a half ($p2)"
else
	fail "g7: the red half page read '$p2', wanted 0.4 to 0.6"
fi
expect "g7: both pages were compared" "$r" "g7 compared 2 of a 2 b 2"
r="$(env -u GATE_SCRATCH python3 "$G" g7 "$S/r_plain.pdf" "$S/r_red.pdf")"
expect "g7: with no scratch directory it refuses, and writes no raster" "$r" "g7 error scratch"
plant "g7 threshold" 'diff > 16' 'diff > 255' "g7 page 2 $p2" g7 "$S/r_plain.pdf" "$S/r_red.pdf"
plant "g7 one channel only" '.max(axis=2)' '.min(axis=2)' "g7 page 2 $p2" g7 "$S/r_plain.pdf" "$S/r_red.pdf"
[ -z "$(ls -A "$S" | grep -v '\.\(typ\|pdf\|py\)$')" ] && ok "g7: its rasters are gone" || fail "g7 left rasters behind: $(ls -A "$S" | grep -v '\.\(typ\|pdf\|py\)$' | head -3)"

# ── 1f. G8 ──────────────────────────────────────────────────────────────────
words() { echo "#range($1).map(_ => \"abcde\").join(\" \")"; }
mkpdf c_a $'#set page(height: 300pt)\n'"$(words 60)"$'\n#pagebreak()\n'"$(words 60)"
mkpdf c_b $'#set page(height: 300pt)\n'"$(words 60) f"$'\n#pagebreak()\n'"$(words 66)"
mkpdf c_blank $'#set page(height: 300pt)\n'"$(words 60)"$'\n#pagebreak()\n#h(1pt)'
[ "$(npages "$S/c_b.pdf")" = 2 ] && [ "$(npages "$S/c_blank.pdf")" = 2 ] || fail "fixture: the character-count documents are not 2 pages"
r="$(python3 "$G" g8 "$S/c_a.pdf" "$S/c_a.pdf")"
expect "g8: a PDF agrees with itself" "$r" "g8 verdict same"
r="$(python3 "$G" g8 "$S/c_a.pdf" "$S/c_b.pdf")"
expect "g8: a page 0.3% over is within the 1%, one 10% over is not" "$r" "g8 out-of-tolerance 1 pages 2"
expect "g8: its two counts are named" "$r" "g8 page 2 a 300 b 330"
expect "g8: and the verdict is differs" "$r" "g8 verdict differs"
r="$(python3 "$G" g8 "$S/c_a.pdf" "$S/c_blank.pdf")"
expect "g8: a page with no text where a has some is named" "$r" "g8 empty 1 pages 2"
plant "g8 tolerance" 'if dev > 0.01:' 'if dev > 0.5:' "g8 out-of-tolerance 1 pages 2" g8 "$S/c_a.pdf" "$S/c_b.pdf"
plant "g8 empty ignored" 'if a > 0 and b == 0:' 'if False:' "g8 empty 1 pages 2" g8 "$S/c_a.pdf" "$S/c_blank.pdf"

# ── 1g. G12 ─────────────────────────────────────────────────────────────────
touch_byte() { # touch_byte <in> <out>: the same size, one byte changed
	python3 - "$1" "$2" <<'PY'
import sys
d = bytearray(open(sys.argv[1], 'rb').read())
d[100] ^= 0x55
open(sys.argv[2], 'wb').write(bytes(d))
PY
}
cp "$S/p3.pdf" "$S/x1.pdf"
cp "$S/p3.pdf" "$S/x2.pdf"
cp "$S/p3.pdf" "$S/x3.pdf"
touch_byte "$S/p3.pdf" "$S/y.pdf"
r="$(python3 "$G" g12 "$S/x1.pdf" "$S/x2.pdf" "$S/x1.pdf" "$S/x2.pdf" "$S/x3.pdf")"
expect "g12: five equal PDFs agree" "$r" "g12 verdict same"
expect "g12: and the CLI's is the door's" "$r" "g12 cli-door equal"
r="$(python3 "$G" g12 "$S/x1.pdf" "$S/y.pdf" "$S/x1.pdf" "$S/x2.pdf" "$S/x3.pdf")"
expect "g12: two CLI runs that differ go red" "$r" "g12 cli-twice differs"
expect "g12: and the verdict is differs" "$r" "g12 verdict differs"
r="$(python3 "$G" g12 "$S/x1.pdf" "$S/x2.pdf" "$S/x1.pdf" "$S/y.pdf" "$S/x3.pdf")"
expect "g12: two compiles on one door instance that differ go red" "$r" "g12 door-one-instance differs"
r="$(python3 "$G" g12 "$S/x1.pdf" "$S/x2.pdf" "$S/x1.pdf" "$S/x2.pdf" "$S/y.pdf")"
expect "g12: a second instance that differs goes red" "$r" "g12 door-two-instances differs"
r="$(python3 "$G" g12 "$S/x1.pdf" "$S/x2.pdf" "$S/x1.pdf" "$S/x2.pdf" "$S/none.pdf")"
expect "g12: a PDF that is absent is named" "$r" "g12 door-two-instances absent"
expect "g12: and the verdict is differs" "$r" "g12 verdict differs"
r="$(python3 "$G" g12 "$S/x1.pdf" "$S/x1.pdf" "$S/y.pdf" "$S/y.pdf" "$S/y.pdf")"
expect "g12: a CLI PDF that is not the door's is reported" "$r" "g12 cli-door differs"
expect "g12: and does not turn the verdict red" "$r" "g12 verdict same"
plant "g12 bytes not compared" "return 'equal' if x[0] == y[0] else 'differs'" "return 'equal' if x[1] == y[1] else 'differs'" "g12 door-one-instance differs" g12 "$S/x1.pdf" "$S/x2.pdf" "$S/x1.pdf" "$S/y.pdf" "$S/x3.pdf"
plant "g12 absent passes" $'\tif x is None or y is None:\n\t\treturn \'absent\'' $'\tif x is None or y is None:\n\t\treturn \'equal\'' "g12 door-two-instances absent" g12 "$S/x1.pdf" "$S/x2.pdf" "$S/x1.pdf" "$S/x2.pdf" "$S/none.pdf"


# ── 2. door_pdf.mjs ─────────────────────────────────────────────────────────
mkproj() { # mkproj <dir> <typst source>: a project of one file and an empty font directory
	mkdir -p "$1/assets/fonts"
	printf '%s\n' "$2" > "$1/main.typ"
}
HEAD='#set page(width: 200pt, height: 80pt, margin: 10pt)
#set document(title: "Synthetic", author: "A. Writer", description: "Sub")'
mkproj "$S/pj_ok" "$HEAD"$'\n= One\nAlpha one two three.\n#pagebreak()\n= Two\nBeta four five six.\n#pagebreak()\nGamma seven.'
mkproj "$S/pj_font" "$HEAD"$'\n#set text(font: "ZZLeakMarkerZZ")\nHello.'
mkproj "$S/pj_syn" "$HEAD"$'\nText #ZZLeakMarkerZZ more.'

# One package in Typst's layout, which typst and the CLI read as their cache and the gather as a directory.
mkdir -p "$S/pk/preview/zzpkg/0.1.0"
printf '[package]\nname = "zzpkg"\nversion = "0.1.0"\nentrypoint = "lib.typ"\n' > "$S/pk/preview/zzpkg/0.1.0/typst.toml"
printf '#let zz(x) = [zz(#x)]\n' > "$S/pk/preview/zzpkg/0.1.0/lib.typ"
mkproj "$S/pj_pkg" "$HEAD"$'\n#import "@preview/zzpkg:0.1.0": zz\n#zz([a])'

door() { # door <project> <out> [options...]: door_pdf.mjs from `tools` (or $TOOLS)
	local p="$1" out="$2"
	shift 2
	gate_cap node "${TOOLS:-$CRATE/tools}/gate/door_pdf.mjs" "$PKG" "$S/$p" main.typ assets/fonts "$out" "$@" 2>&1
}

r="$(door pj_ok "$S/d_ok.pdf" --again "$S/d_ok2.pdf")"
expect "door: a project that compiles strict is ok, with its pages, no diagnostic and no need" "$r" "door ok pages 3 kinds none needs 0"
expect "door: a second compile on the instance is ok" "$r" "door again ok"
[ "$(npages "$S/d_ok.pdf")" = 3 ] && ok "door: the PDF has the 3 pages" || fail "door: the PDF has $(npages "$S/d_ok.pdf") pages"
cmp -s "$S/d_ok.pdf" "$S/d_ok2.pdf" && ok "door: the two compiles on one instance are the same bytes" || fail "door: the two compiles differ"

r="$(door pj_font "$S/d_font.pdf")"
# A refusal's first diagnostic restates its first site, so a refusal for one font counts that kind twice.
expect "door: a font that is not there is a strict refusal, named by its kind" "$r" "door error pages 1 kinds missing_font:2 needs 0"
[ ! -e "$S/d_font.pdf" ] && ok "door: and writes no PDF" || fail "door: wrote a PDF for a refusal"
lackany "door: and prints nothing of the project's text" "$r" "ZZLeakMarker"

r="$(door pj_syn "$S/d_syn.pdf")"
expect "door: a hard error is named by its kind" "$r" "door error pages none kinds unknown_variable:1 needs 0"
lackany "door: and prints nothing of the project's text" "$r" "ZZLeakMarker"

r="$(door pj_pkg "$S/d_pkg0.pdf")"
expect "door: a package that is not supplied is an error, and a need" "$r" "door error pages none kinds package:1 needs 1"
r="$(door pj_pkg "$S/d_pkg1.pdf" --packages "$S/pk")"
expect "door: the same project with the package directory is ok" "$r" "door ok pages 1 kinds none needs 0"
r="$(door pj_pkg "$S/d_pkg2.pdf" --packages "$S/pk" --exclude assets)"
expect "door: an --exclude of a directory without sources changes nothing" "$r" "door ok pages 1 kinds none needs 0"

PACKS="$HOME/usr/code/web/apps/oxedyne/daimond/www/assets/typst/packs"
if [ -f "$PACKS/preview/cetz/0.3.4.pack" ]; then
	mkproj "$S/pj_cetz" "$HEAD"$'\n#import "@preview/oxifmt:0.2.1": strfmt\n#import "@preview/cetz:0.3.4": canvas, draw\n#strfmt("{} and {}", 1, 2)'
	r="$(door pj_cetz "$S/d_cetz.pdf" --packages "$PACKS")"
	expect "door: a project that imports two of Daimond's packs, one the other's dependency, is ok" "$r" "door ok pages 1 kinds none needs 0"
else
	echo "selftest: skip real packs (Daimond's packs are absent)"
fi

r="$(gate_cap node "$CRATE/tools/gate/door_pdf.mjs" "$PKG" 2>&1)"
expect "door: too few arguments is a closed word" "$r" "door fail bad-args"
r="$(gate_cap node "$CRATE/tools/gate/door_pdf.mjs" "$S/nowhere" "$S/pj_ok" main.typ assets/fonts "$S/x.pdf" 2>&1)"
expect "door: a package directory that is not there is a closed word" "$r" "door fail bad-pkg"
r="$(gate_cap node "$CRATE/tools/gate/door_pdf.mjs" "$PKG" "$S/pj_ok" nothing.typ assets/fonts "$S/x.pdf" 2>&1)"
expect "door: a main that is not there is a closed word" "$r" "door fail no-main"
r="$(gate_cap node "$CRATE/tools/gate/door_pdf.mjs" "$PKG" "$S/pj_ok" main.typ assets/fonts "$S/x.pdf" --packages "$S/nowhere" 2>&1)"
expect "door: a package directory that is not there is refused" "$r" "door fail no-packages"

# The plants, on a copy of the tools.
cp -r "$CRATE/tools" "$S/tools_p"
D0="$CRATE/tools/gate/door_pdf.mjs"
DP="$S/tools_p/gate/door_pdf.mjs"
door_plant() { # door_plant <label> <old> <new> <project> <line the planted copy must not print> [options...]
	local label="$1" old="$2" new="$3" p="$4" want="$5" r
	shift 5
	if ! subst "$D0" "$DP" "$old" "$new"; then
		fail "plant $label: the text it replaces has moved or repeats in door_pdf.mjs"
		return
	fi
	r="$(TOOLS="$S/tools_p" door "$p" "$S/dp.pdf" "$@")"
	if printf '%s\n' "$r" | grep -qxF -- "$want"; then
		fail "plant $label: the planted copy still prints: $want"
	else
		ok "plant $label: the planted copy no longer prints the line, so the check is red"
	fi
	rm -f "${S:?}/dp.pdf"
}
door_plant "door not strict" 'strict: true }' 'strict: false }' pj_font "door error pages 1 kinds missing_font:2 needs 0"
door_plant "door packages not supplied" 'of g.packages) {' 'of []) {' pj_pkg "door ok pages 1 kinds none needs 0" --packages "$S/pk"
door_plant "door kinds not counted" "return keys.length ? keys.map((k) => \`\${k}:\${n[k]}\`).join(',') : 'none';" "return 'none';" pj_font "door error pages 1 kinds missing_font:2 needs 0"
door_plant "door needs not counted" 'const needs = first.r && Array.isArray(first.r.needs) ? first.r.needs.length : 0;' 'const needs = 0;' pj_pkg "door error pages none kinds package:1 needs 1"
door_plant "door second compile not run" 'if (a.again) {' 'if (false) {' pj_ok "door again ok" --again "$S/d_ok3.pdf"

cp "$D0" "$DP"

# ── 3. gate.sh end to end ───────────────────────────────────────────────────
gate() { # gate <project> [tools dir]: gate.sh, with this project's packages and the CLI's cache beside them
	local p="$1" t="${2:-$CRATE/tools}"
	TYPST_PACKAGE_CACHE_PATH="$S/pk" GATE_PACKAGES="$S/pk" GATE_OUT="$S/gout" \
		"$t/gate/gate.sh" "$S/$p" main.typ assets/fonts "$BIN" "$PKG" 2>&1
}
r="$(gate pj_ok)"
echo "$r" | sed 's/^/    /'
n="$(printf '%s\n' "$r" | grep -vc '^gate: ')"
[ "$n" -eq 0 ] && ok "gate: every line is a gate: line" || fail "gate: $n stray line(s)"
expect "gate: typst compiled" "$r" "gate: typst exit 0"
expect "gate: the CLI compiled strict" "$r" "gate: cli exit 0"
expect "gate: the door compiled strict, with the CLI's pages" "$r" "gate: door ok pages 3 kinds none needs 0"
expect "gate: the door's PDF is the CLI's, byte for byte" "$r" "gate: door same-as-cli"
expect "gate: G12 is green" "$r" "gate: table g12 green"
for row in cli door; do
	for g in g1 g2 g4 g5 g8; do
		expect "gate: $row $g is green on a project that agrees" "$r" "gate: table $row $g green"
	done
	for g in g3 g7; do
		expect "gate: $row $g is measured" "$r" "gate: table $row $g measured"
	done
done
expect "gate: the verdict is green" "$r" "gate: verdict green"
[ -z "$(ls -A "$S/gout")" ] && ok "gate: its scratch is gone on exit" || fail "gate: left $(ls -A "$S/gout" | wc -l) entr(ies) behind"
GATE_OUT="$S/gout" "$CRATE/tools/gate/gate.sh" "$S/pj_ok" main.typ assets/fonts "$BIN" "$PKG" > /dev/null 2>&1
[ $? -eq 0 ] && ok "gate: the exit status is 0 on a green verdict" || fail "gate: a green verdict did not exit 0"

r="$(gate pj_font)"
echo "$r" | sed 's/^/    /'
expect "gate: a project that strict refuses has no CLI checks" "$r" "gate: cli checks skipped"
expect "gate: and the door refuses it by kind" "$r" "gate: door error pages 1 kinds missing_font:2 needs 0"
expect "gate: and the verdict is red" "$r" "gate: verdict red"
lackany "gate: and nothing of the project's text is printed" "$r" "ZZLeakMarker"
GATE_OUT="$S/gout" "$CRATE/tools/gate/gate.sh" "$S/pj_font" main.typ assets/fonts "$BIN" "$PKG" > /dev/null 2>&1
[ $? -eq 1 ] && ok "gate: the exit status is 1 on a red verdict" || fail "gate: a red verdict did not exit 1"
r="$(gate pj_syn)"
expect "gate: a project that typst cannot compile is a closed word" "$r" "gate: error typst-failed"
lackany "gate: and nothing of the project's text is printed" "$r" "ZZLeakMarker"
r="$(GATE_OUT="$S/gout" "$CRATE/tools/gate/gate.sh" 2>&1)"
expect "gate: no arguments is a usage error" "$r" "gate: error usage"

# The premise of the leak plants: the CLI's own stderr quotes the project's text.
raw="$(cd "$S/pj_font" && gate_hidden "$BIN" --eval --strict --diag-summary --root . --font-path assets/fonts main.typ "$S/rawout" 2>&1)"
printf '%s\n' "$raw" | grep -qiF ZZLeakMarker && ok "leak premise: the CLI's own stderr quotes the project's text" \
	|| fail "leak premise: the CLI did not quote the marker, so the next checks prove nothing"

# The plants, on a copy of the tools.
cp "$CRATE/tools/gate/gate.sh" "$S/gate0.sh"
GP="$S/tools_p/gate/gate.sh"
# A fault in a required check, planted into g_checks.py, must turn the verdict red; the table logic that
# ignores it is the plant that proves that test.
if subst "$G" "$S/tools_p/gate/g_checks.py" 'same = len(ma) == len(mb) and not differ' 'same = False'; then
	r="$(gate pj_ok "$S/tools_p")"
	expect "gate: a required check that is red is red in the table" "$r" "gate: table cli g1 red"
	expect "gate: and turns the verdict red" "$r" "gate: verdict red"
	if subst "$S/gate0.sh" "$GP" 'if [ "$v" = same ]; then' 'if true; then'; then
		r="$(gate pj_ok "$S/tools_p")"
		if printf '%s\n' "$r" | grep -qxF "gate: verdict red"; then
			fail "plant gate red ignored: the planted copy is still red"
			printf '%s\n' "$r" | grep -v 'g[1-8] \|g12 ' | sed 's/^/    /' 
		else
			ok "plant gate red ignored: the planted copy reads green with a red check, so the test is red"
		fi
	else
		fail "plant gate red ignored: the text it replaces has moved in gate.sh"
	fi
else
	fail "plant: g_checks.py's G1 verdict line has moved"
fi
cp "$G" "$S/tools_p/gate/g_checks.py"
# Defence in depth: the stage filter alone does not let the text out, the stage filter and the final whitelist
# together do.
if subst "$S/gate0.sh" "$GP" '/: warning: / { w++; next }' '{ print } /: warning: / { w++; next }'; then
	r="$(gate pj_font "$S/tools_p")"
	lackany "gate: with the CLI filter broken the final whitelist still holds" "$r" "ZZLeakMarker"
	python3 - "$GP" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read()
i = s.index('# The final whitelist')
open(p, 'w').write(s[:i] + 'main "$@" 2>&1\nexit "$?"\n')
PY
	r="$(gate pj_font "$S/tools_p")"
	if printf '%s\n' "$r" | grep -qiF ZZLeakMarker; then
		ok "plant gate whitelist: with the CLI filter and the final whitelist both gone the text is printed, so the filters are what holds it"
	else
		fail "plant gate whitelist: the text was not printed with both filters gone, so the premise is wrong"
	fi
else
	fail "plant: the CLI filter's first line has moved in gate.sh"
fi
rm -rf "${S:?}/gout"
mkdir -p "$S/gout"
if subst "$S/gate0.sh" "$GP" "trap 'rm -rf \"\${W:?}\"' EXIT" "trap '' EXIT"; then
	gate pj_ok "$S/tools_p" > /dev/null
	[ -n "$(ls -A "$S/gout")" ] && ok "plant gate scratch: with the trap gone the scratch is left behind, so the test is red" \
		|| fail "plant gate scratch: the planted copy left nothing behind"
else
	fail "plant: gate.sh's trap has moved"
fi

echo "selftest: $bad failure(s)"
[ "$bad" -eq 0 ]
