#set page(width: 200pt, height: 400pt, margin: 0pt, fill: none)
#let c(id, w, h) = block(width: w * 1pt, height: h * 1pt)[#metadata(id)<m>]
#table(columns: (auto, 30pt, auto,), fill: (yellow, none, aqua,), c(0, 10, 5), c(1, 5, 5), c(2, 7, 5), table.cell(stroke: 2pt + blue, fill: red, c(3, 5, 5)), table.cell(colspan: 2, inset: (left: 1pt, y: 2pt), c(4, 5, 15)), table.cell(rowspan: 2, c(5, 5, 5)), c(6, 5, 5), c(7, 5, 5))
