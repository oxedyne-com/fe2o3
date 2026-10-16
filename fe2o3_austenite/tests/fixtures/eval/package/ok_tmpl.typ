// oracle: levels 3
// A manifest with a template section and tool tables.
#import "@local/tmpl:0.1.0": x
#import "@local/demo:0.1.0"
#let result = (x, demo.f(0))
#repr(result)
