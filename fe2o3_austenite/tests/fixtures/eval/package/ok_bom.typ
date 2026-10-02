// oracle: levels 3
// A byte-order mark before an entrypoint is dropped.
#import "@local/bom:0.1.0"
#let result = bom.x
#repr(result)
