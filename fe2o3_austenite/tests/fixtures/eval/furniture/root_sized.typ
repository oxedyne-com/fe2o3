// oracle: levels 4
// A page holding only a paragraph sized by a `text` call: the call's styles are not liftable, so the footer
// keeps the page's own size.
#set page(width: 200pt, height: 160pt, margin: (x: 20pt, y: 30pt), numbering: "1", header: [Head])
#text(size: 16pt)[Only a sized paragraph.]
