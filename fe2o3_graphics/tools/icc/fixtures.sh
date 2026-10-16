#!/bin/bash
# Prints the numbers that tests/icc.rs holds, each from LittleCMS 2.17 (`transicc`, lcms2-utils) run
# once over the swatch set.  The test file carries the numbers; this script is how they were made, and
# what to run to make them again.
#
# Ghostscript's profiles are AGPL and are read here from their system path only; none is copied into
# the tree, and tests that read them are #[ignore]d.  FOGRA39L Coated is the bundled profile.
#
#   tools/icc/fixtures.sh > /dev/null         # the reader's arrays on stdout, the descriptions on stderr
#   tools/icc/fixtures.sh xform > tests/icc/xform_data.rs   # the transform's arrays, for tests/icc/xform.rs
set -euo pipefail
GS=/usr/share/color/icc/ghostscript
FOGRA="$(dirname "$0")/../../data/icc/FOGRA39L_coated.icc"
T="transicc -n"

# Runs transicc over stdin and prints the last $1 numbers of each line, whatever the banner says.
col() { $T "${@:2}" 2>/dev/null | grep -E '^[-0-9. ]+$' | awk -v n="$1" '{ for (i = 1; i <= n; i++) printf("%s%s", $i, i < n ? ", " : ""); printf("\n") }'; }
arr() {
	echo "const $1: &[$2] = &["
	if [ "${2:0:1}" = "[" ]; then sed 's/^/\t[/; s/$/],/'; else sed 's/^/\t/; s/$/,/'; fi
	echo "];"
}

# The 5-step RGB cube, red slowest, and a grey ramp of 18 steps.
cube() { for r in 0 64 128 191 255; do for g in 0 64 128 191 255; do for b in 0 64 128 191 255; do echo "$r $g $b"; done; done; done; }
ramp() { for i in $(seq 0 17); do echo "$((i * 15))"; done; }
ramp3() { ramp | awk '{ print $1, $1, $1 }'; }
corners() { for r in 0 255; do for g in 0 255; do for b in 0 255; do echo "$r $g $b"; done; done; done; echo "128 128 128"; }

f6() {
	# 1. RGB to grey, sRGB to the Ghostscript sGray (the sRGB curve on D50 Y): the 125-colour cube, then the ramp.
	cube | col 1 -i '*sRGB' -o $GS/default_gray.icc | arr CUBE_GREY f64
	ramp3 | col 1 -i '*sRGB' -o $GS/default_gray.icc | arr RAMP_GREY f64

	# 2. RGB to XYZ (D50, percent), sRGB: the eight corners and mid grey.
	corners | col 3 -i '*sRGB' -o '*XYZ' | arr CORNER_XYZ '[f64; 3]'

	# 3. sGray to XYZ (percent): the grey ramp's Y.
	ramp | col 3 -i $GS/default_gray.icc -o '*XYZ' | arr RAMP_XYZ '[f64; 3]'

	# 4. The same cube through the Ghostscript default_rgb, for the #[ignore]d test of the matrix read from a file.
	cube | col 1 -i $GS/default_rgb.icc -o $GS/default_gray.icc | arr CUBE_GREY_GS f64
	corners | col 3 -i $GS/default_rgb.icc -o '*XYZ' | arr CORNER_XYZ_GS '[f64; 3]'

	# 5. FOGRA39L Coated, relative colorimetric (A2B1), CMYK corners to Lab; C slowest, K fastest.
	for c in 0 100; do for m in 0 100; do for y in 0 100; do for k in 0 100; do echo "$c $m $y $k"; done; done; done; done \
		| col 3 -t1 -i "$FOGRA" -o '*Lab' | arr FOGRA_LAB '[f64; 3]'

	# 6. Ghostscript default_cmyk, relative colorimetric (its three intents share one table; the perceptual one would add
	#    the black point compensation lcms forces whenever the other end is a version 4 profile, which the Lab space is):
	#    A2B corners to Lab, and B2A (a lut8) corners from Lab to CMYK.
	for c in 0 100; do for m in 0 100; do for y in 0 100; do for k in 0 100; do echo "$c $m $y $k"; done; done; done; done \
		| col 3 -t1 -i $GS/default_cmyk.icc -o '*Lab' | arr GS_CMYK_LAB '[f64; 3]'
	for l in 0 100; do for a in -128 127; do for b in -128 127; do echo "$l $a $b"; done; done; done \
		| col 4 -t1 -i '*Lab' -o $GS/default_cmyk.icc | arr GS_LAB_CMYK '[f64; 4]'

	# 7. The descriptions, as `transicc -v3` prints them on the line after "Profile:".
	for p in "$FOGRA" $GS/default_rgb.icc $GS/default_gray.icc $GS/default_cmyk.icc; do
		echo "$(basename "$p"): $(echo '0 0 0' | transicc -v3 -i "$p" -o '*XYZ' 2>/dev/null | sed -n '/^Profile:/{n;p;q}')" >&2
	done
}

# The transform's fixtures: LittleCMS 2.17 over a swatch set, each array one intent and one setting of
# compensation (`-b`), named <profile>_T<intent>[B].  CMYK comes out as percent, grey as 0..255.
swatches() {
	for r in 0 128 255; do for g in 0 128 255; do for b in 0 128 255; do echo "$r $g $b"; done; done; done
	for v in 16 48 80 112 160 192 224 240; do echo "$v $v $v"; done
	echo "224 172 105"; echo "91 155 213"; echo "96 153 60"; echo "110 20 30"
	echo "240 200 220"; echo "128 128 0"; echo "0 128 128"; echo "101 67 33"
}
cmyks() {
	for k in 0 25 50 75 100; do echo "0 0 0 $k"; done
	echo "100 0 0 0"; echo "0 100 0 0"; echo "0 0 100 0"; echo "100 100 100 0"; echo "100 100 100 100"; echo "50 50 50 0"; echo "0 50 100 20"
}
greys() { for v in 0 32 64 96 128 160 192 224 255; do echo "$v"; done; }
# Prints one array per intent and compensation of `transicc` over stdin, the last $2 numbers of each line.
sweep() {
	local name=$1 n=$2; shift 2
	local inp; inp=$(cat)
	for t in 0 1 2 3; do
		for b in "" "-b"; do
			if [ "$t" = 3 ] && [ -n "$b" ]; then continue; fi
			echo "$inp" | col "$n" -t$t $b "$@" | arr "${name}_T${t}${b:+B}" "$(if [ "$n" = 1 ]; then echo f64; else echo "[f64; $n]"; fi)"
		done
	done
}
xform() {
	swatches | sweep FOGRA 4 -i '*sRGB' -o "$FOGRA"
	swatches | sweep SWOP 4 -i '*sRGB' -o $GS/default_cmyk.icc
	swatches | sweep RGB_GREY 1 -i '*sRGB' -o $GS/default_gray.icc
	cmyks | sweep CMYK_GREY 1 -i $GS/default_cmyk.icc -o $GS/default_gray.icc
	greys | sweep GREY_FOGRA 4 -i $GS/default_gray.icc -o "$FOGRA"
}

case "${1:-f6}" in
	f6)		f6;;
	xform)	xform | sed 's/^const /pub const /';;
	*)		echo "usage: $0 [f6|xform]" >&2; exit 2;;
esac
