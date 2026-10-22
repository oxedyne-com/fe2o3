// Nested context and contextual show rules see their own locations.
#context [
  #metadata(1) <inner>
  #context [#metadata(query(<inner>)) <probe>]
]
#let f() = context counter("k").get()
#counter("k").step()
#metadata(f()) <holder>
#counter("k").step()
#context [#metadata((counter("k").get(), counter("k").at(<holder>))) <probe>]
