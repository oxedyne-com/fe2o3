// U6a-fix indent corpus (synthetic): a paragraph that begins with a box
// oracle: levels 4
// needs: u6b
// indents: 0 12
#set page(width: 200pt, height: 480pt, margin: 0pt)
#set par(justify: true, first-line-indent: 12pt, spacing: 6pt)
#set text(size: 11pt)
#lorem(18)

#box(width: 24pt) #lorem(26)

#box(width: 60pt) #lorem(31)
