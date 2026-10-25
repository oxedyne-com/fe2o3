#!/bin/bash
# Regenerates the BMP, GIF and TIFF fixtures in this directory, and the PNG beside each that the
# test compares against.
#
# Nothing here was written by the codec under test. A short Python program draws the sources: a
# one-bit page of text-like strokes, whose runs are long and varied the way a scanned letter's are,
# and small colour, grey and alpha gradients. Pillow (with libtiff) and ImageMagick write each
# fixture, and ImageMagick reads the fixture back and writes what it saw as `<name>.png`, eight-bit
# RGBA. So a fixture's expected pixels come from an independent reader, never from this crate.
#
# The two BMP shapes neither writer offers -- RLE4 and top-down rows -- are built here by a few lines
# of Python from the same source pixels, and still checked against ImageMagick's reading of them.
# Each fixture's header is re-read after writing, and the script stops if a writer quietly chose a
# different depth or coding from the one the name claims.
#
# Requires ImageMagick 7 (`magick`) and Python 3 with Pillow built against libtiff. Run from here.
set -eu
cd "$(dirname "$0")"
rm -f ./*.bmp ./*.gif ./*.tif ./*.png
python3 - <<'PYEOF'
import struct
import subprocess
from PIL import Image

def text(w, h):
	"""A page of text-like strokes: lines of words, each word a run of letter-like stems."""
	im = Image.new('1', (w, h), 1)
	px = im.load()
	seed = 12345
	def rnd(n):
		nonlocal seed
		seed = (seed * 1103515245 + 12345) & 0x7fffffff
		return seed % n
	y = 3
	while y + 9 < h:
		x = 2 + rnd(6)
		while x < w - 4:
			word = 3 + rnd(40)
			for xx in range(x, min(x + word, w - 1)):
				stem = (xx - x) % (2 + rnd(4)) == 0
				for yy in range(y, y + 9):
					if stem or yy in (y, y + 4 + rnd(2), y + 8) and rnd(3):
						px[xx, yy] = 0
			x += word + 2 + rnd(9)
		y += 12 + rnd(4)
	# A solid bar and a blank band, so the longest runs and their makeup codes are exercised.
	for xx in range(w // 5, w - w // 7):
		for yy in range(h - 3, h):
			px[xx, yy] = 0
	return im

def colour(w, h, alpha):
	im = Image.new('RGBA' if alpha else 'RGB', (w, h))
	px = im.load()
	for y in range(h):
		for x in range(w):
			r = (x * 47 + y * 11 + 3) % 256
			g = (x * 13 + y * 61 + 71) % 256
			b = (x * 29 + y * 89 + 151) % 256
			a = (x * 37 + y * 101 + 19) % 256
			px[x, y] = (r, g, b, a) if alpha else (r, g, b)
	return im

def grey(w, h):
	im = Image.new('L', (w, h))
	px = im.load()
	for y in range(h):
		for x in range(w):
			px[x, y] = (x * 7 + y * 19) % 256
	return im

def run(*a):
	subprocess.run(list(a), check=True)

T = text(203, 97)		# odd width, so every row is padded
W = text(1800, 40)		# wider than a fax line, for the long makeup codes
C = colour(37, 23, False)
A = colour(37, 23, True)
G = grey(37, 23)
P = C.quantize(16)		# sixteen colours, for the 4-bit and palette cases
P256 = C.quantize(200)

T.save('src_text.pbm')
C.save('src_rgb.ppm')
A.save('src_rgba.png')
G.save('src_grey.pgm')
P.save('src_p16.png')
P256.save('src_p256.png')

# ---- BMP ----
def bmp_bpp(f):
	b = open(f, 'rb').read()
	hs = struct.unpack('<I', b[14:18])[0]
	if hs == 12:
		return hs, struct.unpack('<H', b[24:26])[0], 0
	return hs, struct.unpack('<H', b[28:30])[0], struct.unpack('<I', b[30:34])[0]

def bmp(name, want_bpp, want_comp, *cmd):
	run('magick', *cmd, name)
	hs, bpp, comp = bmp_bpp(name)
	assert (bpp, comp) == (want_bpp, want_comp), (name, hs, bpp, comp)

run('magick', 'src_text.pbm', '-type', 'bilevel', 'BMP3:bmp1_text.bmp')
assert bmp_bpp('bmp1_text.bmp')[1:] == (1, 0), bmp_bpp('bmp1_text.bmp')
T.save('bmp1_text_pil.bmp')
assert bmp_bpp('bmp1_text_pil.bmp')[1:] == (1, 0)
run('magick', 'src_p16.png', '-type', 'palette', 'BMP3:bmp4.bmp')
assert bmp_bpp('bmp4.bmp')[1:] == (4, 0), bmp_bpp('bmp4.bmp')
P256.save('bmp8.bmp')
assert bmp_bpp('bmp8.bmp')[1:] == (8, 0), bmp_bpp('bmp8.bmp')
run('magick', 'src_p256.png', '-type', 'palette', '-compress', 'RLE', 'BMP3:bmp8_rle.bmp')
assert bmp_bpp('bmp8_rle.bmp')[1:] == (8, 1), bmp_bpp('bmp8_rle.bmp')
run('magick', 'src_rgb.ppm', '-define', 'bmp:subtype=RGB555', 'BMP:bmp16_555.bmp')
assert bmp_bpp('bmp16_555.bmp')[1] == 16, bmp_bpp('bmp16_555.bmp')
run('magick', 'src_rgb.ppm', '-define', 'bmp:subtype=RGB565', 'BMP:bmp16_565.bmp')
assert bmp_bpp('bmp16_565.bmp')[1:] == (16, 3), bmp_bpp('bmp16_565.bmp')
run('magick', 'src_rgb.ppm', 'BMP3:bmp24.bmp')
assert bmp_bpp('bmp24.bmp')[1:] == (24, 0), bmp_bpp('bmp24.bmp')
run('magick', 'src_rgb.ppm', 'BMP2:bmp24_os2.bmp')
assert bmp_bpp('bmp24_os2.bmp')[0] == 12, bmp_bpp('bmp24_os2.bmp')
run('magick', 'src_rgba.png', 'BMP:bmp32_v5.bmp')
hs, bpp, comp = bmp_bpp('bmp32_v5.bmp')
assert (hs, bpp, comp) == (124, 32, 3), (hs, bpp, comp)

# Top-down: bmp24 with its height negated and its rows reversed.
b = bytearray(open('bmp24.bmp', 'rb').read())
off = struct.unpack('<I', b[10:14])[0]
w, h = struct.unpack('<ii', b[18:26])
stride = (w * 24 + 31) // 32 * 4
rows = [b[off + r * stride: off + (r + 1) * stride] for r in range(h)]
b[22:26] = struct.pack('<i', -h)
b[off:] = b''.join(reversed(rows))
open('bmp24_topdown.bmp', 'wb').write(bytes(b))

# RLE4: encoded runs only, a pair of indices alternating within each run, rows bottom-up.
pal = P.getpalette()[:48]
pp = P.load()
w, h = P.size
data = bytearray()
for y in range(h - 1, -1, -1):
	x = 0
	while x < w:
		a = pp[x, y]
		bb = pp[x + 1, y] if x + 1 < w else a
		n = 2
		while x + n < w and n < 254 and pp[x + n, y] == (a if n % 2 == 0 else bb):
			n += 1
		n = min(n, w - x)
		data += bytes([n, (a << 4) | bb])
		x += n
	data += b'\x00\x00'
data += b'\x00\x01'
palb = b''.join(bytes([pal[i * 3 + 2], pal[i * 3 + 1], pal[i * 3], 0]) for i in range(16))
off = 14 + 40 + len(palb)
hdr = struct.pack('<IiiHHIIiiII', 40, w, h, 1, 4, 2, len(data), 2835, 2835, 16, 0)
open('bmp4_rle.bmp', 'wb').write(b'BM' + struct.pack('<IHHI', off + len(data), 0, 0, off) + hdr + palb + bytes(data))

# ---- GIF ----
run('magick', 'src_p256.png', 'gif_plain.gif')
run('magick', 'src_p256.png', '-interlace', 'GIF', 'gif_interlaced.gif')
run('magick', 'src_p16.png', '-transparent', '#%02x%02x%02x' % tuple(P.getpalette()[:3]), 'gif_transparent.gif')
run('magick', 'src_text.pbm', 'gif_text.gif')

# ---- TIFF ----
def tags(f, page=0):
	im = Image.open(f)
	im.seek(page)
	return im.tag_v2

def check(f, comp, **kw):
	t = tags(f)
	assert t.get(259) == comp, (f, t.get(259))
	for k, v in kw.items():
		got = t.get(int(k[1:]))
		assert got == v, (f, k, got, v)

T.save('tif_g1_none.tif', compression='raw')
check('tif_g1_none.tif', 1)
T.save('tif_g1_packbits.tif', compression='packbits')
check('tif_g1_packbits.tif', 32773)
T.save('tif_g1_lzw.tif', compression='tiff_lzw')
check('tif_g1_lzw.tif', 5)
T.save('tif_fax_mh.tif', compression='tiff_ccitt')
check('tif_fax_mh.tif', 2)
T.save('tif_fax_g3_1d.tif', compression='group3')
check('tif_fax_g3_1d.tif', 3)
T.save('tif_fax_g3_2d.tif', compression='group3', tiffinfo={292: 1})
check('tif_fax_g3_2d.tif', 3, t292=1)
T.save('tif_fax_g3_2d_fill.tif', compression='group3', tiffinfo={292: 5})
check('tif_fax_g3_2d_fill.tif', 3, t292=5)
T.save('tif_fax_g4.tif', compression='group4')
check('tif_fax_g4.tif', 4)
W.save('tif_fax_g4_wide.tif', compression='group4')
W.save('tif_fax_g3_wide.tif', compression='group3', tiffinfo={292: 1})
run('magick', 'src_text.pbm', '-compress', 'Group4', '-define', 'tiff:fill-order=lsb', 'tif_fax_g4_lsb.tif')
check('tif_fax_g4_lsb.tif', 4, t266=2)
T.save('tif_fax_g4_strips.tif', compression='group4', tiffinfo={278: 10})
check('tif_fax_g4_strips.tif', 4, t278=10)

for d in (2, 4):
	run('magick', 'src_grey.pgm', '-depth', str(d), '-compress', 'None', 'tif_grey%d.tif' % d)
	check('tif_grey%d.tif' % d, 1, t258=(d,))
G.save('tif_grey8_lzw.tif', compression='tiff_lzw')
check('tif_grey8_lzw.tif', 5)
run('magick', 'src_grey.pgm', '-depth', '16', '-compress', 'Zip', 'tif_grey16_deflate.tif')
check('tif_grey16_deflate.tif', 8, t258=(16,))
run('magick', 'src_grey.pgm', '-depth', '16', '-compress', 'None', '-define', 'tiff:endian=msb', 'tif_grey16_be.tif')
assert open('tif_grey16_be.tif', 'rb').read(2) == b'MM'
run('magick', 'src_grey.pgm', '-negate', '-define', 'quantum:polarity=min-is-white', '-compress', 'None', 'tif_grey8_miniswhite.tif')
check('tif_grey8_miniswhite.tif', 1, t262=0)

P256.save('tif_palette_lzw.tif', compression='tiff_lzw')
check('tif_palette_lzw.tif', 5, t262=3)
run('magick', 'src_p16.png', '-type', 'palette', '-depth', '4', '-compress', 'None', 'tif_palette4.tif')
check('tif_palette4.tif', 1, t262=3)

C.save('tif_rgb_none.tif', compression='raw')
check('tif_rgb_none.tif', 1)
C.save('tif_rgb_packbits.tif', compression='packbits')
check('tif_rgb_packbits.tif', 32773)
run('magick', 'src_rgb.ppm', '-compress', 'LZW', '-define', 'tiff:predictor=2', 'tif_rgb_lzw_pred.tif')
check('tif_rgb_lzw_pred.tif', 5, t317=2)
C.save('tif_rgb_deflate.tif', compression='tiff_adobe_deflate')
check('tif_rgb_deflate.tif', 8)
run('magick', 'src_rgb.ppm', '-compress', 'JPEG', '-quality', '95', 'tif_rgb_jpeg.tif')
check('tif_rgb_jpeg.tif', 7)
run('magick', 'src_rgb.ppm', '-compress', 'LZW', '-define', 'tiff:tile-geometry=16x16', 'tif_rgb_tiled.tif')
check('tif_rgb_tiled.tif', 5, t322=16)
run('magick', 'src_rgb.ppm', '-compress', 'None', '-interlace', 'Plane', 'tif_rgb_planar.tif')
check('tif_rgb_planar.tif', 1, t284=2)
A.save('tif_rgba_lzw.tif', compression='tiff_lzw')
check('tif_rgba_lzw.tif', 5)
run('magick', 'src_rgb.ppm', '-colorspace', 'CMYK', '-compress', 'None', 'tif_cmyk.tif')
check('tif_cmyk.tif', 1, t262=5)
run('magick', 'src_text.pbm', 'src_rgb.ppm', 'src_grey.pgm', '-compress', 'LZW', 'tif_multipage.tif')
PYEOF

# The expected pixels: ImageMagick's own reading of each fixture, as eight-bit RGBA.
for f in ./*.bmp ./*.gif ./*.tif; do
	n=$(magick identify "$f" | wc -l)
	b=${f%.*}
	if [ "$n" -gt 1 ]; then
		for ((i = 0; i < n; i++)); do
			magick "$f[$i]" -depth 8 "PNG32:${b}.p$i.png"
		done
	else
		magick "$f" -depth 8 "PNG32:${b}.png"
	fi
done
rm -f src_*
ls
