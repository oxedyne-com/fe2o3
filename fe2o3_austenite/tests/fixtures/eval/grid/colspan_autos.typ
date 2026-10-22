#set page(width: 200pt, height: 400pt, margin: 0pt, fill: none)
#let c(id, w, h) = block(width: w * 1pt, height: h * 1pt)[#metadata(id)<m>]
#grid(columns: (auto, auto, auto,), c(0, 10, 5), c(1, 20, 5), c(2, 5, 5), grid.cell(colspan: 2, c(3, 60, 5)), grid.vline(stroke: red, ), c(4, 5, 5))
