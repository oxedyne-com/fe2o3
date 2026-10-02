// oracle: rejects
// A package file that is not there is searched for in the package.
#import "@local/esc:0.1.0": rd
#let result = rd("/missing.txt")
