// oracle: levels 4
// A list whose region may not expand sizes its bodies to the widest: in a box of automatic width, in a
// block of automatic width, in a grid cell and beside text.
#set page(width: 300pt, height: 400pt, margin: 25pt)
#box(stroke: 0.5pt)[
  - short
  - a longer second item
]
#block(fill: luma(240), inset: 4pt)[
  + one
  + two is longer
]
#grid(columns: (auto, 1fr), gutter: 6pt,
  [
    - cell item
    - another
  ],
  [Beside the list.],
)
#align(center)[
  - centred list
  - second
]
