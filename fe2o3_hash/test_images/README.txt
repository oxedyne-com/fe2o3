Perceptual hash test fixtures
=============================

Three synthetic 64 by 64 greyscale subjects, each with four variants produced by an external
tool.  The transforms are the oracle: a hash that did not survive a half-size reduction, a
quality forty re-encode, a ten per cent brightening and a lossless to lossy conversion would be
useless, and tests/phash.rs asserts that it does.

The portable greymap is used because it is trivial to read, which keeps the decode outside the
library under test, as the module's design requires.

Regenerate with ImageMagick 7:

    magick -size 64x64 -seed 42 plasma:fractal -colorspace Gray -depth 8 m_plasma.png
    magick -size 64x64 gradient:white-black -colorspace Gray -rotate 30 \
        -crop 64x64+0+0 +repage -depth 8 m_gradient.png
    magick -size 64x64 xc:white -fill black \
        -draw "circle 32,32 32,8" -draw "rectangle 4,4 20,20" \
        -draw "polygon 40,50 60,60 44,62" -colorspace Gray -depth 8 m_shapes.png

    for b in plasma gradient shapes; do
      magick m_$b.png -depth 8                  ${b}_orig.pgm
      magick m_$b.png -resize 50% -depth 8      ${b}_half.pgm
      magick m_$b.png -quality 40 v.jpg && magick v.jpg -depth 8   ${b}_q40.pgm
      magick m_$b.png -modulate 110 -depth 8    ${b}_bright.pgm
      magick m_$b.png -quality 85 v.jpg && magick v.jpg -depth 8   ${b}_png2jpg.pgm
    done

Measured over these fixtures: same-subject distances have a mean of 1.08 and a maximum of 5 for
the difference hash, and a mean of 0.50 and a maximum of 4 for the cosine transform hash, while
unrelated pairs have a minimum of 25 and 28 respectively.

No photograph appears here.
