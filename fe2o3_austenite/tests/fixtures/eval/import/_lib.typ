// A file that fixtures import: plain bindings, and a function that reads a binding it does not export.
#let a = 1
#let b = (2, 3)
#let c = "see"
#let hidden = 100
#let f(x) = x + 10
#let g(x) = x + hidden
