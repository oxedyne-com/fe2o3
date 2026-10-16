// A table broken across pages: a repeated header, rows of different heights, and text before and after it.
#set page(width: 220pt, height: 130pt, margin: 16pt, numbering: "1")
#set text(size: 9pt)

The table below lists the stores held at the station, a row to a store, with the quantity held and a short note on each.

The station is manned by two keepers throughout the year, who share the work of the count between them.

#table(
	columns: (auto, 1fr, auto),
	table.header([No.], [Item], [Qty]),
	[1], [Rope, hemp, in coils of thirty metres], [4],
	[2], [Lamps, brass, with spare wicks and glass], [12],
	[3], [Tins of paraffin, five litres each, stored at the back of the shed away from the stove], [30],
	[4], [Blankets], [18],
	[5], [Boots, various sizes, mostly large], [9],
	[6], [Flags, signal, in a set of the usual twenty-six], [2],
	[7], [Charts of the coast and the approaches, folded, with the harbour plans and the tide tables bound in], [7],
	[8], [Kettles], [3],
	[9], [Tools: hammers, saws, planes, and a box of nails of every kind], [1],
	[10], [Lamp glass, spare], [20],
	[11], [Brooms], [6],
	[12], [Buckets, galvanised, some with holes], [11],
)

The count is made each spring, and the totals are sent to the district office with the log.

Text after the table closes the page and runs on for a line or two, so that the position of the end of the table matters.

A last paragraph ends the document with a line or two of text.
