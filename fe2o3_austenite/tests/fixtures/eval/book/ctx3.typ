// oracle: levels 2 4
#set page(width: 220pt, height: 260pt, margin: 20pt)
#set page(header: context {
  let hs = query(heading.where(level: 1).before(here()))
  if hs.len() > 0 { emph(hs.last().body) }
  line(length: 100%)
})
= One
#lorem(80)
#pagebreak()
= Two
#lorem(40)
