// oracle: levels 2 4
#set page(width: 220pt, height: 260pt, margin: 20pt)
#let n = counter("n")
Text#footnote[Note one.] and#footnote(numbering: "*")[starred].
#set footnote(numbering: "i")
More#footnote[Roman.]
#show footnote.entry: set text(size: 7pt)
#set footnote.entry(separator: line(length: 30%), clearance: 1em, gap: 0.5em, indent: 0pt)
Last#footnote[Final.]
