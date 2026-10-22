// oracle: levels 4
// Nested numbered lists without `full` take the pattern's piece for their depth.
#set page(width: 300pt, height: 400pt, margin: 25pt)
#set enum(numbering: "1.a.i)")
+ one
  + inner
    + innermost
    + innermost two
  + inner two
+ two
#enum(numbering: "(1)", full: true)[
  alpha
  #enum(numbering: "(1)", full: true)[beta][gamma]
]
