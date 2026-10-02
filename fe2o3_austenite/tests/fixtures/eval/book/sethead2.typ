// oracle: levels 2 4
#set page(width: 220pt, height: 260pt, margin: 20pt)
#set heading(numbering: (..nums) => nums.pos().map(str).join("."))
#show heading: it => {
  let n = counter(heading).at(it.location())
  block[#n.first() #h(1em) #it.body]
}
= One
== Two
