// oracle: levels 4
// Indent, body indent, spacing, a marker array and a marker function.
#set page(width: 300pt, height: 400pt, margin: 25pt)
#list(indent: 12pt, body-indent: 1.2em, spacing: 9pt)[one][two][three]
#list(marker: [--], tight: true)[dash][dash two]
#list(marker: ([>], [>>], [>>>]))[
  top
  - middle
    - deep
      - deeper
]
#list(marker: n => [#(n + 1).])[fn one][fn two]
