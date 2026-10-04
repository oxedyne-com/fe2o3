// oracle: levels 4
// A footnote numbered with its own pattern, notes in a list item,
// in a block and in a table cell.
#set page(width: 300pt, height: 300pt, margin: 25pt)
Text#footnote(numbering: "*")[Starred.] and again#footnote[Plain.] and the end.

- an item with a note#footnote[In the list.]

#block(inset: 4pt)[A block with a note#footnote[In the block.]]

#table(columns: 2, [cell with a note#footnote[In the cell.]], [other])
