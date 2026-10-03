// oracle: levels 4
// One-line rows over four pages: a row that does not fit the room at the foot of the second page or a later
// one goes to the next, the room left being what is placed on a page less its full height.
#set page(width: 200pt, height: 90pt, margin: 20pt)
#table(columns: 2, ..range(30).map(i => [Row #i]), ..range(30).map(i => [text #i]))
