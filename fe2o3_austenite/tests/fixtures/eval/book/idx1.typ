// oracle: levels 2 4
#set page(width: 220pt, height: 260pt, margin: 20pt)
#let idx(term) = [#term#metadata(term) <idx>]
#let gloss(term, def) = [#term#metadata((term: term, def: def)) <gloss>]
#idx("zebra") #idx("Apple") #idx("mango") #idx("apple") #gloss("Foo", [a thing])
#pagebreak()
#context {
  let items = query(<idx>).map(m => lower(m.value)).sorted().dedup()
  for it in items [- #it]
  let gs = query(<gloss>).map(m => m.value)
  for g in gs [*#g.term*: #g.def \ ]
}
