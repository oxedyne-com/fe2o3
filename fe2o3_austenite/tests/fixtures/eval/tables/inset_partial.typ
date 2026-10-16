// oracle: levels 4
// A table's inset given for some sides keeps the default 5 pt on the others, and a cell's own inset folds over the
// table's: the text sits where Typst puts it, horizontally and vertically.
#set page(width: 260pt, height: 420pt, margin: 20pt)
#table(columns: 2, inset: (y: 9pt), [one], [two], [three], [four])
#v(8pt)
#table(columns: 2, inset: (left: 0pt), [one], [two], [three], [four])
#v(8pt)
#table(columns: 2, inset: (rest: 2pt, x: 12pt), [one], [two], [three], [four])
#v(8pt)
#table(columns: 2, inset: (top: 14pt), [one], table.cell(inset: (x: 0pt))[two], [three], table.cell(inset: (bottom: 0pt))[four])
#v(8pt)
#grid(columns: 2, inset: (y: 9pt), [one], [two], [three], [four])
