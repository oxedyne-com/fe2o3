// oracle: levels 4
// A short cell beside a long one: the row breaks on the long cell, the short one stays in the first part,
// and the rows after the broken one carry on in the room it leaves.
#set page(width: 220pt, height: 140pt, margin: 20pt)
#table(
	columns: (1fr, 2fr),
	[One], [#lorem(10)],
	[Two], [#lorem(45)],
	[Three], [Last row.],
)
