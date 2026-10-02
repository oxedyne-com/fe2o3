// oracle: levels 2 4
// Page geometry: default A4 margins, paper names, explicit sizes, margins by side and axis, `flipped`,
// `page(..)[..]` and `auto` height fitting its content.
#block(width: 10pt, height: 10pt) <probe>
#set page(paper: "a6")
#block(width: 10pt, height: 10pt) <probe>
#set page(width: 300pt, height: 200pt, margin: (x: 20pt, top: 30pt, bottom: 5pt))
#block(width: 10pt, height: 10pt) <probe>
#set page(margin: (left: 50pt))
#block(width: 10pt, height: 10pt) <probe>
#set page(flipped: true)
#block(width: 10pt, height: 10pt) <probe>
#page(width: 120pt, height: auto, margin: 12pt)[
  #block(width: 10pt, height: 40pt) <probe>
  #block(width: 10pt, height: 40pt) <probe>
]
#set page(width: 200pt, height: 200pt, margin: 10%)
#block(width: 10pt, height: 10pt) <probe>
