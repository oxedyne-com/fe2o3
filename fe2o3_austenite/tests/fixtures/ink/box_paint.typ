// An inline box takes its inset, fill, stroke, radius and outset, and sits on its baseline or a shifted one.
#set page(width: 300pt, height: 160pt, margin: 25pt)
Text #box(fill: yellow, inset: 4pt, radius: 3pt, stroke: 0.5pt)[boxed] and #box(baseline: 30%, fill: luma(220), inset: 2pt)[raised] and #box(height: 18pt, width: 30pt, fill: aqua)[sized] end.

Outset #box(outset: 3pt, stroke: 1pt + red)[out] and #box(baseline: bottom, height: 20pt, stroke: 0.5pt)[low].
