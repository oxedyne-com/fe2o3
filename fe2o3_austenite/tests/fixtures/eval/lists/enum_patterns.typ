// oracle: levels 4
// Numbering patterns, `full`, `reversed`, an explicit start and explicit numbers.
#set page(width: 300pt, height: 500pt, margin: 25pt)
#set enum(numbering: "(i)")
+ roman
+ roman two
#set enum(numbering: "A.", start: 3)
+ letter C
+ letter D
#enum(reversed: true)[three][two][one]
#set enum(numbering: "1.a)", full: true)
+ outer
  + inner
  + inner two
+ outer two
  + inner again
