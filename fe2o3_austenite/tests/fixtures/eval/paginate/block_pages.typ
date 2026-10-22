// A breakable block holding text across pages, with inset and fill: its fragments on each page, the fill
// on each, and the paragraph after it.
#set page(width: 200pt, height: 200pt, margin: 20pt)
#set text(size: 10pt)
#lorem(15)
#block(width: 100%, inset: 6pt, fill: luma(235))[#lorem(140)]
#lorem(30)
