// oracle: levels 3
// The same package imported twice, in two forms, is one module.
#import "@local/demo:0.1.0"
#import "@local/demo:0.1.0": f
#let result = (demo.f(2), f(2), demo.g(5))
#repr(result)
