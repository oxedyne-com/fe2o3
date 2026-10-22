// oracle: levels 3
// A package that imports another package.
#import "@local/dep:0.1.0": h
#let result = h(2)
#repr(result)
