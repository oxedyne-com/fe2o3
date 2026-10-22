// A box with `clip: true` cuts its body to its own frame, inside its stroke; the stroke is drawn.
#set page(width: 300pt, height: 160pt, margin: 25pt)
Before #box(width: 2cm, height: 1cm, clip: true, stroke: 1pt)[#rect(width: 4cm, height: 2cm, fill: orange)] after.

#box(width: 2cm, height: 1cm, clip: true, radius: 8pt, stroke: 2pt + blue)[#rect(width: 4cm, height: 2cm, fill: orange)]
#box(width: 1.5cm, height: 1cm, clip: true, outset: 4pt, fill: luma(230))[#rect(width: 3cm, height: 2cm, fill: green)]
