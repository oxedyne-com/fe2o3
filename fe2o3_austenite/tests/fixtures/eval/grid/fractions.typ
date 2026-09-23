#set page(width: 200pt, height: 400pt, margin: 0pt, fill: none)
#let c(id, w, h) = block(width: w * 1pt, height: h * 1pt)[#metadata(id)<m>]
#grid(columns: (1fr, 25%, 2fr, auto,), gutter: 4pt, c(0, 5, 5), c(1, 5, 5), c(2, 5, 5), c(3, 12, 7), c(4, 5, 9))
