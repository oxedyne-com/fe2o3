#set page(width: 200pt, height: 400pt, margin: 0pt, fill: none)
#let c(id, w, h) = block(width: w * 1pt, height: h * 1pt)[#metadata(id)<m>]
#grid(columns: (auto, 30pt, 1fr,), column-gutter: 3pt, row-gutter: 2pt, c(0, 20, 10), c(1, 10, 15), c(2, 5, 5), grid.cell(colspan: 2, c(3, 70, 4)), c(4, 1, 1), c(5, 1, 1))
