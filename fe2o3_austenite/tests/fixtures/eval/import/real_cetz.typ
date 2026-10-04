// oracle: none
// A real package from Typst's cache, used the way a document uses it; evaluated, not compared.
#import "@preview/cetz:0.3.4": canvas, draw
#canvas({
  import draw: *
  line((0, 0), (2, 1))
  circle((1, 1), radius: 0.5)
})
