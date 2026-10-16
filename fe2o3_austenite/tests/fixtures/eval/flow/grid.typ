// oracle: levels 2 4
// A grid of fixed-size cells, positioned as blocks in the flow, and a grid of unbreakable cells breaking
// across pages between its rows.
#set page(width: 200pt, height: 150pt, margin: 10pt)
#let cell = [#block(width: 30pt, height: 20pt, breakable: false) <probe>]
#block(width: 20pt, height: 20pt) <probe>
#grid(columns: 2, gutter: 5pt,
  [#block(width: 20pt, height: 20pt) <probe>], [#block(width: 30pt, height: 10pt) <probe>],
  [#block(width: 20pt, height: 30pt) <probe>], [#block(width: 30pt, height: 10pt) <probe>],
)
#grid(columns: (40pt, 40pt), row-gutter: 4pt,
  cell, cell, cell, cell, cell, cell, cell, cell, cell, cell, cell, cell,
)
#block(width: 20pt, height: 20pt) <probe>
