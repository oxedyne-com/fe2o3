// oracle: levels 4
// A counter stepped inside a table cell that holds a footnote, and read after the table.
#set page(width: 200pt, height: 170pt, margin: 20pt)
#lorem(20)

#table(columns: 2,
  [first#counter("c").step()#footnote[Note one.]],
  [second#counter("c").step()],
)

#context [Counted #counter("c").get().first() cells.]
