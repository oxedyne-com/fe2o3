// oracle: levels 4
// A table inside a stroked block and inside an inline box.
#set page(width: 220pt, height: 200pt, margin: 20pt)
#block(inset: 6pt, stroke: 1pt)[
  #table(columns: 2, [a], [b], [c], [d])
]
#box(inset: 4pt, stroke: 1pt)[#table(columns: 2, [e], [f])]
