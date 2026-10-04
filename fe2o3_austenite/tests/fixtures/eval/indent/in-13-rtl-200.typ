// U6a-fix indent corpus (synthetic): a right-to-left paragraph, indented at its start
// oracle: levels 4
// known: right-to-left base direction: a last line is set flush left where Typst sets it flush right, and a final full stop is set apart from its line
// indents: 0 12
#set page(width: 200pt, height: 480pt, margin: 0pt)
#set par(justify: true, first-line-indent: 12pt, spacing: 6pt)
#set text(size: 11pt)
#set text(dir: rtl)
#lorem(30)

#lorem(34)

#lorem(25)
