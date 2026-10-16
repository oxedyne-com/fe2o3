// oracle: levels 4
// Blocks nested in blocks, each holding a note, across a page break.
#set page(width: 200pt, height: 150pt, margin: 20pt)
#lorem(24)

#block(inset: 3pt)[
  Outer text#footnote[Outer note.]
  #block(inset: 3pt)[Inner text#footnote[Inner note.] and more.]
  #block(inset: 3pt)[A second inner block#footnote[Second inner note.].]
]

#lorem(10)
