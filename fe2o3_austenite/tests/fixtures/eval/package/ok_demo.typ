// oracle: levels 3
// Named and renamed imports from a package whose entrypoint imports its own files by package-root and relative paths.
#import "@local/demo:0.1.0": f, g
#import "@local/demo:0.1.0" as d
#let result = (f(3), g(4), d.f(1))
#repr(result)
