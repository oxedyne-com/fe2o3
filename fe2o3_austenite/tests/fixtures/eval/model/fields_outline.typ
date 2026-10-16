// query: outline title
#set document(title: [The Doc])
#title()
#title[Explicit]
#outline()
#outline(title: [Figures], target: figure.where(kind: table), depth: 2, indent: 1em)
= A
