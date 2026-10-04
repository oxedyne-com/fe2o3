// oracle: levels 4
// A grid alone on its page is as tall as its rows, not as tall as the page, so `align` places it: in the middle, at the foot, or
// both ways, and the cells move with it. A row of fractional height still fills the page. The same holds for a grid alone in a
// block of the page's height and in a pad. A grid that breaks is as tall as the rows on each page, the gutter that ends a page
// not among them.
#set page(width: 200pt, height: 150pt, margin: 10pt)
#align(horizon, grid(columns: 2, row-gutter: 12pt, [alpha], [beta], [gamma], [delta]))
#pagebreak()
#align(bottom, grid(columns: 2, row-gutter: 12pt, fill: luma(230), [alpha], [beta], [gamma], [delta]))
#pagebreak()
#align(center + horizon, grid(columns: (40pt, 40pt), row-gutter: 12pt, [alpha], [beta], [gamma], [delta]))
#pagebreak()
#align(horizon, grid(columns: 2, rows: (20pt, 1fr), row-gutter: 12pt, [alpha], [beta], [gamma], [delta]))
#pagebreak()
#align(horizon, block(height: 100%, grid(columns: 2, row-gutter: 12pt, [alpha], [beta], [gamma], [delta])))
#pagebreak()
#align(horizon, pad(4pt, grid(columns: 2, row-gutter: 12pt, [alpha], [beta], [gamma], [delta])))
#pagebreak()
#align(horizon, grid(columns: 1, row-gutter: 30pt, [a1], [a2], [a3], [a4], [a5]))
