// `pagebreak(to: "odd")` after a page of text, a strong break leaving a blank page, and the parity
// following the page count through text pages.
#set page(width: 200pt, height: 200pt, margin: 20pt)
#set text(size: 10pt)
#lorem(20)
#pagebreak(to: "odd")
#lorem(90)
#pagebreak(to: "odd")
#lorem(20)
#pagebreak(to: "even")
#lorem(20)
