// Forward references: a value read before what it depends on settles by the second pass.
#let total = state("total", 0)
#context [#metadata((total.final(), counter("n").final(), query(<late>))) <probe>]
#total.update(v => v + 2)
#counter("n").step()
#total.update(v => v + 3)
#counter("n").step()
#metadata("late") <late>
