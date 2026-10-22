#set page(width: 200pt, height: 400pt, margin: 0pt, fill: none)
#let c(id, w, h) = block(width: w * 1pt, height: h * 1pt)[#metadata(id)<m>]
#grid(columns: (10pt, 10pt, 10pt,), rows: 10pt, grid.cell(x: 0, y: 2, c(0, 5, 5)), grid.cell(x: 1, c(1, 5, 5)), grid.cell(y: 1, c(2, 5, 5)), c(3, 5, 5), c(4, 5, 5), grid.cell(x: 0, c(5, 5, 5)), grid.cell(colspan: 2, rowspan: 2, c(6, 5, 5)), c(7, 5, 5), c(8, 5, 5), c(9, 5, 5))
