// oracle: levels 3
// Two versions of one package side by side.
#import "@local/multi:0.1.0" as one
#import "@local/multi:0.2.0" as two
#let result = (one.v, two.v)
#repr(result)
