// oracle: levels 4
// A repeated header above rows that break: each page the rows spill onto starts with the header again.
#set page(width: 220pt, height: 140pt, margin: 20pt)
#table(
	columns: 2,
	table.header([Head A], [Head B]),
	[x], [#lorem(40)],
	[y], [#lorem(10)],
)
