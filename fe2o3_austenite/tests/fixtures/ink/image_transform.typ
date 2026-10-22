// A raster under a transform is drawn under it: rotated, skewed and scaled, and clipped by a box.
#set page(width: 300pt, height: 240pt, margin: 25pt)
#rotate(30deg)[#image("px.png", width: 2cm)]

#skew(ax: 20deg)[#image("px.png", width: 1.5cm)]

#box(width: 1cm, height: 1cm, clip: true)[#image("px.png", width: 2cm)] #scale(x: -100%)[#image("px.png", width: 1cm)]
