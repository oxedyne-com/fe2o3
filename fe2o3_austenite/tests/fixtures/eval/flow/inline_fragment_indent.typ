// oracle: levels 4
// A container body of one paragraph, closed or not by the styles around it, is inline content with no
// first-line indent even when every paragraph takes one; two paragraphs, or a paragraph after a block, take it.
#set page(width: 300pt, height: 560pt, margin: 20pt)
#set text(size: 10pt)
#set par(first-line-indent: (amount: 20pt, all: true), justify: false)
#rect[#set par(leading: 3pt)
A: #lorem(4)]

#rect[#set text(fill: red)
#set par(leading: 3pt)
B: #lorem(4)]

#rect[#metadata(none) <m1>C: #lorem(4)]

#rect[#set align(center)
D: #lorem(4)]

#rect[X: #lorem(3) #[#set par(leading: 3pt)
Y: #lorem(3)] Z: #lorem(3)]

#rect[#set par(leading: 3pt)
E: #lorem(3) #set par(leading: 6pt);more #lorem(2)]

#rect[#par(first-line-indent: 7pt)[F: #lorem(4)]]

#rect[#heading[H] G: #lorem(4)]

#block[#set par(leading: 3pt)
P: #lorem(4)

Q: #lorem(4)]
