// oracle: levels 4
// A footnote in the body of a shape and of a moved block, which a note's relayout lays out again.
#set page(width: 200pt, height: 170pt, margin: 20pt)
#lorem(14)

#rect(inset: 4pt, width: 100%)[A shape with a note#footnote[In the shape.].]

#move(dx: 6pt)[A moved body#footnote[In the moved body.].]

#lorem(10)
