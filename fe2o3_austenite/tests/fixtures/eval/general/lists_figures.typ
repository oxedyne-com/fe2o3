// Lists, enumerations, terms, figures and a table, laid out.
#set page(width: 300pt, height: 400pt, margin: 30pt)
= Lists
- apples
- pears
  - nested
+ one
+ two
/ Term: its gloss

#figure(table(columns: 2, [a], [b], [c], [d]), caption: [A table])
#figure(rect(width: 40pt, height: 20pt), caption: [A shape]) <shape>
See @shape.
#context [#metadata((counter(figure.where(kind: table)).get(), counter(figure.where(kind: image)).get())) <probe>]
