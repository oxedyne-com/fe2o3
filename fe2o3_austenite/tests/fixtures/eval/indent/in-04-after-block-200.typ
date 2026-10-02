// U6a-fix indent corpus (synthetic): a paragraph after a block is not indented, the next one is
// oracle: levels 4
// indents: 1 12
#set page(width: 200pt, height: 480pt, margin: 0pt)
#set par(justify: true, first-line-indent: 12pt, spacing: 0pt)
#set text(size: 11pt)
#set block(spacing: 0pt)
#lorem(30)

#block(height: 0pt)

#lorem(34)

#lorem(29)
