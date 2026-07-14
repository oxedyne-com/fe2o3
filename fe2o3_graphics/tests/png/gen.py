#!/usr/bin/env python3
"""Generate tRNS-bearing PNGs with PIL, and record what PIL says each pixel means.

PIL writes the files and PIL says what they decode to.  Our decoder is not consulted,
so agreeing with this file is agreement with an implementation that is not ours.
"""
import json
import os

from PIL import Image, ImagePalette

OUT = os.path.dirname(os.path.abspath(__file__))
cases = {}


def record(name, img):
    """Write the PNG, then record PIL's own RGBA reading of it."""
    path = os.path.join(OUT, name + ".png")
    img.save(path)
    # Re-open from disk, so we record what the *file* says, not what we held in memory.
    reopened = Image.open(path).convert("RGBA")
    w, h = reopened.size
    cases[name] = {
        "w": w,
        "h": h,
        "rgba": [list(px) for px in reopened.getdata()],
    }


# 1. Palette image with a tRNS chunk: per-entry alpha, some fully transparent.
pal_img = Image.new("P", (4, 4))
palette = []
for i in range(256):
    palette += [(i * 7) % 256, (i * 13) % 256, (i * 29) % 256]
pal_img.putpalette(palette)
pal_img.putdata([0, 1, 2, 3] * 4)
# Entry 0 fully transparent, entry 1 half, entries 2+ opaque.
pal_img.info["transparency"] = bytes([0, 128, 255, 255])
record("palette_trns", pal_img)

# 2. Greyscale image with a tRNS chunk: one luminance value is the transparent one.
grey = Image.new("L", (4, 4))
grey.putdata([0, 64, 128, 255] * 4)
grey.info["transparency"] = 128  # Every pixel of luminance 128 is transparent.
record("grey_trns", grey)

# 3. Truecolour image with a tRNS chunk: one RGB triple is the transparent one.
rgb = Image.new("RGB", (4, 4))
rgb.putdata([(255, 0, 0), (0, 255, 0), (0, 0, 255), (10, 20, 30)] * 4)
rgb.info["transparency"] = (0, 255, 0)  # Every pure-green pixel is transparent.
record("rgb_trns", rgb)

# 4. Controls: images with no tRNS at all must stay fully opaque.
plain_rgb = Image.new("RGB", (4, 4))
plain_rgb.putdata([(1, 2, 3), (4, 5, 6), (7, 8, 9), (10, 11, 12)] * 4)
record("rgb_plain", plain_rgb)

rgba = Image.new("RGBA", (4, 4))
rgba.putdata([(1, 2, 3, 0), (4, 5, 6, 85), (7, 8, 9, 170), (10, 11, 12, 255)] * 4)
record("rgba_plain", rgba)

with open(os.path.join(OUT, "expected.json"), "w") as f:
    json.dump(cases, f, indent=1)

print("wrote {} cases: {}".format(len(cases), ", ".join(sorted(cases))))
for name, c in sorted(cases.items()):
    alphas = [px[3] for px in c["rgba"]]
    print("  {:14s} transparent px = {:2d}/{:2d}".format(
        name, sum(1 for a in alphas if a == 0), len(alphas)))
