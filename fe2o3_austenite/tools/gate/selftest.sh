#!/usr/bin/env bash
# Proves the O1 tools on synthetic projects, and proves each proof can fail.
#
#     selftest.sh <typst-vendor> <scratch>
#
# <typst-vendor> is Daimond's `www/vendor/typst`; <scratch> is a directory this script makes a child of and
# removes on exit. Everything here is synthetic: no path to a real project is taken or needed.
#
# It checks, in order:
#   1. gather.mjs on a project of three .typ files, a csv, an image and a supplied font, with --exclude;
#   2. g_checks.py: identical PDFs agree; a pair that differs on page 2 goes red on page 2; a middle-token
#      change moves the similarity but no hash; and a copy with the plant, page i compared with page i+1,
#      is blind to the pair that differs, so the check on it goes red;
#   3. skew.sh end to end: a project that agrees reports `same` on the row that shares fonts; a project
#      whose page 4 prints the compiler's minor version reports `differs` on page 4 in both rows; a project
#      that does not compile prints a closed vocabulary and none of its own text.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE="$(cd "$HERE/../.." && pwd)"
# shellcheck source=gate_lib.sh
. "$HERE/gate_lib.sh"

V="${1:?usage: selftest.sh <typst-vendor> <scratch>}"
PARENT="${2:?usage: selftest.sh <typst-vendor> <scratch>}"
TYPST="${SKEW_TYPST:-/home/jason/bin/typst}"
mkdir -p "$PARENT" || exit 2
S="$(mktemp -d "$PARENT/o1.XXXXXX")" || exit 2
trap 'rm -rf "${S:?}"' EXIT
GATE_SCRATCH="$S"

bad=0
ok()   { echo "selftest: ok   $1"; }
fail() { echo "selftest: FAIL $1"; bad=$((bad + 1)); }
expect() { # expect <label> <text> <fixed line the text must contain>
	if printf '%s\n' "$2" | grep -qxF -- "$3"; then ok "$1"; else fail "$1 (wanted: $3)"; fi
}
lack() { # lack <label> <text> <fixed string the text must not contain>
	if printf '%s\n' "$2" | grep -qF -- "$3"; then fail "$1 (found: $3)"; else ok "$1"; fi
}

# ── the synthetic projects ──────────────────────────────────────────────────
mkproj() { # mkproj <dir> [extra page text]
	local p="$1"
	mkdir -p "$p/chap" "$p/img" "$p/data" "$p/assets/fonts" "$p/skip"
	cp "$CRATE/fonts/LibertinusMono-Regular.otf" "$p/assets/fonts/"
	python3 - "$p/img/mark.png" <<'PY'
import sys
from PIL import Image
Image.new('RGB', (8, 8), (200, 40, 40)).save(sys.argv[1])
PY
	printf 'a,1\nb,2\n' > "$p/data/rows.csv"
	printf 'junk that --exclude must leave out\n' > "$p/skip/note.txt"
	printf '#let one = [One: some words on the first page.]\n' > "$p/chap/one.typ"
	printf '#let two = [Two: #csv("../data/rows.csv").map(r => r.join("-")).join(" ")]\n' > "$p/chap/two.typ"
	cat > "$p/main.typ" <<'TYP'
#set page(width: 200pt, height: 120pt, margin: 12pt)
#set text(size: 10pt)
#import "chap/one.typ": one
#import "chap/two.typ": two
= Synthetic
#image("img/mark.png", width: 24pt)
#text(font: "Libertinus Mono")[mono face]
#one
#pagebreak()
#two
#pagebreak()
Third page.
TYP
	if [ -n "${2:-}" ]; then
		printf '#pagebreak()\n%s\n' "$2" >> "$p/main.typ"
	fi
}
mkproj "$S/proj"
mkproj "$S/skewed" 'Compiler minor #sys.version.at(1).'
mkdir -p "$S/broken"
printf 'Text #ZZLEAKMARKERZZ more.\n' > "$S/broken/main.typ"

# Synthetic packages. `tpk` is Typst's own layout, which typst 0.15.1 reads through TYPST_PACKAGE_PATH; `pk`
# is what the typst.ts side is given: one package as a directory and two as Daimond's packs. zzpkg imports
# zzdep, by a registry import and by a package-root path, so the closure and both re-pointings are used.
pkgfile() { # pkgfile <relative path> <text>
	mkdir -p "$(dirname "$S/tpk/$1")"
	printf '%s\n' "$2" > "$S/tpk/$1"
}
pkgfile preview/zzpkg/0.1.0/typst.toml $'[package]\nname = "zzpkg"\nversion = "0.1.0"\nentrypoint = "lib.typ"'
pkgfile preview/zzpkg/0.1.0/lib.typ $'#import "@preview/zzdep:0.3.0": dep\n#import "/helper.typ": help\n#let zz(x) = [zz(#x) #dep() #help()]'
pkgfile preview/zzpkg/0.1.0/helper.typ '#let help() = [helper]'
pkgfile preview/zzdep/0.3.0/typst.toml $'[package]\nname = "zzdep"\nversion = "0.3.0"\nentrypoint = "lib.typ"'
pkgfile preview/zzdep/0.3.0/lib.typ '#let dep() = [dependency]'
pkgfile preview/zzpack/0.2.0/typst.toml $'[package]\nname = "zzpack"\nversion = "0.2.0"\nentrypoint = "src/lib.typ"'
pkgfile preview/zzpack/0.2.0/src/lib.typ '#let pk() = [packed]'
pkgfile preview/zzpack/0.2.0/LICENSE 'Synthetic.'
mkdir -p "$S/pk/preview/zzpkg" "$S/pk/preview/zzdep" "$S/pk/preview/zzpack"
cp -r "$S/tpk/preview/zzpkg/0.1.0" "$S/pk/preview/zzpkg/0.1.0"
mkpack() { # mkpack <ns> <name> <version> <out dir>: Daimond's refresh.sh format, from the tpk directory
	python3 - "$S/tpk/$1/$2/$3" "$1" "$2" "$3" "$4/$1/$2/$3.pack" <<'PY'
import os, re, sys
src, ns, name, ver, out = sys.argv[1:]
files = []
for dp, _, fs in os.walk(src):
	for f in fs:
		rel = os.path.relpath(os.path.join(dp, f), src)
		if f.endswith('.typ') or f == 'typst.toml' or f == 'LICENSE' or f.startswith('LICENSE.'):
			files.append(rel)
files.sort()
entry = re.search(r'^\s*entrypoint\s*=\s*"(.*)"', open(os.path.join(src, 'typst.toml')).read(), re.M).group(1)
head = 'DAIMOND TYPST PACK 1\nnamespace %s\nname %s\nversion %s\nentrypoint %s\n' % (ns, name, ver, entry)
body = b''
for f in files:
	data = open(os.path.join(src, f), 'rb').read()
	head += 'file %d %s\n' % (len(data), f)
	body += data
os.makedirs(os.path.dirname(out), exist_ok=True)
open(out, 'wb').write(head.encode() + b'\n' + body)
PY
}
mkpack preview zzdep 0.3.0 "$S/pk"
mkpack preview zzpack 0.2.0 "$S/pk"
mkproj "$S/pkgproj"
cat >> "$S/pkgproj/main.typ" <<'TYP'
#pagebreak()
#import "@preview/zzpkg:0.1.0": zz
#import "@preview/zzpack:0.2.0": pk
#zz([a]) #pk()
Quoted, not imported: `#import "@preview/zzghost:1.0.0"` and the string below.
#let s = "@preview/zzghost2:1.0.0"
TYP

# ── 1. gather ───────────────────────────────────────────────────────────────
g="$(gate_cap node "$CRATE/tools/bench/lib/gather.mjs" "$S/proj" main.typ --exclude skip 2>&1)"
echo "$g" | grep -q '^gather: sources 4 assets 1 fonts 1 packages 0 missing 0 bytes [0-9]*$' \
	&& ok "gather: 3 .typ + csv as sources, the image as an asset, the font as a font, --exclude honoured" \
	|| fail "gather counts: $g"
g="$(gate_cap node "$CRATE/tools/bench/lib/gather.mjs" "$S/proj" main.typ 2>&1)"
echo "$g" | grep -q '^gather: sources 5 assets 1 fonts 1 packages 0 missing 0 bytes [0-9]*$' \
	&& ok "gather: without --exclude the skipped file is gathered" || fail "gather without exclude: $g"
mkdir -p "$S/pkg"
printf '#import "@preview/zzz:0.0.1": f\n' > "$S/pkg/main.typ"
g="$(gate_cap node "$CRATE/tools/bench/lib/gather.mjs" "$S/pkg" main.typ 2>&1)"
echo "$g" | grep -q '^gather: sources 1 assets 0 fonts 0 packages 1 missing 1 bytes [0-9]*$' \
	&& ok "gather: a package import with no package directory is a spec found and missing" \
	|| fail "gather packages: $g"
# Links: a directory reached by two names is gathered under both, a link back up the tree is not followed
# for ever, and a link that points nowhere is passed over.
mkdir -p "$S/links/a"
printf '#set page(width: 100pt)\n' > "$S/links/main.typ"
printf 'x\n' > "$S/links/a/x.txt"
ln -s a "$S/links/b"
ln -s . "$S/links/up"
ln -s nowhere "$S/links/dead"
g="$(gate_cap node "$CRATE/tools/bench/lib/gather.mjs" "$S/links" main.typ 2>&1)"
echo "$g" | grep -q '^gather: sources 3 assets 0 fonts 0 packages 0 missing 0 bytes [0-9]*$' \
	&& ok "gather: a directory under two names counts under both; a link back up and a dangling link do no harm" \
	|| fail "gather links: $g"
g="$(gate_cap node "$CRATE/tools/bench/lib/gather.mjs" "$S/pkgproj" main.typ --exclude skip --packages "$S/pk" 2>&1)"
echo "$g" | grep -q '^gather: sources 4 assets 1 fonts 1 packages 3 missing 0 bytes [0-9]*$' \
	&& ok "gather: the closure finds 3 packages (a directory, two packs, one a dependency); a quoted import and a string are not imports" \
	|| fail "gather package closure: $g"
g="$(gate_cap node "$CRATE/tools/bench/lib/gather.mjs" "$S/pkgproj" main.typ --exclude skip 2>&1)"
echo "$g" | grep -q '^gather: sources 4 assets 1 fonts 1 packages 2 missing 2 bytes [0-9]*$' \
	&& ok "gather: with no package directory the 2 imported specs are missing" || fail "gather no packages: $g"
cp -r "$S/pk" "$S/pk_nodep" && rm -rf "${S:?}/pk_nodep/preview/zzdep"
g="$(gate_cap node "$CRATE/tools/bench/lib/gather.mjs" "$S/pkgproj" main.typ --exclude skip --packages "$S/pk_nodep" 2>&1)"
echo "$g" | grep -q '^gather: sources 4 assets 1 fonts 1 packages 3 missing 1 bytes [0-9]*$' \
	&& ok "gather: a dependency that the directory lacks is missing, the rest supplied" || fail "gather missing dependency: $g"
cp -r "$S/pk" "$S/pk_bad" && head -c 100 "$S/pk/preview/zzpack/0.2.0.pack" > "$S/pk_bad/preview/zzpack/0.2.0.pack"
g="$(gate_cap node "$CRATE/tools/bench/lib/gather.mjs" "$S/pkgproj" main.typ --exclude skip --packages "$S/pk_bad" 2>&1)"
expect "gather: a truncated pack is refused with a closed code" "$g" "gather: error bad-package"
g="$(gate_cap node "$CRATE/tools/bench/lib/gather.mjs" "$S/pkgproj" main.typ --packages "$S/nowhere" 2>&1)"
expect "gather: a package directory that does not exist is refused" "$g" "gather: error no-packages"
g="$(gate_cap node "$CRATE/tools/bench/lib/gather.mjs" "$S/proj" nothing.typ 2>&1)"
expect "gather: a missing main is refused with a closed code" "$g" "gather: error no-main"

# ── 2. g_checks ─────────────────────────────────────────────────────────────
mkpdf() { # mkpdf <name> <page text>... : one page per argument
	local name="$1"
	shift
	{
		printf '#set page(width: 200pt, height: 80pt, margin: 10pt)\n'
		local first=1 t
		for t in "$@"; do
			[ $first -eq 1 ] || printf '#pagebreak()\n'
			first=0
			printf '%s\n' "$t"
		done
	} > "$S/$name.typ"
	gate_typst "$TYPST" compile "$S/$name.typ" "$S/$name.pdf" > /dev/null 2>&1
}
P1="Alpha one two three"
P2="Beta four five six"
P2x="Beta four 5 six"
mkpdf A "$P1" "$P2" "$P2"
mkpdf B "$P1" "$P1" "$P2"
mkpdf C "$P1" "$P2x" "$P2"
mkpdf D "$P1" "Beta four five seven" "$P2"
mkpdf E "$P1" "Gamma four five six" "$P2"
G="$HERE/g_checks.py"
r="$(python3 "$G" compare "$S/A.pdf" "$S/A.pdf")"
expect "g_checks: a PDF agrees with itself" "$r" "g3 verdict same"
expect "g_checks: no page differs" "$r" "g3 differ 0 pages none"
r="$(python3 "$G" compare "$S/A.pdf" "$S/B.pdf")"
expect "g_checks: the pair that differs on page 2 goes red, on page 2" "$r" "g3 differ 1 pages 2"
expect "g_checks: and the verdict is differs" "$r" "g3 verdict differs"
r="$(python3 "$G" compare "$S/A.pdf" "$S/C.pdf")"
expect "g_checks: a middle-token change moves no hash" "$r" "g3 differ 0 pages none"
expect "g_checks: but the verdict is differs" "$r" "g3 verdict differs"
lack "g_checks: and the similarity is below 1" "$r" "g3 min-similarity 1.000"
r="$(python3 "$G" compare "$S/A.pdf" "$S/D.pdf")"
expect "g_checks: a change of the last token alone is a hash difference" "$r" "g3 differ 1 pages 2"
r="$(python3 "$G" compare "$S/A.pdf" "$S/E.pdf")"
expect "g_checks: so is a change of the first token alone" "$r" "g3 differ 1 pages 2"
r="$(python3 "$G" pages "$S/A.pdf")"
[ "$(printf '%s\n' "$r" | wc -l)" -eq 3 ] && ok "g_checks: pages lists 3 pages" || fail "g_checks pages: $r"
l2="$(printf '%s\n' "$r" | sed -n 2p | awk '{ print $4 }')"
l3="$(printf '%s\n' "$r" | sed -n 3p | awk '{ print $4 }')"
[ -n "$l2" ] && [ "$l2" = "$l3" ] && ok "g_checks: equal pages hash alike" || fail "g_checks hashes: $l2 / $l3"

# The plant: page i is compared with page i+1. The pair A and B was made so that page i of A is page i+1
# of B, which that fault cannot see; the same check that is green on the real file must go red on the copy.
sed 's/a, b = pa\[i\], pb\[i\]/a, b = pa[i], pb[min(i + 1, n - 1)]/' "$G" > "$S/g_checks_plant.py"
if cmp -s "$G" "$S/g_checks_plant.py"; then
	fail "plant: the sed did not change g_checks.py (its compare line moved)"
else
	r="$(python3 "$S/g_checks_plant.py" compare "$S/A.pdf" "$S/B.pdf")"
	if printf '%s\n' "$r" | grep -qxF "g3 differ 1 pages 2"; then
		fail "plant: g_checks with page i against page i+1 still found the difference"
	else
		ok "plant: page i against page i+1 reports no difference on the pair that differs, so the check is red"
	fi
fi

# ── 3. skew.sh ──────────────────────────────────────────────────────────────
export SKEW_OUT="$S/out"
export SKEW_EXCLUDE="skip"
r="$("$HERE/skew.sh" "$S/proj" main.typ assets/fonts "$V" 2>&1)"
echo "$r" | sed 's/^/    /'
n="$(printf '%s\n' "$r" | grep -vc '^skew: ')"
[ "$n" -eq 0 ] && ok "skew: every line is a skew: line" || fail "skew: $n stray line(s)"
expect "skew: the gather saw 4 sources, an asset and a font" "$r" "skew: typstts gathered sources 4 assets 1 fonts 1 packages 0 missing 0"
expect "skew: typst.ts compiled" "$r" "skew: typstts compile ok"
expect "skew: row a typst exit 0" "$r" "skew: row a typst exit 0"
expect "skew: row b typst exit 0" "$r" "skew: row b typst exit 0"
expect "skew: row b has 3 pages on both" "$r" "skew: row b pages typstts 3 typst 3"
expect "skew: row b, the same font files, agrees" "$r" "skew: row b verdict same"
[ -z "$(ls -A "$S/out")" ] && ok "skew: its scratch is gone on exit" || fail "skew: left $(ls -A "$S/out" | wc -l) entr(ies) behind"

r="$("$HERE/skew.sh" "$S/skewed" main.typ assets/fonts "$V" 2>&1)"
echo "$r" | sed 's/^/    /'
expect "skew: the page that prints the compiler version goes red in row a" "$r" "skew: row a differ 1 pages 4"
expect "skew: and in row b" "$r" "skew: row b differ 1 pages 4"
expect "skew: and the verdict is differs" "$r" "skew: row b verdict differs"

# Packages: supplied to typst.ts from `pk`, and resolved by typst 0.15.1 from its own layout of the same.
r="$(TYPST_PACKAGE_PATH="$S/tpk" SKEW_PACKAGES="$S/pk" "$HERE/skew.sh" "$S/pkgproj" main.typ assets/fonts "$V" 2>&1)"
echo "$r" | sed 's/^/    /'
n="$(printf '%s\n' "$r" | grep -vc '^skew: ')"
[ "$n" -eq 0 ] && ok "skew: every line is a skew: line, packages included" || fail "skew: $n stray line(s)"
expect "skew: 3 packages found, none missing" "$r" "skew: typstts gathered sources 4 assets 1 fonts 1 packages 3 missing 0"
expect "skew: the directory package is named" "$r" "skew: typstts package preview zzpkg 0.1.0 supplied"
expect "skew: a pack is named" "$r" "skew: typstts package preview zzpack 0.2.0 supplied"
expect "skew: and so is the dependency found by the closure" "$r" "skew: typstts package preview zzdep 0.3.0 supplied"
expect "skew: typst.ts compiled the project that imports them" "$r" "skew: typstts compile ok"
expect "skew: 4 pages on both, row b" "$r" "skew: row b pages typstts 4 typst 4"
expect "skew: and the pages agree, row b" "$r" "skew: row b verdict same"
r="$(TYPST_PACKAGE_PATH="$S/tpk" SKEW_PACKAGES="$S/pk_nodep" "$HERE/skew.sh" "$S/pkgproj" main.typ assets/fonts "$V" 2>&1)"
expect "skew: a package that is not supplied is named missing" "$r" "skew: typstts package preview zzdep 0.3.0 missing"
expect "skew: and typst.ts fails" "$r" "skew: typstts compile failed"
expect "skew: and the row is incomparable, not different" "$r" "skew: row b verdict incomparable"

# Daimond's own vendored packs, where this host has them and Typst's cache holds the same versions: a
# synthetic project that draws with each of them compiles in typst.ts, so the port reads the real format.
PACKS="$V/../../assets/typst/packs"
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/typst/packages/preview"
if [ -d "$PACKS" ] && [ -d "$CACHE/cetz/0.3.4" ] && [ -d "$CACHE/fletcher/0.5.7" ] && [ -d "$CACHE/cetz-plot/0.1.1" ] && [ -d "$CACHE/oxifmt/0.2.1" ]; then
	mkdir -p "$S/real"
	cat > "$S/real/main.typ" <<'TYP'
#set page(width: 220pt, height: 160pt, margin: 10pt)
#import "@preview/cetz:0.3.4": canvas, draw
#import "@preview/fletcher:0.5.7": diagram, node, edge
#import "@preview/cetz-plot:0.1.1": plot
#import "@preview/oxifmt:0.2.1": strfmt
= Real packs
#canvas({
  import draw: *
  circle((0, 0), radius: 1)
})
#pagebreak()
#diagram(node((0,0), [A]), edge("->"), node((1,0), [B]))
#pagebreak()
#strfmt("{} and {}", 1, 2)
TYP
	r="$(SKEW_PACKAGES="$PACKS" "$HERE/skew.sh" "$S/real" main.typ assets/fonts "$V" 2>&1)"
	expect "real packs: cetz, fletcher, cetz-plot, oxifmt and cetz 0.3.2 by closure, none missing" "$r" "skew: typstts gathered sources 1 assets 0 fonts 0 packages 5 missing 0"
	expect "real packs: typst.ts compiled the project" "$r" "skew: typstts compile ok"
	expect "real packs: and agrees with typst 0.15.1 on every page, row a" "$r" "skew: row a verdict same"
else
	echo "selftest: skip real packs (Daimond's packs or Typst's cache of the same versions are absent)"
fi

# The supplied font and the image reached typst.ts: the PDF says so, which text alone would not.
gate_cap node "$HERE/skew_ts.mjs" --root "$S/proj" --main main.typ --vendor "$V" --out "$S/ts.pdf" \
	--font-dir assets/fonts --exclude skip > /dev/null 2>&1
pdffonts "$S/ts.pdf" 2>/dev/null | grep -q LibertinusMono \
	&& ok "typst.ts typeset in the supplied font (add_raw_font)" || fail "typst.ts did not embed the supplied font"
n="$(pdfimages -list "$S/ts.pdf" 2>/dev/null | tail -n +3 | wc -l)"
[ "$n" -eq 1 ] && ok "typst.ts placed the image (map_shadow)" || fail "typst.ts placed $n image(s), wanted 1"

# Every typst 0.15.1 run hides the account's own fonts: a fake home holds a face typst does not carry.
mkdir -p "$S/home/.local/share/fonts"
cp "$CRATE/fonts/LibertinusMono-Regular.otf" "$S/home/.local/share/fonts/"
printf '#set text(font: "Libertinus Mono")\nMono\n' > "$S/iso.typ"
( unset XDG_DATA_HOME; HOME="$S/home" gate_cap "$TYPST" compile "$S/iso.typ" "$S/iso_open.pdf" ) > /dev/null 2>&1
pdffonts "$S/iso_open.pdf" 2>/dev/null | grep -q LibertinusMono \
	&& ok "isolation premise: an open run finds the account's face" || fail "isolation premise: an open run did not find the face"
HOME="$S/home" gate_typst "$TYPST" compile "$S/iso.typ" "$S/iso_hidden.pdf" > /dev/null 2>&1
pdffonts "$S/iso_hidden.pdf" 2>/dev/null | grep -q LibertinusMono \
	&& fail "gate_typst let the account's face through" || ok "gate_typst hides the account's own fonts"

# A project that does not compile: typst quotes its source in the error, and none of it may be printed.
raw="$(cd "$S/broken" && gate_typst "$TYPST" compile --root . main.typ "$S/broken.pdf" 2>&1)"
printf '%s\n' "$raw" | grep -qF ZZLEAKMARKERZZ && ok "leak premise: typst's own stderr quotes the project's text" \
	|| fail "leak premise: typst did not quote the marker, so the next check proves nothing"
r="$("$HERE/skew.sh" "$S/broken" main.typ assets/fonts "$V" 2>&1)"
echo "$r" | sed 's/^/    /'
lack "skew: nothing of a broken project's text is printed" "$r" ZZLEAKMARKERZZ
expect "skew: a typst failure is a closed word" "$r" "skew: row a verdict incomparable"
expect "skew: and a typst.ts failure too" "$r" "skew: typstts compile failed"
n="$(printf '%s\n' "$r" | grep -vc '^skew: ')"
[ "$n" -eq 0 ] && ok "skew: a failing run prints only skew: lines" || fail "skew: $n stray line(s) on a failing run"

echo "selftest: $bad failure(s)"
[ "$bad" -eq 0 ]
