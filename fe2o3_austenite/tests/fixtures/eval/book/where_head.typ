// oracle: levels 4
#set page(width: 220pt, height: 260pt, margin: 20pt)
#show heading.where(level: 1): it => [L1 #it.body]
#show heading.where(depth: 2): it => [D2 #it.body]
#show heading.where(numbering: none): it => [NN #it.body]
= One
== Two
=== Three
#set heading(numbering: "1.")
= Num
