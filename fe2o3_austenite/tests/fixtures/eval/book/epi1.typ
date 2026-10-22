// oracle: levels 2 4
#set page(width: 220pt, height: 260pt, margin: 20pt)
#let epigraph(body, by) = pad(left: 3em, right: 1em, block(width: 100%)[
  #set text(size: 0.9em, style: "italic")
  #body
  #align(right)[#by]
])
#epigraph[To be or not to be.][Shakespeare]
#epigraph(quote(block: true)[Words.], [Someone])
#v(1em)
#block(inset: (left: 1em), stroke: (left: 0.5pt + gray))[Indented block]
