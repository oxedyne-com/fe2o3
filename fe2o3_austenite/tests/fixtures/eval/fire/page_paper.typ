// A page's paper may be given by position, `set page("a5")`, as Typst's `named_or_find` takes it: the
// first unnamed argument that names a paper (aero-check, metronic, sepia-memo and others).
#let paper = "a6"
#set page("a5", margin: 1in)
#context [#metadata((page.width, page.height)) <probe>]
First page.
#page(paper, flipped: true)[Second page.]
#set page(paper, margin: 20pt)
#context [#metadata((page.width, page.height)) <probe>]
Third page.
