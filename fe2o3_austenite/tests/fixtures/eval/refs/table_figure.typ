// oracle: levels 2 4
// A table in a figure is referred to by its figure's kind and number.
#set page(width: 300pt, height: 300pt, margin: 25pt)
#figure(table(columns: 2, [a], [b], [c], [d]), caption: [A table]) <tab>
See @tab and #ref(<tab>).
#figure(rect(width: 30pt, height: 10pt), caption: [A shape]) <shape>
Then @shape and @tab.
