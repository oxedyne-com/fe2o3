// Page geometry, numbering, columns and breaks.
#set page(width: 240pt, height: 180pt, margin: (x: 20pt, y: 25pt), numbering: "1")
#set par(justify: true)
= Pages
#lorem(40)
#pagebreak()
#columns(2)[#lorem(50)]
#context [#metadata((here().page(), page.width, counter(page).get())) <probe>]
