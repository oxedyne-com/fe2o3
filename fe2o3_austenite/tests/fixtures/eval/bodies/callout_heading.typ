// oracle: levels 4
// A heading and a figure with a caption inside a filled, inset block.
#set page(width: 220pt, height: 200pt, margin: 20pt)
#block(fill: luma(240), inset: 8pt, width: 100%)[
  #heading(level: 2)[Callout heading]
  Some text in the callout that wraps over lines to see its width.
]
#block(inset: 6pt, stroke: 1pt)[
  #figure(rect(width: 40pt, height: 20pt), caption: [In a block])
]
