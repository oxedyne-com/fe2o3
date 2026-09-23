// Counters, state and context queries across pages.
#set page(width: 220pt, height: 160pt, margin: 20pt)
#set heading(numbering: "1.")
#let st = state("seen", 0)
#let ctr = counter("mine")
= One
#st.update(x => x + 1) #ctr.step()
#context [#metadata((counter(heading).get(), st.get(), ctr.get(), here().page())) <probe>]
#pagebreak()
= Two
#st.update(x => x + 10) #ctr.step() #ctr.step()
#context [#metadata((counter(heading).display(), st.get(), ctr.get(), counter(page).get(), st.final())) <probe>]
== Two point one
#context [#metadata(query(heading).map(h => (h.body, h.level, h.location().page()))) <probe>]
#context [#metadata(query(selector(heading).before(here())).len()) <probe>]
