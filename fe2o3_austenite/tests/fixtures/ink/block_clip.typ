// A clipping block cuts to its inner stroke edge with rounded corners.
#set page(width: 300pt, height: 200pt, margin: 25pt)
#block(width: 3cm, height: 1.5cm, clip: true, radius: 10pt, stroke: 3pt + red)[#rect(width: 6cm, height: 4cm, fill: orange)]
#block(width: 3cm, height: 1.5cm, clip: true, inset: 4pt, stroke: (left: 4pt + blue), fill: luma(240))[#rect(width: 6cm, height: 4cm, fill: green)]
