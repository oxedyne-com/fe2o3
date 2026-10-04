// oracle: levels 3
// needs: text
// A package file includes another by a relative path that climbs within the package.
#import "@local/demo:0.1.0": part
#let result = part.text
#repr(result)
