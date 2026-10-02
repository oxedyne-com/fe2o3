// A template: a function taking named options and the document body, as `show: doc.with(..)` supplies it.
#let doc(title: none, body) = {
  if title != none [- #title]
  body
}
