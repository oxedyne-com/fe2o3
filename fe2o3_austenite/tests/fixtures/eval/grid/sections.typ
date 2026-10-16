#set page(width: 200pt, height: 400pt, margin: 0pt, fill: none)
#let c(id, w, h) = block(width: w * 1pt, height: h * 1pt)[#metadata(id)<m>]
#grid(columns: (10pt, 10pt, 10pt,), rows: 10pt, row-gutter: 3pt, c(0, 5, 5), grid.header(c(1, 5, 5), grid.cell(x: 2, c(2, 5, 5))), c(3, 5, 5), grid.cell(rowspan: 2, c(4, 5, 5)), c(5, 5, 5), grid.header(level: 2, c(6, 5, 5)), c(7, 5, 5), grid.footer(c(8, 5, 5)))
