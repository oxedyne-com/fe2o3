// oracle: levels 4
// v with fr inside a block that fills the page height.
#set page(width: 200pt, height: 180pt, margin: 20pt)
#block(height: 100%, width: 100%, stroke: 0.5pt)[
  Header line
  #v(1fr)
  Middle line
  #v(3fr)
  Footer line
]
