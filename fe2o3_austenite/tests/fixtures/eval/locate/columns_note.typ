// oracle: levels 4
// A footnote inside `columns` laid out again after a region change.
#set page(width: 220pt, height: 160pt, margin: 20pt)
#lorem(16)

#columns(2)[
  #lorem(20)
  A note in a column#footnote[In the column.] and more text.
]

#lorem(12)
