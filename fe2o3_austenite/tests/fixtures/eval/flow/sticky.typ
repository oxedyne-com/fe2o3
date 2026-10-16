// oracle: levels 2 4
// Sticky blocks move to the next page with the block they stick to; a sticky block at the top of a page
// stays; `block(sticky: true)` runs of two.
#set page(width: 200pt, height: 200pt, margin: 20pt)
#block(width: 30pt, height: 100pt) <probe>
#block(width: 30pt, height: 20pt, sticky: true) <probe>
#block(width: 30pt, height: 50pt) <probe>
#block(width: 30pt, height: 20pt, sticky: true) <probe>
#block(width: 30pt, height: 10pt, sticky: true) <probe>
#block(width: 30pt, height: 100pt) <probe>
#pagebreak()
#block(width: 30pt, height: 20pt, sticky: true) <probe>
#block(width: 30pt, height: 170pt) <probe>
