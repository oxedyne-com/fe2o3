EXIF test fixtures
==================

Two 24 by 16 pixel JPEGs, one in each TIFF byte order, carrying a known set of tags.  They are
the external oracle for tests/exif.rs: the metadata was written by exiftool, and the values the
tests assert are the values exiftool reports back.

Regenerate with:

    magick -size 24x16 gradient:red-blue -quality 60 base.jpg
    for ord in MM II; do
      cp base.jpg "exif_$ord.jpg"
      exiftool -overwrite_original -q \
        -ExifByteOrder=$ord \
        -Make="Oxide Optics" \
        -Model="Model 7 Field" \
        -Orientation#=6 \
        -DateTimeOriginal="2019:04:07 13:45:02" \
        -CreateDate="2019:04:07 13:45:03" \
        -SubSecTimeOriginal=880 \
        -ExposureTime=1/250 \
        -FNumber=2.8 \
        -ISO=400 \
        -FocalLength=35.0 \
        -ExifImageWidth=24 \
        -ExifImageHeight=16 \
        -LensModel="35mm f/2 Prime" \
        -GPSLatitude=-31.9534277 -GPSLatitudeRef=S \
        -GPSLongitude=115.8657722 -GPSLongitudeRef=E \
        -GPSAltitude=45.67 -GPSAltitudeRef=Below \
        "exif_$ord.jpg"
    done
    rm base.jpg

Written with exiftool 13.50 and ImageMagick 7.  The composite values that tool reports for the
result, and which tests/exif.rs asserts, are:

    GPS Latitude   : -31.9534276999917
    GPS Longitude  : 115.865772200058
    GPS Altitude   : -45.67
    Exposure Time  : 0.004
    Image Width    : 24      (from the start of frame marker, not from EXIF)
    Image Height   : 16

The subject matter is a synthetic gradient.  No photograph appears here.
