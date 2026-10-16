// oracle: levels 2 4
#set page(width: 220pt, height: 260pt, margin: 20pt)
#show outline.entry.where(level: 1): it => {
  v(8pt, weak: true)
  strong(it)
}
#show outline.entry: set text(red)
#set outline.entry(fill: repeat[. ])
#outline(indent: 1em)
= A
== B
= C
