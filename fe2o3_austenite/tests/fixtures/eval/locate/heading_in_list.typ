// oracle: levels 4
// A numbered heading in a list item whose note relays the region out.
#set page(width: 200pt, height: 170pt, margin: 20pt)
#set heading(numbering: "1.")
= Before
#lorem(8)

- an item
- an item with a note#footnote[In the item.] and a heading:
  == Inside <inside>
  Body of the heading.

See @inside.
