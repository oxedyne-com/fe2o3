// oracle: levels 4
// Rows of a fixed height under a paragraph, over four pages: a row that does not fit the room at the foot of
// the second page or a later one goes to the next whole; it does not break, so only the room left tells that
// the page may be left.
#set page(width: 200pt, height: 90pt, margin: 20pt)
Intro text.

#table(columns: 2, rows: 16pt, ..range(14).map(i => [Row #i]), ..range(14).map(i => [text #i]))
