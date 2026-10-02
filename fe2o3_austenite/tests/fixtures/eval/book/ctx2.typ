// oracle: levels 2 4
#set page(width: 220pt, height: 260pt, margin: 20pt)
#let chap = state("chap", [none])
#set page(header: context { let p = here().page(); if calc.even(p) [Even #chap.get()] else [Odd #chap.at(here())] })
#show heading.where(level: 1): it => { chap.update(it.body); it }
= First
#lorem(150)
= Second
#lorem(80)
