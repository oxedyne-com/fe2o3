// A table whose rows break across pages, with fills and the default strokes: each part of a row is filled
// to its region, its vertical lines run the part's height, the top line is drawn on the first part and the
// bottom line on the last.
#set page(width: 220pt, height: 140pt, margin: 20pt)
#table(
	columns: (1fr, 2fr),
	fill: (x, y) => if y == 1 { yellow } else { none },
	[One], [#lorem(10)],
	[Two], [#lorem(45)],
	[Three], [Last row.],
)
