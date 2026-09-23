#set page(width: 200pt, height: 400pt, margin: 0pt, fill: none)
#let c(id, w, h) = block(width: w * 1pt, height: h * 1pt)[#metadata(id)<m>]
#table(columns: (10pt, 10pt,), rows: 10pt, inset: 0pt, gutter: 4pt, fill: blue, c(0, 5, 5), c(1, 5, 5), table.cell(colspan: 2, c(2, 5, 5)), table.hline(stroke: red, ), table.vline(x: 1, stroke: green, ), table.vline(x: 1, position: end, stroke: olive, ))
