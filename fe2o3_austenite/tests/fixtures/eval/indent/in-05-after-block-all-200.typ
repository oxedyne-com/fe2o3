// U6a-fix indent corpus (synthetic): all: true indents the paragraph after a block as well
// oracle: levels 4
// indents: 3 12
#set page(width: 200pt, height: 480pt, margin: 0pt)
#set par(justify: true, first-line-indent: (amount: 12pt, all: true), spacing: 0pt)
#set text(size: 11pt)
#set block(spacing: 0pt)
#lorem(30)

#block(height: 0pt)

#lorem(34)

#lorem(29)
