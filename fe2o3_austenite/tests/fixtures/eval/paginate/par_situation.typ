// The first-line indent follows where a paragraph stands in its flow: none for the first, none after a block,
// none after a column break, and the indent for one that follows another paragraph. The indent is wide
// enough to move the line breaks, which is what the oracle's line text shows.
#set page(width: 260pt, height: 200pt, margin: 20pt, columns: 2)
#set par(first-line-indent: 50pt)
#set text(size: 10pt)
#lorem(20)

#lorem(20)

#block(inset: 2pt)[#lorem(8)]
#lorem(20)

#colbreak()
#lorem(20)

#lorem(20)
