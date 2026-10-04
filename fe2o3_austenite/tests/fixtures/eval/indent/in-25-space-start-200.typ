// U6a-fix indent corpus (synthetic): a paragraph that begins with a space, or with a space as wide as the line
// oracle: levels 4
// needs: u6b
// indents: 0 12
#set page(width: 200pt, height: 480pt, margin: 0pt)
#set par(justify: true, first-line-indent: 12pt, spacing: 6pt)
#set text(size: 11pt)
#lorem(18)

#h(8pt) #lorem(24)

#h(150pt) Pneumonoultramicroscopicsilicovolcanoconiosis follows a wide space.

#h(190pt) #lorem(12)
