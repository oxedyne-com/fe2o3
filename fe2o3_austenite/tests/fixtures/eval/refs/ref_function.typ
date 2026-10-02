// oracle: levels 2 4
// `ref` called as a function, with and without a supplement, in a block and in a list item.
#set page(width: 300pt, height: 300pt, margin: 25pt)
#set heading(numbering: "1.")
#ref(<a>) and #ref(<a>, supplement: [Chapter]) and #ref(<a>, supplement: none).
#block(inset: 4pt)[In a block: #ref(<a>).]
- In a list item: #ref(<a>).
= A heading <a>
