// An element's scoped functions are its content's methods: `it.prefix()`, `it.body()`, `it.page()` and
// `it.indented(..)` on an `outline.entry` in a show rule (outrageous, clean-dhbw, latex-lookalike).
#set heading(numbering: "1.")
#show outline.entry: it => link(
  it.element.location(),
  it.indented(if it.prefix() == none { [--] } else { it.prefix() }, it.body() + h(1fr) + it.page()),
)
#outline()
= One
== One A
#heading(numbering: none)[Two]
