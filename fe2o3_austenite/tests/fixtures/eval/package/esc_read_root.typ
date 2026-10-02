// oracle: rejects
// A package-root path that climbs out of the package.
#import "@local/esc:0.1.0": rd
#let result = rd("/../x.txt")
