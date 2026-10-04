// oracle: levels 2 4
#set page(width: 220pt, height: 260pt, margin: 20pt)
#set heading(numbering: "I.", supplement: [Part])
#show heading: set text(size: 14pt, weight: "regular")
#show heading.where(level: 1): it => block(width: 100%, above: 2em, below: 1em)[#smallcaps(it.body)]
= Alpha
== Beta
= Gamma
