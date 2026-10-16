// oracle: levels 3
// Paths that climb and stay within the package.
#import "@local/esc:0.1.0": rd
#let result = (rd("../inside.txt"), rd("/inside.txt"), rd("../src/../inside.txt"))
#repr(result)
