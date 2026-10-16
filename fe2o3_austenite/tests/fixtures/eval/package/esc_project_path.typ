// oracle: rejects
// A project path handed to a package as a string resolves in the package, not the project.
#import "@local/esc:0.1.0": rd
#let result = rd("/_here.txt")
