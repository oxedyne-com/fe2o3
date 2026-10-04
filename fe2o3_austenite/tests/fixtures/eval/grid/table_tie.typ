#set page(width: 200pt, height: 400pt, margin: 0pt, fill: none)
#let c(id, w, h) = block(width: w * 1pt, height: h * 1pt)[#metadata(id)<m>]
#table(columns: (15pt, 15pt,), inset: 1pt, stroke: (top: 2pt + red, bottom: 1.5pt + olive, left: blue, right: 3pt + green), c(0, 5, 5), c(1, 5, 5), c(2, 5, 5), c(3, 5, 5))
