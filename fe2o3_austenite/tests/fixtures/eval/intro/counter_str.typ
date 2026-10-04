// A string-keyed counter: steps, levels, sets, functions, at, final.
#let c = counter("c")
#context [#metadata(c.get()) <probe>]
#c.step()
#c.step()
#context [#metadata(c.get()) <probe>]
#c.step(level: 3)
#metadata(0) <here1>
#context [#metadata(c.get()) <probe>]
#c.update(7)
#context [#metadata(c.get()) <probe>]
#c.update((2, 5, 9))
#c.step(level: 2)
#context [#metadata(c.get()) <probe>]
#c.update((a, b) => (a + 1, b + 1))
#context [#metadata((c.get(), c.at(<here1>), c.final())) <probe>]
#c.update((..n) => 40)
#context [#metadata(counter("other").get()) <probe>]
