// oracle: levels 3
// A package module is named by its manifest, whatever it is bound to.
#import "@local/demo:0.1.0" as d
#let result = repr(d)
#repr(result)
