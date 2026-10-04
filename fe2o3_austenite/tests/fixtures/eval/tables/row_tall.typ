// oracle: levels 4
// A row that does not fit the room left on the page goes to the next page whole, where a first row once
// stood below the bottom margin. A one-line row cannot break, so it moves.
#set page(width: 200pt, height: 150pt, margin: 20pt)
#lorem(30)
#table(columns: 2, [A], [B])
