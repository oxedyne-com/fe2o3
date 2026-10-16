// oracle: levels 4
// The sweep: a table whose rowspan cell is taller than a page. Typst breaks the rows it spans across pages;
// Austenite sets the rows together and warns `unsupported`.
#set page(width: 220pt, height: 120pt, margin: 20pt)
#table(columns: 2, table.cell(rowspan: 2)[#lorem(60)], [a], [b])
