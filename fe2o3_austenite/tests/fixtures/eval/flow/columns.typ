// oracle: levels 2 4
// Columns: `set page(columns:)` with a column break, `columns(n)` inside the page with its gutter, a weak
// column break in an empty column, and a column layout breaking across pages.
#set page(width: 300pt, height: 200pt, margin: 20pt)
#set page(columns: 2)
#block(width: 30pt, height: 60pt) <probe>
#colbreak()
#block(width: 30pt, height: 60pt) <probe>
#colbreak()
#colbreak(weak: true)
#block(width: 30pt, height: 60pt) <probe>
#set page(columns: 1)
#block(width: 30pt, height: 20pt) <probe>
#columns(3, gutter: 10pt)[
  #block(width: 30pt, height: 50pt) <probe>
  #block(width: 30pt, height: 50pt) <probe>
  #block(width: 30pt, height: 50pt) <probe>
  #block(width: 30pt, height: 50pt) <probe>
  #block(width: 30pt, height: 50pt) <probe>
  #block(width: 30pt, height: 50pt) <probe>
  #block(width: 30pt, height: 50pt) <probe>
]
#block(width: 30pt, height: 20pt) <probe>
