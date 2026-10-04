// oracle: levels 4
// An inline box takes its inset and its height into the line, and its baseline follows its content's, a
// `baseline` shift or an alignment.
#set page(width: 300pt, height: 300pt, margin: 25pt)
Before #box(stroke: 0.5pt, inset: 3pt)[Mu: Nu] after text on the same line.

Raised #box(baseline: 30%, inset: 2pt, fill: luma(220))[up] and lowered #box(baseline: -20%)[down] text.

Tall #box(height: 24pt, stroke: 0.5pt)[top] and #box(height: 24pt, baseline: bottom, stroke: 0.5pt)[bottom] end.

A block in a box: #box(width: 80pt, inset: 4pt, stroke: 0.5pt)[
  first paragraph

  second paragraph here
] after.
