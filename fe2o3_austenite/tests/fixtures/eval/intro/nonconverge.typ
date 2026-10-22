// A state that feeds itself never settles; Typst stops after five passes with a warning.
#let s = state("s", 0)
#context s.update(s.final() + 1)
#context [#metadata(s.final()) <probe>]
