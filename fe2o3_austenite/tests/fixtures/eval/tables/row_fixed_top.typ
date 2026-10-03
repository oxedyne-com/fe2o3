// oracle: levels 4
// Rows of a fixed height from the top of the first page: the first region has its full height, yet a row that
// does not fit the room left at its foot goes to the next page.
#set page(width: 200pt, height: 90pt, margin: 20pt)
#table(columns: 2, rows: 20pt, ..range(8).map(i => [Row #i]), ..range(8).map(i => [text #i]))
