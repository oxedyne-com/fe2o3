// oracle: levels 4
// A figure, its label and its number inside a block that a footnote makes the flow lay out again.
#set page(width: 200pt, height: 170pt, margin: 20pt)
See @fig first.
#lorem(18)

#block(inset: 4pt)[
  #figure(rect(width: 30pt, height: 10pt), caption: [Inside]) <fig>
  A note in the block#footnote[The block's note.] and more text of the block.
]

#lorem(14)
