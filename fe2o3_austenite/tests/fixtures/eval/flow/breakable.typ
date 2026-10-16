// oracle: levels 2 4
// A breakable block across pages, with inset; an unbreakable one moving whole; a fixed-height block spread
// over pages; nested blocks keeping their offsets.
#set page(width: 200pt, height: 200pt, margin: 20pt)
#block(width: 100%, height: 60pt) <probe>
#block(inset: 10pt)[
  #block(width: 40pt, height: 50pt) <probe>
  #block(width: 40pt, height: 50pt) <probe>
  #block(width: 40pt, height: 50pt) <probe>
  #block(width: 40pt, height: 50pt) <probe>
] <probe>
#block(breakable: false)[
  #block(width: 40pt, height: 60pt) <probe>
  #block(width: 40pt, height: 60pt) <probe>
] <probe>
#block(height: 250pt)[#block(width: 10pt, height: 10pt) <probe>] <probe>
#block(width: 10pt, height: 10pt) <probe>
