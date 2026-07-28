#!/bin/bash
# Regenerates the depth, colour-type and interlace fixtures in this directory.
#
# Nothing here is a photograph, and nothing here was written by the codec under test: a short Python
# program writes each source image as a PPM or a PAM, ImageMagick turns that into a PNG of the
# requested bit depth, colour type and interlace method, and ImageMagick reads its own PNG back out
# as the PAM the test compares against. A codec that agrees only with itself has been tested against
# nothing.
#
# ImageMagick will quietly ignore `png:bit-depth` and `png:color-type` when the image it holds does
# not suit them, so the recipes below shape the image first -- quantising, greying, forcing a
# channel depth -- and the test re-reads each fixture's IHDR to check that what came out is what the
# fixture's name claims. A silently downgraded fixture is a hole in the matrix, not a passing test.
#
# The `tRNS` fixtures come out of ImageMagick's `PNG8:` writer, which is the only route it offers to
# a palette image with transparency at a sub-byte depth, and which needs a source holding few enough
# colours that the palette fits the depth. It will not write greyscale transparency at a sub-byte
# depth at all, so that combination is covered by a hand-built unit test in `png.rs` instead.
#
# The files `grey_trns.png`, `palette_trns.png`, `rgb_plain.png`, `rgb_trns.png` and
# `rgba_plain.png` are older, come from Pillow by way of `gen.py`, and are not touched here.
#
# Requires ImageMagick's `convert` and Python 3. Run from this directory.
set -eu
cd "$(dirname "$0")"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# The sizes: one at which every Adam7 pass is non-empty and partial, and a handful small enough that
# most passes hold nothing at all.
FULL=17x13
SMALL="1x1 3x2 5x1 1x5 9x9"

# ---- The sources, several per size: 8- and 16-bit, with and without alpha, and four palettes. ----
python3 - "$tmp" $FULL $SMALL <<'PYEOF'
import os
import sys

d = sys.argv[1]

def pix(x, y):
	"""A deterministic colour and alpha, chosen to leave no two channels equal."""
	r = (x * 47 + y * 11 + 3) % 256
	g = (x * 13 + y * 61 + 71) % 256
	b = (x * 29 + y * 89 + 151) % 256
	a = (x * 37 + y * 101 + 19) % 256
	return r, g, b, a

# The colours the palette sources draw on, in the order they are taken.
PAL = [
	(255, 80, 20), (0, 255, 0), (0, 0, 255), (255, 255, 0), (0, 255, 255), (255, 0, 255),
	(10, 20, 30), (200, 100, 50), (60, 180, 90), (90, 30, 220), (240, 240, 10), (5, 120, 200),
]

def write(name, header, px):
	open(os.path.join(d, name), "wb").write(header + bytes(px))

for spec in sys.argv[2:]:
	w, h = (int(v) for v in spec.split("x"))
	p6_8 = b"P6\n%d %d\n255\n" % (w, h)
	p6_16 = b"P6\n%d %d\n65535\n" % (w, h)
	p7_8 = b"P7\nWIDTH %d\nHEIGHT %d\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n" % (w, h)
	p7_16 = b"P7\nWIDTH %d\nHEIGHT %d\nDEPTH 4\nMAXVAL 65535\nTUPLTYPE RGB_ALPHA\nENDHDR\n" % (w, h)

	# Eight bits, no alpha.
	px = bytearray()
	for y in range(h):
		for x in range(w):
			px += bytes(pix(x, y)[:3])
	write("rgb8_%s.ppm" % spec, p6_8, px)

	# Sixteen bits, no alpha. The samples are not multiples of 257, so the low byte carries
	# something a reduction to eight bits has to decide what to do with.
	px = bytearray()
	for y in range(h):
		for x in range(w):
			for i, c in enumerate(pix(x, y)[:3]):
				v = (c * 257 + x * 7 + y * 13 + i * 3) % 65536
				px += bytes((v >> 8, v & 255))
	write("rgb16_%s.ppm" % spec, p6_16, px)

	# Eight bits with alpha.
	px = bytearray()
	for y in range(h):
		for x in range(w):
			px += bytes(pix(x, y))
	write("rgba8_%s.pam" % spec, p7_8, px)

	# Sixteen bits with alpha.
	px = bytearray()
	for y in range(h):
		for x in range(w):
			for i, c in enumerate(pix(x, y)):
				v = (c * 257 + x * 7 + y * 13 + i * 3) % 65536
				px += bytes((v >> 8, v & 255))
	write("rgba16_%s.pam" % spec, p7_16, px)

	# Two colours far enough apart that a quantisation to two keeps both. The general source
	# above is smooth enough that ImageMagick maps all of it onto one entry, which would make a
	# 1-bit palette fixture a picture of nothing.
	px = bytearray()
	for y in range(h):
		for x in range(w):
			px += bytes((250, 40, 10) if (x + y) % 2 == 0 else (5, 30, 240))
	write("duo_%s.ppm" % spec, p6_8, px)

	# A few saturated colours, one pixel in five fully transparent: the shape ImageMagick's PNG8
	# writer turns into a palette with a tRNS chunk. One source per palette size, because the
	# palette has to fit the bit depth asked for.
	for n in (1, 3, 6, 12):
		px = bytearray()
		for y in range(h):
			for x in range(w):
				c = PAL[(x + 2 * y) % n]
				a = 0 if (x + 3 * y) % 5 == 0 else 255
				px += bytes((c[0], c[1], c[2], a))
		write("pal%d_%s.pam" % (n, spec), p7_8, px)
PYEOF

# ---- The fixtures. ----
# $1 is the combination's name, $2 the size, $3 `n` or `i` for the interlace method, and the rest
# the recipe, with a leading `@` on the source's name standing for the temporary directory.
make() {
	local name=$1 size=$2 lace=$3
	shift 3
	local il=none
	[ "$lace" = i ] && il=PNG
	local args=()
	for a in "$@"; do
		args+=("${a/@/$tmp/}")
	done
	convert "${args[0]}" +dither -strip -interlace "$il" "${args[@]:1}"
	mv out.png "${name}_${size}_${lace}.png"
	convert "${name}_${size}_${lace}.png" -alpha set -depth 8 "pam:${name}_${size}_${lace}.pam"
}

# One combination at one size, in both interlace methods.
both() {
	local name=$1 size=$2
	shift 2
	make "$name" "$size" n "$@"
	make "$name" "$size" i "$@"
}

# Every legal depth and colour type, at a size where all seven Adam7 passes hold pixels.
s=$FULL
both g1     "$s" "@rgb8_$s.ppm"   -colorspace Gray -type Bilevel png:out.png
both g2     "$s" "@rgb8_$s.ppm"   -colorspace Gray -type Grayscale -depth 2 \
	-define png:color-type=0 -define png:bit-depth=2 png:out.png
both g4     "$s" "@rgb8_$s.ppm"   -colorspace Gray -type Grayscale -depth 4 \
	-define png:color-type=0 -define png:bit-depth=4 png:out.png
both g8     "$s" "@rgb8_$s.ppm"   -colorspace Gray -type Grayscale -depth 8 \
	-define png:color-type=0 -define png:bit-depth=8 png:out.png
both g16    "$s" "@rgb16_$s.ppm"  -alpha off -colorspace Gray -type Grayscale -depth 16 \
	-define png:color-type=0 -define png:bit-depth=16 png:out.png
both p1     "$s" "@duo_$s.ppm"    -define png:bit-depth=1 PNG8:out.png
both p2     "$s" "@rgb8_$s.ppm"   -type Palette -colors 3 -depth 2 \
	-define png:color-type=3 -define png:bit-depth=2 png:out.png
both p4     "$s" "@rgb8_$s.ppm"   -type Palette -colors 12 -depth 4 \
	-define png:color-type=3 -define png:bit-depth=4 png:out.png
both p8     "$s" "@rgb8_$s.ppm"   -type Palette -colors 32 -depth 8 \
	-define png:color-type=3 -define png:bit-depth=8 png:out.png
both pt1    "$s" "@pal1_$s.pam"   -define png:bit-depth=1 PNG8:out.png
both pt2    "$s" "@pal3_$s.pam"   -define png:bit-depth=2 PNG8:out.png
both pt4    "$s" "@pal6_$s.pam"   -define png:bit-depth=4 PNG8:out.png
both pt8    "$s" "@pal12_$s.pam"  -define png:bit-depth=8 PNG8:out.png
both rgb8   "$s" "@rgb8_$s.ppm"   PNG24:out.png
both rgb16  "$s" "@rgb16_$s.ppm"  -alpha off -depth 16 \
	-define png:color-type=2 -define png:bit-depth=16 png:out.png
both ga8    "$s" "@rgba8_$s.pam"  -colorspace Gray -type GrayscaleAlpha -depth 8 \
	-define png:color-type=4 -define png:bit-depth=8 png:out.png
both ga16   "$s" "@rgba16_$s.pam" -colorspace Gray -type GrayscaleAlpha -depth 16 \
	-define png:color-type=4 -define png:bit-depth=16 png:out.png
both rgba8  "$s" "@rgba8_$s.pam"  PNG32:out.png
both rgba16 "$s" "@rgba16_$s.pam" -depth 16 \
	-define png:color-type=6 -define png:bit-depth=16 png:out.png

# The sizes at which Adam7 passes fall empty, in the narrowest and the widest pixel the format has:
# one bit of greyscale, and four channels of eight and of sixteen bits.
for s in $SMALL; do
	both g1     "$s" "@rgb8_$s.ppm"    -colorspace Gray -type Bilevel png:out.png
	both rgba8  "$s" "@rgba8_$s.pam"   PNG32:out.png
	both rgba16 "$s" "@rgba16_$s.pam"  -depth 16 \
		-define png:color-type=6 -define png:bit-depth=16 png:out.png
done

ls -l
