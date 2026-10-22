#set page(width: 200pt, height: 400pt, margin: 0pt, fill: none)
#let c(id, w, h) = block(width: w * 1pt, height: h * 1pt)[#metadata(id)<m>]
#table(columns: (20pt, 20pt, 20pt,), inset: 2pt, stroke: (x: 0.5pt + black, bottom: 1.5pt + olive), c(0, 5, 5), table.hline(stroke: red, ), c(1, 5, 5), table.vline(stroke: 1pt + blue, ), c(2, 5, 5), c(3, 5, 5), c(9, 5, 5), c(10, 5, 5), table.hline(start: 1, end: 2, stroke: 2pt + green, ), table.vline(x: 1, stroke: none, ), table.cell(colspan: 3, c(4, 5, 5)), c(5, 5, 5))
