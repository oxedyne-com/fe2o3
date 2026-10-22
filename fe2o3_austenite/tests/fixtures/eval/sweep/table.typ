// oracle: levels 4
// The sweep: a table with a header, a footer and a colspan.
#set page(width: 220pt, height: 200pt, margin: 20pt)
#table(columns: 3, table.header([A], [B], [C]), [1], [2], [3], table.cell(colspan: 2)[wide], [4], table.footer([x], [y], [z]))
