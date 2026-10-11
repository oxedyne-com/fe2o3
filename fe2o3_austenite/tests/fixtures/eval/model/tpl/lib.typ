// A package-like template: it calls the bibliography function it was handed, from its own directory.
#let template(bibliography: none, body) = {
  body
  if bibliography != none {
    bibliography(title: [Works cited])
  }
}
