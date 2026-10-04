// oracle: levels 4
// A footnote in the result of `layout`, called again whenever the content is laid out.
#set page(width: 200pt, height: 170pt, margin: 20pt)
#lorem(14)

#layout(size => [Width #calc.round(size.width.pt()) with a note#footnote[In the layout.].])

#lorem(10)
