#let xs = ($1$, $2$, $3$)
#let rows = (($1$, $2$), ($3$, $4$))
$ vec(..#xs) + mat(..#rows) + mat(delim: "[", ..#rows) + vec(a, ..#xs, b) $
