// oracle: levels 3
// A package reads its own data by a package-root path and by a relative one.
#import "@local/demo:0.1.0": data, rel
#let result = (data(), rel())
#repr(result)
