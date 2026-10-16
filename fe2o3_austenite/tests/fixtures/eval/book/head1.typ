// oracle: levels 2 4
#set page(width: 220pt, height: 260pt, margin: 20pt)
#set heading(numbering: "1.1", outlined: true, bookmarked: true)
= Numbered
#heading(numbering: none, outlined: false)[Unnumbered]
#heading(level: 2, outlined: false, supplement: [Sec])[Quiet]
#heading(depth: 3)[Deep]
== Next
#counter(heading).update(5)
= After update
#context counter(heading).display("I")
