= A Raster on the Page

This page exists to exercise the Pearl reader's raster leaf: a PNG loaded from
the asset tree, placed as an `#image(...)` and carried through the `.prl` as a
base64 string that the browser draws directly.

#figure(
	image("raster_src.png", width: 96pt),
	caption: [A small raster mark, drawn from stored PNG bytes.],
)

The text around it is ordinary body copy, so the page carries glyph outlines and
a raster together and the reader must place both correctly.
