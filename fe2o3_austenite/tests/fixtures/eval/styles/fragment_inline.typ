#let c = counter("p")
#show par: it => { c.step(); it }
#block[hello world]
#context [#metadata(c.final().first()) <p>]
