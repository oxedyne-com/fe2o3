// oracle: levels 2 4
#set page(width: 220pt, height: 260pt, margin: 20pt)
#set page(numbering: none)
#align(center + horizon)[
  #text(size: 22pt, weight: "bold")[The Title]
  #v(1em)
  #line(length: 40%, stroke: 0.5pt)
  #v(1em)
  #text(size: 12pt, style: "italic")[A Subtitle]
  #v(2fr)
  #smallcaps[Author Name]
  #v(1fr)
]
#pagebreak(weak: true)
#set page(numbering: "1")
#counter(page).update(1)
#lorem(40)
