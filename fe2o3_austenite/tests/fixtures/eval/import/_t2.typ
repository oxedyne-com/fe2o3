// A template that sets its title as a heading before the body.
#let doc(title: none, body) = {
  if title != none [= #title]
  body
}
