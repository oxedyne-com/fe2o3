// A paragraph across pages: its lines break where the page ends, widow and orphan control keeping two lines
// together, then a second paragraph and a heading-like block starting a page.
#set page(width: 200pt, height: 200pt, margin: 20pt)
#set text(size: 10pt)
#lorem(160)

#lorem(60)

#block(sticky: true)[#text(weight: "bold")[A sticky line]]
#lorem(90)
