// oracle: rejects
#set page(width: 200pt, height: 400pt, margin: 0pt, fill: none)
#let c(id, w, h) = block(width: w * 1pt, height: h * 1pt)[#metadata(id)<m>]
#grid(columns: (10pt, 10pt, 10pt,), c(0, 5, 5), grid.cell(colspan: 3, c(1, 5, 5)))
