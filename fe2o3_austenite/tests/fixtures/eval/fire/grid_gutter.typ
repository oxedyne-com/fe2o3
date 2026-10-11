// `gutter` is a grid's and a table's shorthand for both gutters, in a `set` rule as in a call; a gutter
// given outright wins (subpar's default grid styles, cogsci, ezexam).
#set grid(columns: 2, gutter: 1em)
#grid([A], [B], [C], [D])
#set grid(gutter: 2em, row-gutter: 4pt)
#grid([E], [F], [G], [H])
#set table(columns: 3, gutter: 3pt)
#table([1], [2], [3], [4], [5], [6])
