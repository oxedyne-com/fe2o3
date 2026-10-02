// oracle: levels 4
// A footnote in an inline box, a pad, a stack and a column break.
#set page(width: 200pt, height: 170pt, margin: 20pt)
#lorem(14)

#pad(left: 8pt)[A pad with a note#footnote[In the pad.].]

#stack(dir: ttb, spacing: 4pt,
  [First child of a stack#footnote[In the stack.].],
  [Second child.],
)

A line with a #box(inset: 2pt)[boxed note#footnote[In the box.]] in it.

#lorem(14)
