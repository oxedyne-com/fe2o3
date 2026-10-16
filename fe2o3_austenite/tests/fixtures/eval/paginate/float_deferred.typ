// A float too tall for what is left of a page is deferred to the next page, text flowing on above it, and
// a second float follows it in order.
#set page(width: 200pt, height: 200pt, margin: 20pt)
#set text(size: 10pt)
#lorem(30)
#place(auto, float: true, clearance: 6pt)[#block(width: 100%, height: 120pt, fill: luma(220))]
#lorem(80)
#place(auto, float: true)[#block(width: 100%, height: 40pt, fill: luma(200))]
#lorem(50)
