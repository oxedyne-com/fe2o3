// State: initial values, set and function updates, independent keys, at and final.
#let s = state("s", 1)
#let t = state("t")
#context [#metadata((s.get(), t.get())) <probe>]
#s.update(5)
#t.update("x")
#context [#metadata((s.get(), t.get())) <probe>]
#s.update(v => v * 3)
#metadata(none) <mid>
#s.update(v => (v, v))
#context [#metadata((s.get(), s.at(<mid>), s.final(), t.final())) <probe>]
#context [#metadata(state("s", 100).get()) <probe>]
#context [#metadata(state("fresh", "init").final()) <probe>]
